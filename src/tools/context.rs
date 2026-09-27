//! `red_engine2 context <feature | file | words>`: a **work packet** for one change, sized to a byte budget (default 12 KB, about 3k tokens).
//!
//! An agent that must touch the engine otherwise pays for `src map` (17 KB), `describe commands` (11 KB), several `search` calls and a few `src outline`s
//! before it can start. The packet answers "what do I need to know to change *this*?" in one call, from the same index `impact` uses (`docs/features.json`)
//! and the same scanner as `src` (`tools::symbols`): the feature's summary and neighbours, each owned source file with its purpose and public API
//! (signature + first doc line), the tests that cover it, the exact command that verifies a change, and the ADRs/docs to consult **as one-line pointers**,
//! never their text. Parts are prioritised: orientation, tests and pointers are always kept; the API listing gets the rest of the budget and says how
//! to get more (`src outline`, `src show`) when it is cut.

use super::features::{self, Feature};
use super::symbols::{self, Index};
use std::collections::BTreeMap;

/// Default size of a packet in bytes.
pub const DEFAULT_BUDGET: usize = 12_000;

/// Which features a query names: an exact feature name, a file path (its owners), or words matched against names, summaries and file lists.
pub fn resolve<'a>(all: &'a [Feature], query: &[String]) -> Vec<&'a Feature> {
    let mut out: Vec<&Feature> = Vec::new();
    let mut push = |f: &'a Feature| {
        if !out.iter().any(|g| g.name == f.name) {
            out.push(f);
        }
    };
    let mut words: Vec<String> = Vec::new();
    for q in query {
        if let Some(f) = all.iter().find(|f| f.name == *q) {
            push(f);
        } else if q.contains('/') || q.ends_with(".rs") || q.ends_with(".json") {
            features::owners(all, q).into_iter().for_each(&mut push);
        } else {
            words.push(q.to_lowercase());
        }
    }
    if !words.is_empty() {
        let mut ranked: Vec<(usize, &Feature)> = all
            .iter()
            .map(|f| {
                let name = f.name.replace('_', " ");
                let hay = format!("{} {} {}", name, f.summary, f.files.join(" ")).to_lowercase();
                let score: usize = words.iter().map(|w| usize::from(hay.contains(w.as_str())) + 2 * usize::from(name.contains(w.as_str()))).sum();
                (score, f)
            })
            .filter(|(s, _)| *s > 0)
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));
        ranked.into_iter().take(2).for_each(|(_, f)| push(f));
    }
    out
}

/// Source files under `src/` that `f` owns, resolved against the scanned tree (patterns like `src/net/**` expand).
fn owned_sources<'a>(f: &Feature, ix: &'a Index) -> Vec<&'a str> {
    let mut v: Vec<&str> =
        ix.files.iter().map(|s| s.rel.as_str()).filter(|r| r.starts_with("src/") && f.files.iter().any(|p| features::glob_match(p, r))).collect();
    v.sort();
    v
}

/// Cuts `text` to at most `max` bytes on a line boundary.
fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let cut = text[..end].rfind('\n').unwrap_or(end);
    text[..cut].to_string()
}

/// One feature's packet within `budget` bytes.
pub fn packet(all: &[Feature], f: &Feature, ix: &Index, budget: usize) -> String {
    let dependents: Vec<&str> = all.iter().filter(|g| g.depends_on.contains(&f.name)).map(|g| g.name.as_str()).collect();
    let mut head = format!("== {}: {}\n", f.name, f.summary);
    if !f.depends_on.is_empty() {
        head.push_str(&format!("builds on: {}\n", f.depends_on.join(", ")));
    }
    if !dependents.is_empty() {
        head.push_str(&format!("built on by (a change here can break them): {}\n", dependents.join(", ")));
    }

    let sources = owned_sources(f, ix);
    let mods: BTreeMap<String, symbols::ModInfo> = symbols::modules(ix).into_iter().map(|m| (m.file.clone(), m)).collect();
    let mut files = String::from("files (lines, purpose):\n");
    for s in &sources {
        match mods.get(*s) {
            Some(m) => files.push_str(&format!("  {s} ({}L) {}\n", m.lines, m.purpose.chars().take(90).collect::<String>())),
            None => files.push_str(&format!("  {s}\n")),
        }
    }
    let others: Vec<&String> = f.files.iter().filter(|p| !p.starts_with("src/")).collect();
    if !others.is_empty() {
        files.push_str(&format!("  also: {}\n", others.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    }

    let mut tests = String::from("tests that cover it:\n");
    let (libs, suites): (Vec<&String>, Vec<&String>) = f.tests.iter().partition(|t| t.starts_with("lib:"));
    if !libs.is_empty() {
        tests.push_str(&format!(
            "  unit (cargo test --lib -- <filter>): {}\n",
            libs.iter().map(|t| t.trim_start_matches("lib:")).collect::<Vec<_>>().join(", ")
        ));
    }
    if !suites.is_empty() {
        tests.push_str(&format!("  integration (tests/<name>.rs): {}\n", suites.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
    }
    tests.push_str("  verify a change: `red_engine2 affected --quick` (owners only) then `red_engine2 affected` (plus dependents)\n");
    if !f.commands.is_empty() {
        tests.push_str(&format!("  look/verify by hand: {}\n", f.commands.join(" ; ")));
    }

    let mut docs = String::new();
    if !f.docs.is_empty() {
        docs.push_str("read only if needed (pointers, `search <topic>` prints the relevant section):\n");
        for d in &f.docs {
            let title = if d.starts_with("docs/adr/") { adr_title(d) } else { String::new() };
            docs.push_str(&format!("  {d}{}\n", if title.is_empty() { String::new() } else { format!("  {title}") }));
        }
    }
    let drill = "drill down: `src outline <file>` (all pub items) | `src show <symbol>` (one item's source) | `src refs <symbol>` (who uses it)\n";

    let fixed = head.len() + files.len() + tests.len() + docs.len() + drill.len() + 64;
    let api_budget = budget.saturating_sub(fixed);
    let mut api = String::from("public API (signature // first doc line; `src show <name>` prints the source):\n");
    let mut cut = 0usize;
    'files: for s in &sources {
        let items: Vec<&symbols::Symbol> =
            ix.symbols.iter().filter(|y| y.file == **s && y.public && !y.in_tests && !matches!(y.kind, "impl" | "mod")).collect();
        if items.is_empty() {
            continue;
        }
        let title = s.to_string();
        if api.len() + title.len() + 2 > api_budget {
            cut += items.len();
            continue;
        }
        api.push_str(&format!("{title}\n"));
        for (n, y) in items.iter().enumerate() {
            let sig: String = y.sig.chars().take(110).collect();
            let doc: String = y.doc.chars().take(90).collect();
            let indent = if y.container.is_some() { "    " } else { "  " };
            let line = if doc.is_empty() { format!("{indent}{sig}\n") } else { format!("{indent}{sig}  // {doc}\n") };
            if api.len() + line.len() > api_budget {
                cut += items.len() - n;
                continue 'files;
            }
            api.push_str(&line);
        }
    }
    if cut > 0 {
        api.push_str(&format!("  ... {cut} more public item(s) not shown (budget): `src outline <file>`\n"));
    }
    let text = format!("{head}{files}{api}{tests}{docs}{drill}");
    clip(&text, budget)
}

/// The first heading of an ADR, from the embedded copy.
fn adr_title(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    super::search::adr_title(file).unwrap_or_default()
}

/// The full packet for a query: one section per matched feature (at most three), the budget split between them.
pub fn build(all: &[Feature], ix: &Index, query: &[String], budget: usize) -> Result<String, String> {
    let hits = resolve(all, query);
    if hits.is_empty() {
        return Err(format!("no feature matches '{}': `features` lists them, or name a file (`context src/net/server.rs`)", query.join(" ")));
    }
    let hits: Vec<&Feature> = hits.into_iter().take(3).collect();
    let each = budget / hits.len();
    let mut out = String::new();
    for f in &hits {
        out.push_str(&packet(all, f, ix, each));
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(name: &str, summary: &str, files: &[&str]) -> Feature {
        Feature {
            name: name.into(),
            summary: summary.into(),
            files: files.iter().map(|s| s.to_string()).collect(),
            tests: vec!["lib:sim::flow".into(), "net_flow".into()],
            commands: vec![],
            docs: vec![],
            depends_on: vec![],
        }
    }

    #[test]
    fn a_query_resolves_by_name_path_or_words() {
        let all = vec![
            feature("match_flow", "lobby, countdown and rounds", &["src/sim/flow.rs"]),
            feature("net_client", "the UDP client and prediction", &["src/net/client.rs"]),
        ];
        let names = |q: &[&str]| resolve(&all, &q.iter().map(|s| s.to_string()).collect::<Vec<_>>()).iter().map(|f| f.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&["match_flow"]), ["match_flow"]);
        assert_eq!(names(&["src/net/client.rs"]), ["net_client"]);
        assert_eq!(names(&["lobby"]), ["match_flow"]);
        assert_eq!(names(&["prediction", "client"]), ["net_client"]);
        assert!(names(&["zzz"]).is_empty());
    }

    #[test]
    fn clipping_keeps_whole_lines() {
        assert_eq!(clip("aaa\nbbb\nccc", 9), "aaa\nbbb");
        assert_eq!(clip("short", 100), "short");
    }
}
