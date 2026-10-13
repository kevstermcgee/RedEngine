//! `red_engine2 validate [paths]` over many files (ADR 2026-10-11-one-validate-for-every-content-file): every content file under the given folders (the
//! repository's content folders when none are given), its format detected, checked by that format's own loader (the hand-written checker, so the messages
//! are the usual ones, cross-references included: prefab names, rule/objective/combat names, a blueprint's prefab files, a project's maps), and every
//! problem printed as `file:line: message`.

use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// A content format `validate` recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Scene,
    Game2d,
    PrefabLibrary,
    PackRegistry,
    Score,
    Blueprint,
    Playscript,
    Project,
    Drivers,
    Trace,
    Collection,
}

impl Kind {
    /// Every format, in report order.
    pub const ALL: [Kind; 11] = [
        Kind::Scene,
        Kind::Game2d,
        Kind::PrefabLibrary,
        Kind::PackRegistry,
        Kind::Score,
        Kind::Blueprint,
        Kind::Playscript,
        Kind::Project,
        Kind::Drivers,
        Kind::Trace,
        Kind::Collection,
    ];

    /// The name a report uses.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Scene => "scene",
            Kind::Game2d => "2D game",
            Kind::PrefabLibrary => "prefab library",
            Kind::PackRegistry => "pack registry",
            Kind::Score => "score",
            Kind::Blueprint => "blueprint",
            Kind::Playscript => "playscript",
            Kind::Project => "game.json",
            Kind::Drivers => "kart drivers",
            Kind::Trace => "trace",
            Kind::Collection => "museum collection",
        }
    }
}

/// Where `validate` with no paths looks, relative to an engine checkout.
pub const CONTENT_DIRS: &[&str] = &["examples", "recipes", "assets", "tests/fixtures", "games"];
/// Folders never searched: build output, goldens, tool scratch.
const SKIP_DIRS: &[&str] = &["target", "node_modules", "out", "golden", ".git", ".red", "dist"];

/// The format of a JSON file, from its name and its top-level keys; `None` for JSON that is not content (`features.json`, a vocabulary, a golden list).
pub fn detect(path: &Path, v: &Value) -> Option<Kind> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let has = |k: &str| v.get(k).is_some();
    if name.ends_with(".game2d.json") || has("game2d") {
        Some(Kind::Game2d)
    } else if name.ends_with(".trace.json") {
        Some(Kind::Trace)
    } else if name.ends_with(".blueprint.json") {
        Some(Kind::Blueprint)
    } else if name.ends_with(".play.json") {
        Some(Kind::Playscript)
    } else if name == "game.json" && has("game") {
        Some(Kind::Project)
    } else if name == "packs.json" && has("packs") {
        Some(Kind::PackRegistry)
    } else if has("roster") && has("drivers") {
        Some(Kind::Drivers)
    } else if v.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|d| d.get("name").and_then(Value::as_str).is_some())) {
        Some(Kind::PrefabLibrary)
    } else if has("themes") && has("styles") && has("artists") {
        Some(Kind::Collection)
    } else if has("tracks") && has("instruments") {
        Some(Kind::Score)
    } else if has("objects") {
        Some(Kind::Scene)
    } else {
        None
    }
}

/// Every `.json` file under `dirs` (files are taken as given), sorted, skipping build output and goldens.
pub fn discover(dirs: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if !p.file_name().and_then(|n| n.to_str()).is_some_and(|n| SKIP_DIRS.contains(&n) || n.starts_with('.')) {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "json") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        if d.is_dir() {
            walk(d, &mut out);
        } else {
            out.push(d.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// One problem: the 1-based line it points at, when the message's path can be found in the file, and the checker's message.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub line: Option<usize>,
    pub message: String,
}

/// What `validate` found in one file.
#[derive(Debug, Clone)]
pub struct FileReport {
    pub path: PathBuf,
    /// `None`: not a content file (skipped when found by a folder walk, a problem when named).
    pub kind: Option<Kind>,
    pub problems: Vec<Problem>,
}

/// Checks one file with its format's checker. `named`: the user named this file, so a JSON file of no known format is a problem, not a skip.
pub fn check_file(path: &Path, named: bool) -> FileReport {
    let mut report = FileReport { path: path.to_path_buf(), kind: None, problems: Vec::new() };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            report.problems.push(Problem { line: None, message: format!("cannot read: {e}") });
            return report;
        }
    };
    let v: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            report.problems.push(Problem { line: Some(e.line()), message: format!("not valid JSON: {e}") });
            return report;
        }
    };
    report.kind = detect(path, &v);
    let Some(kind) = report.kind else {
        if named {
            let known: Vec<&str> = Kind::ALL.iter().map(|k| k.name()).collect();
            report.problems.push(Problem { line: None, message: format!("not a content file `validate` knows (formats: {})", known.join(", ")) });
        }
        return report;
    };
    let messages = check(kind, &v, &text, path);
    if !messages.is_empty() {
        let lines = value_lines(&text);
        report.problems = messages.into_iter().map(|m| Problem { line: line_of(&lines, &v, &m), message: m }).collect();
    }
    report
}

/// The checker's messages for one file of a known format (empty when it is valid).
pub fn check(kind: Kind, v: &Value, text: &str, path: &Path) -> Vec<String> {
    let base = path.parent();
    let label = path.display().to_string();
    match kind {
        Kind::Scene => crate::validate_scene_text(text, base, &label).err().unwrap_or_default(),
        Kind::Game2d => red2d::game::parse(text).err().unwrap_or_default(),
        Kind::PrefabLibrary => library_problems(v),
        Kind::PackRegistry => pack_problems(v, base.and_then(Path::parent)),
        Kind::Score => crate::score::parse_score(v).err().unwrap_or_default(),
        Kind::Blueprint => crate::tools::blueprint::compile_in(v, base).err().unwrap_or_default(),
        Kind::Playscript => crate::playscript::Script::parse(text).err().unwrap_or_default(),
        Kind::Project => crate::tools::game::parse(base.unwrap_or(Path::new(".")), text).err().unwrap_or_default(),
        Kind::Drivers => crate::sim::kart::Roster::parse(text).err().into_iter().collect(),
        Kind::Trace => crate::sim::trace::Trace::from_json(v).err().into_iter().collect(),
        Kind::Collection => crate::museum::spec::parse_collection(v, None).err().unwrap_or_default(),
    }
}

/// A prefab library: its definitions load, and each one expands into a valid scene (where its objects and the prefabs it uses are checked).
fn library_problems(v: &Value) -> Vec<String> {
    let mut lib = crate::prefabs::builtin().0.clone();
    let errs = lib.add_json(v, "validate");
    if !errs.is_empty() {
        return errs;
    }
    let mut out = Vec::new();
    for name in v.as_array().into_iter().flatten().filter_map(|d| d.get("name").and_then(Value::as_str)) {
        let scene = crate::prefabs::preview_scene_in(&lib, name, None);
        if let Err(e) = crate::scene_blocks::parse_scene(&scene.to_string()) {
            out.extend(e.into_iter().map(|e| format!("{name}: {e}")));
        }
    }
    out
}

/// The pack registry: its typed shape, and every pack's file exists (relative to the checkout, when known).
fn pack_problems(v: &Value, root: Option<&Path>) -> Vec<String> {
    let reg: super::packs::PackRegistry = match super::typed(v) {
        Ok(r) => r,
        Err(e) => return vec![e],
    };
    let Some(root) = root else { return Vec::new() };
    reg.packs
        .iter()
        .enumerate()
        .filter(|(_, p)| !root.join(&p.file).is_file())
        .map(|(i, p)| format!("packs[{i}].file: `{}` does not exist (pack `{}`)", p.file, p.id))
        .collect()
}

/// The result of a whole run.
pub struct Run {
    pub files: Vec<FileReport>,
    pub seconds: f64,
}

impl Run {
    /// Number of problems over every file.
    pub fn problems(&self) -> usize {
        self.files.iter().map(|f| f.problems.len()).sum()
    }

    /// `file:line: message` per problem, then one summary line (files per format, problems, time).
    pub fn render(&self, root: Option<&Path>) -> String {
        let mut out = String::new();
        for f in &self.files {
            let shown = root.and_then(|r| f.path.strip_prefix(r).ok()).unwrap_or(&f.path).display().to_string().replace('\\', "/");
            for p in &f.problems {
                match p.line {
                    Some(l) => out.push_str(&format!("{shown}:{l}: {}\n", p.message)),
                    None => out.push_str(&format!("{shown}: {}\n", p.message)),
                }
            }
        }
        let mut per: BTreeMap<Kind, usize> = BTreeMap::new();
        for f in &self.files {
            if let Some(k) = f.kind {
                *per.entry(k).or_default() += 1;
            }
        }
        let checked: usize = per.values().sum();
        let kinds: Vec<String> = per.iter().map(|(k, n)| format!("{} {n}", k.name())).collect();
        let bad = self.files.iter().filter(|f| !f.problems.is_empty()).count();
        out.push_str(&format!(
            "validate: {checked} content file(s) ({}), {} problem(s) in {bad} file(s), {:.1} s\n",
            kinds.join(", "),
            self.problems(),
            self.seconds
        ));
        out
    }
}

/// Validates `paths` (files and folders; the checkout's [`CONTENT_DIRS`] under `root` when empty). Files are checked on all cores.
pub fn run(paths: &[PathBuf], root: &Path) -> Run {
    let start = Instant::now();
    let named: Vec<PathBuf> = paths.iter().filter(|p| p.is_file()).cloned().collect();
    let dirs: Vec<PathBuf> = if paths.is_empty() { CONTENT_DIRS.iter().map(|d| root.join(d)).collect() } else { paths.to_vec() };
    let files = discover(&dirs);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(8);
    let chunk = files.len().div_ceil(threads.max(1)).max(1);
    let mut reports: Vec<FileReport> = std::thread::scope(|s| {
        let handles: Vec<_> = files.chunks(chunk).map(|part| s.spawn(|| part.iter().map(|p| check_file(p, named.contains(p))).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    // A missing path the user named is a problem too (a folder walk only lists what exists).
    for p in paths.iter().filter(|p| !p.exists()) {
        reports.push(FileReport { path: p.clone(), kind: None, problems: vec![Problem { line: None, message: "no such file or folder".into() }] });
    }
    Run { files: reports, seconds: start.elapsed().as_secs_f64() }
}

/// Every value's path in a JSON text (`objects[3].material.color`, the root is ``) with the 1-based line where the value starts.
pub fn value_lines(text: &str) -> HashMap<String, usize> {
    struct P<'a> {
        b: &'a [u8],
        i: usize,
        line: usize,
        out: HashMap<String, usize>,
    }
    impl P<'_> {
        fn ws(&mut self) {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                if self.b[self.i] == b'\n' {
                    self.line += 1;
                }
                self.i += 1;
            }
        }
        fn string(&mut self) -> String {
            let start = self.i + 1;
            self.i += 1;
            while self.i < self.b.len() && self.b[self.i] != b'"' {
                if self.b[self.i] == b'\\' {
                    self.i += 1;
                }
                self.i += 1;
            }
            let s = String::from_utf8_lossy(&self.b[start..self.i.min(self.b.len())]).into_owned();
            self.i += 1;
            s
        }
        fn value(&mut self, path: String, depth: usize) {
            self.ws();
            if self.i >= self.b.len() || depth > 256 {
                return;
            }
            self.out.entry(path.clone()).or_insert(self.line);
            match self.b[self.i] {
                b'{' => {
                    self.i += 1;
                    loop {
                        self.ws();
                        if self.i >= self.b.len() || self.b[self.i] == b'}' {
                            self.i += 1;
                            return;
                        }
                        if self.b[self.i] == b',' {
                            self.i += 1;
                            continue;
                        }
                        if self.b[self.i] != b'"' {
                            return;
                        }
                        let key = self.string();
                        self.ws();
                        if self.i < self.b.len() && self.b[self.i] == b':' {
                            self.i += 1;
                        }
                        let child = if path.is_empty() { key } else { format!("{path}.{key}") };
                        self.value(child, depth + 1);
                    }
                }
                b'[' => {
                    self.i += 1;
                    let mut n = 0;
                    loop {
                        self.ws();
                        if self.i >= self.b.len() || self.b[self.i] == b']' {
                            self.i += 1;
                            return;
                        }
                        if self.b[self.i] == b',' {
                            self.i += 1;
                            continue;
                        }
                        self.value(format!("{path}[{n}]"), depth + 1);
                        n += 1;
                    }
                }
                b'"' => {
                    self.string();
                }
                _ => {
                    while self.i < self.b.len() && !matches!(self.b[self.i], b',' | b'}' | b']') && !self.b[self.i].is_ascii_whitespace() {
                        self.i += 1;
                    }
                }
            }
        }
    }
    let mut p = P { b: text.as_bytes(), i: 0, line: 1, out: HashMap::new() };
    p.value(String::new(), 0);
    p.out
}

/// The line a checker message points at: its leading `path:` (`objects[3].material.color`, or an object id as the first step, `sofa_1.material`), cut
/// back to the deepest part that exists in the file. `None` when the message names no path the file has.
pub fn line_of(lines: &HashMap<String, usize>, v: &Value, message: &str) -> Option<usize> {
    // `rules[3] (exit_locked).when.enter.zone: ...`: a checker may name the item in parentheses after its index.
    let raw = message.split(": ").next().filter(|h| h.len() < message.len())?;
    let mut head = String::new();
    let mut depth = 0;
    for c in raw.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => head.push(c),
            _ => {}
        }
    }
    let head = head.replace(" .", ".").trim().to_string();
    if head.is_empty() || head.contains(' ') {
        return None;
    }
    let head = head.as_str();
    let mut candidates = vec![head.to_string()];
    let first = head.split(['.', '[']).next().unwrap_or(head);
    if v.get(first).is_none() {
        if let Some(p) = object_path(v, first) {
            candidates.push(format!("{p}{}", &head[first.len()..]));
        }
    }
    for c in candidates {
        let mut path = c.as_str();
        loop {
            if let Some(l) = lines.get(path) {
                if !path.is_empty() {
                    return Some(*l);
                }
            }
            match path.rfind(['.', '[']) {
                Some(i) if i > 0 => path = &path[..i],
                _ => break,
            }
        }
    }
    None
}

/// The path of the object (in `objects`, or nested in a group's `children`) whose `id` is `id`.
fn object_path(v: &Value, id: &str) -> Option<String> {
    fn find(list: &Value, prefix: &str, id: &str) -> Option<String> {
        for (i, o) in list.as_array()?.iter().enumerate() {
            let here = format!("{prefix}[{i}]");
            if o.get("id").and_then(Value::as_str) == Some(id) {
                return Some(here);
            }
            if let Some(found) = o.get("children").and_then(|c| find(c, &format!("{here}.children"), id)) {
                return Some(found);
            }
        }
        None
    }
    find(v.get("objects")?, "objects", id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_path_or_an_object_id_points_at_its_line() {
        let text = "{\n  \"objects\": [\n    {\"id\": \"a\", \"type\": \"box\"},\n    {\n      \"id\": \"sofa_1\",\n      \"material\": {\"color\": 3}\n    }\n  ],\n  \"rules\": []\n}";
        let v: Value = serde_json::from_str(text).unwrap();
        let lines = value_lines(text);
        assert_eq!(lines.get("objects[1].material.color"), Some(&6));
        assert_eq!(line_of(&lines, &v, "objects[1].material.color: must be a colour"), Some(6));
        assert_eq!(line_of(&lines, &v, "sofa_1.material.color: must be a colour"), Some(6), "an id as the first step");
        assert_eq!(line_of(&lines, &v, "objects[1].material.zzz: unknown field"), Some(6), "cut back to the deepest part that exists");
        assert_eq!(line_of(&lines, &v, "rules: empty"), Some(9));
        assert_eq!(line_of(&lines, &v, "objects[1] (sofa_1).material.color: must be a colour"), Some(6), "a name in parentheses after an index");
        assert_eq!(line_of(&lines, &v, "the scene has no spawn"), None, "prose names no path");
    }

    #[test]
    fn formats_are_told_apart_by_name_and_keys() {
        let k = |name: &str, json: &str| detect(Path::new(name), &serde_json::from_str(json).unwrap());
        assert_eq!(k("x.game2d.json", "{}"), Some(Kind::Game2d));
        assert_eq!(k("map.json", r#"{"objects":[]}"#), Some(Kind::Scene));
        assert_eq!(k("food.json", r#"[{"name":"apple","objects":[]}]"#), Some(Kind::PrefabLibrary));
        assert_eq!(k("song.json", r#"{"tracks":[],"instruments":{}}"#), Some(Kind::Score));
        assert_eq!(k("game.json", r#"{"game":1}"#), Some(Kind::Project));
        assert_eq!(k("features.json", r#"{"features":{}}"#), None);
    }

    #[test]
    fn a_broken_file_is_reported_with_its_line_and_a_named_unknown_file_is_a_problem() {
        let dir = std::env::temp_dir().join(format!("re2_validate_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.json");
        std::fs::write(
            &bad,
            "{\n  \"camera\": {\"position\": [0, 2, 8]},\n  \"objects\": [\n    {\"id\": \"b\", \"type\": \"box\", \"pos\": [0, 1, 0]}\n  ]\n}",
        )
        .unwrap();
        let other = dir.join("notes.json");
        std::fs::write(&other, "{\"hello\": 1}").unwrap();
        let r = run(std::slice::from_ref(&dir), &dir);
        assert_eq!(r.problems(), 1, "{}", r.render(None));
        let text = r.render(Some(&dir));
        assert!(text.starts_with("bad.json:4: b.pos: unknown field"), "{text}");
        let named = run(std::slice::from_ref(&other), &dir);
        assert!(named.render(None).contains("not a content file"), "{}", named.render(None));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
