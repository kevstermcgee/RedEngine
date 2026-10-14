//! Frameless played by its own binary with no window (`frameless --headless --script`): the first room's loop, then the saved visit continued by the
//! next launch. Written before the museum moved behind the client's game-mode seam and out of the engine to this crate (ADR
//! 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library), when `re2` played it: the visit those moves had to keep, the same museum for
//! the same seed and a save that loads, unchanged since.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn frameless() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("content/frameless.json")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("re2_frameless_visit_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `frameless` on its museum with the launch seed 7, saving into `save` (a fresh visit when `fresh`).
fn visit(save: &Path, fresh: bool, script: &Path, dump: &Path) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_frameless"));
    c.args([frameless().to_str().unwrap(), "--headless", "--script", script.to_str().unwrap(), "--dump", dump.to_str().unwrap()])
        .env("RE2_SEED", "7")
        .env("RE2_SAVE_DIR", save)
        .env_remove("RE2_MUSEUM_FRESH");
    if fresh {
        c.env("RE2_MUSEUM_FRESH", "1");
    }
    c.output().expect("start frameless")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn museum(dump: &Path) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dump).unwrap()).unwrap();
    v["museum"].clone()
}

#[test]
fn the_first_rooms_key_is_found_inside_its_painting_and_the_next_launch_continues_the_visit() {
    let dir = scratch("loop");
    let save = dir.join("save");
    // Seed 7: Gallery 1's key is in piece 5, a painting, one world deep (`frameless museum content/frameless.json --seed 7`).
    let script = dir.join("play.json");
    std::fs::write(
        &script,
        serde_json::json!({"policy": "idle", "steps": [
            {"press": "start"},
            {"wait": 1},
            {"expect": {"at": "/museum/gallery", "eq": 1}},
            {"expect": {"at": "/museum/keys", "eq": 0}},
            {"expect": {"at": "/museum/place", "eq": "halls"}},
            {"approach": "museum:5"},
            {"look_at": "museum:5"},
            {"interact": 1},
            {"wait_for": {"at": "/museum/place", "contains": "inside"}},
            {"snapshot": "inside"},
            // The key floats in the painting's world: walk into it (it is taken within a metre and a quarter, and is gone once taken).
            {"look_at": "museum:key"},
            {"approach": "museum:key", "within": 1.6},
            {"look_at": "museum:key"},
            {"hold": ["forward"], "secs": 0.8},
            {"wait_for": {"at": "/museum/keys", "eq": 1}},
            {"approach": "museum:way-back"},
            {"look_at": "museum:way-back"},
            {"interact": 1},
            {"wait_for": {"at": "/museum/place", "eq": "halls"}},
            {"press": "key:M"},
            {"wait": 0.5},
            {"press": "key:M"},
            {"wait": 0.5}
        ]})
        .to_string(),
    )
    .unwrap();
    let dump = dir.join("state.json");
    let o = visit(&save, true, &script, &dump);
    let out = text(&o);
    assert!(o.status.success(), "the visit failed:\n{out}");
    for line in ["museum seed 7: 0 keys found", "museum: SteppingIn { depth: 1 }", "museum: KeyFound", "museum: Returned { depth: 1 }", "museum: DoorOpening"] {
        assert!(out.contains(line), "`{line}` is missing from the visit:\n{out}");
    }
    let m = museum(&dump);
    assert_eq!((m["seed"].as_u64(), m["gallery"].as_u64(), m["keys"].as_u64(), m["place"].as_str()), (Some(7), Some(1), Some(1), Some("halls")), "{m}");
    assert_eq!(m["deepest"].as_u64(), Some(1), "{m}");
    let snap: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&dump).unwrap()).unwrap();
    assert!(
        snap["snapshots"]["inside"]["museum"]["place"].as_str().is_some_and(|p| p.starts_with("inside 1/5")),
        "inside piece 5 of Gallery 1: {}",
        snap["snapshots"]["inside"]["museum"]
    );
    assert!(save.join("museum.json").is_file(), "the visit is saved under RE2_SAVE_DIR");

    // The next launch, without RE2_MUSEUM_FRESH, continues the saved visit: the same seed, the key still found.
    let resume = dir.join("resume.json");
    std::fs::write(&resume, serde_json::json!({"policy": "idle", "steps": [{"press": "start"}, {"wait": 0.5}]}).to_string()).unwrap();
    let dump2 = dir.join("state2.json");
    let o = visit(&save, false, &resume, &dump2);
    let out = text(&o);
    assert!(o.status.success() && out.contains("museum seed 7: 1 keys found"), "the saved visit did not continue:\n{out}");
    let m = museum(&dump2);
    assert_eq!((m["seed"].as_u64(), m["keys"].as_u64()), (Some(7), Some(1)), "{m}");
    let _ = std::fs::remove_dir_all(&dir);
}
