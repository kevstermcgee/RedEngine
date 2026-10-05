//! A published playable ships with every file its scene refers to.
//!
//! `games-publish.json` lists, for each playable game, the files that go into its download (`files`). A scene that names a score (`audio.music`, `audio.layers`) and a
//! `files` list that leaves the score out would ship a game that silently plays no music: nothing fails, the player just never hears it. This reads the scene exactly
//! as the engine does ([`crate::schema::Scene::referenced_files`]) and says, for each file, whether the manifest publishes it and whether the playable's `files` include it.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// One file the manifest publishes: the source it comes from and the path it lands at in the games repository (`games/marcel/audio/day.json`).
struct Published {
    source: PathBuf,
    path: String,
}

fn files_under(dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>, rel: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let r = rel.join(e.file_name());
        if p.is_dir() {
            files_under(&p, out, &r);
        } else {
            out.push((p, r));
        }
    }
}

/// Every file the manifest's collections publish and where each one lands.
fn published_files(root: &Path, manifest: &Value) -> Result<Vec<Published>, String> {
    let collections = manifest["collections"].as_object().ok_or("games-publish.json has no `collections`")?;
    let mut out = Vec::new();
    for (category, entries) in collections {
        for (i, entry) in entries.as_array().into_iter().flatten().enumerate() {
            let (Some(source), Some(destination)) = (entry["source"].as_str(), entry["destination"].as_str()) else {
                return Err(format!("collections.{category}[{i}] needs `source` and `destination`"));
            };
            let src = root.join(source);
            let mut files = Vec::new();
            if src.is_file() {
                files.push((src.clone(), PathBuf::from(src.file_name().unwrap_or_default())));
            } else {
                files_under(&src, &mut files, Path::new(""));
            }
            for (file, rel) in files {
                let path = format!("{category}/{destination}/{}", rel.to_string_lossy().replace('\\', "/"));
                out.push(Published { source: std::fs::canonicalize(&file).unwrap_or(file), path });
            }
        }
    }
    Ok(out)
}

/// For each playable in `manifest` (the parsed `games-publish.json`, with sources under `root`): the files its scene refers to that the download would not contain, as
/// sentences naming the playable, the file and the fix. Empty when every playable is complete.
pub fn playable_resource_gaps(root: &Path, manifest: &Value) -> Result<Vec<String>, String> {
    let published = published_files(root, manifest)?;
    let mut gaps = Vec::new();
    for playable in manifest["playables"].as_array().into_iter().flatten() {
        let slug = playable["slug"].as_str().unwrap_or("?");
        let entry = playable["entry"].as_str().unwrap_or("").replace('\\', "/");
        let included = |path: &str| {
            playable["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|f| f.replace('\\', "/"))
                .any(|f| path == f || path.starts_with(&format!("{f}/")))
        };
        let Some(scene_file) = published.iter().find(|p| p.path == entry) else {
            gaps.push(format!("playable `{slug}`: its entry `{entry}` is not published by any collection"));
            continue;
        };
        if !entry.ends_with(".json") {
            continue;
        }
        let scene = match crate::load_scene(&scene_file.source) {
            Ok(s) => s,
            Err(e) => {
                gaps.push(format!("playable `{slug}`: its scene `{entry}` does not load: {}", e.join("; ")));
                continue;
            }
        };
        for needed in scene.referenced_files() {
            let canon = std::fs::canonicalize(&needed).unwrap_or(needed.clone());
            let shown = needed.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match published.iter().find(|p| p.source == canon) {
                None => gaps.push(format!(
                    "playable `{slug}`: its scene needs `{shown}` ({}), which no collection in games-publish.json publishes: add the folder that holds it",
                    needed.display()
                )),
                Some(p) if !included(&p.path) => gaps.push(format!(
                    "playable `{slug}`: its scene needs `{}`, which is published but not in the playable's `files` ({}): add `{}` (or the folder above it) to `files`",
                    p.path,
                    playable["files"],
                    p.path.rsplit_once('/').map_or(p.path.as_str(), |(dir, _)| dir)
                )),
                Some(_) => {}
            }
        }
    }
    Ok(gaps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A scratch tree: `game/scene.json` naming `audio/day.json`, a score beside it, and a manifest over it.
    fn tree(name: &str, files: Value) -> (PathBuf, Value) {
        let root = std::env::temp_dir().join(format!("re2_publish_check_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("game/audio")).unwrap();
        std::fs::write(
            root.join("game/audio/day.json"),
            r#"{"bpm": 120, "beats": 4, "bars": 2, "key": "D", "scale": "major", "seed": 3, "lufs": -27,
                "instruments": {"pad": {"seconds": 4, "level": 0.5, "layers": [{"sine": 1, "attack": 0.5, "release": 1}]}},
                "tracks": [{"inst": "pad", "play": "chords", "chords": "I V", "every": "1 bar", "octave": 3}]}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("game/scene.json"),
            r#"{"camera": {"position": [0, 1, 3], "target": [0, 1, 0]}, "audio": {"ambience": "nature", "music": {"day": "audio/day.json"}}, "objects": []}"#,
        )
        .unwrap();
        let manifest = json!({
            "collections": {"games": [{"source": "game", "destination": "walk"}]},
            "playables": [{"slug": "walk", "name": "Walk", "entry": "games/walk/scene.json", "files": files, "arguments": []}]
        });
        (root, manifest)
    }

    #[test]
    fn a_playable_that_includes_its_scores_is_complete() {
        let (root, manifest) = tree("ok", json!(["games/walk"]));
        assert_eq!(playable_resource_gaps(&root, &manifest).unwrap(), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(root);
    }

    /// The mistake: `files` lists the scene and forgets the audio folder. Nothing fails at publish time; the game just has no music.
    #[test]
    fn a_files_list_that_omits_a_score_is_a_named_gap() {
        let (root, manifest) = tree("omit", json!(["games/walk/scene.json"]));
        let gaps = playable_resource_gaps(&root, &manifest).unwrap();
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert!(
            gaps[0].contains("`walk`")
                && gaps[0].contains("games/walk/audio/day.json")
                && gaps[0].contains("not in the playable's `files`")
                && gaps[0].contains("games/walk/audio"),
            "{gaps:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_score_no_collection_publishes_is_a_gap_too() {
        let (root, mut manifest) = tree("unpublished", json!(["games/walk"]));
        // Publish only the scene file, not its folder: the score has no place in the games repository at all.
        manifest["collections"]["games"] = json!([{"source": "game/scene.json", "destination": "walk"}]);
        let gaps = playable_resource_gaps(&root, &manifest).unwrap();
        assert!(gaps.len() == 1 && gaps[0].contains("no collection in games-publish.json publishes"), "{gaps:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_scene_naming_a_missing_score_does_not_even_load() {
        let (root, manifest) = tree("missing", json!(["games/walk"]));
        std::fs::remove_file(root.join("game/audio/day.json")).unwrap();
        let gaps = playable_resource_gaps(&root, &manifest).unwrap();
        assert!(gaps.len() == 1 && gaps[0].contains("does not load") && gaps[0].contains("audio.music.day") && gaps[0].contains("not found"), "{gaps:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// And the real manifest: every playable the repository publishes is complete.
    #[test]
    fn every_published_playable_ships_everything_its_scene_needs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let manifest: Value = serde_json::from_str(&std::fs::read_to_string(root.join("games-publish.json")).unwrap()).unwrap();
        assert_eq!(playable_resource_gaps(root, &manifest).unwrap(), Vec::<String>::new());
    }
}
