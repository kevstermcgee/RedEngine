//! Game projects: the layer that lets a game live in its **own** repo and use Red as a dependency.
//!
//! The Cheddar game began as a copy of this whole repository. That worked until the engine improved: fixes such as
//! blocker-naming walks, fresh docs and headless builds could not flow into a fork, and the fork's own docs went stale.
//! A game project is instead a small directory:
//!
//! ```text
//! game.json                  name, pinned engine (git ref or local path), blueprints, maps, server settings
//! blueprints/*.blueprint.json  what the game's maps ARE (see [`super::blueprint`])
//! maps/*.json                built from the blueprints (and checked in, so the game can run without building)
//! STATUS.md, CLAUDE.md       handoff and working rules for the next person or AI
//! scripts/red, scripts/red.ps1   finds/builds the pinned engine and forwards to it
//! ```
//!
//! `red_engine2 game check` is the one command that answers "is this project healthy?": every blueprint still builds
//! and still equals its committed map, and every map passes its own `checks`. `game serve` / `game play` start the
//! headless server and the game client on the project's map. Custom Rust is only needed when the data-driven layer
//! (rules as data, ADR 0020) is not enough, and then the crate depends on `red_engine2` as a library; it never copies it.

use super::blueprint;
use super::verify;
use crate::strict::check_keys;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The `game.json` format version this engine reads.
pub const GAME_VERSION: u64 = 1;
/// Where the pinned engine comes from.
#[derive(Debug, Clone, Default)]
pub struct EngineRef {
    /// A git URL to clone (with `git_ref`).
    pub git: Option<String>,
    /// Branch, tag or commit for `git`.
    pub git_ref: Option<String>,
    /// A local engine checkout (relative to the project), for engine development.
    pub path: Option<String>,
}

/// The `server` block of `game.json`.
#[derive(Debug, Clone)]
pub struct ServerCfg {
    /// Map the dedicated server loads (relative to the project).
    pub map: String,
    /// UDP port.
    pub port: u16,
    /// Spawn group the server uses (empty = all spawns).
    pub spawn_group: String,
    /// Extra `red_server` arguments.
    pub args: Vec<String>,
}

/// A parsed `game.json`.
#[derive(Debug, Clone)]
pub struct GameConfig {
    /// The project directory.
    pub dir: PathBuf,
    /// Project name.
    pub name: String,
    /// Pinned engine.
    pub engine: EngineRef,
    /// Blueprint files, relative to `dir`.
    pub blueprints: Vec<String>,
    /// Map files, relative to `dir`.
    pub maps: Vec<String>,
    /// Dedicated-server settings.
    pub server: ServerCfg,
}

const TOP: &[&str] = &["game", "name", "engine", "blueprints", "maps", "server"];

fn strs(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// Parses `game.json` text (`dir` is where it lives). Errors are `path: message` lines.
pub fn parse(dir: &Path, text: &str) -> Result<GameConfig, Vec<String>> {
    let v: Value = serde_json::from_str(text).map_err(|e| vec![format!("game.json: not valid JSON: {e}")])?;
    let mut errs = Vec::new();
    let Some(root) = v.as_object() else { return Err(vec!["game.json: expected an object".into()]) };
    check_keys(&mut errs, "", root, TOP);
    if root.get("game").and_then(Value::as_u64) != Some(GAME_VERSION) {
        errs.push(format!("game: missing or unsupported version (write \"game\": {GAME_VERSION})"));
    }
    let mut engine = EngineRef::default();
    if let Some(e) = root.get("engine").and_then(Value::as_object) {
        check_keys(&mut errs, "engine", e, &["git", "ref", "path"]);
        engine = EngineRef {
            git: e.get("git").and_then(Value::as_str).map(str::to_string),
            git_ref: e.get("ref").and_then(Value::as_str).map(str::to_string),
            path: e.get("path").and_then(Value::as_str).map(str::to_string),
        };
    }
    if engine.git.is_none() && engine.path.is_none() {
        errs.push("engine: give {\"git\": URL, \"ref\": \"master\"} or {\"path\": \"../red-engine-2\"} so the project knows which engine to use".into());
    }
    let maps = strs(root.get("maps"));
    let mut server = ServerCfg { map: maps.first().cloned().unwrap_or_default(), port: crate::net::DEFAULT_PORT, spawn_group: String::new(), args: Vec::new() };
    if let Some(s) = root.get("server").and_then(Value::as_object) {
        check_keys(&mut errs, "server", s, &["map", "port", "spawn_group", "args"]);
        if let Some(m) = s.get("map").and_then(Value::as_str) {
            server.map = m.to_string();
        }
        if let Some(p) = s.get("port").and_then(Value::as_u64) {
            server.port = u16::try_from(p).unwrap_or(crate::net::DEFAULT_PORT);
        }
        server.spawn_group = s.get("spawn_group").and_then(Value::as_str).unwrap_or("").to_string();
        server.args = strs(s.get("args"));
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(GameConfig {
        dir: dir.to_path_buf(),
        name: root.get("name").and_then(Value::as_str).unwrap_or("game").to_string(),
        engine,
        blueprints: strs(root.get("blueprints")),
        maps,
        server,
    })
}

/// Loads `<dir>/game.json`.
pub fn load(dir: &Path) -> Result<GameConfig, Vec<String>> {
    let p = dir.join("game.json");
    let text = std::fs::read_to_string(&p)
        .map_err(|e| vec![format!("{}: {e} (run this in a game project, or `red_engine2 new-game <dir>` to make one)", p.display())])?;
    parse(dir, &text)
}

/// One line of a project report.
pub struct Line {
    /// Whether it counts as a failure.
    pub failed: bool,
    /// Text (may be multi-line).
    pub text: String,
}

/// The outcome of [`check`].
pub struct CheckReport {
    /// Lines in the order they were produced.
    pub lines: Vec<Line>,
}

impl CheckReport {
    /// Number of failed lines.
    pub fn failed(&self) -> usize {
        self.lines.iter().filter(|l| l.failed).count()
    }
    /// The report as text, one entry per line, ending with a verdict.
    pub fn render(&self) -> String {
        let mut s = String::new();
        for l in &self.lines {
            s.push_str(&format!("{} {}\n", if l.failed { "FAIL" } else { "ok  " }, l.text));
        }
        s.push_str(&if self.failed() == 0 { "project healthy\n".to_string() } else { format!("{} problem(s)\n", self.failed()) });
        s
    }
}

/// Default map path for a blueprint (`blueprints/x.blueprint.json` -> `blueprints/x.json`, else `x.map.json`), matching `build`.
pub fn blueprint_out(bp: &Path) -> PathBuf {
    let stem = bp.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "map".into());
    match stem.strip_suffix(".blueprint") {
        Some(base) => bp.with_file_name(format!("{base}.json")),
        None => bp.with_file_name(format!("{stem}.map.json")),
    }
}

/// The map a blueprint builds into: the `maps` entry with the same base name if there is one, else beside the blueprint.
pub fn map_for(cfg: &GameConfig, bp: &str) -> PathBuf {
    let base = Path::new(bp).file_stem().map(|s| s.to_string_lossy().trim_end_matches(".blueprint").to_string()).unwrap_or_default();
    cfg.maps
        .iter()
        .find(|m| Path::new(m).file_stem().is_some_and(|s| s.to_string_lossy() == base))
        .map(|m| cfg.dir.join(m))
        .unwrap_or_else(|| blueprint_out(&cfg.dir.join(bp)))
}

/// If `bp` belongs to an enclosing game project, returns the map destination configured for it.
/// An unlisted blueprint keeps standalone `build` behavior even when it happens to live under a game directory.
pub fn destination_for_blueprint(bp: &Path) -> Result<Option<PathBuf>, Vec<String>> {
    let bp = std::fs::canonicalize(bp).map_err(|e| vec![format!("{}: {e}", bp.display())])?;
    let Some(parent) = bp.parent() else { return Ok(None) };
    for dir in parent.ancestors() {
        if !dir.join("game.json").is_file() {
            continue;
        }
        let cfg = load(dir)?;
        for configured in &cfg.blueprints {
            let candidate = cfg.dir.join(configured);
            if std::fs::canonicalize(&candidate).ok().as_ref() == Some(&bp) {
                return Ok(Some(map_for(&cfg, configured)));
            }
        }
        return Ok(None);
    }
    Ok(None)
}

/// Compiles every blueprint and writes its map. Returns one line per blueprint.
pub fn build_all(cfg: &GameConfig) -> Vec<Line> {
    let mut out = Vec::new();
    for bp in &cfg.blueprints {
        let path = cfg.dir.join(bp);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                out.push(Line { failed: true, text: format!("{bp}: {e}") });
                continue;
            }
        };
        let compiled =
            serde_json::from_str::<Value>(&text).map_err(|e| vec![format!("not valid JSON: {e}")]).and_then(|v| blueprint::compile_in(&v, path.parent()));
        match compiled {
            Ok(b) => {
                let dest = map_for(cfg, bp);
                if let Some(parent) = dest.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&dest, &b.scene_text) {
                    Ok(()) => out.push(Line {
                        failed: b.errors() > 0,
                        text: format!(
                            "{bp} -> {} ({})",
                            dest.strip_prefix(&cfg.dir).unwrap_or(&dest).display(),
                            b.summary.first().cloned().unwrap_or_default()
                        ),
                    }),
                    Err(e) => out.push(Line { failed: true, text: format!("{}: {e}", dest.display()) }),
                }
            }
            Err(errs) => out.push(Line { failed: true, text: format!("{bp}: {}", errs.join("\n     ")) }),
        }
    }
    out
}

/// The project health check: blueprints build and equal their maps, maps pass their `checks`, handoff exists.
pub fn check(cfg: &GameConfig, views: bool) -> CheckReport {
    let mut lines = Vec::new();
    for bp in &cfg.blueprints {
        let path = cfg.dir.join(bp);
        let compiled = std::fs::read_to_string(&path)
            .map_err(|e| vec![e.to_string()])
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| vec![format!("not valid JSON: {e}")]))
            .and_then(|v| blueprint::compile_in(&v, path.parent()));
        match compiled {
            Err(errs) => lines.push(Line { failed: true, text: format!("blueprint {bp}: {}", errs.join("\n     ")) }),
            Ok(b) => {
                let dest = map_for(cfg, bp);
                match std::fs::read_to_string(&dest) {
                    Err(_) => lines.push(Line { failed: true, text: format!("blueprint {bp}: map {} is missing: run `scripts/red build-all`", dest.display()) }),
                    Ok(existing) if existing.replace("\r\n", "\n") != b.scene_text.replace("\r\n", "\n") => {
                        lines.push(Line { failed: true, text: format!("blueprint {bp}: STALE, {} differs from what it builds: run `scripts/red build-all` (or delete the blueprint if the map is now hand-edited)", dest.display()) })
                    }
                    Ok(_) => lines.push(Line { failed: b.errors() > 0, text: format!("blueprint {bp} builds and matches {} ({} lint error(s))", dest.strip_prefix(&cfg.dir).unwrap_or(&dest).display(), b.errors()) }),
                }
            }
        }
    }
    for m in &cfg.maps {
        let path = cfg.dir.join(m);
        match verify::run(&path, &verify::Options { skip_views: !views, out_dir: Some(cfg.dir.join("out/verify")), ..Default::default() }) {
            Err(e) => lines.push(Line { failed: true, text: format!("map {m}: {e}") }),
            Ok(r) => {
                let failed = r.failed();
                let mut text = format!("map {m}: {} check(s), {failed} failed", r.results.len());
                for c in r.results.iter().filter(|c| !c.ok) {
                    text.push_str(&format!("\n     FAIL {}  {}", c.name, c.detail));
                    if let Some(a) = &c.artifact {
                        text.push_str(&format!("\n     see {}", a.display()));
                    }
                }
                lines.push(Line { failed: failed > 0, text });
            }
        }
    }
    if !cfg.maps.contains(&cfg.server.map) {
        lines.push(Line { failed: true, text: format!("server.map '{}' is not one of the project's maps", cfg.server.map) });
    }
    if !cfg.dir.join("STATUS.md").exists() {
        lines.push(Line { failed: false, text: "no STATUS.md yet (the handoff file): `red_engine2 status --init`".into() });
    }
    CheckReport { lines }
}

/// Arguments for `red_server` from the project's `server` block.
pub fn server_args(cfg: &GameConfig) -> Vec<String> {
    let mut a = vec!["--map".to_string(), cfg.dir.join(&cfg.server.map).display().to_string(), "--port".to_string(), cfg.server.port.to_string()];
    if !cfg.server.spawn_group.is_empty() {
        a.push("--spawn-group".into());
        a.push(cfg.server.spawn_group.clone());
    }
    a.extend(cfg.server.args.iter().cloned());
    a
}

/// A sibling executable of the running one (`red_server` next to `red_engine2`), if it exists.
pub fn sibling_exe(name: &str) -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let dir = me.parent()?;
    [dir.join(name), dir.join(format!("{name}.exe"))].into_iter().find(|p| p.exists())
}

/// Text for `game info`: the resolved project settings and the commands to run.
pub fn info(cfg: &GameConfig) -> String {
    let mut s = format!("game '{}' in {}\n", cfg.name, cfg.dir.display());
    s.push_str(&format!(
        "engine   {}\n",
        match (&cfg.engine.path, &cfg.engine.git) {
            (Some(p), _) => format!("local checkout {p}"),
            (None, Some(g)) => format!("{g} @ {}", cfg.engine.git_ref.as_deref().unwrap_or("master")),
            _ => "unspecified".into(),
        }
    ));
    s.push_str(&format!("blueprints  {}\nmaps        {}\n", cfg.blueprints.join(", "), cfg.maps.join(", ")));
    s.push_str(&format!(
        "server   udp {} on {}{}\n",
        cfg.server.port,
        cfg.server.map,
        if cfg.server.spawn_group.is_empty() { String::new() } else { format!(" (spawn group {})", cfg.server.spawn_group) }
    ));
    s.push_str("commands scripts/red check | build-all | serve | play [HOST:PORT] | <any red_engine2 command>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"game":1,"name":"t","engine":{"path":"../engine"},"blueprints":["blueprints/main.blueprint.json"],"maps":["maps/main.json"],"server":{"port":28000,"spawn_group":"duel"}}"#;

    #[test]
    fn config_parses_with_defaults_and_rejects_typos() {
        let c = parse(Path::new("."), GOOD).unwrap();
        assert_eq!((c.name.as_str(), c.server.port, c.server.map.as_str()), ("t", 28000, "maps/main.json"));
        assert!(server_args(&c).join(" ").contains("--spawn-group duel"));
        let e = parse(Path::new("."), r#"{"game":1,"engine":{"path":"x"},"mapz":[]}"#).unwrap_err();
        assert!(e.iter().any(|m| m.contains("unknown field")), "{e:?}");
        let e = parse(Path::new("."), r#"{"game":1}"#).unwrap_err();
        assert!(e.iter().any(|m| m.contains("engine")), "{e:?}");
        assert!(parse(Path::new("."), r#"{"engine":{"path":"x"}}"#).is_err(), "the version key is required");
    }

    #[test]
    fn a_fresh_project_builds_checks_clean_and_notices_drift() {
        let dir = std::env::temp_dir().join(format!("re2_game_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        super::super::newgame::scaffold(&dir, "demo", &EngineRef { path: Some("../red-engine-2".into()), ..Default::default() }).unwrap();
        let cfg = load(&dir).unwrap();
        let r = check(&cfg, false);
        assert_eq!(r.failed(), 0, "{}", r.render());
        // Hand-edit the map: the blueprint no longer produces it, and `check` must say so.
        let map = dir.join("maps/main.json");
        let text = std::fs::read_to_string(&map).unwrap();
        std::fs::write(&map, text.replace("\"height\": 2.8", "\"height\": 3.1")).unwrap();
        let r = check(&cfg, false);
        assert!(r.render().contains("STALE"), "{}", r.render());
        // Rebuilding fixes it.
        assert!(build_all(&cfg).iter().all(|l| !l.failed));
        assert_eq!(check(&cfg, false).failed(), 0);
        assert_eq!(
            destination_for_blueprint(&dir.join("blueprints/main.blueprint.json")).unwrap(),
            Some(std::fs::canonicalize(dir.join("maps/main.json")).unwrap())
        );

        let other = dir.join("blueprints/other.blueprint.json");
        std::fs::write(&other, "{}").unwrap();
        assert_eq!(destination_for_blueprint(&other).unwrap(), None);
    }
}
