//! Loose-prop physics tests: picking, carrying, dropping, knocking things over, and per-holder carry state.

use super::*;

fn scene(objects: &str) -> Scene {
    let text = format!(
        r##"{{"camera":{{"position":[0,1.7,5],"target":[0,1,0]}},"objects":[
            {{"id":"floor","type":"box","position":[0,-0.1,0],"size":[40,0.2,40],"material":{{"color":"#888888"}}}},
            {objects}]}}"##
    );
    crate::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}"))
}

fn settle(w: &mut PropWorld, scene: &mut Scene, ticks: usize) {
    for _ in 0..ticks {
        w.step();
    }
    w.sync_scene(scene);
}

fn pos(scene: &Scene, id: &str) -> Vec3 {
    scene.objects.iter().find(|o| o.id == id).unwrap().position.sample(0.0)
}

fn up_y(scene: &Scene, id: &str) -> f32 {
    let o = scene.objects.iter().find(|o| o.id == id).unwrap();
    let r = o.rotation.sample(0.0);
    (Quat::from_euler(EulerRot::XYZ, r.x.to_radians(), r.y.to_radians(), r.z.to_radians()) * Vec3::Y).y
}

#[test]
fn classification_follows_what_a_person_can_lift() {
    let s = scene(
        r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}},
            {"id":"fridge","type":"prop","prop":"refrigerator","position":[3,0,0],"material":{"color":"#dddddd"}},
            {"id":"sofa","type":"prop","prop":"sofa","position":[5,0,0],"material":{"color":"#dddddd"}},
            {"id":"toilet","type":"prop","prop":"toilet","position":[7,0,0],"material":{"color":"#ffffff"}},
            {"id":"apple","type":"prefab","prefab":"apple_red","position":[0,0,3]},
            {"id":"tree","type":"prop","prop":"tree_oak","position":[9,0,0],"material":{"color":"#336633"}},
            {"id":"pinned","type":"prop","prop":"chair","position":[11,0,0],"movable":false,"material":{"color":"#336633"}}"##,
    );
    let by = |id: &str| classify(s.objects.iter().find(|o| o.id == id).unwrap());
    assert!(by("crate").is_some() && by("apple").is_some());
    assert!(by("fridge").is_none() && by("sofa").is_none() && by("toilet").is_none() && by("tree").is_none());
    assert!(by("pinned").is_none(), "movable:false wins");
    assert!(by("apple").unwrap().carriable(&RAT_CARRY), "a rat can carry an apple");
    assert!(!by("crate").unwrap().carriable(&RAT_CARRY), "but not a crate");
    assert!(by("crate").unwrap().carriable(&HUMAN_CARRY));
}

#[test]
fn a_dropped_crate_falls_and_comes_to_rest_on_the_floor() {
    let mut s = scene(r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}}"##);
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(4.0, 0.0, 4.0), 0.35, 1.75);
    w.pick_up(0);
    w.set_held_pose(Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0)));
    w.sync_scene(&mut s);
    assert!((pos(&s, "crate").y - 1.5).abs() < 1e-4, "carried object follows the hold pose");
    w.drop_held(Vec3::ZERO);
    settle(&mut w, &mut s, 240);
    let p = pos(&s, "crate");
    assert!(p.y.abs() < 0.03, "rests on the floor (origin at its base), got y = {}", p.y);
    assert!(up_y(&s, "crate") > 0.99, "and is still upright");
    assert!(w.is_asleep(0), "at rest it sleeps again");
}

#[test]
fn a_shoved_crate_topples_a_fire_extinguisher() {
    let mut s = scene(
        r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}},
            {"id":"cone","type":"prop","prop":"fire_extinguisher","position":[0.9,0,0],"material":{"color":"#cc2222"}}"##,
    );
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(6.0, 0.0, 6.0), 0.35, 1.75);
    let cone_before = pos(&s, "cone");
    // Let go of the crate just above the floor while moving toward the cone (a shove / a throw).
    w.pick_up(0);
    w.set_held_pose(Mat4::from_translation(Vec3::new(-0.2, 0.15, 0.0)));
    w.drop_held(Vec3::new(4.0, 0.0, 0.0));
    settle(&mut w, &mut s, 300);
    let moved = (pos(&s, "cone") - cone_before).length();
    assert!(moved > 0.2 || up_y(&s, "cone") < 0.8, "the cone was knocked (moved {moved}, up {})", up_y(&s, "cone"));
    assert!(pos(&s, "cone").y > -0.05, "and it did not fall through the floor");
}

#[test]
fn a_dropped_crate_lands_on_and_pushes_a_small_apple_off_a_table() {
    // A crate stands in for a table top at 0.56 m; the apple sits on its edge.
    let mut s = scene(
        r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"movable":false,"material":{"color":"#a07040"}},
            {"id":"apple","type":"prefab","prefab":"apple_red","position":[0.26,0.56,0]},
            {"id":"box2","type":"prop","prop":"box_stack","position":[0.0,0,3],"material":{"color":"#a07040"}}"##,
    );
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(6.0, 0.0, 6.0), 0.35, 1.75);
    let apple = w.prop_of_object(2).expect("the apple is loose");
    assert!(w.is_asleep(apple), "it sits still until touched");
    // Bump the apple's neighbour: a box dropped beside it rolls it off the edge.
    let b = w.prop_of_object(3).expect("box_stack is loose");
    w.pick_up(b);
    w.set_held_pose(Mat4::from_translation(Vec3::new(0.62, 0.7, 0.0)));
    w.drop_held(Vec3::new(-1.5, 0.0, 0.0));
    settle(&mut w, &mut s, 300);
    let a = pos(&s, "apple");
    assert!(a.y < 0.3, "the apple ended up on the floor, not still on the ledge: {a:?}");
}

#[test]
fn a_bat_hit_sends_a_light_prop_flying_and_only_shuffles_a_heavy_one() {
    let mut s = scene(
        r##"{"id":"apple","type":"prefab","prefab":"apple_red","position":[0,0,0]},
            {"id":"crate","type":"prop","prop":"crate","position":[3,0,0],"material":{"color":"#a07040"}}"##,
    );
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(9.0, 0.0, 9.0), 0.35, 1.75);
    let (apple, crate_) = (w.prop_of_object(1).unwrap(), w.prop_of_object(2).unwrap());
    w.strike(apple, Vec3::X, Vec3::new(-0.03, 0.05, 0.0));
    w.strike(crate_, Vec3::X, Vec3::new(2.7, 0.3, 0.0));
    for _ in 0..90 {
        w.step();
    }
    w.sync_scene(&mut s);
    let (a, c) = (pos(&s, "apple").x, pos(&s, "crate").x - 3.0);
    assert!(a > 1.0, "the apple flew: {a}");
    assert!(c > 0.0 && c < a, "the crate only shuffled: {c} vs {a}");
}

#[test]
fn rays_find_dormant_props_too_so_a_bat_or_bullet_can_hit_something_nobody_has_touched() {
    let s = scene(r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,-3],"material":{"color":"#a07040"}}"##);
    let w = PropWorld::new(&s, None);
    assert!(w.is_asleep(0), "untouched, so still dormant");
    let hit = w.ray_props(Vec3::new(0.0, 0.3, 0.0), -Vec3::Z, 10.0).expect("a ray finds the dormant crate");
    assert_eq!(hit.0, 0);
    assert!((hit.1 - 2.72).abs() < 0.05, "distance to its near face: {}", hit.1);
}

#[test]
fn walking_into_a_small_prop_pushes_it() {
    let mut s = scene(r##"{"id":"cone","type":"prop","prop":"traffic_cone","position":[0,0,0],"material":{"color":"#ff6a00"}}"##);
    let mut w = PropWorld::new(&s, None);
    for i in 0..90 {
        let x = -1.0 + i as f32 * 0.02;
        w.set_player(Vec3::new(x, 0.0, 0.0), 0.35, 1.75);
        w.step();
    }
    w.sync_scene(&mut s);
    assert!(pos(&s, "cone").x > 0.2, "the cone was shoved along by the walker: {:?}", pos(&s, "cone"));
}

#[test]
fn a_wall_hides_a_prop_from_the_pick_up_ray_and_a_held_prop_is_not_pickable() {
    let s = scene(
        r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,-3],"material":{"color":"#a07040"}},
            {"id":"wall","type":"box","position":[0,1,-1.5],"size":[6,2,0.2],"material":{"color":"#dddddd"}}"##,
    );
    let mut w = PropWorld::new(&s, None);
    let eye = Vec3::new(0.0, 0.5, 0.0);
    assert!(w.pick_target(eye, -Vec3::Z, 5.0, &HUMAN_CARRY).is_none(), "wall in the way");
    assert!(w.pick_target(Vec3::new(0.0, 0.5, -2.0), -Vec3::Z, 5.0, &HUMAN_CARRY).is_some(), "clear line of sight");
    w.pick_up(0);
    w.step(); // disabling a body takes effect on the next step
    assert!(w.pick_target(Vec3::new(0.0, 0.5, -2.0), -Vec3::Z, 5.0, &HUMAN_CARRY).is_none(), "already carrying");
    assert!(w.ray_props(Vec3::new(0.0, 0.5, -2.0), -Vec3::Z, 5.0).is_none(), "a carried prop can't be batted");
}

// ---- static instances and promotion (ADR 0014, step 4) ----------------------------------------

const THREE_APART: &str = r##"{"id":"a","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}},
    {"id":"b","type":"prop","prop":"crate","position":[6,0,0],"material":{"color":"#a07040"}},
    {"id":"c","type":"prop","prop":"barrel","position":[0,0,6],"material":{"color":"#3a6ea5"}}"##;

#[test]
fn untouched_props_have_no_body_and_no_entity() {
    let s = scene(THREE_APART);
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
    for _ in 0..120 {
        w.step();
    }
    assert_eq!(w.props().len(), 3);
    assert!((0..3).all(|i| w.is_static(i) && w.entity_of(i).is_none() && w.is_asleep(i)));
    assert_eq!(w.dynamic_count(), 0);
    assert_eq!(w.entities().len(), 0, "no entities for static props");
    assert_eq!(w.body_count(), 1, "only the player's body exists");
}

#[test]
fn touching_one_prop_promotes_only_that_one() {
    let s = scene(THREE_APART);
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
    w.activate(1);
    assert_eq!((w.dynamic_count(), w.body_count(), w.entities().len()), (1, 2, 1));
    assert!(!w.is_static(1) && w.is_static(0) && w.is_static(2));
    assert_eq!(w.entity_of(1).map(|e| e.slot()), Some(0));
    w.activate(1); // idempotent
    assert_eq!((w.dynamic_count(), w.body_count()), (1, 2));
}

#[test]
fn a_promoted_prop_is_where_it_was_and_still_solid_and_hittable() {
    let mut s = scene(THREE_APART);
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
    let before = w.prop_pose(1);
    w.activate(1);
    assert!(before.abs_diff_eq(w.prop_pose(1), 1e-6), "promotion must not move it");
    for _ in 0..90 {
        w.step();
    }
    w.sync_scene(&mut s);
    assert!((pos(&s, "b") - Vec3::new(6.0, 0.0, 0.0)).length() < 0.02, "it rests where it was authored: {:?}", pos(&s, "b"));
    assert!(w.is_asleep(1), "and falls asleep again");
    let hit = w.ray_props(Vec3::new(6.0, 0.3, 4.0), -Vec3::Z, 10.0).expect("a ray still hits the promoted crate");
    assert_eq!(hit.0, 1);
    let hit = w.ray_props(Vec3::new(0.0, 0.3, 4.0), -Vec3::Z, 10.0).expect("and a static one");
    assert_eq!(hit.0, 0);
}

#[test]
fn promoting_a_table_promotes_what_rests_on_it() {
    let s = scene(
        r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"movable":true,"material":{"color":"#a07040"}},
            {"id":"apple","type":"prefab","prefab":"apple_red","position":[0,0.56,0]},
            {"id":"far","type":"prop","prop":"crate","position":[8,0,0],"material":{"color":"#a07040"}}"##,
    );
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
    let crate_id = w.prop_of_object(1).unwrap();
    w.activate(crate_id);
    assert_eq!(w.dynamic_count(), 2, "the crate and its apple");
    assert!(w.is_static(w.prop_of_object(3).unwrap()), "but not the crate across the room");
}

#[test]
fn sync_scene_never_touches_static_props_and_writes_moved_ones_once() {
    let mut s = scene(THREE_APART);
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
    // Sentinel positions: sync must leave the static ones alone.
    for id in ["a", "c"] {
        let o = s.objects.iter_mut().find(|o| o.id == id).unwrap();
        o.position = Track::constant(Vec3::splat(123.0));
    }
    w.strike_impulse(1, Vec3::X, Vec3::new(6.0, 0.3, 0.0), 6.0);
    w.step();
    w.sync_scene(&mut s);
    assert_eq!(pos(&s, "a"), Vec3::splat(123.0));
    assert_eq!(pos(&s, "c"), Vec3::splat(123.0));
    assert!((pos(&s, "b") - Vec3::new(6.0, 0.0, 0.0)).length() > 0.0, "the struck crate moved");
    // With nothing moving, a second sync writes nothing (it does not overwrite a sentinel).
    for _ in 0..600 {
        w.step();
    }
    w.sync_scene(&mut s);
    let o = s.objects.iter_mut().find(|o| o.id == "b").unwrap();
    o.position = Track::constant(Vec3::splat(-9.0));
    w.sync_scene(&mut s);
    assert_eq!(pos(&s, "b"), Vec3::splat(-9.0), "a sleeping prop is not rewritten");
}

#[test]
fn a_sleeping_promoted_prop_publishes_its_resting_pose_then_goes_quiet() {
    use crate::sim::change::ChangeCursor;
    let s = scene(THREE_APART);
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
    w.pick_up(1);
    w.set_held_pose(Mat4::from_translation(Vec3::new(6.0, 1.0, 0.0)));
    w.drop_held(Vec3::ZERO);
    for _ in 0..300 {
        w.step();
    }
    assert!(w.is_asleep(1));
    let slot = w.entity_of(1).unwrap().slot();
    let published = *w.entities().transforms.get(slot);
    let actual = w.prop_pose(1).to_scale_rotation_translation();
    assert!(published.position.abs_diff_eq(actual.2, 1e-5) && published.rotation.abs_diff_eq(actual.1, 1e-5), "the final pose was published");
    // A network-style reader sees it once, then nothing while the prop sleeps.
    let mut cursor = ChangeCursor::default();
    assert!(w.entities().transforms.changed_since(cursor.last()));
    cursor.catch_up(w.clock_mut());
    for _ in 0..60 {
        w.step();
    }
    assert!(!w.entities().transforms.changed_since(cursor.last()), "a sleeping prop generates no changes");
}

// ---- carrying: where you look decides how high the held prop rides ----------------------------

#[test]
fn looking_up_lifts_a_carried_prop_and_looking_down_lowers_it() {
    let s = scene(r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}}"##);
    let w = PropWorld::new(&s, None);
    let eye = Vec3::new(0.0, 1.7, 3.0);
    let base_y = |look: Vec3| w.hold_pose(0, eye, look, 0.35, 0.55, 0.0).w_axis.y;
    let level = base_y(Vec3::new(0.0, 0.0, -1.0));
    let up45 = base_y(Vec3::new(0.0, 1.0, -1.0));
    let overhead = base_y(Vec3::Y);
    let down = base_y(Vec3::new(0.0, -1.0, -0.4));
    assert!((level - (1.7 - 0.55 - w.props()[0].shape.extents.y * 0.5)).abs() < 0.05, "level gaze keeps the old carry height: {level}");
    assert!(up45 > level + 0.3, "looking up 45 degrees lifts it: {level} -> {up45}");
    assert!(overhead > up45, "straight up is higher still: {overhead}");
    assert!(overhead > 2.0, "high enough to hold well over your head: {overhead}");
    assert!(down < level, "looking down lowers it");
    // Looking straight up puts it over the player, not out in front.
    let over = w.hold_pose(0, eye, Vec3::Y, 0.35, 0.55, 0.0).w_axis;
    assert!((over.z - eye.z).abs() < 0.3, "overhead, not out in front: z {}", over.z);
}

#[test]
fn a_ceiling_stops_a_lifted_prop_and_the_floor_stops_a_lowered_one() {
    let s = scene(
        r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}},
            {"id":"ceil","type":"box","position":[0,2.7,3],"size":[8,0.2,8],"material":{"color":"#888888"}}"##,
    );
    let w = PropWorld::new(&s, None);
    let ext = w.props()[0].shape.extents.y;
    let eye = Vec3::new(0.0, 1.7, 3.0);
    let top = w.hold_pose(0, eye, Vec3::Y, 0.35, 0.55, 0.0).w_axis.y + ext;
    assert!(top <= 2.6, "the ceiling is at 2.6 m; the crate's top is at {top}");
    let low = w.hold_pose(0, eye, Vec3::new(0.0, -1.0, -0.1), 0.35, 0.55, 0.0).w_axis.y;
    assert!(low >= -0.001, "never below the floor: {low}");
}

#[test]
fn a_player_can_stand_on_a_crate_and_jump_off_it() {
    let s = scene(r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0]}"##);
    let mut w = PropWorld::new(&s, None);
    w.set_player(Vec3::new(9.0, 0.0, 9.0), 0.35, 1.75);
    w.step();
    assert!(w.floor_under(glam::Vec2::ZERO, 0.0, 0.35, 0.35).is_none(), "a 0.56 m crate is too tall to step onto from the ground: it takes a jump");
    let top = w.floor_under(glam::Vec2::new(0.0, 0.0), 0.3, 0.35, 0.35).expect("mid-jump the crate top is a floor");
    assert!((top - 0.56).abs() < 0.05, "crate top {top}");
    assert!(w.floor_under(glam::Vec2::new(5.0, 5.0), 0.0, 0.35, 0.35).is_none(), "nothing under bare floor");
    // Standing on it (feet at the top) still finds it, and a jump rises off it.
    let ground = crate::collide::GroundCandidates::default();
    let mut y = top;
    let (mut vy, mut peak) = (0.0f32, y);
    let floor = |y: f32| w.floor_under(glam::Vec2::ZERO, y, 0.35, 0.35);
    for tick in 0..30 {
        let (ny, nvy) = crate::player::vertical_step_on(&ground, floor(y), glam::Vec2::ZERO, y, vy, tick == 0);
        (y, vy) = (ny, nvy);
        peak = peak.max(y);
    }
    assert!(peak > top + 0.3, "jumped off the crate top: peak {peak} vs top {top}");
    assert!((y - top).abs() < 0.02, "and lands back on it");
}
