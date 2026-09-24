//! `red_engine2 search <words>`: one query across everything an AI could need — the docs
//! (SPEC/AGENTS/glossary chunked by heading, ADRs whole), the asset catalogue, lint codes, conventions, recipes,
//! CLI commands and the Rust symbol table — returning only the few best *fragments*, each with
//! where to read more. BM25-style scoring (IDF-weighted, title/tag boosts, light stemming and a
//! small synonym table): no model download, no vector store, deterministic, milliseconds.
//!
//! Embeddings were deliberately skipped: for a corpus this small and this jargon-heavy, weighted
//! keyword search with synonyms finds the right section reliably, and adds zero dependencies.

use super::{catalog, describe, recipes, symbols};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const SPEC: &str = include_str!("../../SPEC.md");
const AGENTS: &str = include_str!("../../AGENTS.md");
/// `docs/GLOSSARY.md`, embedded (`describe glossary`, `search --kind glossary`).
pub const GLOSSARY: &str = include_str!("../../docs/GLOSSARY.md");
/// The ADR index table (`describe decisions`).
pub const ADR_INDEX: &str = include_str!("../../docs/adr/README.md");
/// Every ADR, embedded so `search` needs no files at runtime. A new `docs/adr/NNNN-*.md` must be added
/// here; `tests::every_adr_is_registered` fails otherwise.
const ADRS: &[(&str, &str)] = &[
    ("0001-json-scenes-and-self-describing-cli.md", include_str!("../../docs/adr/0001-json-scenes-and-self-describing-cli.md")),
    ("0002-props-in-rust-prefabs-in-json.md", include_str!("../../docs/adr/0002-props-in-rust-prefabs-in-json.md")),
    ("0003-one-physics-shared-by-game-and-tools.md", include_str!("../../docs/adr/0003-one-physics-shared-by-game-and-tools.md")),
    ("0004-reachable-ground-height-for-floors.md", include_str!("../../docs/adr/0004-reachable-ground-height-for-floors.md")),
    ("0005-fixed-step-physics-with-camera-interpolation.md", include_str!("../../docs/adr/0005-fixed-step-physics-with-camera-interpolation.md")),
    ("0006-parse-time-expansion-for-sugar.md", include_str!("../../docs/adr/0006-parse-time-expansion-for-sugar.md")),
    ("0007-docs-cannot-drift.md", include_str!("../../docs/adr/0007-docs-cannot-drift.md")),
    ("0008-procedural-assets-only.md", include_str!("../../docs/adr/0008-procedural-assets-only.md")),
    ("0009-four-maps-one-asset-library.md", include_str!("../../docs/adr/0009-four-maps-one-asset-library.md")),
    ("0010-headless-match-server.md", include_str!("../../docs/adr/0010-headless-match-server.md")),
    ("0011-two-characters-and-exact-melee-hits.md", include_str!("../../docs/adr/0011-two-characters-and-exact-melee-hits.md")),
    ("0012-rapier-prop-physics-dormant-until-disturbed.md", include_str!("../../docs/adr/0012-rapier-prop-physics-dormant-until-disturbed.md")),
    ("0013-revolver-second-weapon-hitscan-infinite-ammo.md", include_str!("../../docs/adr/0013-revolver-second-weapon-hitscan-infinite-ammo.md")),
    ("0014-sim-foundations-fixed-tick-scratch-change-tracking-static-props.md", include_str!("../../docs/adr/0014-sim-foundations-fixed-tick-scratch-change-tracking-static-props.md")),
    ("0015-engine-first-refocus-test-lab-and-legacy-maps.md", include_str!("../../docs/adr/0015-engine-first-refocus-test-lab-and-legacy-maps.md")),
];

/// One searchable fragment: kind, title, body, where to read more, and boosted tokens.
pub struct Doc {
    pub kind: &'static str,
    pub title: String,
    pub body: String,
    /// Where to read more (file:line, or a command to run).
    pub loc: String,
    /// Tokens weighted 3x (tags, names).
    pub extra: String,
}

const STOP: &[&str] = &["the", "a", "an", "of", "to", "in", "is", "it", "and", "or", "for", "on", "how", "do", "i", "can", "what", "with", "my", "be", "are", "does", "make", "get", "use", "add", "want", "need"];

const SYNONYMS: &[(&str, &str)] = &[
    ("collide", "collision collider blocks block solid"),
    ("collision", "collide collider blocks solid walk-through"),
    ("block", "collision collider solid"),
    ("stair", "stairs staircase steps rise run landing"),
    ("door", "doorway opening header"),
    ("window", "opening glass sill"),
    ("light", "lamp lights point sun ambient shadow"),
    ("lamp", "light point"),
    ("floor", "slab ground plane level upstairs"),
    ("upstairs", "floor slab stairs level"),
    ("template", "prefab prefabs"),
    ("reusable", "prefab prefabs"),
    ("wall", "walls opening thickness"),
    ("fence", "perimeter gate posts leak"),
    ("fall", "drop railing"),
    ("stuck", "unreachable walk blocked"),
    ("reach", "reachable unreachable walk"),
    ("spawn", "camera start player"),
    ("color", "material hex emissive"),
    ("glow", "emissive light"),
    ("sound", "audio"),
    ("size", "dimensions bounds"),
    ("furniture", "prop prefab chair table sofa"),
    ("test", "verify checks golden"),
    ("screenshot", "frame tour plan render view"),
    ("picture", "frame tour plan render view png"),
    ("image", "frame tour plan render png"),
    ("place", "position add move"),
    ("delete", "rm remove"),
    ("copy", "clone array duplicate"),
    ("random", "scatter seed"),
    ("tree", "scatter landscaping plant"),
    ("room", "zone wall floor"),
    ("map", "scene json level"),
    ("level", "map scene floor"),
];

fn stem(w: &str) -> String {
    let w = w.to_lowercase();
    for suf in ["ing", "ies", "es", "ed", "s"] {
        if w.len() > suf.len() + 2 && w.ends_with(suf) {
            return if suf == "ies" { format!("{}y", &w[..w.len() - 3]) } else { w[..w.len() - suf.len()].to_string() };
        }
    }
    w
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '-')).filter(|w| w.len() > 1).flat_map(|w| {
        // split snake/camel-ish joined names too: chair_folding -> chair, folding
        let mut v = vec![stem(w)];
        if w.contains('-') {
            v.extend(w.split('-').filter(|p| p.len() > 1).map(stem));
        }
        v
    }).filter(|w| !STOP.contains(&w.as_str())).collect()
}

fn tokens_snake(s: &str) -> Vec<String> {
    tokens(&s.replace('_', " "))
}

fn chunk_markdown(file: &str, kind: &'static str, text: &str, out: &mut Vec<Doc>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut heading = String::from("(top)");
    let mut start = 0usize;
    let mut sec: Vec<&str> = Vec::new();
    let flush = |heading: &str, start: usize, sec: &mut Vec<&str>, out: &mut Vec<Doc>| {
        let body: Vec<&str> = std::mem::take(sec);
        if body.iter().all(|l| l.trim().is_empty()) {
            return;
        }
        for (k, w) in body.chunks(28).enumerate() {
            out.push(Doc { kind, title: heading.to_string(), body: w.join("\n"), loc: format!("{file}:{}", start + k * 28 + 1), extra: String::new() });
        }
    };
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with('#') && !l.starts_with("#!") {
            flush(&heading, start, &mut sec, out);
            heading = l.trim_start_matches('#').trim().to_string();
            start = i;
        }
        sec.push(l);
    }
    flush(&heading, start, &mut sec, out);
}

/// Builds the whole search corpus (docs, glossary, ADRs, assets, lint codes, recipes, commands, public symbols).
pub fn corpus(commands: &Value) -> Vec<Doc> {
    let mut docs = Vec::new();
    chunk_markdown("SPEC.md", "doc", SPEC, &mut docs);
    chunk_markdown("AGENTS.md", "doc", AGENTS, &mut docs);
    chunk_markdown("docs/GLOSSARY.md", "glossary", GLOSSARY, &mut docs);
    for (file, text) in ADRS {
        // One doc per ADR (they are short): the title line names the decision, the rest is the body.
        let title = text.lines().next().unwrap_or("").trim_start_matches('#').trim();
        docs.push(Doc {
            kind: "adr",
            title: format!("ADR {title}"),
            body: text.lines().skip(1).collect::<Vec<_>>().join("\n"),
            loc: format!("docs/adr/{file}"),
            extra: title.to_string(),
        });
    }
    for e in catalog::entries() {
        docs.push(Doc {
            kind: "asset",
            title: format!("{} ({}, {})", e.name, e.kind, e.category),
            body: format!("{} — {:.2}x{:.2}x{:.2} m; tags: {}", e.desc, e.size().x, e.size().y, e.size().z, e.tags.join(" ")),
            loc: format!("red_engine2 catalog {}", e.name),
            extra: format!("{} {}", e.name.replace('_', " "), e.tags.join(" ")),
        });
    }
    for (c, sev, d) in describe::LINT_CODES {
        docs.push(Doc { kind: "lint", title: format!("lint code `{c}` ({sev})"), body: d.to_string(), loc: "red_engine2 describe lint".into(), extra: c.replace('-', " ") });
    }
    for (k, d) in describe::CONVENTIONS {
        docs.push(Doc { kind: "rule", title: format!("convention: {k}"), body: d.to_string(), loc: "red_engine2 describe conventions".into(), extra: String::new() });
    }
    for t in describe::TYPES {
        docs.push(Doc {
            kind: "type",
            title: format!("object type `{}`", t.name),
            body: format!("{}\n{}", t.summary, t.fields.iter().map(|(n, ty, d)| format!("{n} {ty} {d}")).collect::<Vec<_>>().join("\n")),
            loc: "red_engine2 describe objects".into(),
            extra: t.name.to_string(),
        });
    }
    for r in recipes::all() {
        docs.push(Doc {
            kind: "recipe",
            title: format!("recipe `{}`: {}", r.name, r.title()),
            body: format!("{} Teaches: {}", r.summary(), r.json["recipe"]["teaches"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default()),
            loc: format!("red_engine2 recipe {}", r.name),
            extra: r.name.replace('_', " "),
        });
    }
    for c in commands.as_array().into_iter().flatten() {
        let name = c["name"].as_str().unwrap_or("?");
        let flags: Vec<String> = c["args"].as_array().into_iter().flatten().map(|a| format!("{} {}", a["name"].as_str().unwrap_or(""), a["help"].as_str().unwrap_or(""))).collect();
        docs.push(Doc { kind: "command", title: format!("command `{name}`"), body: format!("{}\n{}", c["about"].as_str().unwrap_or(""), flags.join("\n")), loc: format!("red_engine2 {name} --help"), extra: name.to_string() });
    }
    if let Some(root) = symbols::find_root() {
        let ix = symbols::Index::build(&root);
        for s in ix.symbols.iter().filter(|s| !s.in_tests && s.public && !matches!(s.kind, "impl" | "mod")) {
            docs.push(Doc { kind: "src", title: s.sig.clone(), body: s.doc.clone(), loc: format!("{}:{}  (red_engine2 src show {})", s.file, s.line, s.name), extra: format!("{} {}", s.name.replace('_', " "), s.container.clone().unwrap_or_default()) });
        }
    }
    docs
}

/// A scored search result with the best-matching fragment.
pub struct Hit<'a> {
    pub doc: &'a Doc,
    pub score: f32,
    pub fragment: String,
}

fn expand(query: &str) -> Vec<(String, f32)> {
    let mut out: Vec<(String, f32)> = Vec::new();
    for t in tokens_snake(query) {
        out.push((t.clone(), 1.0));
        for (k, syn) in SYNONYMS {
            if stem(k) == t {
                out.extend(tokens_snake(syn).into_iter().map(|s| (s, 0.35)));
            }
        }
    }
    out
}

/// Ranks the corpus for `query`, optionally only one `kind`, returning at most `limit` hits (diversified per kind).
pub fn search<'a>(docs: &'a [Doc], query: &str, kind: Option<&str>, limit: usize) -> Vec<Hit<'a>> {
    let q = expand(query);
    if q.is_empty() {
        return Vec::new();
    }
    let toks: Vec<(Vec<String>, Vec<String>, Vec<String>)> = docs.iter().map(|d| (tokens_snake(&d.title), tokens_snake(&d.body), tokens_snake(&d.extra))).collect();
    let mut df: HashMap<&str, usize> = HashMap::new();
    for (t, b, e) in &toks {
        let set: HashSet<&str> = t.iter().chain(b).chain(e).map(String::as_str).collect();
        for w in set {
            *df.entry(w).or_default() += 1;
        }
    }
    let n = docs.len() as f32;
    let mut hits: Vec<Hit> = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        if kind.is_some_and(|k| d.kind != k) {
            continue;
        }
        let (t, b, e) = &toks[i];
        let mut score = 0.0;
        let mut matched_terms = 0;
        for (term, weight) in &q {
            let tf_t = t.iter().filter(|x| *x == term).count() as f32;
            let tf_e = e.iter().filter(|x| *x == term).count() as f32;
            let tf_b = b.iter().filter(|x| *x == term).count() as f32;
            let tf = tf_t * 4.0 + tf_e * 3.0 + tf_b;
            if tf == 0.0 {
                continue;
            }
            if *weight >= 1.0 {
                matched_terms += 1;
            }
            let idf = ((n - *df.get(term.as_str()).unwrap_or(&0) as f32 + 0.5) / (*df.get(term.as_str()).unwrap_or(&0) as f32 + 0.5) + 1.0).ln();
            let len_norm = 1.0 - 0.5 + 0.5 * (b.len() as f32 + 4.0) / 60.0;
            score += weight * idf * (tf * 2.2) / (tf + 1.2 * len_norm);
        }
        if score > 0.0 {
            // reward covering more of the distinct query words
            let distinct = q.iter().filter(|(_, w)| *w >= 1.0).count().max(1) as f32;
            score *= 1.0 + 0.6 * (matched_terms as f32 / distinct);
            let kind_bias = match d.kind {
                "doc" | "rule" | "recipe" | "adr" | "glossary" => 1.15,
                "asset" | "lint" | "type" | "command" => 1.05,
                _ => 0.9,
            };
            hits.push(Hit { doc: d, score: score * kind_bias, fragment: fragment(d, &q) });
        }
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    // diversity: at most 4 per kind unless a kind filter is on
    let mut per: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::new();
    for h in hits {
        let c = per.entry(h.doc.kind).or_default();
        if kind.is_none() && *c >= 4 {
            continue;
        }
        *c += 1;
        out.push(h);
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// The most query-dense window (<= 7 lines) of a doc body.
fn fragment(d: &Doc, q: &[(String, f32)]) -> String {
    let lines: Vec<&str> = d.body.lines().collect();
    if lines.len() <= 7 {
        return d.body.trim().to_string();
    }
    let hits: Vec<f32> = lines.iter().map(|l| tokens_snake(l).iter().map(|t| q.iter().filter(|(x, _)| x == t).map(|(_, w)| *w).sum::<f32>()).sum()).collect();
    let mut best = (0usize, -1.0f32);
    for s in 0..=lines.len() - 7 {
        let sc: f32 = hits[s..s + 7].iter().sum();
        if sc > best.1 {
            best = (s, sc);
        }
    }
    lines[best.0..best.0 + 7].join("\n").trim_end().to_string()
}

/// Formats hits as text (kind, title, fragment, where to read more).
pub fn render(hits: &[Hit], query: &str) -> String {
    if hits.is_empty() {
        return format!("nothing found for '{query}'. Try other words, or `red_engine2 describe` for the topic list.\n");
    }
    let mut out = String::new();
    for h in hits {
        out.push_str(&format!("[{}] {}\n", h.doc.kind, h.doc.title));
        for l in h.fragment.lines().take(7) {
            out.push_str(&format!("    {}\n", l.chars().take(150).collect::<String>()));
        }
        out.push_str(&format!("    -> {}\n", h.doc.loc));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmds() -> Value {
        serde_json::json!([{"name": "lint", "about": "Static map checker", "args": []}, {"name": "scatter", "about": "Seeded random planting", "args": []}])
    }

    fn top(query: &str, kind: &str) -> String {
        let docs = corpus(&cmds());
        let hits = search(&docs, query, Some(kind), 3);
        hits.first().map(|h| format!("{} | {} | {}", h.doc.title, h.doc.loc, h.fragment)).unwrap_or_default()
    }

    #[test]
    fn finds_the_right_documentation_for_natural_questions() {
        let t = top("why does my door block the player", "doc");
        assert!(t.to_lowercase().contains("door") && t.to_lowercase().contains("header"), "{t}");
        let t = top("how do stairs connect two floors", "doc");
        assert!(t.to_lowercase().contains("stair"), "{t}");
        assert!(top("open stairwell fall railing", "lint").contains("drop"), "{}", top("open stairwell fall railing", "lint"));
    }

    #[test]
    fn finds_decisions_and_glossary_terms() {
        // ADR 0014 also discusses the fixed timestep, so 0005 must be among the top hits, not necessarily first.
        let docs = corpus(&cmds());
        let hits = search(&docs, "why fixed timestep interpolation", Some("adr"), 3);
        assert!(hits.iter().any(|h| h.doc.title.contains("Fixed 1/60")), "{:?}", hits.iter().map(|h| &h.doc.title).collect::<Vec<_>>());
        let t = top("headless server multiplayer physics extract", "adr");
        assert!(t.contains("Headless"), "{t}");
        let t = top("body band ground snap", "glossary").to_lowercase();
        assert!(t.contains("ground snap") || t.contains("body band"), "{t}");
    }

    #[test]
    fn every_adr_is_registered() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/adr");
        for e in std::fs::read_dir(&dir).expect("docs/adr exists") {
            let name = e.unwrap().file_name().to_string_lossy().to_string();
            if !name.ends_with(".md") || name == "README.md" {
                continue;
            }
            assert!(ADRS.iter().any(|(f, _)| *f == name), "docs/adr/{name} is not in search.rs ADRS (add it, and a row in docs/adr/README.md)");
            assert!(ADR_INDEX.contains(&name), "docs/adr/{name} is not listed in docs/adr/README.md");
        }
        for (f, text) in ADRS {
            assert!(text.contains("\nStatus: "), "{f}: needs a `Status:` line");
            for h in ["## Context", "## Decision", "## Consequences"] {
                assert!(text.contains(h), "{f}: missing `{h}`");
            }
        }
    }

    #[test]
    fn finds_assets_by_meaning_and_tag() {
        assert!(top("apple", "asset").contains("apple"));
        assert!(top("folding metal chair", "asset").contains("chair_folding"), "{}", top("folding metal chair", "asset"));
        assert!(top("cash register checkout", "asset").contains("cash_register") || top("cash register checkout", "asset").contains("checkout"));
    }

    #[test]
    fn finds_source_symbols() {
        let t = top("player ground height stairs ramp", "src");
        assert!(t.contains("ground_height") || t.contains("StairsRamp") || t.contains("height"), "{t}");
    }

    #[test]
    fn stemming_and_synonyms_bridge_wording() {
        assert_eq!(stem("stairs"), "stair");
        assert_eq!(stem("doors"), "door");
        let t = top("reusable template object", "type");
        assert!(t.contains("prefab"), "{t}");
    }
}
