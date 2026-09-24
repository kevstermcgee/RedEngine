//! The Red Test Lab (`examples/test_lab.json`) is the primary engine-development map: each room
//! exists to exercise one engine system. This file turns its contents into regression tests, so an
//! engine change that breaks stacks, raycast occlusion, clearances or promotion fails here — not in a
//! playtest. (The map's own `checks` block — lint, reach, real-physics walks — runs from `verify`.)

use glam::{Mat4, Vec3};
use red_engine2::hit::{collect_hit_shapes_where, raycast_shapes};
use red_engine2::physics::PropWorld;
use red_engine2::player::{step_horizontal_r, Character};
use red_engine2::schema::Scene;
use red_engine2::tools::verify::{run, Options};
use red_engine2::viewer::collect_box_colliders_except;
use std::path::Path;

fn lab() -> Scene {
    red_engine2::load_scene(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap_or_else(|e| panic!("test_lab.json: {e:?}"))
}

fn index(scene: &Scene, id: &str) -> usize {
    scene.objects.iter().position(|o| o.id == id).unwrap_or_else(|| panic!("no object '{id}'"))
}

fn prop_index(w: &PropWorld, scene: &Scene, id: &str) -> usize {
    w.prop_of_object(index(scene, id)).unwrap_or_else(|| panic!("'{id}' is not a loose prop"))
}

#[test]
fn the_lab_passes_its_own_checks() {
    let report = run(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json"), &Options { skip_views: true, ..Default::default() }).unwrap();
    assert_eq!(report.failed(), 0, "{}", report.render());
}

#[test]
fn the_lab_carries_the_engine_data_the_roadmap_will_parse() {
    // spawns / portals / interest are a schema DRAFT (ADR 0015): the engine ignores them today, but
    // the map must keep carrying them, consistently, so the parser has real data to be built against.
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let rooms: Vec<&str> = v["zones"].as_array().unwrap().iter().filter(|z| z["kind"] == "room").map(|z| z["id"].as_str().unwrap()).collect();
    assert!(v["spawns"].as_array().unwrap().len() >= 4, "multiplayer needs several spawn points");
    for p in v["portals"].as_array().unwrap() {
        for r in p["between"].as_array().unwrap() {
            assert!(rooms.contains(&r.as_str().unwrap()), "portal {} names unknown room {r}", p["id"]);
        }
    }
    assert!(v["interest"]["cell_size"].as_f64().unwrap() > 0.0);
}

#[test]
fn stacked_crates_stand_when_nobody_touches_them_and_topple_when_struck() {
    let mut scene = lab();
    let mut w = PropWorld::new(&scene, None);
    w.set_player(Vec3::new(-20.0, 0.0, 5.0), 0.35, 1.75);
    let stack: Vec<usize> = ["pyr_0_0", "pyr_0_1", "pyr_0_2", "pyr_1_0", "pyr_1_1", "pyr_2_0", "tower_0", "tower_1", "tower_2", "tower_3", "tower_4"]
        .iter()
        .map(|id| prop_index(&w, &scene, id))
        .collect();
    let before: Vec<Vec3> = stack.iter().map(|&p| w.prop_pose(p).w_axis.truncate()).collect();
    for _ in 0..600 {
        w.step();
    }
    for (k, &p) in stack.iter().enumerate() {
        let d = (w.prop_pose(p).w_axis.truncate() - before[k]).length();
        assert!(d < 0.03, "{} drifted {d} m while nobody touched the stacks", scene.objects[w.props()[p].object_index].id);
    }
    assert_eq!(w.dynamic_count(), 0, "an idle lab promotes nothing: untouched props stay static instances");

    // Knock the bottom crate out from under the tower (a hard shove; a single bat swing only slides a crate):
    // the whole tower is promoted, and the top crate falls.
    let (bottom, top) = (prop_index(&w, &scene, "tower_0"), prop_index(&w, &scene, "tower_4"));
    let start_y = w.prop_pose(top).w_axis.y;
    w.strike_impulse(bottom, Vec3::X, w.prop_pose(bottom).w_axis.truncate() + Vec3::Y * 0.1, 150.0);
    for _ in 0..400 {
        w.step();
    }
    w.sync_scene(&mut scene);
    assert!(w.dynamic_count() >= 5, "the shoved crate promoted the whole tower: {}", w.dynamic_count());
    let end_y = w.prop_pose(top).w_axis.y;
    assert!(end_y < start_y - 0.5, "the top crate fell: y {start_y} -> {end_y}");
    assert!(w.is_static(prop_index(&w, &scene, "pyr_0_0")), "and the pyramid across the room was never touched");
}

#[test]
fn hitscan_hits_the_nearest_target_and_cover_hides_what_is_behind_it() {
    let scene = lab();
    let shapes = collect_hit_shapes_where(&scene, |_| true);
    let shooter = Vec3::new(12.0, 1.0, 6.5);
    // Down the lane: the 3 m target is nearest, and the ray reaches it at ~3 m.
    let hit = raycast_shapes(shooter, -Vec3::Z, 80.0, &shapes).expect("a target in the lane");
    assert_eq!(scene.objects[hit.object_index].id, "target_3m");
    assert!((hit.distance - 3.0).abs() < 0.2, "{}", hit.distance);
    // Behind the low wall: aiming at the target behind cover from the firing point hits the cover first.
    let to = Vec3::new(10.0, 1.0, -1.5) - Vec3::new(10.0, 1.0, 6.5);
    let hit = raycast_shapes(Vec3::new(10.0, 1.0, 6.5), to.normalize(), 80.0, &shapes).expect("cover in the way");
    assert_eq!(scene.objects[hit.object_index].id, "cover_wall", "the cover wall must block the shot");
    // Shooting over it (above its 1.0 m height, aiming at the target's top) gets through.
    let eye = Vec3::new(10.0, 1.5, 6.5);
    let to = Vec3::new(10.0, 1.35, -1.5) - eye;
    let hit = raycast_shapes(eye, to.normalize(), 80.0, &shapes).expect("the target is hit");
    assert_eq!(scene.objects[hit.object_index].id, "target_behind_cover");
}

#[test]
fn clearance_gaps_pass_the_characters_they_are_meant_for() {
    let scene = lab();
    let colliders = collect_box_colliders_except(&scene, &Default::default());
    // Walk straight through the wall at x = -12 at each gap's z, west to east, as each character.
    let try_gap = |radius: f32, z: f32| {
        let mut pos = glam::Vec2::new(-13.0, z);
        for _ in 0..240 {
            pos = step_horizontal_r(&colliders, pos, 0.0, glam::Vec2::new(3.2 / 60.0, 0.0), radius);
        }
        pos.x > -11.0
    };
    let (human, rat) = (Character::Human.body().radius, Character::Rat.body().radius);
    // gaps: z = -6.5 (0.25 m), -4.5 (0.6), -2.0 (0.75), 1.0 (0.9), 5.0 (1.5)
    let table = [(-6.5, false, true), (-4.5, false, true), (-2.0, true, true), (1.0, true, true), (5.0, true, true)];
    for (z, human_fits, rat_fits) in table {
        assert_eq!(try_gap(human, z), human_fits, "human (r {human}) at the gap at z = {z}");
        assert_eq!(try_gap(rat, z), rat_fits, "rat (r {rat}) at the gap at z = {z}");
    }
}

#[test]
fn a_prop_on_a_table_falls_with_it_and_lifting_promotes_only_what_it_carried() {
    let scene = lab();
    let mut w = PropWorld::new(&scene, None);
    w.set_player(Vec3::new(-20.0, 0.0, 5.0), 0.35, 1.75);
    let (apple, mug) = (prop_index(&w, &scene, "apple_1"), prop_index(&w, &scene, "mug_1"));
    assert!(w.is_static(apple) && w.is_static(mug));
    // The dining table is a heavy fixture (not loose), so promoting a can beside it must not touch the fruit.
    let can = prop_index(&w, &scene, "can_floor_0");
    w.activate(can);
    assert!(w.is_static(apple) && w.is_static(mug), "an unrelated promotion leaves the table's items static");
    // But picking up the apple itself promotes it, and it comes to rest after being dropped on the floor.
    w.pick_up(apple);
    w.set_held_pose(Mat4::from_translation(Vec3::new(5.6, 1.4, -5.0)));
    w.drop_held(Vec3::ZERO);
    for _ in 0..300 {
        w.step();
    }
    assert!(!w.is_static(apple) && w.is_asleep(apple));
    assert!(w.prop_pose(apple).w_axis.y < 0.8, "the apple rests on the table or floor, not in the air");
}
