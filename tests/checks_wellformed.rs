//! Every scene the repository ships has a well-formed `checks` block, and the flagship soundscape checks hold for the scene as configured.
//!
//! `verify` reports a misspelled field, a value of the wrong type or a group that asserts nothing as a failed check (`tools::check_schema`, `tools::audio_checks`), so a
//! shipped scene with one would fail its own `verify`. This walks the example, recipe, fixture and asset scenes and holds them to the same rule without running them,
//! so a new scene cannot add a check that quietly proves nothing.

use red_engine2::tools::{audio_checks, check_schema};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            json_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
}

#[test]
fn every_shipped_scene_has_well_formed_checks() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["examples", "recipes", "tests/fixtures", "assets"] {
        json_files(&root.join(dir), &mut files);
    }
    let (mut scenes, mut bad) = (0, Vec::new());
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let Some(checks) = v.get("checks").filter(|c| c.is_object()) else { continue };
        scenes += 1;
        let mut problems = check_schema::invalid_fields(checks);
        if let Some(audio) = checks.get("audio") {
            problems.extend(audio_checks::invalid_fields(audio));
        }
        for p in problems {
            bad.push(format!("{}: {p}", f.strip_prefix(&root).unwrap_or(&f).display()));
        }
    }
    assert!(scenes > 10, "the walk found only {scenes} scenes with checks");
    assert!(bad.is_empty(), "scenes whose `checks` would fail `verify` before running anything:\n  {}", bad.join("\n  "));
}

/// Marcel switches songbirds off (`audio.birds: false`); its own soundscape checks must hold for the scene as the player hears it. They once counted 190 calls nobody
/// would hear and failed `verify` on the engine's flagship example without anything noticing (no test ran them).
#[test]
fn marcels_soundscape_checks_hold_for_the_scene_as_configured() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/marcel/marcel.json");
    let scene: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut block = scene["checks"]["audio"].clone();
    block.as_object_mut().unwrap().remove("scores"); // rendering four scores takes seconds; the scores have their own tests
    let rows = audio_checks::verify_checks(&path, &block).expect("the block is well formed");
    assert!(rows.len() >= 3, "{rows:?}");
    let failed: Vec<String> = rows.iter().filter(|r| !r.1).map(|r| format!("{}: {}", r.0, r.2)).collect();
    assert!(failed.is_empty(), "{failed:?}");
}
