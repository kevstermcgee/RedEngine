//! Marcel's world as something to walk on: the real player step through the real generated world, measured.
//!
//! A boy runs in straight lines across country (meadow, wood and hill) and the test checks, tick by tick, the two things that must hold for the world to feel solid:
//! his feet are on the ground (what the simulation thinks the ground is is also what the renderer draws), and no tree he can see is walked through.

use glam::Vec2;
use red_engine2::collide::collect_ground_candidates_except;
use red_engine2::player::Character;
use red_engine2::procgen::chunk;
use red_engine2::procgen::world::{ChunkId, CHUNK};
use red_engine2::sim::player::{step_player, PlayerInput, PlayerState};

fn marcel() -> red_engine2::schema::Scene {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/marcel/marcel.json")).unwrap();
    red_engine2::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}"))
}

/// What the test keeps of each chunk it has walked through: the ground mesh the renderer draws and the things that stop the boy.
#[derive(Default)]
struct Chunks {
    meshes: std::collections::HashMap<ChunkId, red_engine2::procgen::geo::Geo>,
    blockers: std::collections::HashMap<ChunkId, Vec<red_engine2::procgen::world::Trunk>>,
}

/// The height of the surface the renderer draws at a point: the triangle of the chunk's ground mesh that covers it.
fn drawn_height(world: &red_engine2::procgen::world::World, cache: &mut Chunks, p: Vec2) -> f32 {
    let id = ChunkId::at(p.x as f64, p.y as f64);
    let g = cache.meshes.entry(id).or_insert_with(|| chunk::ground(world, id));
    let (x0, z0) = id.origin();
    let (lx, lz) = (p.x - x0 as f32, p.y - z0 as f32);
    for t in g.idx.as_chunks::<3>().0 {
        let v: Vec<[f32; 3]> = t.iter().map(|&i| g.pos[i as usize]).collect();
        let (a, b, c) = (v[0], v[1], v[2]);
        let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
        if d.abs() < 1e-9 {
            continue;
        }
        let w0 = ((b[2] - c[2]) * (lx - c[0]) + (c[0] - b[0]) * (lz - c[2])) / d;
        let w1 = ((c[2] - a[2]) * (lx - c[0]) + (a[0] - c[0]) * (lz - c[2])) / d;
        let w2 = 1.0 - w0 - w1;
        if w0 >= -1e-5 && w1 >= -1e-5 && w2 >= -1e-5 {
            return w0 * a[1] + w1 * b[1] + w2 * c[1];
        }
    }
    panic!("no triangle under {p:?}");
}

#[test]
fn a_boy_running_across_the_country_stays_on_the_ground_and_out_of_the_trees() {
    let scene = marcel();
    let ground = collect_ground_candidates_except(&scene, &Default::default());
    let world = ground.procgen().expect("marcel has a world").world();
    let (mut worst_below, mut worst_above, mut worst_gap, mut inside, mut ticks, mut stopped) = (0.0f32, 0.0f32, 0.0f32, 0usize, 0usize, 0usize);
    let mut cache = Chunks::default();
    let mut first_inside = None;
    for (n, (sx, sz)) in [(0.0f32, 0.0f32), (700.0, -400.0), (-900.0, 650.0), (1500.0, 1500.0), (-2200.0, -300.0)].into_iter().enumerate() {
        for k in 0..4 {
            let yaw = k as f32 * 1.571 + n as f32 * 0.3;
            let mut s = PlayerState::spawn(sx, sz, world.height(sx as f64, sz as f64), yaw.to_degrees(), Character::Boy);
            let input = PlayerInput { forward: 1, sprint: true, yaw, ..Default::default() };
            for _ in 0..(60 * 25) {
                let before = s.pos;
                step_player(&mut s, &input, &[], &ground);
                // Sprinting this boy covers about 9 cm a tick; much less means a tree is in the way.
                stopped += usize::from(s.pos.distance(before) < 0.04);
                ticks += 1;
                let here = world.height(s.pos.x as f64, s.pos.y as f64);
                // Below the ground the simulation knows is falling through; the picture is another matter, so measure both.
                worst_below = worst_below.max(here - s.foot_y);
                worst_above = worst_above.max(s.foot_y - here);
                worst_gap = worst_gap.max((drawn_height(world, &mut cache, s.pos) - s.foot_y).max(0.0));
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let c = ChunkId::at((s.pos.x + dx as f32 * CHUNK as f32) as f64, (s.pos.y + dz as f32 * CHUNK as f32) as f64);
                        for t in cache.blockers.entry(c).or_insert_with(|| world.blockers(c)).iter() {
                            let d = Vec2::new(t.x as f32, t.z as f32).distance(s.pos);
                            if d < t.radius * 0.8 {
                                inside += 1;
                                first_inside.get_or_insert((s.pos, *t, d));
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("{ticks} ticks ({stopped} spent held up by a tree): feet below ground by up to {worst_below:.3} m, above by up to {worst_above:.3} m, drawn surface above the feet by up to {worst_gap:.3} m, {inside} ticks inside a tree ({first_inside:?})");
    assert!(stopped > 20, "the runs never met a tree ({stopped} held-up ticks): the test proves nothing about trees");
    assert!(worst_below < 0.05, "the boy sank {worst_below} m into the ground");
    assert!(worst_gap < 0.15, "the drawn ground is {worst_gap} m above his feet: he wades through the floor");
    assert_eq!(inside, 0, "walked into a tree: {first_inside:?}");
}

#[test]
fn a_boy_who_spawns_below_the_hill_is_lifted_onto_it_not_left_inside_it() {
    let scene = marcel();
    let ground = collect_ground_candidates_except(&scene, &Default::default());
    let world = ground.procgen().expect("marcel has a world").world();
    for (x, z) in [(0.0f32, 0.0f32), (3.0, 4.0), (500.0, 500.0), (-1200.0, 300.0)] {
        let h = world.height(x as f64, z as f64);
        for start in [0.0f32, h - 2.0, h - 0.5, h + 3.0] {
            let mut s = PlayerState::spawn(x, z, start, 0.0, Character::Boy);
            for _ in 0..240 {
                step_player(&mut s, &PlayerInput::default(), &[], &ground);
            }
            eprintln!("spawn at ({x}, {z}) ground {h:.2} feet start {start:.2} -> after 4 s feet {:.2}", s.foot_y);
            assert!((s.foot_y - h).abs() < 0.05, "spawned at ({x}, {z}) with feet at {start}: after 4 s the feet are at {} but the ground is at {h}", s.foot_y);
        }
    }
}

/// The single-player client builds its ground from per-object groups; Marcel has no objects, so the world must come from the scene itself. (It used to come from
/// nowhere: a flat floor at y = 0 under a hillside and trees that did not block, in single-player only.)
#[test]
fn the_ground_the_client_builds_from_object_groups_has_the_world_in_it() {
    use red_engine2::collide::{collect_ground_candidates_grouped_except, ground_from_groups};
    let scene = marcel();
    assert!(scene.objects.is_empty(), "the point of this scene: all world, no objects");
    let groups = collect_ground_candidates_grouped_except(&scene, &Default::default());
    let ground = ground_from_groups(&scene, &groups);
    let world = ground.procgen().expect("the scene's world is part of the ground").world();
    assert!(ground.has_natural_ground());
    let at = |x: f32, z: f32| red_engine2::collide::ground_height_at(&ground, Vec2::new(x, z), world.height(x as f64, z as f64));
    assert!(
        (at(0.0, 0.0) - world.height(0.0, 0.0)).abs() < 1e-4 && at(0.0, 0.0) < -0.5,
        "the ground at the spawn is the hill's, not a flat 0: {}",
        at(0.0, 0.0)
    );
    // The trees are there too: the step asks the ground for the trunks around the boy.
    let (cx, cz) = (0..40)
        .flat_map(|z| (0..40).map(move |x| (x - 20, z - 20)))
        .find_map(|(cx, cz)| world.blockers(ChunkId { x: cx, z: cz }).first().map(|t| (t.x as f32, t.z as f32)))
        .expect("a tree somewhere");
    assert!(!ground.procgen().unwrap().colliders_near(Vec2::new(cx, cz), 2.0, &[]).is_empty(), "the tree at ({cx}, {cz}) blocks");
    // And a boy dropped at y = 0 is lifted (here: lowered) onto it.
    let mut s = PlayerState::spawn(0.0, 0.0, 0.0, 0.0, Character::Boy);
    for _ in 0..120 {
        step_player(&mut s, &PlayerInput::default(), &[], &ground);
    }
    assert!((s.foot_y - world.height(0.0, 0.0)).abs() < 0.02, "his feet are at {} on ground at {}", s.foot_y, world.height(0.0, 0.0));
}
