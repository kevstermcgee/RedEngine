//! `red_engine2 preflight`: the repository's bookkeeping checks in well under a second, without compiling anything (ADR 2026-09-28-generated-bookkeeping).
//!
//! Every feature used to bounce off CI on paperwork found one full test cycle later: the ADR index, `docs/features.json` owning every file, the headless
//! boundary's list of graphics-only modules, the byte budget of `describe`, a CLI command missing from the tool table, a protocol number in three documents. Each of those
//! checks now lives here as a plain function of the file tree; the test suites call the same functions (one implementation, so preflight and CI cannot disagree), and
//! `preflight` runs them all at once, names what is wrong with **the exact edit that fixes it**, and applies the mechanical ones with `--fix`.

use super::{adr, features, status};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

/// What the caller can measure that a library function cannot (the CLI's own definition lives in the binary crate).
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Also run `cargo fmt --check` (a second or two; skipped silently when rustfmt is not installed).
    pub fmt: bool,
    /// Every CLI command as `(name, about)`: each must be mentioned in the tool reference.
    pub commands: Vec<(String, String)>,
    /// Bytes of `describe --brief` and of the `describe` overview, if the caller rendered them.
    pub describe_sizes: Option<(usize, usize)>,
    /// Tree-only run: skip the two checks that read the *compiled* binary (the CLI command table and the `describe` byte budgets), so a binary built before your
    /// latest edit still gives a correct answer for everything else; the report says what was skipped.
    pub tree: bool,
}

/// A mechanical edit `--fix` can make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// Regenerate the index table in `docs/adr/README.md`.
    AdrIndex,
    /// Rewrite the derived facts block of a document.
    SyncFacts(String),
    /// Rewrite the inline `<!--fact:...-->` values of a document.
    InlineFacts(String),
    /// Add `item` to the `key` array of `feature` in `docs/features.json`.
    FeatureArray {
        /// The feature.
        feature: String,
        /// `files` or `tests`.
        key: String,
        /// The path or suite name.
        item: String,
    },
    /// Add a row for a CLI command to the tool table of `docs/AGENT_REFERENCE.md`.
    CommandRow {
        /// The command.
        command: String,
        /// Its one-line description.
        about: String,
    },
    /// Run `cargo fmt`.
    CargoFmt,
    /// Rewrite a text file with LF line endings.
    NormalizeLf(String),
}

/// One thing wrong, and how to put it right.
#[derive(Debug, Clone)]
pub struct Problem {
    /// Which check found it.
    pub check: &'static str,
    /// What is wrong.
    pub message: String,
    /// The exact edit that fixes it.
    pub edit: String,
    /// The edit `--fix` makes, if it is mechanical.
    pub fix: Option<Fix>,
}

/// The result of one run.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Everything wrong.
    pub problems: Vec<Problem>,
    /// The checks that ran.
    pub ran: Vec<&'static str>,
    /// Checks a tree-only run left out because they need a current build (empty for a full run).
    pub skipped: Vec<&'static str>,
    /// How long it took.
    pub millis: u128,
}

fn problem(check: &'static str, message: impl Into<String>, edit: impl Into<String>, fix: Option<Fix>) -> Problem {
    Problem { check, message: message.into(), edit: edit.into(), fix }
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_default().replace("\r\n", "\n")
}

/// The documents an agent reads first.
pub const AI_DOCS: &[&str] = &["CLAUDE.md", "AGENTS.md", "docs/AGENT_REFERENCE.md"];

// ---- the checks ---------------------------------------------------------------------------------------------------------------------------------------

fn check_adr(root: &Path) -> Vec<Problem> {
    adr::check(root)
        .into_iter()
        .map(|m| {
            if m.contains("index is stale") || m.contains("needs an <!-- adr-index:begin -->") {
                problem("adr", m, "regenerate the table between the adr-index markers of docs/adr/README.md", Some(Fix::AdrIndex))
            } else if m.contains("Summary") {
                problem("adr", m, "add `Summary: <one sentence>` under the `Status:` line of the ADR", None)
            } else {
                problem("adr", m, "edit the ADR file named above (`red_engine2 adr list` shows all of them)", None)
            }
        })
        .collect()
}

fn check_features(root: &Path) -> Vec<Problem> {
    // The index that matters is the file in the tree (the compiled-in copy is only as new as the last build).
    let text = read(root, "docs/features.json");
    let on_disk = match features::parse(&text) {
        Ok(a) => a,
        Err(e) => return vec![problem("features", e, "fix docs/features.json", None)],
    };
    let files = features::repo_files(root);
    let mut out = Vec::new();
    for m in features::check_with(&on_disk, &features::serial_suites_of(&text), root).into_iter().filter(|m| !m.contains("belongs to no feature")) {
        out.push(problem("features", m, "edit docs/features.json so every listed file, suite, doc and dependency exists", None));
    }
    for f in features::unowned(&on_disk, &files) {
        match features::suggest_owner(&on_disk, root, &f, &files) {
            Some(s) => {
                let fix = Fix::FeatureArray { feature: s.feature.clone(), key: s.key.to_string(), item: s.item.clone() };
                out.push(problem(
                    "features",
                    format!("{f} belongs to no feature"),
                    format!("docs/features.json: features.{}.{} += \"{}\"   (a guess: {})", s.feature, s.key, s.item, s.because),
                    Some(fix),
                ));
            }
            None => out.push(problem(
                "features",
                format!("{f} belongs to no feature"),
                "docs/features.json: add it to the `files` array of the feature it belongs to (`red_engine2 features` lists them)",
                None,
            )),
        }
    }
    out
}

fn check_facts(root: &Path) -> Vec<Problem> {
    let mut out = Vec::new();
    let want = status::facts_block(root).trim().to_string();
    for doc in ["CLAUDE.md", "AGENTS.md", "README.md"] {
        if let Some(have) = status::current_facts(&read(root, doc)) {
            if have != want {
                out.push(problem(
                    "facts",
                    format!("{doc}: the derived facts block is stale"),
                    format!("rewrite the region between the facts markers of {doc}"),
                    Some(Fix::SyncFacts(doc.to_string())),
                ));
            }
        }
    }
    for doc in ["CLAUDE.md", "AGENTS.md", "README.md", "SPEC.md", "docs/AGENT_REFERENCE.md", "docs/HOSTING.md"] {
        for (name, have, want) in status::inline_facts(root, &read(root, doc)) {
            match want {
                Some(w) if w != have => out.push(problem(
                    "facts",
                    format!("{doc}: `{name}` says {have}, the code says {w}"),
                    format!("in {doc} replace the value inside <!--fact:{name}-->...<!--/fact--> with {w}"),
                    Some(Fix::InlineFacts(doc.to_string())),
                )),
                None => out.push(problem(
                    "facts",
                    format!("{doc}: <!--fact:{name}--> is not a fact this repository can derive"),
                    "remove the markers or use a known name (protocol)",
                    None,
                )),
                _ => {}
            }
        }
    }
    out
}

/// The `cfg` lines that keep a module out of the headless server build: `gfx` (the desktop client), `render` (the wgpu renderer alone, which `gfx` includes) and the
/// browser player (`web` on wasm32). None of them is on by default without `gfx`, so none is in `--no-default-features`.
fn is_graphics_gate(line: &str) -> bool {
    matches!(
        line.trim(),
        "#[cfg(feature = \"gfx\")]"
            | "#[cfg(feature = \"render\")]"
            | "#[cfg(feature = \"video\")]"
            | "#[cfg(all(feature = \"web\", target_arch = \"wasm32\"))]"
    )
}

/// Modules and directories (relative to `src/`) that are built only with the `gfx` feature (or `render`, or the browser build): every `#[cfg(feature = "gfx")] pub mod x;` of
/// `src/lib.rs`, and the directory or file of each `[[bin]]` that `required-features` gfx.
pub fn gfx_only_paths(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    // `src/lib.rs` gates top-level modules; a module directory's `mod.rs` gates its own parts the same way (`src/app/mod.rs`
    // keeps the client layer's window, input, offscreen and shell parts behind `gfx` while its camera, session and HUD stay headless).
    let mut declaring: Vec<(String, String)> = vec![(String::new(), "src/lib.rs".to_string())];
    for f in features::repo_files(root) {
        if let Some(dir) = f.strip_prefix("src/").and_then(|r| r.strip_suffix("mod.rs")) {
            declaring.push((dir.to_string(), f.clone()));
        }
    }
    for (prefix, file) in declaring {
        let text = read(root, &file);
        let mut lines = text.lines();
        while let Some(l) = lines.next() {
            if is_graphics_gate(l) {
                if let Some(next) = lines.next() {
                    if let Some(name) = next.trim().strip_prefix("pub mod ").or_else(|| next.trim().strip_prefix("mod ")).and_then(|n| n.strip_suffix(';')) {
                        out.push(format!("{prefix}{name}.rs"));
                        out.push(format!("{prefix}{name}/"));
                    }
                }
            }
        }
    }
    let cargo = read(root, "Cargo.toml");
    for table in cargo.split("[[bin]]").skip(1) {
        let table = table.split("\n[").next().unwrap_or("");
        let value = |key: &str| table.lines().find_map(|l| l.trim().strip_prefix(key)?.trim_start().strip_prefix('=').map(|v| v.trim().to_string()));
        if value("required-features").is_some_and(|v| v.contains("\"gfx\"")) {
            if let Some(path) = value("path").map(|p| p.trim_matches('"').to_string()).and_then(|p| p.strip_prefix("src/").map(str::to_string)) {
                out.push(match path.strip_suffix("/main.rs") {
                    Some(dir) => format!("{dir}/"),
                    None => path,
                });
            }
        }
    }
    out
}

/// Names of graphics/audio crates that only `gfx`-gated code may use.
const BANNED_CRATES: &[&str] = &["wgpu::", "winit::", "rodio::", "ffmpeg_sidecar", "pollster::", "softbuffer::"];

/// Source files that are built without the `gfx` feature but name a graphics or audio crate (`file:line: crate`): they would break the headless server build
/// that CI does on a bare Linux box. Which files are graphics-only is read from `src/lib.rs`, every `mod.rs` under `src/` and `Cargo.toml`, never listed by hand.
pub fn headless_violations(root: &Path) -> Vec<String> {
    let gated = gfx_only_paths(root);
    let files: Vec<String> = features::repo_files(root).into_iter().filter(|f| f.starts_with("src/") && f.ends_with(".rs")).collect();
    let mut bad = Vec::new();
    for f in files {
        let rel = f.trim_start_matches("src/");
        if gated.iter().any(|g| if g.ends_with('/') { rel.starts_with(g.as_str()) } else { rel == g }) {
            continue;
        }
        let text = read(root, &f);
        for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.trim_start().starts_with("//")) {
            let code = code_only(line);
            if let Some(b) = BANNED_CRATES.iter().find(|b| code.contains(*b)) {
                bad.push(format!("{f}:{}: `{b}`", n + 1));
            }
        }
    }
    bad
}

/// `line` without its string literals' contents and without a trailing `//` comment: a crate named inside a string or a comment is not a dependency on it.
fn code_only(line: &str) -> String {
    let (mut out, mut in_str, mut escaped) = (String::new(), false, false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => {
                    in_str = false;
                    out.push('"');
                }
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push('"');
            }
            '/' if chars.peek() == Some(&'/') => break,
            _ => out.push(c),
        }
    }
    out
}

fn check_headless(root: &Path) -> Vec<Problem> {
    headless_violations(root)
        .into_iter()
        .map(|v| {
            problem(
                "headless",
                format!("{v} in a file that is built without the `gfx` feature"),
                "move the code into a module declared `#[cfg(feature = \"gfx\")] pub mod ...;` in src/lib.rs, or gate the item with #[cfg(feature = \"gfx\")]",
                None,
            )
        })
        .collect()
}

/// The inline-code tokens of `text` that name a file this repository should contain (`scripts/dev`, `docs/adr/0024-...md`).
pub fn referenced_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (n, span) in text.split('`').enumerate() {
        if n % 2 == 0 {
            continue; // outside backticks
        }
        for tok in span.split_whitespace() {
            let tok = tok.trim_matches(|c: char| ",.;:)(".contains(c));
            let known = ["scripts/", "docs/", "deploy/", "tests/", "recipes/", "assets/"].iter().any(|p| tok.starts_with(p));
            let skip = tok.contains(['*', '<', '>', '{', '$', '|']) || tok.contains("...") || tok.contains("NNNN") || tok.starts_with("scripts/red");
            if known && !skip {
                out.push(tok.to_string());
            }
        }
    }
    out
}

/// `doc: path` for every path the AI-facing documents mention that does not exist.
pub fn missing_referenced_paths(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for f in AI_DOCS.iter().chain(["docs/HOSTING.md"].iter()) {
        for p in referenced_paths(&read(root, f)) {
            let exists = root.join(&p).exists() || Path::new(&p).extension().is_none() && root.join(format!("{p}.md")).exists();
            if !exists {
                out.push(format!("{f} mentions `{p}`, which does not exist"));
            }
        }
    }
    out
}

/// Hand-written test counts (`123 tests`) in the front-door documents: they go stale the day they are written.
pub fn test_count_claims(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for f in AI_DOCS.iter().chain(["README.md"].iter()) {
        let text = read(root, f);
        let words: Vec<&str> = text.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            let next = words.get(i + 1).map(|n| n.trim_matches(|c: char| !c.is_alphanumeric())).unwrap_or("");
            let next2 = words.get(i + 2).map(|n| n.trim_matches(|c: char| !c.is_alphanumeric())).unwrap_or("");
            let counted = w.trim_end_matches('+').parse::<u32>().is_ok();
            let about_tests = next == "tests" || (next == "unit" && next2 == "tests");
            if counted && about_tests {
                out.push(format!("{f}: `{w} {next}...` is a hand-written test count; it goes stale. Say \"run scripts/dev test\" instead"));
            }
        }
    }
    out
}

/// Sentences the engine has disproved, still standing in the AI-facing documents.
pub fn stale_claims(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for f in AI_DOCS {
        let text = read(root, f).to_lowercase();
        for banned in ["multiplayer is **not built**", "**not built** (adr 0010)", "no networking at all", "a headless server is not built"] {
            if text.contains(banned) {
                out.push(format!("{f} still says `{banned}`; multiplayer is built (ADR 0016, 0017, 0022)"));
            }
        }
    }
    out
}

/// The commands (of `commands`) that neither `AGENTS.md` nor the tool reference mentions as `` `command ``.
pub fn unmentioned_commands(root: &Path, commands: &[(String, String)]) -> Vec<(String, String)> {
    let text = format!("{}\n{}", read(root, "AGENTS.md"), read(root, "docs/AGENT_REFERENCE.md"));
    commands.iter().filter(|(c, _)| !text.contains(&format!("`{c}")) && !text.contains(&format!("| `{c}"))).cloned().collect()
}

fn check_docs(root: &Path, commands: &[(String, String)]) -> Vec<Problem> {
    let mut out = Vec::new();
    for m in test_count_claims(root) {
        out.push(problem("docs", m, "delete the number from the sentence", None));
    }
    for m in stale_claims(root) {
        out.push(problem("docs", m, "rewrite the sentence so it is true", None));
    }
    for m in missing_referenced_paths(root) {
        out.push(problem("docs", m, "fix the path in the document, or create the file", None));
    }
    for (c, about) in unmentioned_commands(root, commands) {
        out.push(problem(
            "docs",
            format!("the command `{c}` is not mentioned in AGENTS.md or docs/AGENT_REFERENCE.md"),
            format!("docs/AGENT_REFERENCE.md: add a row to the tool table: | `{c}` | {about} | |"),
            Some(Fix::CommandRow { command: c, about }),
        ));
    }
    out
}

fn check_budgets(sizes: Option<(usize, usize)>) -> Vec<Problem> {
    let Some((brief, overview)) = sizes else { return Vec::new() };
    let mut out = Vec::new();
    if brief > super::describe::BRIEF_BUDGET {
        out.push(problem(
            "budget",
            format!("`describe --brief` is {brief} bytes, the budget is {} ({} over)", super::describe::BRIEF_BUDGET, brief - super::describe::BRIEF_BUDGET),
            "shorten the brief in src/tools/describe.rs: it is the first thing every session reads",
            None,
        ));
    }
    if overview > super::describe::OVERVIEW_BUDGET {
        out.push(problem(
            "budget",
            format!(
                "the `describe` overview is {overview} bytes, the budget is {} ({} over)",
                super::describe::OVERVIEW_BUDGET,
                overview - super::describe::OVERVIEW_BUDGET
            ),
            "shorten the `about` line of the newest commands (src/cli/args.rs doc comments) or a flag's help; the detail belongs in docs/AGENT_REFERENCE.md",
            None,
        ));
    }
    out
}

/// Text files under the indexed directories that contain a carriage return: the repository is LF everywhere (`.gitattributes`), because golden images, trace
/// checksums and the docs tests hash or compare file text. An editor or tool on Windows can leave CRLF in the working tree; git would fix it at the next commit
/// but the tests read the working tree.
pub fn crlf_files(root: &Path) -> Vec<String> {
    const TEXT: &[&str] = &["rs", "md", "json", "toml", "sh", "yml", "yaml", "wgsl", "py", "ps1", "txt", "lock"];
    features::repo_files(root)
        .into_iter()
        .filter(|f| f.rsplit_once('.').is_some_and(|(_, ext)| TEXT.contains(&ext)))
        .filter(|f| std::fs::read(root.join(f)).is_ok_and(|b| b.contains(&b'\r')))
        .collect()
}

fn check_line_endings(root: &Path) -> Vec<Problem> {
    crlf_files(root)
        .into_iter()
        .map(|f| {
            problem(
                "lf",
                format!("{f} has CRLF line endings (the repository is LF, see .gitattributes)"),
                "rewrite the file with LF line endings",
                Some(Fix::NormalizeLf(f)),
            )
        })
        .collect()
}

fn check_fmt(root: &Path) -> Option<Vec<Problem>> {
    let out = Command::new("cargo").args(["fmt", "--check"]).current_dir(root).output().ok()?;
    if out.status.success() {
        return Some(Vec::new());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let diffs = text.lines().filter(|l| l.starts_with("Diff in ")).count();
    if diffs == 0 && !String::from_utf8_lossy(&out.stderr).trim().is_empty() && !text.contains("Diff") {
        return None; // rustfmt could not run at all (not installed, broken toolchain): not a finding
    }
    Some(vec![problem("fmt", format!("rustfmt would change {diffs} place(s)"), "cargo fmt", Some(Fix::CargoFmt))])
}

/// Runs the checks. Nothing is compiled; on this repository it takes a fraction of a second (a second or two with `fmt`).
pub fn run(root: &Path, opts: &Options) -> Report {
    let started = Instant::now();
    let mut r = Report::default();
    let mut add = |name: &'static str, mut found: Vec<Problem>| {
        r.ran.push(name);
        r.problems.append(&mut found);
    };
    add("adr", check_adr(root));
    add("features", check_features(root));
    add("facts", check_facts(root));
    add("headless", check_headless(root));
    add("docs", check_docs(root, if opts.tree { &[] } else { &opts.commands }));
    add("lf", check_line_endings(root));
    if opts.tree {
        r.skipped.extend(["the CLI command table", "the describe byte budgets"]);
    } else if opts.describe_sizes.is_some() {
        add("budget", check_budgets(opts.describe_sizes));
    }
    if opts.fmt {
        if let Some(found) = check_fmt(root) {
            add("fmt", found);
        }
    }
    r.millis = started.elapsed().as_millis();
    r
}

/// Applies the mechanical fixes of `fixes` (each once). Returns one line per edit made.
pub fn apply(root: &Path, fixes: &[Fix]) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    let mut seen: Vec<&Fix> = Vec::new();
    for fix in fixes {
        if seen.contains(&fix) {
            continue;
        }
        seen.push(fix);
        match fix {
            Fix::AdrIndex => {
                if adr::write_index(root)? {
                    done.push("regenerated the index of docs/adr/README.md".to_string());
                }
            }
            Fix::SyncFacts(doc) => {
                let path = root.join(doc);
                let text = std::fs::read_to_string(&path).map_err(|e| format!("{doc}: {e}"))?;
                let new = status::sync_facts(&text, &status::facts_block(root)).map_err(|e| format!("{doc}: {e}"))?;
                if new != text {
                    std::fs::write(&path, new).map_err(|e| format!("{doc}: {e}"))?;
                    done.push(format!("rewrote the facts block of {doc}"));
                }
            }
            Fix::InlineFacts(doc) => {
                let path = root.join(doc);
                let text = std::fs::read_to_string(&path).map_err(|e| format!("{doc}: {e}"))?;
                let new = status::sync_inline_facts(root, &text);
                if new != text {
                    std::fs::write(&path, new).map_err(|e| format!("{doc}: {e}"))?;
                    done.push(format!("updated the inline facts of {doc}"));
                }
            }
            Fix::FeatureArray { feature, key, item } => {
                let path = root.join("docs/features.json");
                let text = std::fs::read_to_string(&path).map_err(|e| format!("docs/features.json: {e}"))?;
                let new = features::add_to_array(&text, feature, key, item)?;
                features::parse(&new).map_err(|e| format!("the edit would leave docs/features.json invalid: {e}"))?;
                if new != text {
                    std::fs::write(&path, new).map_err(|e| format!("docs/features.json: {e}"))?;
                    done.push(format!("added \"{item}\" to features.{feature}.{key}"));
                }
            }
            Fix::CommandRow { command, about } => {
                let path = root.join("docs/AGENT_REFERENCE.md");
                let text = std::fs::read_to_string(&path).map_err(|e| format!("docs/AGENT_REFERENCE.md: {e}"))?;
                if text.contains(&format!("| `{command}")) {
                    continue;
                }
                let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
                let lines: Vec<&str> = text.split(nl).collect();
                let Some(last_row) = lines.iter().rposition(|l| l.starts_with("| `")) else {
                    return Err("docs/AGENT_REFERENCE.md has no tool table to add a row to".into());
                };
                let row = format!("| `{command}` | {} | |", about.replace('|', "/"));
                let mut out: Vec<&str> = lines.clone();
                out.insert(last_row + 1, &row);
                std::fs::write(&path, out.join(nl)).map_err(|e| format!("docs/AGENT_REFERENCE.md: {e}"))?;
                done.push(format!("added a row for `{command}` to the tool table of docs/AGENT_REFERENCE.md (improve its description)"));
            }
            Fix::CargoFmt => {
                let ok = Command::new("cargo").arg("fmt").current_dir(root).status().map(|s| s.success()).unwrap_or(false);
                if !ok {
                    return Err("`cargo fmt` failed".into());
                }
                done.push("ran cargo fmt".to_string());
            }
            Fix::NormalizeLf(file) => {
                let path = root.join(file);
                let bytes = std::fs::read(&path).map_err(|e| format!("{file}: {e}"))?;
                let mut fixed = Vec::with_capacity(bytes.len());
                for (i, b) in bytes.iter().enumerate() {
                    if *b == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                        continue;
                    }
                    fixed.push(*b);
                }
                if fixed != bytes {
                    std::fs::write(&path, fixed).map_err(|e| format!("{file}: {e}"))?;
                    done.push(format!("rewrote {file} with LF line endings"));
                }
            }
        }
    }
    Ok(done)
}

/// The report as text: each problem, the exact edit, and what `--fix` would do.
pub fn render(r: &Report) -> String {
    let mut checked = r.ran.join(", ");
    if !r.skipped.is_empty() {
        checked.push_str(&format!("; NOT checked, they need a current build: {} (run `scripts/dev preflight --full`)", r.skipped.join(", ")));
    }
    if r.problems.is_empty() {
        return format!("preflight: OK in {} ms (checked {checked})\n", r.millis);
    }
    let fixable = r.problems.iter().filter(|p| p.fix.is_some()).count();
    let mut s = format!("preflight: {} problem(s) in {} ms (checked {checked})\n", r.problems.len(), r.millis);
    for (i, p) in r.problems.iter().enumerate() {
        s.push_str(&format!("{}. [{}] {}\n     edit: {}{}\n", i + 1, p.check, p.message, p.edit, if p.fix.is_some() { "   (--fix does this)" } else { "" }));
    }
    if fixable > 0 {
        s.push_str(&format!("`red_engine2 preflight --fix` makes the {fixable} mechanical edit(s); the rest need a person.\n"));
    }
    s
}

/// The report as JSON (the `data` of the `--json` envelope).
pub fn to_json(r: &Report) -> Value {
    json!({
        "ok": r.problems.is_empty(),
        "millis": r.millis,
        "checked": r.ran,
        "skipped": r.skipped,
        "problems": r.problems.iter().map(|p| json!({"check": p.check, "message": p.message, "edit": p.edit, "fixable": p.fix.is_some()})).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny repository with everything preflight looks at.
    fn repo(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("re2_preflight_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for dir in ["src/net", "src/tools", "src/bin/win", "docs/adr", "tests", "scripts"] {
            std::fs::create_dir_all(d.join(dir)).unwrap();
        }
        let w = |p: &str, t: &str| std::fs::write(d.join(p), t).unwrap();
        w("scripts/dev", "#!/bin/sh\n"); // the facts block points at it
        w("Cargo.toml", "[package]\nname = \"demo\"\n\n[features]\ndefault = [\"gfx\"]\ngfx = []\n\n[[bin]]\nname = \"win\"\npath = \"src/bin/win/main.rs\"\nrequired-features = [\"gfx\"]\n");
        w("src/lib.rs", "#[cfg(feature = \"gfx\")]\npub mod render;\n#[cfg(feature = \"render\")]\npub mod mesh;\npub mod net;\n");
        w("src/render.rs", "use wgpu::Device;\n");
        w("src/bin/win/main.rs", "use winit::window::Window;\n");
        w("src/net/protocol.rs", "pub const PROTOCOL_VERSION: u16 = 5;\n");
        w("src/tools/a.rs", "//! a\n");
        w("src/tools/b.rs", "//! b\n");
        w("docs/features.json", "{\n  \"serial_suites\": [],\n  \"features\": {\n    \"tools\": {\n      \"summary\": \"t\",\n      \"files\": [\"src/tools/a.rs\", \"src/tools/b.rs\", \"src/lib.rs\",\"src/render.rs\", \"src/net/protocol.rs\", \"src/bin/win/main.rs\", \"Cargo.toml\"],\n      \"tests\": []\n    }\n  }\n}\n");
        w("docs/adr/README.md", &format!("# ADRs\n{}\n{}\n", adr::INDEX_BEGIN, adr::INDEX_END));
        w("CLAUDE.md", "# c\n<!-- facts:begin -->\nold\n<!-- facts:end -->\nProtocol v<!--fact:protocol-->4<!--/fact-->.\n");
        w("AGENTS.md", "commands: `lint`\n");
        w("docs/AGENT_REFERENCE.md", "| `lint <scene>` | finds bugs | |\n| `walk <scene>` | walks | |\n");
        d
    }

    fn opts(cmds: &[(&str, &str)]) -> Options {
        Options { fmt: false, commands: cmds.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), describe_sizes: None, tree: false }
    }

    #[test]
    fn a_tree_only_run_skips_exactly_the_checks_that_read_the_compiled_binary_and_says_so() {
        let d = repo("tree");
        let huge = Some((usize::MAX / 2, usize::MAX / 2));
        let full = run(&d, &Options { describe_sizes: huge, ..opts(&[]) });
        assert!(full.skipped.is_empty() && full.ran.contains(&"budget"), "{full:?}");
        assert!(full.problems.iter().any(|p| p.check == "budget"), "a compiled-in describe that is too big is a problem in a full run");
        let tree = run(&d, &Options { describe_sizes: huge, tree: true, ..opts(&[("not-in-the-table", "x")]) });
        assert!(!tree.ran.contains(&"budget") && tree.problems.iter().all(|p| p.check != "budget"), "{tree:?}");
        assert!(
            tree.problems.iter().all(|p| p.check != "docs" || !p.message.contains("not-in-the-table")),
            "the command table is not consulted: {:?}",
            tree.problems
        );
        for check in ["adr", "features", "facts", "headless", "docs", "lf"] {
            assert!(tree.ran.contains(&check), "{check} still runs from the tree: {:?}", tree.ran);
        }
        let text = render(&tree);
        assert!(text.contains("NOT checked") && text.contains("scripts/dev preflight --full"), "{text}");
        assert_eq!(to_json(&tree)["skipped"].as_array().map(Vec::len), Some(2));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn graphics_only_code_is_read_from_lib_rs_and_cargo_toml_not_listed() {
        let d = repo("gfx");
        let g = gfx_only_paths(&d);
        assert!(g.contains(&"render.rs".to_string()) && g.contains(&"bin/win/".to_string()), "{g:?}");
        assert!(g.contains(&"mesh.rs".to_string()), "a module behind `render` is not in the headless build either: {g:?}");
        assert!(headless_violations(&d).is_empty(), "{:?}", headless_violations(&d));
        std::fs::write(
            d.join("src/net/leak.rs"),
            "fn f() { let _ = wgpu::Backends::all(); }\n// wgpu:: in a comment is fine\nfn g() -> &'static str { \"winit::\" } // and rodio:: after code\n",
        )
        .unwrap();
        assert_eq!(headless_violations(&d), vec!["src/net/leak.rs:1: `wgpu::`"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn every_problem_names_the_exact_edit_and_fix_makes_the_mechanical_ones() {
        let d = repo("fix");
        std::fs::write(d.join("src/tools/c.rs"), "//! c\n").unwrap(); // unowned; its neighbour a.rs belongs to `tools`
        std::fs::write(
            d.join("docs/adr/2026-09-28-x.md"),
            "# 2026-09-28. X\nStatus: accepted\nSummary: s\n\n## Context\n.\n## Decision\n.\n## Consequences\n.\n",
        )
        .unwrap();
        let cmds = [("lint", "l"), ("walk", "w"), ("preflight", "checks the bookkeeping")];
        let r = run(&d, &opts(&cmds));
        let by = |c: &str| r.problems.iter().filter(|p| p.check == c).collect::<Vec<_>>();
        assert_eq!(by("adr").len(), 1, "the index lacks the new ADR: {:?}", r.problems);
        let f = by("features");
        assert!(f.iter().any(|p| p.message.contains("src/tools/c.rs") && p.edit.contains("features.tools.files += \"src/tools/c.rs\"")), "{f:?}");
        assert!(by("facts").len() >= 2, "the facts block and the inline protocol fact are stale: {:?}", by("facts"));
        assert!(by("docs").iter().any(|p| p.message.contains("`preflight`")), "{:?}", by("docs"));
        assert!(r.problems.iter().all(|p| !p.edit.is_empty()), "every problem says what to edit");
        let text = render(&r);
        assert!(text.contains("--fix does this") && text.contains("[features] src/tools/c.rs belongs to no feature"), "{text}");

        let fixes: Vec<Fix> = r.problems.iter().filter_map(|p| p.fix.clone()).collect();
        let done = apply(&d, &fixes).unwrap();
        assert!(done.len() >= 4, "{done:?}");
        let again = run(&d, &opts(&cmds));
        assert!(again.problems.is_empty(), "fix left: {}", render(&again));
        assert!(std::fs::read_to_string(d.join("CLAUDE.md")).unwrap().contains("<!--fact:protocol-->5<!--/fact-->"));
        assert!(std::fs::read_to_string(d.join("docs/AGENT_REFERENCE.md")).unwrap().contains("| `preflight` | checks the bookkeeping | |"));
        assert_eq!(apply(&d, &fixes).unwrap().len(), 0, "a second --fix has nothing to do");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_describe_budgets_say_how_far_over_they_are() {
        let over = check_budgets(Some((super::super::describe::BRIEF_BUDGET + 10, super::super::describe::OVERVIEW_BUDGET + 154)));
        assert_eq!(over.len(), 2);
        assert!(over[1].message.contains("154 over"), "{}", over[1].message);
        assert!(check_budgets(Some((1, 1))).is_empty() && check_budgets(None).is_empty());
    }

    #[test]
    fn hand_written_test_counts_and_stale_claims_are_found() {
        let d = repo("prose");
        std::fs::write(d.join("AGENTS.md"), "We have 95+ tests. `lint`. Multiplayer is **not built**.\n").unwrap();
        assert_eq!(test_count_claims(&d).len(), 1);
        assert_eq!(stale_claims(&d).len(), 1);
        std::fs::write(d.join("AGENTS.md"), "see `docs/nowhere.md` and `docs/adr/README.md` and `docs/adr/NNNN-x.md`\n").unwrap();
        assert_eq!(missing_referenced_paths(&d), vec!["AGENTS.md mentions `docs/nowhere.md`, which does not exist"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn carriage_returns_in_text_files_are_found_and_removed() {
        let d = repo("crlf");
        std::fs::write(d.join("src/tools/a.rs"), "//! a\r\nfn f() {}\r\n").unwrap();
        std::fs::write(d.join("docs/notes.txt"), "one\r\ntwo\r\n").unwrap();
        std::fs::write(d.join("docs/logo.png"), b"\x89PNG\r\n\x1a\n").unwrap(); // binary: not a text file, never touched
        assert_eq!(crlf_files(&d), vec!["src/tools/a.rs".to_string(), "docs/notes.txt".to_string()]);
        let r = run(&d, &opts(&[]));
        let lf: Vec<&Problem> = r.problems.iter().filter(|p| p.check == "lf").collect();
        assert_eq!(lf.len(), 2, "{:?}", r.problems);
        let fixes: Vec<Fix> = lf.iter().filter_map(|p| p.fix.clone()).collect();
        assert_eq!(apply(&d, &fixes).unwrap().len(), 2);
        assert_eq!(std::fs::read(d.join("src/tools/a.rs")).unwrap(), b"//! a\nfn f() {}\n");
        assert_eq!(std::fs::read(d.join("docs/notes.txt")).unwrap(), b"one\ntwo\n");
        assert!(crlf_files(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn json_report_carries_the_same_facts() {
        let d = repo("json");
        let r = run(&d, &opts(&[("lint", "l")]));
        let v = to_json(&r);
        assert_eq!(v["ok"], r.problems.is_empty());
        assert!(v["checked"].as_array().unwrap().len() >= 5);
        let _ = std::fs::remove_dir_all(&d);
    }
}
