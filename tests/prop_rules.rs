//! Prop-aware rules and scenario expectations (ADR 2026-09-29-prop-aware-rules-and-scenarios), proven on the committed fixture
//! `tests/fixtures/prop_rules.json`: a crate launched off a 2 m deck into a `pit` zone scores through `prop_enter`; one bat swing
//! topples a three-domino line and `tilt(d3) > 60` sees it; `reset` stands the line up for a second, identical swing; `place` moves a
//! crate; `held(id)` sees a pickup. Each positive scenario has its negative twin (no impulse, no swing), a trace of the busiest one
//! replays clean (prop occupancy is part of the rule checksum), and the real offline client runs the same rules headless.

use red_engine2::sim::replay::{describe, replay};
use red_engine2::sim::scenario::RecordOptions;
use red_engine2::tools::simrun;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture() -> PathBuf {
    root().join("tests/fixtures/prop_rules.json")
}

/// A fresh scratch directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("re2_prop_rules_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The fixture with `edit` applied, written to `dir` (the physics does not care where the file is).
fn variant(dir: &Path, name: &str, edit: impl FnOnce(&mut Value)) -> PathBuf {
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    edit(&mut v);
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    path
}

fn scenarios_of(v: &mut Value) -> &mut Vec<Value> {
    v["checks"]["sim"].as_array_mut().unwrap()
}

#[test]
fn every_scenario_of_the_fixture_passes_and_the_report_lists_the_props() {
    let (report, _) = simrun::run(&fixture(), None, None, None).expect("run");
    let text = report.render();
    assert!(report.all_passed(), "{text}");
    assert_eq!(report.results.len(), 6, "{text}");
    let crate_run = &report.results[0];
    let crate_final = crate_run.props.iter().find(|p| p.id == "crate").expect("every loose prop is reported");
    assert!(crate_final.moved > 2.0 && crate_final.pos.x > 1.5 && crate_final.asleep, "the crate rests in the pit: {crate_final:?}");
    assert!(crate_run.props.iter().any(|p| p.id == "d1" && p.moved < 0.01), "the dominoes were not touched in the crate run");
    let json = crate_run.to_json();
    assert_eq!(json["props"].as_array().unwrap().len(), 5, "crate, crate_b, d1, d2, d3");
    assert!(text.contains("prop crate at") && text.contains("carried by p1"), "{text}");
}

#[test]
fn the_score_needs_the_impulse_that_puts_the_crate_in_the_pit() {
    let dir = scratch("no_impulse");
    let path = variant(&dir, "no_impulse", |v| {
        let kick = v["rules"].as_array_mut().unwrap().iter_mut().find(|r| r["id"] == "kick").unwrap();
        kick["do"] = serde_json::json!([{ "emit": "kicked" }]);
    });
    let (report, _) = simrun::run(&path, None, Some("crate:"), None).expect("run");
    let text = report.render();
    assert!(!report.all_passed(), "without the impulse the crate stays on the deck:\n{text}");
    let failed: Vec<&str> = report.results[0].outcomes.iter().filter(|o| !o.ok).map(|o| o.label.as_str()).collect();
    for want in ["var score == 1", "crate in zone pit", "crate moved", "crate y < 1", "event `in_pit`"] {
        assert!(failed.iter().any(|f| f == &want), "expected `{want}` to fail; failed: {failed:?}\n{text}");
    }
    assert!(text.contains("tilt 0 deg, moved 0.00 m"), "the evidence says where the crate is:\n{text}");
}

#[test]
fn the_dominoes_only_fall_when_the_bat_swings() {
    let dir = scratch("no_swing");
    let path = variant(&dir, "no_swing", |v| {
        let s = scenarios_of(v).iter_mut().find(|s| s["name"].as_str().unwrap().starts_with("dominoes: one bat swing")).unwrap();
        s["script"][0]["hold"]["attack"] = Value::Bool(false);
    });
    let (report, _) = simrun::run(&path, None, Some("dominoes: one bat swing"), None).expect("run");
    let text = report.render();
    assert!(!report.all_passed(), "{text}");
    let failed: Vec<&str> = report.results[0].outcomes.iter().filter(|o| !o.ok).map(|o| o.label.as_str()).collect();
    for want in ["d1 tilt > 60 deg", "d3 tilt > 60 deg", "d3 moved", "event `swing`", "event `line_down`"] {
        assert!(failed.iter().any(|f| f == &want), "expected `{want}` to fail; failed: {failed:?}\n{text}");
    }
}

#[test]
fn a_bad_prop_expectation_or_rule_is_a_validate_error_with_the_fix() {
    let dir = scratch("bad");
    let path = variant(&dir, "bad", |v| {
        scenarios_of(v)[0]["expect"] = serde_json::json!([{ "prop": "crat", "in_zone": "pit" }, { "prop": "crate", "in_zone": "pitt" }, { "prop": "crate" }, { "prop": "crate", "held_by": "p9" }]);
    });
    let err = simrun::run(&path, None, None, None).err().expect("invalid expectations are refused");
    for needle in [
        "no loose prop `crat` — did you mean `crate`?",
        "no zone `pitt` — did you mean `pit`?",
        "a `prop` check needs exactly one of in_zone, not_in_zone, below_y, y_lt, y_gt, tilt_gt, tilt_lt, moved, near, held_by",
        "no player `p9`",
    ] {
        assert!(err.contains(needle), "missing `{needle}` in:\n{err}");
    }
    let path = variant(&dir, "bad_rule", |v| {
        v["rules"][2]["if"] = Value::String("tilt(deck) > 60".to_string());
    });
    let err = simrun::run(&path, None, None, None).err().expect("a rule about a solid object is refused");
    assert!(err.contains("`tilt(deck)`: no loose prop `deck`"), "{err}");
}

#[test]
fn the_reset_scenario_is_deterministic_and_its_trace_replays_clean() {
    let opts = RecordOptions { map_hash: 0, checkpoint_every: 1, dump_every: 60 };
    let (a, trace) = simrun::run(&fixture(), None, Some("reset:"), Some(opts)).expect("run");
    let (b, _) = simrun::run(&fixture(), None, Some("reset:"), None).expect("run");
    assert!(a.all_passed() && b.all_passed(), "{}\n{}", a.render(), b.render());
    assert_eq!(a.results[0].checksum, b.results[0].checksum, "two runs of a reset + two swings end in the same state");
    let trace = trace.expect("recorded");
    let loaded = simrun::load(&fixture()).unwrap();
    let report = replay(&trace, &loaded.scene, &loaded.spawns).expect("replay");
    assert!(report.is_clean(), "{}", report.exact.as_ref().or(report.coarse.as_ref()).map(describe).unwrap_or_else(|| format!("{report:?}")));
    assert_eq!(report.compared as u64, trace.final_tick);
    let names: Vec<&str> = trace.events.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["swing", "prop_hit", "line_down", "line_reset", "swing", "prop_hit", "line_down"], "the recorded event log");
}

/// The real offline client (`re2 --headless`) runs the same rules: walking onto the kick pad launches the crate into the pit, the
/// `prop_enter` rule scores, and the client's state dump (`/rules`, `/props`) shows both. `re2` needs the gfx feature (the headless
/// build has no client).
#[cfg(feature = "gfx")]
#[test]
fn the_offline_client_runs_the_prop_rules_too() {
    let dir = scratch("client");
    let script = dir.join("play.json");
    let dump = dir.join("state.json");
    // The offline client starts at the camera's x/z (-6, 4) facing the camera target; the kick pad is centred on (-3.25, 0):
    // yaw 0 faces -Z and 90 faces +X, so head about 34 degrees right of -Z for 5 m, then wait for the crate to land.
    std::fs::write(
        &script,
        serde_json::json!({"steps": [
            {"wait": 0.3}, {"look": {"yaw": 34.5, "pitch": 0}}, {"hold": ["forward"], "secs": 1.6}, {"wait": 3.5},
            {"expect": {"at": "/phase", "eq": "playing"}}, {"snapshot": "kicked"}]})
        .to_string(),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_re2"))
        .args([fixture().to_str().unwrap(), "--as", "human", "--headless", "--script", script.to_str().unwrap(), "--dump", dump.to_str().unwrap()])
        .output()
        .expect("start re2");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    let score = d["rules"]["vars"].as_array().unwrap().iter().find(|v| v["name"] == "score").map(|v| v["value"].as_f64().unwrap());
    assert_eq!(score, Some(1.0), "the prop_enter rule scored in the client: {}\n{text}", d["rules"]);
    let props = d["props"].as_array().expect("the offline dump lists the loose props");
    let crate_state = props.iter().find(|p| p["id"] == "crate").expect("the crate is listed");
    assert!(
        crate_state["pos"][0].as_f64().unwrap() > 1.5 && crate_state["moved"].as_f64().unwrap() > 2.0,
        "the crate flew into the pit: {crate_state}\n{text}"
    );
    assert_eq!(crate_state["held_by"], Value::Null);
}
