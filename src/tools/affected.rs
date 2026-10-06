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
//! **Escalation.** Some changes cannot be verified by a subset: `Cargo.toml`/`Cargo.lock`, `src/lib.rs` (unless it only gained module declarations, see [`crate_root_only_adds_modules`]), `rustfmt.toml`, `.cargo/`, `scripts/ci.sh`, a very
//! large diff, or an affected set that is most of the suite. Those (and `--full`) plan exactly one step: `bash scripts/ci.sh`, the same as CI.
//!
//! **Tiers.** `--quick` verifies the features that *own* the changed files (the inner edit loop, seconds); the default also verifies every feature built on
//! them (before you say "done"); `--full` is CI (before you push, or at any integration boundary). What a tier did not run is listed, never silent.
//!
//! **Green stamps.** A passing run is recorded in `out/.affected-green.json` under a hash of the *contents* of every changed file, the scope and the feature
//! index. Asking again with identical inputs costs nothing ("already verified"); any edit changes the hash. Failures are never cached.
//!
//! **Partial iteration.** `--partial` (`scripts/dev iterate`) is the bounded edit-loop path: it looks only at what changed since `HEAD`, never escalates, and runs
//! `cargo fmt --check`, a type-check of the touched targets and the focused unit tests. It always says that full verification is still required, is stamped under
//! its own scope (`partial`) that no other scope ever accepts, and never prints "verified". Its cargo steps state their feature set (`--headless` for files that are
//! provably not graphics code) and the stamp key includes that configuration, the toolchain and the base commit, so a partial, headless or other-toolchain pass cannot
//! stand in for anything else.
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
    /// The bounded edit-loop path: changes since `HEAD`, type-check plus focused unit tests, never escalates and never counts as verification.
    Partial,
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
            Scope::Partial => "partial",
            Scope::Quick => "quick",
            Scope::Closure => "closure",
            Scope::Full => "full",
        }
    }
}

/// Which cargo feature set a plan's steps use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Features {
    /// Everything, graphics included (`cargo` defaults).
    Default,
    /// `--no-default-features`: the graphics-free server/analysis build.
    Headless,
}

impl Features {
    /// Stable name (`default`, `headless`), shown in plans and part of the stamp key.
    pub fn name(self) -> &'static str {
        match self {
            Features::Default => "default",
            Features::Headless => "headless",
        }
    }
    /// The cargo flags that select this feature set.
    pub fn flags(self) -> &'static [&'static str] {
        match self {
            Features::Default => &[],
            Features::Headless => &["--no-default-features"],
        }
    }
}

/// The effective configuration a plan's steps run under, shown in the plan and (except the two build-speed fields) part of the stamp key: a green result is
/// only reused under the same feature set, toolchain and result-affecting environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Feature set of the cargo steps.
    pub features: Features,
    /// `rustc -vV` release and commit hash (`unknown` when rustc cannot be run).
    pub toolchain: String,
    /// Environment variables that change what a build or test does (`RUSTFLAGS`, `RUST_TEST_THREADS`, `CARGO_PROFILE_*`, `CARGO_BUILD_TARGET`), sorted.
    pub env: Vec<(String, String)>,
    /// `CARGO_BUILD_JOBS` if set: how fast, not what, so it is reported but not part of [`id`](Self::id).
    pub jobs: Option<String>,
    /// The cargo target directory: where artifacts live, not what they prove, so reported but not part of [`id`](Self::id).
    pub target_dir: String,
}

/// Environment variables that change the result of a build or a test run.
fn result_env_names(name: &str) -> bool {
    matches!(name, "RUSTFLAGS" | "CARGO_ENCODED_RUSTFLAGS" | "RUSTDOCFLAGS" | "CARGO_BUILD_TARGET" | "RUST_TEST_THREADS" | "RUST_TEST_NOCAPTURE")
        || name.starts_with("CARGO_PROFILE_")
}

impl Config {
    /// The configuration of this process's environment for `features`.
    pub fn detect(features: Features) -> Config {
        let toolchain = Command::new("rustc")
            .arg("-vV")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                let text = String::from_utf8_lossy(&o.stdout).to_string();
                let get = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).unwrap_or("?").trim().to_string();
                format!("{} ({})", get("release:"), get("commit-hash:").chars().take(9).collect::<String>())
            })
            .unwrap_or_else(|| "unknown".into());
        let mut env: Vec<(String, String)> = std::env::vars().filter(|(k, _)| result_env_names(k)).collect();
        env.sort();
        Config {
            features,
            toolchain,
            env,
            jobs: std::env::var("CARGO_BUILD_JOBS").ok(),
            target_dir: std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| "target".into()),
        }
    }

    /// What makes two results comparable: feature set, toolchain and result-affecting environment (not jobs or target directory).
    pub fn id(&self) -> String {
        let env: Vec<String> = self.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("features={};profile=dev;toolchain={};env=[{}]", self.features.name(), self.toolchain, env.join(","))
    }

    /// One line for humans: everything, including the two fields that are not part of the identity.
    pub fn render(&self) -> String {
        let env: Vec<String> = self.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!(
            "features={} profile=dev/test toolchain={} jobs={} target-dir={}{}",
            self.features.name(),
            self.toolchain,
            self.jobs.as_deref().unwrap_or("auto"),
            self.target_dir,
            if env.is_empty() { String::new() } else { format!(" env: {}", env.join(" ")) }
        )
    }
}

/// Everything besides the changed files' contents that a stamp must agree on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StampKey {
    /// The resolved commit the change set is relative to: together with the changed files' contents it fixes the whole tree.
    pub base: String,
    /// [`Config::id`]: feature set, toolchain, result-affecting environment.
    pub config: String,
    /// The plan variant (`--check-only` runs fewer steps than a plain partial run).
    pub variant: String,
}

/// Bump when the planner's steps change in a way that an old green result should not vouch for.
const PLANNER_REV: &str = "4";

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
    /// The cargo feature set the steps use.
    pub features: Features,
    /// Why full verification is still required after this plan (a partial plan only; empty for every other scope): never empty for `Scope::Partial`.
    pub full_required: Vec<String>,
}

/// Engine files the 2D crate (`crates/red2d`) includes by path: changing one changes that crate too, so its tests and lints run.
const RED2D_SHARED: &[&str] = &[
    "src/fields.rs",
    "src/suggest.rs",
    "src/sim/rules_expr.rs",
    "src/synth.rs",
    "src/dsp.rs",
    "src/audio_analysis.rs",
    "src/voice_spec.rs",
    "src/audio_fx.rs",
    "src/score.rs",
];

/// Whether a change reaches the 2D crate: a file in it, or an engine file it includes.
pub fn touches_red2d(changed: &[String]) -> bool {
    changed.iter().any(|c| c.starts_with("crates/red2d/") || RED2D_SHARED.contains(&c.as_str()))
}

/// The steps the 2D crate adds. Cargo's package selection is by `-p`, so none of the engine-crate steps above covers it. `browser` (the full tier only) adds the
/// WebAssembly lint and the real-browser run, which fails rather than skips when no browser is set up: a green `affected` must not mean "browser skipped".
fn red2d_steps(changed: &[String], tests: bool, browser: bool) -> Vec<Step> {
    if !touches_red2d(changed) {
        return Vec::new();
    }
    let mut v = vec![Step::new(
        "red2d-clippy",
        &["cargo", "clippy", "--locked", "-p", "red2d", "--all-targets", "--", "-D", "warnings"],
        "the 2D crate (crates/red2d) or a file it includes changed",
    )];
    if tests {
        v.push(Step::new("red2d", &["cargo", "test", "--locked", "-p", "red2d"], "unit tests of the 2D crate: parser, simulation, renderer, sound, host"));
    }
    if browser {
        v.push(Step::new(
            "red2d-wasm",
            &["cargo", "clippy", "--locked", "-p", "red2d", "--target", "wasm32-unknown-unknown", "--", "-D", "warnings"],
            "the WebAssembly surface (web.rs) only compiles for wasm32",
        ));
        v.push(Step {
            name: "web".into(),
            argv: vec!["bash".into(), "scripts/web_check.sh".into()],
            env: vec![("RED_CI_REQUIRE_BROWSER".into(), "1".into())],
            why: "the 2D games run in a real headless browser (needs `red_engine2 web setup-browser` once); a missing browser fails this step, never skips it"
                .into(),
        });
    }
    v
}

/// Whether a change reaches the browser build of the 3D player: anything in the engine crate (it compiles the same sources for wasm32) or the wrapper crate.
pub fn touches_web3d(changed: &[String]) -> bool {
    changed.iter().any(|c| c.starts_with("src/") || c.starts_with("crates/web3d/") || c == "Cargo.toml" || c == "Cargo.lock" || c.starts_with(".cargo/"))
}

/// The step the 3D browser build adds (full tier): a wasm32 lint of the engine crate under `--features web`, because nothing else compiles those cfgs.
fn web3d_steps(changed: &[String], browser: bool) -> Vec<Step> {
    if !browser || !touches_web3d(changed) {
        return Vec::new();
    }
    vec![Step::new(
        "web3d-wasm",
        &["bash", "scripts/ci.sh", "web3d"],
        "the browser build of the 3D player: the engine's renderer and simulation must still compile (and lint clean) for wasm32 with the `web` feature",
    )]
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

/// The module declarations of a crate root and everything else in it: `(declarations, rest)`. A declaration is `(its line, the attributes above it)`; the rest is every
/// other line, trimmed, with blank lines and plain `//`/`///` comments dropped (they change no behaviour).
fn crate_root_parts(text: &str) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    let (mut decls, mut rest, mut attrs) = (Vec::new(), Vec::new(), Vec::new());
    for line in text.lines().map(str::trim) {
        if line.is_empty() || (line.starts_with("//") && !line.starts_with("//!")) {
            continue;
        }
        if line.starts_with("#[") && line.ends_with(']') {
            attrs.push(line.to_string());
        } else if (line.starts_with("pub mod ") || line.starts_with("mod ")) && line.ends_with(';') {
            decls.push((line.to_string(), std::mem::take(&mut attrs)));
        } else {
            rest.append(&mut attrs);
            rest.push(line.to_string());
        }
    }
    rest.append(&mut attrs);
    (decls, rest)
}

/// Whether `new` differs from `old` (two versions of `src/lib.rs`) only by added module declarations (`pub mod x;`, with its own `#[cfg(..)]` gate). Such a change cannot move
/// any existing code or feature gate, and the new file owns its own tests, so it needs no more than the file itself does. Anything else (a removed or re-gated module, a `use`,
/// a function, the crate docs) is a crate-root change that only the whole suite can vouch for.
pub fn crate_root_only_adds_modules(old: &str, new: &str) -> bool {
    let ((old_decls, old_rest), (new_decls, new_rest)) = (crate_root_parts(old), crate_root_parts(new));
    old_rest == new_rest && new_decls.len() > old_decls.len() && old_decls.iter().all(|d| new_decls.contains(d))
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
    /// Plan the bounded partial iteration path instead (see [`plan_partial`]).
    pub partial: bool,
    /// Partial only: type-check and format only, no tests.
    pub check_only: bool,
    /// Partial only: ask for the graphics-free feature set (honoured only when every changed file is provably headless-safe).
    pub headless: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { quick: false, full: false, max_files: 60, escalate_percent: 75, partial: false, check_only: false, headless: false }
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
        features: Features::Default,
        full_required: Vec::new(),
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
        features: Features::Default,
        full_required: Vec::new(),
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
    plan.steps.extend(red2d_steps(&changed, true, !opts.quick));
    plan.steps.extend(web3d_steps(&changed, !opts.quick));
    plan.deferred = deferred.into_iter().collect();
    plan.suggest.sort();
    plan.suggest.dedup();
    plan
}

/// Top-level modules of `src/lib.rs` that exist only with the `gfx` feature (`#[cfg(feature = "gfx")]` directly above `mod name;`).
fn gfx_modules(lib_rs: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut gated = false;
    for line in lib_rs.lines() {
        let t = line.trim();
        if t.starts_with("#[cfg(feature = \"gfx\")]") {
            gated = true;
        } else if t.starts_with("#[") {
            // another attribute between the cfg and the module keeps the gate
        } else {
            if gated {
                if let Some(name) = t.strip_prefix("pub mod ").or_else(|| t.strip_prefix("mod ")).map(|r| r.trim_end_matches(';').trim()) {
                    out.insert(name.to_string());
                }
            }
            gated = false;
        }
    }
    out
}

/// Whether every changed file can be checked without graphics: `Ok(())`, or `Err(why)` naming the first file that needs the default feature set.
/// Reviewed rule, deliberately conservative: a Rust file under `src/` whose top-level module is not `gfx`-gated in `src/lib.rs`, that is not the live
/// client (`src/bin/re2/`), and whose text never mentions `feature = "gfx"`. Tests, benches and examples run under the default features; a change to the
/// build settings does too.
pub fn headless_safe(root: &Path, changed: &[String]) -> Result<(), String> {
    let gated = std::fs::read_to_string(root.join("src/lib.rs")).map(|t| gfx_modules(&t)).unwrap_or_default();
    for c in changed {
        if boundary_reason(c).is_some() {
            return Err(format!("{c}: build settings change for every feature set"));
        }
        if !c.ends_with(".rs") {
            continue;
        }
        let Some(rel) = c.strip_prefix("src/") else {
            return Err(format!("{c}: tests, benches and examples run under the default features"));
        };
        if rel.starts_with("bin/re2/") {
            return Err(format!("{c}: the live client needs graphics"));
        }
        let top = rel.split('/').next().unwrap_or("").trim_end_matches(".rs");
        if gated.contains(top) {
            return Err(format!("{c}: module `{top}` exists only with the gfx feature"));
        }
        match std::fs::read_to_string(root.join(c)) {
            Ok(text) if text.contains("feature = \"gfx\"") => return Err(format!("{c}: has gfx-only code inside")),
            Ok(_) => {}
            Err(_) => return Err(format!("{c}: cannot be read to check for gfx-only code")),
        }
    }
    Ok(())
}

/// The bounded partial plan (`--partial`, `scripts/dev iterate`): for the files changed since the base (the caller passes `HEAD`'s diff), format check, a type-check
/// of the touched cargo targets, the unit tests of the touched library modules (unless `check_only`) and any changed test file running itself.
/// It **never escalates**: a boundary file does not turn it into a full CI run, it is reported in [`Plan::full_required`] instead. The result is never "verified":
/// `full_required` always says what the plan did not cover, and `deferred` lists the integration suites a quick run would have run.
pub fn plan_partial(all: &[Feature], serial: &[String], changed: &[String], opts: &Options, features: Features) -> Plan {
    let changed: Vec<String> = changed.iter().map(|c| c.replace('\\', "/")).filter(|c| !ignorable(c)).collect();
    let mut plan = Plan {
        scope: Scope::Partial,
        changed: changed.clone(),
        steps: Vec::new(),
        escalated: None,
        deferred: Vec::new(),
        notes: Vec::new(),
        suggest: Vec::new(),
        features,
        full_required: Vec::new(),
    };
    if changed.is_empty() {
        plan.notes.push("nothing changed since the base: nothing to check".into());
        return plan;
    }
    for c in &changed {
        if let Some(why) = boundary_reason(c) {
            plan.full_required.push(format!("{c}: {why}; no subset of tests can vouch for it"));
        }
    }
    if changed.len() > opts.max_files {
        plan.full_required.push(format!("{} changed files: a partial check says little about a change this large", changed.len()));
    }
    let is_test_file = |c: &str| matches!(target_of(c), Some(Target::Test(_)));
    let (_, others): (Vec<String>, Vec<String>) = changed.iter().cloned().partition(|c| is_test_file(c));
    let imp = features::impact(all, &others);
    let mut would: BTreeSet<String> = BTreeSet::new();
    for f in all.iter().filter(|f| imp.direct.contains_key(&f.name) || imp.downstream.contains(&f.name)) {
        would.extend(f.tests.iter().filter(|t| !t.starts_with("lib:")).cloned());
    }
    let mut targets: BTreeSet<Target> = BTreeSet::new();
    let mut lib_filters: BTreeSet<String> = BTreeSet::new();
    for c in &changed {
        if let Some(t) = target_of(c) {
            targets.insert(t);
        }
        if let Some(m) = lib_filter(c) {
            lib_filters.insert(m);
        }
    }
    let own_tests: BTreeSet<String> = targets.iter().filter_map(|t| if let Target::Test(n) = t { Some(n.clone()) } else { None }).collect();
    plan.deferred = would.difference(&own_tests).cloned().collect();
    let flags = features.flags();
    let has_rs = changed.iter().any(|c| c.ends_with(".rs"));
    let build_settings = changed.iter().any(|c| boundary_reason(c).is_some() && matches!(c.as_str(), "Cargo.toml" | "Cargo.lock" | "build.rs"));
    if has_rs {
        plan.steps.push(Step::new("fmt", &["cargo", "fmt", "--check"], "a Rust file changed"));
    }
    if has_rs || build_settings {
        // clippy, not check: it type-checks the same targets and also fails on what CI's clippy stage would (warnings are errors), so a lint
        // problem shows up in seconds here and not at the end of a 7-minute full run.
        let mut argv: Vec<String> = ["cargo", "clippy", "--locked"].iter().map(|s| s.to_string()).collect();
        argv.extend(flags.iter().map(|f| f.to_string()));
        let lib_changed = targets.contains(&Target::Lib) || build_settings || targets.is_empty();
        if lib_changed {
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
            name: "check".into(),
            argv,
            env: Vec::new(),
            why: format!("type-check and lint the touched targets ({} features)", features.name()),
        });
    }
    if !opts.check_only {
        let lib_filters = minimal_filters(&lib_filters);
        if !lib_filters.is_empty() {
            let mut argv: Vec<String> = ["cargo", "test", "--locked"].iter().map(|s| s.to_string()).collect();
            argv.extend(flags.iter().map(|f| f.to_string()));
            argv.extend(["--lib".into(), "--".into()]);
            argv.extend(lib_filters.iter().cloned());
            plan.steps.push(Step { name: "unit".into(), argv, env: Vec::new(), why: format!("unit tests of the touched modules: {}", lib_filters.join(", ")) });
        }
        let (serial_own, parallel_own): (Vec<&String>, Vec<&String>) = own_tests.iter().partition(|s| serial.contains(s));
        for (name, list, env) in [("suites", parallel_own, Vec::new()), ("suites-serial", serial_own, vec![("RUST_TEST_THREADS".to_string(), "1".to_string())])]
        {
            if list.is_empty() {
                continue;
            }
            let mut argv: Vec<String> = ["cargo", "test", "--locked"].iter().map(|s| s.to_string()).collect();
            argv.extend(flags.iter().map(|f| f.to_string()));
            for t in &list {
                argv.extend(["--test".into(), (*t).clone()]);
            }
            plan.steps.push(Step { name: name.into(), argv, env, why: "a changed test file runs itself".into() });
        }
    }
    plan.steps.extend(red2d_steps(&changed, !opts.check_only, false));
    if opts.check_only {
        plan.notes.push("--check-only: formatting and type-check only, no tests".into());
    }
    if !plan.deferred.is_empty() {
        plan.full_required.push(format!("{} integration suite(s) of the affected features did not run (see `not run`)", plan.deferred.len()));
    }
    plan.full_required.push("a partial pass is not verification: run `scripts/dev affected` before you say done, `--full` before pushing".into());
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
            Ok(text) => has_runnable_doc_example(&text),
            Err(_) => true,
        }
    };
    if plan.steps.iter().any(|s| s.name == "doc") && !plan.changed.iter().any(has_example) {
        plan.steps.retain(|s| s.name != "doc");
    }
}

/// Whether a source text has a doctest rustdoc would actually run: a fenced block in a `///` or `//!` comment whose info string is empty or rust-ish. A
/// ```` ```json ````, ```` ```text ```` or ```` ```sh ```` block is documentation, and `ignore`d blocks are not run, so they must not keep the (seconds-long) `doc` step.
pub fn has_runnable_doc_example(text: &str) -> bool {
    let mut open = false;
    for l in text.lines() {
        let t = l.trim_start();
        let Some(body) = t.strip_prefix("///").or_else(|| t.strip_prefix("//!")) else { continue };
        let Some(info) = body.trim_start().strip_prefix("```") else { continue };
        if open {
            open = false; // the closing fence
            continue;
        }
        open = true;
        let info = info.trim();
        let runnable = info.is_empty()
            || info
                .split([',', ' '])
                .filter(|w| !w.is_empty())
                .all(|w| matches!(w, "rust" | "no_run" | "should_panic" | "compile_fail" | "edition2015" | "edition2018" | "edition2021" | "edition2024"));
        if runnable {
            return true;
        }
    }
    false
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

/// A hash of what is being verified: the scope, the planner revision, the feature index **as it is on disk**, everything in the [`StampKey`] (base commit,
/// configuration, plan variant) and the *contents* of every changed file (a deleted file hashes as such). Equal hashes mean equal inputs, so a previous green
/// result still holds; anything that could change what the result proves changes the hash.
pub fn fingerprint_in(root: &Path, changed: &[String], scope: Scope, key: &StampKey) -> String {
    let mut h = crypto::Sha256::new();
    h.update(
        format!(
            "affected-v2\n{}\n{}\n{PLANNER_REV}\nbase={}\nconfig={}\nvariant={}\n",
            scope.name(),
            env!("CARGO_PKG_VERSION"),
            key.base,
            key.config,
            key.variant
        )
        .as_bytes(),
    );
    h.update(features::index_text_at(root).as_bytes());
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

/// [`fingerprint_in`] with an empty key (no base, configuration or variant): kept for callers that only know files and scope.
pub fn fingerprint(root: &Path, changed: &[String], scope: Scope) -> String {
    fingerprint_in(root, changed, scope, &StampKey::default())
}

/// Path of the green-stamp file under `root`.
pub fn stamp_path(root: &Path) -> PathBuf {
    root.join("out").join(".affected-green.json")
}

/// The scopes whose green result also proves `scope`. A quick run is proven by a green closure or full run, and so on up. **A partial pass proves only a
/// partial pass, and is proven by nothing stronger being assumed**: it type-checks and runs a few unit tests, so it must never stand in for a real
/// verification, and the lookup for any other scope never reads a partial stamp.
pub fn scopes_covering(scope: Scope) -> Vec<Scope> {
    if scope == Scope::Partial {
        return vec![Scope::Partial];
    }
    [Scope::Quick, Scope::Closure, Scope::Full].into_iter().filter(|s| *s >= scope).collect()
}

/// Whether a stamp for `changed` at `scope` or stronger is already green under `key`.
pub fn already_green_in(root: &Path, changed: &[String], scope: Scope, key: &StampKey) -> Option<Scope> {
    let text = std::fs::read_to_string(stamp_path(root)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let greens: BTreeSet<&str> = v.get("green")?.as_array()?.iter().filter_map(|x| x.as_str()).collect();
    scopes_covering(scope).into_iter().find(|s| greens.contains(fingerprint_in(root, changed, *s, key).as_str()))
}

/// [`already_green_in`] with an empty key.
pub fn already_green(root: &Path, changed: &[String], scope: Scope) -> Option<Scope> {
    already_green_in(root, changed, scope, &StampKey::default())
}

/// Records a green run under `key` (keeps the most recent 16).
pub fn record_green_in(root: &Path, changed: &[String], scope: Scope, key: &StampKey) {
    let path = stamp_path(root);
    let mut greens: Vec<String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("green").and_then(|g| g.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()))
        .unwrap_or_default();
    let k = fingerprint_in(root, changed, scope, key);
    greens.retain(|g| g != &k);
    greens.push(k);
    let keep = greens.len().saturating_sub(16);
    greens.drain(..keep);
    let _ = std::fs::create_dir_all(root.join("out"));
    let _ = std::fs::write(path, serde_json::json!({ "green": greens }).to_string());
}

/// [`record_green_in`] with an empty key.
pub fn record_green(root: &Path, changed: &[String], scope: Scope) {
    record_green_in(root, changed, scope, &StampKey::default());
}

/// The plan as text: what changed, the scope, each step with its reason, what was deferred.
pub fn render_plan(p: &Plan) -> String {
    let mut s = format!("{} changed file(s), scope {}", p.changed.len(), p.scope.name());
    if let Some(why) = &p.escalated {
        s.push_str(&format!(" (full CI: {why})"));
    }
    if p.scope == Scope::Partial {
        s.push_str(" (PARTIAL: does not count as verification)");
    }
    s.push('\n');
    for st in &p.steps {
        s.push_str(&format!("  {:<14} {}\n", st.name, st.command_line()));
    }
    for n in &p.notes {
        s.push_str(&format!("note: {n}\n"));
    }
    if !p.deferred.is_empty() {
        if p.scope == Scope::Partial {
            s.push_str(&format!("not run (integration suites of the affected features): {}\n", p.deferred.join(", ")));
        } else {
            s.push_str(&format!("not run in this scope (dependents; run without --quick before integrating): {}\n", p.deferred.join(", ")));
        }
    }
    for r in &p.full_required {
        s.push_str(&format!("full verification still required: {r}\n"));
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
        "partial": p.scope == Scope::Partial,
        "features": p.features.name(),
        "full_verification_required": p.full_required,
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
    fn a_change_to_the_2d_crate_or_a_file_it_includes_plans_the_crates_own_checks() {
        for f in ["crates/red2d/src/sim.rs", "crates/red2d/web/runtime.js", "src/sim/rules_expr.rs", "src/synth.rs"] {
            assert!(touches_red2d(&[f.to_string()]), "{f}");
        }
        assert!(!touches_red2d(&["src/tools/lint.rs".to_string(), "src/net/mod.rs".to_string()]));
        let p = plan(&world(), &serial(), &["crates/red2d/src/sim.rs".to_string()], &Options::default());
        assert_eq!(step(&p, "red2d").argv.join(" "), "cargo test --locked -p red2d");
        assert!(step(&p, "red2d-clippy").argv.join(" ").contains("-p red2d --all-targets"));
        let web = step(&p, "web");
        assert_eq!(web.env, vec![("RED_CI_REQUIRE_BROWSER".to_string(), "1".to_string())], "a missing browser must fail the full tier, not skip it");
        assert!(step(&p, "red2d-wasm").argv.join(" ").contains("--target wasm32-unknown-unknown"));
        let quick = plan(&world(), &serial(), &["crates/red2d/src/sim.rs".to_string()], &Options { quick: true, ..Options::default() });
        assert!(quick.steps.iter().any(|s| s.name == "red2d") && quick.steps.iter().all(|s| s.name != "web"), "{:?}", names(&quick));
        let none = plan(&world(), &serial(), &["src/b.rs".to_string()], &Options::default());
        assert!(none.steps.iter().all(|s| !s.name.starts_with("red2d") && s.name != "web"), "{:?}", names(&none));
    }

    #[test]
    fn a_change_to_the_engine_plans_the_3d_browser_build_in_the_full_tier_only() {
        for f in ["src/viewer.rs", "src/web3d.rs", "crates/web3d/src/lib.rs", "Cargo.toml", ".cargo/config.toml"] {
            assert!(touches_web3d(&[f.to_string()]), "{f}");
        }
        assert!(!touches_web3d(&["docs/WEB_PLATFORM.md".to_string(), "crates/red2d/src/sim.rs".to_string()]));
        let p = plan(&world(), &serial(), &["src/viewer.rs".to_string()], &Options::default());
        assert_eq!(step(&p, "web3d-wasm").argv.join(" "), "bash scripts/ci.sh web3d");
        let quick = plan(&world(), &serial(), &["src/viewer.rs".to_string()], &Options { quick: true, ..Options::default() });
        assert!(quick.steps.iter().all(|s| s.name != "web3d-wasm"), "{:?}", names(&quick));
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

    const ROOT: &str = "//! Crate docs.\npub mod a;\n#[cfg(feature = \"gfx\")]\npub mod b;\n\npub fn load() -> u32 {\n    1\n}\n";

    #[test]
    fn a_crate_root_that_only_gains_modules_does_not_escalate() {
        let plain = ROOT.replace("pub mod a;\n", "pub mod a;\npub mod c;\n");
        assert!(crate_root_only_adds_modules(ROOT, &plain), "a bare `pub mod`");
        let gated = ROOT.replace("pub fn load", "// A new module.\n#[cfg(feature = \"gfx\")]\npub mod c;\n\npub fn load");
        assert!(crate_root_only_adds_modules(ROOT, &gated), "a gated module with a comment");
    }

    #[test]
    fn any_other_crate_root_change_still_escalates() {
        let cases = [
            ("no change at all", ROOT.to_string()),
            ("a module removed", ROOT.replace("pub mod a;\n", "")),
            ("a module re-gated", ROOT.replace("pub mod a;", "#[cfg(feature = \"gfx\")]\npub mod a;")),
            ("a gate taken off", ROOT.replace("#[cfg(feature = \"gfx\")]\n", "")),
            (
                "a module added in front of an existing gate (the gate moves to it)",
                ROOT.replace("#[cfg(feature = \"gfx\")]\npub mod b;", "#[cfg(feature = \"gfx\")]\npub mod c;\npub mod b;"),
            ),
            ("a module and a function change", ROOT.replace("pub mod a;\n", "pub mod a;\npub mod c;\n").replace("1\n", "2\n")),
            ("a module and a use", ROOT.replace("pub mod a;\n", "pub mod a;\npub mod c;\npub use glam;\n")),
            ("the crate docs", ROOT.replace("Crate docs.", "Other docs.").replace("pub mod a;\n", "pub mod a;\npub mod c;\n")),
        ];
        for (what, new) in cases {
            assert!(!crate_root_only_adds_modules(ROOT, &new), "{what}");
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
            features: Features::Default,
            full_required: vec![],
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

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("re2_affected_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn partial_of(changed: &[&str], opts: &Options) -> Plan {
        let changed: Vec<String> = changed.iter().map(|c| c.to_string()).collect();
        plan_partial(&world(), &serial(), &changed, opts, Features::Default)
    }

    #[test]
    fn a_partial_plan_type_checks_and_runs_the_touched_modules_and_always_says_full_verification_is_still_required() {
        let p = partial_of(&["src/sim/flow.rs"], &Options { partial: true, ..Options::default() });
        assert_eq!(p.scope, Scope::Partial);
        assert_eq!(names(&p), ["fmt", "check", "unit"]);
        assert_eq!(step(&p, "check").argv.join(" "), "cargo clippy --locked --lib --bins -- -D warnings");
        assert_eq!(step(&p, "unit").argv.join(" "), "cargo test --locked --lib -- sim::flow");
        assert!(p.escalated.is_none());
        assert!(p.deferred.iter().any(|d| d == "net_flow"), "the owner's integration suites are listed as not run: {:?}", p.deferred);
        assert!(p.full_required.iter().any(|r| r.contains("not verification")), "{:?}", p.full_required);
        let text = render_plan(&p);
        assert!(text.contains("PARTIAL") && text.contains("full verification still required"), "{text}");
        assert_eq!(plan_json(&p)["partial"], true);
        // Even a change with nothing deferred carries the reminder.
        let docs = partial_of(&["README.md"], &Options { partial: true, ..Options::default() });
        assert!(docs.steps.is_empty() && !docs.full_required.is_empty(), "{docs:?}");
    }

    #[test]
    fn a_partial_plan_never_escalates_to_full_ci_and_check_only_drops_the_tests() {
        for boundary in ["Cargo.toml", "src/lib.rs", "scripts/ci.sh"] {
            let p = partial_of(&[boundary, "src/sim/flow.rs"], &Options { partial: true, ..Options::default() });
            assert_eq!(p.scope, Scope::Partial, "{boundary}");
            assert!(p.steps.iter().all(|s| s.name != "ci"), "{boundary}: {:?}", names(&p));
            assert!(p.full_required.iter().any(|r| r.contains(boundary)), "{boundary}: {:?}", p.full_required);
        }
        let quick = partial_of(&["src/sim/flow.rs"], &Options { partial: true, check_only: true, ..Options::default() });
        assert_eq!(names(&quick), ["fmt", "check"]);
        // A changed test file runs itself (serial suites one at a time); nothing else is run for it.
        let t = partial_of(&["tests/net_e2e.rs"], &Options { partial: true, ..Options::default() });
        assert_eq!(step(&t, "suites-serial").env, vec![("RUST_TEST_THREADS".to_string(), "1".to_string())]);
        assert_eq!(step(&t, "check").argv.join(" "), "cargo clippy --locked --test net_e2e -- -D warnings");
    }

    #[test]
    fn the_feature_set_is_chosen_explicitly_and_reaches_every_cargo_step() {
        let changed = vec!["src/sim/flow.rs".to_string()];
        let p = plan_partial(&world(), &serial(), &changed, &Options { partial: true, ..Options::default() }, Features::Headless);
        assert_eq!(p.features, Features::Headless);
        assert!(step(&p, "check").argv.contains(&"--no-default-features".to_string()));
        assert!(step(&p, "unit").argv.contains(&"--no-default-features".to_string()));
        assert_eq!(plan_json(&p)["features"], "headless");
        let d = plan_partial(&world(), &serial(), &changed, &Options { partial: true, ..Options::default() }, Features::Default);
        assert!(d.steps.iter().all(|s| !s.argv.contains(&"--no-default-features".to_string())));
    }

    #[test]
    fn headless_is_used_only_for_files_that_are_provably_not_graphics_code() {
        let dir = scratch("headless");
        std::fs::create_dir_all(dir.join("src/sim")).unwrap();
        std::fs::create_dir_all(dir.join("src/bin/re2")).unwrap();
        std::fs::write(
            dir.join("src/lib.rs"),
            "pub mod sim;\n#[cfg(feature = \"gfx\")]\n#[allow(missing_docs)]\npub mod render;\n#[cfg(feature = \"gfx\")]\nmod gpu;\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/sim/clock.rs"), "pub fn tick() {}\n").unwrap();
        std::fs::write(dir.join("src/sim/mixed.rs"), "#[cfg(feature = \"gfx\")]\nfn draw() {}\n").unwrap();
        std::fs::write(dir.join("src/render.rs"), "pub fn draw() {}\n").unwrap();
        std::fs::write(dir.join("src/bin/re2/main.rs"), "fn main() {}\n").unwrap();
        let ok = |files: &[&str]| headless_safe(&dir, &files.iter().map(|f| f.to_string()).collect::<Vec<_>>());
        assert_eq!(ok(&["src/sim/clock.rs", "docs/x.md"]), Ok(()));
        assert!(ok(&["src/render.rs"]).unwrap_err().contains("only with the gfx feature"));
        assert!(ok(&["src/sim/clock.rs", "src/sim/mixed.rs"]).unwrap_err().contains("gfx-only code inside"));
        assert!(ok(&["src/bin/re2/main.rs"]).unwrap_err().contains("live client"));
        assert!(ok(&["tests/net_e2e.rs"]).unwrap_err().contains("default features"));
        assert!(ok(&["Cargo.toml"]).unwrap_err().contains("every feature set"));
        assert!(ok(&["src/missing.rs"]).is_err(), "an unreadable file is not assumed safe");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_partial_pass_never_stands_in_for_any_other_scope_and_nothing_stands_in_for_it() {
        let dir = scratch("partial_stamp");
        std::fs::write(dir.join("a.txt"), "one").unwrap();
        let files = vec!["a.txt".to_string()];
        let key = StampKey { base: "abc".into(), config: "features=default".into(), variant: String::new() };
        assert_eq!(scopes_covering(Scope::Partial), vec![Scope::Partial]);
        assert!(scopes_covering(Scope::Quick).iter().all(|s| *s != Scope::Partial));
        record_green_in(&dir, &files, Scope::Partial, &key);
        assert_eq!(already_green_in(&dir, &files, Scope::Partial, &key), Some(Scope::Partial));
        for scope in [Scope::Quick, Scope::Closure, Scope::Full] {
            assert_eq!(already_green_in(&dir, &files, scope, &key), None, "a partial pass must not satisfy {scope:?}");
        }
        let dir2 = scratch("partial_stamp_full");
        std::fs::write(dir2.join("a.txt"), "one").unwrap();
        record_green_in(&dir2, &files, Scope::Full, &key);
        assert_eq!(already_green_in(&dir2, &files, Scope::Partial, &key), None, "and a full pass is not assumed to have run the partial steps");
        assert_eq!(already_green_in(&dir2, &files, Scope::Quick, &key), Some(Scope::Full), "the ordinary tiers still cover each other");
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&dir2).unwrap();
    }

    #[test]
    fn a_green_stamp_is_not_reused_across_base_configuration_toolchain_variant_or_index() {
        let dir = scratch("stamp_key");
        std::fs::write(dir.join("a.txt"), "one").unwrap();
        let files = vec!["a.txt".to_string()];
        let cfg = |features: Features, toolchain: &str, env: &[(&str, &str)]| {
            Config {
                features,
                toolchain: toolchain.into(),
                env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                jobs: None,
                target_dir: "target".into(),
            }
            .id()
        };
        let key = |base: &str, config: String, variant: &str| StampKey { base: base.into(), config, variant: variant.into() };
        let base = key("b1", cfg(Features::Default, "1.98.1", &[]), "");
        record_green_in(&dir, &files, Scope::Quick, &base);
        assert_eq!(already_green_in(&dir, &files, Scope::Quick, &base), Some(Scope::Quick));
        let variants = [
            ("another base commit", key("b2", cfg(Features::Default, "1.98.1", &[]), "")),
            ("headless features", key("b1", cfg(Features::Headless, "1.98.1", &[]), "")),
            ("another toolchain", key("b1", cfg(Features::Default, "1.99.0", &[]), "")),
            ("another RUSTFLAGS", key("b1", cfg(Features::Default, "1.98.1", &[("RUSTFLAGS", "-C target-cpu=native")]), "")),
            ("another test-thread setting", key("b1", cfg(Features::Default, "1.98.1", &[("RUST_TEST_THREADS", "1")]), "")),
            ("another plan variant", key("b1", cfg(Features::Default, "1.98.1", &[]), "check-only")),
        ];
        for (what, k) in &variants {
            assert_eq!(already_green_in(&dir, &files, Scope::Quick, k), None, "a green result must not cross {what}");
        }
        // Build speed is not part of the identity: the same result under a different job count or target directory is the same result.
        let fast = Config { features: Features::Default, toolchain: "1.98.1".into(), env: vec![], jobs: Some("2".into()), target_dir: "/elsewhere".into() };
        assert_eq!(fast.id(), cfg(Features::Default, "1.98.1", &[]));
        assert!(fast.render().contains("jobs=2") && fast.render().contains("/elsewhere"));
        // The index on disk is part of the key: adding a file to a feature changes what a plan covers.
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/features.json"), "{\"features\":{}}").unwrap();
        assert_eq!(already_green_in(&dir, &files, Scope::Quick, &base), None, "an edited feature index invalidates the stamp");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_runnable_doc_examples_keep_the_doc_step() {
        assert!(has_runnable_doc_example("/// ```\n/// let x = 1;\n/// ```\nfn f() {}"));
        assert!(has_runnable_doc_example("//! ```rust\n//! let x = 1;\n//! ```"));
        assert!(has_runnable_doc_example("/// ```should_panic\n/// panic!();\n/// ```"));
        assert!(!has_runnable_doc_example("//! ```json\n//! {}\n//! ```"), "a json block is documentation");
        assert!(!has_runnable_doc_example("/// ```text\n/// hello\n/// ```\n/// ```sh\n/// ls\n/// ```"));
        assert!(!has_runnable_doc_example("/// ```ignore\n/// nope\n/// ```"), "ignored blocks are not run");
        assert!(!has_runnable_doc_example("// ```\n// not a doc comment\n// ```"));
        // The closing fence of a non-rust block is not an opening fence.
        assert!(!has_runnable_doc_example("//! ```json\n//! {}\n//! ```\nfn f() {}"));
        assert!(has_runnable_doc_example("//! ```json\n//! {}\n//! ```\n//! ```\n//! let x = 1;\n//! ```"));
    }

    #[test]
    fn gated_modules_are_read_from_the_crate_root() {
        let gated = gfx_modules("pub mod a;\n#[cfg(feature = \"gfx\")]\npub mod b;\n#[cfg(feature = \"gfx\")]\n#[allow(dead_code)]\nmod c;\npub mod d;\n");
        assert_eq!(gated.into_iter().collect::<Vec<_>>(), ["b", "c"]);
    }
}
