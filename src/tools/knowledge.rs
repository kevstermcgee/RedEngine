//! Known traps, rejected approaches and guard-test codes (`docs/KNOWLEDGE.md`, embedded): the institutional memory `context` and
//! `search` surface so an agent does not rediscover a mistake or reconsider a settled decision. One entry per ID; see the file's header
//! for the format. `context <ID>` prints one entry; a failing guard test names its ID.

/// The knowledge file, embedded so the tools need no files at runtime.
pub const KNOWLEDGE: &str = include_str!("../../docs/KNOWLEDGE.md");

/// One entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// `HEADLESS-001`, `DEC-FORK`, ...
    pub id: String,
    /// One line.
    pub title: String,
    /// `trap`, `check` or `decision`.
    pub kind: String,
    /// Owning features (names in `docs/features.json`).
    pub features: Vec<String>,
    /// `(field, text)` in file order (symptom, cause, dont, look, verify / rejected, reason, reconsider, adr).
    pub fields: Vec<(String, String)>,
}

impl Entry {
    /// A field's text.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// The entry in full (what `context <ID>` prints).
    pub fn render(&self) -> String {
        let mut s = format!("{} {} ({})\n", self.kind.to_uppercase(), self.id, self.title);
        for (k, v) in &self.fields {
            let label = match k.as_str() {
                "dont" => "do not",
                "look" => "fix / look at",
                "adr" => "ADR",
                other => other,
            };
            s.push_str(&format!("  {label}: {v}\n"));
        }
        if !self.features.is_empty() {
            s.push_str(&format!("  features: {}\n", self.features.join(", ")));
        }
        s
    }

    /// One line for a packet: the ID, the title and the most useful field.
    pub fn line(&self) -> String {
        let hint = match self.kind.as_str() {
            "decision" => self.field("reason").map(|r| format!("rejected: {}", self.field("rejected").unwrap_or(r))),
            _ => self.field("dont").map(|d| format!("do not: {d}")),
        };
        match hint {
            Some(h) => format!("{} {}: {}", self.id, self.title, h),
            None => format!("{} {}", self.id, self.title),
        }
    }
}

/// Parses the knowledge file (`## ID — title` headings, `- key: value` fields).
pub fn parse(text: &str) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            let (id, title) = head.split_once(" — ").unwrap_or((head, ""));
            out.push(Entry { id: id.trim().to_string(), title: title.trim().to_string(), kind: String::new(), features: Vec::new(), fields: Vec::new() });
        } else if let (Some(e), Some(field)) = (out.last_mut(), line.strip_prefix("- ")) {
            let Some((k, v)) = field.split_once(": ") else { continue };
            match k {
                "kind" => e.kind = v.trim().to_string(),
                "features" => e.features = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                _ => e.fields.push((k.to_string(), v.trim().to_string())),
            }
        }
    }
    out
}

/// Every entry in the embedded file.
pub fn entries() -> Vec<Entry> {
    parse(KNOWLEDGE)
}

/// The entry with this ID (case-insensitive), if any.
pub fn find(id: &str) -> Option<Entry> {
    entries().into_iter().find(|e| e.id.eq_ignore_ascii_case(id.trim()))
}

/// Whether `s` looks like an entry ID (`ABC-001`, `DEC-FORK`): upper-case letters, a dash, letters or digits.
pub fn looks_like_id(s: &str) -> bool {
    let Some((a, b)) = s.split_once('-') else { return false };
    !a.is_empty()
        && !b.is_empty()
        && a.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && b.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Entries owned by any of `features`, traps and checks first.
pub fn for_features(features: &[&str]) -> Vec<Entry> {
    let mut v: Vec<Entry> = entries().into_iter().filter(|e| e.features.iter().any(|f| features.contains(&f.as_str()))).collect();
    v.sort_by_key(|e| (e.kind == "decision", e.id.clone()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_parses_and_every_entry_is_complete_and_owned_by_real_features() {
        let all = entries();
        assert!(all.len() >= 10, "{}", all.len());
        let features: Vec<String> = crate::tools::features::load().unwrap().into_iter().map(|f| f.name).collect();
        let mut ids = std::collections::HashSet::new();
        for e in &all {
            assert!(ids.insert(e.id.clone()), "duplicate id {}", e.id);
            assert!(looks_like_id(&e.id), "{} is not an ID", e.id);
            assert!(!e.title.is_empty(), "{} has no title", e.id);
            assert!(!e.features.is_empty(), "{} names no feature", e.id);
            for f in &e.features {
                assert!(features.contains(f), "{}: unknown feature {f}", e.id);
            }
            let needed: &[&str] = match e.kind.as_str() {
                "trap" | "check" => &["symptom", "cause", "look", "verify"],
                "decision" => &["rejected", "reason", "reconsider", "adr"],
                other => panic!("{}: unknown kind '{other}'", e.id),
            };
            for n in needed {
                assert!(e.field(n).is_some(), "{} lacks `{n}`", e.id);
            }
            if let Some(adr) = e.field("adr") {
                assert!(crate::tools::search::ADRS.iter().any(|(f, _)| f.starts_with(adr)), "{}: ADR {adr} does not exist", e.id);
            }
        }
    }

    #[test]
    fn ids_are_found_case_insensitively_and_rendered() {
        let e = find("headless-001").unwrap();
        assert!(e.render().contains("headless_boundary"));
        assert!(e.line().starts_with("HEADLESS-001"));
        assert!(looks_like_id("DEC-FORK") && looks_like_id("NET-001") && !looks_like_id("net-test") && !looks_like_id("replicated"));
        assert!(for_features(&["sim_core"]).iter().any(|e| e.id == "TICK-001"));
    }
}
