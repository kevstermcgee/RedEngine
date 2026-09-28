//! Architecture decision records as data: `red_engine2 adr new|list|index`, and the checks behind them (ADR 2026-09-28-generated-bookkeeping).
//!
//! An ADR is `docs/adr/<id>-<slug>.md`. Its id is either a legacy four-digit number (`0057`) or the date it was written
//! (`2026-09-28`): a date cannot collide with the ADR another branch is writing at the same moment, a "next number" always did. The file
//! starts with a title line, a `Status:` line and a `Summary:` line (one sentence, it becomes the index row), then `## Context`,
//! `## Decision` and `## Consequences`. Nothing else is kept in step by hand: `docs/adr/README.md` holds a table generated from the
//! files (between the `adr-index` markers) and the library embeds the folder at build time (`build.rs`).

use std::path::{Path, PathBuf};

/// Opens the generated table inside `docs/adr/README.md`.
pub const INDEX_BEGIN: &str = "<!-- adr-index:begin -->";
/// Closes the generated table inside `docs/adr/README.md`.
pub const INDEX_END: &str = "<!-- adr-index:end -->";
/// The `Summary:` text a new ADR starts with; [`check`] rejects an ADR that still has it.
pub const SUMMARY_PLACEHOLDER: &str = "TODO one sentence for the index";

/// One decision record, as far as the index needs to know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adr {
    /// The file name (`0057-avatars-for-every-body.md`).
    pub file: String,
    /// What other documents call it: `0057`, or the whole `2026-09-28-lag-comp` for a dated one.
    pub id: String,
    /// The title without the id (`The avatar pool covers every body somebody can wear`).
    pub title: String,
    /// The `Status:` line (`accepted`, `superseded by 0016 (...)`).
    pub status: String,
    /// The `Summary:` line.
    pub summary: String,
}

/// Whether `stem` starts with a date (`2026-09-28`).
fn starts_with_date(stem: &str) -> bool {
    let b = stem.as_bytes();
    b.len() >= 10 && [0, 1, 2, 3, 5, 6, 8, 9].iter().all(|&i| b[i].is_ascii_digit()) && b[4] == b'-' && b[7] == b'-'
}

/// A dated id: a date, a dash, and a slug (`2026-09-28-lag-comp`).
fn is_dated(stem: &str) -> bool {
    starts_with_date(stem) && stem.len() > 11 && stem.as_bytes()[10] == b'-'
}

/// The id of an ADR file: its four-digit number, or the whole stem of a dated one. `None` for a file that is neither (a date without a slug is not an id).
pub fn id_of(file: &str) -> Option<String> {
    let stem = file.strip_suffix(".md")?;
    if starts_with_date(stem) {
        return is_dated(stem).then(|| stem.to_string());
    }
    let b = stem.as_bytes();
    (b.len() > 5 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-').then(|| stem[..4].to_string())
}

/// The token the title line starts with: the four-digit number, or the date of a dated ADR.
fn title_token(file: &str) -> Option<String> {
    let id = id_of(file)?;
    Some(if id.len() == 4 { id } else { id[..10].to_string() })
}

/// The first word(s) of a status: the parenthesised explanations are dropped (`superseded by 0016 (the server) and 0017 (x)` -> `superseded by 0016 and 0017`).
pub fn short_status(status: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in status.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads one ADR. `Err` says what the file lacks.
pub fn parse(file: &str, text: &str) -> Result<Adr, String> {
    let text = text.replace("\r\n", "\n");
    let id = id_of(file).ok_or_else(|| format!("{file}: the name must start with a four-digit number (0057-...) or a date (2026-09-28-...)"))?;
    let token = title_token(file).unwrap_or_default();
    let mut lines = text.lines();
    let h1 = lines.next().unwrap_or("");
    let rest = h1.strip_prefix("# ").ok_or_else(|| format!("{file}: the first line must be `# {token}. Title`"))?;
    let title = rest
        .strip_prefix(&format!("{token}."))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| format!("{file}: the first line must be `# {token}. Title` (it says `{h1}`)"))?
        .to_string();
    let head: Vec<&str> = text.lines().take(8).collect();
    let field = |name: &str| head.iter().find_map(|l| l.strip_prefix(name)).map(|v| v.trim().to_string());
    let status = field("Status:").ok_or_else(|| format!("{file}: needs a `Status: accepted|proposed|superseded by <id>` line right after the title"))?;
    let summary = field("Summary:").ok_or_else(|| format!("{file}: needs a `Summary: <one sentence>` line after the Status (it becomes the index row)"))?;
    Ok(Adr { file: file.to_string(), id, title, status, summary })
}

/// Index order: the numbered ADRs by number, then the dated ones by date.
fn sort_key(file: &str) -> (bool, String) {
    (is_dated(file.trim_end_matches(".md")), file.to_string())
}

/// The markdown table of `adrs`, one row per ADR.
pub fn render_index(adrs: &[Adr]) -> String {
    let mut sorted: Vec<&Adr> = adrs.iter().collect();
    sorted.sort_by_key(|a| sort_key(&a.file));
    let mut s = String::from("| # | Decision | Status |\n|---|---|---|\n");
    for a in sorted {
        s.push_str(&format!("| [{}]({}) | {} | {} |\n", a.id, a.file, a.summary.replace('|', "/"), short_status(&a.status)));
    }
    s
}

/// Replaces the region between the index markers of `readme` with `table`. `Err` if the markers are missing.
pub fn sync_index(readme: &str, table: &str) -> Result<String, String> {
    let (Some(a), Some(b)) = (readme.find(INDEX_BEGIN), readme.find(INDEX_END)) else {
        return Err(format!("docs/adr/README.md needs an {INDEX_BEGIN} ... {INDEX_END} region to hold the generated index"));
    };
    if b < a {
        return Err("adr-index:end comes before adr-index:begin".into());
    }
    let nl = if readme.contains("\r\n") { "\r\n" } else { "\n" };
    Ok(format!("{}{}{}{}", &readme[..a + INDEX_BEGIN.len()], nl, table.replace('\n', nl), &readme[b..]))
}

/// The text between the index markers (trimmed), if present.
pub fn current_index(readme: &str) -> Option<String> {
    let (a, b) = (readme.find(INDEX_BEGIN)?, readme.find(INDEX_END)?);
    (a < b).then(|| readme[a + INDEX_BEGIN.len()..b].replace("\r\n", "\n").trim().to_string())
}

/// `(file name, text)` of every ADR under `root/docs/adr`, sorted by name.
pub fn load_dir(root: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(root.join("docs/adr"))
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.ends_with(".md") && n != "README.md")
                .filter_map(|n| std::fs::read_to_string(root.join("docs/adr").join(&n)).ok().map(|t| (n, t)))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Every ADR that parses, sorted for the index.
pub fn all(files: &[(String, String)]) -> Vec<Adr> {
    let mut v: Vec<Adr> = files.iter().filter_map(|(f, t)| parse(f, t).ok()).collect();
    v.sort_by_key(|a| sort_key(&a.file));
    v
}

/// The ids a `superseded by ...` status names (four-digit numbers and dated ids).
fn superseders(status: &str) -> Vec<String> {
    status
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|w| (w.len() == 4 && w.bytes().all(|b| b.is_ascii_digit())) || (w.len() > 11 && is_dated(w)))
        .map(str::to_string)
        .collect()
}

/// Everything wrong with the decision records under `root`, each with the edit that fixes it. Empty = true.
pub fn check(root: &Path) -> Vec<String> {
    let files = load_dir(root);
    let mut problems = Vec::new();
    let mut parsed = Vec::new();
    for (file, text) in &files {
        match parse(file, text) {
            Ok(a) => {
                let word = a.status.split(|c: char| !c.is_alphabetic()).next().unwrap_or("");
                if !["accepted", "proposed", "superseded"].contains(&word) {
                    problems.push(format!("docs/adr/{file}: unknown status `{}` (accepted, proposed, or superseded by <id>)", a.status));
                }
                if a.summary.is_empty() || a.summary.contains(SUMMARY_PLACEHOLDER) {
                    problems.push(format!("docs/adr/{file}: write the `Summary:` line (one sentence: it is the row in the index)"));
                }
                let body = text.replace("\r\n", "\n");
                for h in ["## Context", "## Decision", "## Consequences"] {
                    if !body.contains(h) {
                        problems.push(format!("docs/adr/{file}: missing the `{h}` section"));
                    }
                }
                parsed.push(a);
            }
            Err(e) => problems.push(format!("docs/adr/{e}")),
        }
    }
    let mut seen = std::collections::BTreeMap::new();
    for a in &parsed {
        if let Some(other) = seen.insert(a.id.clone(), a.file.clone()) {
            problems.push(format!(
                "docs/adr/{} and docs/adr/{other} share the id {}: rename one (a dated id, `red_engine2 adr new`, cannot collide)",
                a.file, a.id
            ));
        }
    }
    for a in &parsed {
        if a.status.starts_with("superseded") {
            let by = superseders(&a.status);
            if by.is_empty() {
                problems.push(format!("docs/adr/{}: `superseded` must say by which ADR: `{}`", a.file, a.status));
            }
            for id in by {
                if !parsed.iter().any(|p| p.id == id) {
                    problems.push(format!("docs/adr/{}: superseded by {id}, which does not exist", a.file));
                }
            }
        }
    }
    let readme = std::fs::read_to_string(root.join("docs/adr/README.md")).unwrap_or_default().replace("\r\n", "\n");
    match current_index(&readme) {
        None => problems.push("docs/adr/README.md needs an <!-- adr-index:begin --> ... <!-- adr-index:end --> region (the generated index)".to_string()),
        Some(have) if have != render_index(&parsed).trim() => {
            problems.push("docs/adr/README.md: the index is stale; run `red_engine2 adr index --write`".to_string())
        }
        Some(_) => {}
    }
    problems
}

/// Rewrites the generated index of `root/docs/adr/README.md`. Returns whether the file changed.
pub fn write_index(root: &Path) -> Result<bool, String> {
    let path = root.join("docs/adr/README.md");
    let readme = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let table = render_index(&all(&load_dir(root)));
    let new = sync_index(&readme, &table)?;
    if new == readme {
        return Ok(false);
    }
    std::fs::write(&path, new).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// A file-name slug: lower-case words joined by `-`, at most `max` characters, cut between words.
pub fn slugify(title: &str, max: usize) -> String {
    let mut slug = String::new();
    for word in title.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        let w = word.to_ascii_lowercase();
        if !slug.is_empty() && slug.len() + 1 + w.len() > max {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&w);
    }
    slug.chars().take(max).collect::<String>().trim_matches('-').to_string()
}

/// The text of a new ADR.
fn template(date: &str, title: &str, status: &str, summary: &str) -> String {
    format!(
        "# {date}. {title}\nStatus: {status}\nSummary: {summary}\n\n## Context\nWhat forced a decision: the problem, what was tried, what it cost.\n\n## Decision\nWhat we do. Link code by symbol name (`red_engine2 src show <symbol>`), not by line number.\n\n## Consequences\nWhat gets easier and what gets harder; how to undo it.\n"
    )
}

/// Creates `docs/adr/<date>-<slug>.md` from the template and refreshes the index. Returns the new file's path.
pub fn new_adr(root: &Path, title: &str, summary: Option<&str>, status: &str, slug: Option<&str>, date: &str) -> Result<PathBuf, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("give the decision a title: `red_engine2 adr new \"Lag compensation\"`".into());
    }
    if !["accepted", "proposed"].contains(&status) {
        return Err(format!("status is `accepted` or `proposed` (a later ADR supersedes an old one by editing its Status line), not `{status}`"));
    }
    let slug = slugify(slug.unwrap_or(title), 56);
    if slug.is_empty() {
        return Err("the title has no letters or digits to make a file name from; pass --slug".into());
    }
    let file = format!("{date}-{slug}.md");
    let path = root.join("docs/adr").join(&file);
    if path.exists() {
        return Err(format!("{} already exists: pick another --slug, or edit it", path.display()));
    }
    std::fs::create_dir_all(path.parent().unwrap_or(root)).map_err(|e| e.to_string())?;
    std::fs::write(&path, template(date, title, status, summary.unwrap_or(SUMMARY_PLACEHOLDER))).map_err(|e| format!("{}: {e}", path.display()))?;
    write_index(root)?;
    Ok(path)
}

/// One line per ADR: id, status word, summary.
pub fn render_list(adrs: &[Adr]) -> String {
    adrs.iter().map(|a| format!("{:<28} {:<11} {}\n", a.id, short_status(&a.status).split(' ').next().unwrap_or(""), a.summary)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adr(id_file: &str, status: &str) -> (String, String) {
        let token = title_token(id_file).unwrap();
        (id_file.to_string(), format!("# {token}. A title\nStatus: {status}\nSummary: the point.\n\n## Context\nc\n\n## Decision\nd\n\n## Consequences\nq\n"))
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("re2_adr_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("docs/adr")).unwrap();
        std::fs::write(d.join("docs/adr/README.md"), format!("# ADRs\n\n{INDEX_BEGIN}\n{INDEX_END}\n\n## Writing one\n")).unwrap();
        d
    }

    #[test]
    fn ids_come_from_the_number_or_the_whole_dated_stem() {
        assert_eq!(id_of("0057-avatars.md").as_deref(), Some("0057"));
        assert_eq!(id_of("2026-09-28-lag-comp.md").as_deref(), Some("2026-09-28-lag-comp"));
        assert_eq!(id_of("2026-09-28.md"), None, "a date alone has no slug");
        assert_eq!(id_of("notes.md"), None);
        assert_eq!(title_token("2026-09-28-lag-comp.md").as_deref(), Some("2026-09-28"));
    }

    #[test]
    fn a_status_loses_its_explanations_in_the_index() {
        assert_eq!(short_status("accepted"), "accepted");
        assert_eq!(short_status("superseded by 0016 (the server) and 0017 (the build)"), "superseded by 0016 and 0017");
    }

    #[test]
    fn parsing_names_what_a_file_lacks() {
        let ok = parse("0001-x.md", "# 0001. Maps are JSON\nStatus: accepted\nSummary: one line\n\n## Context\n").unwrap();
        assert_eq!((ok.id.as_str(), ok.title.as_str(), ok.status.as_str(), ok.summary.as_str()), ("0001", "Maps are JSON", "accepted", "one line"));
        assert!(parse("0001-x.md", "# 0002. Wrong number\nStatus: accepted\nSummary: s\n").unwrap_err().contains("# 0001. Title"));
        assert!(parse("0001-x.md", "# 0001. T\nSummary: s\n").unwrap_err().contains("Status"));
        assert!(parse("0001-x.md", "# 0001. T\nStatus: accepted\n").unwrap_err().contains("Summary"));
        assert!(parse("x.md", "# T\n").unwrap_err().contains("must start with"));
        let dated = parse("2026-09-28-lag.md", "# 2026-09-28. Lag\nStatus: proposed\nSummary: s\n").unwrap();
        assert_eq!(dated.id, "2026-09-28-lag");
    }

    #[test]
    fn the_index_lists_numbered_adrs_first_and_dated_ones_after() {
        let files =
            vec![adr("2026-09-28-b.md", "accepted"), adr("0002-two.md", "accepted"), adr("0001-one.md", "accepted"), adr("2026-09-27-a.md", "proposed")];
        let table = render_index(&all(&files));
        let order: Vec<&str> = table.lines().skip(2).map(|l| l.split(']').next().unwrap().trim_start_matches("| [")).collect();
        assert_eq!(order, ["0001", "0002", "2026-09-27-a", "2026-09-28-b"]);
        assert!(table.contains("| [0001](0001-one.md) | the point. | accepted |"), "{table}");
    }

    #[test]
    fn new_creates_a_dated_file_and_refreshes_the_index() {
        let root = tmp("new");
        let path = new_adr(&root, "Lag compensation, v2!", Some("Rewind the shooter's view."), "accepted", None, "2026-09-28").unwrap();
        assert!(path.ends_with("2026-09-28-lag-compensation-v2.md"), "{path:?}");
        let readme = std::fs::read_to_string(root.join("docs/adr/README.md")).unwrap();
        assert!(readme.contains("[2026-09-28-lag-compensation-v2](2026-09-28-lag-compensation-v2.md) | Rewind the shooter's view. | accepted"), "{readme}");
        assert!(readme.contains("## Writing one"), "the hand-written part survives");
        assert!(check(&root).is_empty(), "{:?}", check(&root));
        assert!(new_adr(&root, "Lag compensation, v2!", None, "accepted", None, "2026-09-28").unwrap_err().contains("already exists"));
        assert!(new_adr(&root, "x", None, "done", None, "2026-09-28").unwrap_err().contains("status"));
        assert!(new_adr(&root, "  ", None, "accepted", None, "2026-09-28").unwrap_err().contains("title"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_template_left_as_it_is_fails_the_check_and_says_what_to_write() {
        let root = tmp("todo");
        new_adr(&root, "Something", None, "proposed", None, "2026-09-28").unwrap();
        let problems = check(&root);
        assert!(problems.iter().any(|p| p.contains("Summary")), "{problems:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_check_finds_duplicates_missing_superseders_and_a_stale_index() {
        let root = tmp("bad");
        for (f, t) in [
            adr("0001-one.md", "accepted"),
            adr("0001-uno.md", "accepted"),
            adr("0002-two.md", "superseded by 0009"),
            adr("2026-01-01-x.md", "superseded by 2026-01-02-nope"),
        ] {
            std::fs::write(root.join("docs/adr").join(f), t).unwrap();
        }
        let problems = check(&root).join("\n");
        assert!(problems.contains("share the id 0001"), "{problems}");
        assert!(problems.contains("superseded by 0009, which does not exist"), "{problems}");
        assert!(problems.contains("superseded by 2026-01-02-nope, which does not exist"), "{problems}");
        assert!(problems.contains("index is stale"), "{problems}");
        write_index(&root).unwrap();
        assert!(!check(&root).join("\n").contains("index is stale"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn slugs_are_short_lower_case_words() {
        assert_eq!(slugify("Lag compensation, v2!", 56), "lag-compensation-v2");
        assert_eq!(slugify("A very long title that keeps going and going and going forever", 30), "a-very-long-title-that-keeps");
        assert_eq!(slugify("???", 56), "");
    }
}
