//! Analysis notes: `red_engine2 analysis new|list`. `docs/analysis/` keeps what people (and AIs) who built something on the engine reported, and what was done about
//! it, so the next builder learns from them. It used to be a folder the CLI could neither list nor add to; now the notes are embedded (`search --kind analysis`),
//! listed by date, and a pasted report becomes a note in one command.

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
}
