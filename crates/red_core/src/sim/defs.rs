//! Namespaced definitions: what game content defines (a machine kind, a recipe, an ability) under a stable id, with where it was written
//! (ADR 2026-10-14-game-systems-compose-under-one-match-authority).
//!
//! An [`Id`] is `namespace:name` (`ws:furnace`); a reference written without a namespace means the referring file's own ([`Id::resolve`]).
//! [`Defs`] keeps definitions in id order, so iteration never depends on the order files were read. Defining an id twice is an error that names
//! both places ([`code::DUPLICATE`]); a later file replaces an earlier definition only when it says so ([`Defs::replace`]), and replacing an id
//! nothing defined is an error too. Every problem is a [`Diagnostic`] with a stable code, the place it was found and, where one exists, a fix.
//!
//! Used by the game-system host (`sim::systems`) and the games built on it (Workshop, `games/workshop`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Stable diagnostic codes of game definitions and systems (also in `tools::envelope::CODES`, so `--json` reports them as they are).
pub mod code {
    /// An id that is not `namespace:name`.
    pub const ID_FORM: &str = "invalid";
    /// Two definitions, systems or stores share an id.
    pub const DUPLICATE: &str = "duplicate-id";
    /// A reference to an id nothing defines.
    pub const UNKNOWN: &str = "unknown-name";
    /// A system, definition or replacement depends on something it does not declare or that is not there.
    pub const DEPENDENCY: &str = "undeclared-dependency";
    /// Systems that must each run after the other.
    pub const CYCLE: &str = "dependency-cycle";
    /// An event or command a system does not declare.
    pub const EVENT: &str = "undeclared-event";
    /// More events in one tick than the host's bound: a cascade that does not settle.
    pub const OVERFLOW: &str = "event-overflow";
    /// Saved state of a version this program cannot read.
    pub const STATE_VERSION: &str = "state-version";
    /// A transaction that would take more than a store holds.
    pub const MISSING: &str = "insufficient";
    /// A transaction that would put more into a store than it has room for.
    pub const FULL: &str = "store-full";
    /// A query a system reads that nothing answers, or answers with another type.
    pub const QUERY: &str = "undeclared-query";
}

/// A stable, namespaced id: `namespace:name`, each part 1 to 40 lowercase letters, digits, `_` or `-`, starting with a letter (`ws:furnace`); the
/// name may go on in `/`-separated parts for something an instance owns (`ws:furnace_a/in`, its input store).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Id(String);

fn part_ok(p: &str) -> bool {
    (1..=40).contains(&p.len())
        && p.starts_with(|c: char| c.is_ascii_lowercase())
        && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

impl Id {
    /// Parses `namespace:name`.
    pub fn parse(s: &str) -> Result<Id, String> {
        match s.split_once(':') {
            Some((ns, name)) if part_ok(ns) && name.split('/').all(part_ok) => Ok(Id(s.to_string())),
            _ => Err(format!(
                "`{s}` is not an id: write `namespace:name` (lowercase letters, digits, `_`, `-`; like `ws:furnace`, or `ws:furnace/in` for a part of one)"
            )),
        }
    }

    /// A reference as written in a file of namespace `ns`: `name` means `ns:name`, `other:name` stays as written.
    pub fn resolve(ns: &str, reference: &str) -> Result<Id, String> {
        if reference.contains(':') {
            Id::parse(reference)
        } else {
            Id::parse(&format!("{ns}:{reference}"))
        }
    }

    /// The namespace (`ws`).
    pub fn namespace(&self) -> &str {
        self.0.split_once(':').map_or("", |(ns, _)| ns)
    }

    /// The name inside the namespace (`furnace`).
    pub fn name(&self) -> &str {
        self.0.split_once(':').map_or(self.0.as_str(), |(_, n)| n)
    }

    /// The whole id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Id {
    type Error = String;
    fn try_from(s: String) -> Result<Id, String> {
        Id::parse(&s)
    }
}

impl From<Id> for String {
    fn from(id: Id) -> String {
        id.0
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// Where something was written: a file and the JSON pointer inside it (`content/base.json#/machines/1`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Provenance {
    /// The file, as the content names it (relative to the scene).
    pub file: String,
    /// The JSON pointer to the definition inside the file (`/machines/1`).
    pub pointer: String,
}

impl Provenance {
    /// `file#pointer`.
    pub fn new(file: &str, pointer: &str) -> Provenance {
        Provenance { file: file.to_string(), pointer: pointer.to_string() }
    }

    /// The same file, one level further in (`/machines` -> `/machines/1`).
    pub fn at(&self, key: impl fmt::Display) -> Provenance {
        Provenance { file: self.file.clone(), pointer: format!("{}/{key}", self.pointer) }
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.file, if self.pointer.is_empty() { "/" } else { &self.pointer })
    }
}

/// One problem: a stable [`code`], where (`file#pointer`, or a system or instance id), what, and how to fix it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// A stable code ([`code`]).
    pub code: &'static str,
    /// Where it was found.
    pub at: String,
    /// What is wrong.
    pub message: String,
    /// What to change, when there is one answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Diagnostic {
    /// A problem with no fix to suggest.
    pub fn new(code: &'static str, at: impl fmt::Display, message: impl Into<String>) -> Diagnostic {
        Diagnostic { code, at: at.to_string(), message: message.into(), fix: None }
    }

    /// The same problem with a fix.
    pub fn fix(mut self, fix: impl Into<String>) -> Diagnostic {
        self.fix = Some(fix.into());
        self
    }
}

/// `at: [code] message (fix: ...)`: one line, as every engine error (`path: message`), with the code where `--json` finds it.
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: [{}] {}", self.at, self.code, self.message)?;
        if let Some(fix) = &self.fix {
            write!(f, " (fix: {fix})")?;
        }
        Ok(())
    }
}

/// Definitions of one kind by id, each with where it was written. Iteration is in id order.
#[derive(Debug, Clone)]
pub struct Defs<T> {
    map: BTreeMap<Id, (T, Provenance)>,
}

impl<T> Default for Defs<T> {
    fn default() -> Self {
        Defs { map: BTreeMap::new() }
    }
}

impl<T> Defs<T> {
    /// No definitions.
    pub fn new() -> Defs<T> {
        Defs::default()
    }

    /// Defines `id`; an id already defined is a [`code::DUPLICATE`] naming both places.
    pub fn define(&mut self, id: Id, def: T, at: Provenance) -> Result<(), Diagnostic> {
        if let Some((_, first)) = self.map.get(&id) {
            return Err(Diagnostic::new(code::DUPLICATE, &at, format!("`{id}` is already defined at {first}"))
                .fix(format!("rename one, or write `\"replace\": true` here to replace the definition at {first} on purpose")));
        }
        self.map.insert(id, (def, at));
        Ok(())
    }

    /// Replaces the definition of `id` on purpose (content that says `"replace": true`); replacing an id nothing defined is a
    /// [`code::DEPENDENCY`]: the replacement depends on a definition that is not there (a file missing from the list, or listed after this one).
    pub fn replace(&mut self, id: Id, def: T, at: Provenance) -> Result<(), Diagnostic> {
        if !self.map.contains_key(&id) {
            return Err(Diagnostic::new(code::DEPENDENCY, &at, format!("replaces `{id}`, which no earlier file defines"))
                .fix("list the file that defines it before this one, or drop `\"replace\": true`"));
        }
        self.map.insert(id, (def, at));
        Ok(())
    }

    /// The definition of `id`.
    pub fn get(&self, id: &Id) -> Option<&T> {
        self.map.get(id).map(|(d, _)| d)
    }

    /// Where `id` was defined.
    pub fn provenance(&self, id: &Id) -> Option<&Provenance> {
        self.map.get(id).map(|(_, p)| p)
    }

    /// Whether `id` is defined.
    pub fn contains(&self, id: &Id) -> bool {
        self.map.contains_key(id)
    }

    /// Every definition in id order.
    pub fn iter(&self) -> impl Iterator<Item = (&Id, &T)> {
        self.map.iter().map(|(id, (d, _))| (id, d))
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Resolves a reference written in namespace `ns` at `at` to a defined id, or says which ids are close ([`code::UNKNOWN`]).
    pub fn reference(&self, ns: &str, reference: &str, what: &str, at: &Provenance) -> Result<Id, Diagnostic> {
        let id = Id::resolve(ns, reference).map_err(|e| Diagnostic::new(code::ID_FORM, at, e))?;
        if self.map.contains_key(&id) {
            return Ok(id);
        }
        let near = closest(id.as_str(), self.map.keys().map(Id::as_str));
        let d = Diagnostic::new(code::UNKNOWN, at, format!("no {what} `{id}`"));
        Err(match near {
            Some(n) => d.fix(format!("did you mean `{n}`?")),
            None if self.map.is_empty() => d.fix(format!("define a {what} first")),
            None => d.fix(format!("one of: {}", self.map.keys().map(Id::as_str).collect::<Vec<_>>().join(", "))),
        })
    }
}

/// The candidate within a small edit distance of `word`, if any (the did-you-mean).
pub fn closest<'a>(word: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    fn distance(a: &str, b: &str) -> usize {
        let b: Vec<char> = b.chars().collect();
        let mut row: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.chars().enumerate() {
            let mut prev = row[0];
            row[0] = i + 1;
            for (j, cb) in b.iter().enumerate() {
                let next = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(row[j + 1] + 1);
                prev = row[j + 1];
                row[j + 1] = next;
            }
        }
        row[b.len()]
    }
    let limit = (word.len() / 3).max(2);
    candidates.map(|c| (distance(word, c), c)).filter(|(d, _)| *d <= limit).min().map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> Id {
        Id::parse(s).unwrap()
    }

    #[test]
    fn an_id_is_a_namespace_and_a_name_and_a_bare_reference_means_the_file_s_own_namespace() {
        assert_eq!((id("ws:furnace").namespace(), id("ws:furnace").name()), ("ws", "furnace"));
        assert_eq!(id("ws:furnace_a/in").name(), "furnace_a/in");
        for bad in ["furnace", "ws:", ":x", "WS:x", "ws:Furnace", "ws:a:b", "1ws:x", "ws:a/", "ws/a:b"] {
            assert!(Id::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(Id::resolve("ws", "ore").unwrap(), id("ws:ore"));
        assert_eq!(Id::resolve("ws2", "ws:ore").unwrap(), id("ws:ore"));
        let json: Id = serde_json::from_str("\"ws:press\"").unwrap();
        assert_eq!(json, id("ws:press"));
        assert!(serde_json::from_str::<Id>("\"press\"").is_err());
    }

    #[test]
    fn a_duplicate_names_both_places_and_only_an_explicit_replacement_wins() {
        let mut d = Defs::new();
        d.define(id("ws:smelt"), 1, Provenance::new("base.json", "/recipes/0")).unwrap();
        let e = d.define(id("ws:smelt"), 2, Provenance::new("more.json", "/recipes/3")).unwrap_err();
        assert_eq!(e.code, code::DUPLICATE);
        assert!(e.to_string().starts_with("more.json#/recipes/3: [duplicate-id] `ws:smelt` is already defined at base.json#/recipes/0"), "{e}");
        assert_eq!(d.get(&id("ws:smelt")), Some(&1), "a rejected duplicate changes nothing");
        d.replace(id("ws:smelt"), 3, Provenance::new("more.json", "/recipes/3")).unwrap();
        assert_eq!((d.get(&id("ws:smelt")), d.provenance(&id("ws:smelt")).unwrap().file.as_str()), (Some(&3), "more.json"));
        let e = d.replace(id("ws:nothing"), 4, Provenance::new("more.json", "/recipes/4")).unwrap_err();
        assert_eq!(e.code, code::DEPENDENCY);
    }

    #[test]
    fn an_unknown_reference_suggests_the_closest_id_and_iteration_is_in_id_order() {
        let mut d = Defs::new();
        for (i, n) in ["ws:press", "ws:furnace", "ws:assembler"].iter().enumerate() {
            d.define(id(n), i, Provenance::default()).unwrap();
        }
        assert_eq!(d.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), ["ws:assembler", "ws:furnace", "ws:press"]);
        let e = d.reference("ws", "furnance", "machine", &Provenance::new("m.json", "/machines/0/kind")).unwrap_err();
        assert_eq!((e.code, e.fix.as_deref()), (code::UNKNOWN, Some("did you mean `ws:furnace`?")));
        assert_eq!(d.reference("ws", "press", "machine", &Provenance::default()).unwrap(), id("ws:press"));
    }
}
