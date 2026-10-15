//! The story toolchain (ADR 2026-10-15-narrative-is-compiled-content-run-by-one-story-system): `.story` text in, a package the runtime
//! (`sim::story`) loads out. `story check` parses, binds, type-checks and lints; `story build` also writes the package (a content build: no
//! Rust is compiled); `story refs` answers who reads, writes, tells or depends on something; `story ledger` lists setups and payoffs and
//! what each choice really changes; `story walk` runs the real runtime under the game's match (`walk`). None of it is in a shipped game:
//! the game loads the package.

pub mod compile;
pub mod lint;
pub mod parse;
pub mod refs;
pub mod types;
pub mod walk;

use crate::sim::story::package::{Catalog, Package};
use parse::At;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// How bad a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The story is refused.
    Error,
    /// Valid, probably not what was meant.
    Warning,
    /// A warning the source accepts, with its reason (`allow`): listed, never hidden.
    Allowed,
}

/// One problem, at a `file:line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diag {
    /// Error or warning.
    pub severity: Severity,
    /// A stable code (`story-syntax`, `story-flow`, ...).
    pub code: &'static str,
    /// Where.
    pub at: At,
    /// What.
    pub message: String,
    /// What to change.
    pub fix: Option<String>,
}

impl Diag {
    /// An error.
    pub fn error(code: &'static str, at: At, message: String, fix: &str) -> Diag {
        Diag { severity: Severity::Error, code, at, message, fix: (!fix.is_empty()).then(|| fix.to_string()) }
    }

    /// A warning.
    pub fn warning(code: &'static str, at: At, message: String, fix: &str) -> Diag {
        Diag { severity: Severity::Warning, code, at, message, fix: (!fix.is_empty()).then(|| fix.to_string()) }
    }

    /// Whether it refuses the story.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// `file:line: error[code]: message (fix)`.
    pub fn render(&self, files: &[String]) -> String {
        let file = files.get(self.at.file).map_or("story", String::as_str);
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Allowed => "allowed",
        };
        let place = if self.at.line == 0 { file.to_string() } else { format!("{file}:{}", self.at.line) };
        format!("{place}: {sev}[{}]: {}{}", self.code, self.message, self.fix.as_ref().map(|f| format!(" -> {f}")).unwrap_or_default())
    }

    /// As JSON.
    pub fn to_json(&self, files: &[String]) -> Value {
        json!({"severity": self.severity, "code": self.code, "file": files.get(self.at.file), "line": self.at.line, "message": self.message, "fix": self.fix})
    }
}

/// A story read and compiled.
pub struct Built {
    /// The parse tree.
    pub ast: parse::Ast,
    /// The package (when there are no errors).
    pub package: Option<Package>,
    /// What the compiler learned for the linter and `refs`.
    pub index: compile::Index,
    /// Every problem, errors first.
    pub diags: Vec<Diag>,
    /// The game's bindings it was compiled against.
    pub catalog: Catalog,
    /// What each binding means (the bindings file's `notes`).
    pub notes: std::collections::BTreeMap<String, String>,
    /// Files outside the story that may name its ids: the scene (`map`, its `checks.sim`) and the folder's other JSON (walks).
    pub outside: Vec<PathBuf>,
    /// The folder (or file) it was read from.
    pub root: PathBuf,
}

impl Built {
    /// How many errors.
    pub fn errors(&self) -> usize {
        self.diags.iter().filter(|d| d.is_error()).count()
    }

    /// How many warnings (not counting the allowed ones).
    pub fn warnings(&self) -> usize {
        self.diags.iter().filter(|d| d.severity == Severity::Warning).count()
    }

    /// How many warnings the source allows.
    pub fn allowed(&self) -> usize {
        self.diags.iter().filter(|d| d.severity == Severity::Allowed).count()
    }
}

/// The bindings file a game writes for its story authors (`<game>_server story-bindings > bindings.json`): its catalog, plus what each
/// query, command and event means.
pub const BINDINGS_FORMAT: u64 = 1;

/// The `.story` files a path names: the file itself, or every `.story` file in the folder (sorted).
pub fn sources(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut out: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "story"))
        .collect();
    out.sort();
    if out.is_empty() {
        return Err(format!("{}: no .story files", path.display()));
    }
    Ok(out)
}

/// Reads a bindings file into the game's catalog.
pub fn read_bindings(path: &Path) -> Result<Catalog, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: not JSON: {e}", path.display()))?;
    match v.get("story_bindings").and_then(Value::as_u64) {
        Some(BINDINGS_FORMAT) => {}
        other => return Err(format!("{}: not a story bindings file of format {BINDINGS_FORMAT} (`story_bindings` is {other:?})", path.display())),
    }
    serde_json::from_value(v.get("catalog").cloned().unwrap_or(Value::Null)).map_err(|e| format!("{}: catalog: {e}", path.display()))
}

/// A bindings file for `catalog`, with a meaning for each id in `notes`.
pub fn bindings_json(game: &str, catalog: &Catalog, notes: &[(&str, &str)]) -> Value {
    json!({
        "story_bindings": BINDINGS_FORMAT,
        "game": game,
        "about": "what this game offers a story: queries it answers (conditions read them), commands it takes (effects run them; `unattended` ones a director may), events it emits (barks answer them). Generated from the game's code; a test keeps it fresh.",
        "catalog": catalog,
        "notes": notes.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
    })
}

fn fnv(texts: &[(String, String)]) -> String {
    let mut h = crate::sim::ledger::Fnv::new();
    for (name, text) in texts {
        h.str(name);
        h.str(text);
    }
    format!("{:016x}", h.0)
}

/// Reads, compiles and lints the story at `path` (a folder of `.story` files, or one). The `uses` and `map` paths are relative to the
/// file that names them.
pub fn build(path: &Path) -> Result<Built, String> {
    let files = sources(path)?;
    let root = if path.is_file() { path.parent().unwrap_or(Path::new(".")).to_path_buf() } else { path.to_path_buf() };
    let mut texts = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        let name = f.strip_prefix(&root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| f.display().to_string());
        texts.push((name, text));
    }
    let (ast, mut diags) = parse::parse(&texts);
    let near = |s: &parse::Spanned| root.join(Path::new(&texts[s.at.file].0).parent().unwrap_or(Path::new(""))).join(&s.text);
    let mut catalog = Catalog::default();
    let mut notes = std::collections::BTreeMap::new();
    let mut outside: Vec<PathBuf> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json") && !p.to_string_lossy().ends_with(".story.json") && p.file_name().is_some_and(|n| n != "bindings.json")
        })
        .collect();
    if let Some(u) = &ast.header.uses {
        match read_bindings(&near(u)) {
            Ok(c) => catalog = c,
            Err(e) => diags.push(Diag::error("story-binding", u.at, e, "regenerate it with the game's `story-bindings`")),
        }
        let v: Option<Value> = std::fs::read_to_string(near(u)).ok().and_then(|t| serde_json::from_str(&t).ok());
        for (k, n) in v.as_ref().and_then(|v| v.get("notes")).and_then(Value::as_object).into_iter().flatten() {
            notes.insert(k.clone(), n.as_str().unwrap_or("").to_string());
        }
    }
    if let Some(m) = &ast.header.map {
        outside.push(near(m));
    }
    outside.sort();
    let mut scene = std::collections::BTreeMap::new();
    if let Some(m) = &ast.header.map {
        match std::fs::read_to_string(near(m)).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string())) {
            Ok(v) => {
                scene = types::scene_types(&v);
                catalog.scene_vars = scene.keys().cloned().collect();
            }
            Err(e) => diags.push(Diag::error("story-binding", m.at, format!("{}: {e}", m.text), "")),
        }
    }
    let (package, index, more) =
        if diags.iter().any(Diag::is_error) { (None, compile::Index::default(), Vec::new()) } else { compile::compile(&ast, &catalog, &scene, &fnv(&texts)) };
    diags.extend(more);
    let mut package = package;
    if let Some(p) = &mut package {
        diags.extend(lint::lint(&ast, p, &index, &catalog));
        // A choice whose author did not declare its stakes carries what it was traced to change.
        for (_, li, k, stakes, _) in lint::trace_stakes(p, &index, &catalog) {
            if !index.declared.contains(&(li, k)) {
                p.lines[li as usize].choices[k].stakes = stakes;
            }
        }
    }
    diags.sort_by_key(|d| (d.severity, d.at.file, d.at.line));
    let package = package.filter(|_| !diags.iter().any(Diag::is_error));
    Ok(Built { ast, package, index, diags, catalog, notes, outside, root })
}

/// Where `story build` writes the package of the story at `root`: `<root>/<story>.story.json`.
pub fn package_path(root: &Path, p: &Package) -> PathBuf {
    root.join(format!("{}.story.json", p.story))
}

/// The package as the file holds it (pretty JSON, one trailing newline: what `--check` compares).
pub fn package_text(p: &Package) -> String {
    let mut s = serde_json::to_string_pretty(p).unwrap_or_default();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests;

/// `describe story`: the format, the commands and the contract on one screen (the whole guide is `docs/STORY.md`, found by `search`).
pub fn describe_text() -> String {
    String::from(
        "Stories: `.story` text compiled by a content build into a package the game's story system runs under the match's one authority (ADR\n\
2026-10-15-narrative-is-compiled-content-run-by-one-story-system). Reads the game through typed queries, acts only through its commands.\n\
Guide + contract: docs/STORY.md (search \"story <question>\"). Reference game: games/tidewatch. Exemplars: examples/story/{the_debt,the_rumour,rain}.\n\
\n\
WORKFLOW  inventory (bindings.json, scene vars, the map) -> premise -> character sheets -> structure -> skeleton + bindings (`story check`)\n\
          -> dialogue scene by scene -> `story build` -> scenarios + `story walk` -> `story review` -> patch only what is named -> play it\n\
          a mechanic the game lacks is `request <id> \"<what>\"`, never invented in prose\n\
\n\
COMMANDS  story check DIR | build DIR [--check] | refs DIR ID | ledger DIR | review DIR      (red_engine2: no game binary needed)\n\
          <game>_server cli story walk SCENE WALKS.json   <game>_server cli sim SCENE --inspect story   (they run the real game)\n\
\n\
FORMAT    story <name> version <n> / uses bindings.json / map scene.json / round <event>\n\
          character <id> \"Name\" (want need|arc flaw contradiction values relationships voice example x2; knows present cooldown)\n\
          extra <id> \"Name\" (role voice)   fact <id> bool|int = v [round|run|profile] \"meaning\"   info <id> \"says\" [true if <expr>]\n\
          setup <id> \"what\" + by <ref> + pays acknowledge|resolve|complicate|leave <ref>   category <id> cooldown 12s\n\
          conversation <id> with <speaker> [if, priority, interrupt n, repeat]; node <id>; <speaker>: text {n}\n\
            line attrs: if / do e; e / reveals info / claims info / id name / -> node|end\n\
            * choice: text   attrs: if / do / kind consequential|informational|expressive / -> node|end / else -> node\n\
          bark <id> on <event> [about id]: one line or `silence`; if, priority, cooldown 20s, once, category, do, reveals\n\
          notice <id> on <event>: if + do (what witnesses learn), never speaks   director <id>: every, recover, budget n per round,\n\
            tension, rest if, intervene <id> (if, chance n%, group, once, do run <unattended command>)\n\
          effects: set f = e | add f e | learn|believe who info | run cmd [on id] [n] | bark id | start conv\n\
          names in expressions: fact, ns:query, scene:var, knows(who, info), believes(who, info), heard(line), present(speaker)\n\
\n\
RUNTIME   answers name the revision shown (stale/repeated refused); a choice is checked again when picked; its commands apply whole or\n\
          not at all, then the story's effects. Barks: notices apply, then best = priority, specificity, least heard, first written;\n\
          a real tie draws once (seeded); cooldowns: bark, category, speaker (no higher priority); queue 3, stale after 4 s; higher\n\
          priority interrupts (seen, not heard). Directors never act during a conversation; rest/chance do nothing on purpose.\n\
          Knowledge: reveals -> know, claims -> believe, only those present. Saves by id. The world does not pause by itself.\n\
EXPECT    {\"game\": \"story\", \"path\": \"facts.F|heard.LINE|chosen.BARK|reads.QUERY\", \"eq|gte|...\": n}   steps {\"story\": \"talk|advance|until_choice|choose|leave|skip\"}\n\
LEVELS    content-valid / verified on tested paths / presentation inspected / editorially reviewed / human-playtested (claim only what ran)\n",
    )
}
