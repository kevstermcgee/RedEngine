//! `red_engine2 game publish ../RedEngineGames`: put a game project where its friends can install it.
//!
//! Every game has to ship to the RedEngineGames repository (its Windows workflow builds one ZIP per playable). Doing that by hand was: copy the project without
//! its scratch folders, point `game.json` at the engine three directories up, and add a playable to `.release-games.json` in the repository's own style, with
//! `--host` when the game's opponents are bots. This does exactly that, refuses to copy anything that looks like a secret, and never commits or pushes.

use super::game::GameConfig;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Folders and files that are never published: scratch output, the fetched engine, version control, the hosting script (its identity and key live outside
/// the project, but the script is for the host's box, not for players).
const SKIP: &[&str] = &[".git", ".github", ".red", "out", "target", "deploy", ".gitignore", ".gitattributes", ".DS_Store"];

/// Names that look like secrets: publishing refuses when the project holds one.
fn looks_secret(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".pem")
        || n.ends_with(".key")
        || n.ends_with(".env")
        || n == "server.env"
        || n.contains("secret")
        || (n.contains("fingerprint") && n.ends_with(".txt"))
}

/// What publishing did.
#[derive(Debug, Clone, PartialEq)]
pub struct Published {
    /// Files copied.
    pub files: usize,
    /// The playable's slug.
    pub slug: String,
    /// Whether a new playable entry was added to `.release-games.json` (false when one already existed).
    pub added_entry: bool,
    /// The arguments the Windows launcher passes.
    pub arguments: Vec<String>,
}

/// The slug and display name of a project called `name` (`great-outdoors` -> `Great Outdoors`); the slug must already be the name.
pub fn slug_and_title(name: &str) -> Result<(String, String), String> {
    let ok = !name.is_empty() && name.split('-').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    if !ok {
        return Err(format!("the game name '{name}' cannot be a download name: use lowercase letters, digits and single hyphens (`game.json` \"name\")"));
    }
    let title = name
        .split('-')
        .map(|p| {
            let mut c = p.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    Ok((name.to_string(), title))
}

fn copy_tree(from: &Path, to: &Path, top: bool, count: &mut usize) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let mut entries: Vec<_> = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        if SKIP.contains(&name.as_str()) {
            continue;
        }
        let path = e.path();
        if looks_secret(&name) {
            return Err(format!("{} looks like a secret (a key, an identity or an env file): move it out of the project before publishing", path.display()));
        }
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            copy_tree(&path, &to.join(&name), false, count)?;
        } else if !(top && name == "STATUS.md") {
            std::fs::copy(&path, to.join(&name)).map_err(|e| format!("{}: {e}", path.display()))?;
            *count += 1;
        }
    }
    Ok(())
}

/// `game.json` for the published copy: a path to the engine becomes the path the games repository uses (`projects/<name>` is three levels below the sibling
/// RedEngine checkout); a pinned git engine is kept as it is.
pub fn published_game_json(text: &str) -> Result<String, String> {
    let mut v: Value = serde_json::from_str(text).map_err(|e| format!("game.json: {e}"))?;
    if let Some(p) = v.get_mut("engine").and_then(|e| e.get_mut("path")) {
        *p = Value::String("../../../RedEngine".into());
    }
    Ok(serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n")
}

/// Adds a playable to `.release-games.json` text in the file's own style, or returns `None` if one with this slug exists. Only `data_playables` is touched.
pub fn add_playable(text: &str, slug: &str, title: &str, arguments: &[String]) -> Result<Option<String>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!(".release-games.json: {e}"))?;
    let list = v.get("data_playables").and_then(Value::as_array).ok_or(".release-games.json has no data_playables list")?;
    if list.iter().any(|p| p.get("slug").and_then(Value::as_str) == Some(slug)) {
        return Ok(None);
    }
    let start = text.find("\"data_playables\"").ok_or("no data_playables")?;
    let open = start + text[start..].find('[').ok_or("data_playables is not a list")?;
    let mut depth = 0;
    let mut close = None;
    let mut in_str = false;
    let mut prev = ' ';
    for (i, c) in text[open..].char_indices() {
        if in_str {
            if c == '"' && prev != '\\' {
                in_str = false;
            }
        } else {
            match c {
                '"' => in_str = true,
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        prev = c;
    }
    let close = close.ok_or("data_playables is not closed")?;
    let last = text[..close].rfind('}').ok_or("data_playables is empty; add the first entry by hand")?;
    let args = arguments.iter().map(|a| serde_json::to_string(a).unwrap_or_default()).collect::<Vec<_>>().join(", ");
    let entry = format!(
        ",\n    {{\n      \"slug\": \"{slug}\",\n      \"name\": \"{title}\",\n      \"files\": [\"projects/{slug}\"],\n      \"arguments\": [{args}]\n    }}"
    );
    let mut out = String::with_capacity(text.len() + entry.len());
    out.push_str(&text[..=last]);
    out.push_str(&entry);
    out.push_str(&text[last + 1..]);
    serde_json::from_str::<Value>(&out).map_err(|e| format!("internal error: the edited .release-games.json is not JSON: {e}"))?;
    Ok(Some(out))
}

/// Publishes `cfg`'s project into the checkout of the games repository at `games`. `host` adds `--host` to the launcher's arguments (a game whose opponents are
/// bots needs a server of its own); `None` decides from the map (bots).
pub fn publish(cfg: &GameConfig, games: &Path, host: Option<bool>) -> Result<Published, String> {
    let release = games.join(".release-games.json");
    if !release.is_file() || !games.join("projects").is_dir() {
        return Err(format!("{} is not a RedEngineGames checkout (no .release-games.json and projects/): clone https://github.com/kevstermcgee/RedEngineGames beside this project", games.display()));
    }
    let (slug, title) = slug_and_title(&cfg.name)?;
    let dest: PathBuf = games.join("projects").join(&slug);
    let map_abs = cfg.dir.join(&cfg.server.map);
    if !map_abs.is_file() {
        return Err(format!("the server map {} does not exist: build the game first (`game build-all` / your generator)", map_abs.display()));
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    let mut files = 0;
    copy_tree(&cfg.dir, &dest, true, &mut files)?;
    let gj = std::fs::read_to_string(cfg.dir.join("game.json")).map_err(|e| format!("game.json: {e}"))?;
    std::fs::write(dest.join("game.json"), published_game_json(&gj)?).map_err(|e| format!("game.json: {e}"))?;
    let host = host.unwrap_or_else(|| super::game::map_has_bots(&map_abs));
    let mut arguments: Vec<String> = Vec::new();
    if host {
        arguments.push("--host".into());
    }
    arguments.push(format!("content/projects/{slug}/{}", cfg.server.map.replace('\\', "/")));
    let text = std::fs::read_to_string(&release).map_err(|e| format!("{}: {e}", release.display()))?;
    let added_entry = match add_playable(&text, &slug, &title, &arguments)? {
        Some(new) => {
            std::fs::write(&release, new).map_err(|e| format!("{}: {e}", release.display()))?;
            true
        }
        None => false,
    };
    Ok(Published { files, slug, added_entry, arguments })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = "{\n  \"version\": 1,\n  \"game_roots\": [\"games\", \"projects\"],\n  \"data_playables\": [\n    {\n      \"slug\": \"a-game\",\n      \"name\": \"A Game\",\n      \"files\": [\"projects/a-game\"],\n      \"arguments\": [\"content/projects/a-game/maps/main.json\"]\n    }\n  ],\n  \"native_playables\": []\n}\n";

    #[test]
    fn slugs_and_titles() {
        assert_eq!(slug_and_title("great-outdoors").unwrap(), ("great-outdoors".into(), "Great Outdoors".into()));
        assert!(slug_and_title("Great_Outdoors").is_err() && slug_and_title("a--b").is_err() && slug_and_title("").is_err());
    }

    #[test]
    fn a_playable_is_added_in_the_files_own_style_once() {
        let args = vec!["--host".to_string(), "content/projects/kart/maps/main.json".to_string()];
        let out = add_playable(RELEASE, "kart", "Kart", &args).unwrap().expect("new entry");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["data_playables"].as_array().unwrap().len(), 2);
        assert_eq!(v["data_playables"][1]["arguments"][0], "--host");
        assert!(out.contains("\"native_playables\": []") && out.starts_with("{\n  \"version\": 1"), "the rest of the file is untouched:\n{out}");
        assert_eq!(add_playable(&out, "kart", "Kart", &args).unwrap(), None, "a second publish keeps the entry");
    }

    #[test]
    fn the_published_game_json_points_at_the_sibling_engine_and_keeps_a_pin() {
        let a = published_game_json(r#"{"game":1,"name":"k","engine":{"path":"../RedEngine"}}"#).unwrap();
        assert!(a.contains("../../../RedEngine"));
        let b = published_game_json(r#"{"game":1,"name":"k","engine":{"git":"https://x/y.git","ref":"abc"}}"#).unwrap();
        assert!(b.contains("https://x/y.git") && !b.contains("RedEngine"));
    }

    #[test]
    fn publishing_copies_the_project_without_scratch_or_secrets_and_refuses_a_key() {
        let root = std::env::temp_dir().join(format!("red_publish_{}", std::process::id()));
        let (proj, games) = (root.join("kart"), root.join("games"));
        std::fs::create_dir_all(proj.join("maps")).unwrap();
        std::fs::create_dir_all(proj.join("out")).unwrap();
        std::fs::create_dir_all(proj.join("deploy")).unwrap();
        std::fs::create_dir_all(games.join("projects")).unwrap();
        std::fs::write(games.join(".release-games.json"), RELEASE).unwrap();
        std::fs::write(
            proj.join("game.json"),
            r#"{"game":1,"name":"kart","engine":{"path":"../RedEngine"},"maps":["maps/main.json"],"server":{"map":"maps/main.json","port":27015}}"#,
        )
        .unwrap();
        std::fs::write(proj.join("maps/main.json"), r#"{"bots":{"fill":4},"objects":[]}"#).unwrap();
        std::fs::write(proj.join("out/shot.png"), "x").unwrap();
        std::fs::write(proj.join("deploy/install.sh"), "x").unwrap();
        std::fs::write(proj.join("README.md"), "hi").unwrap();
        let cfg = super::super::game::load(&proj).unwrap_or_else(|e| panic!("{e:?}"));
        let done = publish(&cfg, &games, None).unwrap();
        assert_eq!(
            (done.slug.as_str(), done.added_entry, done.arguments.as_slice()),
            ("kart", true, &["--host".to_string(), "content/projects/kart/maps/main.json".to_string()][..])
        );
        let out = games.join("projects/kart");
        assert!(out.join("maps/main.json").is_file() && out.join("README.md").is_file() && !out.join("out").exists() && !out.join("deploy").exists());
        assert!(std::fs::read_to_string(out.join("game.json")).unwrap().contains("../../../RedEngine"));
        // a key in the project stops it
        std::fs::write(proj.join("server.env"), "RED_KEY=abc").unwrap();
        let err = publish(&cfg, &games, None).unwrap_err();
        assert!(err.contains("looks like a secret"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }
}
