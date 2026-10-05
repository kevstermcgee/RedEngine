//! The fresh-author exercise, kept: a small game (`examples/lantern_walk`) that combines a procedural world, a day/night clock, generated music and nature ambience,
//! rule-driven play with a carried prop, a declared UI and a hosted match, written the way a newcomer writes it (guessing a prop height, putting a target on a lantern,
//! misspelling an assertion), and every mistake that was made on the way is a test here that the engine explains it where the author reads it.
//!
//! These prove the *messages and the checks*. They do not prove the game is pleasant, the music is good, or that it runs smoothly on a player's GPU.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/lantern_walk")
}

/// A copy of the game (scene and score) in a scratch folder, with the scene changed by `edit`; returns the scene path.
fn variant(name: &str, edit: impl FnOnce(&mut Value)) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("re2_lantern_{}_{}_{name}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("audio")).unwrap();
    std::fs::copy(example().join("audio/dusk.json"), dir.join("audio/dusk.json")).unwrap();
    let mut scene: Value = serde_json::from_str(&std::fs::read_to_string(example().join("lantern_walk.json")).unwrap()).unwrap();
    edit(&mut scene);
    let path = dir.join("lantern_walk.json");
    std::fs::write(&path, serde_json::to_string_pretty(&scene).unwrap()).unwrap();
    path
}

/// Every `verify` row of a scene as `"PASS|FAIL name detail"`.
fn rows(path: &Path, only: Option<&str>) -> Vec<(bool, String)> {
    let opts = red_engine2::tools::verify::Options { only: only.map(str::to_string), ..Default::default() };
    let report = red_engine2::tools::verify::run(path, &opts).unwrap_or_else(|e| panic!("verify: {e}"));
    report.results.iter().map(|r| (r.ok, format!("{} {}", r.name, r.detail))).collect()
}

#[test]
fn the_game_passes_everything_it_claims() {
    let all = rows(&example().join("lantern_walk.json"), None);
    let failed: Vec<&String> = all.iter().filter(|(ok, _)| !ok).map(|(_, t)| t).collect();
    assert!(failed.is_empty(), "{failed:#?}");
    // It really checks the game: lint, four places to stand, three playthroughs, the music and the night's soundscape.
    for what in ["lint", "reach[3]", "sim `one lantern", "sim `all three lanterns", "sim `night falls", "audio score dusk", "audio night has owls"] {
        assert!(all.iter().any(|(_, t)| t.contains(what)), "no `{what}` row in {all:#?}");
    }
}

/// Mistake: a misspelled nested assertion (`peek_max`). It must not become a check that passes while asserting nothing.
#[test]
fn a_misspelled_audio_assertion_is_refused_with_its_fix() {
    let path = variant("typo", |s| s["checks"]["audio"]["scores"]["peek_max"] = json!(-6));
    let all = rows(&path, Some("audio"));
    assert!(all.iter().any(|(ok, t)| !ok && t.contains("checks.audio.scores.peek_max: unknown field") && t.contains("did you mean `peak_max`")), "{all:#?}");
}

/// Mistake: a sound that does not exist. A failed row that names it, never a pass.
#[test]
fn a_nonexistent_sound_is_a_failed_row_naming_it() {
    let path = variant("nosound", |s| s["checks"]["audio"] = json!({"sounds": [{"name": "the robin", "sound": "bird.robn"}]}));
    let all = rows(&path, Some("audio"));
    assert!(all.iter().any(|(ok, t)| !ok && t.contains("the robin") && t.contains("bird.robn")), "{all:#?}");
}

/// Mistake: a score file the scene names but that is not there. The scene does not even load, and says which field and what was looked for.
#[test]
fn a_missing_score_file_stops_the_scene_loading() {
    let path = variant("noscore", |s| s["audio"]["music"]["night"] = json!("audio/night_that_is_not_there.json"));
    let Err(errs) = red_engine2::load_scene(&path) else { panic!("a scene that names a missing score must not load") };
    let err = errs.join("; ");
    assert!(err.contains("audio.music.night") && err.contains("not found") && err.contains("night_that_is_not_there.json"), "{err}");
    // And a file that is there but is not a score says so.
    let path = variant("notascore", |s| s["audio"]["music"]["night"] = json!("lantern_walk.json"));
    let Err(errs) = red_engine2::load_scene(&path) else { panic!("a file that is not a score must not load as one") };
    let err = errs.join("; ");
    assert!(err.contains("audio.music.night") && err.contains("is not a score"), "{err}");
}

/// Mistake: an objective on top of a generated bush (a target on the lantern's own spot is the same mistake). The row says what covers it and where to stand instead.
#[test]
fn an_objective_inside_a_generated_bush_says_what_covers_it_and_where_to_stand() {
    // The generator's own bush near the spawn: a lantern placed there is a plausible blind guess.
    let probe = red_engine2::tools::world::MapWorld::load(&example().join("lantern_walk.json")).unwrap();
    let gen = probe.ground.procgen().expect("an endless scene").world();
    let bush = (-2..=2)
        .flat_map(|z| (-2..=2).map(move |x| red_engine2::procgen::ChunkId { x, z }))
        .flat_map(|c| gen.blockers(c))
        .find(|t| (t.x * t.x + t.z * t.z).sqrt() > 6.0 && (t.x * t.x + t.z * t.z).sqrt() < 30.0 && t.radius > 0.4)
        .expect("a shrub or tree between 6 and 30 m of the spawn");
    let path = variant("bush", |s| s["checks"]["reach"] = json!([{"to": [bush.x, bush.z], "why": "the lantern I put in a bush"}]));
    let all = rows(&path, Some("reach"));
    assert!(
        all.iter().any(|(ok, t)| !ok
            && t.contains("the lantern I put in a bush")
            && t.contains("a generated tree or shrub covers it")
            && t.contains("the nearest place the player can stand")),
        "{all:#?}"
    );
}

/// Mistake: a zone authored at the height of a flat map over rolling ground.
#[test]
fn a_zone_at_the_wrong_height_is_an_error_that_names_the_ground() {
    let path = variant("zoney", |s| s["zones"][0]["y"] = json!(1.2));
    let all = rows(&path, Some("lint"));
    assert!(all.iter().any(|(ok, t)| !ok && t.contains("zone 'shrine_zone' sits at y=1.2 but the ground") && t.contains("set its \"y\"")), "{all:#?}");
}

/// Mistake: a reach target written with its height in the middle, `[x, y, z]`: the height is the LAST number, so the check asks about the wrong place. The schema says so.
#[test]
fn the_reach_target_documents_which_number_is_the_height() {
    let path = variant("order", |s| s["checks"]["reach"] = json!([{"to": "9,-11"}]));
    let all = rows(&path, Some("reach"));
    assert!(all.iter().any(|(ok, t)| !ok && t.contains("checks.reach[0].to") && t.contains("[x, z, y]: height last")), "{all:#?}");
}

// ---- the client: the hosted match and the split-screen limit --------------------------------------------------------------------------------------------

#[cfg(feature = "gfx")]
mod client {
    use super::*;
    use std::process::{Command, Output};

    fn re2(scene: &Path, args: &[&str], script: Value) -> (Output, PathBuf) {
        let dir = scene.parent().unwrap();
        let (script_path, dump) = (dir.join("play.json"), dir.join("state.json"));
        std::fs::write(&script_path, json!({"steps": script}).to_string()).unwrap();
        let o = Command::new(env!("CARGO_BIN_EXE_re2"))
            .arg(scene)
            .args(["--as", "human", "--headless", "--script"])
            .arg(&script_path)
            .arg("--dump")
            .arg(&dump)
            .args(args)
            .output()
            .expect("start re2");
        (o, dump)
    }

    fn text(o: &Output) -> String {
        format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    }

    /// A hosted match of the game: the player joins, and what they can read is the declared HUD (objective and counter), not the engine's variable list.
    #[test]
    fn a_hosted_match_shows_the_declared_objective_and_counter() {
        let scene = variant("hosted", |_| {});
        let (o, dump) = re2(
            &scene,
            &["--host", "--name", "Tester"],
            json!([{"wait_for": {"at": "/online/connected", "eq": true, "within": 20}}, {"wait": 1.5}, {"snapshot": "later"}]),
        );
        assert!(o.status.success(), "{}", text(&o));
        let d: Value = serde_json::from_str(&std::fs::read_to_string(dump).unwrap()).unwrap();
        let lines: Vec<String> = d["hud"]["lines"].as_array().unwrap().iter().filter_map(|l| l["text"].as_str().map(str::to_string)).collect();
        assert!(
            // A long objective wraps over several lines of the HUD; what the player reads is the whole sentence.
            lines.join(" ").contains("CARRY THE THREE LANTERNS TO THE SHRINE BEFORE NIGHT") && lines.iter().any(|l| l == "LANTERNS LIT: 0 / 3"),
            "{lines:?}"
        );
    }

    /// Mistake: planning the game for two people with the lanterns to carry. Split-screen guests cannot carry props: the client says so at start-up, and a script that
    /// makes a guest carry one fails with the reason and the way out.
    #[test]
    fn a_two_player_game_built_on_carrying_is_diagnosed_not_discovered() {
        let scene = variant("coop", |_| {});
        let (o, _) = re2(&scene, &["--players", "2"], json!([{"player": 2}, {"interact": "lantern_1"}]));
        let t = text(&o);
        assert!(t.contains("note: split-screen: this scene has 3 loose prop(s)"), "{t}");
        assert_eq!(o.status.code(), Some(1), "{t}");
        assert!(t.contains("split-screen guest") && t.contains("only player 1 can pick up"), "{t}");
    }
}
