//! `red_engine2 web status GAME`: where a browser game stands, and the one command to run next.
//!
//! A model that does not know the engine should never have to work out the workflow: this reads what is on disk (the game file, the package `web build` wrote, the record `web verify`
//! wrote, the report `publish` wrote), compares them with the game *as it is now*, and says what is current, what is stale, and the exact next command with the reason. It never builds,
//! never opens a browser and never changes anything; the only thing it runs is the native `verify` (about a second) so "the scenarios pass" is a fact and not a memory.

use super::evidence::{Evidence, Status};
use super::game2d;
use serde_json::{json, Value};
use std::path::Path;

/// Schema of the status object.
pub const SCHEMA: &str = "red2d-web-status/1";

/// What a browser game cannot do today. One list: `describe web`, `web status` and the docs print it.
pub const LIMITS: &[&str] = &[
    "Browser games are 2D or hybrid. Hybrid draws 3D with a built-in software renderer (flat or toon shading, one light, a few thousand triangles); it is NOT the wgpu engine and is not a port of it.",
    "The native 3D engine (first person, lighting, multiplayer, big worlds) runs on windows and linux. Running it in a browser is experimental: `describe web3d`.",
    "Browser games are offline and single player: no UDP or QUIC from a page, and no browser transport for the authoritative server yet.",
    "Audio starts only after the player's first key press or tap (every browser's rule); nobody has listened to it, only that it started.",
    "Saves are the browser's localStorage for that origin: per device and browser, cleared if the player clears site data; there is a backup and restore link on the start card.",
    "Automated checks run one engine (Chromium, headless, emulated touch). Safari, Firefox and a physical phone are not covered: `docs/DEVICE_QUALIFICATION.md` lists what a person must do.",
    "A passing run never means a human played, heard or enjoyed the game: human_playtested is always false until a person records it.",
];

fn read_json(p: &Path) -> Option<Value> {
    std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// The status of the game at `game`, with the `out/` directories the commands write to taken relative to `base` (the current directory when run for real).
pub fn status(game: &Path, base: &Path) -> Value {
    let g = game.display().to_string();
    let mut next: Vec<Value> = Vec::new();
    let mut step = |command: String, why: String| next.push(json!({"command": command, "why": why}));

    let validation = game2d::validate(game);
    let loaded = game2d::load(game);
    let Ok((def, _)) = loaded else {
        step(format!("red_engine2 validate {g}"), "the file does not validate; every problem is listed with its fix".into());
        return json!({
            "schema": SCHEMA, "game": g, "valid": false, "errors": validation.text.lines().take(12).collect::<Vec<_>>(),
            "next": next, "limits": LIMITS,
        });
    };
    let id = def.id.clone();
    let native = game2d::verify(game, None);
    let counts = native.text.lines().find(|l| l.contains(" passed, ")).unwrap_or("").to_string();
    let native_failures: Vec<String> = native.text.lines().filter(|l| l.starts_with("FAIL")).take(6).map(str::to_string).collect();

    // The package `web build` / `web verify` wrote, and whether it is of the game as it is now.
    let pkg_dir = base.join("out/web").join(&id);
    let manifest = read_json(&pkg_dir.join("manifest.json"));
    let package_id = manifest.as_ref().and_then(|m| m["package_id"].as_str()).map(str::to_string);
    let package_current = manifest.as_ref().map(|m| m["game"]["game_revision"].as_str() == Some(def.rev.as_str()));

    // The browser record: for which package, did it pass, and which pieces of evidence stand.
    let rec_path = base.join("out/web-verify").join(&id).join("verification.json");
    let rec = read_json(&rec_path);
    let rec_package = rec.as_ref().and_then(|r| r["package_id"].as_str()).map(str::to_string);
    let rec_ok = rec.as_ref().and_then(|r| r["ok"].as_bool());
    let evidence = rec.as_ref().map(|r| Evidence::from_json(&r["evidence"]));
    let rec_matches = matches!((&rec_package, &package_id), (Some(a), Some(b)) if a == b);
    let failed_pieces: Vec<String> = evidence
        .as_ref()
        .map(|e| e.0.iter().filter(|(_, c)| c.status == Status::Failed).map(|(k, c)| format!("{k}: {}", c.detail)).collect())
        .unwrap_or_default();

    // The last `publish` run.
    let pub_path = base.join("out/publish").join(&id).join("publication.json");
    let publication = read_json(&pub_path);
    let pub_build = publication.as_ref().and_then(|p| p["build_id"].as_str()).map(str::to_string);
    let published_this_build = matches!((&pub_build, &package_id), (Some(a), Some(b)) if a == b);

    // The next command: the first thing that is not true yet.
    if !native.ok {
        step(format!("red_engine2 verify {g}"), format!("the native scenarios fail: {}", native_failures.join(" | ")));
    } else if package_current != Some(true) {
        step(
            format!("red_engine2 web verify {g}"),
            if manifest.is_none() {
                "no package has been built: this builds it and plays it in a real headless browser".to_string()
            } else {
                "the package is of an older version of the game file: rebuild and re-verify".to_string()
            },
        );
    } else if !rec_matches || rec_ok != Some(true) {
        step(
            format!("red_engine2 web verify {g}"),
            match (&rec, rec_matches, failed_pieces.is_empty()) {
                (None, _, _) => "the package has not been played in a browser (needs `red_engine2 web setup-browser` once)".to_string(),
                (Some(_), false, _) => "the browser record is for another package: play this one".to_string(),
                (Some(_), true, false) => format!("the browser run failed: {}", failed_pieces.join(" | ")),
                (Some(_), true, true) => "the browser run failed: read its FAIL rows".to_string(),
            },
        );
    } else if !published_this_build {
        step(format!("red_engine2 publish {g}"), "the browser run passed; publish writes the site (out/site) and publication.json".to_string());
    } else if publication.as_ref().is_some_and(|p| p["levels"]["remotely_playable"] != true) {
        step(
            format!("red_engine2 publish {g} --backend github-pages --repo ../RedEngineGames --push"),
            "it is published to a local site only; this commits only this game into the RedEngineGames checkout, pushes, and plays the deployed copy"
                .to_string(),
        );
    }

    let (p, f, n, na) = evidence.as_ref().map_or((0, 0, 0, 0), Evidence::count);
    json!({
        "schema": SCHEMA,
        "game": g,
        "id": id,
        "title": def.title,
        "valid": true,
        "presentation": def.caps.presentation.name(),
        "game_revision": def.rev,
        "native": {"ok": native.ok, "summary": counts, "failures": native_failures},
        "package": {"dir": pkg_dir.display().to_string(), "built": manifest.is_some(), "package_id": package_id, "current": package_current},
        "browser": {
            "record": rec_path.display().to_string(), "exists": rec.is_some(), "for_this_package": rec_matches, "ok": rec_ok,
            "browser": rec.as_ref().and_then(|r| r["browser"].as_str()),
            "evidence": {"passed": p, "failed": f, "not_run": n, "not_applicable": na, "failed_pieces": failed_pieces},
        },
        "publication": {
            "report": pub_path.display().to_string(), "exists": publication.is_some(), "is_this_build": published_this_build,
            "levels": publication.as_ref().map(|p| p["levels"].clone()),
            "url": publication.as_ref().and_then(|p| p["url"].as_str()),
        },
        "human_playtested": false,
        "next": next,
        "after_that_a_person": "play it on a real phone and a real desktop browser and record it: docs/DEVICE_QUALIFICATION.md",
        "limits": LIMITS,
    })
}

/// The text form: a few lines, the next command last.
pub fn render(v: &Value) -> String {
    let mut t = String::new();
    if v["valid"] != true {
        t.push_str(&format!("{}: NOT VALID\n", v["game"].as_str().unwrap_or("")));
        for e in v["errors"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            t.push_str(&format!("  {e}\n"));
        }
    } else {
        let yn = |b: &Value| match b.as_bool() {
            Some(true) => "yes",
            Some(false) => "NO",
            None => "-",
        };
        t.push_str(&format!(
            "{} `{}` ({}), game revision {}\n",
            v["title"].as_str().unwrap_or(""),
            v["id"].as_str().unwrap_or(""),
            v["presentation"].as_str().unwrap_or(""),
            v["game_revision"].as_str().unwrap_or("")
        ));
        t.push_str(&format!("  native scenarios   {}   {}\n", yn(&v["native"]["ok"]), v["native"]["summary"].as_str().unwrap_or("")));
        t.push_str(&format!("  package built      {}   current with the game file: {}\n", yn(&v["package"]["built"]), yn(&v["package"]["current"])));
        let ev = &v["browser"]["evidence"];
        t.push_str(&format!(
            "  browser record     {}   for this package: {}   passed: {}   evidence: {} passed, {} failed, {} not run, {} not applicable\n",
            yn(&v["browser"]["exists"]),
            yn(&v["browser"]["for_this_package"]),
            yn(&v["browser"]["ok"]),
            ev["passed"],
            ev["failed"],
            ev["not_run"],
            ev["not_applicable"]
        ));
        let lv = &v["publication"]["levels"];
        t.push_str(&format!(
            "  published          {}   built {} / locally verified {} / uploaded {} / remotely playable {} / human playtested {}\n",
            yn(&v["publication"]["is_this_build"]),
            yn(&lv["built"]),
            yn(&lv["locally_verified"]),
            yn(&lv["uploaded"]),
            yn(&lv["remotely_playable"]),
            yn(&lv["human_playtested"])
        ));
    }
    match v["next"].as_array().and_then(|a| a.first()) {
        Some(n) => t.push_str(&format!("NEXT: {}\n      ({})\n", n["command"].as_str().unwrap_or(""), n["why"].as_str().unwrap_or(""))),
        None => t.push_str("NEXT: nothing is stale. The remaining step is a person's: record a real-device play (docs/DEVICE_QUALIFICATION.md).\n"),
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("re2_webstatus_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn game_in(dir: &Path) -> std::path::PathBuf {
        let p = dir.join("coin-dash.game2d.json");
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/2d/coin-dash.game2d.json"), &p).unwrap();
        p
    }

    fn next_of(v: &Value) -> String {
        v["next"][0]["command"].as_str().unwrap_or("").to_string()
    }

    #[test]
    fn an_invalid_game_says_validate_and_a_new_game_says_web_verify() {
        let dir = scratch("steps");
        let bad = dir.join("broken.game2d.json");
        std::fs::write(&bad, r#"{"game2d":1,"id":"broken"}"#).unwrap();
        let v = status(&bad, &dir);
        assert_eq!(v["valid"], false);
        assert!(next_of(&v).starts_with("red_engine2 validate "), "{v}");
        let game = game_in(&dir);
        let v = status(&game, &dir);
        assert_eq!(v["valid"], true);
        assert_eq!(v["native"]["ok"], true);
        assert_eq!(v["package"]["built"], false);
        assert!(next_of(&v).starts_with("red_engine2 web verify "), "{v}");
        assert!(v["limits"].as_array().unwrap().len() >= 5 && v["human_playtested"] == false);
    }

    #[test]
    fn a_stale_package_and_a_record_for_another_package_send_you_back_to_web_verify_and_a_current_one_to_publish() {
        let dir = scratch("stale");
        let game = game_in(&dir);
        let rev = status(&game, &dir)["game_revision"].as_str().unwrap().to_string();
        let write = |rel: &str, v: Value| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, v.to_string()).unwrap();
        };
        // A package of an older version of the game.
        write("out/web/coin-dash/manifest.json", json!({"package_id": "aaaa", "game": {"game_revision": "old"}}));
        let v = status(&game, &dir);
        assert_eq!(v["package"]["current"], false);
        assert!(next_of(&v).contains("web verify") && v["next"][0]["why"].as_str().unwrap().contains("older version"), "{v}");
        // A current package nobody played, then one played but for another package, then one that failed.
        write("out/web/coin-dash/manifest.json", json!({"package_id": "aaaa", "game": {"game_revision": rev}}));
        assert!(status(&game, &dir)["next"][0]["why"].as_str().unwrap().contains("not been played"));
        let mut ev = Evidence::default();
        ev.set("offline_reload", Status::Failed, "reload with no network failed");
        write("out/web-verify/coin-dash/verification.json", json!({"package_id": "bbbb", "ok": true, "evidence": ev.to_json()}));
        assert!(status(&game, &dir)["next"][0]["why"].as_str().unwrap().contains("another package"));
        write("out/web-verify/coin-dash/verification.json", json!({"package_id": "aaaa", "ok": false, "evidence": ev.to_json()}));
        let v = status(&game, &dir);
        assert!(v["next"][0]["why"].as_str().unwrap().contains("offline_reload"), "{v}");
        assert_eq!(v["browser"]["evidence"]["failed"], 1);
        // A passing record for this package: publish, then (published locally) the GitHub Pages step, then nothing stale.
        write("out/web-verify/coin-dash/verification.json", json!({"package_id": "aaaa", "ok": true, "evidence": Evidence::default().to_json()}));
        assert_eq!(next_of(&status(&game, &dir)), format!("red_engine2 publish {}", game.display()));
        write(
            "out/publish/coin-dash/publication.json",
            json!({"build_id": "aaaa", "levels": {"built": true, "locally_verified": true, "uploaded": true, "remotely_playable": false, "human_playtested": false}}),
        );
        assert!(next_of(&status(&game, &dir)).contains("--backend github-pages"));
        write(
            "out/publish/coin-dash/publication.json",
            json!({"build_id": "aaaa", "levels": {"built": true, "locally_verified": true, "uploaded": true, "remotely_playable": true, "human_playtested": false}}),
        );
        let v = status(&game, &dir);
        assert!(v["next"].as_array().unwrap().is_empty(), "{v}");
        let text = render(&v);
        assert!(text.contains("remotely playable yes") && text.contains("human playtested NO") && text.contains("a person's"), "{text}");
    }
}
