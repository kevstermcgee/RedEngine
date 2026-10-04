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

/// The finished game: the boy, a day that turns, a countryside and four scores, a count of days lived.
fn marcel() -> red_engine2::schema::Scene {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/marcel");
    let text = std::fs::read_to_string(dir.join("marcel.json")).expect("examples/marcel/marcel.json");
    red_engine2::schema::parse_scene_in(&text, Some(&dir)).expect("Marcel parses")
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

#[test]
fn marcel_has_a_countryside_and_four_scores_that_parse() {
    let s = marcel();
    let audio = s.audio.clone().expect("an audio block");
    assert!(audio.nature);
    assert_eq!(audio.music.len(), 4, "a score for each time of day");
    for (mood, path) in &audio.music {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(path))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        let score = red_engine2::score::parse_score(&value).unwrap_or_else(|e| panic!("{}: {e:?}", mood.name()));
        assert!(score.lufs <= -24.0, "{} should sit quietly under the world", mood.name());
    }
}

#[test]
fn each_sunrise_adds_a_day_to_the_count_the_game_keeps() {
    use red_engine2::sim::rules_run::{RulePlayer, RulesEngine};
    let s = marcel();
    assert_eq!(s.rules.persist, vec!["days_lived".to_string()]);
    let mut rules = RulesEngine::new(s.rules.clone());
    assert_eq!(rules.persisted(), vec![("days_lived".to_string(), 0.0)]);
    // A game that was played before comes back where it was.
    assert!(rules.set_var("days_lived", 6.0));
    assert!(!rules.set_var("no_such_var", 1.0));
    let me = RulePlayer { slot: 0, pos: glam::Vec3::ZERO, radius: 0.3, height: 1.3, character: Character::Boy, team: 0 };
    rules.step(1, &[me]);
    for tick in [100u64, 200] {
        rules.inject(tick, "sunrise", None);
        rules.step(tick + 1, &[me]);
    }
    assert_eq!(rules.persisted(), vec![("days_lived".to_string(), 8.0)], "two sunrises seen, two days more");
    // The sunset is not a day.
    rules.inject(300, "sunset", None);
    rules.step(301, &[me]);
    assert_eq!(rules.var("days_lived"), Some(8.0));
}

#[test]
fn only_declared_variables_can_be_kept() {
    let e = red_engine2::schema::parse_scene(r#"{"camera":{"position":[0,1,0],"target":[0,1,-1]},"vars":{"a":0},"persist":["b"],"objects":[]}"#)
        .unwrap_err()
        .join(" ");
    assert!(e.contains("persist[0]") && e.contains("`b`"), "{e}");
}

#[test]
fn marcel_is_a_boy_behind_him_in_a_peaceful_world_with_a_menu_that_counts_days() {
    let s = marcel();
    assert_eq!(s.player.character, Some(Character::Boy));
    assert!(s.player.third_person && s.player.mode.is_peaceful());
    let clock = s.clock.as_ref().expect("a clock");
    assert!(clock.day_secs >= 900.0 && clock.state(clock.t_for_hour(5.0), 0).sun_elev_deg < 0.0, "a long day that begins before sunrise");
    let card = s.ui.as_ref().and_then(|u| u.start_card(&[("days_lived", 1.0)])).expect("a start card");
    assert_eq!(card.title, "Marcel");
    assert!(card.text.contains("1 day lived"), "{}", card.text);
    assert!(s.ui.as_ref().and_then(|u| u.start_card(&[("days_lived", 12.0)])).unwrap().text.contains("12 days"));
}
