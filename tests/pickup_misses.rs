//! A `sim` scenario that presses interact empty-handed and picks nothing up says why (too big, too far, nothing aimed at), and a
//! pick-up that works prints no note. The reasons are diagnostics: they are not part of the match checksum.

use red_engine2::tools::simrun;
use serde_json::{json, Value};

fn scene() -> Value {
    let aim = |name: &str, at: [f64; 3]| {
        json!({"name": name, "players": [{"id": "p1", "spawn": "s"}], "max_seconds": 10, "expect": [{"not_ended": true}],
               "script": [{"player": "p1", "hold": {"look_at": at, "seconds": 0.2}},
                          {"player": "p1", "hold": {"look_at": at, "interact": true, "seconds": 0.2}},
                          {"player": "p1", "wait": 0.3}]})
    };
    json!({
        "camera": {"position": [0, 6, -6], "target": [0, 0, 2]},
        "player": {"mode": "peaceful"},
        "spawns": [{"id": "s", "position": [0, 0, 0], "yaw_deg": 0}],
        "objects": [
            {"id": "floor", "type": "plane", "size": [30, 30], "position": [0, 0, 0]},
            {"id": "light", "type": "prop", "prop": "crate", "position": [0, 0, 1.4]},
            {"id": "heavy", "type": "prop", "prop": "crate", "position": [-1.3, 0, 1.3], "scale": [2.4, 2.4, 2.4], "movable": true},
            {"id": "far", "type": "prop", "prop": "crate", "position": [0, 0, 9]}
        ],
        "checks": {"sim": [
            aim("light works", [0.0, 0.3, 1.4]),
            aim("heavy", [-1.3, 0.6, 1.3]),
            aim("far", [0.0, 0.3, 9.0]),
            aim("empty floor", [4.0, 0.0, 3.0])
        ]}
    })
}

#[test]
fn a_failed_pickup_names_its_reason_and_a_good_one_stays_quiet() {
    let path = std::env::temp_dir().join(format!("re2_pickup_misses_{}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_string(&scene()).unwrap()).unwrap();
    let (report, _) = simrun::run(&path, None, None, None).expect("run");
    let by = |name: &str| report.results.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no scenario {name}"));
    assert!(by("light works").pickup_misses.is_empty(), "{:?}", by("light works").pickup_misses);
    assert!(by("light works").render().contains("carried by p1"), "it really was picked up");
    let note = |name: &str| by(name).pickup_misses.first().cloned().unwrap_or_else(|| panic!("{name}: no note\n{}", by(name).render()));
    assert!(note("heavy").contains("too big to carry") && note("heavy").contains("limit 0.45"), "{}", note("heavy"));
    assert!(note("far").contains("away") && note("far").contains("step closer"), "{}", note("far"));
    assert!(note("empty floor").contains("no loose prop under the crosshair"), "{}", note("empty floor"));
    assert!(by("heavy").render().contains("note: tick"), "the text report prints the note");
    assert_eq!(by("heavy").to_json()["pickup_misses"].as_array().unwrap().len(), by("heavy").pickup_misses.len());
    let _ = std::fs::remove_file(&path);
}
