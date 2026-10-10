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
//! and still equals its committed map, and every map passes its own `checks`. `game play-local` starts the map with
//! no network dependency; `game serve` / `game play` start the headless server and an online client. Custom Rust is only needed when the data-driven layer
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
    /// The game's stable identity (`"id"`): what its saved settings and progress are filed under, whatever folder the game is unpacked into. `None` for a project
    /// that has not chosen one: it is then identified by its name and the folder it lives in (see `settings::key_for`).
    pub id: Option<String>,
    /// Pinned engine.
    pub engine: EngineRef,
    /// Blueprint files, relative to `dir`.
    pub blueprints: Vec<String>,
    /// Map files, relative to `dir`.
    pub maps: Vec<String>,
    /// Dedicated-server settings.
    pub server: ServerCfg,
    /// What the game declares it is and where it runs (`red_engine2 capabilities`), if it says: checked against the support matrix by `game check`.
    pub capabilities: Option<red2d::caps::Capabilities>,
}

const TOP: &[&str] = &["game", "id", "name", "engine", "blueprints", "maps", "server", "capabilities"];

/// Whether `id` is a valid game id: 1 to 40 lowercase letters and digits, in groups separated by single hyphens.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

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
    let id = match root.get("id") {
        None => None,
        Some(v) => match v.as_str().filter(|s| valid_id(s)) {
            Some(s) => Some(s.to_string()),
            None => {
                errs.push(
                    "id: 1 to 40 lowercase letters and digits, in groups separated by single hyphens (it names this game's saved settings and progress)".into(),
                );
                None
            }
        },
    };
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
    let mut capabilities = None;
    if let Some(c) = root.get("capabilities") {
        let (parsed, problems) = red2d::caps::parse(c);
        errs.extend(problems.iter().map(|p| p.to_string()));
        if let Some(parsed) = parsed {
            errs.extend(red2d::caps::check(&parsed).iter().map(|p| p.to_string()));
            capabilities = Some(parsed);
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(GameConfig {
        capabilities,
        dir: dir.to_path_buf(),
        name: root.get("name").and_then(Value::as_str).unwrap_or("game").to_string(),
        id,
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
#[derive(Debug, Clone)]
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

/// Remembers which maps already passed their `checks` (`out/cache/check-<hash>`), so `game check` re-verifies only what changed. A map's key is
/// the hash of: the running engine binary (size + mtime, so any rebuild invalidates), the map's own bytes, and every *other* `.json` file in the
/// project (blueprints, local prefab libraries, `game.json`). Only passes are stored, so a failure always re-runs and prints its evidence.
/// Views (golden images) are never cached: they depend on the renderer.
struct CheckCache {
    dir: PathBuf,
    shared: crate::crypto::Sha256,
}

impl CheckCache {
    fn new(cfg: &GameConfig) -> CheckCache {
        let mut h = crate::crypto::Sha256::new();
        h.update(b"game-check-cache-v1|");
        h.update(env!("CARGO_PKG_VERSION").as_bytes());
        if let Some(meta) = std::env::current_exe().ok().and_then(|e| std::fs::metadata(e).ok()) {
            let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
            h.update(format!("|engine {} {mtime}|", meta.len()).as_bytes());
        }
        let maps: Vec<PathBuf> = cfg.maps.iter().map(|m| cfg.dir.join(m)).collect();
        let mut files = Vec::new();
        collect_json(&cfg.dir, &mut files);
        files.sort();
        for f in files.iter().filter(|f| !maps.contains(f)) {
            h.update(f.strip_prefix(&cfg.dir).unwrap_or(f).to_string_lossy().replace('\\', "/").as_bytes());
            h.update(&std::fs::read(f).map(|b| crate::crypto::sha256(&b).to_vec()).unwrap_or_default());
        }
        CheckCache { dir: cfg.dir.join("out").join("cache"), shared: h }
    }

    fn file_for(&self, map: &Path) -> Option<PathBuf> {
        let bytes = std::fs::read(map).ok()?;
        let mut h = self.shared.clone();
        h.update(&crate::crypto::sha256(&bytes));
        Some(self.dir.join(format!("check-{}", &crate::crypto::hex(&h.finish())[..32])))
    }

    /// The number of checks that passed last time for these exact inputs.
    fn passed(&self, map: &Path) -> Option<usize> {
        std::fs::read_to_string(self.file_for(map)?).ok()?.trim().parse().ok()
    }

    fn record(&self, map: &Path, checks: usize) {
        if let Some(f) = self.file_for(map) {
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = std::fs::write(f, checks.to_string());
        }
    }
}

/// Every `.json` file under `dir`, skipping build output, caches and VCS metadata.
fn collect_json(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !matches!(name.as_str(), "out" | "target" | ".git" | "node_modules") {
                collect_json(&p, out);
            }
        } else if name.ends_with(".json") {
            out.push(p);
        }
    }
}

/// Whether each blueprint still builds and still equals its committed map: `check`'s first section, pulled out so
/// `game upgrade plan` can read the same drift evidence without running every other check too.
pub fn blueprint_drift(cfg: &GameConfig) -> Vec<Line> {
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
    lines
}

/// The project health check: blueprints build and equal their maps, maps pass their own `checks`, handoff exists.
pub fn check(cfg: &GameConfig, views: bool) -> CheckReport {
    let mut lines = blueprint_drift(cfg);
    lines.push(match &cfg.capabilities {
        Some(c) => Line {
            failed: false,
            text: format!(
                "capabilities: {} on {}, {} networking (checked against the support matrix: `capabilities`)",
                c.presentation.name(),
                c.platforms.iter().map(|p| p.name()).collect::<Vec<_>>().join("+"),
                c.networking.name()
            ),
        },
        None => Line { failed: false, text: "capabilities: not declared (assumed 3d on windows+linux, authoritative): add a `capabilities` block to game.json so a target the engine cannot deliver fails here".into() },
    });
    let cache = (!views && std::env::var_os("RED_NO_CACHE").is_none()).then(|| CheckCache::new(cfg));
    for m in &cfg.maps {
        let path = cfg.dir.join(m);
        if let Some(n) = cache.as_ref().and_then(|c| c.passed(&path)) {
            lines.push(Line { failed: false, text: format!("map {m}: {n} check(s), 0 failed (cached: this map, the project's other JSON and the engine binary are unchanged since it last passed; RED_NO_CACHE=1 re-runs)") });
            continue;
        }
        match verify::run(&path, &verify::Options { skip_views: !views, out_dir: Some(cfg.dir.join("out/verify")), ..Default::default() }) {
            Err(e) => lines.push(Line { failed: true, text: format!("map {m}: {e}") }),
            Ok(r) => {
                let failed = r.failed();
                if failed == 0 {
                    if let Some(c) = &cache {
                        c.record(&path, r.results.len());
                    }
                }
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
    for m in &cfg.maps {
        if let Some(line) = std::fs::read_to_string(cfg.dir.join(m)).ok().and_then(|text| avatar_line(m, &text)) {
            lines.push(line);
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

/// A map with bots or spawn points is played online, where every other player is drawn with an avatar the client prepared before its renderer existed. This line checks
/// that the client will have one for every body the map's people and bots wear (the bug that shipped Trigger Happy with invisible enemies: a pool sized for the humans'
/// body only). `None` for a map that is not played online. The check builds the pool the way the client does ([`crate::net::session::avatars_made`]).
pub fn avatar_line(map: &str, text: &str) -> Option<Line> {
    let scene = crate::schema::parse_scene(text).ok()?;
    if scene.race.is_some() {
        return Some(kart_line(map, &scene));
    }
    let has_spawns = crate::sim::spawns::parse_spawns(text).is_ok_and(|s| !s.is_empty());
    if scene.bots.fill == 0 && !has_spawns {
        return None;
    }
    let made = match crate::net::session::avatars_made(text) {
        Ok(m) => m,
        Err(e) => return Some(Line { failed: true, text: format!("map {map}: avatars: {}", e.join("; ")) }),
    };
    let index = |who: crate::player::Character| crate::player::Character::ALL.iter().position(|c| *c == who).unwrap_or(0);
    let mut missing: Vec<String> = Vec::new();
    for k in 0..scene.bots.fill {
        let spec = scene.bots.spec(k);
        if made[index(spec.character)] == 0 {
            missing.push(format!(
                "bot '{}' wears {} and the client prepares no {} avatar: it would be invisible",
                spec.name,
                spec.character.name(),
                spec.character.name()
            ));
        }
    }
    let plan = crate::net::session::avatar_plan(&scene);
    for who in plan.short() {
        missing.push(format!(
            "a match can field {} {} at once but the client prepares {}: the rest are drawn in another costume",
            plan.needed[index(who)],
            who.name(),
            plan.pool[index(who)]
        ));
    }
    let held: Vec<String> = crate::player::Character::ALL.iter().zip(made).filter(|(_, n)| *n > 0).map(|(c, n)| format!("{} x{n}", c.name())).collect();
    Some(if missing.is_empty() {
        Line { failed: false, text: format!("map {map}: avatars: every body in play is drawn ({})", held.join(", ")) }
    } else {
        Line { failed: true, text: format!("map {map}: avatars: {}", missing.join("\n     ")) }
    })
}

/// A race draws karts, never bodies: every animal needs a model. The map's own `kart_<animal>` object is used when it has one; without it the client draws the
/// built-in fallback kart, which works but looks the same for every animal, so it is reported (a note, not a failure).
fn kart_line(map: &str, scene: &crate::schema::Scene) -> Line {
    use crate::sim::kart::Driver;
    let own: Vec<&str> = Driver::ALL.iter().filter(|d| scene.objects.iter().any(|o| o.id == crate::net::fleet::model_id(**d))).map(|d| d.name()).collect();
    let plain: Vec<&str> = Driver::ALL.iter().map(|d| d.name()).filter(|n| !own.contains(n)).collect();
    let text = if plain.is_empty() {
        format!("map {map}: karts: all {} drivers have their own model in the map", own.len())
    } else {
        format!(
            "map {map}: karts: {} of {} drivers have their own model; {} are drawn as the plain built-in kart (add `kart_<animal>` objects)",
            own.len(),
            Driver::ALL.len(),
            plain.join(", ")
        )
    };
    Line { failed: false, text }
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

/// Whether the scene at `map` asks for bots (`bots.fill` above zero): a game whose opponents are bots is played against a server of its own
/// (`re2 --host`, ADR 0054), because bots live in the server's simulation and not in the offline client.
pub fn map_has_bots(map: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(map) else { return false };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    v.get("bots").and_then(|b| b.get("fill")).and_then(serde_json::Value::as_u64).is_some_and(|n| n > 0)
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
    s.push_str("commands scripts/red check | build-all | play-local | serve | play [HOST:PORT] | <any red_engine2 command>\n");
    s
}

/// The engine commit a release is pinned to: where to clone it from and exactly which commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnginePin {
    /// The clone URL (`origin` of the checkout).
    pub url: String,
    /// The full commit hash.
    pub sha: String,
}

/// Rewrites `game.json` text so the engine is pinned to `git` at commit `sha` (a release: its client and server are built from exactly that
/// engine). Every other key keeps its place; the result is checked to still parse as a project.
pub fn pin_text(game_json: &str, git: &str, sha: &str) -> Result<String, String> {
    with_engine(game_json, serde_json::json!({"git": git, "ref": sha}))
}

/// Rewrites `game.json` text so the engine is the local checkout at `path` (relative to the project): development follows that checkout.
pub fn unpin_text(game_json: &str, path: &str) -> Result<String, String> {
    with_engine(game_json, serde_json::json!({"path": path}))
}

fn with_engine(game_json: &str, engine: Value) -> Result<String, String> {
    let mut v: Value = serde_json::from_str(game_json).map_err(|e| format!("game.json: not valid JSON: {e}"))?;
    v.as_object_mut().ok_or("game.json: expected an object")?.insert("engine".into(), engine);
    let out = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n";
    parse(Path::new("."), &out).map_err(|e| e.join("; "))?;
    Ok(out)
}

/// The engine checkout `game pin` reads: `explicit`, else the project's local `engine.path` (relative to the project), else the checkout this
/// binary was built from.
pub fn engine_checkout(cfg: &GameConfig, explicit: Option<&Path>) -> Result<PathBuf, String> {
    let dir = match (explicit, &cfg.engine.path) {
        (Some(e), _) => e.to_path_buf(),
        (None, Some(p)) => cfg.dir.join(p),
        (None, None) => PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    };
    if dir.join(".git").exists() {
        Ok(dir)
    } else {
        Err(format!("{} is not a git checkout of the engine: pass --engine <dir>", dir.display()))
    }
}

/// Reads the pin from an engine checkout: its HEAD commit and `origin` URL. Refuses what could not be reproduced: uncommitted changes to tracked
/// files (unless `allow_dirty`), a commit no remote branch contains (a clone of the pin would not find it; push first), and — when `expect_sha`
/// is given — a HEAD that is not that exact commit. `checkout` is usually a branch, not a fixed point: between a `game upgrade verify` run and
/// the person actually pinning, its HEAD can move on. Without this check, `game pin --engine <checkout>` would silently pin whatever that
/// checkout happens to be at *now* — a commit nobody verified — not the one the report actually certified.
pub fn engine_pin(checkout: &Path, allow_dirty: bool, expect_sha: Option<&str>) -> Result<EnginePin, String> {
    let git = |args: &[&str]| -> Result<String, String> {
        let out = std::process::Command::new("git").arg("-C").arg(checkout).args(args).output().map_err(|e| format!("git: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
        }
    };
    let sha = git(&["rev-parse", "HEAD"])?;
    if let Some(expect) = expect_sha {
        if sha != expect {
            return Err(format!(
                "{} is now at {} but the verified target was {}: the checkout moved since `game upgrade verify` ran — \
                 check out the verified commit exactly before pinning, or re-verify its current HEAD",
                checkout.display(),
                &sha[..sha.len().min(12)],
                &expect[..expect.len().min(12)]
            ));
        }
    }
    let url = git(&["remote", "get-url", "origin"]).map_err(|e| format!("{e} (the pin needs an `origin` remote to clone from)"))?;
    if !allow_dirty && !git(&["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
        return Err(format!("{} has uncommitted changes, so HEAD is not what you built: commit them (or --allow-dirty)", checkout.display()));
    }
    if git(&["branch", "-r", "--contains", &sha])?.is_empty() {
        return Err(format!("commit {} is on no remote branch, so a clone could not fetch it: push it first", &sha[..sha.len().min(12)]));
    }
    Ok(EnginePin { url, sha })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_race_map_is_audited_for_karts_not_bodies() {
        let text = r#"{"camera":{"position":[0,1,5],"target":[0,0,0]},
            "zones":[{"id":"a","rect":[0,0,1,1]},{"id":"b","rect":[5,0,6,1]},{"id":"c","rect":[0,5,1,6]}],
            "race":{"laps":1,"gates":["a","b","c"]},
            "spawns":[{"id":"s","position":[0,0,0],"yaw_deg":0}],
            "objects":[{"id":"kart_duck","type":"box","size":[1,1,1],"position":[0,-50,0],"collide":false}]}"#;
        let line = avatar_line("race.json", text).expect("a race is played online");
        assert!(!line.failed, "a plain kart is a note, not a failure: {}", line.text);
        assert!(line.text.contains("karts: 1 of 8") && line.text.contains("Bunny"), "{}", line.text);
    }

    use super::*;

    const GOOD: &str = r#"{"game":1,"name":"t","engine":{"path":"../engine"},"blueprints":["blueprints/main.blueprint.json"],"maps":["maps/main.json"],"server":{"port":28000,"spawn_group":"duel"}}"#;

    #[test]
    fn a_game_id_is_optional_and_must_be_a_plain_slug() {
        assert_eq!(parse(Path::new("."), GOOD).unwrap().id, None, "no id: the project is identified by name and folder, as before");
        let with = |id: &str| GOOD.replace("\"name\":\"t\"", &format!("\"name\":\"t\",\"id\":{id}"));
        assert_eq!(parse(Path::new("."), &with("\"moon-delivery-2\"")).unwrap().id.as_deref(), Some("moon-delivery-2"));
        for bad in ["\"\"", "\"Marcel\"", "\"two  words\"", "\"a--b\"", "\"-a\"", "\"a-\"", "\"snake_case\"", "7", &format!("\"{}\"", "x".repeat(41))] {
            let errs = parse(Path::new("."), &with(bad)).expect_err(bad);
            assert!(errs.iter().any(|e| e.starts_with("id:")), "{bad}: {errs:?}");
        }
    }

    #[test]
    fn config_parses_with_defaults_and_rejects_typos() {
        let c = parse(Path::new("."), GOOD).unwrap();
        assert_eq!((c.name.as_str(), c.server.port, c.server.map.as_str()), ("t", 28000, "maps/main.json"));
        assert!(server_args(&c).join(" ").contains("--spawn-group duel"));
        assert!(info(&c).contains("play-local"), "local single-player must be discoverable for every project");
        let e = parse(Path::new("."), r#"{"game":1,"engine":{"path":"x"},"mapz":[]}"#).unwrap_err();
        assert!(e.iter().any(|m| m.contains("unknown field")), "{e:?}");
        let e = parse(Path::new("."), r#"{"game":1}"#).unwrap_err();
        assert!(e.iter().any(|m| m.contains("engine")), "{e:?}");
        assert!(parse(Path::new("."), r#"{"engine":{"path":"x"}}"#).is_err(), "the version key is required");
    }

    #[test]
    fn a_map_with_bots_is_played_against_a_host_of_its_own() {
        let dir = std::env::temp_dir().join(format!("re2_bots_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (with, without, broken) = (dir.join("with.json"), dir.join("without.json"), dir.join("broken.json"));
        std::fs::write(&with, r#"{"bots":{"fill":4}}"#).unwrap();
        std::fs::write(&without, r#"{"bots":{"fill":0}}"#).unwrap();
        std::fs::write(&broken, "not json").unwrap();
        assert!(map_has_bots(&with));
        assert!(!map_has_bots(&without) && !map_has_bots(&broken) && !map_has_bots(&dir.join("missing.json")));
        let _ = std::fs::remove_dir_all(&dir);
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

    /// `examples/test_lab.json` with the arena game's shape: Humans forced, a roster of four other bodies.
    fn arena_text(bots: &str) -> String {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json");
        let mut root: Value = serde_json::from_str(&std::fs::read_to_string(source).unwrap()).unwrap();
        root["player"]["character"] = "human".into();
        root["bots"] = serde_json::from_str(bots).unwrap();
        root.to_string()
    }

    #[test]
    fn game_check_confirms_the_client_will_draw_every_body_the_bots_wear() {
        let roster = r#"{"fill":8,"roster":[{"name":"Cow","character":"cowboy"},{"name":"Wiz","character":"wizard"},{"name":"Ali","character":"alien"},{"name":"Rob","character":"robot"}]}"#;
        let line = avatar_line("arena.json", &arena_text(roster)).expect("a map with bots is played online");
        assert!(!line.failed, "{}", line.text);
        assert!(line.text.contains("Cowboy x4") && line.text.contains("Robot x4") && line.text.contains("Human x12"), "{}", line.text);
        // A map nobody plays online (no bots, no spawn points) is not this check's business.
        let quiet = r#"{"camera":{"position":[0,1.7,0]},"objects":[]}"#;
        assert!(avatar_line("quiet.json", quiet).is_none());
    }

    #[test]
    fn pin_and_unpin_rewrite_only_the_engine_block() {
        let pinned = pin_text(GOOD, "https://example.com/red.git", "d96b6c009d2027aec4efdc0acdb4d1f7fd6f91db").unwrap();
        let c = parse(Path::new("."), &pinned).unwrap();
        assert_eq!(c.engine.git.as_deref(), Some("https://example.com/red.git"));
        assert_eq!(c.engine.git_ref.as_deref(), Some("d96b6c009d2027aec4efdc0acdb4d1f7fd6f91db"));
        assert_eq!(c.engine.path, None);
        assert_eq!((c.name.as_str(), c.server.port, c.maps.len()), ("t", 28000, 1), "everything else is untouched");
        assert!(info(&c).contains("d96b6c009d"), "the pin is visible in `game info`");
        let back = parse(Path::new("."), &unpin_text(&pinned, "../RedEngine").unwrap()).unwrap();
        assert_eq!((back.engine.path.as_deref(), back.engine.git.as_deref()), (Some("../RedEngine"), None));
        assert!(pin_text("[1]", "u", "s").is_err() && pin_text("nope", "u", "s").is_err(), "not a game.json: refuse, never write");
    }

    #[test]
    fn engine_pin_refuses_what_a_clone_could_not_reproduce() {
        let dir = std::env::temp_dir().join(format!("re2_pin_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bare = dir.join("remote.git");
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let git = |cwd: &Path, args: &[&str]| {
            let mut cmd = std::process::Command::new("git");
            cmd.current_dir(cwd).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args);
            assert!(cmd.output().unwrap().status.success(), "git {args:?}");
        };
        git(&work, &["init", "-q", "-b", "main"]);
        std::fs::write(work.join("a.txt"), "1").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "one"]);
        let e = engine_pin(&work, false, None).unwrap_err();
        assert!(e.contains("origin"), "no remote: {e}");
        git(&dir, &["init", "-q", "--bare", "remote.git"]);
        git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        let e = engine_pin(&work, false, None).unwrap_err();
        assert!(e.contains("no remote branch"), "unpushed commit: {e}");
        git(&work, &["push", "-q", "-u", "origin", "main"]);
        let pin = engine_pin(&work, false, None).unwrap();
        assert_eq!((pin.sha.len(), pin.url.as_str()), (40, bare.to_str().unwrap()));
        std::fs::write(work.join("a.txt"), "2").unwrap();
        assert!(engine_pin(&work, false, None).unwrap_err().contains("uncommitted"), "a dirty tree is not what was built");
        assert!(engine_pin(&work, true, None).is_ok(), "--allow-dirty is the explicit way past that");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A verify report names an exact commit as its target; if the engine checkout is a branch that has since
    /// moved on, `game pin` must refuse rather than silently pin whatever HEAD happens to be now.
    #[test]
    fn engine_pin_refuses_a_checkout_that_moved_since_the_verified_sha() {
        let dir = std::env::temp_dir().join(format!("re2_pin_moved_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bare = dir.join("remote.git");
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let git = |cwd: &Path, args: &[&str]| {
            let mut cmd = std::process::Command::new("git");
            cmd.current_dir(cwd).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args);
            assert!(cmd.output().unwrap().status.success(), "git {args:?}");
        };
        git(&dir, &["init", "-q", "--bare", "remote.git"]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        std::fs::write(work.join("a.txt"), "1").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "one"]);
        git(&work, &["push", "-q", "-u", "origin", "main"]);
        let verified = engine_pin(&work, false, None).unwrap().sha;

        // The branch advances (exactly what a background `git pull`/another session's commit would do).
        std::fs::write(work.join("a.txt"), "2").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "two"]);
        git(&work, &["push", "-q", "origin", "main"]);

        let e = engine_pin(&work, false, Some(&verified)).unwrap_err();
        assert!(e.contains(&verified[..12]) && e.contains("moved"), "{e}");
        // Pinning without an expectation (today's existing behaviour) still works — this is additive, not a block.
        assert!(engine_pin(&work, false, None).is_ok());
        // Re-stating the checkout's new, actual HEAD as the expectation passes.
        let now = engine_pin(&work, false, None).unwrap().sha;
        assert!(engine_pin(&work, false, Some(&now)).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
