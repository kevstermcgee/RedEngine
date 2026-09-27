//! `red_engine2 affected`: **change -> affected subsystems -> the minimal high-confidence verification -> full CI only at integration boundaries.**
//!
//! `impact` (ADR 0033) already knows which features a changed file belongs to and which features are built on those. This module turns that into
//! a *plan of commands* and can run it, so an agent does not have to choose between "run everything" (minutes) and "guess" (unsafe):
//!
//! | step | what | when |
//! |---|---|---|
//! | `fmt` | `cargo fmt --check` | any `.rs` changed |
//! | `clippy` | `cargo clippy -D warnings` on only the changed targets (lib + bins, or one bin, or the changed test/bench/example) | any `.rs` changed |
//! | `unit` | `cargo test --lib -- <module filters>` derived from the changed paths **and** the owning features' `lib:` entries | lib changed / feature owns unit tests |
//! | `bin` | `cargo test --bin X` | a bin's own files changed |
//! | `doc` | `cargo test --doc -- <module filters>` | lib changed |
//! | `suites` | `cargo test --test A --test B` (parallel threads) | owning features' integration suites (+ dependents unless `--quick`) |
//! | `suites-serial` | the same with `RUST_TEST_THREADS=1` | the suites listed under `serial_suites` (real-time UDP) |
//!
//! **Escalation.** Some changes cannot be verified by a subset: `Cargo.toml`/`Cargo.lock`, `src/lib.rs`, `rustfmt.toml`, `.cargo/`, `scripts/ci.sh`, a very
//! large diff, or an affected set that is most of the suite. Those (and `--full`) plan exactly one step: `bash scripts/ci.sh`, the same as CI.
//!
//! **Tiers.** `--quick` verifies the features that *own* the changed files (the inner edit loop, seconds); the default also verifies every feature built on
//! them (before you say "done"); `--full` is CI (before you push, or at any integration boundary). What a tier did not run is listed, never silent.
//!
//! **Green stamps.** A passing run is recorded in `out/.affected-green.json` under a hash of the *contents* of every changed file, the scope and the feature
//! index. Asking again with identical inputs costs nothing ("already verified"); any edit changes the hash. Failures are never cached.
//!
//! Planning is pure (`plan`), so it is unit-tested without running cargo; `run` executes a plan with one log file per step and prints only summaries.

use super::features::{self, Feature};
use crate::crypto;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

/// How much verification to plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// Only the features that own a changed file (the edit loop).
    Quick,
    /// Owners plus every feature built on them (before "done").
    Closure,
    /// Everything CI runs (`scripts/ci.sh`).
    Full,
}

impl Scope {
    /// Stable name (`quick`, `closure`, `full`), used in stamps and output.
    pub fn name(self) -> &'static str {
        match self {
            Scope::Quick => "quick",
            Scope::Closure => "closure",
            Scope::Full => "full",
        }
    }
}

/// One command of a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Short name (`fmt`, `clippy`, `unit`, `suites`, ...).
    pub name: String,
    /// Program and arguments.
    pub argv: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
    /// Why this step is in the plan (one line).
    pub why: String,
}

impl Step {
    fn new(name: &str, argv: &[&str], why: impl Into<String>) -> Step {
        Step { name: name.to_string(), argv: argv.iter().map(|s| s.to_string()).collect(), env: Vec::new(), why: why.into() }
    }
    /// The command as one shell-looking line.
    pub fn command_line(&self) -> String {
        let env: String = self.env.iter().map(|(k, v)| format!("{k}={v} ")).collect();
        format!("{env}{}", self.argv.join(" "))
    }
}

/// What to verify and why.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The scope actually planned (may be higher than requested after escalation).
    pub scope: Scope,
    /// Changed files considered (ignored paths removed).
    pub changed: Vec<String>,
    /// Commands to run, in order (cheapest and most likely to fail first).
    pub steps: Vec<Step>,
    /// Why the plan escalated to a full CI run, if it did.
    pub escalated: Option<String>,
    /// Integration suites that a smaller scope left out (`--quick` skips dependents): run without `--quick` before integrating.
    pub deferred: Vec<String>,
    /// Things worth knowing (an unowned file, a Docker-only change, ...).
    pub notes: Vec<String>,
    /// Verification commands from the affected features that need eyes (`frame`, `verify ...`): suggestions, not run.
    pub suggest: Vec<String>,
}

/// Paths that never need verification (generated output, logs, the handoff file).
fn ignorable(path: &str) -> bool {
    path.starts_with("out/")
        || path.starts_with("target/")
        || path.starts_with(".gauntlet/")
        || path == "STATUS.md"
        || path == ".gitignore"
        || path.starts_with(".git/")
}

/// A changed file that no subset of tests can vouch for: it changes how *everything* builds or is checked.
fn boundary_reason(path: &str) -> Option<&'static str> {
    match path {
        "Cargo.toml" | "Cargo.lock" | "build.rs" => Some("dependencies or build settings changed"),
        "rustfmt.toml" => Some("the formatting rules changed"),
        "src/lib.rs" => Some("the crate root changed (every module and feature gate hangs off it)"),
        "scripts/ci.sh" | ".github/workflows/ci.yml" => Some("CI itself changed"),
        p if p.starts_with(".cargo/") => Some("cargo configuration changed"),
        _ => None,
    }
}

/// The cargo target a changed Rust file belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Target {
    Lib,
    Bin(String),
    Test(String),
    Bench(String),
    Example(String),
}

fn target_of(path: &str) -> Option<Target> {
    if !path.ends_with(".rs") {
        return None;
    }
    let seg: Vec<&str> = path.split('/').collect();
    match seg.as_slice() {
        ["src", "main.rs"] | ["src", "cli", ..] => Some(Target::Bin("red_engine2".into())),
        ["src", "bin", "re2", ..] => Some(Target::Bin("re2".into())),
        ["src", "bin", file] => Some(Target::Bin(file.trim_end_matches(".rs").to_string())),
        ["src", ..] => Some(Target::Lib),
        ["tests", file] => Some(Target::Test(file.trim_end_matches(".rs").to_string())),
        ["benches", "sim.rs"] | ["benches", "common", ..] => Some(Target::Bench("sim".into())),
        ["benches", file] => Some(Target::Bench(file.trim_end_matches(".rs").to_string())),
        ["examples", file] => Some(Target::Example(file.trim_end_matches(".rs").to_string())),
        _ => None,
    }
}

/// The library test filter for a changed source file: `src/sim/flow.rs` -> `sim::flow`, `src/net/mod.rs` -> `net`. `None` for files outside the lib.
fn lib_filter(path: &str) -> Option<String> {
    let rel = path.strip_prefix("src/")?.strip_suffix(".rs")?;
    if rel.starts_with("bin/") || rel.starts_with("cli/") || rel == "main" || rel == "lib" {
        return None;
    }
    let rel = rel.strip_suffix("/mod").unwrap_or(rel);
    Some(rel.replace('/', "::"))
}

/// Drops every module filter that a shorter one already covers (`tools::lint` when `tools` is present): test filters match by substring of the path.
fn minimal_filters(set: &BTreeSet<String>) -> Vec<String> {
    set.iter().filter(|f| !set.iter().any(|g| g != *f && (f.starts_with(&format!("{g}::")) || f.as_str() == g.as_str()))).cloned().collect()
}

/// Everything the planner can be told besides the changed files.
#[derive(Debug, Clone)]
pub struct Options {
    /// Verify only the features that own the changed files.
    pub quick: bool,
    /// Plan a full CI run regardless of the change.
    pub full: bool,
    /// Above this many changed files a subset stops being meaningful: escalate.
    pub max_files: usize,
    /// Escalate when the suites to run are at least this fraction of all suites (percent).
    pub escalate_percent: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { quick: false, full: false, max_files: 60, escalate_percent: 75 }
    }
}

/// Every integration suite the index knows about.
fn all_suites(all: &[Feature]) -> BTreeSet<String> {
    all.iter().flat_map(|f| f.tests.iter()).filter(|t| !t.starts_with("lib:")).cloned().collect()
}

fn full_plan(changed: Vec<String>, why: String) -> Plan {
    Plan {
        scope: Scope::Full,
        changed,
        steps: vec![Step::new("ci", &["bash", "scripts/ci.sh"], "fmt, clippy, every test, benches compile, headless build: what CI runs")],
        escalated: Some(why),
        deferred: Vec::new(),
        notes: Vec::new(),
        suggest: Vec::new(),
    }
}

/// Plans the verification for `changed` files. `serial` lists the suites that must not share a process with other tests (`serial_suites` in the index).
pub fn plan(all: &[Feature], serial: &[String], changed: &[String], opts: &Options) -> Plan {
    let changed: Vec<String> = changed.iter().map(|c| c.replace('\\', "/")).filter(|c| !ignorable(c)).collect();
    if opts.full {
        return full_plan(changed, "requested with --full".into());
    }
    if let Some((f, why)) = changed.iter().find_map(|c| boundary_reason(c).map(|w| (c, w))) {
        return full_plan(changed.clone(), format!("{f}: {why}, so no subset of tests can vouch for it"));
    }
    if changed.len() > opts.max_files {
        let n = changed.len();
        return full_plan(changed, format!("{n} changed files (more than {}): a subset is no longer meaningful", opts.max_files));
    }

    let mut plan = Plan {
        scope: if opts.quick { Scope::Quick } else { Scope::Closure },
        changed: changed.clone(),
        steps: Vec::new(),
        escalated: None,
        deferred: Vec::new(),
        notes: Vec::new(),
        suggest: Vec::new(),
    };
    if changed.is_empty() {
        plan.notes.push("nothing to verify: no changed files".into());
        return plan;
    }

    // A top-level test file can only break itself: it runs its own suite and does not drag in its owning features. Everything else goes through `impact`.
    let is_test_file = |c: &str| matches!(target_of(c), Some(Target::Test(_)));
    let (test_files, others): (Vec<String>, Vec<String>) = changed.iter().cloned().partition(|c| is_test_file(c));
    let imp = features::impact(all, &others);
    let names_of = |set: &BTreeSet<String>| -> Vec<&Feature> { all.iter().filter(|f| set.contains(&f.name)).collect() };
    let direct: BTreeSet<String> = imp.direct.keys().cloned().collect();
    let mut wanted = direct.clone();
    let mut deferred_from = BTreeSet::new();
    if opts.quick {
        deferred_from = imp.downstream.clone();
    } else {
        wanted.extend(imp.downstream.iter().cloned());
    }

    // Tests from the owning features, plus what the changed paths say directly (a feature entry may be too coarse or missing).
    let mut lib_filters: BTreeSet<String> = BTreeSet::new();
    let mut suites: BTreeSet<String> = BTreeSet::new();
    let mut deferred: BTreeSet<String> = BTreeSet::new();
    for f in names_of(&wanted) {
        for t in &f.tests {
            match t.strip_prefix("lib:") {
                Some(filter) => lib_filters.insert(filter.to_string()),
                None => suites.insert(t.clone()),
            };
        }
    }
    // Only the features that own a changed file suggest a look: the dependents' commands are what their suites already run.
    for f in names_of(&direct) {
        plan.suggest.extend(f.commands.iter().filter(|c| c.starts_with("red_engine2 ")).cloned());
    }
    for f in names_of(&deferred_from) {
        deferred.extend(f.tests.iter().filter(|t| !t.starts_with("lib:")).cloned());
    }

    let mut targets: BTreeSet<Target> = BTreeSet::new();
    let mut doc_filters: BTreeSet<String> = BTreeSet::new();
    let mut unowned_code = false;
    for c in &test_files {
        if features::owners(all, c).is_empty() {
            unowned_code = true;
        }
    }
    for c in &changed {
        if let Some(t) = target_of(c) {
            if let Target::Test(name) = &t {
                suites.insert(name.clone()); // a changed test always runs itself
            }
            targets.insert(t);
        }
        if let Some(m) = lib_filter(c) {
            lib_filters.insert(m.clone());
            doc_filters.insert(m);
        }
        if imp.unowned.contains(c) && (c.starts_with("src/") || c.starts_with("tests/") || c.starts_with("benches/")) {
            unowned_code = true;
        }
    }
    if unowned_code {
        suites.insert("features_index".into());
        plan.notes.push(format!("new or unlisted source file(s) {}: add them to docs/features.json (features_index will say where)", imp.unowned.join(", ")));
    } else if !imp.unowned.is_empty() {
        plan.notes.push(format!("no automated check covers: {} (they are not in docs/features.json)", imp.unowned.join(", ")));
    }
    if changed.iter().any(|c| c == "Dockerfile" || c == "docker-compose.yml" || c.starts_with("deploy/")) {
        plan.notes.push("Dockerfile/deploy changes are only exercised by the `container image` CI job (docker build + start)".into());
    }
    deferred.retain(|s| !suites.contains(s));

    // Too much of the suite is affected: running "the subset" is running the suite, so run it exactly as CI does.
    let total = all_suites(all).len().max(1);
    if !opts.quick && suites.len() * 100 >= total * opts.escalate_percent {
        return full_plan(changed, format!("{} of {total} integration suites are affected: run it all, as CI does", suites.len()));
    }

    let has_rs = changed.iter().any(|c| c.ends_with(".rs"));
    if has_rs {
        plan.steps.push(Step::new("fmt", &["cargo", "fmt", "--check"], "a Rust file changed"));
        let mut argv: Vec<String> = ["cargo", "clippy", "--locked"].iter().map(|s| s.to_string()).collect();
        let lib_changed = targets.contains(&Target::Lib);
        if lib_changed {
            // A lib change can break any bin at compile time, and checking them is cheap next to the lib itself.
            argv.extend(["--lib".into(), "--bins".into()]);
        }
        for t in &targets {
            match t {
                Target::Lib => {}
                Target::Bin(b) if !lib_changed => argv.extend(["--bin".into(), b.clone()]),
                Target::Bin(_) => {}
                Target::Test(n) => argv.extend(["--test".into(), n.clone()]),
                Target::Bench(n) => argv.extend(["--bench".into(), n.clone()]),
                Target::Example(n) => argv.extend(["--example".into(), n.clone()]),
            }
        }
        argv.extend(["--".into(), "-D".into(), "warnings".into()]);
        plan.steps.push(Step {
            name: "clippy".into(),
            argv,
            env: Vec::new(),
            why: "warnings are errors in CI; only the changed targets (test-module lints run in the full tier)".into(),
        });
    }

    let lib_filters = minimal_filters(&lib_filters);
    let doc_filters = minimal_filters(&doc_filters);
    if !lib_filters.is_empty() {
        let mut argv: Vec<String> = ["cargo", "test", "--locked", "--lib", "--"].iter().map(|s| s.to_string()).collect();
        argv.extend(lib_filters.iter().cloned());
        plan.steps.push(Step { name: "unit".into(), argv, env: Vec::new(), why: format!("unit tests of {}", lib_filters.join(", ")) });
    }
    for t in &targets {
        if let Target::Bin(b) = t {
            plan.steps.push(Step::new("bin", &["cargo", "test", "--locked", "--bin", b], format!("unit tests inside the {b} binary")));
        }
    }
    if targets.contains(&Target::Lib) && !doc_filters.is_empty() {
        let mut argv: Vec<String> = ["cargo", "test", "--locked", "--doc", "--"].iter().map(|s| s.to_string()).collect();
        argv.extend(doc_filters.iter().cloned());
        plan.steps.push(Step { name: "doc".into(), argv, env: Vec::new(), why: "doc examples of the changed modules".into() });
    }
    let (serial_suites, parallel_suites): (Vec<&String>, Vec<&String>) = suites.iter().partition(|s| serial.contains(s));
    let suite_argv = |list: &[&String]| -> Vec<String> {
        let mut a: Vec<String> = ["cargo", "test", "--locked"].iter().map(|s| s.to_string()).collect();
        for s in list {
            a.extend(["--test".into(), (*s).clone()]);
        }
        a
    };
    if !parallel_suites.is_empty() {
        let list: Vec<&str> = parallel_suites.iter().map(|s| s.as_str()).collect();
        plan.steps.push(Step {
            name: "suites".into(),
            argv: suite_argv(&parallel_suites),
            env: Vec::new(),
            why: format!("integration suites: {}", list.join(", ")),
        });
    }
    if !serial_suites.is_empty() {
        let list: Vec<&str> = serial_suites.iter().map(|s| s.as_str()).collect();
        plan.steps.push(Step {
            name: "suites-serial".into(),
            argv: suite_argv(&serial_suites),
            env: vec![("RUST_TEST_THREADS".into(), "1".into())],
            why: format!("real-time network suites, one test at a time: {}", list.join(", ")),
        });
    }
    plan.deferred = deferred.into_iter().collect();
    plan.suggest.sort();
    plan.suggest.dedup();
    plan
}

/// Drops the `doc` step when none of the changed library files contains a doc example (a fenced block inside a `///` or `//!` comment): starting
/// rustdoc to run zero doctests costs seconds. Files that cannot be read keep the step.
pub fn prune_doc_step(plan: &mut Plan, root: &Path) {
    let has_example = |rel: &String| -> bool {
        if !rel.starts_with("src/") || !rel.ends_with(".rs") || matches!(target_of(rel), Some(Target::Bin(_))) {
            return false;
        }
        match std::fs::read_to_string(root.join(rel)) {
            Ok(text) => text.lines().any(|l| {
                let t = l.trim_start();
                (t.starts_with("///") || t.starts_with("//!")) && t.contains("```")
            }),
            Err(_) => true,
        }
    };
    if plan.steps.iter().any(|s| s.name == "doc") && !plan.changed.iter().any(has_example) {
        plan.steps.retain(|s| s.name != "doc");
    }
}

/// The result of running one step.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// The step's name.
    pub name: String,
    /// Whether it exited 0.
    pub ok: bool,
    /// Wall-clock seconds.
    pub secs: f64,
    /// `N passed, F failed, I ignored` summed over the `test result:` lines, when there were any.
    pub tally: Option<String>,
    /// The few lines that explain a failure.
    pub failures: Vec<String>,
    /// Where the full output went.
    pub log: PathBuf,
}

/// Sums the `test result:` lines of a cargo test log.
pub fn tally(log: &str) -> Option<String> {
    let (mut p, mut f, mut i, mut any) = (0u64, 0u64, 0u64, false);
    for l in log.lines().filter(|l| l.starts_with("test result:")) {
        any = true;
        let num = |after: &str| -> u64 { l.split(after).next().and_then(|h| h.split_whitespace().last()).and_then(|n| n.parse().ok()).unwrap_or(0) };
        p += num(" passed");
        f += num(" failed");
        i += num(" ignored");
    }
    any.then(|| format!("{p} passed, {f} failed, {i} ignored"))
}

/// The lines of a failed step's log that say what went wrong (compiler errors with locations, failed tests, panics, formatting diffs), bounded.
pub fn failure_lines(log: &str, max: usize) -> Vec<String> {
    let lines: Vec<&str> = log.lines().collect();
    let mut out: Vec<String> = Vec::new();
    for (n, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        let keep = (t.starts_with("test ") && t.ends_with("FAILED"))
            || t.starts_with("error")
            || t.starts_with("--> ")
            || t.starts_with("Diff in ")
            || t.contains("panicked at")
            || t == "failures:";
        if keep {
            out.push(l.trim_end().to_string());
            if l.contains("panicked at") {
                if let Some(next) = lines.get(n + 1).filter(|x| !x.trim().is_empty()) {
                    out.push(format!("  {}", next.trim()));
                }
            }
        }
        if out.len() >= max {
            break;
        }
    }
    if out.is_empty() {
        out.extend(lines.iter().rev().take(6).rev().map(|l| l.to_string()));
    }
    out
}

/// The program to launch for `name`. On Windows a bare `bash` may be the WSL launcher (which has no cargo and a different filesystem view), so prefer
/// the bash that ships with Git for Windows (`RED_BASH` overrides).
fn program_for(name: &str) -> String {
    if name != "bash" || !cfg!(windows) {
        return name.to_string();
    }
    if let Ok(b) = std::env::var("RED_BASH") {
        return b;
    }
    if let Ok(o) = Command::new("git").arg("--exec-path").output() {
        let p = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
        for a in p.ancestors() {
            for rel in ["bin/bash.exe", "usr/bin/bash.exe"] {
                let c = a.join(rel);
                if c.is_file() {
                    return c.to_string_lossy().to_string();
                }
            }
        }
    }
    name.to_string()
}

/// `PATH` with `~/.cargo/bin` in front (like `scripts/dev`), so a step finds cargo even when the parent shell was started without it.
fn path_with_cargo() -> Option<std::ffi::OsString> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let bin = PathBuf::from(home).join(".cargo").join("bin");
    let mut paths = vec![bin.clone()];
    if let Some(cur) = std::env::var_os("PATH") {
        if std::env::split_paths(&cur).any(|p| p == bin) {
            return None;
        }
        paths.extend(std::env::split_paths(&cur));
    }
    std::env::join_paths(paths).ok()
}

/// Runs a plan from `root`, writing `<log_dir>/affected-<n>-<step>.log` per step and calling `on_step` as each finishes. Stops at the first failure
/// unless `keep_going`. Returns every step that ran.
pub fn run(plan: &Plan, root: &Path, log_dir: &Path, keep_going: bool, on_step: &mut dyn FnMut(&StepResult)) -> Vec<StepResult> {
    let _ = std::fs::create_dir_all(log_dir);
    let mut results = Vec::new();
    for (n, step) in plan.steps.iter().enumerate() {
        let log = log_dir.join(format!("affected-{}-{}.log", n + 1, step.name));
        let t0 = Instant::now();
        let ok = match std::fs::File::create(&log).and_then(|f| f.try_clone().map(|g| (f, g))) {
            Err(_) => false,
            Ok((out, err)) => {
                let mut cmd = Command::new(program_for(&step.argv[0]));
                if let Some(path) = path_with_cargo() {
                    cmd.env("PATH", path);
                }
                cmd.args(&step.argv[1..])
                    .current_dir(root)
                    .env("CARGO_TERM_COLOR", "never")
                    .stdout(Stdio::from(out))
                    .stderr(Stdio::from(err))
                    .stdin(Stdio::null());
                for (k, v) in &step.env {
                    cmd.env(k, v);
                }
                match cmd.status() {
                    Ok(s) => s.success(),
                    Err(e) => {
                        let _ = std::fs::write(&log, format!("error: cannot run `{}`: {e}\n", step.argv[0]));
                        false
                    }
                }
            }
        };
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let r = StepResult {
            name: step.name.clone(),
            ok,
            secs: t0.elapsed().as_secs_f64(),
            tally: tally(&text),
            failures: if ok { Vec::new() } else { failure_lines(&text, 40) },
            log,
        };
        on_step(&r);
        let failed = !r.ok;
        results.push(r);
        if failed && !keep_going {
            break;
        }
    }
    results
}

/// A hash of what is being verified: the scope, the feature index and the *contents* of every changed file (a deleted file hashes as such). Equal
/// hashes mean equal inputs, so a previous green result still holds.
pub fn fingerprint(root: &Path, changed: &[String], scope: Scope) -> String {
    let mut h = crypto::Sha256::new();
    h.update(format!("affected-v1\n{}\n{}\n", scope.name(), env!("CARGO_PKG_VERSION")).as_bytes());
    h.update(features::INDEX.as_bytes());
    let mut files: Vec<&String> = changed.iter().collect();
    files.sort();
    for f in files {
        h.update(format!("\n{f}\n").as_bytes());
        match std::fs::read(root.join(f)) {
            Ok(bytes) => h.update(&crypto::sha256(&bytes)),
            Err(_) => h.update(b"<absent>"),
        }
    }
    crypto::hex(&h.finish())
}

/// Path of the green-stamp file under `root`.
pub fn stamp_path(root: &Path) -> PathBuf {
    root.join("out").join(".affected-green.json")
}

/// The scopes at least as strong as `scope` (a green full run also proves the smaller ones).
pub fn scopes_covering(scope: Scope) -> Vec<Scope> {
    [Scope::Quick, Scope::Closure, Scope::Full].into_iter().filter(|s| *s >= scope).collect()
}

/// Whether a stamp for `changed` at `scope` or stronger is already green.
pub fn already_green(root: &Path, changed: &[String], scope: Scope) -> Option<Scope> {
    let text = std::fs::read_to_string(stamp_path(root)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let greens: BTreeSet<&str> = v.get("green")?.as_array()?.iter().filter_map(|x| x.as_str()).collect();
    scopes_covering(scope).into_iter().find(|s| greens.contains(fingerprint(root, changed, *s).as_str()))
}

/// Records a green run (keeps the most recent 16).
pub fn record_green(root: &Path, changed: &[String], scope: Scope) {
    let path = stamp_path(root);
    let mut greens: Vec<String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("green").and_then(|g| g.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()))
        .unwrap_or_default();
    let key = fingerprint(root, changed, scope);
    greens.retain(|g| g != &key);
    greens.push(key);
    let keep = greens.len().saturating_sub(16);
    greens.drain(..keep);
    let _ = std::fs::create_dir_all(root.join("out"));
    let _ = std::fs::write(path, serde_json::json!({ "green": greens }).to_string());
}

/// The plan as text: what changed, the scope, each step with its reason, what was deferred.
pub fn render_plan(p: &Plan) -> String {
    let mut s = format!("{} changed file(s), scope {}", p.changed.len(), p.scope.name());
    if let Some(why) = &p.escalated {
        s.push_str(&format!(" (full CI: {why})"));
    }
    s.push('\n');
    for st in &p.steps {
        s.push_str(&format!("  {:<14} {}\n", st.name, st.command_line()));
    }
    for n in &p.notes {
        s.push_str(&format!("note: {n}\n"));
    }
    if !p.deferred.is_empty() {
        s.push_str(&format!("not run in this scope (dependents; run without --quick before integrating): {}\n", p.deferred.join(", ")));
    }
    if !p.suggest.is_empty() {
        s.push_str("worth a look (not run):\n");
        for c in &p.suggest {
            s.push_str(&format!("  {c}\n"));
        }
    }
    s
}

/// The plan as JSON.
pub fn plan_json(p: &Plan) -> serde_json::Value {
    serde_json::json!({
        "scope": p.scope.name(),
        "changed": p.changed,
        "escalated": p.escalated,
        "steps": p.steps.iter().map(|s| serde_json::json!({"name": s.name, "command": s.command_line(), "why": s.why})).collect::<Vec<_>>(),
        "deferred": p.deferred,
        "notes": p.notes,
        "suggest": p.suggest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(name: &str, files: &[&str], tests: &[&str], deps: &[&str]) -> Feature {
        Feature {
            name: name.into(),
            summary: format!("{name} summary"),
            files: files.iter().map(|s| s.to_string()).collect(),
            tests: tests.iter().map(|s| s.to_string()).collect(),
            commands: vec![format!("red_engine2 verify {name}")],
            docs: vec![],
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// A small world: many suites, so one feature is a minority and the escalation threshold is not hit by accident.
    fn world() -> Vec<Feature> {
        vec![
            feature("core", &["src/sim/clock.rs"], &["lib:sim::clock", "alloc_budget"], &[]),
            feature("net", &["src/net/**"], &["lib:net", "net_e2e"], &["core"]),
            feature("flow", &["src/sim/flow.rs"], &["lib:sim::flow", "net_flow"], &["net"]),
            feature("maps", &["src/tools/lint.rs"], &["lib:tools::lint", "maps_verify"], &[]),
            feature("docs", &["README.md"], &["repo_hygiene"], &[]),
            feature("a", &["src/a.rs"], &["s1", "s2"], &[]),
            feature("b", &["src/b.rs"], &["s3", "s4"], &[]),
            feature("c", &["src/c.rs"], &["s5", "s6"], &[]),
        ]
    }

    fn serial() -> Vec<String> {
        vec!["net_e2e".into(), "net_flow".into()]
    }

    fn names(p: &Plan) -> Vec<&str> {
        p.steps.iter().map(|s| s.name.as_str()).collect()
    }

    fn step<'a>(p: &'a Plan, name: &str) -> &'a Step {
        p.steps.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no step {name} in {:?}", names(p)))
    }

    #[test]
    fn a_map_tool_change_verifies_only_map_tooling() {
        let p = plan(&world(), &serial(), &["src/tools/lint.rs".to_string()], &Options::default());
        assert_eq!(p.scope, Scope::Closure);
        assert_eq!(names(&p), ["fmt", "clippy", "unit", "doc", "suites"]);
        assert_eq!(step(&p, "unit").argv.join(" "), "cargo test --locked --lib -- tools::lint");
        assert_eq!(step(&p, "suites").argv.join(" "), "cargo test --locked --test maps_verify");
        assert!(step(&p, "clippy").argv.join(" ").starts_with("cargo clippy --locked --lib --bins -- -D warnings"));
    }

    #[test]
    fn dependents_run_by_default_but_quick_defers_them() {
        let changed = ["src/sim/clock.rs".to_string()];
        let full = plan(&world(), &serial(), &changed, &Options::default());
        let quick = plan(&world(), &serial(), &changed, &Options { quick: true, ..Options::default() });
        assert!(step(&full, "suites-serial").argv.join(" ").contains("--test net_e2e --test net_flow"), "{:?}", full.steps);
        assert_eq!(step(&full, "suites-serial").env, vec![("RUST_TEST_THREADS".to_string(), "1".to_string())]);
        assert_eq!(step(&full, "suites").argv.join(" "), "cargo test --locked --test alloc_budget");
        assert_eq!(quick.scope, Scope::Quick);
        assert!(quick.steps.iter().all(|s| s.name != "suites-serial"), "the real-time suites belong to dependents: {:?}", names(&quick));
        assert_eq!(quick.deferred, ["net_e2e", "net_flow"], "what quick left out is named, never silent");
        assert!(render_plan(&quick).contains("not run in this scope"));
    }

    #[test]
    fn the_changed_path_itself_names_its_unit_tests_even_when_the_index_is_coarse() {
        // `src/b.rs` is owned by `b`, whose entries list no `lib:` filter: the path still yields `b`.
        let p = plan(&world(), &serial(), &["src/b.rs".to_string()], &Options::default());
        assert_eq!(step(&p, "unit").argv.last().map(String::as_str), Some("b"));
        assert_eq!(lib_filter("src/net/mod.rs").as_deref(), Some("net"));
        assert_eq!(lib_filter("src/tools/lint.rs").as_deref(), Some("tools::lint"));
        assert_eq!(lib_filter("src/cli/args.rs"), None, "cli lives in the red_engine2 binary");
    }

    #[test]
    fn a_binary_only_change_lints_and_tests_that_binary_alone() {
        let mut w = world();
        w.push(feature("cli", &["src/cli/**", "src/main.rs"], &["cli_envelope"], &[]));
        let p = plan(&w, &serial(), &["src/cli/args.rs".to_string()], &Options::default());
        assert_eq!(names(&p), ["fmt", "clippy", "bin", "suites"]);
        assert_eq!(step(&p, "clippy").argv.join(" "), "cargo clippy --locked --bin red_engine2 -- -D warnings");
        assert_eq!(step(&p, "bin").argv.join(" "), "cargo test --locked --bin red_engine2");
    }

    #[test]
    fn a_changed_test_file_always_runs_itself() {
        let p = plan(&world(), &serial(), &["tests/maps_verify.rs".to_string()], &Options::default());
        assert!(step(&p, "suites").argv.join(" ").contains("--test maps_verify"));
        assert_eq!(step(&p, "clippy").argv.join(" "), "cargo clippy --locked --test maps_verify -- -D warnings");
        assert!(p.steps.iter().all(|s| s.name != "unit"), "no lib change, no lib tests: {:?}", names(&p));
    }

    #[test]
    fn boundary_files_escalate_to_the_full_ci_run() {
        for f in ["Cargo.toml", "Cargo.lock", "src/lib.rs", "rustfmt.toml", "scripts/ci.sh", ".cargo/config.toml", ".github/workflows/ci.yml"] {
            let p = plan(&world(), &serial(), &[f.to_string(), "src/b.rs".to_string()], &Options::default());
            assert_eq!(p.scope, Scope::Full, "{f}");
            assert_eq!(p.steps.len(), 1);
            assert_eq!(p.steps[0].argv, ["bash", "scripts/ci.sh"]);
            assert!(p.escalated.as_deref().unwrap_or("").contains(f), "{f}: {:?}", p.escalated);
        }
    }

    #[test]
    fn a_huge_diff_or_most_of_the_suite_escalates() {
        let many: Vec<String> = (0..70).map(|i| format!("src/x{i}.rs")).collect();
        assert_eq!(plan(&world(), &serial(), &many, &Options::default()).scope, Scope::Full);
        // core -> net -> flow drags in nearly every suite of a tiny world.
        let mut w = world();
        w.retain(|f| ["core", "net", "flow"].contains(&f.name.as_str()));
        assert_eq!(plan(&w, &serial(), &["src/sim/clock.rs".to_string()], &Options::default()).scope, Scope::Full);
        assert_eq!(
            plan(&w, &serial(), &["src/sim/clock.rs".to_string()], &Options { quick: true, ..Options::default() }).scope,
            Scope::Quick,
            "quick never escalates on size"
        );
        assert_eq!(plan(&world(), &serial(), &["src/b.rs".to_string()], &Options { full: true, ..Options::default() }).scope, Scope::Full);
    }

    #[test]
    fn generated_output_is_ignored_and_an_unlisted_source_file_is_flagged() {
        let none = plan(&world(), &serial(), &["out/logs/x.log".to_string(), "target/debug/x".to_string(), "STATUS.md".to_string()], &Options::default());
        assert!(none.steps.is_empty() && none.notes.iter().any(|n| n.contains("nothing to verify")));
        let p = plan(&world(), &serial(), &["src/brand_new.rs".to_string()], &Options::default());
        assert!(step(&p, "suites").argv.join(" ").contains("features_index"), "an unlisted source file makes the index check run");
        assert!(p.notes.iter().any(|n| n.contains("docs/features.json")));
        let docs = plan(&world(), &serial(), &["mcp_server.py".to_string()], &Options::default());
        assert!(docs.steps.is_empty() && docs.notes.iter().any(|n| n.contains("no automated check covers")), "{:?}", docs.notes);
    }

    #[test]
    fn the_doc_step_is_kept_only_when_a_changed_file_has_a_doc_example() {
        let dir = std::env::temp_dir().join(format!("re2_affected_doc_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/plain.rs"), ["//! Nothing to run here.", "pub fn f() {}", ""].join("\n")).unwrap();
        std::fs::write(dir.join("src/example.rs"), ["/// Adds.", "///", "/// ```", "/// assert_eq!(1 + 1, 2);", "/// ```", "pub fn f() {}", ""].join("\n"))
            .unwrap();
        let docs = |file: &str| {
            let mut p = plan(&world(), &serial(), &[file.to_string()], &Options::default());
            assert!(p.steps.iter().any(|s| s.name == "doc"), "the planner asks for it: {:?}", names(&p));
            prune_doc_step(&mut p, &dir);
            p.steps.iter().any(|s| s.name == "doc")
        };
        assert!(!docs("src/plain.rs"), "no example, no rustdoc run");
        assert!(docs("src/example.rs"), "an example keeps the step");
        assert!(docs("src/missing.rs"), "an unreadable file keeps it (safe side)");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn logs_are_summarised_not_dumped() {
        let log = "running 2 tests\ntest a ... ok\ntest b ... FAILED\n\nfailures:\n\nthread 'b' panicked at tests/x.rs:9:5:\nassertion failed: 1 == 2\n\ntest result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n\ntest result: ok. 4 passed; 0 failed; 2 ignored; 0 measured\n";
        assert_eq!(tally(log).as_deref(), Some("5 passed, 1 failed, 2 ignored"));
        let f = failure_lines(log, 40);
        assert!(f.iter().any(|l| l.contains("test b ... FAILED")) && f.iter().any(|l| l.contains("assertion failed: 1 == 2")), "{f:?}");
        assert!(!f.iter().any(|l| l.contains("test a ... ok")));
        assert!(failure_lines("error[E0432]: unresolved import\n  --> src/x.rs:1:5\n", 40).len() == 2);
        assert_eq!(tally("nothing here"), None);
    }

    #[test]
    fn the_fingerprint_tracks_content_scope_and_deletion() {
        let dir = std::env::temp_dir().join(format!("re2_affected_fp_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one").unwrap();
        let files = vec!["a.txt".to_string()];
        let k1 = fingerprint(&dir, &files, Scope::Closure);
        assert_eq!(k1, fingerprint(&dir, &files, Scope::Closure), "deterministic");
        assert_ne!(k1, fingerprint(&dir, &files, Scope::Quick), "scope is part of the key");
        std::fs::write(dir.join("a.txt"), "two").unwrap();
        assert_ne!(k1, fingerprint(&dir, &files, Scope::Closure), "content is part of the key");
        std::fs::remove_file(dir.join("a.txt")).unwrap();
        assert_ne!(k1, fingerprint(&dir, &files, Scope::Closure), "a deleted file is a different input");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_green_stamp_covers_the_same_inputs_and_weaker_scopes_only() {
        let dir = std::env::temp_dir().join(format!("re2_affected_stamp_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one").unwrap();
        let files = vec!["a.txt".to_string()];
        assert_eq!(already_green(&dir, &files, Scope::Quick), None);
        record_green(&dir, &files, Scope::Closure);
        assert_eq!(already_green(&dir, &files, Scope::Quick), Some(Scope::Closure), "a closure run proves quick");
        assert_eq!(already_green(&dir, &files, Scope::Closure), Some(Scope::Closure));
        assert_eq!(already_green(&dir, &files, Scope::Full), None, "but not the full run");
        std::fs::write(dir.join("a.txt"), "edited").unwrap();
        assert_eq!(already_green(&dir, &files, Scope::Quick), None, "any edit invalidates it");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn running_a_plan_logs_each_step_and_stops_at_the_first_failure() {
        let dir = std::env::temp_dir().join(format!("re2_affected_run_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = std::env::current_exe().unwrap().to_string_lossy().to_string();
        // The test binary itself is a portable command: `--help` exits 0, an unknown flag exits non-zero.
        let mk = |name: &str, arg: &str| Step { name: name.into(), argv: vec![exe.clone(), arg.into()], env: vec![], why: String::new() };
        let p = Plan {
            scope: Scope::Closure,
            changed: vec![],
            steps: vec![mk("good", "--help"), mk("bad", "--definitely-not-a-flag"), mk("never", "--help")],
            escalated: None,
            deferred: vec![],
            notes: vec![],
            suggest: vec![],
        };
        let mut seen = Vec::new();
        let r = run(&p, &dir, &dir.join("logs"), false, &mut |s| seen.push((s.name.clone(), s.ok)));
        assert_eq!(seen, [("good".to_string(), true), ("bad".to_string(), false)]);
        assert_eq!(r.len(), 2);
        assert!(r[1].log.exists() && !r[1].failures.is_empty());
        let all = run(&p, &dir, &dir.join("logs"), true, &mut |_| {});
        assert_eq!(all.len(), 3, "--keep-going runs every step");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
