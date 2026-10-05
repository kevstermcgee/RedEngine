//! The real client (`re2`), run with no window (ADR 2026-09-28-seeing-what-the-player-sees): its exit code and its state dump are enough to assert what a player would see,
//! so "the bots are invisible" is a failing test instead of a bug report. These tests start the real binary, so they cover the glue that decides who gets a body
//! (`bin/re2` and `net::session`), which unit tests of the pure parts cannot.

// `re2` needs the gfx feature; cargo still sets CARGO_BIN_EXE_re2 without it, pointing at a binary that was never built.
#![cfg(feature = "gfx")]

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

fn lab() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")
}

/// A fresh scratch directory for one test.
fn scratch(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("re2_client_headless_{}_{}_{name}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn re2(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_re2")).args(args).output().expect("start re2")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn the_debug_help_lists_the_switches_and_the_headless_flags() {
    let o = re2(&["--debug-help"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("RE2_STATS") && t.contains("RE2_AUTOAIM") && t.contains("F12") && t.contains("--headless") && t.contains("--playtest"), "{t}");
}

#[test]
fn a_script_plays_the_solo_game_and_the_dump_says_what_the_player_would_see() {
    let dir = scratch("solo");
    let script = dir.join("play.json");
    let dump = dir.join("state.json");
    std::fs::write(
        &script,
        json!({"steps": [
            {"wait": 0.5}, {"look": {"yaw": 90, "pitch": 0}}, {"hold": ["forward"], "secs": 0.5},
            {"expect": {"at": "/phase", "eq": "playing"}}, {"expect": {"at": "/secs", "min": 0.9, "max": 5}},
            {"expect": {"at": "/crosshair/state", "exists": true}}, {"snapshot": "walked"}]})
        .to_string(),
    )
    .unwrap();
    let o = re2(&[lab().to_str().unwrap(), "--as", "human", "--headless", "--script", script.to_str().unwrap(), "--dump", dump.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    assert_eq!(d["schema"], "re2-dump/1");
    assert_eq!(d["headless"], true);
    assert!(d["snapshots"]["walked"].is_object(), "the script's snapshot step is in the dump: {d}");
    assert!(d["failures"].as_array().unwrap().is_empty(), "{d}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn split_screen_co_op_gives_each_player_their_own_body_and_the_script_plays_them_in_turn() {
    let dir = scratch("coop");
    let script = dir.join("play.json");
    let dump = dir.join("state.json");
    std::fs::write(
        &script,
        json!({"steps": [
            {"wait": 0.3}, {"player": 2}, {"look": {"yaw": 90, "pitch": 0}}, {"hold": ["forward"], "secs": 0.6},
            {"player": 3}, {"look": {"yaw": 270, "pitch": 0}}, {"hold": ["forward"], "secs": 0.6}, {"wait": 0.2},
            {"expect": {"at": "/players/0/slot", "eq": 1}}, {"expect": {"at": "/players/2/device", "eq": "Pad(1)"}},
            {"expect": {"at": "/players/1/yaw_deg", "min": 89, "max": 91}}, {"expect": {"at": "/players/2/yaw_deg", "min": 269, "max": 271}},
            {"snapshot": "end"}]})
        .to_string(),
    )
    .unwrap();
    let o = re2(&[lab().to_str().unwrap(), "--as", "human", "--players", "3", "--script", script.to_str().unwrap(), "--dump", dump.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    let players = d["players"].as_array().expect("a co-op dump lists the players");
    assert_eq!(players.len(), 3, "{d}");
    let at = |i: usize| (players[i]["pos"][0].as_f64().unwrap(), players[i]["pos"][2].as_f64().unwrap());
    let apart = ((at(1).0 - at(2).0).powi(2) + (at(1).1 - at(2).1).powi(2)).sqrt();
    assert!(apart > 2.0, "the two guests were played in opposite directions and stand apart ({apart:.1} m): {players:?}");
    // The first player was never given the controls after the start.
    assert_eq!(players[0]["device"], "KeyboardMouse");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_single_player_game_has_no_players_list_and_a_script_cannot_play_a_second_player() {
    let dir = scratch("solo-players");
    let script = dir.join("play.json");
    std::fs::write(&script, json!({"steps": [{"player": 2}]}).to_string()).unwrap();
    let o = re2(&[lab().to_str().unwrap(), "--as", "human", "--script", script.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("--players"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_failed_expectation_is_exit_code_1_and_says_what_was_expected() {
    let dir = scratch("failing");
    let script = dir.join("play.json");
    std::fs::write(&script, json!({"steps": [{"wait": 0.2}, {"expect": {"at": "/remote/drawn", "eq": 7, "msg": "8 fighters means 7 drawn"}}]}).to_string())
        .unwrap();
    let o = re2(&[lab().to_str().unwrap(), "--as", "human", "--headless", "--script", script.to_str().unwrap()]);
    let t = text(&o);
    assert_eq!(o.status.code(), Some(1), "{t}");
    assert!(t.contains("FAILED:") && t.contains("8 fighters means 7 drawn"), "{t}");
    let bad = dir.join("bad.json");
    std::fs::write(&bad, r#"{"steps":[{"fier":3}]}"#).unwrap();
    let o = re2(&[lab().to_str().unwrap(), "--as", "human", "--headless", "--script", bad.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2), "a script that does not parse never starts the game: {}", text(&o));
    assert!(text(&o).contains("did you mean 'fire'"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(dir);
}

/// The Trigger Happy bug, as a test: every human is forced to one body while the bots wear four others. The client used to size its avatar pool for the humans' body only and
/// skip everybody it had no avatar for, so the bots were invisible in the shipped game and every test with a human opponent passed.
#[test]
fn bots_in_bodies_the_humans_are_not_forced_to_are_drawn() {
    let dir = scratch("roster");
    let mut map: Value = serde_json::from_str(&std::fs::read_to_string(lab()).unwrap()).unwrap();
    map["player"] = json!({"humans_play_as": "human"});
    // The duel spawns stand in one room: interest management (a player is told about their own room and its neighbours) cannot hide a bot from the human here.
    let duel: Vec<Value> = map["spawns"].as_array().unwrap().iter().filter(|s| s["group"] == "duel").cloned().collect();
    assert!(duel.len() >= 4, "the Test Lab has four duel spawns");
    map["spawns"] = json!(duel);
    map["bots"] = json!({"fill": 4, "roster": [
        {"name": "Cow", "character": "cowboy"}, {"name": "Wiz", "character": "wizard"}, {"name": "Ali", "character": "alien"}, {"name": "Rob", "character": "robot"}]});
    let map_file = dir.join("arena.json");
    std::fs::write(&map_file, map.to_string()).unwrap();
    let script = dir.join("play.json");
    let dump = dir.join("state.json");
    std::fs::write(
        &script,
        json!({"steps": [
            {"wait_for": {"at": "/online/connected", "eq": true, "within": 20, "msg": "never joined the hosted match"}},
            {"wait_for": {"at": "/remote/in_view", "min": 3, "within": 20, "msg": "the bots never showed up in the snapshots"}},
            {"wait": 1.0},
            {"expect": {"at": "/remote/drawn", "eq": 3, "msg": "4 fighters means 3 drawn"}},
            {"expect": {"at": "/remote/undrawn", "eq": 0, "msg": "nobody may be invisible"}},
            {"snapshot": "end"}]})
        .to_string(),
    )
    .unwrap();
    let o = re2(&[
        map_file.to_str().unwrap(),
        "--host",
        "--as",
        "human",
        "--name",
        "Tester",
        "--headless",
        "--script",
        script.to_str().unwrap(),
        "--dump",
        dump.to_str().unwrap(),
    ]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    let mut bodies: Vec<String> = d["remote"]["players"].as_array().unwrap().iter().map(|p| p["body"].as_str().unwrap().to_lowercase()).collect();
    bodies.sort();
    assert_eq!(bodies.len(), 3, "{d}");
    assert!(bodies.iter().all(|b| ["cowboy", "wizard", "alien", "robot"].contains(&b.as_str())), "the bots wear their own bodies, not the humans': {bodies:?}");
    for p in d["remote"]["players"].as_array().unwrap() {
        assert_eq!(p["body"], p["avatar_body"], "each bot is drawn in its own avatar, not a stand-in: {p}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// One scene, one vocabulary: `approach` / `look_at` / `interact` by object id play the same in the real client as in a `checks.sim` scenario
/// (`tests/object_actions.rs` runs this scene through the simulator).
fn parcel_scene() -> Value {
    json!({
        "camera": {"position": [0, 6, -6], "target": [0, 0, 2]},
        "player": {"mode": "peaceful"},
        "spawns": [{"id": "s", "position": [0, 0, 0], "yaw_deg": 0}],
        "objects": [
            {"id": "floor", "type": "plane", "size": [40, 40], "position": [0, 0, 0]},
            {"id": "far_crate", "type": "prop", "prop": "crate", "position": [6, 0, 9]},
            {"id": "wall", "type": "box", "size": [8, 3, 0.3], "position": [0, 1.5, 4.5]},
            {"id": "behind_wall", "type": "prop", "prop": "crate", "position": [0, 0, 6.5]}
        ]
    })
}

#[test]
fn interact_by_id_walks_to_a_far_crate_and_picks_it_up_in_the_real_client() {
    let dir = scratch("objects");
    let scene = dir.join("parcel.json");
    std::fs::write(&scene, parcel_scene().to_string()).unwrap();
    let script = dir.join("play.json");
    std::fs::write(
        &script,
        json!({"steps": [
            {"interact": "far_crate"},
            {"expect": {"at": "/player/carrying", "eq": true, "msg": "interact by id should leave the crate in hand"}},
            {"say": "picked up"}]})
        .to_string(),
    )
    .unwrap();
    let o = re2(&[scene.to_str().unwrap(), "--as", "human", "--headless", "--script", script.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    // A wall in the way fails the step by name instead of hanging or passing.
    std::fs::write(&script, json!({"steps": [{"approach": "behind_wall"}, {"say": "unreachable"}]}).to_string()).unwrap();
    let o = re2(&[scene.to_str().unwrap(), "--as", "human", "--headless", "--script", script.to_str().unwrap()]);
    let t = text(&o);
    assert_eq!(o.status.code(), Some(1), "{t}");
    assert!(t.contains("approach `behind_wall`") && t.contains("in the way"), "{t}");
    // A typo in the id is not silently ignored.
    std::fs::write(&script, json!({"steps": [{"interact": "far_crat"}]}).to_string()).unwrap();
    let o = re2(&[scene.to_str().unwrap(), "--as", "human", "--headless", "--script", script.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("no such top-level object"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(dir);
}

/// A tiny game that ends 0.4 s after it starts, with a `ui` block: a start card the game waits for, an end card with a restart button, and a HUD with
/// a friendly counter and objective (ADR 2026-10-03-a-scene-declared-game-ui).
fn tiny_game() -> Value {
    json!({
        "camera": {"position": [0, 6, -6], "target": [0, 0, 2]},
        "player": {"mode": "peaceful"},
        "spawns": [{"id": "s", "position": [0, 0, 0], "yaw_deg": 0}],
        "vars": {"n": 0},
        "rules": [{"id": "finish", "when": {"after": 0.4}, "do": [{"add": ["n", 1]}, {"end": "victory"}]}],
        "ui": {
            "title": "Tiny game",
            "counters": [{"var": "n", "of": 1, "label": "Wins"}],
            "objective": "Wait for it",
            "start": {"title": "Tiny game", "text": "Press start.", "button": "Go"},
            "end": {"victory": {"title": "Done", "text": "Score {n}.", "button": "Again"}}
        },
        "objects": [{"id": "floor", "type": "plane", "size": [20, 20], "position": [0, 0, 0]}]
    })
}

#[test]
fn the_start_card_holds_the_game_until_pressed_and_the_end_card_restarts_it() {
    let dir = scratch("cards");
    let scene = dir.join("tiny.json");
    std::fs::write(&scene, tiny_game().to_string()).unwrap();
    let script = dir.join("play.json");
    let dump = dir.join("state.json");
    std::fs::write(
        &script,
        json!({"steps": [
            {"wait_for": {"at": "/card/kind", "eq": "start", "within": 3}},
            {"expect": {"at": "/card/title", "eq": "Tiny game"}},
            {"expect": {"at": "/card/button/id", "eq": "start"}},
            {"wait": 1.0},
            {"expect": {"at": "/rules/ended", "eq": null, "msg": "the game must wait for the start card"}},
            {"press": "start"},
            {"expect": {"at": "/card", "eq": null}},
            {"wait_for": {"at": "/card/kind", "eq": "end", "within": 5}},
            {"expect": {"at": "/card/title", "eq": "Done"}},
            {"expect": {"at": "/card/text", "eq": "Score 1."}},
            {"expect": {"at": "/card/button/label", "eq": "Again"}},
            {"press": "restart"},
            {"expect": {"at": "/card", "eq": null, "msg": "a restart does not show the start card again"}},
            {"expect": {"at": "/rules/ended", "eq": null}},
            {"wait_for": {"at": "/card/kind", "eq": "end", "within": 5}},
            {"snapshot": "second_end"}]})
        .to_string(),
    )
    .unwrap();
    let o = re2(&[scene.to_str().unwrap(), "--as", "human", "--headless", "--script", script.to_str().unwrap(), "--dump", dump.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    assert!(d["snapshots"]["second_end"]["card"]["kind"] == "end", "the restarted game ran to its end again: {d}");
    // The HUD a player reads is the declared one: a friendly label and counter, not `N: 1`.
    let lines: Vec<String> = d["hud"]["lines"].as_array().unwrap().iter().filter_map(|l| l["text"].as_str().map(str::to_string)).collect();
    assert!(lines.iter().any(|l| l == "WINS: 1 / 1") && lines.iter().any(|l| l == "WAIT FOR IT"), "{lines:?}");
    // A button that is not there fails by name.
    std::fs::write(&script, json!({"steps": [{"press": "restart"}]}).to_string()).unwrap();
    let o = re2(&[scene.to_str().unwrap(), "--as", "human", "--headless", "--script", script.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("press `restart`") && text(&o).contains("start card has `start`"), "{}", text(&o));
    let _ = std::fs::remove_dir_all(dir);
}

/// What an online player reads when the game ends. End cards (like start cards) are an offline presentation: the card replaces the plain outcome banner only where a card
/// is actually shown. A hosted/online client shows no card, so a scene that declares an end card must still show its players the outcome as the plain banner: an online
/// player never loses the result because the scene was written for the card.
#[test]
fn an_online_player_still_reads_the_outcome_when_the_scene_declares_an_end_card() {
    let dir = scratch("online_outcome");
    let scene = dir.join("tiny.json");
    std::fs::write(&scene, tiny_game().to_string()).unwrap(); // a `ui` block with a start card and a "victory" end card; the rule ends the game 0.4 s in
    let script = dir.join("play.json");
    let dump = dir.join("state.json");
    std::fs::write(
        &script,
        json!({"steps": [
            {"wait_for": {"at": "/online/connected", "eq": true, "within": 20, "msg": "never joined the hosted match"}},
            {"wait": 3.0},
            {"snapshot": "end"}]})
        .to_string(),
    )
    .unwrap();
    let o = re2(&[
        scene.to_str().unwrap(),
        "--host",
        "--as",
        "human",
        "--name",
        "Tester",
        "--headless",
        "--script",
        script.to_str().unwrap(),
        "--dump",
        dump.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    let lines: Vec<String> = d["hud"]["lines"].as_array().unwrap().iter().filter_map(|l| l["text"].as_str().map(str::to_string)).collect();
    assert!(d["card"].is_null(), "an online client shows no card: {}", d["card"]);
    assert!(
        lines.iter().any(|l| l.to_lowercase().contains("victory")),
        "the outcome must be on screen online, whatever the scene declares for a card: {lines:?}"
    );
    // The declared HUD is still the one shown beside it (friendly label and counter, the objective), not the generic variable list.
    assert!(lines.iter().any(|l| l == "WINS: 1 / 1"), "{lines:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Split-screen guests do not carry props, and the engine says so instead of letting a game find out: the client names the limit when it starts with `--players`, and a
/// script that makes a guest `interact` with a crate fails at once with the reason and the way out. Player 1, in the same scene, can. If guests are ever given props,
/// this test fails and `splitscreen::GUEST_LIMITS` (and with it `describe multiplayer`) must change with them.
#[test]
fn split_screen_guests_cannot_carry_props_and_the_engine_says_so() {
    let dir = scratch("guest_props");
    let scene = dir.join("parcel.json");
    std::fs::write(&scene, parcel_scene().to_string()).unwrap();
    let run = |steps: Value| {
        let script = dir.join("play.json");
        std::fs::write(&script, json!({"steps": steps}).to_string()).unwrap();
        re2(&[scene.to_str().unwrap(), "--as", "human", "--players", "2", "--headless", "--script", script.to_str().unwrap()])
    };
    // Player 1 picks the crate up (the scene's far crate).
    let first = run(json!([{"interact": "far_crate"}, {"expect": {"at": "/player/carrying", "eq": true, "msg": "player 1 carries it"}}]));
    assert!(first.status.success(), "{}", text(&first));
    assert!(
        text(&first).contains("note: split-screen: this scene has") && text(&first).contains("loose prop"),
        "the client names the limit at start-up: {}",
        text(&first)
    );
    // A guest cannot: the script fails immediately and says why.
    let guest = run(json!([{"player": 2}, {"interact": "far_crate"}]));
    assert_eq!(guest.status.code(), Some(1), "{}", text(&guest));
    let t = text(&guest);
    assert!(t.contains("split-screen guest") && t.contains("only player 1 can pick up") && t.contains("{\"player\": 1}"), "{t}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_soundscape_follows_the_hour_a_rule_variable_fades_a_layer_in_and_an_event_ducks_the_music() {
    let dir = scratch("soundscape");
    // A flat scene: a clock starting just after sunrise, the nature ambience, one score layer driven by `tension`, and the music ducking when `boom` is raised.
    let scene = dir.join("s.json");
    std::fs::write(dir.join("chase.json"), r#"{"bpm": 100, "bars": 2, "tracks": []}"#).unwrap();
    std::fs::write(
        &scene,
        json!({
            "camera": {"position": [0, 1.6, 0], "target": [0, 1.6, -5]},
            "clock": {"day_secs": 600, "start": 0.27},
            "spawns": [{"id": "s", "position": [0, 0, 0], "yaw_deg": 0}],
            "vars": {"tension": 0},
            "rules": [
                {"id": "rise", "when": {"after": 3}, "do": [{"set": ["tension", 1]}]},
                {"id": "bang", "when": {"after": 6}, "do": [{"emit": "boom"}]}],
            "audio": {"ambience": "nature", "duck": {"events": ["boom"], "depth": 0.5}, "layers": [{"score": "chase.json", "var": "tension", "above": 0.5, "fade": 2}]},
            "objects": [{"id": "floor", "type": "plane", "size": [40, 40], "position": [0, 0, 0]}]
        })
        .to_string(),
    )
    .unwrap();
    let script = dir.join("play.json");
    let dump = dir.join("state.json");
    std::fs::write(&script, json!({"steps": [{"wait": 1}, {"snapshot": "early"}, {"wait": 10}]}).to_string()).unwrap();
    let o = re2(&[scene.to_str().unwrap(), "--as", "human", "--headless", "--script", script.to_str().unwrap(), "--dump", dump.to_str().unwrap()]);
    assert!(o.status.success(), "{}", text(&o));
    let d: Value = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    let audio = &d["audio"];
    assert!(audio.is_object(), "a scene with an audio block reports its soundscape: {d}");
    assert!(audio["beds"]["wind"].as_f64().unwrap() > 0.2, "{audio}");
    assert_eq!(audio["layers"][0]["var"], "tension");
    assert!(audio["layers"][0]["level"].as_f64().unwrap() > 0.99, "after the variable rose the layer is fully in: {audio}");
    assert_eq!(audio["ducks"], 1, "the one boom ducked the music once: {audio}");
    assert_eq!(audio["music"]["dawn"], 1.0, "just after sunrise the mood is dawn: {audio}");
    assert!(d["snapshots"]["early"]["audio"]["layers"][0]["level"].as_f64().unwrap() < 0.01, "and before it rose the layer was out");
}
