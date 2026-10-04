//! Per-game player settings (music/sound on or off) that survive a relaunch, and where they live on disk.
//!
//! A setting is identified by [`key_for`]: the game project's name (from `game.json`, ADR 0024) plus a short
//! hash of its canonical directory, so two different games (even same-named ones in different folders) never
//! share settings, and a map played with no enclosing project (dev/testing) still gets a stable key of its own.
//! Settings "stay with the game", not with the machine as a whole: turning music off in one game never
//! silences another.
//!
//! Same philosophy as [`crate::audio`]: a settings file that is missing, unreadable or corrupt is never an
//! error — [`load`] just returns [`Settings::default`], the same way a missing sound card returns `None`
//! instead of taking the game down.

use serde_json::json;
use std::path::{Path, PathBuf};

/// A player's audio choices for one game. `Default` is "everything on" (today's behavior, unchanged for anyone
/// who has never touched the settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub music: bool,
    pub sfx: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { music: true, sfx: true }
    }
}

/// The directory a `game.json` ancestor-walk starts from, same technique as
/// [`crate::tools::game::destination_for_blueprint`]: canonicalize, then walk parents looking for `game.json`.
fn find_game_dir(map: &Path) -> Option<PathBuf> {
    let map = std::fs::canonicalize(map).ok()?;
    let parent = map.parent()?;
    parent.ancestors().find(|dir| dir.join("game.json").is_file()).map(Path::to_path_buf)
}

/// The stable settings key for the game that owns `map`: `<slugified-name>-<hash12>` when an enclosing
/// `game.json` is found (the hash is of its canonical directory, so a rename of the project keeps its
/// settings but a copy to a new location starts fresh, matching how `game::engine_pin` already treats a path
/// as evidence of *a* location, not an identity); otherwise `map-<hash12>` of the map's own canonical path.
pub fn key_for(map: &Path) -> String {
    if let Some(dir) = find_game_dir(map) {
        let name = crate::tools::game::load(&dir).map(|c| c.name).unwrap_or_else(|_| "game".to_string());
        let slug = crate::tools::adr::slugify(&name, 40);
        let slug = if slug.is_empty() { "game".to_string() } else { slug };
        format!("{slug}-{}", short_hash(&dir))
    } else {
        let canon = std::fs::canonicalize(map).unwrap_or_else(|_| map.to_path_buf());
        format!("map-{}", short_hash(&canon))
    }
}

fn short_hash(path: &Path) -> String {
    let digest = crate::crypto::sha256(path.to_string_lossy().as_bytes());
    crate::crypto::hex(&digest)[..12].to_string()
}

/// `$XDG_CONFIG_HOME` / `%APPDATA%` / `$HOME/.config` — the same three-branch, no-new-dependency approach the
/// upgrade-tooling build cache uses (`src/cli/info.rs::dirs_cache_root`), kept here as the engine's own copy
/// since this module is linked into `re2`/`red_server`, not just the CLI binary.
fn config_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return Some(PathBuf::from(appdata));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
}

/// `RE2_SAVE_DIR`: one folder that holds this game's settings and saved variables, whatever the game's key is. An installed game sets it (to `Saved Games\<game>`), so
/// a player's progress follows the *game*, not the folder it was unpacked into, and survives an update, a reinstall or a move.
const SAVE_DIR_ENV: &str = "RE2_SAVE_DIR";

fn save_dir_override() -> Option<PathBuf> {
    std::env::var_os(SAVE_DIR_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The file `name` for `key`: in `RE2_SAVE_DIR` when it is set, else under the per-user config directory keyed by the game's location.
fn state_path(key: &str, name: &str) -> Option<PathBuf> {
    match save_dir_override() {
        Some(dir) => Some(dir.join(name)),
        None => legacy_path(key, name),
    }
}

/// Where `name` lived before `RE2_SAVE_DIR`: `<config>/red_engine2/games/<key>/<name>`.
fn legacy_path(key: &str, name: &str) -> Option<PathBuf> {
    Some(config_dir()?.join("red_engine2").join("games").join(key).join(name))
}

/// Reads `name` for `key`; with `RE2_SAVE_DIR` set and nothing there yet, the file from the old keyed location is used once (and written to the new place on the next save).
fn read_state(key: &str, name: &str) -> Option<String> {
    let path = state_path(key, name)?;
    std::fs::read_to_string(&path).ok().or_else(|| {
        let old = legacy_path(key, name).filter(|old| *old != path)?;
        save_dir_override()?;
        std::fs::read_to_string(old).ok()
    })
}

fn settings_path(key: &str) -> Option<PathBuf> {
    state_path(key, "settings.json")
}

/// Loads the settings for `key`. Anything short of a clean, parseable file (missing, unreadable, corrupt, no
/// resolvable config directory) silently falls back to [`Settings::default`] — a settings file is a
/// convenience, never a reason to refuse to start.
pub fn load(key: &str) -> Settings {
    (|| -> Option<Settings> {
        let text = read_state(key, "settings.json")?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        Some(Settings {
            music: v.get("music").and_then(serde_json::Value::as_bool).unwrap_or(true),
            sfx: v.get("sfx").and_then(serde_json::Value::as_bool).unwrap_or(true),
        })
    })()
    .unwrap_or_default()
}

/// Saves `s` for `key`, creating its directory if needed.
pub fn save(key: &str, s: &Settings) -> Result<(), String> {
    let path = settings_path(key).ok_or("no resolvable config directory (neither XDG_CONFIG_HOME, APPDATA nor HOME is set)")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let doc = json!({"music": s.music, "sfx": s.sfx});
    std::fs::write(&path, serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

fn vars_path(key: &str) -> Option<PathBuf> {
    state_path(key, "vars.json")
}

/// Loads the variables a game keeps between sessions (`persist`); empty when there is nothing saved (or it is unreadable).
pub fn load_vars(key: &str) -> std::collections::BTreeMap<String, f64> {
    (|| -> Option<std::collections::BTreeMap<String, f64>> {
        let v: serde_json::Value = serde_json::from_str(&read_state(key, "vars.json")?).ok()?;
        Some(v.as_object()?.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?))).collect())
    })()
    .unwrap_or_default()
}

/// Saves the persisted variables for `key`.
pub fn save_vars(key: &str, vars: &std::collections::BTreeMap<String, f64>) -> Result<(), String> {
    let path = vars_path(key).ok_or("no resolvable config directory (neither XDG_CONFIG_HOME, APPDATA nor HOME is set)")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let doc: serde_json::Map<String, serde_json::Value> = vars.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
    std::fs::write(&path, serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("re2_settings_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn key_for_is_stable_and_distinguishes_different_game_directories() {
        let a = scratch("project_a");
        let b = scratch("project_b");
        for dir in [&a, &b] {
            crate::tools::newgame::scaffold(dir, "demo", &crate::tools::game::EngineRef { path: Some("../engine".into()), ..Default::default() }).unwrap();
        }
        let (ka1, ka2, kb) = (key_for(&a.join("maps/main.json")), key_for(&a.join("maps/main.json")), key_for(&b.join("maps/main.json")));
        assert_eq!(ka1, ka2, "the same game must get the same key every time");
        assert_ne!(ka1, kb, "two different directories must not collide even with the same project name 'demo'");
        assert!(ka1.starts_with("demo-"), "{ka1}");
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn a_map_with_no_game_json_still_gets_a_stable_key() {
        let dir = scratch("bare_map");
        std::fs::write(dir.join("x.json"), "{}").unwrap();
        let k1 = key_for(&dir.join("x.json"));
        let k2 = key_for(&dir.join("x.json"));
        assert_eq!(k1, k2);
        assert!(k1.starts_with("map-"), "{k1}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_and_save_round_trip_through_a_fake_home() {
        let home = scratch("home");
        // SAFETY: this process-wide env mutation is confined to this single-threaded test and restored
        // immediately after, like the rest of the test suite's temp-HOME tests.
        let old = std::env::var_os("XDG_CONFIG_HOME");
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &home) };

        assert_eq!(load("some-key"), Settings::default(), "nothing saved yet: defaults");
        let custom = Settings { music: false, sfx: true };
        save("some-key", &custom).unwrap();
        assert_eq!(load("some-key"), custom);

        // Corrupt the file: still never an error, falls back to defaults.
        std::fs::write(settings_path("some-key").unwrap(), "not json").unwrap();
        assert_eq!(load("some-key"), Settings::default());

        // An installed game names its own folder: the settings and the saved variables live there, wherever the game was unpacked, and what was saved the old way is
        // picked up once.
        let saves = scratch("saves");
        unsafe { std::env::set_var(SAVE_DIR_ENV, &saves) };
        assert_eq!(load("some-key"), Settings::default(), "the old file was corrupted above");
        save("some-key", &custom).unwrap();
        let mut vars = std::collections::BTreeMap::new();
        vars.insert("days".to_string(), 12.0);
        save_vars("other-key", &vars).unwrap();
        assert!(saves.join("settings.json").is_file() && saves.join("vars.json").is_file(), "both files are in the save folder");
        assert_eq!(load("a-different-key"), custom, "the key no longer matters: the folder is the game's");
        assert_eq!(load_vars("a-different-key"), vars);
        unsafe { std::env::remove_var(SAVE_DIR_ENV) };
        assert_eq!(load("some-key"), Settings::default(), "without the folder the old keyed place is used again");
        let legacy = Settings { music: true, sfx: false };
        save("legacy-key", &legacy).unwrap();
        let fresh = scratch("saves_fresh");
        unsafe { std::env::set_var(SAVE_DIR_ENV, &fresh) };
        assert_eq!(load("legacy-key"), legacy, "progress made before the save folder existed is not lost");
        unsafe { std::env::remove_var(SAVE_DIR_ENV) };
        let _ = (std::fs::remove_dir_all(&saves), std::fs::remove_dir_all(&fresh));

        match old {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
