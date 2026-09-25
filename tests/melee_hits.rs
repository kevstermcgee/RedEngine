//! A bat swing only connects with real geometry: swinging through a doorway, at open air or at the
//! sky must not report a hit (which is what plays the impact thunk and flashes the object).
//!
//! The first implementation tested one bounding box per top-level object, so a ray through the
//! front door of `examples/house.json` "hit" the wall group whose box spans the doorway.

use glam::Vec3;
use red_engine2::collide::{collect_interactables, raycast_nearest};
use red_engine2::hit::{collect_hit_shapes, raycast_shapes};
use std::path::Path;

fn house() -> red_engine2::schema::Scene {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/house.json");
    red_engine2::load_scene(&path).unwrap_or_else(|e| panic!("house.json: {e:?}"))
}

const REACH: f32 = 2.2; // MELEE_REACH in re2

#[test]
fn a_swing_through_the_open_front_door_hits_nothing() {
    let scene = house();
    let shapes = collect_hit_shapes(&scene, None);
    // Standing on the porch, eye height, swinging straight in through the 1.2 m door at x = 0.
    let (eye, dir) = (Vec3::new(0.0, 1.5, -1.0), Vec3::Z);
    assert!(raycast_shapes(eye, dir, REACH, &shapes).is_none(), "the doorway is empty");
    // ...which the old bounding-box test got wrong: it called the wall a hit.
    let coarse = collect_interactables(&scene);
    assert!(raycast_nearest(eye, dir, REACH, &coarse).is_some(), "documents the bug this test guards against");
}

#[test]
fn a_swing_at_the_wall_beside_the_door_connects() {
    let scene = house();
    let shapes = collect_hit_shapes(&scene, None);
    let hit = raycast_shapes(Vec3::new(3.0, 1.5, -1.0), Vec3::Z, REACH, &shapes).expect("solid wall at x = 3");
    assert!(scene.objects[hit.object_index].id.starts_with("wall"), "hit {}", scene.objects[hit.object_index].id);
    assert!((hit.distance - 0.88).abs() < 0.2, "the wall face is about a metre away: {}", hit.distance);
}

#[test]
fn a_swing_at_open_air_or_sky_hits_nothing() {
    let scene = house();
    let shapes = collect_hit_shapes(&scene, None);
    let spawn = Vec3::new(0.0, 1.7, -8.0);
    assert!(raycast_shapes(spawn, Vec3::Z, REACH, &shapes).is_none(), "level, into the front yard");
    assert!(raycast_shapes(spawn, Vec3::Y, REACH, &shapes).is_none(), "straight up at the sky");
    // Swinging down at the lawn does touch the ground.
    assert!(raycast_shapes(spawn, -Vec3::Y, REACH, &shapes).is_some());
}

#[test]
fn the_players_own_body_is_never_a_target() {
    let mut scene = house();
    scene.objects.push(red_engine2::characters::rat_object("player_body"));
    let skip = scene.objects.len() - 1;
    let shapes = collect_hit_shapes(&scene, Some(skip));
    assert!(shapes.iter().all(|s| s.object_index != skip));
}
