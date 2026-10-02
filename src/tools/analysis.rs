//! Analysis notes: `red_engine2 analysis new|list|digest`. `docs/analysis/` keeps what people (and AIs) who built something on the engine reported, and what was
//! done about it, so the next builder learns from them. It used to be a folder the CLI could neither list nor add to; now the notes are embedded
//! (`search --kind analysis`), listed by date, and a pasted report becomes a note in one command.
//!
//! `digest` is the cross-game half: on its own, each note only tells the *next* builder what *one* game's build taught us. It cannot show that the same
//! friction keeps recurring across games, because nothing reads more than one note at a time. `digest` pulls every note's feedback rows (its
//! `| # | feedback | status | what now happens |`-shaped table, where present), matches each row's text to the engine feature it is about using the same
//! word-overlap scoring `context` already uses to answer "what do I need to know to change this?" (`super::context::resolve`), and groups rows by feature,
//! ranked by how many *different* notes raised something about it. The result lives in `docs/analysis/README.md`, generated exactly like `docs/adr/README.md`'s
//! index (markers, `--write`, a `preflight` staleness check) — `list()` already excluded `README.md` from being parsed as a note, which is what made this safe
//! to add without touching the nine notes already there.

use super::context;
use super::features::Feature;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One note, as far as a listing needs to know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// The file name (`2026-09-28-trigger-happy-feedback.md`).
    pub file: String,
    /// The date in the file name.
    pub date: String,
    /// The title line without the `#` and without a trailing `(date)`.
    pub title: String,
    /// The first sentence of the first paragraph.
    pub summary: String,
}

/// Reads one note; `None` when the file name does not start with a date.
pub fn parse(file: &str, text: &str) -> Option<Note> {
    let date = file.get(..10).filter(|d| {
        d.len() == 10 && d.as_bytes()[4] == b'-' && d.as_bytes()[7] == b'-' && d.bytes().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    })?;
    let text = text.replace("\r\n", "\n");
    let mut lines = text.lines();
    let h1 = lines.next().unwrap_or("").trim_start_matches('#').trim();
    let title = h1.strip_suffix(')').and_then(|t| t.rsplit_once(" (")).map_or(h1, |(t, _)| t).trim().to_string();
    let para = lines.skip_while(|l| l.trim().is_empty()).take_while(|l| !l.trim().is_empty()).collect::<Vec<_>>().join(" ");
    let summary = match para.find(". ") {
        Some(i) if i > 20 => para[..=i].to_string(),
        _ => para.chars().take(160).collect(),
    };
    Some(Note { file: file.to_string(), date: date.to_string(), title, summary })
}

/// The notes under `root/docs/analysis`, oldest first.
pub fn list(root: &Path) -> Vec<Note> {
    let dir = root.join("docs/analysis");
    let mut notes: Vec<Note> = std::fs::read_dir(dir.clone())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.ends_with(".md") && n != "README.md")
                .filter_map(|n| parse(&n, &std::fs::read_to_string(dir.join(&n)).unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    notes.sort_by(|a, b| a.file.cmp(&b.file));
    notes
}

/// One line per note.
pub fn render_list(notes: &[Note]) -> String {
    notes.iter().map(|n| format!("{}  {}: {}\n", n.date, n.title, n.summary)).collect()
}

/// The text of a new note.
fn template(title: &str, date: &str) -> String {
    format!(
        "# {title} ({date})\n\nWho built what, on which engine commit, and what they reported (one paragraph).\n\n## What worked\n\n## What cost time, ranked\n\n## What was done about it\n\n## Not done, and why\n"
    )
}

/// Creates `docs/analysis/<date>-<slug>.md` (from `body`, a pasted report, or the template) and returns its path.
pub fn new_note(root: &Path, title: &str, body: Option<&str>, date: &str) -> Result<PathBuf, String> {
    let title = title.trim();
    let slug = super::adr::slugify(title, 56);
    if slug.is_empty() {
        return Err("give the note a title: `red_engine2 analysis new \"What building X taught us\"`".into());
    }
    let path = root.join("docs/analysis").join(format!("{date}-{slug}.md"));
    if path.exists() {
        return Err(format!("{} already exists: edit it, or pick another title", path.display()));
    }
    let text = match body {
        Some(b) if b.trim_start().starts_with("# ") => b.replace("\r\n", "\n"),
        Some(b) => format!("# {title} ({date})\n\n{}", b.replace("\r\n", "\n")),
        None => template(title, date),
    };
    std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(|e| e.to_string())?;
    std::fs::write(&path, text.trim_end().to_string() + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Markers around the generated digest in `docs/analysis/README.md` (mirrors `adr::INDEX_BEGIN`/`INDEX_END`).
pub const DIGEST_BEGIN: &str = "<!-- analysis-index:begin -->";
/// See [`DIGEST_BEGIN`].
pub const DIGEST_END: &str = "<!-- analysis-index:end -->";

const DIGEST_TEMPLATE: &str = "# Analysis notes\n\nWhat builders (people and AIs) reported about using Red, and what was done about it (`red_engine2 analysis new|list`). The table below is generated: it groups every note's feedback by the engine feature it matches, ranked by how many different games raised something about it. Regenerate it with `red_engine2 analysis digest --write`.\n\n<!-- analysis-index:begin -->\n<!-- analysis-index:end -->\n";

/// The feedback rows of one markdown table in `body` (a `|`-delimited header line followed by a `---`-style
/// separator line, then data rows until a blank or non-table line). Column names are not hard-coded — the whole
/// row's cells, minus a leading sequence-number column if there is one, are joined into one line of text, which is
/// all `digest` needs to match it against a feature. Finds every such table in the note, not just the first.
fn table_rows(body: &str) -> Vec<String> {
    let lines: Vec<&str> = body.lines().collect();
    let mut rows = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        let (header, sep) = (lines[i].trim(), lines[i + 1].trim());
        let is_sep = sep.starts_with('|') && sep.len() > 1 && sep.chars().all(|c| "|-: ".contains(c)) && sep.contains('-');
        if header.starts_with('|') && is_sep {
            let mut j = i + 2;
            while j < lines.len() && lines[j].trim().starts_with('|') {
                let mut cells: Vec<&str> = lines[j].trim().trim_matches('|').split('|').map(str::trim).collect();
                if cells.len() > 1 && !cells[0].is_empty() && cells[0].chars().all(|c| c.is_ascii_digit()) {
                    cells.remove(0);
                }
                let text = cells.join(" — ");
                if !text.is_empty() {
                    rows.push(text);
                }
                j += 1;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    rows
}

/// Every feedback row across every note under `root/docs/analysis`, each tagged with the note it came from. A note
/// with no table contributes one row built from its title and summary, so nothing is silently dropped.
fn all_rows(root: &Path) -> Vec<(Note, String)> {
    let dir = root.join("docs/analysis");
    list(root)
        .into_iter()
        .flat_map(|n| {
            let body = std::fs::read_to_string(dir.join(&n.file)).unwrap_or_default();
            let rows = table_rows(&body);
            if rows.is_empty() {
                vec![(n.clone(), format!("{} — {}", n.title, n.summary))]
            } else {
                rows.into_iter().map(|r| (n.clone(), r)).collect()
            }
        })
        .collect()
}

/// One feedback row: the note it came from, and the row's text.
type Row = (Note, String);
/// One digest section: the feature every row in it matches (`None` for "no feature matched"), and its rows.
type Group = (Option<String>, Vec<Row>);

/// Every feedback row, grouped by the feature its text best matches (`context::resolve`'s existing word-overlap
/// scoring — no new text matching here), most-cross-game-first (by how many *distinct* notes landed in that
/// group, not raw row count, so one chatty note cannot dominate). Rows matching no feature are kept under `None`,
/// listed last, never dropped.
pub fn digest(root: &Path, features: &[Feature]) -> Vec<Group> {
    let mut groups: std::collections::BTreeMap<String, Vec<Row>> = std::collections::BTreeMap::new();
    let mut unassigned: Vec<Row> = Vec::new();
    for (note, text) in all_rows(root) {
        // `context::resolve` matches by substring (`hay.contains(word)`), built for short, deliberate CLI queries —
        // handed raw prose, a short word ("a", "and", "its") would substring-match almost any feature's text and
        // swamp the real signal, so only words long enough to be meaningful are offered to it.
        let words: Vec<String> = text.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| w.len() >= 4).map(str::to_string).collect();
        match context::resolve(features, &words).first() {
            Some(f) => groups.entry(f.name.clone()).or_default().push((note, text)),
            None => unassigned.push((note, text)),
        }
    }
    let note_count = |rows: &[Row]| rows.iter().map(|(n, _)| n.file.as_str()).collect::<BTreeSet<_>>().len();
    let mut out: Vec<Group> = groups.into_iter().map(|(k, v)| (Some(k), v)).collect();
    out.sort_by(|a, b| note_count(&b.1).cmp(&note_count(&a.1)).then_with(|| a.0.cmp(&b.0)));
    if !unassigned.is_empty() {
        out.push((None, unassigned));
    }
    out
}

/// The generated digest table: one section per feature (most cross-game first), each listing its matching rows.
pub fn render_digest(groups: &[Group]) -> String {
    let mut s = String::new();
    for (feature, rows) in groups {
        let notes = rows.iter().map(|(n, _)| n.file.as_str()).collect::<BTreeSet<_>>().len();
        let heading = feature.as_deref().unwrap_or("unassigned (no feature matched)");
        s.push_str(&format!("### {heading} — {notes} note(s)\n\n"));
        for (note, text) in rows {
            s.push_str(&format!("- {} ({}): {}\n", note.date, note.file, text.replace('\n', " ")));
        }
        s.push('\n');
    }
    if s.is_empty() {
        s.push_str("_no notes yet._\n");
    }
    s.trim_end().to_string() + "\n"
}

/// Replaces the region between the digest markers of `readme` with `table`. `Err` if the markers are missing.
pub fn sync_digest(readme: &str, table: &str) -> Result<String, String> {
    let (Some(a), Some(b)) = (readme.find(DIGEST_BEGIN), readme.find(DIGEST_END)) else {
        return Err(format!("docs/analysis/README.md needs a {DIGEST_BEGIN} ... {DIGEST_END} region to hold the generated digest"));
    };
    if b < a {
        return Err("analysis-index:end comes before analysis-index:begin".into());
    }
    let nl = if readme.contains("\r\n") { "\r\n" } else { "\n" };
    Ok(format!("{}{}{}{}", &readme[..a + DIGEST_BEGIN.len()], nl, table.replace('\n', nl), &readme[b..]))
}

/// The text between the digest markers (trimmed), if present.
pub fn current_digest(readme: &str) -> Option<String> {
    let (a, b) = (readme.find(DIGEST_BEGIN)?, readme.find(DIGEST_END)?);
    (a < b).then(|| readme[a + DIGEST_BEGIN.len()..b].replace("\r\n", "\n").trim().to_string())
}

/// Rewrites the generated digest of `root/docs/analysis/README.md`, creating the file from a small template the
/// first time it is run. Returns whether the file changed.
pub fn write_digest(root: &Path, features: &[Feature]) -> Result<bool, String> {
    let path = root.join("docs/analysis/README.md");
    let existing = std::fs::read_to_string(&path).ok();
    let readme = existing.clone().unwrap_or_else(|| DIGEST_TEMPLATE.to_string());
    let table = render_digest(&digest(root, features));
    let new = sync_digest(&readme, &table)?;
    if existing.as_deref() == Some(new.as_str()) {
        return Ok(false);
    }
    std::fs::write(&path, new).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// What's wrong with `docs/analysis/README.md`'s digest (stale or missing), if anything.
pub fn check_digest(root: &Path, features: &[Feature]) -> Vec<String> {
    let readme = std::fs::read_to_string(root.join("docs/analysis/README.md")).unwrap_or_default().replace("\r\n", "\n");
    let want = render_digest(&digest(root, features));
    match current_digest(&readme) {
        None => vec!["docs/analysis/README.md needs a <!-- analysis-index:begin --> ... <!-- analysis-index:end --> region (the generated digest)".to_string()],
        Some(have) if have != want.trim() => vec!["docs/analysis/README.md: the digest is stale; run `red_engine2 analysis digest --write`".to_string()],
        Some(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_is_dated_by_its_file_name_and_summarised_by_its_first_sentence() {
        let n = parse("2026-09-24-cheddar-feedback.md", "# What building Cheddar taught us about Red (2026-09-24)\n\nAn AI built the game Cheddar on Red and had a very hard time. It wrote down more.\n\n## x\n").unwrap();
        assert_eq!((n.date.as_str(), n.title.as_str()), ("2026-09-24", "What building Cheddar taught us about Red"));
        assert_eq!(n.summary, "An AI built the game Cheddar on Red and had a very hard time.");
        assert!(parse("notes.md", "# x").is_none() && parse("2026-9-24-x.md", "# x").is_none());
    }

    #[test]
    fn new_notes_come_from_the_template_or_a_pasted_report_and_never_overwrite() {
        let root = std::env::temp_dir().join(format!("re2_analysis_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let p = new_note(&root, "Trigger Happy feedback", None, "2026-09-28").unwrap();
        assert!(p.ends_with("2026-09-28-trigger-happy-feedback.md"));
        assert!(std::fs::read_to_string(&p).unwrap().starts_with("# Trigger Happy feedback (2026-09-28)"));
        assert!(new_note(&root, "Trigger Happy feedback", None, "2026-09-28").unwrap_err().contains("already exists"));
        let q = new_note(&root, "Another report", Some("Pasted text.\n\nMore."), "2026-09-28").unwrap();
        assert!(std::fs::read_to_string(&q).unwrap().starts_with("# Another report (2026-09-28)\n\nPasted text."));
        let listed = list(&root);
        assert_eq!(listed.len(), 2);
        assert!(render_list(&listed).contains("2026-09-28  Another report: Pasted text."), "{}", render_list(&listed));
        assert!(new_note(&root, "???", None, "2026-09-28").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn table_rows_finds_every_table_strips_a_leading_number_column_and_ignores_prose() {
        let body = "Some prose with a | in it is not a table.\n\n| # | feedback | status |\n|---|---|---|\n| 1 | spawns felt cramped | fixed |\n| 2 | no sprint key | fixed |\n\nMore prose.\n\n| thing | note |\n| --- | --- |\n| second table | also found |\n";
        let rows = table_rows(body);
        assert_eq!(rows, vec!["spawns felt cramped — fixed", "no sprint key — fixed", "second table — also found"]);
        assert!(table_rows("just a paragraph, no tables here.").is_empty());
    }

    fn fixture_features() -> Vec<Feature> {
        vec![
            Feature {
                name: "net_client".to_string(),
                summary: "spawns and connection handling for joining players".to_string(),
                files: vec![],
                tests: vec![],
                commands: vec![],
                docs: vec![],
                depends_on: vec![],
            },
            Feature {
                name: "audio".to_string(),
                summary: "music and sound effect playback".to_string(),
                files: vec![],
                tests: vec![],
                commands: vec![],
                docs: vec![],
                depends_on: vec![],
            },
        ]
    }

    #[test]
    fn digest_groups_rows_by_feature_ranked_by_distinct_notes_and_keeps_unmatched_rows_separate() {
        let root = std::env::temp_dir().join(format!("re2_analysis_digest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/analysis")).unwrap();
        std::fs::write(
            root.join("docs/analysis/2026-09-24-a-feedback.md"),
            "# A feedback (2026-09-24)\n\nBuilding A taught us things.\n\n| # | feedback | status |\n|---|---|---|\n| 1 | spawns felt cramped on connection | fixed |\n",
        )
        .unwrap();
        std::fs::write(root.join("docs/analysis/2026-09-25-b-feedback.md"), "# B feedback (2026-09-25)\n\nBuilding B also hit a connection spawn issue.\n")
            .unwrap();
        std::fs::write(
            root.join("docs/analysis/2026-09-26-c-feedback.md"),
            "# C feedback (2026-09-26)\n\nThe documentation formatting felt inconsistent across chapters.\n",
        )
        .unwrap();
        let groups = digest(&root, &fixture_features());
        assert_eq!(groups.len(), 2, "{groups:?}");
        let (feature, rows) = &groups[0];
        assert_eq!(feature.as_deref(), Some("net_client"), "the two-note group ranks first: {groups:?}");
        assert_eq!(rows.len(), 2);
        assert_eq!(groups[1].0, None, "a row about nothing feature-shaped lands in the unassigned bucket");
        let rendered = render_digest(&groups);
        assert!(rendered.contains("### net_client — 2 note(s)") && rendered.contains("### unassigned"), "{rendered}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_digest_file_is_created_regenerated_idempotently_and_keeps_its_preamble() {
        let root = std::env::temp_dir().join(format!("re2_analysis_digestfile_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/analysis")).unwrap();
        assert_eq!(
            check_digest(&root, &fixture_features()),
            vec!["docs/analysis/README.md needs a <!-- analysis-index:begin --> ... <!-- analysis-index:end --> region (the generated digest)".to_string()]
        );

        assert!(write_digest(&root, &fixture_features()).unwrap(), "the first write creates the file");
        assert!(check_digest(&root, &fixture_features()).is_empty());
        assert!(!write_digest(&root, &fixture_features()).unwrap(), "a second write with nothing new changed nothing");

        let path = root.join("docs/analysis/README.md");
        let with_preamble = format!("# Analysis notes\n\nHand-written preamble.\n\n{}\n{}\n", DIGEST_BEGIN, DIGEST_END);
        std::fs::write(&path, &with_preamble).unwrap();
        std::fs::write(root.join("docs/analysis/2026-09-24-a-feedback.md"), "# A feedback (2026-09-24)\n\nSomething happened with audio playback and music.\n")
            .unwrap();
        assert!(write_digest(&root, &fixture_features()).unwrap());
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("Hand-written preamble.") && after.contains("audio"), "{after}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
