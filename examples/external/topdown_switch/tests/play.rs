//! Plays the game through its real input path (clicks and keys into `TopDown::frame`) and checks what is drawn with the
//! engine's offscreen renderer. The gameplay assertions read the engine's rules state; nothing here re-implements it.

use red_engine2::app::{Frame, InputState, KeyCode, MouseButton, Offscreen};
use red_engine2::glam::Vec3;
use topdown_switch::{default_scene, TopDown};

const W: u32 = 1280;
const H: u32 = 720;
const DT: f32 = 1.0 / 60.0;

fn frame() -> Frame {
    Frame { dt: DT, width: W, height: H }
}

/// Clicks the ground point `p` on screen, then lets the player walk (no input) for up to `max_frames`.
fn click_and_walk(game: &mut TopDown, p: Vec3, max_frames: usize) {
    let (x, y) = game.view().world_to_screen(p, W, H).expect("point on screen");
    let mut input = InputState::new();
    input.click(MouseButton::Left, x, y);
    assert!(game.frame(&input, frame()));
    let idle = InputState::new();
    for _ in 0..max_frames {
        if game.target.is_none() || game.session().outcome().is_some() {
            break;
        }
        game.frame(&idle, frame());
    }
}

fn hidden(game: &TopDown) -> Vec<String> {
    game.session().hidden().map(str::to_string).collect()
}

#[test]
fn clicks_press_both_switches_open_the_gate_and_reach_the_exit() {
    let mut game = TopDown::load(&default_scene()).unwrap();

    // Straight for the exit first: the closed gate (authored collision) stops the player short of it.
    click_and_walk(&mut game, Vec3::new(0.0, 0.0, -5.5), 400);
    let p = game.session().player().pos;
    assert!(p.y > -2.0, "walked through the closed gate: {p}");
    assert!(game.session().outcome().is_none());
    assert!(!hidden(&game).contains(&"gate".to_string()));

    click_and_walk(&mut game, Vec3::new(-5.0, 0.0, 2.5), 900);
    assert_eq!(game.session().rules().var("switches"), Some(1.0));
    assert!(hidden(&game).contains(&"lamp_a_off".to_string()), "lamp A shows on");
    click_and_walk(&mut game, Vec3::new(5.0, 0.0, 2.5), 900);
    assert_eq!(game.session().rules().var("gate_open"), Some(1.0));
    assert!(hidden(&game).contains(&"gate".to_string()), "the rules hid the gate");

    click_and_walk(&mut game, Vec3::new(0.0, 0.0, 1.0), 600);
    click_and_walk(&mut game, Vec3::new(0.0, 0.0, -5.5), 900);
    assert_eq!(game.session().outcome(), Some("escaped"), "ended at {}", game.session().player().pos);
    let hud = game.session().hud();
    assert_eq!(hud.outcome.as_deref(), Some("escaped"));
    assert!(hud.vars.iter().any(|(n, v)| n == "switches" && *v == 2.0));
}

#[test]
fn keys_walk_up_the_screen_whichever_way_the_view_is_turned() {
    let mut game = TopDown::load(&default_scene()).unwrap();
    let start = game.session().player().pos;
    let mut input = InputState::new();
    input.set_key(KeyCode::KeyW, true);
    for _ in 0..30 {
        game.frame(&input, frame());
        input.end_frame();
    }
    let after_w = game.session().player().pos;
    assert!(after_w.y < start.y - 0.5 && (after_w.x - start.x).abs() < 0.05, "W at view yaw 0 walks -Z: {start} -> {after_w}");

    let mut turn = InputState::new();
    turn.set_key(KeyCode::KeyE, true);
    game.frame(&turn, frame());
    assert_eq!(game.view_yaw, 90.0);
    for _ in 0..30 {
        game.frame(&input, frame());
        input.end_frame();
    }
    let after_turn = game.session().player().pos;
    assert!(after_turn.x > after_w.x + 0.5, "W at view yaw 90 walks +X: {after_w} -> {after_turn}");
}

#[test]
fn the_hud_is_audited_at_common_window_sizes() {
    let game = TopDown::load(&default_scene()).unwrap();
    for (w, h) in [(640, 360), (1280, 720), (1920, 1080), (2560, 1440)] {
        let l = game.hud_layout(w, h);
        assert!(l.check().is_empty(), "{w}x{h}: {:?}", l.check());
    }
}

/// Mean colour of a small square around a pixel.
fn sample(px: &[u8], (x, y): (f32, f32), r: i32) -> [f32; 3] {
    let (mut sum, mut n) = ([0.0f32; 3], 0.0);
    for dy in -r..=r {
        for dx in -r..=r {
            let (xi, yi) = (x as i32 + dx, y as i32 + dy);
            if xi < 0 || yi < 0 || xi >= W as i32 || yi >= H as i32 {
                continue;
            }
            let i = ((yi as u32 * W + xi as u32) * 4) as usize;
            for c in 0..3 {
                sum[c] += px[i + c] as f32;
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

#[test]
fn what_is_drawn_follows_the_rules_gate_marker_and_outcome_banner() {
    let mut game = TopDown::load(&default_scene()).unwrap();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("out");
    let mut shot = match Offscreen::new(game.session().scene(), W, H) {
        Ok(s) => s,
        // CI runners without any adapter set RED_OFFSCREEN_OPTIONAL=1; everywhere else a missing GPU is a failure.
        Err(e) if std::env::var_os("RED_OFFSCREEN_OPTIONAL").is_some() => {
            eprintln!("SKIPPED presentation check: {e:#}");
            return;
        }
        Err(e) => panic!("no offscreen GPU adapter (set RED_OFFSCREEN_OPTIONAL=1 to skip on a GPU-less runner): {e:#}"),
    };
    // Renders the frame the window would show (world + HUD), saves it, and also returns the world alone.
    let render = |game: &TopDown, shot: &mut Offscreen, name: &str| {
        let hidden: Vec<String> = game.session().hidden().map(str::to_string).collect();
        let hud = game.hud_layout(W, H);
        let (scene, cam) = (game.session().scene(), game.view());
        shot.save_png(&out.join(name), scene, 0.0, &cam, hidden.iter().map(String::as_str), Some(&hud)).unwrap();
        let with_hud = shot.render(scene, 0.0, &cam, hidden.iter().map(String::as_str), Some(&hud)).unwrap();
        let world = shot.render(scene, 0.0, &cam, hidden.iter().map(String::as_str), None).unwrap();
        (with_hud, world)
    };

    let (start, _) = render(&game, &mut shot, "start.png");
    let gate_px = game.view().world_to_screen(Vec3::new(0.0, 1.2, -2.0), W, H).unwrap();
    let gate = sample(&start, gate_px, 3);
    assert!(gate[0] > 120.0 && gate[0] > gate[1] * 2.0, "the closed gate is red at {gate_px:?}: {gate:?}");
    let marker_px = game.view().world_to_screen(game.session().player_feet() + Vec3::Y * 1.56, W, H).unwrap();
    let marker = sample(&start, marker_px, 2);
    assert!(marker.iter().all(|c| *c > 150.0), "the white player marker is under the camera at {marker_px:?}: {marker:?}");

    for p in [Vec3::new(-5.0, 0.0, 2.5), Vec3::new(5.0, 0.0, 2.5), Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 0.0, -5.5)] {
        click_and_walk(&mut game, p, 900);
    }
    assert_eq!(game.session().outcome(), Some("escaped"));
    let (end, end_world) = render(&game, &mut shot, "escaped.png");
    let gate_px = game.view().world_to_screen(Vec3::new(0.0, 1.2, -2.0), W, H).unwrap();
    let gate = sample(&end_world, gate_px, 3);
    assert!(!(gate[0] > 120.0 && gate[0] > gate[1] * 2.0), "the opened gate is no longer drawn at {gate_px:?}: {gate:?}");
    // The outcome banner: its opaque dark panel with gold text covers the middle of the window. (The exit plate under
    // the camera is gold too, so count only pixels the HUD changed.)
    let banner = (0..W as usize * H as usize)
        .filter(|i| {
            let (x, y) = ((i % W as usize) as u32, (i / W as usize) as u32);
            let (c, w) = (&end[i * 4..i * 4 + 3], &end_world[i * 4..i * 4 + 3]);
            (W / 3..2 * W / 3).contains(&x) && (H / 3..2 * H / 3).contains(&y) && c[0] > 220 && c[1] > 170 && c[2] < 120 && c != w
        })
        .count();
    assert!(banner > 500, "the ESCAPED banner is drawn ({banner} HUD gold pixels in the centre)");
}
