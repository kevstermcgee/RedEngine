//! RedCraft played by its own binary with no window (`redcraft --headless --script`), the way a player plays it: the client's real input,
//! the game mode, the world streamed by its workers, the save written when the run ends and read by the next launch.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scene() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("content/redcraft.json")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("redcraft_play_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One run of `redcraft` in world `world` (seed 1, survival when new) with the saves in `saves`.
fn run(saves: &Path, world: &str, steps: serde_json::Value, dir: &Path, name: &str) -> (Output, serde_json::Value) {
    let script = dir.join(format!("{name}.json"));
    std::fs::write(&script, serde_json::json!({"policy": "idle", "steps": steps}).to_string()).unwrap();
    let dump = dir.join(format!("{name}-state.json"));
    let o = Command::new(env!("CARGO_BIN_EXE_redcraft"))
        .args([scene().to_str().unwrap(), "--headless", "--script", script.to_str().unwrap(), "--dump", dump.to_str().unwrap()])
        .env("RED_REDCRAFT_SAVES", saves)
        .env("REDCRAFT_WORLD", world)
        .env("REDCRAFT_SEED", "1")
        .env("RE2_SAVE_DIR", dir.join("settings"))
        .output()
        .expect("start redcraft");
    let state = std::fs::read_to_string(&dump).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::Value::Null);
    (o, state)
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn a_survival_morning_digs_walks_crafts_builds_and_the_next_launch_continues_it() {
    let dir = scratch("survival");
    let saves = dir.join("worlds");
    let (o, state) = run(
        &saves,
        "Morning",
        serde_json::json!([
            {"expect": {"at": "/redcraft/ready", "eq": true, "within": 30, "msg": "the spawn loads and the player is put on the ground"}},
            {"wait": 1},
            {"expect": {"at": "/redcraft/on_ground", "eq": true}},
            {"press": "key:give:oak_log:2"},
            {"press": "key:give:cobblestone:1"},
            {"look": {"yaw": 0, "pitch": -40}},
            {"press": "key:mine"},
            {"wait": 1.2},
            {"press": "key:stop"},
            {"expect": {"at": "/redcraft/mined", "min": 1, "msg": "a block in front dug by hand"}},
            {"snapshot": "dug"},
            {"press": "key:inventory"},
            {"expect": {"at": "/redcraft/screen", "eq": "inventory"}},
            {"press": "key:click:inv:0"},
            {"press": "key:rclick:craft:0"},
            {"press": "key:click:inv:0"},
            {"press": "key:shift:out"},
            {"expect": {"at": "/redcraft/count/oak_planks", "eq": 4, "msg": "a log makes four planks"}},
            {"press": "key:close"},
            {"press": "key:slot:2"},
            {"wait": 0.2},
            {"press": "key:use"},
            {"wait": 0.5},
            {"expect": {"at": "/redcraft/placed", "eq": 1, "msg": "the cobblestone was put back where the dirt was"}},
            {"snapshot": "built"},
            {"look": {"yaw": 90, "pitch": 0}},
            {"hold": ["forward"], "secs": 1.5},
            {"snapshot": "walked"},
        ]),
        &dir,
        "first",
    );
    assert!(o.status.success(), "{}", text(&o));
    let r = &state["redcraft"];
    let snap = |name: &str, key: &str| state["snapshots"][name]["redcraft"][key].clone();
    let dug = snap("dug", "pos");
    let walked = snap("walked", "pos");
    let moved = (walked[0].as_f64().unwrap() - dug[0].as_f64().unwrap()).abs() + (walked[2].as_f64().unwrap() - dug[2].as_f64().unwrap()).abs();
    assert!(moved > 1.0, "holding W walked the player ({dug} -> {walked})");
    assert!(r["count"]["cobblestone"].is_null(), "the cobblestone was used up: {}", r["count"]);
    // The run ended: the world was saved.
    let level = saves.join("morning").join("level.json");
    assert!(level.exists(), "{}: {}", level.display(), text(&o));
    let placed_at = state["snapshots"]["built"]["redcraft"]["target"]["pos"].clone();
    // The next launch continues where this one stopped.
    let (o2, state2) = run(
        &saves,
        "Morning",
        serde_json::json!([
            {"expect": {"at": "/redcraft/ready", "eq": true, "within": 30}},
            {"wait": 0.5}
        ]),
        &dir,
        "second",
    );
    assert!(o2.status.success(), "{}", text(&o2));
    let r2 = &state2["redcraft"];
    assert_eq!(r2["count"]["oak_planks"], 4, "the inventory was kept: {}", r2["count"]);
    let (a, b) = (&r["pos"], &r2["pos"]);
    let d = (0..3).map(|i| (a[i].as_f64().unwrap() - b[i].as_f64().unwrap()).abs()).sum::<f64>();
    assert!(d < 1.5, "the player is where they were: {a} vs {b}");
    assert!(!placed_at.is_null());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_zombie_at_night_is_fought_off_with_a_sword_and_the_dark_brings_more() {
    let dir = scratch("night");
    let saves = dir.join("worlds");
    let (o, state) = run(
        &saves,
        "Night",
        serde_json::json!([
            {"expect": {"at": "/redcraft/ready", "eq": true, "within": 30}},
            {"press": "key:time:0.85"},
            {"wait": 0.5},
            {"press": "key:give:diamond_sword:1"},
            {"press": "key:slot:1"},
            {"look": {"yaw": 0, "pitch": -8}},
            {"press": "key:spawn:zombie:3"},
            {"press": "key:mine"},
            {"wait": 5},
            {"press": "key:stop"},
            {"expect": {"at": "/redcraft/kills", "min": 1, "msg": "the zombie was killed"}},
            {"wait": 25},
            {"expect": {"at": "/redcraft/entities/zombies", "min": 1, "msg": "the night brings zombies"}}
        ]),
        &dir,
        "fight",
    );
    assert!(o.status.success(), "{}", text(&o));
    assert!(state["redcraft"]["kills"].as_u64().unwrap_or(0) >= 1);
    let _ = std::fs::remove_dir_all(&dir);
}
