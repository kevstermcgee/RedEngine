//! Every 3D game an Idea Forge run adds (`examples/3d/<slug>/<slug>.json`, docs/IDEA_FORGE.md) must pass its own `verify` (lint, reach, scripted playthroughs, audio): a game that was merged
//! because its feedback was useful must not turn CI red later. Nothing here proves a game is fun; it proves what the game claims about itself still holds.
//! With no 3D examples yet the test has nothing to check and passes.

use std::path::{Path, PathBuf};

fn games() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/3d");
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            e.path().join(format!("{name}.json"))
        })
        .collect();
    out.sort();
    out
}

#[test]
fn every_3d_example_game_has_its_scene_named_like_its_folder() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/3d");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for e in entries.flatten().filter(|e| e.path().is_dir()) {
        let name = e.file_name().to_string_lossy().to_string();
        assert!(e.path().join(format!("{name}.json")).is_file(), "examples/3d/{name}/ has no {name}.json: a 3D game is examples/3d/<slug>/<slug>.json");
    }
}

#[test]
fn every_3d_example_game_passes_its_own_checks() {
    for game in games() {
        let report = red_engine2::tools::verify::run(&game, &red_engine2::tools::verify::Options::default())
            .unwrap_or_else(|e| panic!("verify {}: {e}", game.display()));
        let failed: Vec<String> = report.results.iter().filter(|r| !r.ok).map(|r| format!("{} {}", r.name, r.detail)).collect();
        assert!(failed.is_empty(), "{}: {failed:#?}", game.display());
        assert!(
            report.results.iter().any(|r| r.name.starts_with("sim")),
            "{} has no scripted playthrough (`checks.sim`): a game that is never played proves nothing",
            game.display()
        );
    }
}
