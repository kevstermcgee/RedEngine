//! The custom-client boundary (`red_engine2::app`, ADR 0043) without a window: the external example's scene loads through
//! strict validation, plays through `LocalSession` (the authoritative `MatchSim`), and its rules drive what a client would
//! hide and show. Runs in the graphics-free build too; the external crate's own tests cover input and rendering.

use red_engine2::app::{input_toward, place_object, HudState, LocalSession, ViewCamera};
use red_engine2::glam::{Vec2, Vec3};
use std::path::Path;

fn scene() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/external/topdown_switch/topdown_switch.json")
}

fn walk(s: &mut LocalSession, to: Vec2, max_ticks: usize) {
    for _ in 0..max_ticks {
        if s.player().pos.distance(to) <= 0.15 || s.outcome().is_some() {
            return;
        }
        let input = input_toward(&s.player(), to, 0.15);
        s.step(input);
    }
}

#[test]
fn the_top_down_game_is_won_through_the_shared_simulation() {
    let mut s = LocalSession::load(&scene()).unwrap();
    walk(&mut s, Vec2::new(0.0, -5.5), 300);
    assert!(s.player().pos.y > -2.0, "the closed gate blocks: {}", s.player().pos);
    for p in [Vec2::new(-5.0, 2.5), Vec2::new(5.0, 2.5), Vec2::new(0.0, 1.0), Vec2::new(0.0, -5.5)] {
        walk(&mut s, p, 900);
    }
    assert_eq!(s.outcome(), Some("escaped"));
    let hidden: Vec<&str> = s.hidden().collect();
    for id in ["gate", "lamp_a_off", "lamp_b_off"] {
        assert!(hidden.contains(&id), "{id} hidden: {hidden:?}");
    }
    let hud: HudState = s.hud();
    assert_eq!(hud.outcome.as_deref(), Some("escaped"));
    assert!(hud.layout(1280, 720).check().is_empty());
    // Presentation edits never reach gameplay.
    assert!(place_object(s.scene_mut(), "player", Vec3::new(3.0, 0.0, 3.0), Some(90.0)));
    assert_eq!(s.outcome(), Some("escaped"));
}

#[test]
fn a_camera_click_on_a_switch_is_a_ground_point_on_it() {
    let s = LocalSession::load(&scene()).unwrap();
    let feet = s.player_feet();
    let cam = ViewCamera::top_down(Vec3::new(feet.x, 0.0, feet.z - 1.0), 13.0, 28.0, 0.0);
    let (x, y) = cam.world_to_screen(Vec3::new(-5.0, 0.0, 2.5), 1280, 720).unwrap();
    let p = cam.pick_ground(x, y, 1280, 720, 0.0).unwrap();
    assert!(p.distance(Vec3::new(-5.0, 0.0, 2.5)) < 1e-3, "{p}");
}

#[test]
fn invalid_content_is_refused_with_field_paths() {
    let err = LocalSession::from_json(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [{"id": "a", "type": "box", "pos": [0, 0, 0]}]}"#)
        .err()
        .unwrap();
    assert!(err.iter().any(|e| e.contains("a.pos") && e.contains("position")), "{err:?}");
}
