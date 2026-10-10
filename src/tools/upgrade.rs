//! `red_engine2 game upgrade plan|verify`: moving a game project (ADR 0024) to a different pinned
//! engine commit, safely. See `docs/adr/2026-10-01-game-upgrade-planning-and-staged-verification.md`.
//!
//! `plan` ([`run_plan`]) is read-only: it reads `game.json`, the project's own Cargo manifests, and
//! (via `git show` against a reachable checkout, never a checkout switch) source text at specific
//! commits, resolves the target revision to one fixed commit, matches [`Migration`] records against
//! real evidence, and writes a packet. It never edits the project, never builds an engine, and never
//! touches the network on its own.
//!
//! `verify` ([`run_verify`]) is the only place that builds anything, and only into its own identity-
//! stamped cache directory ([`ensure_target_build`]); it stages the project's content into a scratch
//! copy ([`stage_and_build`]) and diffs the result against the real, untouched files.

use super::diff;
use super::game::{self, GameConfig, Line};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the migration registry lives in this checkout (read live, not compiled in, like `docs/features.json`'s
/// on-disk form — an edit to it needs no rebuild).
pub const MIGRATIONS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/upgrade-migrations.json");

// ---------------------------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------------------------

/// What kind of project the evidence supports — never assumed, only read off disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectClass {
    /// A `game.json` project whose own blueprints/maps are the whole game (ADR 0024).
    StandardBlueprint,
    /// A `game.json` project with at least one sibling Cargo crate naming `red_engine2` as a dependency.
    CustomRustClient { crates: Vec<PathBuf> },
    /// Not enough evidence to call it either of the above; a reason, never a guess.
    Unknown { reason: String },
}

/// How a project's engine reference resolves right now. A local path is not an immutable revision:
/// `sha` is what it resolved to *at the moment this was read*, not a promise it will stay there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEngine {
    pub kind: &'static str, // "path" | "git"
    pub location: String,
    pub sha: Option<String>, // None when it could not be resolved without a side effect (clone, fetch)
    pub dirty: bool,
}

/// A Cargo manifest under the project that names `red_engine2` as a dependency, and what it resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CargoEngineRef {
    pub manifest: PathBuf,
    pub kind: &'static str, // "path" | "git"
    pub location: String,
    pub sha: Option<String>,
}

/// Everything [`run_plan`] reasons about, gathered read-only.
#[derive(Debug, Clone)]
pub struct Evidence {
    pub game: GameConfig,
    pub current: ResolvedEngine,
    pub cargo: Vec<CargoEngineRef>,
    pub class: ProjectClass,
    pub drift: Vec<Line>,
}

fn git_read_only(checkout: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").arg("-C").arg(checkout).args(args).output().map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// A checkout's HEAD commit, or `None` if it is not a git checkout / has no commits — a probe, never a refusal.
fn probe_sha(checkout: &Path) -> Option<String> {
    git_read_only(checkout, &["rev-parse", "HEAD"]).ok()
}

fn probe_dirty(checkout: &Path) -> bool {
    git_read_only(checkout, &["status", "--porcelain", "--untracked-files=no"]).map(|s| !s.is_empty()).unwrap_or(false)
}

fn resolve_current_engine(cfg: &GameConfig, explicit: Option<&Path>) -> ResolvedEngine {
    if let Some(p) = explicit.map(Path::to_path_buf).or_else(|| cfg.engine.path.as_ref().map(|p| cfg.dir.join(p))) {
        return ResolvedEngine { kind: "path", location: p.display().to_string(), sha: probe_sha(&p), dirty: probe_dirty(&p) };
    }
    if let Some(g) = &cfg.engine.git {
        return ResolvedEngine { kind: "git", location: g.clone(), sha: cfg.engine.git_ref.clone(), dirty: false };
    }
    ResolvedEngine { kind: "path", location: "unspecified".into(), sha: None, dirty: false }
}

/// A line like `red_engine2 = { path = "../engine" }` or `{ git = "...", rev = "..." }`: good enough for the one
/// dependency this scans for (the same spirit as `scripts/red`'s own line-scanning `json_get`, not a real TOML parser).
fn parse_red_engine2_dep(text: &str) -> Option<(&'static str, String, Option<String>)> {
    let line = text.lines().find(|l| l.trim_start().starts_with("red_engine2"))?;
    let field = |key: &str| -> Option<String> {
        let pat = format!("{key} = \"");
        let start = line.find(&pat)? + pat.len();
        let end = start + line[start..].find('"')?;
        Some(line[start..end].to_string())
    };
    if let Some(path) = field("path") {
        Some(("path", path, None))
    } else if let Some(git) = field("git") {
        let sha = field("rev").or_else(|| field("tag")).or_else(|| field("branch"));
        Some(("git", git, sha))
    } else {
        None
    }
}

fn collect_manifests(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !matches!(name.as_str(), "out" | "target" | ".git" | "node_modules" | ".red") {
                collect_manifests(&p, out);
            }
        } else if name == "Cargo.toml" {
            out.push(p);
        }
    }
}

/// Every Cargo manifest under `project_dir` that names `red_engine2`, with its dependency resolved to a commit
/// where that is possible without a side effect (a `path` dependency's checkout HEAD; a `git` dependency's own
/// `rev`/`tag`/`branch` field, as written, not fetched).
pub fn find_cargo_engine_refs(project_dir: &Path) -> Vec<CargoEngineRef> {
    let mut manifests = Vec::new();
    collect_manifests(project_dir, &mut manifests);
    let mut out = Vec::new();
    for m in manifests {
        let Ok(text) = std::fs::read_to_string(&m) else { continue };
        let Some((kind, location, mut sha)) = parse_red_engine2_dep(&text) else { continue };
        if kind == "path" {
            let dir = m.parent().unwrap_or(Path::new(".")).join(&location);
            sha = probe_sha(&dir);
        }
        out.push(CargoEngineRef { manifest: m, kind, location, sha });
    }
    out
}

/// Classifies a project from evidence, never from assumption (task step 3).
pub fn classify(cfg: &GameConfig, cargo: &[CargoEngineRef]) -> ProjectClass {
    if !cargo.is_empty() {
        return ProjectClass::CustomRustClient { crates: cargo.iter().map(|c| c.manifest.clone()).collect() };
    }
    if cfg.blueprints.is_empty() && cfg.maps.is_empty() {
        return ProjectClass::Unknown { reason: "game.json lists no blueprints or maps".into() };
    }
    ProjectClass::StandardBlueprint
}

/// Gathers everything [`run_plan`] needs. Read-only: no file is written, no engine is built, nothing is cloned.
pub fn gather_evidence(game_dir: &Path, explicit_engine: Option<&Path>) -> Result<Evidence, Vec<String>> {
    let cfg = game::load(game_dir)?;
    let current = resolve_current_engine(&cfg, explicit_engine);
    let cargo = find_cargo_engine_refs(&cfg.dir);
    let class = classify(&cfg, &cargo);
    let drift = game::blueprint_drift(&cfg);
    Ok(Evidence { game: cfg, current, cargo, class, drift })
}

/// Resolves `rev` (branch, tag or commit) to one fixed commit hash, against `checkout`, without a checkout switch
/// or a network fetch. Refuses a dirty checkout: resolving a name like `HEAD` against uncommitted changes would
/// silently pick a commit that is not what is actually on disk there (the same reasoning as `game::engine_pin`).
pub fn resolve_target(checkout: &Path, rev: &str) -> Result<String, String> {
    if !checkout.join(".git").exists() {
        return Err(format!("{} is not a git checkout: pass --engine <dir> containing the target revision", checkout.display()));
    }
    if probe_dirty(checkout) {
        return Err(format!(
            "{} has uncommitted changes: resolving '{rev}' there would not be reproducible (commit them, or point --engine at a clean checkout)",
            checkout.display()
        ));
    }
    git_read_only(checkout, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")])
        .map_err(|e| format!("'{rev}' could not be resolved in {}: {e} (this never fetches on its own; fetch it there first)", checkout.display()))
}

/// A file's text at a specific commit, read via `git show` (no checkout switch, no working-tree change).
pub fn git_show(checkout: &Path, sha: &str, path: &str) -> Result<String, String> {
    git_read_only(checkout, &["show", &format!("{sha}:{path}")])
}

// ---------------------------------------------------------------------------------------------
// Migrations
// ---------------------------------------------------------------------------------------------

/// One known, previously-verified engine compatibility change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    pub id: String,
    pub affected: String,
    pub detect: Vec<String>,
    pub required: bool,
    pub repair: String,
    pub verify: String,
    pub limitations: String,
}

/// Whether a migration applies to this project's upgrade. A text-search hit is [`Confidence::Possible`], never
/// [`Confidence::Applicable`] on its own — only a detector with real evidence (a parsed constant, a real git read)
/// returns `Applicable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confidence {
    Applicable,
    Possible(String),
    NotApplicable,
}

/// Reads `docs/upgrade-migrations.json` (the sidecar registry, read live from the checkout like `docs/features.json`).
pub fn load_migrations(path: &Path) -> Result<Vec<Migration>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: not valid JSON: {e}", path.display()))?;
    let arr = v.get("migrations").and_then(Value::as_array).ok_or_else(|| format!("{}: needs a top-level \"migrations\" array", path.display()))?;
    let str_field = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let str_list =
        |o: &Value, k: &str| o.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    Ok(arr
        .iter()
        .map(|m| Migration {
            id: str_field(m, "id"),
            affected: str_field(m, "affected"),
            detect: str_list(m, "detect"),
            required: m.get("required").and_then(Value::as_bool).unwrap_or(false),
            repair: str_field(m, "repair"),
            verify: str_field(m, "verify"),
            limitations: str_field(m, "limitations"),
        })
        .collect())
}

fn protocol_version(checkout: &Path, sha: &str) -> Option<u16> {
    let text = git_show(checkout, sha, "src/net/protocol.rs").ok()?;
    text.lines().find_map(|l| l.trim().strip_prefix("pub const PROTOCOL_VERSION: u16 = ")?.strip_suffix(';')?.parse().ok())
}

fn protocol_version_confidence(ev: &Evidence, checkout: &Path, baseline: Option<&str>, target: &str) -> Confidence {
    if ev.game.server.map.is_empty() {
        return Confidence::NotApplicable;
    }
    let Some(baseline) = baseline else {
        return Confidence::Possible("the baseline engine commit could not be resolved: cannot compare protocol versions".into());
    };
    match (protocol_version(checkout, baseline), protocol_version(checkout, target)) {
        (Some(a), Some(b)) if a != b => Confidence::Applicable,
        (Some(_), Some(_)) => Confidence::NotApplicable,
        _ => Confidence::Possible(format!("could not read PROTOCOL_VERSION from {} at the baseline or target commit", checkout.display())),
    }
}

fn verify_groups(checkout: &Path, sha: &str) -> Option<Vec<String>> {
    let text = git_show(checkout, sha, "src/tools/verify.rs").ok()?;
    let line = text.lines().find(|l| l.trim_start().starts_with("const GROUPS"))?;
    let (_, after) = line.rsplit_once('[')?;
    let inside = after.split(']').next()?;
    Some(inside.split(',').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()).collect())
}

fn checks_groups_confidence(checkout: &Path, baseline: Option<&str>, target: &str) -> Confidence {
    let Some(baseline) = baseline else {
        return Confidence::Possible("the baseline engine commit could not be resolved: cannot compare `checks` groups".into());
    };
    match (verify_groups(checkout, baseline), verify_groups(checkout, target)) {
        (Some(a), Some(b)) => {
            if b.iter().any(|g| !a.contains(g)) {
                Confidence::Applicable
            } else {
                Confidence::NotApplicable
            }
        }
        _ => Confidence::Possible("could not read the GROUPS constant from the baseline or target commit".into()),
    }
}

/// Matches every known migration against this project's evidence. Each migration id has its own small, honest
/// detector (never a generic text-matcher: "a text search is a hint, not proof", task step 5) — an id with no
/// detector implemented yet comes back `Possible`, not silently `Applicable` or `NotApplicable`.
pub fn applicable(migrations: &[Migration], ev: &Evidence, checkout: &Path, baseline: Option<&str>, target: &str) -> Vec<(Migration, Confidence)> {
    migrations
        .iter()
        .cloned()
        .map(|m| {
            let c = match m.id.as_str() {
                "protocol-version-lockstep" => protocol_version_confidence(ev, checkout, baseline, target),
                "verify-checks-groups-grew" => checks_groups_confidence(checkout, baseline, target),
                _ => Confidence::Possible("no detector implemented for this migration id yet: review by hand".into()),
            };
            (m, c)
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Packet
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    RequiredRepair,
    RequestedRepair,
    OptionalModernization,
}

/// The upgrade packet: exact baseline/target, requested problems, applicable migrations, relevant files, next
/// checks and uncertainties — not the engine changelog (task step 5).
#[derive(Debug, Clone)]
pub struct Packet {
    pub baseline: ResolvedEngine,
    pub target_sha: String,
    pub target_resolved_from: PathBuf,
    pub class: ProjectClass,
    pub requested: Vec<String>,
    pub migrations: Vec<(Migration, Confidence)>,
    pub changes: Vec<(ChangeKind, String)>,
    pub files: Vec<String>,
    pub next_checks: Vec<String>,
    pub uncertainties: Vec<String>,
    pub millis: u128,
}

/// Builds the packet from already-gathered evidence and already-matched migrations. Pure: no I/O.
pub fn plan(ev: &Evidence, target_sha: String, target_resolved_from: &Path, fixes: &[String], migrations: &[(Migration, Confidence)]) -> Packet {
    let mut changes = Vec::new();
    let mut uncertainties = Vec::new();

    for (m, c) in migrations {
        match c {
            Confidence::Applicable => {
                changes.push((if m.required { ChangeKind::RequiredRepair } else { ChangeKind::OptionalModernization }, format!("{}: {}", m.id, m.repair)))
            }
            Confidence::Possible(why) => uncertainties.push(format!("{}: possibly applicable ({why}); not certified either way", m.id)),
            Confidence::NotApplicable => {}
        }
    }
    for f in fixes {
        changes.push((ChangeKind::RequestedRepair, f.clone()));
    }
    for c in &ev.cargo {
        if let (Some(cs), Some(gs)) = (&c.sha, &ev.current.sha) {
            if cs != gs {
                changes.push((
                    ChangeKind::RequiredRepair,
                    format!(
                        "{} depends on red_engine2 at a different commit ({cs}) than game.json's own engine ({gs}): resolve before upgrading",
                        c.manifest.display()
                    ),
                ));
            }
        }
    }
    if ev.drift.iter().any(|l| l.failed) {
        uncertainties.push(
            "the committed map(s) already differ from what the current blueprint builds (pre-existing drift, not caused by this upgrade): \
             resolve this first, or regeneration cannot tell a hand edit apart from an engine change"
                .into(),
        );
    }
    if let ProjectClass::Unknown { reason } = &ev.class {
        uncertainties
            .push(format!("project layout could not be classified from evidence ({reason}): proposing a rehabilitation plan, not an automatic upgrade route"));
    }

    let mut files: Vec<String> = ev.game.blueprints.clone();
    files.extend(ev.game.maps.clone());
    files.extend(ev.cargo.iter().map(|c| c.manifest.display().to_string()));

    let next_checks =
        vec!["red_engine2 game upgrade verify <packet.json>".to_string(), "scripts/red check (after `game pin` to the target commit)".to_string()];

    Packet {
        baseline: ev.current.clone(),
        target_sha,
        target_resolved_from: target_resolved_from.to_path_buf(),
        class: ev.class.clone(),
        requested: fixes.to_vec(),
        migrations: migrations.to_vec(),
        changes,
        files,
        next_checks,
        uncertainties,
        millis: 0,
    }
}

fn class_json(c: &ProjectClass) -> Value {
    match c {
        ProjectClass::StandardBlueprint => json!({"kind": "standard_blueprint"}),
        ProjectClass::CustomRustClient { crates } => {
            json!({"kind": "custom_rust_client", "crates": crates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()})
        }
        ProjectClass::Unknown { reason } => json!({"kind": "unknown", "reason": reason}),
    }
}

/// The inverse of [`class_json`]: reads a packet's `class` back. An unrecognized or missing `kind` is read as
/// `Unknown`, never silently as `StandardBlueprint` — a malformed packet must not look like the safest, most
/// ordinary project class.
fn class_from_json(v: &Value) -> ProjectClass {
    match v.get("kind").and_then(Value::as_str) {
        Some("standard_blueprint") => ProjectClass::StandardBlueprint,
        Some("custom_rust_client") => ProjectClass::CustomRustClient {
            crates: v.get("crates").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(PathBuf::from).collect()).unwrap_or_default(),
        },
        Some("unknown") => ProjectClass::Unknown { reason: v.get("reason").and_then(Value::as_str).unwrap_or("packet did not say").to_string() },
        _ => ProjectClass::Unknown { reason: "packet's class field is missing or unrecognized".to_string() },
    }
}

fn confidence_json(c: &Confidence) -> Value {
    match c {
        Confidence::Applicable => json!("applicable"),
        Confidence::Possible(why) => json!({"possible": why}),
        Confidence::NotApplicable => json!("not_applicable"),
    }
}

fn confidence_text(c: &Confidence) -> String {
    match c {
        Confidence::Applicable => "applicable".into(),
        Confidence::Possible(why) => format!("possible: {why}"),
        Confidence::NotApplicable => "not applicable".into(),
    }
}

fn change_kind_label(k: &ChangeKind) -> &'static str {
    match k {
        ChangeKind::RequiredRepair => "required_repair",
        ChangeKind::RequestedRepair => "requested_repair",
        ChangeKind::OptionalModernization => "optional_modernization",
    }
}

pub fn packet_json(p: &Packet) -> Value {
    json!({
        "baseline": {"kind": p.baseline.kind, "location": p.baseline.location, "sha": p.baseline.sha, "dirty": p.baseline.dirty},
        "target": {"sha": p.target_sha, "resolved_from": p.target_resolved_from.display().to_string()},
        "class": class_json(&p.class),
        "requested": p.requested,
        "migrations": p.migrations.iter().map(|(m, c)| json!({
            "id": m.id, "required": m.required, "confidence": confidence_json(c),
            "affected": m.affected, "repair": m.repair, "verify": m.verify, "limitations": m.limitations,
        })).collect::<Vec<_>>(),
        "changes": p.changes.iter().map(|(k, text)| json!({"kind": change_kind_label(k), "text": text})).collect::<Vec<_>>(),
        "files": p.files,
        "next_checks": p.next_checks,
        "uncertainties": p.uncertainties,
        "millis": p.millis,
    })
}

pub fn render_packet(p: &Packet) -> String {
    let mut s = String::new();
    s.push_str(&format!("upgrade packet: {} -> {}\n", p.baseline.sha.as_deref().unwrap_or("unresolved"), &p.target_sha[..p.target_sha.len().min(12)]));
    s.push_str(&format!(
        "baseline  {} {} (sha {}{})\n",
        p.baseline.kind,
        p.baseline.location,
        p.baseline.sha.as_deref().unwrap_or("unresolved"),
        if p.baseline.dirty { ", DIRTY" } else { "" }
    ));
    s.push_str(&format!("target    {} (resolved from {})\n", p.target_sha, p.target_resolved_from.display()));
    s.push_str(&format!("class     {:?}\n", p.class));
    if !p.requested.is_empty() {
        s.push_str(&format!("requested {}\n", p.requested.join("; ")));
    }
    s.push_str("\nmigrations:\n");
    if p.migrations.is_empty() {
        s.push_str("  (none in docs/upgrade-migrations.json)\n");
    }
    for (m, c) in &p.migrations {
        s.push_str(&format!("  [{}] {} ({})\n", if m.required { "required" } else { "optional" }, m.id, confidence_text(c)));
    }
    s.push_str("\nchanges:\n");
    if p.changes.is_empty() {
        s.push_str("  (none)\n");
    }
    for (k, text) in &p.changes {
        s.push_str(&format!("  {}: {text}\n", change_kind_label(k)));
    }
    if !p.uncertainties.is_empty() {
        s.push_str("\nuncertainties (not authorized by this plan; review by hand):\n");
        for u in &p.uncertainties {
            s.push_str(&format!("  - {u}\n"));
        }
    }
    s.push_str("\nfiles:\n");
    for f in &p.files {
        s.push_str(&format!("  {f}\n"));
    }
    s.push_str("\nnext checks:\n");
    for c in &p.next_checks {
        s.push_str(&format!("  {c}\n"));
    }
    s.push_str(&format!("\nplanning took {} ms\n", p.millis));
    s
}

fn create_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    Ok(())
}

pub fn write_packet(p: &Packet, out_json: &Path, out_md: &Path) -> Result<(), String> {
    create_parent(out_json)?;
    create_parent(out_md)?;
    std::fs::write(out_json, serde_json::to_string_pretty(&packet_json(p)).map_err(|e| e.to_string())? + "\n")
        .map_err(|e| format!("{}: {e}", out_json.display()))?;
    std::fs::write(out_md, render_packet(p)).map_err(|e| format!("{}: {e}", out_md.display()))?;
    Ok(())
}

/// What [`run_plan`] produced and where it wrote it.
pub struct PlanOutcome {
    pub packet: Packet,
    pub json_path: PathBuf,
    pub md_path: PathBuf,
}

/// The whole read-only planning flow: gather evidence, resolve the target once, match migrations, write the packet.
/// Never edits `game_dir`, never builds an engine, never switches a checkout, never touches the network.
pub fn run_plan(
    game_dir: &Path,
    to: &str,
    explicit_engine: Option<&Path>,
    fixes: &[String],
    out: Option<&Path>,
    migrations_path: &Path,
) -> Result<PlanOutcome, String> {
    let start = std::time::Instant::now();
    let ev = gather_evidence(game_dir, explicit_engine).map_err(|e| e.join("\n"))?;
    let engine_checkout = explicit_engine
        .map(Path::to_path_buf)
        .or_else(|| ev.game.engine.path.as_ref().map(|p| ev.game.dir.join(p)))
        .ok_or_else(|| "no engine checkout to resolve the target against: pass --engine <checkout>, or set game.json's engine.path".to_string())?;
    let target_sha = resolve_target(&engine_checkout, to)?;
    let baseline_sha = ev.current.sha.clone();
    let migrations = load_migrations(migrations_path)?;
    let verdicts = applicable(&migrations, &ev, &engine_checkout, baseline_sha.as_deref(), &target_sha);
    let mut packet = plan(&ev, target_sha.clone(), &engine_checkout, fixes, &verdicts);
    packet.millis = start.elapsed().as_millis();

    let short = &target_sha[..target_sha.len().min(12)];
    let base = out.map(Path::to_path_buf).unwrap_or_else(|| ev.game.dir.join("out/upgrade").join(short));
    let (json_path, md_path) =
        if base.extension().is_some() { (base.clone(), base.with_extension("md")) } else { (base.join("packet.json"), base.join("packet.md")) };
    write_packet(&packet, &json_path, &md_path)?;
    Ok(PlanOutcome { packet, json_path, md_path })
}

/// Reads a packet back (the input to [`run_verify`]).
pub fn read_packet(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: not valid JSON: {e}", path.display()))
}

// ---------------------------------------------------------------------------------------------
// Verify: the only stage allowed to build or write staged content
// ---------------------------------------------------------------------------------------------

/// Identity evidence for a built engine: enough to tell two builds of "the same commit" apart when the toolchain,
/// profile, features or lockfile differ (task step 6 — "adequate identity evidence").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildIdentity {
    pub sha: String,
    pub rustc: String,
    pub profile: String,
    pub features: String,
    pub lockfile_hash: String,
}

fn identity_path(dir: &Path) -> PathBuf {
    dir.join("IDENTITY.json")
}

pub fn read_identity(dir: &Path) -> Option<BuildIdentity> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(identity_path(dir)).ok()?).ok()?;
    Some(BuildIdentity {
        sha: v.get("sha")?.as_str()?.to_string(),
        rustc: v.get("rustc")?.as_str()?.to_string(),
        profile: v.get("profile")?.as_str()?.to_string(),
        features: v.get("features")?.as_str()?.to_string(),
        lockfile_hash: v.get("lockfile_hash")?.as_str()?.to_string(),
    })
}

pub fn write_identity(dir: &Path, id: &BuildIdentity) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let doc = json!({"sha": id.sha, "rustc": id.rustc, "profile": id.profile, "features": id.features, "lockfile_hash": id.lockfile_hash});
    std::fs::write(identity_path(dir), serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// The identity a build of `sha` from `checkout` with `profile`/`features` *should* have, computed without building
/// anything (reads `rustc -Vv` and the commit's `Cargo.lock` via `git show`).
pub fn identity_for(checkout: &Path, sha: &str, profile: &str, features: &str) -> Result<BuildIdentity, String> {
    let out = Command::new("rustc").arg("-Vv").output().map_err(|e| format!("rustc: {e}"))?;
    let rustc = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let lock = git_show(checkout, sha, "Cargo.lock").unwrap_or_default();
    let lockfile_hash = crate::crypto::hex(&crate::crypto::sha256(lock.as_bytes()));
    Ok(BuildIdentity { sha: sha.to_string(), rustc, profile: profile.to_string(), features: features.to_string(), lockfile_hash })
}

fn bin_dir(cache_dir: &Path, profile: &str) -> PathBuf {
    let p = if profile == "release" {
        "release"
    } else if profile == "dev" {
        "debug"
    } else {
        profile
    };
    cache_dir.join("target").join(p)
}

fn run_checked(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        // `red_engine2` subcommands print their failure detail to stdout (via the envelope's write_out) and return
        // an empty-string error; stderr alone would silently drop the useful part.
        Err(format!("{cmd:?} failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }
}

/// How long a second caller waits for another process's build of the *same identity* before giving up — generous
/// (a cold engine build is minutes, not seconds) but finite, so a crashed builder cannot hang every future caller.
const BUILD_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const BUILD_LOCK_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// Releases a build lock on drop (including on an early return via `?` or a panic), so a failed build does not
/// leave the lock file behind forever.
#[derive(Debug)]
struct BuildLock(PathBuf);
impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Claims the right to build `dir`'s cache entry, or waits for whoever already holds it. Two concurrent
/// verifications computing the same identity must not have one delete or overwrite the checkout/build the other
/// is currently using. The lock is an atomically-created file (`create_new` fails if it already exists, on every
/// platform this engine supports — no Unix-only `flock` needed): exactly one caller gets `Some`, and every other
/// caller polls until either the lock clears (the builder finished or gave up) or `timeout` is reached, at which
/// point it fails loudly rather than silently waiting forever or barging in.
fn acquire_build_lock(dir: &Path, timeout: std::time::Duration, poll: std::time::Duration) -> Result<Option<BuildLock>, String> {
    let lock_path = dir.join(".building");
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock_path) {
        Ok(_) => Ok(Some(BuildLock(lock_path))),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let start = std::time::Instant::now();
            while start.elapsed() < timeout {
                if !lock_path.exists() {
                    return Ok(None); // the other builder finished (or cleaned up after giving up); caller re-checks identity
                }
                std::thread::sleep(poll);
            }
            Err(format!(
                "{} has been building for over {} minute(s) (lock at {}); if that process is confirmed dead, remove the lock file and retry",
                dir.display(),
                timeout.as_secs() / 60,
                lock_path.display()
            ))
        }
        Err(e) => Err(format!("{}: {e}", lock_path.display())),
    }
}

/// The compiled binary's platform-specific name: `ensure_target_build` must find this exact file before it
/// accepts a build as successful (task step 6 — "verify the expected artifacts exist before accepting a receipt").
fn bin_name() -> &'static str {
    if cfg!(windows) {
        "red_engine2.exe"
    } else {
        "red_engine2"
    }
}

/// Obtains a target-engine build: an explicit `--engine-build` reused only if its identity matches, else a cached
/// build keyed by identity, else a fresh isolated clone+build (never the engine's or an agent's own working
/// checkout — task step 6, "never rebuild identical engine binaries separately for every map"). Concurrent callers
/// for the same identity are serialized by [`acquire_build_lock`], not raced.
pub fn ensure_target_build(
    cache_root: &Path,
    engine_checkout: &Path,
    sha: &str,
    profile: &str,
    features: &str,
    explicit: Option<&Path>,
) -> Result<(PathBuf, bool), String> {
    let wanted = identity_for(engine_checkout, sha, profile, features)?;
    if let Some(dir) = explicit {
        let got = read_identity(dir).ok_or_else(|| format!("{}: no IDENTITY.json; refusing to reuse a build without identity evidence", dir.display()))?;
        if got != wanted {
            return Err(format!("{} does not match the target: has {got:?}, need {wanted:?} — refusing to reuse it", dir.display()));
        }
        let bin = bin_dir(dir, profile).join(bin_name());
        if !bin.exists() {
            return Err(format!("{} claims identity {:?} but {} is missing: refusing to reuse it", dir.display(), got, bin.display()));
        }
        return Ok((bin_dir(dir, profile), true));
    }
    let dir = cache_root.join(format!(
        "{}-{}-{}-{}",
        &sha[..sha.len().min(12)],
        wanted.profile,
        wanted.features,
        &wanted.lockfile_hash[..wanted.lockfile_hash.len().min(12)]
    ));
    loop {
        if let Some(got) = read_identity(&dir) {
            let bin = bin_dir(&dir, profile).join(bin_name());
            if got == wanted && bin.exists() {
                return Ok((bin_dir(&dir, profile), true));
            }
        }
        let Some(_lock) = acquire_build_lock(&dir, BUILD_LOCK_TIMEOUT, BUILD_LOCK_POLL)? else {
            continue; // someone else just finished (or gave up); loop re-checks identity above
        };
        // Holding the lock: safe to rebuild over whatever (if anything) is here — a stale/failed prior attempt,
        // never another live verification's in-progress work. Only the checkout/target/identity are cleared, not
        // the cache entry directory itself, which still holds our just-created lock file.
        let checkout_dir = dir.join("checkout");
        let target_dir = dir.join("target");
        let _ = std::fs::remove_dir_all(&checkout_dir);
        let _ = std::fs::remove_dir_all(&target_dir);
        let _ = std::fs::remove_file(identity_path(&dir));
        run_checked(Command::new("git").args(["clone", "--no-checkout", "--quiet"]).arg(engine_checkout).arg(&checkout_dir))?;
        run_checked(Command::new("git").arg("-C").arg(&checkout_dir).args(["checkout", "--quiet", "--detach", sha]))?;
        let mut cmd = Command::new("cargo");
        // `--locked`: the recorded identity includes a hash of Cargo.lock at this exact commit (`identity_for`) —
        // without `--locked`, Cargo is free to silently resolve a different (if compatible) set of dependency
        // versions, and the resulting binary would not actually match the identity this cache entry claims.
        cmd.current_dir(&checkout_dir).env("CARGO_TARGET_DIR", &target_dir).args(["build", "--locked", "--bins"]);
        if profile == "release" {
            cmd.arg("--release");
        } else if profile != "dev" {
            cmd.args(["--profile", profile]);
        }
        if !features.is_empty() && features != "default" {
            cmd.args(["--no-default-features", "--features", features]);
        }
        run_checked(&mut cmd)?;
        let bin = bin_dir(&dir, profile).join(bin_name());
        if !bin.exists() {
            return Err(format!("cargo build reported success but {} does not exist: refusing to publish a build identity for it", bin.display()));
        }
        write_identity(&dir, &wanted)?;
        return Ok((bin_dir(&dir, profile), false));
    }
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    for e in std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?.flatten() {
        let name = e.file_name();
        let name_s = name.to_string_lossy();
        if matches!(name_s.as_ref(), "out" | "target" | ".git" | "node_modules" | ".red") {
            continue;
        }
        let (from, to) = (e.path(), dst.join(&name));
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
        }
    }
    Ok(())
}

/// What staged regeneration found: maps whose candidate is byte-for-byte the committed one (modulo formatting/float
/// noise, via the semantic [`diff`]), and maps whose candidate differs — surfaced, never written over the real file.
pub struct ContentResult {
    pub clean: Vec<String>,
    pub conflicts: Vec<String>,
}

/// Copies the project into `stage_dir` (never touching the real project) and runs the **target** binary's own
/// `game build-all` there, then semantically diffs each candidate map against the real, untouched committed one.
pub fn stage_and_build(cfg: &GameConfig, stage_dir: &Path, target_bin_dir: &Path) -> Result<ContentResult, String> {
    let _ = std::fs::remove_dir_all(stage_dir);
    copy_tree(&cfg.dir, stage_dir)?;
    let bin = target_bin_dir.join("red_engine2");
    run_checked(Command::new(&bin).args(["game", "build-all", "--dir"]).arg(stage_dir))?;

    let mut result = ContentResult { clean: Vec::new(), conflicts: Vec::new() };
    for m in &cfg.maps {
        let real_path = cfg.dir.join(m);
        let candidate_path = stage_dir.join(m);
        let (Ok(real), Ok(candidate)) = (std::fs::read_to_string(&real_path), std::fs::read_to_string(&candidate_path)) else {
            result.conflicts.push(format!("{m}: could not read both the real and the staged candidate map"));
            continue;
        };
        let (rv, cv): (Value, Value) = match (serde_json::from_str(&real), serde_json::from_str(&candidate)) {
            (Ok(rv), Ok(cv)) => (rv, cv),
            _ => {
                result.conflicts.push(format!("{m}: not valid JSON on one side"));
                continue;
            }
        };
        let d = diff::diff(&rv, &cv);
        if d.changed {
            result.conflicts.push(format!("{m}:\n{}", d.text));
        } else {
            result.clean.push(m.clone());
        }
    }
    Ok(result)
}

/// One stage's outcome, timed separately (task step 6 — "measure planning, compilation and test execution separately").
#[derive(Clone)]
pub struct StageResult {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub ms: u128,
}

pub struct VerifyReport {
    pub baseline_sha: Option<String>,
    pub target_sha: String,
    /// The project's class, re-derived fresh from disk at verify time (never trusted from the packet alone —
    /// see [`run_verify`]'s staleness check), since whether a custom Rust client exists changes what "verified"
    /// can honestly mean for this project.
    pub class: ProjectClass,
    pub build_identity: BuildIdentity,
    pub build_reused: bool,
    /// Whether a target-engine build was actually obtained (false for `--only baseline`, which never builds).
    pub target_built: bool,
    pub migrations: Vec<(Migration, Confidence)>,
    pub stages: Vec<StageResult>,
    pub conflicts: Vec<String>,
    pub clean_maps: Vec<String>,
}

impl VerifyReport {
    /// Whether every stage that actually *ran* passed. Says nothing about which stages those were: a `--only
    /// baseline` run that passes is `scope_ok()` without the candidate target ever having been built or checked.
    /// This is what the CLI's exit code reflects (did the requested scope succeed) — never read it as "the
    /// upgrade is safe to apply."
    pub fn scope_ok(&self) -> bool {
        self.stages.iter().all(|s| s.ok)
    }

    /// Whether the candidate target engine/game combination was actually built and exercised end to end — a
    /// target build, staged content regeneration, and the full project check all ran (not skipped by a narrower
    /// `--only`) and passed. False for `--only baseline`, `--only target-build` and `--only content`: each is a
    /// real, useful partial check, but none of them is proof the candidate works.
    pub fn candidate_verified(&self) -> bool {
        self.target_built && self.stages.iter().any(|s| s.name == "content") && self.stages.iter().any(|s| s.name == "full") && self.scope_ok()
    }

    /// Required migrations this report cannot certify resolved: `required` and not `NotApplicable`. Nothing here
    /// auto-applies a repair or confirms by hand that one was done, so any of these staying `Applicable` (a
    /// known-needed repair) or `Possible` (unknown; review by hand) blocks [`ready_to_apply`](Self::ready_to_apply)
    /// until a fresh `game upgrade plan` shows it resolved.
    pub fn unresolved_required_migrations(&self) -> Vec<&Migration> {
        self.migrations.iter().filter(|(m, c)| m.required && !matches!(c, Confidence::NotApplicable)).map(|(m, _)| m).collect()
    }

    /// Whether this project has a verification path no stage here actually exercises: a custom Rust client. The
    /// `content`/`full` stages only regenerate and check blueprint-built maps through the engine's own CLI —
    /// successful map regeneration is not proof a sibling client crate still compiles or runs against the target.
    pub fn unverified_custom_client(&self) -> bool {
        matches!(self.class, ProjectClass::CustomRustClient { .. })
    }

    /// Whether this report licenses `game pin --sha <target_sha>`: the candidate was actually verified end to
    /// end, every stage that ran passed, no required migration is left unresolved, and (for a custom Rust
    /// client) that unverified path is not silently waved through by a clean map regeneration.
    pub fn ready_to_apply(&self) -> bool {
        self.candidate_verified() && self.unresolved_required_migrations().is_empty() && !self.unverified_custom_client()
    }
}

pub struct VerifyOptions<'a> {
    pub engine_build: Option<&'a Path>,
    pub only: Option<&'a str>,
    pub cache_root: PathBuf,
}

/// The stages `--only` may select, in pipeline order. Each later stage needs everything before it as a
/// prerequisite (`content` has nothing to regenerate against without a target build, `full` has nothing to check
/// without staged content) — selecting one runs it and its prerequisites, reusing the target build by identity
/// when it is already cached, and stops there. It never silently continues past the requested stage.
const STAGE_NAMES: &[&str] = &["baseline", "target-build", "content", "full"];

/// Rejects an unrecognized `--only` value up front, by name, instead of letting it fall through to "run
/// everything" — the behaviour at the review anchor for anything other than the literal string `"baseline"`.
fn validate_only(only: Option<&str>) -> Result<Option<&str>, String> {
    match only {
        None => Ok(None),
        Some(s) if STAGE_NAMES.contains(&s) => Ok(Some(s)),
        Some(s) => Err(format!("unknown --only stage '{s}': one of {} (or omit --only to run the full pipeline)", STAGE_NAMES.join(", "))),
    }
}

fn run_capture(cmd: &mut Command) -> (bool, String) {
    match cmd.output() {
        Ok(out) => (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))),
        Err(e) => (false, e.to_string()),
    }
}

/// Target sha, the checkout it was resolved from, the baseline sha, the project class, and the migration verdicts.
type PacketPieces = (String, PathBuf, Option<String>, ProjectClass, Vec<(Migration, Confidence)>);

/// Reads a packet's own JSON back into the pieces [`run_verify`] needs. Unlike the review anchor, nothing here
/// quietly becomes "no migrations" or "not applicable" when evidence is missing or unreadable: a missing
/// migrations array, an unreadable `docs/upgrade-migrations.json`, or a confidence value this code does not
/// recognize is an error, not a silent empty/negative default — a required migration must never vanish because a
/// file could not be read. A migration id the current registry no longer has (it was renamed or removed since
/// this packet was planned) keeps the packet's own `required` flag and is flagged as registry drift instead of
/// being dropped.
fn packet_pieces(doc: &Value, migrations_path: &Path) -> Result<PacketPieces, String> {
    let target_sha = doc.get("target").and_then(|t| t.get("sha")).and_then(Value::as_str).ok_or("packet: missing target.sha")?.to_string();
    let resolved_from =
        doc.get("target").and_then(|t| t.get("resolved_from")).and_then(Value::as_str).ok_or("packet: missing target.resolved_from")?.to_string();
    let baseline_sha = doc.get("baseline").and_then(|b| b.get("sha")).and_then(Value::as_str).map(str::to_string);
    let class = class_from_json(doc.get("class").ok_or("packet: missing class")?);
    let registry = load_migrations(migrations_path).map_err(|e| format!("could not read the migration registry: {e}"))?;
    let migrations_arr = doc.get("migrations").and_then(Value::as_array).ok_or("packet: missing or malformed \"migrations\" array")?;
    let mut migrations = Vec::with_capacity(migrations_arr.len());
    for m in migrations_arr {
        let id = m.get("id").and_then(Value::as_str).ok_or("packet: a migration entry is missing its \"id\"")?.to_string();
        let required = m.get("required").and_then(Value::as_bool).ok_or_else(|| format!("packet: migration '{id}' is missing \"required\""))?;
        let confidence = match m.get("confidence") {
            Some(Value::String(s)) if s == "applicable" => Confidence::Applicable,
            Some(Value::String(s)) if s == "not_applicable" => Confidence::NotApplicable,
            Some(Value::Object(o)) if o.contains_key("possible") => {
                Confidence::Possible(o.get("possible").and_then(Value::as_str).unwrap_or_default().to_string())
            }
            other => return Err(format!("packet: migration '{id}' has an unrecognized confidence value: {other:?}")),
        };
        // The packet's own `required` flag is authoritative for this specific packet even if the registry's
        // record has since changed; a vanished registry entry becomes visible drift, not a silent drop.
        let reg = registry.iter().find(|r| r.id == id).cloned().unwrap_or(Migration {
            id: id.clone(),
            affected: String::new(),
            detect: Vec::new(),
            required,
            repair: String::new(),
            verify: String::new(),
            limitations: "this migration id is no longer in docs/upgrade-migrations.json: the registry changed since this packet was planned; run `game upgrade plan` again".to_string(),
        });
        migrations.push((Migration { required, ..reg }, confidence));
    }
    Ok((target_sha, PathBuf::from(resolved_from), baseline_sha, class, migrations))
}

/// Whether the project on disk right now still matches the evidence this packet was planned against. Every
/// migration verdict in the packet was computed by comparing a *specific* baseline against the target; if the
/// project's actual current baseline (or its class — whether it has grown a custom Rust client) has since
/// changed, those verdicts describe a project that no longer exists, and `game upgrade plan` needs to run again.
fn evidence_is_stale(cfg: &GameConfig, packet_baseline_sha: Option<&str>, packet_class: &ProjectClass) -> Option<String> {
    let current = resolve_current_engine(cfg, None);
    if current.sha.as_deref() != packet_baseline_sha {
        return Some(format!(
            "this packet is stale: the project's current baseline engine ({}) no longer matches what it was planned against ({}) — run `game upgrade plan` again",
            current.sha.as_deref().unwrap_or("unresolved"),
            packet_baseline_sha.unwrap_or("unresolved"),
        ));
    }
    let cargo = find_cargo_engine_refs(&cfg.dir);
    let fresh_class = classify(cfg, &cargo);
    if &fresh_class != packet_class {
        return Some(format!(
            "this packet is stale: the project's class has changed since it was planned ({packet_class:?} -> {fresh_class:?}) — run `game upgrade plan` again"
        ));
    }
    None
}

/// The staged verification flow. Only this function builds anything, and only into `opts.cache_root` (or an
/// explicit, identity-checked `--engine-build`); the real project at `game_dir` is read, never written.
pub fn run_verify(game_dir: &Path, packet: &Value, migrations_path: &Path, opts: &VerifyOptions) -> Result<VerifyReport, String> {
    let only = validate_only(opts.only)?;
    let (target_sha, target_checkout, baseline_sha, class, migrations) = packet_pieces(packet, migrations_path)?;
    let cfg = game::load(game_dir).map_err(|e| e.join("\n"))?;
    if let Some(reason) = evidence_is_stale(&cfg, baseline_sha.as_deref(), &class) {
        return Err(reason);
    }
    let mut stages = Vec::new();

    // "baseline" always runs: it is cheap (no build) and names the project's state before anything else happens.
    // It checks with *this* CLI binary, not a build of the recorded baseline engine commit — labelled so here,
    // never implied to be a baseline-engine execution that did not occur.
    let t0 = std::time::Instant::now();
    let baseline_report = game::check(&cfg, false);
    stages.push(StageResult {
        name: "baseline".into(),
        ok: baseline_report.failed() == 0,
        detail: format!(
            "(checked against the project's current files using this CLI's own build, not a build of the recorded baseline engine)\n{}",
            baseline_report.render()
        ),
        ms: t0.elapsed().as_millis(),
    });

    if only == Some("baseline") {
        let identity = identity_for(&target_checkout, &target_sha, "dev", "default").unwrap_or(BuildIdentity {
            sha: target_sha.clone(),
            rustc: String::new(),
            profile: "dev".into(),
            features: "default".into(),
            lockfile_hash: String::new(),
        });
        return Ok(VerifyReport {
            baseline_sha,
            target_sha,
            class,
            build_identity: identity,
            build_reused: false,
            target_built: false,
            migrations,
            stages,
            conflicts: Vec::new(),
            clean_maps: Vec::new(),
        });
    }

    // target-build: a prerequisite for content/full, reused by identity when already cached. `--only target-build`
    // stops here, exactly as requested — it does not continue into content or full.
    let t1 = std::time::Instant::now();
    let (bin_dir, reused) = ensure_target_build(&opts.cache_root, &target_checkout, &target_sha, "dev", "default", opts.engine_build)?;
    let identity = identity_for(&target_checkout, &target_sha, "dev", "default")?;
    stages.push(StageResult {
        name: "target-build".into(),
        ok: true,
        detail: format!(
            "{} ({}) identity: sha={} rustc={} profile={} features={} lockfile_hash={}",
            bin_dir.display(),
            if reused { "reused" } else { "fresh" },
            &identity.sha[..identity.sha.len().min(12)],
            identity.rustc.lines().next().unwrap_or_default(),
            identity.profile,
            identity.features,
            &identity.lockfile_hash[..identity.lockfile_hash.len().min(12)],
        ),
        ms: t1.elapsed().as_millis(),
    });

    if only == Some("target-build") {
        return Ok(VerifyReport {
            baseline_sha,
            target_sha,
            class,
            build_identity: identity,
            build_reused: reused,
            target_built: true,
            migrations,
            stages,
            conflicts: Vec::new(),
            clean_maps: Vec::new(),
        });
    }

    // content: needs the target build above; `--only content` stops here, before `full`.
    let stage_dir = cfg.dir.join("out/upgrade").join(&target_sha[..target_sha.len().min(12)]).join("stage");
    let t2 = std::time::Instant::now();
    let content = stage_and_build(&cfg, &stage_dir, &bin_dir)?;
    stages.push(StageResult {
        name: "content".into(),
        ok: content.conflicts.is_empty(),
        detail: format!("{} clean map(s), {} conflict(s)", content.clean.len(), content.conflicts.len()),
        ms: t2.elapsed().as_millis(),
    });

    if only == Some("content") {
        return Ok(VerifyReport {
            baseline_sha,
            target_sha,
            class,
            build_identity: identity,
            build_reused: reused,
            target_built: true,
            migrations,
            stages,
            conflicts: content.conflicts,
            clean_maps: content.clean,
        });
    }

    // full: `--only full` (or no `--only` at all) needs content's staged output above as its own prerequisite, so
    // it necessarily reruns target-build (free, by identity) and content on the way to it — this is not the
    // selector being ignored, it is what "full" legitimately requires, and every stage that ran is listed above.
    let t3 = std::time::Instant::now();
    let (ok, detail) = run_capture(Command::new(bin_dir.join("red_engine2")).args(["game", "check", "--dir"]).arg(&stage_dir));
    stages.push(StageResult { name: "full".into(), ok, detail, ms: t3.elapsed().as_millis() });

    Ok(VerifyReport {
        baseline_sha,
        target_sha,
        class,
        build_identity: identity,
        build_reused: reused,
        target_built: true,
        migrations,
        stages,
        conflicts: content.conflicts,
        clean_maps: content.clean,
    })
}

fn confidence_label(c: &Confidence) -> &'static str {
    match c {
        Confidence::Applicable => "applicable",
        Confidence::Possible(_) => "possible",
        Confidence::NotApplicable => "not_applicable",
    }
}

pub fn report_json(r: &VerifyReport) -> Value {
    json!({
        "baseline_sha": r.baseline_sha,
        "target_sha": r.target_sha,
        "class": class_json(&r.class),
        "build": {
            "identity": {"sha": r.build_identity.sha, "rustc": r.build_identity.rustc, "profile": r.build_identity.profile,
                         "features": r.build_identity.features, "lockfile_hash": r.build_identity.lockfile_hash},
            "obtained": r.target_built,
            "reused": r.build_reused,
        },
        "migrations": r.migrations.iter().map(|(m, c)| json!({"id": m.id, "required": m.required, "confidence": confidence_label(c)})).collect::<Vec<_>>(),
        "stages": r.stages.iter().map(|s| json!({"name": s.name, "ok": s.ok, "ms": s.ms, "detail": s.detail})).collect::<Vec<_>>(),
        "conflicts": r.conflicts,
        "clean_maps": r.clean_maps,
        // Three different questions, each answered separately — never collapse them into one "ok":
        "scope_ok": r.scope_ok(),                 // did the stages that ran pass (the CLI's exit code)
        "candidate_verified": r.candidate_verified(), // was the target actually built and exercised end to end
        "ready_to_apply": r.ready_to_apply(),         // does this report license `game pin --sha <target_sha>`
        "unresolved_required_migrations": r.unresolved_required_migrations().iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
        "unverified_custom_client": r.unverified_custom_client(),
    })
}

pub fn render_report(r: &VerifyReport) -> String {
    let mut s = String::new();
    s.push_str(&format!("upgrade verify: {} -> {}\n", r.baseline_sha.as_deref().unwrap_or("unresolved"), &r.target_sha[..r.target_sha.len().min(12)]));
    if r.target_built {
        s.push_str(&format!("target build: {} ({})\n\n", r.build_identity.sha, if r.build_reused { "reused" } else { "fresh" }));
    } else {
        s.push_str(&format!("target build: not obtained ({} stage only)\n\n", r.stages.last().map(|s| s.name.as_str()).unwrap_or("partial")));
    }
    for st in &r.stages {
        s.push_str(&format!("[{}] {} ({} ms)\n", if st.ok { "ok" } else { "FAIL" }, st.name, st.ms));
        if !st.ok {
            for line in st.detail.lines().take(20) {
                s.push_str(&format!("    {line}\n"));
            }
        }
    }
    if !r.conflicts.is_empty() {
        s.push_str("\nconflicts (never written over the real files; staged candidates kept under out/upgrade/.../stage for review):\n");
        for c in &r.conflicts {
            s.push_str(&format!("  {c}\n"));
        }
    }
    if r.ready_to_apply() {
        s.push_str(&format!(
            "\nready to apply: `red_engine2 game pin --engine <target checkout> --sha {}`, then copy in the clean regenerated map(s) by hand\n",
            r.target_sha
        ));
    } else if !r.scope_ok() {
        if let Some(f) = r.stages.iter().find(|s| !s.ok) {
            s.push_str(&format!("\nnot ready to apply — next: fix `{}` (see detail above), then `game upgrade verify <packet> --only {}`\n", f.name, f.name));
        }
    } else if !r.candidate_verified() {
        let ran = |name: &str| r.stages.iter().any(|s| s.name == name);
        let missing: Vec<&str> = [("target-build", r.target_built), ("content", ran("content")), ("full", ran("full"))]
            .into_iter()
            .filter(|(_, done)| !done)
            .map(|(name, _)| name)
            .collect();
        s.push_str(&format!(
            "\nnot ready to apply: this was a partial check — {} did not run; `game upgrade verify <packet>` with no `--only` runs the full pipeline\n",
            missing.join(", ")
        ));
    } else {
        s.push_str("\nnot ready to apply:\n");
        for m in r.unresolved_required_migrations() {
            let how = if m.repair.is_empty() { "see docs/upgrade-migrations.json".to_string() } else { m.repair.clone() };
            s.push_str(&format!("  - required migration unresolved: {} — {how}\n", m.id));
        }
        if r.unverified_custom_client() {
            s.push_str(
                "  - this project has a custom Rust client; no stage here builds or runs it against the target engine \
                 (a clean map regeneration is not proof it compiles) — verify it by hand before pinning\n",
            );
        }
    }
    s
}

pub fn write_report(r: &VerifyReport, out_json: &Path, out_md: &Path) -> Result<(), String> {
    create_parent(out_json)?;
    create_parent(out_md)?;
    std::fs::write(out_json, serde_json::to_string_pretty(&report_json(r)).map_err(|e| e.to_string())? + "\n")
        .map_err(|e| format!("{}: {e}", out_json.display()))?;
    std::fs::write(out_md, render_report(r)).map_err(|e| format!("{}: {e}", out_md.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(cwd: &Path, args: &[&str]) {
        let mut cmd = Command::new("git");
        cmd.current_dir(cwd).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args);
        assert!(cmd.output().unwrap().status.success(), "git {args:?}");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("re2_upgrade_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_target_pins_a_branch_to_one_fixed_commit_and_refuses_dirty() {
        let dir = scratch("resolve");
        git(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "1").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "one"]);
        let first = resolve_target(&dir, "main").unwrap();
        assert_eq!(first.len(), 40);

        std::fs::write(dir.join("a.txt"), "2").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "two"]);
        let second = resolve_target(&dir, "main").unwrap();
        assert_ne!(first, second, "the branch moved: a fresh resolve sees its new tip");
        assert_eq!(resolve_target(&dir, &first).unwrap(), first, "a commit, once resolved, never changes under it");

        std::fs::write(dir.join("a.txt"), "3").unwrap();
        let e = resolve_target(&dir, "main").unwrap_err();
        assert!(e.contains("uncommitted"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn classify_reads_evidence_not_assumption() {
        let dir = scratch("classify");
        let cfg = GameConfig {
            dir: dir.clone(),
            name: "t".into(),
            id: None,
            engine: game::EngineRef { path: Some("../engine".into()), ..Default::default() },
            blueprints: vec!["blueprints/main.blueprint.json".into()],
            maps: vec!["maps/main.json".into()],
            server: game::ServerCfg { map: "maps/main.json".into(), port: 28000, spawn_group: String::new(), args: Vec::new() },
            capabilities: None,
        };
        assert_eq!(classify(&cfg, &[]), ProjectClass::StandardBlueprint);
        let cargo_ref = CargoEngineRef { manifest: dir.join("client/Cargo.toml"), kind: "path", location: "../engine".into(), sha: None };
        assert!(matches!(classify(&cfg, &[cargo_ref]), ProjectClass::CustomRustClient { .. }));
        let empty = GameConfig { blueprints: Vec::new(), maps: Vec::new(), ..cfg };
        assert!(matches!(classify(&empty, &[]), ProjectClass::Unknown { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrations_load_from_the_real_sidecar_and_match_a_real_protocol_bump() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let migrations = load_migrations(&root.join("docs/upgrade-migrations.json")).unwrap();
        assert!(migrations.iter().any(|m| m.id == "protocol-version-lockstep" && m.required));

        // Two real commits where PROTOCOL_VERSION is known to differ (v9 -> v11, per git history).
        let baseline = "7918af0";
        let target = "af0f0e0";
        if git_show(root, baseline, "src/net/protocol.rs").is_ok() && git_show(root, target, "src/net/protocol.rs").is_ok() {
            let c = protocol_version_confidence(
                &Evidence {
                    game: GameConfig {
                        dir: root.to_path_buf(),
                        name: "t".into(),
                        id: None,
                        engine: game::EngineRef::default(),
                        blueprints: vec![],
                        maps: vec!["maps/main.json".into()],
                        server: game::ServerCfg { map: "maps/main.json".into(), port: 1, spawn_group: String::new(), args: vec![] },
                        capabilities: None,
                    },
                    current: ResolvedEngine { kind: "path", location: ".".into(), sha: Some(baseline.into()), dirty: false },
                    cargo: vec![],
                    class: ProjectClass::StandardBlueprint,
                    drift: vec![],
                },
                root,
                Some(baseline),
                target,
            );
            assert_eq!(c, Confidence::Applicable, "two commits with real, different PROTOCOL_VERSION constants must match this migration");
        }
    }

    #[test]
    fn an_unmatched_migration_id_stays_possible_not_silently_certified() {
        let m = Migration {
            id: "future-thing".into(),
            affected: "x".into(),
            detect: vec![],
            required: true,
            repair: "r".into(),
            verify: "v".into(),
            limitations: "l".into(),
        };
        let ev = Evidence {
            game: GameConfig {
                dir: PathBuf::from("."),
                name: "t".into(),
                id: None,
                engine: game::EngineRef::default(),
                blueprints: vec![],
                maps: vec![],
                server: game::ServerCfg { map: String::new(), port: 1, spawn_group: String::new(), args: vec![] },
                capabilities: None,
            },
            current: ResolvedEngine { kind: "path", location: ".".into(), sha: None, dirty: false },
            cargo: vec![],
            class: ProjectClass::StandardBlueprint,
            drift: vec![],
        };
        let got = applicable(&[m], &ev, Path::new("."), None, "deadbeef");
        assert!(matches!(got[0].1, Confidence::Possible(_)));
    }

    #[test]
    fn an_identity_mismatch_is_rejected_not_reused() {
        let dir = scratch("identity");
        let claimed = BuildIdentity { sha: "aaa".into(), rustc: "x".into(), profile: "dev".into(), features: "default".into(), lockfile_hash: "h1".into() };
        write_identity(&dir, &claimed).unwrap();
        assert_eq!(read_identity(&dir), Some(claimed));
        let wanted = BuildIdentity { sha: "bbb".into(), ..read_identity(&dir).unwrap() };
        assert_ne!(read_identity(&dir).unwrap(), wanted, "a different sha must not compare equal: ensure_target_build would refuse to reuse this directory");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_explicit_engine_build_with_a_matching_identity_but_no_binary_is_refused() {
        // A matching IDENTITY.json is not, by itself, proof a usable binary exists there (task step 6: "verify
        // the expected artifacts exist before accepting a receipt"). No real build needed to prove this: an
        // identity file with nothing beside it is exactly the shape a half-cleaned-up or corrupted cache entry
        // would have.
        let dir = scratch("missing_artifact");
        let wanted = identity_for(Path::new("."), "deadbeef", "dev", "default").unwrap();
        let claimed = BuildIdentity { sha: "deadbeef".into(), ..wanted.clone() };
        write_identity(&dir, &claimed).unwrap();
        assert!(!dir.join("target/debug").join(bin_name()).exists(), "the fixture must not accidentally have a real binary");
        let err = ensure_target_build(Path::new("/unused"), Path::new("."), "deadbeef", "dev", "default", Some(&dir)).unwrap_err();
        assert!(err.contains("missing") && err.contains(&dir.display().to_string()), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_build_lock_serializes_concurrent_callers_instead_of_racing_them() {
        let dir = scratch("lock");
        std::fs::create_dir_all(&dir).unwrap();
        let first = acquire_build_lock(&dir, std::time::Duration::from_secs(5), std::time::Duration::from_millis(10)).unwrap();
        assert!(first.is_some(), "the first caller must win the lock");

        // A concurrent caller must not also proceed, and must not delete anything out from under the first: it
        // waits, and — since the first caller never releases it within this short timeout — gives up loudly.
        let timed_out = acquire_build_lock(&dir, std::time::Duration::from_millis(100), std::time::Duration::from_millis(10)).unwrap_err();
        assert!(timed_out.contains("building"), "{timed_out}");

        // Once the first caller's guard drops (its build finished, one way or another), the lock clears and a
        // new caller may proceed.
        drop(first);
        let second = acquire_build_lock(&dir, std::time::Duration::from_secs(5), std::time::Duration::from_millis(10)).unwrap();
        assert!(second.is_some(), "the lock must be free once the holder released it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_caller_proceeds_once_the_first_releases_the_lock_from_another_thread() {
        let dir = scratch("lock_threaded");
        std::fs::create_dir_all(&dir).unwrap();
        let dir2 = dir.clone();
        let holder = std::thread::spawn(move || {
            let guard = acquire_build_lock(&dir2, std::time::Duration::from_secs(5), std::time::Duration::from_millis(10)).unwrap();
            assert!(guard.is_some());
            std::thread::sleep(std::time::Duration::from_millis(200));
            // guard drops here, releasing the lock
        });
        std::thread::sleep(std::time::Duration::from_millis(20)); // let the holder claim the lock first
        let waited = acquire_build_lock(&dir, std::time::Duration::from_secs(5), std::time::Duration::from_millis(10)).unwrap();
        assert!(waited.is_none(), "this caller waited for the holder to finish, rather than racing it or timing out");
        holder.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn packet_round_trips_through_json_for_verify() {
        let ev = Evidence {
            game: GameConfig {
                dir: PathBuf::from("."),
                name: "t".into(),
                id: None,
                engine: game::EngineRef { path: Some("../engine".into()), ..Default::default() },
                blueprints: vec!["blueprints/main.blueprint.json".into()],
                maps: vec!["maps/main.json".into()],
                server: game::ServerCfg { map: "maps/main.json".into(), port: 1, spawn_group: String::new(), args: vec![] },
                capabilities: None,
            },
            current: ResolvedEngine { kind: "path", location: "../engine".into(), sha: Some("a".repeat(40)), dirty: false },
            cargo: vec![],
            class: ProjectClass::StandardBlueprint,
            drift: vec![],
        };
        let p = plan(&ev, "b".repeat(40), Path::new("../engine"), &["fix the thing".to_string()], &[]);
        let doc = packet_json(&p);
        let (target_sha, resolved_from, baseline_sha, class, _) = packet_pieces(&doc, Path::new("docs/upgrade-migrations.json")).unwrap();
        assert_eq!(target_sha, "b".repeat(40));
        assert_eq!(resolved_from, Path::new("../engine"));
        assert_eq!(baseline_sha.as_deref(), Some("a".repeat(40).as_str()));
        assert_eq!(class, ProjectClass::StandardBlueprint);
        assert!(render_packet(&p).contains("fix the thing"));
    }

    /// Builds this real engine checkout's own HEAD once into the cache, then confirms a second call reuses it
    /// (no second build) and that handing its directory back as `--engine-build` also reuses it. Real compilation
    /// is too slow for the default suite (`cargo test`): run explicitly with `cargo test --lib -- --ignored
    /// upgrade::tests::a_real_build_is_cached_and_reused_by_identity`. Skips (does not fail) if this checkout is
    /// currently dirty, since a dirty HEAD cannot be reproducibly resolved — that refusal is covered separately by
    /// `resolve_target_pins_a_branch_to_one_fixed_commit_and_refuses_dirty`.
    #[test]
    #[ignore]
    fn a_real_build_is_cached_and_reused_by_identity() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        if probe_dirty(root) {
            eprintln!("skipping: {} is dirty, HEAD is not reproducibly resolvable here", root.display());
            return;
        }
        let sha = probe_sha(root).expect("a real checkout has a HEAD");
        let cache = scratch("real_build_cache");
        let (first_dir, first_reused) = ensure_target_build(&cache, root, &sha, "dev", "default", None).unwrap();
        assert!(!first_reused, "the first call for this identity must actually build");
        assert!(first_dir.join("red_engine2").exists() || first_dir.join("red_engine2.exe").exists(), "{}", first_dir.display());

        let (second_dir, second_reused) = ensure_target_build(&cache, root, &sha, "dev", "default", None).unwrap();
        assert!(second_reused, "the same identity must be served from cache, not rebuilt");
        assert_eq!(first_dir, second_dir);

        let explicit = first_dir.parent().unwrap(); // the cache entry's own root, holding IDENTITY.json
        let (explicit_dir, explicit_reused) = ensure_target_build(&cache, root, &sha, "dev", "default", Some(explicit)).unwrap();
        assert!(explicit_reused);
        assert_eq!(explicit_dir, first_dir);

        let _ = std::fs::remove_dir_all(&cache);
    }

    fn sample_identity() -> BuildIdentity {
        BuildIdentity { sha: "a".repeat(40), rustc: "rustc 1.0".into(), profile: "dev".into(), features: "default".into(), lockfile_hash: "h".into() }
    }

    fn sample_report(stages: Vec<StageResult>, target_built: bool, class: ProjectClass, migrations: Vec<(Migration, Confidence)>) -> VerifyReport {
        VerifyReport {
            baseline_sha: Some("b".repeat(40)),
            target_sha: "a".repeat(40),
            class,
            build_identity: sample_identity(),
            build_reused: false,
            target_built,
            migrations,
            stages,
            conflicts: Vec::new(),
            clean_maps: vec!["maps/main.json".into()],
        }
    }

    fn ok_stage(name: &str) -> StageResult {
        StageResult { name: name.into(), ok: true, detail: String::new(), ms: 0 }
    }

    #[test]
    fn a_baseline_only_pass_is_never_ready_to_apply_or_candidate_verified() {
        let r = sample_report(vec![ok_stage("baseline")], false, ProjectClass::StandardBlueprint, vec![]);
        assert!(r.scope_ok(), "the one stage that ran (baseline) passed");
        assert!(!r.candidate_verified(), "the target was never built");
        assert!(!r.ready_to_apply());
        assert!(!render_report(&r).contains("game pin"), "{}", render_report(&r));
        assert!(report_json(&r)["ready_to_apply"] == json!(false));
    }

    #[test]
    fn only_target_build_or_only_content_are_not_candidate_verified_either() {
        let r = sample_report(vec![ok_stage("baseline"), ok_stage("target-build")], true, ProjectClass::StandardBlueprint, vec![]);
        assert!(!r.candidate_verified(), "content and full never ran");
        let r = sample_report(vec![ok_stage("baseline"), ok_stage("target-build"), ok_stage("content")], true, ProjectClass::StandardBlueprint, vec![]);
        assert!(!r.candidate_verified(), "full never ran");
    }

    #[test]
    fn the_full_pipeline_passing_is_candidate_verified_and_ready_with_no_obligations() {
        let r = sample_report(
            vec![ok_stage("baseline"), ok_stage("target-build"), ok_stage("content"), ok_stage("full")],
            true,
            ProjectClass::StandardBlueprint,
            vec![],
        );
        assert!(r.candidate_verified());
        assert!(r.ready_to_apply());
        assert!(render_report(&r).contains("ready to apply") && render_report(&r).contains(&r.target_sha));
    }

    #[test]
    fn an_unresolved_required_migration_blocks_readiness_even_when_every_stage_passed() {
        let m = Migration {
            id: "protocol-version-lockstep".into(),
            affected: "wire".into(),
            detect: vec![],
            required: true,
            repair: "rebuild client and server from the target".into(),
            verify: String::new(),
            limitations: String::new(),
        };
        let fully_passed = vec![ok_stage("baseline"), ok_stage("target-build"), ok_stage("content"), ok_stage("full")];
        let applicable = sample_report(fully_passed.clone(), true, ProjectClass::StandardBlueprint, vec![(m.clone(), Confidence::Applicable)]);
        assert!(applicable.candidate_verified(), "every stage still passed");
        assert_eq!(applicable.unresolved_required_migrations().len(), 1);
        assert!(!applicable.ready_to_apply());
        assert!(render_report(&applicable).contains("protocol-version-lockstep"));

        let possible =
            sample_report(fully_passed.clone(), true, ProjectClass::StandardBlueprint, vec![(m.clone(), Confidence::Possible("unreachable commit".into()))]);
        assert!(!possible.ready_to_apply(), "an uncertain required migration is not a resolved one");

        let not_applicable = sample_report(fully_passed, true, ProjectClass::StandardBlueprint, vec![(m, Confidence::NotApplicable)]);
        assert!(not_applicable.ready_to_apply(), "a migration this project's evidence rules out does not block readiness");
    }

    #[test]
    fn a_custom_rust_client_is_never_ready_to_apply_from_map_regeneration_alone() {
        let fully_passed = vec![ok_stage("baseline"), ok_stage("target-build"), ok_stage("content"), ok_stage("full")];
        let r = sample_report(fully_passed, true, ProjectClass::CustomRustClient { crates: vec![PathBuf::from("client/Cargo.toml")] }, vec![]);
        assert!(r.candidate_verified(), "the blueprint-built maps did verify cleanly");
        assert!(r.unverified_custom_client());
        assert!(!r.ready_to_apply(), "a clean map regeneration is not proof the client crate still compiles");
        assert!(render_report(&r).contains("custom Rust client"));
    }

    #[test]
    fn unknown_only_values_are_rejected_by_name_not_run_as_everything() {
        for good in STAGE_NAMES {
            assert_eq!(validate_only(Some(good)).unwrap(), Some(*good));
        }
        assert_eq!(validate_only(None).unwrap(), None);
        let e = validate_only(Some("basline")).unwrap_err(); // a plausible typo
        assert!(e.contains("basline") && STAGE_NAMES.iter().all(|s| e.contains(s)), "{e}");
    }

    #[test]
    fn a_malformed_packet_fails_loudly_instead_of_silently_certifying_nothing() {
        let good = json!({
            "target": {"sha": "a".repeat(40), "resolved_from": "../engine"},
            "baseline": {"sha": "b".repeat(40)},
            "class": {"kind": "standard_blueprint"},
            "migrations": [{"id": "protocol-version-lockstep", "required": true, "confidence": "applicable"}],
        });
        let (_, _, _, class, migrations) = packet_pieces(&good, Path::new(MIGRATIONS_PATH)).unwrap();
        assert_eq!(class, ProjectClass::StandardBlueprint);
        assert_eq!(migrations.len(), 1);
        assert!(migrations[0].0.required);

        // Missing the migrations array entirely: must not silently become "no migrations".
        let mut no_migrations = good.clone();
        no_migrations.as_object_mut().unwrap().remove("migrations");
        let e = packet_pieces(&no_migrations, Path::new(MIGRATIONS_PATH)).unwrap_err();
        assert!(e.contains("migrations"), "{e}");

        // An unreadable registry must not silently become "no migrations" either.
        let e = packet_pieces(&good, Path::new("/nonexistent/upgrade-migrations.json")).unwrap_err();
        assert!(e.contains("registry"), "{e}");

        // A migration entry with a confidence shape this code does not recognize must error, not quietly become
        // `NotApplicable` (the single most dangerous silent default for a *required* migration).
        let mut weird_confidence = good.clone();
        weird_confidence["migrations"][0]["confidence"] = json!(42);
        let e = packet_pieces(&weird_confidence, Path::new(MIGRATIONS_PATH)).unwrap_err();
        assert!(e.contains("protocol-version-lockstep"), "{e}");

        // A migration id the current registry no longer has: kept (with the packet's own `required`), flagged as
        // drift, never dropped.
        let mut renamed = good.clone();
        renamed["migrations"][0]["id"] = json!("some-id-the-registry-no-longer-has");
        let (_, _, _, _, migrations) = packet_pieces(&renamed, Path::new(MIGRATIONS_PATH)).unwrap();
        assert_eq!(migrations.len(), 1, "the entry must survive, not vanish");
        assert!(migrations[0].0.required, "the packet's own required flag is kept");
        assert!(migrations[0].0.limitations.contains("no longer"), "{:?}", migrations[0].0.limitations);
    }

    #[test]
    fn stale_baseline_or_class_evidence_is_rejected_before_any_stage_runs() {
        let (engine, old, new) = fake_engine_repo_for_evidence("staleness");
        let dir = scratch("stale_evidence_game");
        // `json!` escapes the engine path properly (a Windows path's backslashes are not valid JSON escapes if
        // hand-formatted into a string literal directly).
        let game_json = json!({
            "game": 1, "name": "t",
            "engine": {"path": engine.display().to_string()},
            "blueprints": [], "maps": ["maps/main.json"],
            "server": {"map": "maps/main.json", "port": 1},
        });
        std::fs::write(dir.join("game.json"), game_json.to_string()).unwrap();
        std::fs::create_dir_all(dir.join("maps")).unwrap();
        std::fs::write(dir.join("maps/main.json"), "{}").unwrap();
        let cfg = game::load(&dir).unwrap();

        // The packet claims the project's baseline was `old`; the project's engine.path actually resolves `new`
        // right now (as if main advanced after planning) — this must be refused, not silently verified.
        assert!(evidence_is_stale(&cfg, Some(&old), &ProjectClass::StandardBlueprint).is_some());
        assert!(evidence_is_stale(&cfg, Some(&new), &ProjectClass::StandardBlueprint).is_none(), "matching evidence must pass");
        assert!(evidence_is_stale(&cfg, Some(&new), &ProjectClass::CustomRustClient { crates: vec![] }).is_some(), "a changed class is also stale evidence");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&engine);
    }

    /// A tiny real git checkout standing in for "the engine", with two commits — just enough for
    /// `resolve_current_engine`'s `probe_sha` to read a real HEAD, no buildable crate needed.
    fn fake_engine_repo_for_evidence(name: &str) -> (PathBuf, String, String) {
        let dir = scratch(&format!("evidence_engine_{name}"));
        git(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("MARKER"), name).unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "old"]);
        let old = probe_sha(&dir).unwrap();
        std::fs::write(dir.join("MARKER"), format!("{name}-2")).unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "new"]);
        let new = probe_sha(&dir).unwrap();
        (dir, old, new)
    }
}
