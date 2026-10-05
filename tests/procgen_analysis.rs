//! The analysis tools see the world the player walks in.
//!
//! In an endless generated world (`procgen`) the trees and shrubs that stop the player are in no authored collider list. The player's own movement asks the generator
//! for them every tick; reachability, route planning, `lint` and the diagnostics used to ask only the authored list, so they declared open ground where a trunk stood and
//! a "perimeter leak" where there is no perimeter. These tests put one isolated tree in front of one player and check that the abstract answer (`reach`, `walk --auto`,
//! `lint`) and the physically executed one (`walk` on the real per-tick movement) say the same thing. Abstract reachability and physical replay stay separate questions:
//! each is asked here and each is expected to agree, not to be the other.

use glam::Vec2;
use red_engine2::player::PLAYER_RADIUS;
use red_engine2::procgen::{ChunkId, Trunk};
use red_engine2::tools::lint::lint;
use red_engine2::tools::reach::{compute, ReachParams};
use red_engine2::tools::verify::{run, Options};
use red_engine2::tools::walk::walk_from;
use red_engine2::tools::world::MapWorld;
use std::path::PathBuf;

/// A scratch folder for a scene file (unique per call: the tests run in parallel).
fn scratch(name: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("re2_procgen_analysis_{}_{name}_{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An endless-world scene with a spawn at `(x, z)` and these `checks` (a JSON text for the block's contents).
fn write_scene(dir: &std::path::Path, spawn: Vec2, checks: &str) -> PathBuf {
    let path = dir.join("scene.json");
    let text = format!(
        r#"{{"camera": {{"position": [{x}, 1.6, {z}], "target": [{x}, 1.6, {z2}]}}, "procgen": {{"seed": 7}},
            "spawns": [{{"id": "start", "position": [{x}, 0, {z}], "yaw_deg": 0}}], "objects": [], "checks": {checks}}}"#,
        x = spawn.x,
        z = spawn.y,
        z2 = spawn.y - 1.0
    );
    std::fs::write(&path, text).unwrap();
    path
}

/// The blocking trunks of a chunk neighbourhood, from the generator itself.
fn blockers_around(world: &MapWorld, centre: Vec2, chunks: i32) -> Vec<Trunk> {
    let gen = world.ground.procgen().expect("an endless scene").world();
    let c = ChunkId::at(centre.x as f64, centre.y as f64);
    let mut out = Vec::new();
    for z in c.z - chunks..=c.z + chunks {
        for x in c.x - chunks..=c.x + chunks {
            out.extend(gen.blockers(ChunkId { x, z }));
        }
    }
    out
}

/// An isolated blocking tree near the origin with open ground five metres either side of it: `(tree, start, goal)`, the start and goal on one line through the trunk.
fn tree_between_two_clearings() -> (Trunk, Vec2, Vec2) {
    static FOUND: std::sync::OnceLock<(Trunk, Vec2, Vec2)> = std::sync::OnceLock::new();
    *FOUND.get_or_init(find_tree_between_two_clearings)
}

fn find_tree_between_two_clearings() -> (Trunk, Vec2, Vec2) {
    let probe_dir = scratch("probe");
    let probe = MapWorld::load(&write_scene(&probe_dir, Vec2::ZERO, r#"{"lint": {"max_errors": 0}}"#)).expect("the scene loads");
    let all = blockers_around(&probe, Vec2::ZERO, 4);
    for t in all.iter().filter(|t| t.radius >= 0.3) {
        let (c, start, goal) = (Vec2::new(t.x as f32, t.z as f32), Vec2::new(t.x as f32 - 5.0, t.z as f32), Vec2::new(t.x as f32 + 5.0, t.z as f32));
        let clear = |p: Vec2| all.iter().all(|o| (Vec2::new(o.x as f32, o.z as f32) - p).length() > o.radius + PLAYER_RADIUS + 0.4);
        let alone = all.iter().filter(|o| (Vec2::new(o.x as f32, o.z as f32) - c).length() < 6.0).count() == 1;
        if alone && clear(start) && clear(goal) {
            let _ = std::fs::remove_dir_all(&probe_dir);
            return (*t, start, goal);
        }
    }
    panic!("no isolated tree with clearings either side near the origin of seed 7");
}

#[test]
fn reachability_sees_the_trunk_the_player_collides_with() {
    let (tree, start, goal) = tree_between_two_clearings();
    let dir = scratch("reach");
    let world = MapWorld::load(&write_scene(&dir, start, r#"{"lint": {"max_errors": 0}}"#)).unwrap();
    assert!(world.is_endless());
    let centre = Vec2::new(tree.x as f32, tree.z as f32);
    let reach = compute(&world, &ReachParams { radius: Some(24.0), include: vec![goal], ..Default::default() });
    assert!(reach.start_ok);
    assert!(reach.levels_at(centre).is_empty(), "the trunk's own cell is not somewhere the player can stand");
    // Just inside the body's reach of the trunk is blocked, just outside it is not: the same line the player's collision draws.
    let touching = centre + Vec2::new(-(tree.radius + PLAYER_RADIUS - 0.1), 0.0);
    let clear = centre + Vec2::new(-(tree.radius + PLAYER_RADIUS + 0.2), 0.0);
    assert!(reach.levels_at(touching).is_empty() && !reach.levels_at(clear).is_empty(), "blocked at {touching:?}, free at {clear:?}");
    assert!(!reach.levels_at(goal).is_empty(), "the far side is reachable by going round");
    assert!(reach.leaks.is_empty(), "the edge of an endless world's window is not a leak");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The physical replay on the same scene: walking straight at the trunk stops at it (it does not pass through), exactly where the analysis said there is no floor.
#[test]
fn a_straight_walk_stops_at_the_tree_where_the_analysis_said_it_would() {
    let (tree, start, goal) = tree_between_two_clearings();
    let dir = scratch("walk");
    let world = MapWorld::load(&write_scene(&dir, start, r#"{"lint": {"max_errors": 0}}"#)).unwrap();
    let legs = walk_from(&world, start, 0.0, &[goal]);
    assert!(!legs[0].reached, "a straight line through a trunk does not arrive");
    assert!(
        legs[0].pos.x < tree.x as f32 - tree.radius,
        "the walker stopped short of the trunk (x {}, trunk edge {}): {:?}",
        legs[0].pos.x,
        tree.x as f32 - tree.radius,
        legs[0]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The abstract plan and the physical replay agree end to end: `verify` plans a route to the far side, replays it on the real movement, and a `reach` check that
/// the trunk's own point is unreachable holds. A planner that ignored the trunk would plan the straight line and fail the replay.
#[test]
fn the_planned_route_goes_round_the_tree_and_the_replay_arrives() {
    let (tree, start, goal) = tree_between_two_clearings();
    let dir = scratch("verify");
    let checks = format!(
        r#"{{"reach": [{{"to": [{tx}, {tz}], "why": "the tree itself", "reachable": false}}, {{"to": [{gx}, {gz}], "why": "the far side"}}],
            "walk": [{{"name": "round the tree", "to": [{gx}, {gz}], "auto": true}}]}}"#,
        tx = tree.x,
        tz = tree.z,
        gx = goal.x,
        gz = goal.y
    );
    let report = run(&write_scene(&dir, start, &checks), &Options::default()).expect("verify runs");
    let rows: Vec<String> = report.results.iter().map(|r| format!("{} {} {}", if r.ok { "PASS" } else { "FAIL" }, r.name, r.detail)).collect();
    assert!(report.results.iter().all(|r| r.ok) && report.results.len() == 3, "{rows:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_endless_world_has_no_perimeter_to_leak_through() {
    let dir = scratch("lint");
    let world = MapWorld::load(&write_scene(&dir, Vec2::ZERO, r#"{"lint": {"max_errors": 0}}"#)).unwrap();
    let reach = compute(&world, &ReachParams::default());
    let found: Vec<String> = lint(&world, &reach).iter().map(|f| format!("{}: {}", f.code, f.message)).collect();
    assert!(!found.iter().any(|f| f.starts_with("leak")), "{found:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_shared_query_returns_what_the_player_collides_with_and_nothing_visual() {
    let dir = scratch("query");
    let world = MapWorld::load(&write_scene(&dir, Vec2::ZERO, r#"{"lint": {"max_errors": 0}}"#)).unwrap();
    let (lo, hi) = (Vec2::splat(-20.0), Vec2::splat(20.0));
    let colliders = world.blockers_in(lo, hi);
    assert!(!colliders.is_empty(), "there are trees within 20 m of the origin of seed 7");
    // Every generated blocker in the box is in the answer, at the same place and size the generator reports.
    let physical = blockers_around(&world, Vec2::ZERO, 2);
    for t in physical.iter().filter(|t| (t.x.abs() < 18.0) && (t.z.abs() < 18.0)) {
        let found = colliders.iter().any(|c| {
            let mid = (c.min + c.max) * 0.5;
            (mid - Vec2::new(t.x as f32, t.z as f32)).length() < 1e-3 && ((c.max.x - c.min.x) * 0.5 - t.radius).abs() < 1e-3
        });
        assert!(found, "the player collides with {t:?} but the analysis query lacks it");
    }
    // Grass and flowers are drawn and never block: the query holds a tree-sized handful, not thousands of blades.
    assert!(colliders.len() < 4 * physical.len().max(1), "{} colliders for {} blockers", colliders.len(), physical.len());
    // A scene with no generated world answers with its own list, borrowed (no copy, no change in behaviour).
    let plain = dir.join("plain.json");
    std::fs::write(&plain, r#"{"camera": {"position": [0, 1.6, 3], "target": [0, 1.6, 0]}, "objects": [{"id": "wall", "type": "box", "size": [4, 2, 0.3], "position": [0, 1, 0]}]}"#).unwrap();
    let bounded = MapWorld::load(&plain).unwrap();
    assert!(!bounded.is_endless());
    assert!(matches!(bounded.blockers_in(lo, hi), std::borrow::Cow::Borrowed(_)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The scenes the engine ships as its endless-world examples lint clean: they used to report a perimeter "leak" in a world that has no perimeter.
#[test]
fn the_shipped_endless_scenes_lint_clean() {
    for scene in ["examples/marcel/marcel.json", "examples/endless_meadow.json"] {
        let world = MapWorld::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(scene)).unwrap_or_else(|e| panic!("{scene}: {e:?}"));
        assert!(world.is_endless(), "{scene}");
        let reach = compute(&world, &ReachParams::default());
        let findings = lint(&world, &reach);
        let errors: Vec<String> =
            findings.iter().filter(|f| matches!(f.sev, red_engine2::tools::lint::Severity::Error)).map(|f| format!("{}: {}", f.code, f.message)).collect();
        assert!(errors.is_empty(), "{scene}: {errors:?}");
        assert!(reach.total_area() > 1000.0, "{scene}: the analysis window holds real ground ({} m2)", reach.total_area());
    }
}

/// A zone authored at y = 0 over ground that is somewhere else is never entered; the lint says where the ground is instead of "is its rect/y right?".
#[test]
fn a_zone_at_the_wrong_height_names_the_height_of_the_ground_under_it() {
    let (tree, start, _) = tree_between_two_clearings();
    let dir = scratch("zone");
    let path = write_scene(&dir, start, r#"{"lint": {"max_errors": 0}}"#);
    let world = MapWorld::load(&path).unwrap();
    let ground = world.ground.procgen().expect("endless").height(start + Vec2::new(0.0, 3.0));
    // A 2 m square clear of the tree, a metre above (wrong) and at the ground (right).
    let (lo, hi) = (start + Vec2::new(-1.0, 2.0), start + Vec2::new(1.0, 4.0));
    let zone = |y: f32| {
        let text = std::fs::read_to_string(&path).unwrap().replace(
            "\"objects\": []",
            &format!("\"zones\": [{{\"id\": \"z\", \"rect\": [{}, {}, {}, {}], \"y\": {y}}}], \"objects\": []", lo.x, lo.y, hi.x, hi.y),
        );
        let p = dir.join(format!("zone_{y}.json"));
        std::fs::write(&p, text).unwrap();
        let w = MapWorld::load(&p).unwrap();
        let reach = compute(&w, &ReachParams { radius: Some(24.0), ..Default::default() });
        lint(&w, &reach).iter().filter(|f| f.code == "zone").map(|f| f.message.clone()).collect::<Vec<_>>()
    };
    let wrong = zone(ground + 1.0);
    assert!(
        wrong.len() == 1 && wrong[0].contains("is never entered") && wrong[0].contains(&format!("y={ground:.1}")) && wrong[0].contains("set its \"y\""),
        "{wrong:?}"
    );
    assert_eq!(zone(ground), Vec::<String>::new(), "at the ground's own height the zone is fine");
    let _ = (tree, std::fs::remove_dir_all(&dir));
}

/// An unreachable point says what covers it and where the player can stand instead, so the thing placed there can be moved without exploring.
#[test]
fn an_unreachable_point_says_what_covers_it_and_the_nearest_place_to_stand() {
    use red_engine2::tools::pathing::unreachable_why;
    let (tree, start, _) = tree_between_two_clearings();
    let dir = scratch("why");
    let world = MapWorld::load(&write_scene(&dir, start, r#"{"lint": {"max_errors": 0}}"#)).unwrap();
    let reach = compute(&world, &ReachParams { radius: Some(24.0), ..Default::default() });
    let centre = Vec2::new(tree.x as f32, tree.z as f32);
    let why = unreachable_why(&world, &reach, centre);
    assert!(why.contains("a generated tree or shrub covers it") && why.contains("the nearest place the player can stand and reach is"), "{why}");
    assert!(why.contains(" at y="), "the nearest spot comes with its height: {why}");
    assert_eq!(unreachable_why(&world, &reach, start), "", "a reachable point needs no explanation");
    // Far outside the analysed window nothing reachable is near: cut off, not covered.
    let far = unreachable_why(&world, &reach, start + Vec2::new(200.0, 0.0));
    assert!(far.contains("nothing the player can reach is within 8 m"), "{far}");
    // An authored object covers its own spot, and is named.
    let boxed = dir.join("boxed.json");
    let text = std::fs::read_to_string(dir.join("scene.json")).unwrap().replace(
        "\"objects\": []",
        &format!(
            "\"objects\": [{{\"id\": \"altar\", \"type\": \"box\", \"size\": [1, 1, 1], \"position\": [{}, {}, {}]}}]",
            start.x,
            world.ground.procgen().unwrap().height(start + Vec2::new(0.0, 3.0)) + 0.5,
            start.y + 3.0
        ),
    );
    std::fs::write(&boxed, text).unwrap();
    let w2 = MapWorld::load(&boxed).unwrap();
    let r2 = compute(&w2, &ReachParams { radius: Some(24.0), ..Default::default() });
    let why = unreachable_why(&w2, &r2, start + Vec2::new(0.0, 3.0));
    assert!(why.contains("'altar'"), "{why}");
    let _ = std::fs::remove_dir_all(&dir);
}
