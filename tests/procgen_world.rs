//! A generated world as something to walk on (ADR 2026-10-04-an-endless-world): the ground under the player is the generator's, trunks block, and
//! walking a long way in a straight line never falls through, floats, or ends up inside a tree. Headless: no GPU is involved.

use glam::Vec2;
use red_engine2::collide::{collect_box_colliders, collect_ground_candidates, ground_height_at};
use red_engine2::player::{Character, PLAYER_RADIUS};
use red_engine2::procgen::{ChunkId, Kind};
use red_engine2::sim::player::{step_player, PlayerInput, PlayerState};

fn scene() -> red_engine2::schema::Scene {
    let text =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/endless_meadow.json")).expect("examples/endless_meadow.json");
    red_engine2::schema::parse_scene(&text).expect("the example scene parses")
}

#[test]
fn the_scene_parses_and_its_ground_is_the_generator() {
    let s = scene();
    let cfg = s.procgen.clone().expect("a procgen block");
    assert_eq!(cfg.seed, 7);
    let g = collect_ground_candidates(&s);
    let pg = g.procgen().expect("ground has the generated world");
    for (x, z) in [(0.0, 0.0), (130.5, -77.25), (-4000.0, 2500.0)] {
        assert_eq!(g.terrain_height_at(Vec2::new(x, z)), Some(pg.world().height(x as f64, z as f64)));
        assert_eq!(ground_height_at(&g, Vec2::new(x, z), 0.0), pg.world().height(x as f64, z as f64));
    }
}

#[test]
fn a_bad_procgen_block_is_refused_with_the_path() {
    let e = red_engine2::schema::parse_scene(r#"{"camera":{"position":[0,1,0],"target":[0,1,-1]},"procgen":{"seed":-3},"objects":[]}"#).unwrap_err().join(" ");
    assert!(e.contains("procgen.seed"), "{e}");
    let e = red_engine2::schema::parse_scene(r#"{"camera":{"position":[0,1,0],"target":[0,1,-1]},"procgen":{"forest":1},"objects":[]}"#).unwrap_err().join(" ");
    assert!(e.contains("forest"), "{e}");
}

#[test]
fn walking_a_long_way_stays_on_the_ground_and_out_of_the_trees() {
    let s = scene();
    let ground = collect_ground_candidates(&s);
    let statics = collect_box_colliders(&s);
    let world = ground.procgen().unwrap().world();
    // Walk east (yaw 90 degrees) for two minutes at a run.
    let mut st = PlayerState::spawn(0.0, 0.0, world.height(0.0, 0.0), 90.0, Character::Human);
    let input = PlayerInput { forward: 1, sprint: true, yaw: st.yaw, ..Default::default() };
    let mut worst_gap = 0.0f32;
    let mut closest = f32::MAX;
    let mut trunks_seen = 0;
    for tick in 0..(60 * 120) {
        step_player(&mut st, &input, &statics, &ground);
        let floor = ground_height_at(&ground, st.pos, st.foot_y);
        worst_gap = worst_gap.max((st.foot_y - floor).abs());
        if tick % 30 == 0 {
            let id = ChunkId::at(st.pos.x as f64, st.pos.y as f64);
            for dz in -1..=1 {
                for dx in -1..=1 {
                    for t in world.trunks(ChunkId { x: id.x + dx, z: id.z + dz }) {
                        trunks_seen += 1;
                        let d = Vec2::new(t.x as f32, t.z as f32).distance(st.pos) - t.radius;
                        closest = closest.min(d);
                    }
                }
            }
        }
    }
    assert!(st.pos.x > 600.0, "only got {} m in two minutes at a run", st.pos.x);
    assert!(worst_gap < 0.12, "feet strayed {worst_gap} m from the ground");
    assert!(trunks_seen > 200, "the walk passed too few trees to mean anything: {trunks_seen}");
    // Colliders are squares round each trunk, so the circle of the player may graze a trunk's corner but never be inside it.
    assert!(closest > PLAYER_RADIUS * 0.6, "the player ended up {closest} m from a trunk's surface");
    assert!(!world.plants(ChunkId::at(st.pos.x as f64, st.pos.y as f64), Kind::Grass).is_empty());
}

#[test]
fn the_same_walk_twice_gives_the_same_place() {
    let s = scene();
    let walk = || {
        let ground = collect_ground_candidates(&s);
        let statics = collect_box_colliders(&s);
        let mut st = PlayerState::spawn(0.0, 0.0, 0.0, 40.0, Character::Human);
        let input = PlayerInput { forward: 1, yaw: st.yaw, ..Default::default() };
        for _ in 0..1800 {
            step_player(&mut st, &input, &statics, &ground);
        }
        (st.pos, st.foot_y)
    };
    assert_eq!(walk(), walk());
}
