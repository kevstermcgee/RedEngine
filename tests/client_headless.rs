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
