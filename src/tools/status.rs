//! `red_engine2 status`: the handoff file and the facts that keep docs honest.
//!
//! Two problems this solves, both reported by an AI that resumed a game project after its terminal died:
//!
//! * **No handoff.** The only record of "what is done / in flight / failing / next" was a 13 MB session transcript.
//!   `status` prints one screen to resume from: repo facts, the last commits, uncommitted files and the project's
//!   `STATUS.md` (`--init` creates it, `--note "..." --section next` appends a dated bullet).
//! * **Confidently stale docs.** `CLAUDE.md` said multiplayer "is not built" and "95+ tests" long after both were false.
//!   [`facts_block`] derives a short block (binaries, features, ADR count, suites, maps) from the files themselves;
//!   `status --sync-docs CLAUDE.md` rewrites the region between the `facts` markers and a test
//!   (`tests/docs_fresh.rs`) fails when the committed text differs from the derived one.
//!
//! Everything here reads plain files (plus `git` when it is installed); nothing needs a window, a GPU or a network.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Marker that opens the derived region inside a doc.
pub const FACTS_BEGIN: &str = "<!-- facts:begin -->";
/// Marker that closes the derived region inside a doc.
pub const FACTS_END: &str = "<!-- facts:end -->";

/// The sections a `STATUS.md` has, as `(key for --section, heading)`.
pub const SECTIONS: [(&str, &str); 5] =
    [("now", "Now (in flight)"), ("done", "Done"), ("next", "Next"), ("blocked", "Failing / blocked"), ("notes", "Decisions & gotchas")];

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Package name, `[[bin]]` names and `[features]` keys of a Cargo.toml (a line scan, enough for the repo's own layout).
fn parse_cargo(text: &str) -> (String, Vec<String>, Vec<String>) {
    let (mut name, mut bins, mut features) = (String::new(), Vec::new(), Vec::new());
    let mut table = String::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            table = l.to_string();
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim().trim_matches('"'));
        match table.as_str() {
            "[package]" if k == "name" => name = v.to_string(),
            "[[bin]]" if k == "name" => bins.push(v.to_string()),
            "[features]" if !k.is_empty() && !k.starts_with('#') => features.push(k.to_string()),
            _ => {}
        }
    }
    (name, bins, features)
}

/// Executable names a repo builds: explicit `[[bin]]`s, `src/main.rs` (the package name), `src/bin/*.rs` and `src/bin/*/main.rs`.
pub fn binaries(root: &Path) -> Vec<String> {
    let (name, mut bins, _) = parse_cargo(&read(&root.join("Cargo.toml")));
    if root.join("src/main.rs").exists() && !name.is_empty() {
        bins.push(name);
    }
    if let Ok(rd) = std::fs::read_dir(root.join("src/bin")) {
        for e in rd.flatten() {
            let p = e.path();
            let n = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if (p.is_file() && p.extension().is_some_and(|x| x == "rs")) || p.join("main.rs").exists() {
                bins.push(if p.is_dir() { e.file_name().to_string_lossy().to_string() } else { n });
            }
        }
    }
    bins.sort();
    bins.dedup();
    bins
}

fn count_files(dir: &Path, ext: &str) -> usize {
    std::fs::read_dir(dir).map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == ext)).count()).unwrap_or(0)
}

/// `(count, latest "NNNN title")` of the ADRs in `docs/adr/`.
fn adrs(root: &Path) -> (usize, String) {
    let mut names: Vec<String> = std::fs::read_dir(root.join("docs/adr"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.len() > 5 && n[..4].chars().all(|c| c.is_ascii_digit()) && n.ends_with(".md"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let latest = names.last().map(|n| n.trim_end_matches(".md").replacen('-', " ", 1).replace('-', " ")).unwrap_or_default();
    (names.len(), latest)
}

/// The derived, deterministic facts block (no git, no clock): what a doc may safely state about the repo.
pub fn facts_block(root: &Path) -> String {
    let (name, _, features) = parse_cargo(&read(&root.join("Cargo.toml")));
    let bins = binaries(root);
    let (n_adr, latest_adr) = adrs(root);
    let mut s = String::new();
    s.push_str(&format!(
        "- Crate `{}`; binaries: {}.\n",
        if name.is_empty() { "?" } else { &name },
        if bins.is_empty() { "none".to_string() } else { bins.iter().map(|b| format!("`{b}`")).collect::<Vec<_>>().join(", ") }
    ));
    if !features.is_empty() {
        s.push_str(&format!("- Cargo features: {}.\n", features.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")));
    }
    s.push_str(&format!(
        "- {} integration test suites (`tests/`), {} recipes (`recipes/`), {} example maps (`examples/`); {} ADRs (latest: {}). Test *counts* are not stated here: run `scripts/dev test`.\n",
        count_files(&root.join("tests"), "rs"),
        count_files(&root.join("recipes"), "json"),
        count_files(&root.join("examples"), "json"),
        n_adr,
        if latest_adr.is_empty() { "-".to_string() } else { latest_adr }
    ));
    s
}

/// Replaces the region between the facts markers of `doc` with `block`. `Err` if the markers are missing.
pub fn sync_facts(doc: &str, block: &str) -> Result<String, String> {
    let (Some(a), Some(b)) = (doc.find(FACTS_BEGIN), doc.find(FACTS_END)) else {
        return Err(format!("the doc needs a {FACTS_BEGIN} ... {FACTS_END} region to hold derived facts"));
    };
    if b < a {
        return Err("facts:end comes before facts:begin".into());
    }
    let nl = if doc.contains("\r\n") { "\r\n" } else { "\n" };
    let body = block.replace('\n', nl);
    Ok(format!("{}{}{}{}", &doc[..a + FACTS_BEGIN.len()], nl, body, &doc[b..]))
}

/// The text between the facts markers of `doc` (trimmed), if present.
pub fn current_facts(doc: &str) -> Option<String> {
    let (a, b) = (doc.find(FACTS_BEGIN)?, doc.find(FACTS_END)?);
    (a < b).then(|| doc[a + FACTS_BEGIN.len()..b].replace("\r\n", "\n").trim().to_string())
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Today's date `YYYY-MM-DD` (UTC) without a date crate: civil-from-days on the Unix clock.
pub fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The path of a project's handoff file.
pub fn status_path(root: &Path) -> PathBuf {
    root.join("STATUS.md")
}

fn template(project: &str) -> String {
    let mut s = format!(
        "# STATUS — {project}\n\n_Handoff file for whoever (human or AI) resumes this work. Keep it short and current: update it at every checkpoint with\n`red_engine2 status --note \"what changed\" --section done|now|next|blocked|notes`._\n\n"
    );
    for (_, h) in SECTIONS {
        s.push_str(&format!("## {h}\n\n"));
    }
    s
}

/// Creates `STATUS.md` from the template if it does not exist; returns its path.
pub fn init(root: &Path) -> Result<PathBuf, String> {
    let p = status_path(root);
    if p.exists() {
        return Ok(p);
    }
    let (name, _, _) = parse_cargo(&read(&root.join("Cargo.toml")));
    let project = if name.is_empty() { root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".into()) } else { name };
    std::fs::write(&p, template(&project)).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p)
}

/// Appends `- YYYY-MM-DD: text` under the `section` heading of `STATUS.md` (creating the file and the heading as needed).
pub fn add_note(root: &Path, section: &str, text: &str) -> Result<PathBuf, String> {
    let Some((_, heading)) = SECTIONS.iter().find(|(k, _)| *k == section) else {
        return Err(format!("unknown section '{section}' (use one of: {})", SECTIONS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")));
    };
    let p = init(root)?;
    let doc = read(&p).replace("\r\n", "\n");
    let head = format!("## {heading}");
    let line = format!("- {}: {}", today(), text.trim());
    let out = match doc.find(&head) {
        Some(i) => {
            // Insert after the last non-blank line of that section (before the next `## ` or the end).
            let body_start = i + head.len();
            let next = doc[body_start..].find("\n## ").map(|n| body_start + n + 1).unwrap_or(doc.len());
            let body = doc[body_start..next].trim_end();
            format!("{}{}\n{}\n\n{}", &doc[..body_start], body, line, doc[next..].trim_start_matches('\n'))
        }
        None => format!("{}\n{head}\n\n{line}\n", doc.trim_end()),
    };
    std::fs::write(&p, out.trim_end().to_string() + "\n").map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p)
}

/// One-screen resume report: derived facts, git position, recent commits, uncommitted files and `STATUS.md`.
pub fn render(root: &Path) -> String {
    let mut s = String::from("== facts ==\n");
    s.push_str(&facts_block(root));
    if let Some(branch) = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        let head = git(root, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
        let dirty = git(root, &["status", "--short"]).unwrap_or_default();
        let files: Vec<&str> = dirty.lines().collect();
        s.push_str(&format!(
            "\n== git ==\nbranch {branch} @ {head}, {} uncommitted file(s){}\n",
            files.len(),
            if files.is_empty() { "" } else { " (commit or note them before you stop)" }
        ));
        for f in files.iter().take(12) {
            s.push_str(&format!("  {f}\n"));
        }
        if files.len() > 12 {
            s.push_str(&format!("  ... and {} more\n", files.len() - 12));
        }
        if let Some(log) = git(root, &["log", "--oneline", "-n", "6"]) {
            s.push_str(&format!("recent commits:\n{}\n", log.lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")));
        }
    }
    let p = status_path(root);
    s.push_str("\n== STATUS.md ==\n");
    if p.exists() {
        s.push_str(read(&p).trim_end());
        s.push('\n');
    } else {
        s.push_str("(none yet: `red_engine2 status --init` creates it; then `--note \"...\" --section next` as you work)\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("re2_status_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src/bin")).unwrap();
        std::fs::create_dir_all(d.join("docs/adr")).unwrap();
        std::fs::create_dir_all(d.join("tests")).unwrap();
        std::fs::write(
            d.join("Cargo.toml"),
            "[package]\nname = \"demo\"\n\n[features]\ndefault = [\"gfx\"]\ngfx = []\n\n[[bin]]\nname = \"gamebin\"\npath = \"src/bin/g.rs\"\n",
        )
        .unwrap();
        std::fs::write(d.join("src/main.rs"), "fn main(){}").unwrap();
        std::fs::write(d.join("src/bin/server.rs"), "fn main(){}").unwrap();
        std::fs::write(d.join("docs/adr/0001-first-thing.md"), "# x").unwrap();
        std::fs::write(d.join("docs/adr/0002-second-thing.md"), "# x").unwrap();
        std::fs::write(d.join("tests/a.rs"), "").unwrap();
        d
    }

    #[test]
    fn facts_come_from_the_files() {
        let d = tmp("facts");
        let f = facts_block(&d);
        assert!(f.contains("`demo`") && f.contains("`gamebin`") && f.contains("`server`"), "{f}");
        assert!(f.contains("`default`") && f.contains("`gfx`"), "{f}");
        assert!(f.contains("1 integration test suites") && f.contains("2 ADRs (latest: 0002 second thing)"), "{f}");
    }

    #[test]
    fn sync_replaces_only_the_marked_region_and_is_idempotent() {
        let doc = format!("# T\nkeep me\n{FACTS_BEGIN}\nstale line\n{FACTS_END}\ntail\n");
        let once = sync_facts(&doc, "- fresh\n").unwrap();
        assert!(once.contains("keep me") && once.contains("tail") && !once.contains("stale"), "{once}");
        assert_eq!(current_facts(&once).as_deref(), Some("- fresh"));
        assert_eq!(sync_facts(&once, "- fresh\n").unwrap(), once);
        assert!(sync_facts("no markers", "x").is_err());
    }

    #[test]
    fn notes_land_under_their_section_in_order() {
        let d = tmp("notes");
        add_note(&d, "done", "wired the thing").unwrap();
        add_note(&d, "done", "second thing").unwrap();
        add_note(&d, "next", "ship it").unwrap();
        let t = std::fs::read_to_string(status_path(&d)).unwrap();
        let done = t.find("## Done").unwrap();
        let next = t.find("## Next").unwrap();
        let first = t.find("wired the thing").unwrap();
        let second = t.find("second thing").unwrap();
        assert!(done < first && first < second && second < next, "{t}");
        assert!(t.find("ship it").unwrap() > next, "{t}");
        assert!(add_note(&d, "bogus", "x").is_err());
        assert!(render(&d).contains("== STATUS.md ==") && render(&d).contains("ship it"));
    }

    #[test]
    fn today_is_a_plausible_iso_date() {
        let t = today();
        assert_eq!(t.len(), 10);
        assert!(t.starts_with("20") && &t[4..5] == "-" && &t[7..8] == "-", "{t}");
    }
}
