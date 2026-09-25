//! `red_engine2 sim` and `red_engine2 replay`: the command-line side of headless scenarios and match replays
//! (the engine is in [`crate::sim::scenario`], [`crate::sim::trace`] and [`crate::sim::replay`]).
//!
//! `sim scene.json` runs every scenario in the scene's `checks.sim` (or `--scenario file.json`) against the real
//! authoritative simulation, prints PASS/FAIL with evidence, and can record a trace. `replay trace.json` re-runs a
//! recording with no renderer or socket and reports the first tick where it stops agreeing.

use crate::net::map_hash;
use crate::schema::{Object, ObjectKind, Scene};
use crate::sim::replay::{self, ReplayReport};
use crate::sim::scenario::{self, RecordOptions, Scenario, ScenarioResult};
use crate::sim::spawns::{parse_spawns, Spawn};
use crate::sim::trace::Trace;
use serde_json::{json, Value};
use std::path::Path;

/// A scene loaded for simulation: parsed scene, spawn points, raw JSON and its text hash.
pub struct Loaded {
    /// The parsed scene (rules included).
    pub scene: Scene,
    /// The scene's spawn points.
    pub spawns: Vec<Spawn>,
    /// The raw JSON (for `checks.sim`).
    pub raw: Value,
    /// Hash of the file text, for traces.
    pub hash: u32,
}

/// Loads and validates `path`; the error is the usual `path: message` list.
pub fn load(path: &Path) -> Result<Loaded, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let scene = crate::schema::parse_scene(&text).map_err(|errs| format!("{}: scene is invalid:\n  error: {}", path.display(), errs.join("\n  error: ")))?;
    let spawns = parse_spawns(&text).map_err(|e| format!("{}: spawns: {e}", path.display()))?;
    let raw = serde_json::from_str(&text).map_err(|e| format!("{}: json: {e}", path.display()))?;
    Ok(Loaded { scene, spawns, raw, hash: map_hash(&text) })
}

fn all_ids(objects: &[Object], out: &mut Vec<String>) {
    for o in objects {
        out.push(o.id.clone());
        if let ObjectKind::Group(kids) = &o.kind {
            all_ids(kids, out);
        }
    }
}

/// Parses every scenario of `values` against the scene, collecting all problems.
pub fn parse_scenarios(loaded: &Loaded, values: &[Value], source: &str) -> Result<Vec<Scenario>, Vec<String>> {
    let mut ids = Vec::new();
    all_ids(&loaded.scene.objects, &mut ids);
    let (mut out, mut errs) = (Vec::new(), Vec::new());
    for (i, v) in values.iter().enumerate() {
        match scenario::parse(v, &loaded.scene.rules, &ids) {
            Ok(s) => out.push(s),
            Err(e) => errs.extend(e.into_iter().map(|m| format!("{source}[{i}]: {m}"))),
        }
    }
    if errs.is_empty() {
        Ok(out)
    } else {
        Err(errs)
    }
}

/// The scenarios a scene declares in `checks.sim`.
pub fn scene_scenarios(loaded: &Loaded) -> Result<Vec<Scenario>, Vec<String>> {
    let values: Vec<Value> = loaded.raw.pointer("/checks/sim").and_then(Value::as_array).cloned().unwrap_or_default();
    parse_scenarios(loaded, &values, "checks.sim")
}

/// Outcome of the `sim` command.
pub struct SimReport {
    /// One result per scenario run.
    pub results: Vec<ScenarioResult>,
}

impl SimReport {
    /// True when every scenario passed.
    pub fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }

    /// The report as JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "passed": self.results.iter().filter(|r| r.passed).count(),
            "failed": self.results.iter().filter(|r| !r.passed).count(),
            "scenarios": self.results.iter().map(ScenarioResult::to_json).collect::<Vec<_>>(),
        })
    }

    /// The report as text.
    pub fn render(&self) -> String {
        let mut out: String = self.results.iter().map(ScenarioResult::render).collect();
        out.push_str(&format!(
            "{} scenario(s): {} passed, {} failed\n",
            self.results.len(),
            self.results.iter().filter(|r| r.passed).count(),
            self.results.iter().filter(|r| !r.passed).count()
        ));
        out
    }
}

/// Runs scenarios: those in `scenario_file` if given, else the scene's `checks.sim`; `only` filters by name substring.
/// When `record` is given, the first scenario's trace is returned alongside the report.
pub fn run(path: &Path, scenario_file: Option<&Path>, only: Option<&str>, record: Option<RecordOptions>) -> Result<(SimReport, Option<Trace>), String> {
    let loaded = load(path)?;
    let scenarios = match scenario_file {
        Some(f) => {
            let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: json: {e}", f.display()))?;
            let list = match v {
                Value::Array(a) => a,
                one => vec![one],
            };
            parse_scenarios(&loaded, &list, &f.display().to_string())
        }
        None => scene_scenarios(&loaded),
    }
    .map_err(|errs| errs.join("\n"))?;
    let scenarios: Vec<Scenario> = scenarios.into_iter().filter(|s| only.is_none_or(|o| s.name.contains(o))).collect();
    if scenarios.is_empty() {
        return Err(match scenario_file {
            Some(_) => "no scenario to run (empty file, or `--only` matched nothing)".to_string(),
            None => "no scenarios: add `checks.sim` to the scene (see `red_engine2 describe sim`) or pass --scenario file.json".to_string(),
        });
    }
    let mut results = Vec::new();
    let mut trace = None;
    for (i, s) in scenarios.iter().enumerate() {
        let mut r = scenario::run(s, &loaded.scene, &loaded.spawns, if i == 0 { record.map(|o| RecordOptions { map_hash: loaded.hash, ..o }) } else { None })?;
        if let Some(mut t) = r.trace.take() {
            t.header.scene = path.display().to_string();
            trace = Some(t);
        }
        results.push(r);
    }
    Ok((SimReport { results }, trace))
}

/// The scenario checks `verify` runs: one `(name, ok, detail)` per scenario in the scene's `checks.sim`.
pub fn verify_checks(path: &Path, only: Option<&str>) -> Result<Vec<(String, bool, String)>, String> {
    let loaded = load(path)?;
    let scenarios = scene_scenarios(&loaded).map_err(|e| e.join("\n"))?;
    let mut out = Vec::new();
    for s in scenarios.iter().filter(|s| only.is_none_or(|o| s.name.contains(o))) {
        let r = scenario::run(s, &loaded.scene, &loaded.spawns, None)?;
        let detail = if r.passed {
            format!("{} check(s) held in {:.1} s of play", r.outcomes.len(), r.ticks as f64 / 60.0)
        } else {
            r.outcomes.iter().filter(|o| !o.ok).map(|o| format!("{}: {}", o.label, o.detail)).collect::<Vec<_>>().join("\n     ")
        };
        out.push((format!("sim `{}`", s.name), r.passed, detail));
    }
    Ok(out)
}

/// The report of a `replay`, ready to print.
pub struct ReplayOutcome {
    /// What the replay found.
    pub report: ReplayReport,
    /// The scene file used.
    pub scene: String,
    /// Set when the trace was recorded against a different version of the map file.
    pub map_note: Option<String>,
}

impl ReplayOutcome {
    /// True when the replay confirms the recording (a different-platform, last-bit-only mismatch still counts).
    pub fn ok(&self) -> bool {
        self.report.game_agrees() && (self.report.exact.is_none() || self.report.note.is_some())
    }

    /// The outcome as JSON.
    pub fn to_json(&self) -> Value {
        let div = |d: &Option<replay::Divergence>| match d {
            Some(d) => json!({"tick": d.tick, "components": d.components, "detail": d.detail}),
            None => Value::Null,
        };
        json!({
            "ok": self.ok(), "scene": self.scene, "ticks": self.report.ticks, "checkpoints_compared": self.report.compared,
            "events_match": self.report.events_match, "first_exact_divergence": div(&self.report.exact),
            "first_coarse_divergence": div(&self.report.coarse), "note": self.report.note, "map_note": self.map_note,
        })
    }

    /// The outcome as text.
    pub fn render(&self) -> String {
        let r = &self.report;
        let mut out =
            format!("{} replay of {}: {} ticks, {} checkpoints compared\n", if self.ok() { "OK" } else { "DIVERGED" }, self.scene, r.ticks, r.compared);
        if let Some(n) = &self.map_note {
            out.push_str(&format!("  note: {n}\n"));
        }
        if let Some(n) = &r.note {
            out.push_str(&format!("  note: {n}\n"));
        }
        match (&r.exact, &r.coarse) {
            (None, None) => out.push_str("  every checksum matched"),
            (Some(e), None) => out.push_str(&format!(
                "  bit-exact checksums differ from tick {} but the game agrees (float noise, not a desync)\n  {}",
                e.tick,
                replay::describe(e).replace('\n', "\n  ")
            )),
            (_, Some(c)) => out.push_str(&format!("  {}", replay::describe(c).replace('\n', "\n  "))),
        }
        if !r.events_match {
            out.push_str("\n  the game events differ from the recording");
        }
        out.push('\n');
        out
    }
}

/// Replays a trace file. `scene` overrides the map the trace names.
pub fn replay_file(trace_path: &Path, scene: Option<&Path>) -> Result<ReplayOutcome, String> {
    let text = std::fs::read_to_string(trace_path).map_err(|e| format!("{}: {e}", trace_path.display()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: json: {e}", trace_path.display()))?;
    let trace = Trace::from_json(&v).map_err(|e| format!("{}: {e}", trace_path.display()))?;
    let scene_path = match scene {
        Some(s) => s.to_path_buf(),
        None if !trace.header.scene.is_empty() => trace.header.scene.clone().into(),
        None => return Err("the trace does not name its map: pass --scene <map.json>".to_string()),
    };
    let loaded = load(&scene_path)?;
    let map_note = (trace.header.map_hash != 0 && trace.header.map_hash != loaded.hash).then(|| {
        format!(
            "the trace was recorded against a different version of {} (hash {:08x}, now {:08x}): a divergence may just be the map changing",
            scene_path.display(),
            trace.header.map_hash,
            loaded.hash
        )
    });
    let report = replay::replay(&trace, &loaded.scene, &loaded.spawns)?;
    Ok(ReplayOutcome { report, scene: scene_path.display().to_string(), map_note })
}

/// Compares two trace files of the same match and describes the first divergence (or says they agree).
pub fn compare_files(a: &Path, b: &Path) -> Result<(bool, String), String> {
    let read = |p: &Path| -> Result<Trace, String> {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        Trace::from_json(&serde_json::from_str(&text).map_err(|e| format!("{}: json: {e}", p.display()))?).map_err(|e| format!("{}: {e}", p.display()))
    };
    let (ta, tb) = (read(a)?, read(b)?);
    Ok(match replay::compare_traces(&ta, &tb) {
        None => (true, format!("the two traces agree at every shared checkpoint ({} vs {})", ta.checkpoints.len(), tb.checkpoints.len())),
        Some(d) => (false, replay::describe(&d)),
    })
}
