//! One physical world: the runtime and the analysis tools agree on what exists, before and after a rule opens a gate.
//!
//! The scene is a pen on generated country (hills, trees) with a gate in its north wall and a lever inside. Standing on the lever fires a rule that switches the gate's collision off,
//! and a named phase (`gate_open`) tells the analysis tools to assume that rule has fired. Everything that answers "what is solid here, and how high is the floor?" is asked in both
//! states: the single-player client's assembly, the server and `LocalSession` (`MatchSim`), the online client and bots (`ClientWorld`), `MapWorld` (lint, reach, walk, plan, verify) and
//! the scene's own scripted `checks`. Opening the gate must remove the gate's collider and nothing else: not the hills, not a tree, not a wall.

use glam::Vec2;
use red_engine2::collide::{ground_height_at, Collider2D, GroundCandidates, PhysicalWorld};
use red_engine2::net::bot::ClientWorld;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use red_engine2::tools::verify::{run, Options};
use red_engine2::tools::world::MapWorld;
use std::path::PathBuf;

const SEED: u32 = 7;

fn scene_text(checks: &str) -> String {
    let wall = |id: &str, size: [f32; 3], at: [f32; 3]| format!(r#"{{"id": "{id}", "type": "box", "size": {size:?}, "position": {at:?}}}"#);
    // 16 x 16 m pen, walls 30 m tall so a hill never goes over them; a 3 m gate in the middle of the north wall.
    let objects = [
        wall("wall_nw", [6.5, 30.0, 0.5], [-4.75, 0.0, -8.0]),
        wall("wall_ne", [6.5, 30.0, 0.5], [4.75, 0.0, -8.0]),
        wall("gate", [3.0, 30.0, 0.5], [0.0, 0.0, -8.0]),
        wall("wall_s", [16.5, 30.0, 0.5], [0.0, 0.0, 8.0]),
        wall("wall_w", [0.5, 30.0, 16.5], [-8.0, 0.0, 0.0]),
        wall("wall_e", [0.5, 30.0, 16.5], [8.0, 0.0, 0.0]),
    ]
    .join(",");
    format!(
        r#"{{"camera": {{"position": [0, 1.6, 4], "target": [0, 1.6, -4]}}, "procgen": {{"seed": {SEED}}},
        "spawns": [{{"id": "start", "position": [0, 0, 6.5], "yaw_deg": 0}}],
        "zones": [{{"id": "lever", "rect": [-2, 1.5, 2, 5], "y": -1.0, "kind": "room"}}],
        "rules": [{{"id": "open_gate", "when": {{"enter": {{"zone": "lever"}}}}, "once": true, "do": [{{"deactivate": "gate"}}, {{"emit": "gate_open"}}]}}],
        "phases": {{"gate_open": ["open_gate"]}},
        "objects": [{objects}], "checks": {checks}}}"#
    )
}

fn write(name: &str, text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("re2_ground_consistency_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.json");
    std::fs::write(&path, text).unwrap();
    path
}

/// What a world looks like to anything that asks: every footprint, the floor height on a grid (standing high, so a box top counts, and the terrain alone), and the trees near the pen.
#[derive(Debug, PartialEq, Clone)]
struct Fingerprint {
    colliders: Vec<String>,
    floor_high: Vec<i64>,
    terrain: Vec<i64>,
    trees: Vec<String>,
}

fn mm(v: f32) -> i64 {
    (v * 1000.0).round() as i64
}

fn fingerprint(colliders: &[Collider2D], ground: &GroundCandidates) -> Fingerprint {
    let mut c: Vec<String> =
        colliders.iter().map(|c| format!("{:.3},{:.3}..{:.3},{:.3} y{:.2}..{:.2}", c.min.x, c.min.y, c.max.x, c.max.y, c.min_y, c.max_y)).collect();
    c.sort();
    let mut pts: Vec<Vec2> = (-24..=24).step_by(3).flat_map(|z| (-24..=24).step_by(3).map(move |x| Vec2::new(x as f32, z as f32))).collect();
    pts.extend([Vec2::new(0.0, -8.0), Vec2::new(-1.0, -8.0), Vec2::new(1.0, -7.9), Vec2::new(-5.0, -8.0), Vec2::new(5.0, -8.0)]); // on the gate, and on the walls beside it
    let floor_high = pts.iter().map(|p| mm(ground_height_at(ground, *p, 16.0))).collect();
    let terrain = pts.iter().map(|p| ground.terrain_height_at(*p).map_or(i64::MIN, mm)).collect();
    let mut trees: Vec<String> = ground
        .procgen()
        .map(|g| g.colliders_near(Vec2::ZERO, 40.0, &[]).iter().map(|c| format!("{:.2},{:.2}..{:.2},{:.2}", c.min.x, c.min.y, c.max.x, c.max.y)).collect())
        .unwrap_or_default();
    trees.sort();
    Fingerprint { colliders: c, floor_high, terrain, trees }
}

fn physical(scene: &red_engine2::schema::Scene, disabled: &[&str]) -> Fingerprint {
    let w = PhysicalWorld::of(scene, &Default::default(), disabled.iter().copied());
    fingerprint(w.colliders(), w.ground())
}

/// The match behind a server or a `LocalSession`, run until the rule has opened the gate (or not, when `walk_in` is false).
fn match_world(text: &str, walk_in: bool) -> Fingerprint {
    let scene = red_engine2::schema::parse_scene(text).unwrap();
    let spawns = parse_spawns(text).unwrap();
    let mut sim = MatchSim::new(&scene, spawns);
    if walk_in {
        sim.add_player(red_engine2::player::Character::Human).unwrap();
        for k in 1..=100 {
            sim.push_input(0, red_engine2::sim::player::PlayerInput { seq: k, forward: 1, yaw: 0.0, ..Default::default() });
            sim.tick_once();
        }
        assert!(sim.rules().collision_disabled().any(|id| id == "gate"), "walking onto the lever opened the gate");
    }
    let (c, g) = sim.static_world();
    fingerprint(c, g)
}

fn client_world(path: &std::path::Path, open: bool) -> Fingerprint {
    let (_scene, mut world) = ClientWorld::load(path).unwrap();
    if open {
        let gate = world.rule_object_ids.iter().position(|id| id == "gate").expect("the gate is a rule object") as u16;
        world.set_collision_disabled(&[gate]);
    }
    fingerprint(&world.colliders, &world.ground)
}

fn analysis(path: &std::path::Path, phase: Option<&str>) -> (Fingerprint, MapWorld) {
    let w = MapWorld::load_phase(path, phase).unwrap_or_else(|e| panic!("{e:?}"));
    (fingerprint(&w.colliders, &w.ground), w)
}

#[test]
fn the_runtime_and_the_analysis_see_the_same_world_closed_and_open() {
    let text = scene_text("{}");
    let path = write("agree", &text);
    let scene = red_engine2::schema::parse_scene(&text).unwrap();

    let closed = physical(&scene, &[]);
    let open = physical(&scene, &["gate"]);

    // The world under test really is hills and trees, so "terrain still exists" means something.
    let (lo, hi) = closed.terrain.iter().fold((i64::MAX, i64::MIN), |(lo, hi), &h| (lo.min(h), hi.max(h)));
    assert!(hi - lo > 500, "the generated country has hills here: terrain spans {lo}..{hi} mm");
    assert!(closed.trees.len() >= 3, "and trees that block the player: {} found near the pen", closed.trees.len());
    assert_eq!(closed.colliders.len(), 6, "six authored boxes: four walls, two wall halves... and the gate");

    // Opening the gate changes exactly the gate's collider.
    assert_eq!(open.colliders.len(), closed.colliders.len() - 1, "one collider fewer");
    let gone: Vec<&String> = closed.colliders.iter().filter(|c| !open.colliders.contains(c)).collect();
    assert_eq!(gone.len(), 1, "{gone:?}");
    assert!(gone[0].starts_with("-1.500,-8.250..1.500,-7.750"), "it is the gate's footprint: {}", gone[0]);
    assert_eq!(open.terrain, closed.terrain, "the hills are untouched");
    assert_eq!(open.trees, closed.trees, "so are the trees");
    assert_ne!(open.floor_high, closed.floor_high, "the gate's top was standable and is not any more");

    // Every consumer, in both states, is that same world.
    assert_eq!(match_world(&text, false), closed, "server / LocalSession, before the rule");
    assert_eq!(match_world(&text, true), open, "server / LocalSession, after the rule fired");
    assert_eq!(client_world(&path, false), closed, "online client and bots, before");
    assert_eq!(client_world(&path, true), open, "online client and bots, after the server says the gate is open");
    let (a_closed, w_closed) = analysis(&path, None);
    assert_eq!(a_closed, closed, "analysis, initial state");
    assert!(w_closed.is_endless() && w_closed.collision_disabled.is_empty());
    let (a_open, w_open) = analysis(&path, Some("gate_open"));
    assert_eq!(w_open.collision_disabled, ["gate"], "the phase opens the gate and only it");
    assert_eq!(a_open, open, "analysis, phase gate_open: the gate is open and the country is still there");
    assert_eq!(w_open.phase.as_deref(), Some("gate_open"));
    // The question the bug came from: after a phase opened a gate, analysis still has the generated world.
    assert!(w_open.ground.procgen().is_some() && w_open.is_endless());
    assert!(!w_open.blockers_in(Vec2::splat(-20.0), Vec2::splat(20.0)).is_empty());
}

#[test]
fn local_session_agrees_with_the_server_world() {
    let text = scene_text("{}");
    let scene = red_engine2::schema::parse_scene(&text).unwrap();
    let mut session = red_engine2::app::session::LocalSession::from_json(&text).unwrap_or_else(|e| panic!("{e:?}"));
    let (c, g) = session.sim().static_world();
    assert_eq!(fingerprint(c, g), physical(&scene, &[]));
    for _ in 0..100 {
        session.step(red_engine2::sim::player::PlayerInput { forward: 1, yaw: 0.0, ..Default::default() });
    }
    assert!(session.rules().collision_disabled().any(|id| id == "gate"));
    let (c, g) = session.sim().static_world();
    assert_eq!(fingerprint(c, g), physical(&scene, &["gate"]), "the session's world after the rule is the world without the gate, with the hills");
    // And the boy stands on the hill, not on a flat floor.
    let want = g.procgen().unwrap().world().height(session.player().pos.x as f64, session.player().pos.y as f64);
    assert!((session.player().foot_y - want).abs() < 0.05, "feet {} on ground {want}", session.player().foot_y);
}

#[test]
fn the_scenes_own_checks_prove_the_gate_on_generated_ground() {
    // Scripted verification: the gate is a wall until the rule fires; a player who stands on the lever walks out through it over the hills.
    let checks = r#"{
        "lint": {"max_errors": 0},
        "reach": [
            {"to": [0, -14], "reachable": false, "why": "the pen is sealed while the gate is shut"},
            {"to": [0, -14], "phase": "gate_open", "why": "open, the way out is through the gate"}
        ],
        "sim": [{
            "name": "the lever opens the gate and the player walks out over the hills",
            "spawn_group": "",
            "players": [{"id": "p1", "character": "human", "spawn": "start"}],
            "script": [{"player": "p1", "walk": "0,6.5; 0,-14"}],
            "expect": [{"event": "gate_open", "count": 1}, {"collision_disabled": "gate"}, {"player": "p1", "near": [0, -14], "tol": 1.0}]
        }]
    }"#;
    let path = write("checks", &scene_text(checks));
    let report = run(&path, &Options::default()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(report.failed(), 0, "{}", report.render());
    assert!(report.results.len() >= 4, "lint, two reach checks and the play-through all ran:\n{}", report.render());
}

/// Nobody but `collide` assembles a ground from per-object groups: that is how a consumer once forgot the generated world. A new consumer asks `PhysicalWorld`.
#[test]
fn no_module_outside_collide_assembles_ground_from_groups() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut offenders = Vec::new();
    for f in files {
        if f.ends_with("src/collide.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        for needle in ["collect_ground_candidates_grouped_except", "collect_box_colliders_grouped_except", "ground_from_groups", "scene_ground("] {
            if text.contains(needle) {
                offenders.push(format!("{} uses {needle}", f.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "assemble the static world with `collide::PhysicalWorld` (one definition of what exists), not by hand:\n{}",
        offenders.join("\n")
    );
}
