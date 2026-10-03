//! `approach` / `look_at` / `interact` by object id in `checks.sim` scenarios (ADR 2026-10-03-object-based-player-actions): a far crate is
//! picked up without a coordinate or an angle in the script, and each way it can go wrong names the object, the place and the reason.

use red_engine2::tools::simrun;
use serde_json::{json, Value};

fn scenario(name: &str, script: Value, expect: Value) -> Value {
    json!({"name": name, "players": [{"id": "p1", "spawn": "s"}], "max_seconds": 20, "script": script, "expect": expect})
}

fn scene(scenarios: Vec<Value>) -> Value {
    json!({
        "camera": {"position": [0, 6, -6], "target": [0, 0, 2]},
        "player": {"mode": "peaceful"},
        "spawns": [{"id": "s", "position": [0, 0, 0], "yaw_deg": 0}],
        "vars": {"pressed": 0},
        "rules": [{"id": "press", "when": {"enter": {"object": "button", "pad": 1.8}}, "once": true, "do": [{"add": ["pressed", 1]}]}],
        "objects": [
            {"id": "floor", "type": "plane", "size": [40, 40], "position": [0, 0, 0]},
            {"id": "near_crate", "type": "prop", "prop": "crate", "position": [0, 0, 1.4]},
            {"id": "far_crate", "type": "prop", "prop": "crate", "position": [6, 0, 9]},
            {"id": "heavy", "type": "prop", "prop": "crate", "position": [-5, 0, 3], "scale": [2.4, 2.4, 2.4], "movable": true},
            {"id": "wall", "type": "box", "size": [8, 3, 0.3], "position": [0, 1.5, 4.5]},
            {"id": "behind_wall", "type": "prop", "prop": "crate", "position": [0, 0, 6.5]},
            {"id": "button", "type": "box", "size": [0.4, 0.4, 0.4], "position": [-6, 0.2, -4]}
        ],
        "checks": {"sim": scenarios}
    })
}

fn run(scenarios: Vec<Value>) -> simrun::SimReport {
    // Tests run in parallel: the file is named after the first scenario.
    let first = scenarios[0]["name"].as_str().unwrap_or("x").replace(' ', "_");
    let path = std::env::temp_dir().join(format!("re2_object_actions_{}_{first}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_string(&scene(scenarios)).unwrap()).unwrap();
    let (report, _) = simrun::run(&path, None, None, None).expect("run");
    let _ = std::fs::remove_file(&path);
    report
}

#[test]
fn interact_by_id_walks_up_aims_and_picks_up_a_crate_nine_metres_away() {
    let report = run(vec![
        scenario("far", json!([{"player": "p1", "interact": "far_crate"}]), json!([{"prop": "far_crate", "held_by": "p1"}])),
        scenario("near", json!([{"player": "p1", "interact": "near_crate"}]), json!([{"prop": "near_crate", "held_by": "p1"}])),
        scenario(
            "approach then look then interact, step by step",
            json!([{"player": "p1", "approach": "far_crate", "within": 1.0}, {"player": "p1", "look_at": "far_crate"}, {"player": "p1", "interact": "far_crate"}]),
            json!([{"prop": "far_crate", "held_by": "p1"}, {"player": "p1", "near": [6.0, 9.0], "tol": 1.6}]),
        ),
        scenario(
            "a non-prop object can be walked up to and pressed",
            json!([{"player": "p1", "interact": "button"}]),
            json!([{"var": "pressed", "gte": 1}, {"player": "p1", "near": [-6.0, -4.0], "tol": 1.8}]),
        ),
    ]);
    for r in &report.results {
        assert!(r.passed, "{}", r.render());
    }
}

#[test]
fn each_failure_names_the_object_the_place_and_the_reason() {
    let report = run(vec![
        scenario("walled off", json!([{"player": "p1", "approach": "behind_wall"}]), json!([])),
        scenario("too slow", json!([{"player": "p1", "approach": "far_crate", "timeout": 0.5}]), json!([])),
        scenario("too heavy", json!([{"player": "p1", "interact": "heavy"}]), json!([])),
        scenario("hands full", json!([{"player": "p1", "interact": "near_crate"}, {"player": "p1", "interact": "far_crate"}]), json!([])),
    ]);
    let by = |n: &str| report.results.iter().find(|r| r.name == n).unwrap_or_else(|| panic!("no scenario {n}"));
    let text = |n: &str| by(n).render();
    assert!(
        !by("walled off").passed && text("walled off").contains("approach `behind_wall`") && text("walled off").contains("in the way"),
        "{}",
        text("walled off")
    );
    assert!(!by("too slow").passed && text("too slow").contains("raise `timeout`"), "{}", text("too slow"));
    assert!(
        !by("too heavy").passed && text("too heavy").contains("did not pick it up") && text("too heavy").contains("too big to carry"),
        "{}",
        text("too heavy")
    );
    assert!(!by("hands full").passed && text("hands full").contains("already carrying `near_crate`"), "{}", text("hands full"));
}

#[test]
fn an_unknown_object_is_a_parse_error_with_a_suggestion_and_a_nested_one_is_refused_at_run() {
    let path = std::env::temp_dir().join(format!("re2_object_actions_bad_{}.json", std::process::id()));
    let bad = scene(vec![
        scenario("typo", json!([{"player": "p1", "interact": "far_crat"}]), json!([])),
        scenario("wrong key", json!([{"player": "p1", "walk": "1,1", "within": 1.0}]), json!([])),
    ]);
    std::fs::write(&path, serde_json::to_string(&bad).unwrap()).unwrap();
    let err = simrun::run(&path, None, None, None).err().expect("a scenario with a typo is rejected");
    assert!(err.contains("no object `far_crat`") && err.contains("did you mean `far_crate`"), "{err}");
    assert!(err.contains("only `approach` and `interact` take `within`"), "{err}");
    let _ = std::fs::remove_file(&path);
}
