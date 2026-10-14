//! Workshop through its real client (`Workshop::frame`: keys and clicks into `app::LocalSession`), checked against the headless match and
//! drawn with the engine's offscreen renderer.

use red_engine2::app::{Frame, InputState, MouseButton, Offscreen};
use red_engine2::glam::Vec3;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use workshop::{default_scene, Workshop};
use workshop_server::game::WorkshopGame;

const W: u32 = 1280;
const H: u32 = 720;

fn frame() -> Frame {
    Frame { dt: 1.0 / 60.0, width: W, height: H }
}

fn idle(game: &mut Workshop, frames: usize) {
    let input = InputState::new();
    for _ in 0..frames {
        assert!(game.frame(&input, frame()));
    }
}

fn click_and_walk(game: &mut Workshop, p: Vec3, max_frames: usize) {
    let (x, y) = game.view().world_to_screen(p, W, H).expect("on screen");
    let mut input = InputState::new();
    input.click(MouseButton::Left, x, y);
    assert!(game.frame(&input, frame()));
    let none = InputState::new();
    for _ in 0..max_frames {
        if game.target.is_none() {
            break;
        }
        game.frame(&none, frame());
    }
}

fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("workshop-client-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.join("workshop.json")
}

#[test]
fn the_client_runs_the_same_authority_as_the_headless_match() {
    let mut game = Workshop::load(&default_scene(), None).unwrap();
    idle(&mut game, 600);
    let ticks = game.session().sim().tick();
    // The same scene in a bare `MatchSim` (what `sim` and `workshop_server` run), the same number of ticks, nobody touching the workshop.
    let text = std::fs::read_to_string(default_scene()).unwrap();
    let scene = red_engine2::scene_blocks::parse_scene_in(&text, default_scene().parent()).unwrap();
    let mut sim = MatchSim::try_new(&scene, parse_spawns(&text).unwrap()).unwrap();
    for _ in 0..ticks {
        sim.tick_once();
    }
    assert_eq!(sim.game::<WorkshopGame>().unwrap().systems().hash(), game.game().systems().hash(), "after {ticks} ticks");
}

#[test]
fn walking_onto_the_lever_ships_the_order_and_the_lamps_follow_the_game() {
    let save = temp("play");
    let mut game = Workshop::load(&default_scene(), Some(save.clone())).unwrap();
    assert!(game.save_note().is_none(), "no save yet");
    idle(&mut game, 1800);
    assert!(game.machines().iter().any(|(_, s, _)| s == "blocked"), "{:?}", game.machines());
    assert!(game.hidden_objects().contains(&"furnace_a_ok".to_string()) || game.hidden_objects().contains(&"press_a_ok".to_string()));
    let mut cam = Offscreen::new(game.session().scene(), W, H).expect("an offscreen renderer");
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("out");
    let hidden = game.hidden_objects();
    cam.save_png(&out.join("blocked.png"), game.session().scene(), 0.0, &game.view(), hidden.iter().map(String::as_str), Some(&game.hud_layout(W, H))).unwrap();
    click_and_walk(&mut game, Vec3::new(4.0, 0.0, 1.5), 900);
    assert_eq!(game.session().rules().var("route"), Some(1.0), "the lever flipped the route");
    // A save mid-production: a new client restores it and the order ships from where it was.
    idle(&mut game, 300);
    game.save_now().unwrap();
    let shipped_at_save = game.session().rules().var("gears_shipped").unwrap();
    let mut again = Workshop::load(&default_scene(), Some(save.clone())).unwrap();
    assert_eq!(again.save_note(), Some("progress restored"));
    assert_eq!(again.game().systems().hash(), game.game().systems().hash());
    assert_eq!(again.session().rules().var("route"), Some(1.0), "the scene's variables come back with the save");
    for _ in 0..3000 {
        if again.session().outcome().is_some() {
            break;
        }
        idle(&mut again, 1);
    }
    assert_eq!(again.session().outcome(), Some("shipped"));
    assert!(again.session().rules().var("gears_shipped").unwrap() >= 12.0 && shipped_at_save < 12.0);
    let hidden = again.hidden_objects();
    let px = cam.render(again.session().scene(), 0.0, &again.view(), hidden.iter().map(String::as_str), Some(&again.hud_layout(W, H))).unwrap();
    image_check(&px);
    cam.save_png(&out.join("shipped.png"), again.session().scene(), 0.0, &again.view(), hidden.iter().map(String::as_str), Some(&again.hud_layout(W, H)))
        .unwrap();
}

/// The picture has the room in it (many distinct colours), not a blank or a single colour.
fn image_check(px: &[u8]) {
    let mut colours = std::collections::BTreeSet::new();
    for p in px.chunks(4).step_by(97) {
        colours.insert((p[0] / 16, p[1] / 16, p[2] / 16));
    }
    assert!(colours.len() > 20, "{} distinct colours", colours.len());
}

#[test]
fn a_version_1_save_is_migrated_by_the_client_and_a_broken_one_is_left_alone() {
    let save = temp("v1");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("content/saves/v1.json");
    std::fs::copy(&fixture, &save).unwrap();
    let game = Workshop::load(&default_scene(), Some(save.clone())).unwrap();
    assert_eq!(game.save_note(), Some("progress restored, migrated: ws:machines v1 -> v2"));
    let broken = temp("broken");
    std::fs::write(&broken, "{\"game\": \"workshop\", \"systems\": {\"systems_save\": 9}}").unwrap();
    let game = Workshop::load(&default_scene(), Some(broken.clone())).unwrap();
    assert!(game.save_note().unwrap().starts_with("progress not restored (the file is kept as it is)"), "{:?}", game.save_note());
    assert!(std::fs::read_to_string(&broken).unwrap().contains("\"systems_save\": 9"), "the broken file is untouched");
}
