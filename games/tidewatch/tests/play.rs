//! Tidewatch through its real client (`Tidewatch::frame_with`: the presenter's controls and the movement axes into `app::LocalSession`),
//! against the same authority the headless match runs, with the story presented by the engine's optional presenter and drawn by the
//! offscreen renderer: what a player sees (`out/*.png`), that no gameplay input leaks while talking, and that a save resumes the talk.

use red_engine2::app::dialogue::Controls;
use red_engine2::app::{Frame, Offscreen};
use tidewatch::{default_scene, Tidewatch};

const W: u32 = 1280;
const H: u32 = 720;

fn frame() -> Frame {
    Frame { dt: 1.0 / 60.0, width: W, height: H }
}

fn idle(game: &mut Tidewatch, frames: usize) {
    for _ in 0..frames {
        assert!(game.frame_with(&Controls::default(), (0, 0), false, false, frame()));
    }
}

fn press(game: &mut Tidewatch, c: Controls) {
    assert!(game.frame_with(&c, (0, 0), false, false, frame()));
    idle(game, 2);
}

fn confirm() -> Controls {
    Controls { confirm: true, ..Controls::default() }
}

/// Walks with the keys toward `(x, z)` (W is north, -z; D is east, +x).
fn walk_to(game: &mut Tidewatch, x: f32, z: f32) {
    for _ in 0..60 * 30 {
        let p = game.session().player().pos;
        let (dx, dz) = (x - p.x, z - p.y);
        if dx.abs() < 0.3 && dz.abs() < 0.3 {
            idle(game, 5);
            return;
        }
        let axis = |d: f32| {
            if d > 0.2 {
                1
            } else if d < -0.2 {
                -1
            } else {
                0
            }
        };
        assert!(game.frame_with(&Controls::default(), (-axis(dz), axis(dx)), false, false, frame()));
    }
    panic!("could not walk to ({x}, {z})");
}

fn offscreen(scene: &red_engine2::schema::Scene) -> Option<Offscreen> {
    match Offscreen::new(scene, W, H) {
        Ok(o) => Some(o),
        Err(e) if std::env::var_os("RED_OFFSCREEN_OPTIONAL").is_some() => {
            eprintln!("SKIPPED the pictures: {e:#}");
            None
        }
        Err(e) => panic!("no offscreen GPU adapter (set RED_OFFSCREEN_OPTIONAL=1 to skip the pictures on a GPU-less runner): {e:#}"),
    }
}

fn picture(cam: &mut Option<Offscreen>, game: &Tidewatch, name: &str) {
    let Some(cam) = cam.as_mut() else { return };
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("out").join(name);
    let hidden: Vec<String> = game.session().hidden().map(str::to_string).collect();
    cam.save_png(&out, game.session().scene(), 0.0, &game.camera_view(), hidden.iter().map(String::as_str), Some(&game.hud_layout(W, H))).unwrap();
}

fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("tidewatch-client-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.join("tidewatch.json")
}

#[test]
fn a_conversation_through_the_client_reveals_then_advances_and_holds_the_player() {
    let save = temp("talk");
    let mut game = Tidewatch::load(&default_scene(), Some(save.clone()), false).unwrap();
    let mut cam = offscreen(game.session().scene());
    walk_to(&mut game, 13.0, 5.5);
    assert_eq!(game.near().map(|n| n.0), Some("bram"));
    picture(&mut cam, &game, "dock_prompt.png");
    press(&mut game, confirm());
    let v = game.view();
    assert_eq!(v.talk.as_ref().map(|t| t.line.id.as_str()), Some("talk_bram/hello/l1"));
    // Half typed: a press shows the whole line and does not move on.
    assert!(!game.presenter().revealed(&v));
    picture(&mut cam, &game, "talk_typing.png");
    press(&mut game, confirm());
    assert!(game.presenter().revealed(&game.view()));
    assert_eq!(game.view().talk.as_ref().map(|t| t.line.id.clone()).as_deref(), Some("talk_bram/hello/l1"));
    // Held still: walking keys do nothing while the conversation is open.
    let at = game.session().player().pos;
    for _ in 0..60 {
        game.frame_with(&Controls::default(), (1, 1), false, false, frame());
    }
    assert_eq!(game.session().player().pos, at, "no gameplay input leaks while talking");
    // Move on to the request: reveal, advance, reveal, ... until it offers choices.
    for _ in 0..12 {
        if game.view().talk.as_ref().is_some_and(|t| !t.choices.is_empty()) && game.presenter().revealed(&game.view()) {
            break;
        }
        press(&mut game, confirm());
    }
    let v = game.view();
    assert_eq!(v.talk.as_ref().unwrap().line.id, "talk_bram/ask/l1");
    picture(&mut cam, &game, "talk_choices.png");
    // Focus: down twice from the first, then a save mid-choice, then picking through a fresh client restored from it.
    press(&mut game, Controls { down: true, ..Controls::default() });
    assert_eq!(game.presenter().focus(), 1);
    game.save_now().unwrap();
    let mut again = Tidewatch::load(&default_scene(), Some(save), false).unwrap();
    assert_eq!(again.save_note(), Some("progress restored"));
    assert_eq!(again.view(), game.view(), "the same line, choices and revision");
    // The restored client shows the line typing again; one press reveals it, then `1` picks the first choice (lend).
    assert_eq!(again.near().map(|n| n.0), Some("bram"), "resumed where the player stood, so Bram is still here");
    press(&mut again, confirm());
    press(&mut again, Controls { pick: Some(1), ..Controls::default() });
    assert_eq!(again.session().rules().var("shed"), Some(3.0), "two real bricks went to the dock lamp");
    // His reply, then the conversation ends and his bark (queued behind it) plays as a subtitle.
    press(&mut again, confirm());
    press(&mut again, confirm());
    assert!(again.view().talk.is_none());
    idle(&mut again, 20);
    assert_eq!(again.view().bark.map(|b| b.id).as_deref(), Some("lent_cheer"));
    picture(&mut cam, &again, "bark_after_lend.png");
}

#[test]
fn a_stale_press_from_the_client_is_refused_by_the_authority() {
    let mut game = Tidewatch::load(&default_scene(), None, false).unwrap();
    walk_to(&mut game, 13.0, 5.5);
    press(&mut game, confirm());
    let old = game.view().revision;
    press(&mut game, confirm());
    press(&mut game, confirm());
    // A press that names the revision before (a duplicate packet): refused, nothing changes.
    let before = game.game().systems().hash();
    game.intend(&red_engine2::sim::game::Intent { name: "story:advance".into(), subject: None, value: old as i64 });
    assert!(game.refused.as_deref().is_some_and(|r| r.contains("stale")), "{:?}", game.refused);
    assert_eq!(game.game().systems().hash(), before);
}
