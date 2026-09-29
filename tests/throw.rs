//! How a carried prop leaves the hand (ADR 2026-09-29-one-release-velocity): one release-velocity function on the offline client and
//! the authoritative server (the holder's own motion plus `player.throw_speed` along the look), a short window in which the holder's
//! body ignores what it just let go of, and a hold pose that keeps the carried box out of walls lower than the eye. Proven on the
//! committed fixture `tests/fixtures/throw.json` (a flat range with a 1.2 m ledge) and on `PropWorld` directly.

use glam::{Mat4, Vec3};
use red_engine2::physics::{PropWorld, RELEASE_GRACE_TICKS};
use red_engine2::tools::simrun;
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture() -> PathBuf {
    root().join("tests/fixtures/throw.json")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("re2_throw_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn crate_of(result: &red_engine2::sim::scenario::ScenarioResult) -> &red_engine2::sim::scenario::PropFinal {
    result.props.iter().find(|p| p.id == "crate").expect("the crate is reported")
}

#[test]
fn a_release_keeps_the_holders_speed_so_walking_and_sprinting_throw_farther_than_standing() {
    let (report, _) = simrun::run(&fixture(), None, None, None).expect("run");
    let text = report.render();
    assert!(report.all_passed(), "{text}");
    assert_eq!(report.results.len(), 4, "{text}");
    // Distance from the release point (the player's spot at the release, plus the hold offset) to where the crate rests.
    let travel = |i: usize| {
        let r = &report.results[i];
        let (px, _, _) = (r.finals[0].1, r.finals[0].2, r.finals[0].3);
        crate_of(r).pos.x - px
    };
    let (standing, walking, sprinting) = (travel(0), travel(1), travel(2));
    assert!(standing > 0.5 && standing < 1.6, "a standing toss barely leaves the hand: {standing:.2} m ahead of the player\n{text}");
    assert!(walking > standing + 1.0, "walking throws farther than standing: {walking:.2} vs {standing:.2}\n{text}");
    assert!(sprinting > walking + 1.5, "sprinting throws farther than walking: {sprinting:.2} vs {walking:.2}\n{text}");
    assert!(crate_of(&report.results[2]).moved > 10.0, "the sprint throw carried the crate down the range\n{text}");
}

/// Looking up 45 degrees with `throw_speed` 6 lofts the crate onto a 1.2 m ledge whose face is 2.2 m away: an upward throw exists.
#[test]
fn looking_up_with_a_real_throw_speed_clears_a_low_ledge() {
    let dir = scratch("ledge");
    let mut scene: Value = serde_json::from_str(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    scene["player"]["throw_speed"] = json!(6);
    let map = dir.join("throw6.json");
    std::fs::write(&map, serde_json::to_string_pretty(&scene).unwrap()).unwrap();
    let scenario = dir.join("up45.json");
    std::fs::write(
        &scenario,
        json!([{
            "name": "standing, looking up 45 toward the ledge",
            "players": [{"id": "p1", "character": "human", "spawn": "tee"}],
            "script": [
                {"player": "p1", "hold": {"yaw_deg": 90, "pitch_deg": -45, "seconds": 0.2}},
                {"player": "p1", "hold": {"yaw_deg": 90, "pitch_deg": -45, "interact": true, "seconds": 0.1}},
                {"player": "p1", "hold": {"yaw_deg": 270, "pitch_deg": 45, "seconds": 0.3}},
                {"player": "p1", "hold": {"yaw_deg": 270, "pitch_deg": 45, "interact": true, "seconds": 0.05}},
                {"player": "p1", "wait": 3}
            ],
            "expect": [{"event": "drop", "count": 1}, {"prop": "crate", "y_gt": 1.1}, {"prop": "crate", "near": [-3.6, 0], "tol": 1.0}]
        }])
        .to_string(),
    )
    .unwrap();
    let (report, _) = simrun::run(&map, Some(&scenario), None, None).expect("run");
    assert!(report.all_passed(), "{}", report.render());
    // The same throw at the default 1 m/s toss stays on the floor in front of the player.
    let (report, _) = simrun::run(&fixture(), Some(&scenario), None, None).expect("run");
    assert!(!report.all_passed() && crate_of(&report.results[0]).pos.y < 0.2, "{}", report.render());
}

/// The real offline client (`re2 --headless`) releases a crate exactly where the authoritative simulation does: same function, same
/// numbers. `re2` needs the gfx feature (the headless build has no client).
#[cfg(feature = "gfx")]
#[test]
fn the_offline_client_and_the_server_release_a_crate_identically() {
    let dir = scratch("client");
    let script = dir.join("play.json");
    let dump = dir.join("state.json");
    // The client starts at the camera's x/z (the tee) facing +X; the crate is 1.3 m ahead. Same moves as the sim's standing scenario.
    std::fs::write(
        &script,
        json!({"steps": [
            {"wait": 0.3}, {"look": {"yaw": 90, "pitch": -45}}, {"wait": 0.3}, {"interact": 1}, {"wait": 0.3},
            {"look": {"yaw": 90, "pitch": 0}}, {"wait": 0.3}, {"interact": 1}, {"wait": 3}, {"snapshot": "thrown"}]})
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
    let client = d["props"].as_array().unwrap().iter().find(|p| p["id"] == "crate").expect("the crate is in the dump");
    assert_eq!(client["held_by"], Value::Null, "{client}");
    let client_pos = Vec3::new(client["pos"][0].as_f64().unwrap() as f32, client["pos"][1].as_f64().unwrap() as f32, client["pos"][2].as_f64().unwrap() as f32);
    let (report, _) = simrun::run(&fixture(), None, Some("standing, level"), None).expect("run");
    assert!(report.all_passed(), "{}", report.render());
    let server_pos = crate_of(&report.results[0]).pos;
    assert!((client_pos - server_pos).length() < 0.03, "client {client_pos} vs server {server_pos}\n{text}\n{}", report.render());
    assert!(client_pos.x > 0.9, "the crate did leave the hand: {client_pos}");
}

fn world(objects: &str) -> (red_engine2::schema::Scene, PropWorld) {
    let text = format!(
        r##"{{"camera":{{"position":[0,1.7,5],"target":[0,1,0]}},"objects":[
            {{"id":"floor","type":"box","position":[0,-0.1,0],"size":[40,0.2,40],"material":{{"color":"#888888"}}}},
            {objects}]}}"##
    );
    let scene = red_engine2::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}"));
    let world = PropWorld::new(&scene, None);
    (scene, world)
}

/// The world-space box of prop 0 at `pose`: (min, max).
fn box_of(w: &PropWorld, pose: Mat4) -> (Vec3, Vec3) {
    let s = w.props()[0].shape;
    let centre = pose.transform_point3(s.center);
    let half = s.extents * 0.5;
    // The crate is a cube and the hold pose only yaws it, so an axis-aligned box of the same extents is exact enough here.
    (centre - half, centre + half)
}

#[test]
fn a_crate_held_facing_a_low_wall_is_not_pushed_into_it() {
    // A 1.2 m wall whose near face is at x = 0.9; the player stands 0.5 m from it, eye at 1.7 m (the ray a hold pose used to cast
    // from the eye passes clean over it).
    let (_, w) = world(
        r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,6],"material":{"color":"#a07040"}},
            {"id":"low_wall","type":"box","position":[1.0,0.6,0],"size":[0.2,1.2,6],"material":{"color":"#999999"}}"##,
    );
    let eye = Vec3::new(0.4, 1.7, 0.0);
    let (_, max) = box_of(&w, w.hold_pose(0, eye, Vec3::X, 0.35, 0.55, 0.0));
    assert!(max.x <= 0.9 + 0.01, "the held crate's box stops at the wall face (x 0.9), it reaches x {:.2}", max.x);
    // Away from the wall the crate sits at its usual place in front of the player; the wall really did pull it in.
    let (_, free_max) = box_of(&w, w.hold_pose(0, Vec3::new(-3.0, 1.7, 0.0), Vec3::X, 0.35, 0.55, 0.0));
    assert!(free_max.x - (-3.0) > max.x - 0.4 + 0.1, "free reach {:.2} vs pulled-in reach {:.2}", free_max.x + 3.0, max.x - 0.4);
    // A wall taller than the eye gives the same answer as before (the sweep agrees with the old ray there).
    let (_, w2) = world(
        r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,6],"material":{"color":"#a07040"}},
            {"id":"tall_wall","type":"box","position":[1.0,1.5,0],"size":[0.2,3.0,6],"material":{"color":"#999999"}}"##,
    );
    let (_, max2) = box_of(&w2, w2.hold_pose(0, eye, Vec3::X, 0.35, 0.55, 0.0));
    assert!((max2.x - max.x).abs() < 0.02, "low wall {:.2} vs tall wall {:.2}", max.x, max2.x);
}

#[test]
fn a_released_prop_ignores_its_former_holder_for_the_grace_window_then_is_solid_again() {
    let (_, mut w) = world(r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,6],"material":{"color":"#a07040"}}"##);
    let crate_x = |w: &PropWorld| w.prop_pose(0).w_axis.x;
    // Drops the crate, at rest, 0.6 m ahead of a player standing at `x`, then walks the body straight through that spot for
    // `ticks` ticks at 0.1 m per tick (a sprint), and reports how far the crate went.
    let walk_through = |w: &mut PropWorld, x: f32, ticks: u32| -> f32 {
        w.set_player(Vec3::new(x, 0.0, 0.0), 0.35, 1.75);
        w.step();
        w.pick_up(0);
        assert!(w.held().is_some(), "picked up");
        w.set_held_pose(Mat4::from_translation(Vec3::new(x + 0.6, 0.0, 0.0)));
        assert_eq!(w.drop_held(Vec3::ZERO), Some(0));
        let before = crate_x(w);
        for t in 1..=ticks {
            w.set_player(Vec3::new(x + 0.1 * t as f32, 0.0, 0.0), 0.35, 1.75);
            w.step();
        }
        crate_x(w) - before
    };
    // Inside the window the body passes through what it just let go of.
    let during = walk_through(&mut w, 0.0, RELEASE_GRACE_TICKS - 1);
    assert!(during.abs() < 0.03, "during the grace window the body does not shove the crate: it moved {during:.3} m");
    // Once the window is over the same walk pushes it along.
    let x = w.prop_pose(0).w_axis.x + 1.5;
    w.set_player(Vec3::new(x, 0.0, 0.0), 0.35, 1.75);
    for _ in 0..RELEASE_GRACE_TICKS {
        w.step();
    }
    let after = {
        let before = crate_x(&w);
        for t in 1..=30 {
            w.set_player(Vec3::new(x - 0.1 * t as f32, 0.0, 0.0), 0.35, 1.75);
            w.step();
        }
        before - crate_x(&w)
    };
    assert!(after > 0.3, "after the window the body pushes the crate: it moved {after:.3} m");
    // Picking it up again and releasing it starts a fresh window.
    let start = crate_x(&w) - 0.6;
    let again = walk_through(&mut w, start, RELEASE_GRACE_TICKS - 1);
    assert!(again.abs() < 0.03, "a second release gets its own window: it moved {again:.3} m");
}
