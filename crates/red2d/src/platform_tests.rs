//! Tests for what a 2D game asks of the platform and the screen: the flutter (`glide`), a pause made of rules (`pause_if`), the `window`/`save`/`export_music`
//! requests, the `gamepad` variable, HUD images, a level's sky and parallax backdrop, the icon and a music track's title.

use crate::game::parse;
use crate::host::Host;
use crate::sim::{Request, Sim};
use std::sync::Arc;

/// A small game with a floor, a platformer hero and whatever else the test adds at the top level (after a comma) and in `rules`, `ui` and `levels`.
fn text(top: &str, feel: &str, rules: &str, ui: &str) -> String {
    format!(
        r##"{{"game2d":1,"id":"t","title":"T","description":"d",
        "capabilities":{{"presentation":"2d","platforms":["windows","linux"],"networking":"offline","input":["keyboard","mouse","gamepad"],"persistence":["progress","settings"]}},
        "view":{{"width":160,"height":90}},
        "sprites":{{"dot":{{"rows":["ab","ba"],"palette":{{"a":"#ff0000","b":"#0000ff"}}}}}},
        "music":{{"tune":{{"title":"Little Tune","bpm":120,"bars":1,"instruments":{{"p":{{"seconds":0.3,"layers":[{{"sine":1,"decay":9}}]}}}},"tracks":[{{"inst":"p","play":"pattern","pattern":"x...","notes":"1"}}]}}}},
        "vars":{{"menu":0,"picks":0,"best":0}},"persist":["best"],
        "prefabs":{{"floor":{{"tag":"solid","shape":{{"rect":[160,10],"color":"#888"}},"body":{{"type":"static"}}}},
          "hero":{{"tag":"hero","shape":{{"rect":[8,8],"color":"#fc0"}},"body":{{"type":"dynamic","gravity":1000,"max_fall":400}},"collide":["solid"],
            "move":{{"keys":{{"mode":"platformer","speed":60,"jump":300{feel}}}}}}}}},
        "levels":[{{"id":"a","scene":[{{"prefab":"floor","at":[80,85]}},{{"prefab":"hero","at":[40,-400],"id":"p"}}],"music":"tune",
           "sky":["#000000","#ffffff"],"backdrop":[{{"sprites":["dot"],"y":60,"parallax":0.5,"spacing":[2,2],"fill":"#00ff00","scale":2}}]}}],
        "ui":[{ui}],
        "rules":[{rules}]{top}}}"##
    )
}

fn sim(t: &str) -> Sim {
    Sim::new(Arc::new(parse(t).unwrap_or_else(|e| panic!("{e:#?}\n{t}"))), 1)
}

/// Pip's flutter: holding jump while falling caps the fall; letting go (or running out of `glide_time`) falls at full speed again.
#[test]
fn holding_jump_while_falling_glides_for_its_time_and_the_ground_gives_it_back() {
    let fall = |feel: &str, hold: bool| {
        let mut s = sim(&text("", feel, "", ""));
        s.set_action("action", hold);
        s.run_ticks(24);
        s.by_id("p").unwrap().vy
    };
    assert!(fall("", true) > 300.0, "no glide: it falls fast");
    let gliding = fall(",\"glide\":40", true);
    assert!((gliding - 40.0).abs() < 0.5, "held jump: the fall is caught at the glide speed, got {gliding}");
    assert!(fall(",\"glide\":40", false) > 300.0, "jump not held: no glide");
    let mut s = sim(&text("", ",\"glide\":40,\"glide_time\":0.2", "", ""));
    s.set_action("action", true);
    s.run_ticks(40);
    assert!(s.by_id("p").unwrap().vy > 100.0, "after glide_time it falls at full speed again");
}

/// A pause menu made of rules: while `pause_if` holds nothing moves and the clock stops, but press rules still run.
#[test]
fn pause_if_stands_the_world_and_its_clock_still_but_press_rules_run() {
    let rules = r##"{"when":{"press":"pause"},"do":[{"set":["menu",0]}],"if":"menu == 1"},{"when":{"press":"down"},"do":[{"add":["picks",1]}],"if":"menu == 1"},
        {"when":{"every":0.1},"do":[{"add":["best",1]}]}"##;
    let mut s = sim(&text(",\"pause_if\":\"menu > 0\"", "", rules, ""));
    s.run_ticks(30);
    let (y0, t0, best0) = (s.by_id("p").unwrap().y, s.var("time").unwrap(), s.var("best").unwrap());
    let menu = s.def.vars.iter().position(|(n, _)| n == "menu").unwrap();
    s.vars[menu] = 1.0; // the menu opens
    s.run_ticks(120);
    assert_eq!(s.by_id("p").unwrap().y, y0, "nothing falls while paused");
    assert!(s.var("time").unwrap() - t0 < 0.02, "the game's clock stood still (the variable lags a tick): {} then {}", t0, s.var("time").unwrap());
    assert_eq!(s.var("best").unwrap(), best0, "no timer fired while paused (and none fire in a burst after)");
    s.set_action("down", true);
    s.step();
    s.set_action("down", false);
    s.step();
    assert_eq!(s.var("picks"), Some(1.0), "a press rule ran while paused");
    s.set_action("pause", true);
    s.step();
    s.set_action("pause", false);
    s.run_ticks(12);
    assert_eq!(s.var("menu"), Some(0.0));
    assert!(s.by_id("p").unwrap().y > y0, "the world moves again");
    assert!(s.var("best").unwrap() - best0 <= 3.0, "the timer carried on at its pace, not in a burst: {}", s.var("best").unwrap() - best0);
}

/// `window`, `save` and `export_music` are requests the platform takes; `export_music` names the track playing now and the platform answers with an event.
#[test]
fn window_save_and_export_actions_reach_the_platform_and_it_answers() {
    let rules = r##"{"when":{"press":"action"},"do":[{"window":"fullscreen"},{"save":true},{"export_music":true},{"window":"close"}]},
        {"when":{"event":"music_exported"},"do":[{"set":["picks",7]}]}"##;
    let mut h = Host::default();
    h.init(&text("", "", rules, ""), 1).unwrap();
    h.step(1);
    h.action("action", true);
    h.step(1);
    assert_eq!(h.take_requests(), vec![Request::Fullscreen, Request::Save, Request::ExportMusic(0), Request::Close]);
    assert!(h.take_requests().is_empty(), "taken once");
    assert_eq!(h.music_title(0).as_deref(), Some("Little Tune"));
    h.notify("music_exported");
    h.step(1);
    let snap: serde_json::Value = serde_json::from_str(&h.snapshot_json()).unwrap();
    assert_eq!(snap["vars"]["picks"], 7.0, "the platform's answer is an event the rules see");
}

/// The screen: a level's sky and backdrop are drawn behind everything, a HUD image where it is put, `{music_title}` in text, and the `gamepad` variable.
#[test]
fn sky_backdrop_images_titles_and_the_gamepad_reach_the_screen() {
    let ui = r##"{"image":"dot","at":[150,2],"scale":3},{"text":"{music_title}","at":[2,2],"show":"gamepad == 1"}"##;
    let mut h = Host::default();
    h.init(&text("", "", "", ui), 1).unwrap();
    h.step(1);
    h.render();
    let (w, _) = h.view_size();
    let px = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        h.frame()[i..i + 3].to_vec()
    };
    assert!(px(80, 1)[0] < 20 && (100..180).contains(&px(80, 50)[0]), "the sky fades from black at the top towards white at the bottom: {:?}", px(80, 50));
    assert_eq!(px(80, 70), vec![0, 255, 0], "the backdrop's fill below its band");
    assert_eq!(px(0, 56), vec![255, 0, 0], "the backdrop's first sprite stands on its line at x 0 (scale 2: rows 56..59)");
    assert_eq!(px(150, 2), vec![255, 0, 0], "the HUD image's top-left pixel");
    assert_eq!(px(153, 2), vec![0, 0, 255], "scaled 3x");
    let before = h.frame().to_vec();
    h.set_gamepad(true);
    h.step(1);
    h.render();
    assert_ne!(h.frame(), &before[..], "the `gamepad` variable shows the pad's text");
    let s = sim(&text("", "", "", ""));
    assert_eq!(crate::render::fill_template("{music_title}!", &s), "Little Tune!");
}

/// The game's icon, scaled up crisp for a window; a game without one has none.
#[test]
fn the_icon_is_the_named_sprite_scaled_up_crisp() {
    let mut h = Host::default();
    h.init(&text(",\"icon\":\"dot\"", "", "", ""), 1).unwrap();
    let (w, hh, rgba) = h.icon_rgba(32).unwrap();
    assert_eq!((w, hh), (32, 32));
    assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
    assert_eq!(&rgba[(16 * 4)..(16 * 4 + 4)], &[0, 0, 255, 255], "the second pixel column, 16x");
    let mut none = Host::default();
    none.init(&text("", "", "", ""), 1).unwrap();
    assert!(none.icon_rgba(32).is_none());
}

/// The keys every game shares (fullscreen) and names that would hide a placeholder are refused with the way out.
#[test]
fn reserved_keys_and_names_are_refused() {
    let button = r##"{"button":{"id":"b","label":"B","at":[0,0],"size":[10,10],"key":"KeyF","do":[{"set":["picks",1]}]}}"##;
    let e = parse(&text("", "", "", button)).unwrap_err().join("\n");
    assert!(e.contains("toggles fullscreen in every game"), "{e}");
    let e = parse(&text(",\"icon\":\"nope\"", "", "", "")).unwrap_err().join("\n");
    assert!(e.contains("icon") && e.contains("sprite"), "{e}");
    let e = parse(&text("", "", "", "").replace("\"menu\":0", "\"music_title\":0")).unwrap_err().join("\n");
    assert!(e.contains("music_title"), "{e}");
    let e = parse(&text("", "", r##"{"when":{"start":true},"do":[{"window":"maximise"}]}"##, "")).unwrap_err().join("\n");
    assert!(e.contains("fullscreen") && e.contains("close"), "{e}");
}

#[test]
fn a_bare_trigger_string_and_a_show_inside_a_widget_say_the_fix() {
    let e = parse(&text("", "", r##"{"when":"start","do":[{"set":["picks",1]}]}"##, "")).unwrap_err().join("\n");
    assert!(e.contains(r#"{ "start": true }"#), "{e}");
    let panel = r##"{"panel":{"at":[0,0],"size":[10,10],"show":"picks > 0"}}"##;
    let e = parse(&text("", "", "", panel)).unwrap_err().join("\n");
    assert!(e.contains("beside the widget"), "{e}");
}
