//! Scene `fields`: force volumes on loose props (a river current, a conveyor, a wind tunnel). A crate lying in a field with a target
//! velocity is carried along and stops at a wall (it is pulled toward a *speed*, so it never runs away the way a repeating `impulse` rule
//! does); the same scene without the field leaves it where it was; a bad field is an error that names the fix; the run is deterministic.

use red_engine2::tools::simrun;
use serde_json::{json, Value};
use std::path::PathBuf;

fn scene(fields: Value) -> Value {
    json!({
        "camera": {"position": [0, 1.7, 8], "target": [0, 1, 0]},
        "spawns": [{"id": "start", "position": [0, 0, 8], "yaw_deg": 0}],
        "zones": [{"id": "belt", "rect": [-1, -1, 7, 1], "y": 0}],
        "fields": fields,
        "checks": {"sim": [{
            "name": "the crate rides the field to the wall",
            "players": [{"id": "p1", "spawn": "start"}],
            "script": [{"player": "p1", "wait": 6}],
            "expect": [{"prop": "crate", "near": [6.2, 0.0], "tol": 0.7}],
            "max_seconds": 20
        }]},
        "objects": [
            {"id": "floor", "type": "box", "position": [0, -0.1, 0], "size": [30, 0.2, 30], "material": {"color": "#777777"}},
            {"id": "wall", "type": "box", "position": [7.5, 0.6, 0], "size": [0.3, 1.2, 3], "material": {"color": "#555555"}},
            {"id": "crate", "type": "prop", "prop": "crate", "position": [0, 0, 0]}
        ]
    })
}

fn write(name: &str, v: &Value) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("re2_fields_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(v).unwrap()).unwrap();
    path
}

#[test]
fn a_field_carries_a_crate_to_the_wall_and_without_it_the_crate_stays() {
    let with = write("with", &scene(json!([{"id": "current", "zone": "belt", "velocity": [2.0, 0.0]}])));
    let (report, _) = simrun::run(&with, None, None, None).expect("run");
    assert!(report.all_passed(), "{}", report.render());
    let none = write("without", &scene(json!([])));
    let (report, _) = simrun::run(&none, None, None, None).expect("run");
    assert!(!report.all_passed(), "without the field the crate must not reach the wall:\n{}", report.render());
}

#[test]
fn the_same_field_run_twice_ends_in_exactly_the_same_place() {
    let path = write("twice", &scene(json!([{"id": "current", "zone": "belt", "velocity": [2.0, 0.3], "rate": 6}])));
    let end = |p: &PathBuf| {
        let (report, _) = simrun::run(p, None, None, None).expect("run");
        report.results[0].to_json()["props"].to_string()
    };
    assert_eq!(end(&path), end(&path));
}

#[test]
fn a_bad_field_says_what_to_fix() {
    let errs = |fields: Value| red_engine2::schema::parse_scene(&scene(fields).to_string()).err().unwrap_or_default().join("\n");
    assert!(errs(json!([{"id": "a", "zone": "nowhere", "velocity": [1, 0]}])).contains("no zone `nowhere`"));
    assert!(errs(json!([{"id": "a", "zone": "belt", "velocity": [1]}])).contains("must be two numbers [x, z]"));
    assert!(errs(json!([{"id": "a", "zone": "belt"}])).contains("give `velocity`"));
    assert!(errs(json!([{"id": "a", "zone": "belt", "velocity": [1, 0], "rate": 0}])).contains("rate"));
    assert!(errs(json!([{"id": "a", "zone": "belt", "velocity": [1, 0], "velocty": [1, 0]}])).contains("velocity"), "an unknown key gets a did-you-mean");
    assert!(errs(json!([{"id": "a", "zone": "belt", "velocity": [1, 0]}, {"id": "a", "zone": "belt", "lift": 2}])).contains("duplicate field id"));
    assert_eq!(errs(json!([{"id": "a", "zone": "belt", "lift": 2.0}])), "", "a lift-only field is valid");
}

#[test]
fn an_internal_underscore_variable_can_be_declared_and_read() {
    let mut v = scene(json!([]));
    v["vars"] = json!({"_latch": false});
    v["rules"] = json!([{"id": "r", "when": {"start": true}, "if": "!_latch", "do": [{"set": ["_latch", true]}]}]);
    assert_eq!(red_engine2::schema::parse_scene(&v.to_string()).err().unwrap_or_default().join("\n"), "");
}
