//! Scene `fields`: force volumes on loose props (a river current, a conveyor, a wind tunnel). A crate lying in a field with a target
//! velocity is carried along and stops at a wall (it is pulled toward a *speed*, so it never runs away the way a repeating `impulse` rule
//! does); the same scene without the field leaves it where it was; a bad field is an error that names the fix; the run is deterministic.
//!
//! **Aimed fields** (`toward` + `speed`, ADR 2026-10-13-aimed-fields-and-the-rule-s-subject-prop), through the real simulation: a crate is pulled along
//! its own bearing to the target in every direction (not along an axis), the speed expression orders gentle < rush, inverse pushes away and
//! zero is off, and the two games built on it (Velvet Orchestra, `recipe whistle_herd`) record and replay without a divergence.

use red_engine2::sim::replay::replay;
use red_engine2::sim::scenario::RecordOptions;
use red_engine2::tools::simrun;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

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

/// A crate at the origin, an anchor (a loose traffic cone) `dist` metres away at `bearing_deg` (0 = +x, 90 = +z), an aimed field over a big
/// meadow pulling only the crate toward the anchor at `speed` (an expression over `dial`), and a scenario that waits `secs`.
fn aimed(bearing_deg: f32, dist: f32, speed: &str, dial: f64, secs: f64) -> Value {
    let (s, c) = bearing_deg.to_radians().sin_cos();
    json!({
        "camera": {"position": [0, 12, -12], "target": [0, 0, 0]},
        "spawns": [{"id": "start", "position": [0, 0, -12], "yaw_deg": 0}],
        "zones": [{"id": "meadow", "rect": [-14, -14, 14, 14], "y": 0}],
        "vars": {"dial": dial},
        "fields": [{"id": "pull", "zone": "meadow", "toward": "anchor", "speed": speed, "props": ["crate"]}],
        "checks": {"sim": [{"name": "wait", "players": [{"id": "p1", "spawn": "start"}], "script": [{"player": "p1", "wait": secs}], "expect": [{"field": "pull", "aimed_at": "anchor"}]}]},
        "objects": [
            {"id": "floor", "type": "box", "position": [0, -0.1, 0], "size": [30, 0.2, 30], "material": {"color": "#777777"}},
            {"id": "crate", "type": "prop", "prop": "crate", "position": [0, 0, 0]},
            {"id": "anchor", "type": "prop", "prop": "traffic_cone", "position": [c * dist, 0, s * dist]}
        ]
    })
}

/// Where loose prop `id` ended in the (single) scenario of `path`: `(x, z)`.
fn end_xz(path: &Path, id: &str) -> (f32, f32) {
    let (report, _) = simrun::run(path, None, None, None).expect("run");
    assert!(report.all_passed(), "{}", report.render());
    let props = report.results[0].to_json()["props"].clone();
    let p = props.as_array().unwrap().iter().find(|p| p["id"] == id).unwrap_or_else(|| panic!("no prop {id} in {props}"));
    let at = &p["pos"];
    (at[0].as_f64().unwrap() as f32, at[2].as_f64().unwrap() as f32)
}

#[test]
fn an_aimed_field_pulls_a_crate_along_its_own_bearing_in_every_direction() {
    for k in 0..8 {
        let bearing = 22.5 + 45.0 * k as f32; // never on an axis
        let path = write(&format!("bearing_{k}"), &aimed(bearing, 6.0, "3", 0.0, 1.0));
        let (x, z) = end_xz(&path, "crate");
        let moved = (x * x + z * z).sqrt();
        assert!(moved > 0.8 && moved < 5.5, "bearing {bearing}: the crate moved {moved} m (it should travel toward the anchor, not reach it)");
        let got = z.atan2(x).to_degrees().rem_euclid(360.0);
        let off = ((got - bearing + 540.0).rem_euclid(360.0) - 180.0).abs();
        // Measured 2.2-4.4 deg (2026-10-12): a sliding box's contacts nudge it sideways and the pull does not steer it back. A lean along the
        // nearest axis would be 22.5 deg off for every one of these bearings.
        assert!(off < 6.0, "bearing {bearing}: the crate travelled at {got:.1} deg, {off:.1} deg off its line to the anchor");
    }
}

#[test]
fn the_speed_expression_orders_gentle_and_rush_inverse_pushes_away_and_zero_is_off() {
    let speed = "(dial == 0) * 1.5 + (dial == 1) * 4 - (dial == 3) * 3";
    let dist = |dial: f64| {
        let (x, z) = end_xz(&write(&format!("dial_{dial}"), &aimed(30.0, 8.0, speed, dial, 1.5)), "crate");
        (x * x + z * z).sqrt() // how far the crate went from the origin
    };
    let to_anchor = |dial: f64| {
        let (x, z) = end_xz(&write(&format!("dial_far_{dial}"), &aimed(30.0, 8.0, speed, dial, 1.5)), "crate");
        let (ax, az) = (8.0 * 30f32.to_radians().cos(), 8.0 * 30f32.to_radians().sin());
        ((x - ax).powi(2) + (z - az).powi(2)).sqrt()
    };
    let (gentle, rush, hush) = (dist(0.0), dist(1.0), dist(2.0));
    assert!(gentle > 0.3, "gentle still moves the crate: {gentle}");
    assert!(rush > gentle * 2.0, "a rush is much faster than gentle: {rush} vs {gentle}");
    assert!(hush < 0.01, "hush: no pull and no drift: {hush}");
    assert!(to_anchor(3.0) > 8.5, "inverse ends farther from the anchor than it started (8 m)");
}

/// Records scenario `only` of `scene` tick by tick, replays it with no renderer or socket, and asserts the game agrees bit for bit.
fn records_and_replays_clean(scene: &Path, only: &str) {
    let opts = RecordOptions { map_hash: 0, checkpoint_every: 1, dump_every: 60 };
    let (report, trace) = simrun::run(scene, None, Some(only), Some(opts)).expect("run");
    assert!(report.all_passed(), "{}", report.render());
    let trace = trace.expect("a trace");
    let loaded = simrun::load(scene).expect("load");
    let r = replay(&trace, &loaded.scene, &loaded.spawns).expect("replay");
    assert!(r.is_clean(), "{only}: the replay diverged: {r:?}");
    assert!(r.compared > 100, "{only}: compared {} checkpoints", r.compared);
}

#[test]
fn aimed_gravity_and_its_subject_rules_record_and_replay_without_a_divergence() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let velvet = root.join("examples/3d/velvet-orchestra/velvet-orchestra.json");
    records_and_replays_clean(&velvet, "win: carry the weight east");
    records_and_replays_clean(&velvet, "the listener stands in the west slot");
    records_and_replays_clean(&root.join("recipes/whistle_herd.json"), "carry the lantern behind the flock");
    // Two runs of the same inputs end with the same props, to the bit.
    let props = |p: &Path| simrun::run(p, None, Some("win: carry the weight east"), None).unwrap().0.results[0].to_json()["props"].to_string();
    assert_eq!(props(&velvet), props(&velvet));
}
