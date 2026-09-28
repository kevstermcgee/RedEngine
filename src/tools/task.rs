//! `red_engine2 context "<task in plain words>"`: route a coding task to the features that own it and hand back a small **task packet**
//! (ADR 0045): owners and how confident the routing is, the files and symbols to read first, the invariants annotated in them, known traps
//! and settled decisions, canonical examples, the verification to run, what depends on the change, what probably does not matter, the
//! expected scope, and when to stop. It is a starting point, never a fence: `context <feature> --full`, `src`, `search` widen it.
//!
//! Routing is lexical but not naive: stop words dropped, words stemmed and expanded with `search`'s synonyms, and every term weighted by how
//! rare it is across features (a word every feature shares says nothing). A feature is matched on its name, summary and file paths and on
//! the purposes and public symbol names of the source files it owns, plus the knowledge entries it owns (`docs/KNOWLEDGE.md`).

use super::features::Feature;
use super::knowledge;
use super::symbols::{self, Index};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Default size of a task packet in bytes (about 1.2k tokens).
pub const TASK_BUDGET: usize = 5_000;

/// Words that say nothing about *where* a change goes.
const TASK_STOP: &[&str] = &[
    "add", "adds", "adding", "make", "makes", "fix", "fixes", "change", "changes", "update", "new", "support", "allow", "let", "should", "want", "need",
    "please", "into", "from", "that", "this", "when", "then", "than", "into", "so", "it", "its", "be", "by", "at", "as", "we", "our", "all", "some", "any",
    "more", "less", "way", "thing", "things", "use", "using", "work", "works", "able", "also", "just", "only", "up",
];

/// Lower-case, split on anything not a letter or digit (so `snake_case` and paths split too), camelCase split, stop words dropped, stemmed.
pub fn terms(text: &str) -> Vec<String> {
    let mut spaced = String::with_capacity(text.len() + 8);
    let mut prev_lower = false;
    for c in text.chars() {
        if c.is_ascii_uppercase() && prev_lower {
            spaced.push(' ');
        }
        prev_lower = c.is_ascii_lowercase();
        spaced.push(if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { ' ' });
    }
    let mut out: Vec<String> = Vec::new();
    for w in spaced.split_whitespace() {
        if w.len() < 2 || TASK_STOP.contains(&w) || super::search::STOP.contains(&w) {
            continue;
        }
        let s = stem(w);
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// A light suffix stripper: `replicated`/`replication`/`replicate` -> `replic`, `doors` -> `door`, `jumping` -> `jump`.
pub fn stem(w: &str) -> String {
    for suf in ["ations", "ation", "ions", "ion", "ings", "ing", "ated", "ates", "ate", "ers", "ies", "ed", "es", "s"] {
        if let Some(r) = w.strip_suffix(suf) {
            if r.len() >= 3 {
                return if suf == "ies" { format!("{r}y") } else { r.to_string() };
            }
        }
    }
    w.to_string()
}

/// A task's words plus their synonyms (from `search`), each with a weight: 1 for the word, 0.5 for a synonym.
fn expand(words: &[String]) -> Vec<(String, f32)> {
    let mut out: Vec<(String, f32)> = words.iter().map(|w| (w.clone(), 1.0)).collect();
    for (k, syn) in super::search::SYNONYMS {
        if words.iter().any(|w| *w == stem(k)) {
            for s in terms(syn) {
                if !out.iter().any(|(t, _)| *t == s) {
                    out.push((s, 0.5));
                }
            }
        }
    }
    out
}

/// What each feature is "about", as weighted term sets.
struct Profile<'a> {
    feature: &'a Feature,
    /// term -> strongest field weight (name 3, summary 2, path 1.5, module purpose / public symbol / knowledge 1).
    weight: BTreeMap<String, f32>,
}

fn owned_sources<'a>(f: &Feature, ix: &'a Index) -> Vec<&'a str> {
    let mut v: Vec<&str> =
        ix.files.iter().map(|s| s.rel.as_str()).filter(|r| r.starts_with("src/") && f.files.iter().any(|p| super::features::glob_match(p, r))).collect();
    v.sort();
    v
}

fn profiles<'a>(all: &'a [Feature], ix: &Index, purposes: &BTreeMap<String, String>) -> Vec<Profile<'a>> {
    let know = knowledge::entries();
    all.iter()
        .map(|f| {
            let mut weight: BTreeMap<String, f32> = BTreeMap::new();
            let mut add = |text: &str, w: f32| {
                for t in terms(text) {
                    let e = weight.entry(t).or_insert(0.0);
                    *e = e.max(w);
                }
            };
            add(&f.name, 3.0);
            add(&f.summary, 2.0);
            for p in &f.files {
                add(p.trim_end_matches("/**").trim_end_matches(".rs"), 1.5);
            }
            // Module purposes and public symbol names count less the more files a feature owns (length normalisation): a feature with
            // forty files matches almost any word through sheer vocabulary, which says little about ownership.
            let sources = owned_sources(f, ix);
            let w_code = 1.0 / (1.0 + (1.0 + sources.len() as f32).ln());
            for src in sources {
                if let Some(p) = purposes.get(src) {
                    add(p, w_code);
                }
                for y in ix.symbols.iter().filter(|y| y.file == src && y.public && !y.in_tests) {
                    add(&y.name, w_code);
                }
            }
            for e in know.iter().filter(|e| e.features.contains(&f.name)) {
                add(&e.title, 1.0);
                add(e.field("symptom").unwrap_or(""), 1.0);
            }
            Profile { feature: f, weight }
        })
        .collect()
}

/// A routed owner.
#[derive(Debug, Clone)]
pub struct Owner {
    /// Feature name.
    pub name: String,
    /// Relevance score (sum of rarity-weighted matches).
    pub score: f32,
    /// Task words that matched it.
    pub matched: Vec<String>,
}

/// A file worth reading first, with the symbols that matched.
#[derive(Debug, Clone)]
pub struct ReadFirst {
    /// Path under the repo.
    pub file: String,
    /// Its `//!` purpose.
    pub purpose: String,
    /// `sig // doc` for up to three matching public items.
    pub symbols: Vec<String>,
}

/// Everything a task packet says, before rendering.
#[derive(Debug, Clone)]
pub struct TaskPacket {
    /// The task as given.
    pub task: String,
    /// `high`, `medium` or `low`.
    pub confidence: &'static str,
    /// Why the confidence is what it is.
    pub confidence_note: String,
    /// Owning features, best first.
    pub owners: Vec<Owner>,
    /// Task words nothing matched.
    pub unmatched: Vec<String>,
    /// Files (and symbols) to read first.
    pub read_first: Vec<ReadFirst>,
    /// `AI-*` annotations in those files.
    pub invariants: Vec<String>,
    /// Known traps, checks and decisions owned by the owners, one line each.
    pub knowledge: Vec<String>,
    /// Canonical examples of the owners.
    pub canonical: Vec<String>,
    /// ADR pointers of the owners (`0016 title`).
    pub adrs: Vec<String>,
    /// Unit filters and suites that prove the owners.
    pub tests: Vec<String>,
    /// Features built on the owners (their tests run in `affected`).
    pub dependents: Vec<String>,
    /// Features with no dependency path to or from the owners.
    pub unneeded: Vec<String>,
    /// Expected edit scope.
    pub scope: String,
}

/// Routes `task` and builds its packet. `Err` when no word matches anything.
pub fn plan(all: &[Feature], ix: &Index, task: &str) -> Result<TaskPacket, String> {
    let words = terms(task);
    if words.is_empty() {
        return Err(format!("'{task}' has no words that say where the change goes: name a feature, a file, a symbol or a behaviour"));
    }
    let purposes: BTreeMap<String, String> = symbols::modules(ix).into_iter().map(|m| (m.file, m.purpose)).collect();
    let profs = profiles(all, ix, &purposes);
    let n = profs.len() as f32;
    let query = expand(&words);
    let idf = |t: &str| {
        let df = profs.iter().filter(|p| p.weight.contains_key(t)).count() as f32;
        if df == 0.0 {
            0.0
        } else {
            (1.0 + n / df).ln()
        }
    };
    let mut owners: Vec<Owner> = profs
        .iter()
        .map(|p| {
            let mut score = 0.0;
            let mut matched = Vec::new();
            for (t, qw) in &query {
                if let Some(w) = p.weight.get(t) {
                    score += w * qw * idf(t);
                    if *qw >= 1.0 {
                        matched.push(t.clone());
                    }
                }
            }
            Owner { name: p.feature.name.clone(), score, matched }
        })
        .filter(|o| o.score > 0.0)
        .collect();
    owners.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.name.cmp(&b.name)));
    let Some(top) = owners.first().map(|o| o.score) else {
        return Err(format!("no feature matches '{task}': try `search \"{task}\"`, name a file (`context src/net/server.rs`), or `features`"));
    };
    let second = owners.get(1).map_or(0.0, |o| o.score);
    owners.retain(|o| o.score >= top * 0.55);
    owners.truncate(3);
    let covered: BTreeSet<&String> = owners.iter().flat_map(|o| o.matched.iter()).collect();
    let unmatched: Vec<String> = words.iter().filter(|w| !covered.contains(w)).cloned().collect();
    let coverage = 1.0 - unmatched.len() as f32 / words.len() as f32;
    let (confidence, confidence_note) = if coverage >= 0.66 && top >= 1.6 * second {
        ("high", "one feature clearly owns the task".to_string())
    } else if coverage >= 0.5 {
        ("medium", format!("{} plausible owner(s); read the first file before assuming", owners.len()))
    } else {
        ("low", "most task words matched nothing: broaden with `search \"<question>\"` or name a file, and verify with `affected` (not --quick)".to_string())
    };

    let owner_features: Vec<&Feature> = owners.iter().filter_map(|o| all.iter().find(|f| f.name == o.name)).collect();
    let owner_names: Vec<&str> = owner_features.iter().map(|f| f.name.as_str()).collect();

    // Read first: the owners' source files, ranked by how many task terms their path, purpose and public symbols hit.
    let mut files: Vec<(f32, ReadFirst)> = Vec::new();
    for f in &owner_features {
        for src in owned_sources(f, ix) {
            if files.iter().any(|(_, r)| r.file == src) {
                continue;
            }
            let purpose = purposes.get(src).cloned().unwrap_or_default();
            let path_terms = terms(src.trim_end_matches(".rs"));
            let purpose_terms = terms(&purpose);
            let mut score = 0.0;
            for (t, qw) in &query {
                if path_terms.contains(t) {
                    score += 2.0 * qw * idf(t);
                }
                if purpose_terms.contains(t) {
                    score += qw * idf(t);
                }
            }
            let mut syms: Vec<(f32, String)> = Vec::new();
            for y in ix.symbols.iter().filter(|y| y.file == src && y.public && !y.in_tests && !matches!(y.kind, "impl" | "mod")) {
                let hay = format!("{} {}", y.name, y.doc);
                let yt = terms(&hay);
                let s: f32 = query.iter().filter(|(t, _)| yt.contains(t)).map(|(t, qw)| qw * idf(t)).sum();
                if s > 0.0 {
                    let sig: String = y.sig.chars().take(100).collect();
                    let doc: String = y.doc.chars().take(70).collect();
                    syms.push((s, if doc.is_empty() { sig } else { format!("{sig}  // {doc}") }));
                }
            }
            syms.sort_by(|a, b| b.0.total_cmp(&a.0));
            score += syms.iter().take(3).map(|s| s.0 * 0.5).sum::<f32>();
            if score > 0.0 {
                files.push((
                    score,
                    ReadFirst { file: src.to_string(), purpose: purpose.chars().take(100).collect(), symbols: syms.into_iter().take(3).map(|s| s.1).collect() },
                ));
            }
        }
    }
    files.sort_by(|a, b| b.0.total_cmp(&a.0));
    let read_first: Vec<ReadFirst> = files.into_iter().take(5).map(|(_, r)| r).collect();

    let read_set: BTreeSet<&str> = read_first.iter().map(|r| r.file.as_str()).collect();
    let owned_all: BTreeSet<String> = owner_features.iter().flat_map(|f| owned_sources(f, ix)).map(str::to_string).collect();
    let mut invariants: Vec<String> = symbols::annotations(ix)
        .into_iter()
        .filter(|a| read_set.contains(a.file.as_str()) || owned_all.contains(&a.file))
        .map(|a| {
            let first = read_set.contains(a.file.as_str());
            (!first, a.line())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    invariants.truncate(6);

    let knowledge: Vec<String> = knowledge::for_features(&owner_names).iter().map(|e| e.line()).take(6).collect();
    let mut canonical: Vec<String> = Vec::new();
    let mut adrs: Vec<String> = Vec::new();
    let mut tests: Vec<String> = Vec::new();
    for f in &owner_features {
        for c in &f.canonical {
            if !canonical.contains(c) {
                canonical.push(c.clone());
            }
        }
        for d in f.docs.iter().filter(|d| d.starts_with("docs/adr/")) {
            let file = d.rsplit('/').next().unwrap_or(d);
            let line = format!("{} {}", &file[..4.min(file.len())], super::search::adr_title(file).unwrap_or_default());
            if !adrs.contains(&line) {
                adrs.push(line);
            }
        }
        for t in &f.tests {
            if !tests.contains(t) {
                tests.push(t.clone());
            }
        }
    }
    adrs.truncate(4);

    // Dependents (transitive) and the features unrelated to the owners in either direction.
    let closure = |start: &[&str], forward: bool| -> BTreeSet<String> {
        let mut seen: BTreeSet<String> = start.iter().map(|s| s.to_string()).collect();
        let mut stack: Vec<String> = seen.iter().cloned().collect();
        while let Some(cur) = stack.pop() {
            let next: Vec<String> = if forward {
                all.iter().filter(|g| g.depends_on.contains(&cur)).map(|g| g.name.clone()).collect()
            } else {
                all.iter().find(|g| g.name == cur).map(|g| g.depends_on.clone()).unwrap_or_default()
            };
            for n in next {
                if seen.insert(n.clone()) {
                    stack.push(n);
                }
            }
        }
        seen
    };
    let down = closure(&owner_names, true);
    let up = closure(&owner_names, false);
    let dependents: Vec<String> = down.iter().filter(|d| !owner_names.contains(&d.as_str())).cloned().collect();
    let unneeded: Vec<String> = all
        .iter()
        .map(|f| f.name.clone())
        .filter(|n| !down.contains(n) && !up.contains(n) && !matches!(n.as_str(), "docs_and_adrs" | "test_support"))
        .collect();

    let scope = match owners.len() {
        1 => "one feature: expect about 1-4 implementation files".to_string(),
        _ => format!(
            "{} features: expect about 3-8 files; if they sit in unrelated subsystems, check whether the change belongs in a lower shared layer",
            owners.len()
        ),
    };
    Ok(TaskPacket {
        task: task.to_string(),
        confidence,
        confidence_note,
        owners,
        unmatched,
        read_first,
        invariants,
        knowledge,
        canonical,
        adrs,
        tests,
        dependents,
        unneeded,
        scope,
    })
}

/// The completion contract every task packet ends with.
pub const DONE_WHEN: &str =
    "DONE WHEN the requested behaviour works, a regression test covers it where one can, `scripts/dev affected` passes, and docs changed \
only if a public contract did. Then STOP: no refactoring of adjacent code you merely read, no redesign of unrelated systems, no speculative \
abstractions, no unrelated cleanup. Crossing >10 files on a local task: re-run `context`/`affected` before going on.";

impl TaskPacket {
    /// The packet as text, cut to `budget` bytes on whole sections (lowest priority last).
    pub fn render(&self, budget: usize) -> String {
        let mut head = format!("TASK {}\nROUTE confidence {}: {}", self.task, self.confidence, self.confidence_note);
        if !self.unmatched.is_empty() {
            head.push_str(&format!(" (unmatched: {})", self.unmatched.join(", ")));
        }
        head.push('\n');
        for o in &self.owners {
            head.push_str(&format!("== {} (matched: {})\n", o.name, if o.matched.is_empty() { "synonyms".to_string() } else { o.matched.join(", ") }));
        }
        let mut read = String::from("READ FIRST (then `src outline <file>` / `src show <symbol>`; widen with `context <feature> --full`):\n");
        for r in &self.read_first {
            read.push_str(&format!("  {}  {}\n", r.file, r.purpose));
            for s in &r.symbols {
                read.push_str(&format!("      {s}\n"));
            }
        }
        let list = |title: &str, items: &[String]| if items.is_empty() { String::new() } else { format!("{title}\n  {}\n", items.join("\n  ")) };
        let inv = list("INVARIANTS (annotated in these files):", &self.invariants);
        let know = list("TRAPS / DECISIONS (`context <ID>` for the whole entry):", &self.knowledge);
        let canon = if self.canonical.is_empty() { String::new() } else { format!("CANONICAL (imitate): {}\n", self.canonical.join(", ")) };
        let adrs = if self.adrs.is_empty() { String::new() } else { format!("ADRS: {}\n", self.adrs.join(" | ")) };
        let verify = format!(
            "VERIFY: `scripts/dev affected --quick` while editing (owners' tests: {}); `scripts/dev affected` before done{}\n",
            self.tests.join(" "),
            if self.dependents.is_empty() { String::new() } else { format!(" (adds dependents: {})", self.dependents.join(", ")) }
        );
        let unneeded = if self.unneeded.is_empty() {
            String::new()
        } else {
            format!("PROBABLY NOT NEEDED (no dependency path to the owners): {}\n", self.unneeded.join(", "))
        };
        let scope = format!("SCOPE: {}\n", self.scope);
        let done = format!("{DONE_WHEN}\n");
        // Priority: head, read, verify, done, scope, knowledge, invariants, canonical, unneeded, adrs.
        let parts = [&head, &read, &verify, &done, &scope, &know, &inv, &canon, &unneeded, &adrs];
        let mut kept = vec![false; parts.len()];
        let mut used = 0usize;
        for (i, p) in parts.iter().enumerate() {
            if used + p.len() <= budget || i == 0 {
                kept[i] = true;
                used += p.len();
            }
        }
        // Print in reading order.
        let order = [0usize, 1, 6, 5, 7, 9, 2, 8, 4, 3];
        let mut out = String::new();
        for i in order {
            if kept[i] {
                out.push_str(parts[i]);
            }
        }
        out
    }

    /// The packet as compact JSON (the `--json` envelope's `data`).
    pub fn to_json(&self) -> Value {
        json!({
            "task": self.task,
            "confidence": self.confidence,
            "owners": self.owners.iter().map(|o| json!({"feature": o.name, "matched": o.matched})).collect::<Vec<_>>(),
            "unmatched": self.unmatched,
            "read_first": self.read_first.iter().map(|r| json!({"file": r.file, "symbols": r.symbols})).collect::<Vec<_>>(),
            "invariants": self.invariants,
            "knowledge": self.knowledge,
            "canonical": self.canonical,
            "verify": {"quick": "scripts/dev affected --quick", "done": "scripts/dev affected", "tests": self.tests, "dependents": self.dependents},
            "unneeded": self.unneeded,
            "scope": self.scope,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_split_stemmed_and_stripped_of_noise() {
        assert_eq!(terms("add replicated doors"), ["replic", "door"]);
        assert_eq!(terms("fix client-side prediction jitter"), ["client", "side", "predict", "jitter"]);
        assert_eq!(terms("snapshot_prop_budget handleInput"), ["snapshot", "prop", "budget", "handle", "input"]);
        assert!(terms("make a new thing").is_empty());
        assert_eq!(stem("replication"), "replic");
        assert_eq!(stem("entities"), "entity");
    }
}
