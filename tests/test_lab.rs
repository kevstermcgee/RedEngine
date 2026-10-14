//! The Red Test Lab (`examples/test_lab.json`) is the primary engine-development map: each room
//! exists to exercise one engine system. This file turns its contents into regression tests, so an
//! engine change that breaks stacks, raycast occlusion, clearances or promotion fails here — not in a
//! playtest. (The map's own `checks` block — lint, reach, real-physics walks — runs from `verify`.)

use glam::{Mat4, Vec3};
use red_engine2::collide::{collect_box_colliders_except, collect_ground_candidates_except};
use red_engine2::hit::{collect_hit_shapes_where, raycast_shapes};
use red_engine2::physics::PropWorld;
use red_engine2::player::{step_horizontal_r, Character};
use red_engine2::schema::Scene;
use red_engine2::tools::verify::{run, Options};
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

#[test]
fn the_rat_runs_under_tables_and_platforms_but_a_human_cannot() {
    use red_engine2::sim::player::{step_player, PlayerInput, PlayerState};
    let scene = lab();
    let loose: std::collections::HashSet<usize> = red_engine2::physics::loose_props(&scene, None).iter().map(|(i, _)| *i).collect();
    let (colliders, ground) = (collect_box_colliders_except(&scene, &loose), collect_ground_candidates_except(&scene, &loose));
    let run = |who: Character, x: f32, z: f32, yaw_deg: f32, ticks: u32| {
        let mut s = PlayerState::spawn(x, z, 0.0, yaw_deg, who);
        for k in 0..ticks {
            let yaw = s.yaw;
            step_player(&mut s, &PlayerInput { seq: k, forward: 1, yaw, ..Default::default() }, &colliders, &ground);
        }
        s
    };
    // The dining table (table_a) spans x 5.2..6.8, z -6.45..-5.55 with a top 0.78 m up. Walk north through
    // the middle of it, starting south of it.
    let rat = run(Character::Rat, 6.0, -4.5, 0.0, 120);
    assert!(rat.pos.y < -6.7, "the rat ran right under the table and out the other side: z = {}", rat.pos.y);
    assert_eq!(rat.foot_y, 0.0, "and stayed on the floor");
    let human = run(Character::Human, 6.0, -4.5, 0.0, 120);
    assert!(human.pos.y > -5.3, "a person is stopped by the tabletop: z = {}", human.pos.y);

    // The platform (a slab 1.8..2.0 m up over x -7.88..-4.4, z 1.4..3.2): run west underneath it from the east.
    let rat = run(Character::Rat, -3.5, 2.3, 270.0, 100);
    assert!(rat.pos.x < -6.5, "the rat ran under the platform: x = {}", rat.pos.x);
    let human = run(Character::Human, -3.5, 2.3, 270.0, 100);
    assert!(human.pos.x > -4.1, "a person cannot walk under it: x = {}", human.pos.x);

    // Legs still count: aimed straight at one of the table's legs, the rat is stopped by it.
    let leg = red_engine2::props::game_collision_boxes(red_engine2::props::PropKind::DiningTable)
        .into_iter()
        .filter(|(lo, _)| lo.y < 0.05)
        .max_by(|a, b| ((a.0.z + a.1.z) / 2.0).total_cmp(&((b.0.z + b.1.z) / 2.0)))
        .expect("a leg");
    let (leg_x, leg_z) = (6.0 + (leg.0.x + leg.1.x) / 2.0, -6.0 + (leg.0.z + leg.1.z) / 2.0); // table_a is at (6, -6), unrotated
    let boxed = run(Character::Rat, leg_x, leg_z + 1.5, 0.0, 120);
    assert!(boxed.pos.y > leg_z, "a rat aimed at a table leg at ({leg_x:.2}, {leg_z:.2}) is stopped by it: z = {}", boxed.pos.y);
}

/// The Test Lab's spawn points (moved here from `sim::spawns` when it moved into the core: the map is the engine's content, the parser is the core's).
#[test]
fn the_test_lab_has_four_duel_spawns_facing_each_other_and_props_spawns() {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
    let s = red_engine2::sim::spawns::parse_spawns(&text).unwrap();
    assert!(s.len() >= 6);
    let a = s.iter().find(|s| s.id == "spawn_a").unwrap();
    let b = s.iter().find(|s| s.id == "spawn_b").unwrap();
    // a faces +X (yaw 90), b faces -X (yaw 270): they look at each other across the hall.
    assert!(a.position[0] < b.position[0] && (a.yaw_deg - 90.0).abs() < 1e-3 && (b.yaw_deg - 270.0).abs() < 1e-3);
    assert!(s.iter().any(|s| s.group == "props"));
}

/// `sim::player` on the Test Lab: moved from the module's own tests when `sim::player` moved into the core (the map is the engine's content).
mod sim_player {
    use glam::Vec2;
    use red_engine2::collide::{collect_box_colliders, collect_ground_candidates, Collider2D, GroundCandidates};
    use red_engine2::player::{Character, JumpPad, PlayerTuning};
    use red_engine2::sim::player::*;
    use std::path::Path;

    #[test]
    fn analog_strength_and_diagonal_speed_are_bounded() {
        let tuning = PlayerTuning::default();
        let mut half = PlayerState::spawn(0.0, 0.0, 0.0, 0.0, Character::Human);
        let mut full = half;
        let mut diagonal = half;
        let ground = red_engine2::collide::collect_ground_candidates(&red_engine2::schema::parse_scene(include_str!("../examples/test_lab.json")).unwrap());
        let a = PlayerInput { analog: true, forward: 64, ..Default::default() };
        let b = PlayerInput { forward: 1, ..Default::default() };
        let c = PlayerInput { analog: true, forward: 127, strafe: 127, ..Default::default() };
        step_player_tuned(&mut half, &a, &[], &ground, tuning, &[]);
        step_player_tuned(&mut full, &b, &[], &ground, tuning, &[]);
        step_player_tuned(&mut diagonal, &c, &[], &ground, tuning, &[]);
        assert!((half.pos.length() / full.pos.length() - 64.0 / 127.0).abs() < 0.001);
        assert!((diagonal.pos.length() - full.pos.length()).abs() < 0.001);
        assert_eq!(a.with_flags(a.flags()), a);
    }
    fn world() -> (Vec<Collider2D>, GroundCandidates) {
        let scene = red_engine2::load_scene(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
        (collect_box_colliders(&scene), collect_ground_candidates(&scene))
    }

    fn walk(state: &mut PlayerState, forward: i8, ticks: u32, colliders: &[Collider2D], ground: &GroundCandidates) {
        for k in 0..ticks {
            let yaw = state.yaw;
            step_player(state, &PlayerInput { seq: k, forward, yaw, ..Default::default() }, colliders, ground);
        }
    }

    #[test]
    fn walking_moves_at_walk_speed_and_sprint_is_faster() {
        let (c, g) = world();
        let mut s = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human); // facing +X
        walk(&mut s, 1, 60, &c, &g);
        assert!((s.pos.x - (-22.0 + 3.2)).abs() < 0.02, "one second at 3.2 m/s: {}", s.pos.x);
        let mut r = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human);
        for k in 0..30 {
            let yaw = r.yaw;
            step_player(&mut r, &PlayerInput { seq: k, forward: 1, sprint: true, yaw, ..Default::default() }, &c, &g);
        }
        assert!((r.pos.x - (-22.0 + 3.25)).abs() < 0.02, "half a second at 6.5 m/s: {}", r.pos.x);
    }

    #[test]
    fn the_rat_has_its_own_pace_slower_than_a_human_sprint() {
        let (c, g) = world();
        let mut rat = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Rat);
        walk(&mut rat, 1, 30, &c, &g); // half a second
        assert!((rat.pos.x - (-22.0 + red_engine2::player::RAT_SPEED * 0.5)).abs() < 0.03, "{}", rat.pos.x);
        assert!(rat.pos.x - (-22.0) < red_engine2::player::SPRINT_SPEED * 0.5 - 0.5, "well short of a human sprint's half-second distance");
        assert!(rat.pos.x - (-22.0) > red_engine2::player::WALK_SPEED * 0.5 + 0.2, "but brisker than a human's walk");
    }

    #[test]
    fn diagonal_movement_is_not_faster_and_hostile_input_cannot_exceed_full_speed() {
        let (c, g) = world();
        let mut s = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human);
        for k in 0..60 {
            // forward and strafe both "127": sanitised to 1, and diagonals are normalised.
            let yaw = s.yaw;
            step_player(&mut s, &PlayerInput { seq: k, forward: 127, strafe: 127, yaw, ..Default::default() }, &c, &g);
        }
        let moved = (s.pos - Vec2::new(-22.0, -3.0)).length();
        assert!(moved <= 3.2 + 0.02, "moved {moved} m in a second");
        let mut n = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human);
        step_player(&mut n, &PlayerInput { forward: 1, yaw: f32::NAN, pitch: f32::INFINITY, ..Default::default() }, &c, &g);
        assert!(n.pos.is_finite() && n.yaw.is_finite() && n.pitch.is_finite());
    }

    #[test]
    fn walls_stop_a_player_and_the_step_is_deterministic() {
        let (c, g) = world();
        let mut a = PlayerState::spawn(-22.0, -3.0, 0.0, 270.0, Character::Human); // facing -X, into the west wall
        let mut b = a;
        walk(&mut a, 1, 300, &c, &g);
        walk(&mut b, 1, 300, &c, &g);
        assert_eq!(a, b, "same inputs, same result");
        assert!(a.pos.x > -23.9, "stopped by the wall at x = -24: {}", a.pos.x);
    }

    #[test]
    fn the_rat_squeezes_through_a_gap_the_human_cannot() {
        let (c, g) = world();
        let mut rat = PlayerState::spawn(-13.0, -6.5, 0.0, 90.0, Character::Rat);
        walk(&mut rat, 1, 120, &c, &g);
        assert!(rat.pos.x > -11.0, "the rat passes the 0.25 m gap: {}", rat.pos.x);
        let mut human = PlayerState::spawn(-13.0, -6.5, 0.0, 90.0, Character::Human);
        walk(&mut human, 1, 120, &c, &g);
        assert!(human.pos.x < -12.0, "the human does not: {}", human.pos.x);
    }

    #[test]
    fn jumping_leaves_the_ground_and_lands_again() {
        let (c, g) = world();
        let mut s = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human);
        let yaw = s.yaw;
        step_player(&mut s, &PlayerInput { jump: true, yaw, ..Default::default() }, &c, &g);
        assert!(s.foot_y > 0.0);
        let mut peak = 0.0f32;
        for _ in 0..60 {
            let yaw = s.yaw;
            step_player(&mut s, &PlayerInput { yaw, ..Default::default() }, &c, &g);
            peak = peak.max(s.foot_y);
        }
        assert!(peak > 0.3 && peak < 0.5, "jump apex ~0.4 m: {peak}");
        assert_eq!(s.foot_y, 0.0);
    }

    #[test]
    fn authored_arena_tuning_and_jump_pads_are_deterministic() {
        let (c, g) = world();
        let tuning =
            PlayerTuning { fov_deg: 90.0, walk_speed: 9.0, sprint_speed: 13.0, crouch_multiplier: 0.5, jump_speed: 6.0, gravity: 18.0, ..Default::default() };
        let pad = JumpPad { id: "test_pad".into(), center: Vec2::new(-22.0, -3.0), size: Vec2::splat(2.0), foot_y: 0.0, launch_speed: 11.0 };
        let mut a = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human);
        let mut b = a;
        for k in 0..30 {
            let input = PlayerInput { seq: k, forward: 1, yaw: 90f32.to_radians(), ..Default::default() };
            step_player_tuned(&mut a, &input, &c, &g, tuning, std::slice::from_ref(&pad));
            step_player_tuned(&mut b, &input, &c, &g, tuning, std::slice::from_ref(&pad));
        }
        assert_eq!(a, b, "authored movement stays bit-identical");
        assert!(a.pos.x > -18.0, "fast profile moved {:.2} m", a.pos.x + 22.0);
        assert!(a.foot_y > 1.0, "jump pad launched to y={}", a.foot_y);
    }
}

/// `sim::match_sim` on the Test Lab: moved from the module's own tests when `sim::match_sim` moved into the core.
mod match_sim {
    use glam::Vec3;
    use red_engine2::player::Character;
    use red_engine2::sim::match_sim::*;
    use red_engine2::sim::player::PlayerInput;
    use red_engine2::sim::spawns::parse_spawns;
    use std::path::Path;

    fn lab_sim(group: &str) -> MatchSim {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json");
        let scene = red_engine2::load_scene(&path).unwrap();
        let mut spawns = parse_spawns(&std::fs::read_to_string(&path).unwrap()).unwrap();
        if !group.is_empty() {
            spawns.retain(|s| s.group == group);
        }
        MatchSim::new(&scene, spawns)
    }

    fn input(seq: u32, forward: i8, yaw_deg: f32) -> PlayerInput {
        PlayerInput { seq, forward, yaw: yaw_deg.to_radians(), ..Default::default() }
    }

    #[test]
    fn players_join_at_distinct_spawns_and_the_match_fills_up() {
        let mut sim = lab_sim("duel");
        let a = sim.add_player(Character::Human).unwrap();
        let b = sim.add_player(Character::Rat).unwrap();
        assert_ne!(a, b);
        assert_ne!(sim.player(a).unwrap().state.pos, sim.player(b).unwrap().state.pos);
        for _ in 2..MAX_PLAYERS {
            assert!(sim.add_player(Character::Human).is_some());
        }
        assert!(sim.add_player(Character::Human).is_none(), "match is full");
        sim.remove_player(a);
        assert_eq!(sim.add_player(Character::Human), Some(a), "the freed slot is reused");
    }

    #[test]
    fn a_player_moves_only_by_processed_inputs_and_does_not_move_without_them() {
        let mut sim = lab_sim("duel");
        let a = sim.add_player(Character::Human).unwrap();
        let start = sim.player(a).unwrap().state.pos;
        for _ in 0..30 {
            sim.tick_once(); // no inputs: no movement (and no extrapolation)
        }
        assert_eq!(sim.player(a).unwrap().state.pos, start);
        for k in 1..=60 {
            assert!(sim.push_input(a, input(k, 1, 90.0)));
            sim.tick_once();
        }
        let p = sim.player(a).unwrap();
        assert_eq!(p.last_processed_seq, 60);
        assert!((p.state.pos.x - (start.x + 3.2)).abs() < 0.05, "one second forward: {}", p.state.pos.x - start.x);
    }

    #[test]
    fn duplicate_and_stale_inputs_are_ignored_and_a_backlog_catches_up() {
        let mut sim = lab_sim("duel");
        let a = sim.add_player(Character::Human).unwrap();
        assert!(sim.push_input(a, input(5, 1, 90.0)));
        assert!(!sim.push_input(a, input(5, 1, 90.0)), "duplicate");
        assert!(!sim.push_input(a, input(4, 1, 90.0)), "stale");
        for k in 6..=12 {
            sim.push_input(a, input(k, 1, 90.0));
        }
        sim.tick_once();
        assert_eq!(sim.player(a).unwrap().last_processed_seq, 6, "queue over the target: two inputs in one tick (5 then 6)");
        for _ in 0..10 {
            sim.tick_once();
        }
        assert_eq!(sim.player(a).unwrap().last_processed_seq, 12);
    }

    #[test]
    fn a_hostile_client_cannot_move_faster_than_full_speed() {
        let mut sim = lab_sim("duel");
        let a = sim.add_player(Character::Human).unwrap();
        let start = sim.player(a).unwrap().state.pos;
        for k in 1..=60 {
            sim.push_input(
                a,
                PlayerInput { seq: k, forward: 100, strafe: -100, sprint: false, yaw: std::f32::consts::FRAC_PI_2, pitch: f32::NAN, ..Default::default() },
            );
            sim.tick_once();
        }
        assert!((sim.player(a).unwrap().state.pos - start).length() <= 3.3);
    }

    #[test]
    fn walking_into_a_barrel_moves_it_authoritatively_for_everyone() {
        let mut sim = lab_sim("props");
        let a = sim.add_player(Character::Human).unwrap(); // spawn_props_a faces -Z toward domino_0
        assert_eq!(sim.props().dynamic_count(), 0, "nothing is promoted until something touches it");
        let before: Vec<Vec3> = (0..sim.props().props().len()).map(|i| sim.props().prop_pose(i).w_axis.truncate()).collect();
        for k in 1..=120 {
            sim.push_input(a, input(k, 1, 0.0)); // walk toward -Z, into the barrels at z = 4
            sim.tick_once();
        }
        assert!(sim.props().dynamic_count() >= 1, "the barrel was promoted");
        let moved = (0..before.len()).filter(|&i| (sim.props().prop_pose(i).w_axis.truncate() - before[i]).length() > 0.1).count();
        assert!(moved >= 1, "and it moved");
        let e = sim.props().entities();
        assert!(!e.is_empty() && (0..e.len()).any(|s| e.transforms.get(s).position != before[sim.props().prop_of_entity(s)]));
    }

    #[test]
    fn identical_inputs_give_identical_checksums_and_different_inputs_do_not() {
        let run = |turn: f32| {
            let mut sim = lab_sim("props");
            let a = sim.add_player(Character::Human).unwrap();
            for k in 1..=150 {
                sim.push_input(a, input(k, 1, turn));
                sim.tick_once();
            }
            sim.checksum()
        };
        assert_eq!(run(0.0), run(0.0), "deterministic on one machine");
        assert_ne!(run(0.0), run(20.0));
    }

    #[test]
    fn removing_a_player_removes_their_physics_body_too() {
        let mut sim = lab_sim("duel");
        let a = sim.add_player(Character::Human).unwrap();
        let b = sim.add_player(Character::Human).unwrap();
        let bodies = sim.props().body_count();
        sim.remove_player(a);
        assert_eq!(sim.props().body_count(), bodies - 1);
        assert_eq!(sim.player_count(), 1);
        assert!(sim.player(b).is_some() && sim.player(a).is_none());
    }
}

/// `net::server` on the Test Lab: moved from the module's own tests when `net::server` moved into the core (the map is the engine's content).
mod net_server {
    use red_engine2::net::server::{Server, ServerConfig};
    use red_engine2::sim::spawns::parse_spawns;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    #[test]
    fn a_pause_flag_stops_the_simulation_and_nothing_else() {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
        let scene = red_engine2::scene_blocks::parse_scene(&text).unwrap();
        let mut spawns = parse_spawns(&text).unwrap();
        spawns.retain(|s| s.group == "duel");
        let mut server = Server::bind(
            ServerConfig::new("127.0.0.1:0".parse().unwrap(), red_engine2::net::map_hash(&text)),
            red_engine2::sim::match_sim::MatchSim::new(&scene, spawns),
        )
        .unwrap();
        server.set_logger(|_| {});
        let flag = Arc::new(AtomicBool::new(false));
        server.set_pause_flag(flag.clone());
        let now = Instant::now();
        for _ in 0..10 {
            server.tick(now);
        }
        assert_eq!((server.sim().tick(), server.stats().ticks), (10, 10));
        flag.store(true, Ordering::Relaxed);
        for _ in 0..10 {
            server.tick(now);
        }
        assert_eq!((server.sim().tick(), server.stats().ticks), (10, 20), "the world stands still while the server carries on ticking");
        flag.store(false, Ordering::Relaxed);
        for _ in 0..5 {
            server.tick(now);
        }
        assert_eq!(server.sim().tick(), 15, "and runs on from where it stopped");
    }
}
