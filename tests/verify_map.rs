//! The verification tools honour what the map declares (ADR 2026-09-29-verification-honours-the-map): a spawn's height, the scene's
//! `player` tuning and `jump_pads`, `from` / `from_y` on any walk or reach entry, a `start` rule that opens a gate, and a blueprint whose
//! `scene` block brings the spawns. Each test writes a tiny scene and runs the real CLI (`lint`, `verify`, `build`) on it; the spawn
//! height is also checked in the real offline client.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("re2_verify_map_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_red_engine2")).args(args).output().expect("start red_engine2")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// A walled 16 x 16 m room, floor at y 0, 6 m high (a 3 m deck needs the 2.05 m headroom above it), with `objects` added and the
/// top-level `keys` merged in.
fn room(objects: Value, keys: Value) -> Value {
    let mut v = json!({
        "meta": {"fps": 30, "duration": 4, "resolution": [640, 360]},
        "camera": {"fov": 70, "position": [-6, 1.7, 0], "target": [0, 1, 0]},
        "ambient": {"color": "#ffffff", "intensity": 0.5},
        "lights": [{"id": "lamp", "type": "point", "position": [0, 5.6, 0], "color": "#fff0d0", "intensity": 12, "range": 16}],
        "objects": [
            {"id": "floor", "type": "plane", "size": [16, 16], "position": [0, 0.01, 0], "material": {"color": "#c8c0b0"}},
            {"id": "ceiling", "type": "box", "size": [16, 0.2, 16], "position": [0, 6.1, 0], "material": {"color": "#e8e4dc"}},
            {"id": "wall_n", "type": "wall", "from": [-8, -8], "to": [8, -8], "height": 6, "thickness": 0.24},
            {"id": "wall_s", "type": "wall", "from": [-8, 8], "to": [8, 8], "height": 6, "thickness": 0.24},
            {"id": "wall_w", "type": "wall", "from": [-8, -8], "to": [-8, 8], "height": 6, "thickness": 0.24},
            {"id": "wall_e", "type": "wall", "from": [8, -8], "to": [8, 8], "height": 6, "thickness": 0.24}
        ]
    });
    v["objects"].as_array_mut().unwrap().extend(objects.as_array().unwrap().iter().cloned());
    for (k, val) in keys.as_object().unwrap() {
        v[k] = val.clone();
    }
    v
}

fn write(dir: &std::path::Path, name: &str, v: &Value) -> String {
    let p = dir.join(name);
    std::fs::write(&p, serde_json::to_string_pretty(v).unwrap()).unwrap();
    p.to_str().unwrap().to_string()
}

fn deck_scene() -> Value {
    room(
        json!([{"id": "deck", "type": "box", "size": [6, 3, 6], "position": [0, 1.5, 0], "material": {"color": "#8a8a90"}}]),
        json!({
            "spawns": [{"id": "on_deck", "position": [0, 3, 0], "yaw_deg": 90, "group": "solo"}],
            "checks": {
                "lint": {"max_errors": 0, "max_warnings": 20},
                "reach": [{"to": [2, 2], "why": "the deck top is reachable from the spawn on it"}],
                "walk": [
                    {"name": "across the deck from the spawn", "path": "2,2; -2,-2", "ends_near": [-2, -2], "floor_y": 3},
                    {"name": "a path walk that starts elsewhere on the deck", "from": [2, 2], "from_y": 3, "path": "-2,2", "ends_near": [-2, 2], "floor_y": 3}
                ]
            }
        }),
    )
}

#[test]
fn a_spawn_on_a_deck_lints_clean_and_walks_start_on_the_deck() {
    let dir = scratch("deck");
    let map = write(&dir, "deck.json", &deck_scene());
    let lint = cli(&["lint", &map]);
    let t = text(&lint);
    assert!(lint.status.success() && t.contains("0 error(s)"), "a spawn on a 3 m deck is not `inside solid geometry`:\n{t}");
    let verify = cli(&["verify", &map, "--no-views"]);
    let t = text(&verify);
    assert!(verify.status.success(), "reach and both walks start on the deck (y 3):\n{t}");
    assert!(t.contains("y=3.00"), "the walks end on the deck:\n{t}");
    // The CLI walk without --from starts at the spawn too, on its floor.
    let walk = cli(&["walk", &map, "--path", "2,2"]);
    let t = text(&walk);
    assert!(walk.status.success() && t.contains("y=3.00"), "{t}");
}

/// The real offline client starts where a match would: at the first spawn, at its height, facing its way.
#[cfg(feature = "gfx")]
#[test]
fn the_offline_client_starts_at_the_first_spawn_at_its_height() {
    let dir = scratch("client");
    let map = write(&dir, "deck.json", &deck_scene());
    let script = dir.join("look.json");
    let dump = dir.join("state.json");
    std::fs::write(&script, json!({"steps": [{"wait": 0.5}, {"snapshot": "start"}]}).to_string()).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_re2"))
        .args([&map, "--as", "human", "--headless", "--script", script.to_str().unwrap(), "--dump", dump.to_str().unwrap()])
        .output()
        .expect("start re2");
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    let pos = &d["player"]["pos"];
    assert!((pos[1].as_f64().unwrap() - 3.0).abs() < 0.05, "feet on the deck, not at y 0: {pos}\n{t}");
    assert!(pos[0].as_f64().unwrap().abs() < 0.05 && pos[2].as_f64().unwrap().abs() < 0.05, "at the spawn's x/z, not the camera's: {pos}");
    assert!((d["player"]["yaw_deg"].as_f64().unwrap() - 90.0).abs() < 1.0, "facing the spawn's way: {}", d["player"]["yaw_deg"]);
}

#[test]
fn a_blueprint_without_spawns_takes_them_from_its_scene_block() {
    let dir = scratch("blueprint");
    // The hall's centre (-9, 0) is inside a pillar, so a self-check from the room centre would fail; the `scene` block puts the
    // spawn on free floor and the built map must start there.
    let bp = json!({
        "blueprint": 1, "name": "pillar_hall", "height": 2.8,
        "rooms": [{"id": "hall", "rect": [-14, -6, -4, 6]}, {"id": "store", "rect": [-4, -6, 6, 6]}],
        "doors": [{"between": ["hall", "store"], "width": 1.4}],
        "spawns": [],
        "extra": [{"id": "pillar", "type": "box", "size": [3, 2.8, 3], "position": [-9, 1.4, 0], "material": {"color": "#999999"}}],
        "scene": {"spawns": [{"id": "start", "position": [-12, 0, 4], "yaw_deg": 90, "group": "solo"}]}
    });
    let bp_path = write(&dir, "pillar.blueprint.json", &bp);
    let map = dir.join("pillar.json");
    let build = cli(&["build", &bp_path, "--out", map.to_str().unwrap()]);
    let t = text(&build);
    assert!(build.status.success(), "the self-check starts from the merged spawn, not the pillar at the room centre:\n{t}");
    let built: Value = serde_json::from_str(&std::fs::read_to_string(&map).unwrap()).unwrap();
    let nums = |v: &Value| -> Vec<f64> { v.as_array().unwrap().iter().map(|n| n.as_f64().unwrap()).collect() };
    assert_eq!(nums(&built["spawns"][0]["position"]), [-12.0, 0.0, 4.0]);
    assert_eq!(nums(&built["camera"]["position"]), [-12.0, 1.7, 4.0], "the camera looks from the spawn: {}", built["camera"]);
    assert_eq!(nums(&built["checks"]["reach"][0]["from"]), [-12.0, 4.0], "the generated checks start at the spawn: {}", built["checks"]["reach"]);
    let verify = cli(&["verify", map.to_str().unwrap(), "--no-views"]);
    assert!(verify.status.success(), "{}", text(&verify));
}

fn pad_scene(with_pad: bool) -> Value {
    let mut keys = json!({
        "spawns": [{"id": "start", "position": [-5, 0, 0], "yaw_deg": 90, "group": "solo"}],
        "checks": {
            "lint": {"max_errors": 0, "max_warnings": 20},
            "walk": [{"name": "over the pad onto the platform", "path": "1.5,0; 5,0", "ends_near": [5, 0], "tol": 0.5, "floor_y": 2}]
        }
    });
    if with_pad {
        keys["jump_pads"] = json!([{"id": "pad", "position": [1.5, 0, 0], "size": [1, 1], "launch_speed": 9.5}]);
    }
    room(json!([{"id": "platform", "type": "box", "size": [4, 2, 4], "position": [5, 1, 0], "material": {"color": "#8a8a90"}}]), keys)
}

#[test]
fn a_walk_over_a_jump_pad_reaches_a_platform_only_the_pad_can_reach() {
    let dir = scratch("pad");
    let map = write(&dir, "pad.json", &pad_scene(true));
    let verify = cli(&["verify", &map, "--no-views", "--only", "walk"]);
    let t = text(&verify);
    assert!(verify.status.success() && t.contains("y=2.00"), "the walker rides the pad onto the 2 m platform:\n{t}");
    let map = write(&dir, "no_pad.json", &pad_scene(false));
    let verify = cli(&["verify", &map, "--no-views", "--only", "walk"]);
    let t = text(&verify);
    assert!(!verify.status.success() && t.contains("stuck"), "without the pad the 2 m face stops the walker:\n{t}");
}

fn gate_scene(with_rule: bool, path: &str) -> Value {
    let mut keys = json!({
        "spawns": [{"id": "start", "position": [-5, 0, 0], "yaw_deg": 90, "group": "solo"}],
        "zones": [{"id": "east", "rect": [3, -7.5, 7.5, 7.5], "y": 0, "kind": "room"}],
        "checks": {
            "lint": {"max_errors": 0, "max_warnings": 0},
            "walk": [{"name": "through the gate", "path": path, "ends_near": [5, 0], "tol": 0.4}]
        }
    });
    if with_rule {
        keys["rules"] = json!([{"id": "open_gate", "when": {"start": true}, "do": [{"collision": ["gate", false]}, {"hide": "gate"}]}]);
    }
    room(json!([{"id": "gate", "type": "box", "size": [0.4, 3, 16], "position": [2, 1.5, 0], "material": {"color": "#a05030"}}]), keys)
}

#[test]
fn a_gate_a_start_rule_opens_is_open_to_lint_and_walks_too() {
    let dir = scratch("gate");
    let map = write(&dir, "open.json", &gate_scene(true, "5,0"));
    let lint = cli(&["lint", &map]);
    let t = text(&lint);
    assert!(lint.status.success() && t.contains("0 error(s), 0 warning(s)"), "the east zone is fully reachable through the open gate:\n{t}");
    let verify = cli(&["verify", &map, "--no-views"]);
    assert!(verify.status.success(), "{}", text(&verify));
    // Without the rule the gate is a wall: the zone is sealed and the walk is blocked.
    let map = write(&dir, "closed.json", &gate_scene(false, "5,0"));
    let lint = cli(&["lint", &map]);
    let t = text(&lint);
    assert!(t.contains("zone 'east'"), "a closed gate seals the zone:\n{t}");
    let verify = cli(&["verify", &map, "--no-views", "--only", "walk"]);
    let t = text(&verify);
    assert!(!verify.status.success() && t.contains("gate"), "the walk names the gate that blocked it:\n{t}");
}

#[test]
fn a_path_walk_that_meant_its_first_point_as_the_start_is_told_so() {
    let dir = scratch("hint");
    let map = write(&dir, "closed.json", &gate_scene(false, "5,0; 6,0"));
    let verify = cli(&["verify", &map, "--no-views", "--only", "walk"]);
    let t = text(&verify);
    assert!(!verify.status.success(), "{t}");
    assert!(t.contains("a walk starts at the spawn (-5.0, 0.0), not at the first point of `path`"), "{t}");
}

#[test]
fn a_map_can_ignore_lint_codes_it_accepts_everywhere() {
    let dir = scratch("ignore");
    let mut strict = deck_scene();
    strict["checks"]["lint"] = json!({"max_errors": 0, "max_warnings": 0});
    let map = write(&dir, "strict.json", &strict);
    let verify = cli(&["verify", &map, "--no-views", "--only", "lint"]);
    let t = text(&verify);
    assert!(
        !verify.status.success() && t.contains("warning(s)"),
        "the deck's open edges are `drop` warnings:
{t}"
    );
    let mut lenient = deck_scene();
    lenient["checks"]["lint"] = json!({"max_errors": 0, "max_warnings": 0, "ignore": ["drop"]});
    let map = write(&dir, "lenient.json", &lenient);
    let verify = cli(&["verify", &map, "--no-views", "--only", "lint"]);
    let t = text(&verify);
    assert!(
        verify.status.success() && t.contains("ignored (drop)"),
        "with `ignore: [\"drop\"]` the same map passes and says what it skipped:
{t}"
    );
}

#[test]
fn info_says_whether_an_object_is_loose_and_why() {
    let dir = scratch("info");
    let map = write(&dir, "deck.json", &deck_scene());
    let t = text(&cli(&["info", &map, "deck"]));
    assert!(t.contains("loose: no (not a `prop` or a prefab"), "{t}");
    let props = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/prop_rules.json");
    let t = text(&cli(&["info", props, "crate"]));
    assert!(t.contains("loose: yes") && t.contains("kg") && t.contains("a human can carry it: yes"), "{t}");
    let t = text(&cli(&["info", props, "d1"]));
    assert!(t.contains("loose: yes"), "a monolith with `movable: true`: {t}");
}
