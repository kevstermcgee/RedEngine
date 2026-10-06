//! The 2D path without a browser: every example game validates and verifies, the capability matrix says what it says, and the mistakes an author (or a smaller model) makes are
//! refused early, with the path, the reason and the way out. The browser half is `web verify` (CI stage `web`) and `tests/web2d_package.rs`.
//!
//! These prove the *messages and the checks*. They do not prove any game is fun.

use red_engine2::tools::game2d;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

fn examples() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/2d");
    let mut v: Vec<PathBuf> =
        std::fs::read_dir(&dir).expect("examples/2d").flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().ends_with(".game2d.json")).collect();
    v.sort();
    v
}

fn example(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("examples/2d/{name}.game2d.json"))).unwrap()).unwrap()
}

/// A copy of an example with `edit` applied, written to a scratch file; returns the validate report.
fn mutate(name: &str, edit: impl FnOnce(&mut Value)) -> game2d::Report {
    static N: AtomicU32 = AtomicU32::new(0);
    let mut g = example(name);
    edit(&mut g);
    let dir = std::env::temp_dir().join(format!("re2_web2d_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.game2d.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&g).unwrap()).unwrap();
    let r = game2d::validate(&path);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

fn refused(r: &game2d::Report, parts: &[&str]) {
    assert!(!r.ok, "this mistake must be refused, got:\n{}", r.text);
    for p in parts {
        assert!(r.text.contains(p), "the message must contain `{p}`:\n{}", r.text);
    }
}

#[test]
fn every_example_game_validates_and_passes_everything_it_claims() {
    let games = examples();
    assert!(games.len() >= 3, "three different games are the point: {games:?}");
    for g in &games {
        let v = game2d::validate(g);
        assert!(v.ok, "{}:\n{}", g.display(), v.text);
        let r = game2d::verify(g, None);
        assert!(r.ok, "{}:\n{}", g.display(), r.text);
        for claim in ["[simulation]", "[render]", "[audio-waveform]", "PROVEN HERE", "NOT PROVEN HERE"] {
            assert!(r.text.contains(claim), "{} verify prints no `{claim}`:\n{}", g.display(), r.text);
        }
    }
}

#[test]
fn the_games_are_genuinely_different_kinds_of_game() {
    use red2d::game::{parse, KeyMode, Move, When};
    let defs: Vec<_> = examples().iter().map(|p| parse(&std::fs::read_to_string(p).unwrap()).unwrap()).collect();
    let topdown = defs.iter().any(|d| d.prefabs.iter().any(|p| matches!(p.mv, Move::Keys { mode: KeyMode::TopDown, .. })));
    let platformer = defs.iter().any(|d| {
        d.prefabs.iter().any(|p| matches!(p.mv, Move::Keys { mode: KeyMode::Platformer, .. }))
            && d.prefabs.iter().any(|p| p.body.is_some_and(|b| b.gravity > 0.0))
    });
    let mouse = defs.iter().any(|d| d.rules.iter().any(|r| matches!(&r.when, When::Click(_))));
    assert!(topdown && platformer && mouse, "arcade (top-down keys), platformer (gravity + jump) and mouse-driven management must all be present");
    let ids: std::collections::BTreeSet<&str> = defs.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids.len(), defs.len(), "ids are unique");
    // Collision, audio, UI, animation, particles, persistence and settings are each exercised by at least one game.
    assert!(defs.iter().all(|d| !d.sounds.is_empty() && !d.ui.is_empty() && !d.scenarios.is_empty()));
    assert!(defs.iter().any(|d| d.sprites.iter().any(|s| s.frames.len() > 1)), "animation");
    assert!(defs.iter().any(|d| d.prefabs.iter().any(|p| p.emit.is_some())), "particles");
    assert!(defs.iter().any(|d| !d.persist.is_empty()), "persisted progress");
    assert!(defs.iter().any(|d| !d.music.is_empty()), "music");
    assert!(defs.iter().any(|d| d.prefabs.iter().any(|p| !p.collide.is_empty())), "collision");
}

// ---- unsupported combinations fail early and say what to do ----------------------------------------------------------------------------------------

#[test]
fn a_3d_game_cannot_target_the_browser() {
    refused(
        &mutate("coin-dash", |g| g["capabilities"]["presentation"] = json!("3d")),
        &["3D cannot target `web`", "not supported", "wgpu", "declare `presentation: \"2d\"`", "drop `web` from `platforms`"],
    );
}

#[test]
fn a_browser_game_cannot_ask_for_native_udp_networking() {
    refused(
        &mutate("coin-dash", |g| g["capabilities"]["networking"] = json!("authoritative")),
        &[
            "capabilities.networking",
            "Browser target cannot use the native UDP transport",
            "Supported networking for web games: offline",
            "architecturally prepared",
        ],
    );
}

#[test]
fn a_native_window_for_a_2d_game_is_prepared_not_built_and_says_so() {
    refused(
        &mutate("coin-dash", |g| g["capabilities"]["platforms"] = json!(["web", "windows"])),
        &["capabilities.platforms[1]", "cannot target `windows`", "prepared", "not built yet"],
    );
}

#[test]
fn unknown_platforms_and_empty_declarations_are_refused_with_suggestions() {
    refused(&mutate("coin-dash", |g| g["capabilities"]["platforms"] = json!(["wev"])), &["wev", "did you mean `web`"]);
    refused(&mutate("coin-dash", |g| g["capabilities"]["platforms"] = json!([])), &["platforms"]);
    refused(&mutate("coin-dash", |g| g["capabilities"]["platforms"] = json!(["macos"])), &["macos", "no macOS build"]);
    refused(&mutate("coin-dash", |g| g["capabilities"]["networking"] = json!("p2p")), &["networking", "p2p", "offline"]);
}

#[test]
fn saved_progress_needs_the_declaration_and_the_declaration_needs_a_real_variable() {
    refused(&mutate("coin-dash", |g| g["capabilities"]["persistence"] = json!(["settings"])), &["`persist` saves variables", "add \"progress\""]);
    refused(&mutate("coin-dash", |g| g["persist"] = json!(["bset"])), &["persist[0]", "not a declared variable", "did you mean `best`"]);
    refused(&mutate("coin-dash", |g| g["capabilities"]["persistence"] = json!(["settings", "cloud"])), &["cloud"]);
    refused(&mutate("coin-dash", |g| g["capabilities"]["persistence"] = json!(["progress"])), &["music setting", "settings"]);
}

#[test]
fn input_that_is_used_must_be_declared() {
    refused(&mutate("coin-dash", |g| g["capabilities"]["input"] = json!(["keyboard"])), &["neither `mouse` nor `touch` is declared"]);
    refused(&mutate("coin-dash", |g| g["capabilities"]["input"] = json!(["mouse"])), &["keyboard", "not declared"]);
}

#[test]
fn a_3d_concept_in_a_2d_game_is_an_unknown_field_not_a_silent_ignore() {
    refused(&mutate("coin-dash", |g| g["lights"] = json!([{"type": "sun"}])), &["lights", "unknown"]);
    refused(&mutate("coin-dash", |g| g["prefabs"]["pillar"]["shape"] = json!({"mesh": "cube"})), &["mesh", "unknown"]);
    refused(&mutate("coin-dash", |g| g["prefabs"]["pillar"]["rotation"] = json!([0, 90, 0])), &["rotation", "unknown"]);
}

// ---- references that resolve to nothing --------------------------------------------------------------------------------------------------------------

#[test]
fn a_missing_sprite_sound_prefab_tag_variable_or_scene_id_says_what_exists() {
    refused(&mutate("coin-dash", |g| g["prefabs"]["coin"]["shape"] = json!({"sprite": "coins"})), &["no sprite `coins`", "did you mean `coin`", "known:"]);
    refused(&mutate("coin-dash", |g| g["rules"][0]["do"][2] = json!({"play": "pik"})), &["no sound `pik`", "did you mean `pick`"]);
    refused(&mutate("coin-dash", |g| g["scene"][1]["prefab"] = json!("pilar")), &["no prefab `pilar`", "did you mean `pillar`"]);
    refused(&mutate("coin-dash", |g| g["rules"][0]["when"] = json!({"touch": ["player", "coins"]})), &["no tag `coins`", "did you mean `coin`"]);
    refused(&mutate("coin-dash", |g| g["rules"][0]["do"][0] = json!({"add": ["scor", 1]})), &["no variable `scor`", "did you mean `score`"]);
    refused(&mutate("coin-dash", |g| g["rules"][3]["if"] = json!("scoer >= 12")), &["scoer"]);
    refused(&mutate("coin-dash", |g| g["checks"]["scenarios"][2]["expect"][0]["entity"] = json!("q")), &["no scene id `q`", "did you mean `p`"]);
}

#[test]
fn a_sprite_that_cannot_be_drawn_is_refused_with_the_row_and_column() {
    refused(&mutate("coin-dash", |g| g["sprites"]["player"]["rows"][3] = json!(".dssssd")), &["sprites.player", "row 3", "equally wide"]);
    refused(&mutate("coin-dash", |g| g["sprites"]["player"]["rows"][3] = json!(".dsZZsd.")), &["`Z`", "not in `palette`"]);
    refused(&mutate("coin-dash", |g| g["sprites"]["player"]["palette"]["d"] = json!("blue")), &["not a color", "#rrggbb"]);
}

#[test]
fn a_sound_that_cannot_be_rendered_is_refused_by_the_shared_voice_checker() {
    refused(&mutate("coin-dash", |g| g["sounds"]["pick"]["layers"][0]["sinee"] = json!(880)), &["sounds.pick", "sinee"]);
    refused(&mutate("coin-dash", |g| g["sounds"]["pick"]["seconds"] = json!(99)), &["sounds.pick", "seconds"]);
}

// ---- checks that prove nothing -----------------------------------------------------------------------------------------------------------------------

#[test]
fn a_scenario_that_asserts_nothing_or_does_nothing_is_refused() {
    refused(&mutate("coin-dash", |g| g["checks"]["scenarios"][0]["expect"] = json!([])), &["non-empty `expect`", "asserts nothing"]);
    refused(&mutate("coin-dash", |g| g["checks"]["scenarios"][0]["script"] = json!([])), &["non-empty `script`"]);
    refused(&mutate("coin-dash", |g| g["checks"]["scenarios"][0]["expect"] = json!([{"var": "score"}])), &["needs a comparison"]);
    refused(&mutate("coin-dash", |g| g["rules"][0]["do"] = json!([])), &["`do` is empty"]);
}

#[test]
fn a_game_with_no_scenarios_fails_verify_even_though_it_validates() {
    static N: AtomicU32 = AtomicU32::new(0);
    let mut g = example("coin-dash");
    g["checks"] = json!({});
    let path = std::env::temp_dir().join(format!("re2_noscn_{}_{}.game2d.json", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, g.to_string()).unwrap();
    assert!(game2d::validate(&path).ok);
    let r = game2d::verify(&path, None);
    assert!(!r.ok && r.text.contains("no `checks.scenarios`"), "{}", r.text);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_regression_in_the_rules_fails_the_playthrough_and_says_what_was_found() {
    static N: AtomicU32 = AtomicU32::new(0);
    let mut g = example("coin-dash");
    g["rules"][0]["do"][0] = json!({"add": ["score", 0]}); // picking up a coin no longer scores
    let path = std::env::temp_dir().join(format!("re2_regress_{}_{}.game2d.json", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, g.to_string()).unwrap();
    let r = game2d::verify(&path, None);
    assert!(!r.ok, "{}", r.text);
    assert!(r.text.contains("FAIL") && r.text.contains("expected score") && r.text.contains("found 0"), "{}", r.text);
    let _ = std::fs::remove_file(path);
}

// ---- the commands -----------------------------------------------------------------------------------------------------------------------------------

#[test]
fn capabilities_answers_a_question_the_whole_matrix_and_a_games_declaration() {
    let q = |w: &[&str]| game2d::capabilities(None, &w.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
    let r = q(&["2d", "web"]);
    assert!(r.ok && r.text.contains("SUPPORTED"), "{}", r.text);
    let r = q(&["3d", "web"]);
    assert!(!r.ok && r.text.contains("NOT SUPPORTED") && r.text.contains("wgpu"), "{}", r.text);
    let r = q(&["2d", "web", "authoritative"]);
    assert!(!r.ok && r.text.contains("Browser target cannot use the native UDP transport"), "{}", r.text);
    let r = q(&["3d", "linux", "authoritative"]);
    assert!(r.ok, "{}", r.text);
    let all = q(&[]);
    assert!(all.ok && all.text.contains("PRESENTATION x PLATFORM"), "{}", all.text);
    assert!(game2d::capabilities(None, &["banana".to_string()]).is_err(), "a question with no presentation and platform is an error that lists them");
    let r = game2d::capabilities(Some(&examples()[0]), &[]).unwrap();
    assert!(r.ok && r.text.contains("2d on web"), "{}", r.text);
}

#[test]
fn frame_writes_the_picture_the_browser_would_show_at_any_window_shape() {
    let dir = std::env::temp_dir().join(format!("re2_frames_{}", std::process::id()));
    let game = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/2d/coin-dash.game2d.json");
    let msg = game2d::frame(&game, &dir.join("a.png"), None, 0.0, None).unwrap();
    assert!(msg.contains("320x180") && msg.contains("same CPU renderer the browser uses"), "{msg}");
    let msg = game2d::frame(&game, &dir.join("b.png"), Some("a bot collects coins and wins"), 5.0, Some((1000, 400))).unwrap();
    assert!(msg.contains("1000x400") && msg.contains("tick 300"), "{msg}");
    let img = image::open(dir.join("b.png")).unwrap().to_rgba8();
    assert_eq!((img.width(), img.height()), (1000, 400));
    assert_eq!(img.get_pixel(2, 200).0, [0, 0, 0, 255], "a letterbox bar is black");
    assert!(game2d::frame(&game, &dir.join("c.png"), Some("nope"), 1.0, None).unwrap_err().contains("no scenario `nope`"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn routing_is_by_file_so_the_3d_verbs_still_mean_3d() {
    assert!(game2d::is_game(Path::new("anything.game2d.json")));
    assert!(!game2d::is_game(Path::new("examples/house.json")), "a 3D scene is not a 2D game");
    assert!(!game2d::is_game(Path::new("examples/nonexistent.json")));
    assert!(game2d::is_game(&examples()[0]));
}

/// A fresh author's first `search` for a 2D question returned only 3D fragments; the 2D reference and docs are in the corpus now.
#[test]
fn searching_for_a_2d_question_finds_the_2d_material() {
    for q in ["spawn falling objects random position 2d", "how do I publish a 2d game to the browser", "sprite palette rows animation frames"] {
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_red_engine2")).args(["search", q, "--limit", "3"]).output().unwrap();
        let text = String::from_utf8_lossy(&o.stdout);
        assert!(
            text.contains("describe 2d") || text.contains("PUBLISHING_2D") || text.contains("WEB_PLATFORM"),
            "`search {q}` found no 2D material in its top 3:\n{text}"
        );
    }
}

/// Every example says what a phone shows, and a phone controller that could not work is refused.
#[test]
fn every_example_has_a_phone_controller_and_the_mistakes_are_refused() {
    use red2d::controls::PadLayout;
    let layouts: Vec<(String, PadLayout, bool)> = examples()
        .iter()
        .map(|p| {
            let d = red2d::game::parse(&std::fs::read_to_string(p).unwrap()).unwrap();
            assert!(d.caps.input.contains(&red2d::caps::Input::Touch), "{} should declare touch", d.id);
            (d.id.clone(), d.controls.layout, d.controls.visible())
        })
        .collect();
    assert!(layouts.contains(&("coin-dash".into(), PadLayout::Dpad, true)), "{layouts:?}");
    assert!(layouts.contains(&("moon-hopper".into(), PadLayout::Platformer, true)), "{layouts:?}");
    assert!(layouts.contains(&("tiny-station".into(), PadLayout::Tap, true)), "a tap game shows only its pause button: {layouts:?}");
    refused(&mutate("coin-dash", |g| g["controls"] = json!("platformer")), &["controls.layout", "platformer", "`platformer` mode"]);
    refused(
        &mutate("coin-dash", |g| g["controls"] = json!({"layout": "dpad", "b": "BOOST"})),
        &["controls.b", "drives `secondary`", "nothing in the game reads it"],
    );
    refused(&mutate("coin-dash", |g| g["controls"] = json!("dpda")), &["did you mean `dpad`"]);
    refused(
        &mutate("coin-dash", |g| {
            g["controls"] = json!("dpad");
            g["capabilities"]["input"] = json!(["keyboard", "mouse"]);
        }),
        &["`touch` is not declared"],
    );
}

#[test]
fn hybrid_games_use_3d_where_asked_and_everything_else_stays_2d() {
    use red2d::game::parse;
    for name in ["warden-arena", "lantern-yard"] {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("examples/2d/{name}.game2d.json"))).unwrap();
        let d = parse(&text).unwrap();
        assert_eq!(d.caps.presentation, red2d::caps::Presentation::Hybrid, "{name}");
        assert!(d.uses_3d() && d.ui.iter().any(|w| matches!(w.kind, red2d::game::WidgetKind::Minimap { .. })), "{name} has a 3D element and a minimap");
    }
    let q = |w: &[&str]| game2d::capabilities(None, &w.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
    let r = q(&["hybrid", "web"]);
    assert!(
        r.ok && r.text.contains("SUPPORTED") && r.text.contains("online distribution: SUPPORTED") && r.text.contains("install distribution: SUPPORTED"),
        "{}",
        r.text
    );
    let r = q(&["3d", "windows", "install"]);
    assert!(r.ok && r.text.contains("install distribution: SUPPORTED") && !r.text.contains("online distribution"), "{}", r.text);
    let r = q(&["3d", "web", "online"]);
    assert!(!r.ok && r.text.contains("NOT SUPPORTED"), "{}", r.text);
}

#[test]
fn a_3d_element_needs_the_hybrid_declaration_and_its_mistakes_are_named() {
    // A model in a game that says it is 2D: refused, with the way out.
    refused(&mutate("warden-arena", |g| g["capabilities"]["presentation"] = json!("2d")), &["uses 3D elements", "shape.model", "declare \"hybrid\""]);
    // A hybrid game with nothing 3D in it is refused too: declare what it is.
    refused(&mutate("coin-dash", |g| g["capabilities"]["presentation"] = json!("hybrid")), &["hybrid", "no 3D element"]);
    refused(&mutate("warden-arena", |g| g["models"]["warden"]["parts"][0]["shape"] = json!("bos")), &["bos", "did you mean `box`"]);
    refused(&mutate("warden-arena", |g| g["prefabs"]["warden"]["shape"]["model"] = json!("wardn")), &["no model `wardn`", "did you mean `warden`"]);
    refused(&mutate("lantern-yard", |g| g["view"]["world3d"]["plane"] = json!("floor")), &["\"ground\"", "\"wall\""]);
    refused(&mutate("warden-arena", |g| g["ui"][5]["view3d"]["camera"]["eye"] = json!([0, 1.5, 0])), &["same point"]);
    refused(&mutate("warden-arena", |g| g["ui"][6]["minimap"]["colors"] = json!({"nobody": "#fff"})), &["no tag `nobody`"]);
}
