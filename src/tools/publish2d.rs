//! `red_engine2 publish GAME`: a verified 2D game to a playable URL, through stages that each say whether they passed.
//!
//! ```text
//! 1 validate  2 gameplay tests  3 WebAssembly build  4 static package  5 integrity check  6 browser smoke      <- everything up to here needs no credentials and no internet
//! 7 publication metadata  8 upload  9 remote smoke  10 URL                                                    <- only a backend that exists can do these
//! ```
//!
//! BUILD is separate from PUBLISH: stages 1-6 are `web build` + `web verify` and work offline; stages 7-10 only run once 1-6 passed, and a failure names the stage. The four
//! results are different claims and are reported separately: **BUILD SUCCESS** (the package exists and is consistent), **LOCAL BROWSER SUCCESS** (a real headless browser played it),
//! **UPLOAD SUCCESS** (the files are where the backend keeps them), **REMOTE PLAYABLE SUCCESS** (a real browser played the *deployed* copy at a non-loopback URL). The last is never
//! reported unless it happened, and a URL is never printed unless one exists.
//!
//! The site is one static library: `games/<id>/` is the stable URL (always the newest build), `games/<id>/builds/<build_id>/` is the immutable copy of every build, `games/<id>/game.json`
//! is the machine-readable record of that game and `catalog.json` lists them all (see `docs/PUBLISHING_2D.md` for the contract).

use super::{game2d, webpkg, webverify};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Schema of a game's record.
pub const META_SCHEMA: &str = "red2d-game-meta/1";
/// Schema of the catalog.
pub const CATALOG_SCHEMA: &str = "red2d-catalog/1";

/// The stages, in order.
pub const STAGES: &[&str] = &[
    "validate",
    "gameplay tests",
    "wasm build",
    "static package",
    "integrity check",
    "browser smoke",
    "publication metadata",
    "upload",
    "remote smoke",
    "url",
];

/// Where a build goes.
#[derive(Debug, Clone)]
pub enum Backend {
    /// A static site directory on this machine (the test backend, and the first half of any other).
    Local {
        /// The site root.
        site: PathBuf,
        /// Where that directory is served from, if somewhere serves it (then the URL is real and the remote smoke runs against it).
        base_url: Option<String>,
    },
    /// A checkout of the RedEngineGames repository (GitHub Pages): `webgames/<id>/...` is committed there, and its site generator publishes it.
    GithubPages {
        /// The checkout.
        repo: PathBuf,
        /// `git push` after committing. Without it nothing leaves this machine and no URL is claimed.
        push: bool,
        /// The site's address, like `https://kevstermcgee.github.io/RedEngineGames`.
        pages_url: String,
    },
}

/// What one stage did.
#[derive(Debug, Clone)]
pub struct StageResult {
    /// Its name (one of [`STAGES`]).
    pub stage: &'static str,
    /// Passed.
    pub ok: bool,
    /// Skipped because an earlier stage failed or the backend cannot do it.
    pub skipped: bool,
    /// One sentence (the failures, if it failed).
    pub detail: String,
    /// Seconds.
    pub secs: f32,
}

/// The four separate results.
#[derive(Debug, Clone, Default)]
pub struct States {
    /// The package exists and is internally consistent.
    pub build: bool,
    /// A real headless browser played it from a local server.
    pub local_browser: bool,
    /// The files are in the backend's storage.
    pub upload: bool,
    /// A real browser played the deployed copy at a non-loopback URL.
    pub remote_playable: bool,
}

/// The pipeline's result.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// Every stage, in order (stages after a failure are present and skipped).
    pub stages: Vec<StageResult>,
    /// The four claims.
    pub states: States,
    /// The URL players use, only if it exists and was reached.
    pub url: Option<String>,
    /// Where a human can look at the published files, when there is no URL.
    pub location: Option<String>,
    /// The package id of this build.
    pub build_id: Option<String>,
    /// What is left to do outside this tool, if anything.
    pub external_step: Option<String>,
}

impl Default for Outcome {
    fn default() -> Self {
        Outcome::new()
    }
}

impl Outcome {
    fn new() -> Outcome {
        Outcome {
            stages: STAGES.iter().map(|s| StageResult { stage: s, ok: false, skipped: true, detail: "not reached".into(), secs: 0.0 }).collect(),
            states: States::default(),
            url: None,
            location: None,
            build_id: None,
            external_step: None,
        }
    }
    fn set(&mut self, stage: &'static str, ok: bool, detail: impl Into<String>, secs: f32) {
        if let Some(s) = self.stages.iter_mut().find(|s| s.stage == stage) {
            *s = StageResult { stage, ok, skipped: false, detail: detail.into(), secs };
        }
    }
    /// The first stage that failed.
    pub fn failed_stage(&self) -> Option<&StageResult> {
        self.stages.iter().find(|s| !s.ok && !s.skipped)
    }
    /// Everything the pipeline was asked to do happened (a backend that cannot reach a URL still counts when it said so).
    pub fn ok(&self) -> bool {
        self.failed_stage().is_none()
    }
}

// ---- time ------------------------------------------------------------------------------------------------------------------------------------------

/// Seconds since the Unix epoch (`SOURCE_DATE_EPOCH` when set, so a build can be made reproducible).
pub fn now_epoch() -> u64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

/// `2026-10-05T17:03:09Z` for an epoch second.
pub fn iso8601(epoch: u64) -> String {
    let (days, rem) = ((epoch / 86_400) as i64, epoch % 86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

// ---- verification record ---------------------------------------------------------------------------------------------------------------------------

/// What a browser run says about a package (written by `web verify` next to its screenshots, read by `publish --package`).
#[derive(Debug, Clone)]
pub struct Verification {
    /// The package it ran against.
    pub package_id: String,
    /// All checks passed.
    pub ok: bool,
    /// `Chromium 153...`.
    pub browser: String,
    /// How many checks.
    pub checks: usize,
    /// The audio claims that were made, by name (`browser audio initialised`...), never listening.
    pub audio_claims: Vec<String>,
}

impl Verification {
    /// The JSON form stored beside the screenshots.
    pub fn to_json(&self) -> Value {
        json!({"schema": "red2d-browser-verification/1", "package_id": self.package_id, "ok": self.ok, "browser": self.browser, "checks": self.checks, "audio_claims": self.audio_claims, "human_listening_verified": false, "human_playtest": false})
    }
    /// Reads one back.
    pub fn from_json(v: &Value) -> Option<Verification> {
        Some(Verification {
            package_id: v["package_id"].as_str()?.to_string(),
            ok: v["ok"].as_bool()?,
            browser: v["browser"].as_str()?.to_string(),
            checks: v["checks"].as_u64()? as usize,
            audio_claims: v["audio_claims"].as_array()?.iter().filter_map(|a| a.as_str().map(str::to_string)).collect(),
        })
    }
}

// ---- metadata and the site -------------------------------------------------------------------------------------------------------------------------

/// The record of one game for the catalog: everything a library page needs without opening the package.
pub fn game_meta(manifest: &Value, v: &Verification, epoch: u64, previous_builds: &[Value]) -> Value {
    let g = &manifest["game"];
    let id = g["id"].as_str().unwrap_or("");
    let build_id = manifest["package_id"].as_str().unwrap_or("");
    let mut builds: Vec<Value> = previous_builds.iter().filter(|b| b["build_id"] != build_id).cloned().collect();
    builds.push(json!({"build_id": build_id, "built_at": iso8601(epoch), "game_revision": g["game_revision"], "engine_revision": manifest["engine"]["revision"], "path": format!("games/{id}/builds/{build_id}/")}));
    json!({
        "schema": META_SCHEMA,
        "id": id,
        "title": g["title"],
        "description": g["description"],
        "presentation": g["presentation"],
        "platforms": g["platforms"],
        "input": g["input"],
        "networking": g["networking"],
        "persistence": g["persistence"],
        "screen": g["screen"],
        "thumbnail": "thumbnail.png",
        "game_revision": g["game_revision"],
        "engine_revision": manifest["engine"]["revision"],
        "engine_dirty": manifest["engine"]["dirty"],
        "build_id": build_id,
        "build_timestamp": iso8601(epoch),
        "compatibility": {"requires": manifest["compat"]["requires"], "optional": manifest["compat"]["optional"], "networking": manifest["compat"]["networking"], "browsers_verified": [v.browser.clone()], "browsers_other": "untested"},
        "verification": {
            "native": {"scenarios": manifest["native"]["scenarios"].as_array().map_or(0, Vec::len), "passed": true},
            "browser": {"engine": v.browser, "checks": v.checks, "passed": v.ok, "package_id": v.package_id},
            "audio": {"claims": v.audio_claims, "human_listening_verified": false},
            "human_playtest": false,
        },
        "urls": {"stable": format!("games/{id}/"), "immutable": format!("games/{id}/builds/{build_id}/")},
        "builds": builds,
    })
}

fn copy_package(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(from.join("manifest.json")).map_err(|e| format!("manifest.json: {e}"))?)
        .map_err(|e| format!("manifest.json: {e}"))?;
    let mut files: Vec<String> =
        manifest["files"].as_array().map(|a| a.iter().filter_map(|f| f["path"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    files.push("manifest.json".into());
    for f in files {
        let dest = to.join(&f);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(from.join(&f), &dest).map_err(|e| format!("{} -> {}: {e}", from.join(&f).display(), dest.display()))?;
    }
    Ok(())
}

/// Writes a verified package into a static site (`games/<id>/builds/<build_id>/`, `games/<id>/` = newest, `games/<id>/game.json`, `catalog.json`, `index.html`).
/// A build directory that already exists must hold the same bytes (builds are immutable). Returns the game's record.
pub fn write_site(site: &Path, pkg: &Path, v: &Verification, epoch: u64) -> Result<Value, String> {
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(pkg.join("manifest.json")).map_err(|e| format!("manifest.json: {e}"))?)
        .map_err(|e| format!("manifest.json: {e}"))?;
    let id = manifest["game"]["id"].as_str().ok_or("manifest has no game id")?.to_string();
    let build_id = manifest["package_id"].as_str().ok_or("manifest has no package id")?.to_string();
    if v.package_id != build_id {
        return Err(format!("the browser verification is for package {} but this package is {build_id}: verify the package you publish", v.package_id));
    }
    let game_dir = site.join("games").join(&id);
    let build_dir = game_dir.join("builds").join(&build_id);
    if build_dir.join("manifest.json").is_file() {
        let old = std::fs::read(build_dir.join("manifest.json")).unwrap_or_default();
        let new = std::fs::read(pkg.join("manifest.json")).map_err(|e| e.to_string())?;
        if old != new {
            return Err(format!("{} already exists with a different manifest: published builds are immutable (the build id is a hash of the contents, so this should be impossible)", build_dir.display()));
        }
    } else {
        copy_package(pkg, &build_dir)?;
    }
    // The stable URL always serves the newest build.
    for entry in std::fs::read_dir(&game_dir).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name != "builds" && name != "game.json" {
            let p = entry.path();
            if p.is_dir() {
                std::fs::remove_dir_all(&p).map_err(|e| e.to_string())?;
            } else {
                std::fs::remove_file(&p).map_err(|e| e.to_string())?;
            }
        }
    }
    copy_package(pkg, &game_dir)?;
    let previous: Vec<Value> = std::fs::read_to_string(game_dir.join("game.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["builds"].as_array().cloned())
        .unwrap_or_default();
    let meta = game_meta(&manifest, v, epoch, &previous);
    std::fs::write(game_dir.join("game.json"), serde_json::to_string_pretty(&meta).unwrap_or_default() + "\n").map_err(|e| e.to_string())?;
    write_catalog(site)?;
    Ok(meta)
}

/// Rebuilds `catalog.json` and `index.html` from the games' own records.
pub fn write_catalog(site: &Path) -> Result<(), String> {
    let mut games: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(site.join("games")) {
        let mut dirs: Vec<_> = rd.flatten().collect();
        dirs.sort_by_key(|e| e.file_name());
        for e in dirs {
            if let Some(m) = std::fs::read_to_string(e.path().join("game.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
                games.push(json!({
                    "id": m["id"], "title": m["title"], "description": m["description"], "url": m["urls"]["stable"], "thumbnail": format!("{}thumbnail.png", m["urls"]["stable"].as_str().unwrap_or("")),
                    "presentation": m["presentation"], "platforms": m["platforms"], "input": m["input"], "networking": m["networking"], "persistence": m["persistence"],
                    "build_id": m["build_id"], "build_timestamp": m["build_timestamp"], "game_revision": m["game_revision"], "engine_revision": m["engine_revision"],
                    "compatibility": m["compatibility"], "verification": m["verification"], "record": format!("games/{}/game.json", m["id"].as_str().unwrap_or("")),
                }));
            }
        }
    }
    let catalog = json!({"schema": CATALOG_SCHEMA, "games": games});
    std::fs::write(site.join("catalog.json"), serde_json::to_string_pretty(&catalog).unwrap_or_default() + "\n").map_err(|e| e.to_string())?;
    let esc = |v: &Value| v.as_str().unwrap_or("").replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let cards: String = games
        .iter()
        .map(|g| {
            format!(
                "<a class=\"card\" href=\"{}\"><img src=\"{}\" alt=\"\"><h2>{}</h2><p>{}</p><small>{} · {} · {}</small></a>\n",
                esc(&g["url"]),
                esc(&g["thumbnail"]),
                esc(&g["title"]),
                esc(&g["description"]),
                g["presentation"].as_str().unwrap_or(""),
                g["input"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("+")).unwrap_or_default(),
                esc(&g["build_timestamp"])
            )
        })
        .collect();
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>RedEngine games</title>\
         <style>body{{margin:0;background:#0b0d12;color:#cfd6e6;font:16px/1.4 system-ui,sans-serif}}main{{max-width:60rem;margin:0 auto;padding:1.5rem}}\
         .grid{{display:grid;grid-template-columns:repeat(auto-fill,minmax(14rem,1fr));gap:1rem}}.card{{display:block;background:#141a26;border-radius:8px;padding:.75rem;color:inherit;text-decoration:none}}\
         .card img{{width:100%;image-rendering:pixelated;border-radius:4px}}.card h2{{margin:.5rem 0 .25rem;font-size:1.1rem}}.card p{{margin:0 0 .5rem;opacity:.85}}small{{opacity:.6}}</style></head>\
         <body><main><h1>Games</h1><div class=\"grid\">\n{cards}</div><p><small>Data: <a href=\"catalog.json\">catalog.json</a></small></p></main></body></html>\n"
    );
    std::fs::write(site.join("index.html"), html).map_err(|e| e.to_string())
}

// ---- backends ---------------------------------------------------------------------------------------------------------------------------------------

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let o = std::process::Command::new("git").arg("-C").arg(repo).args(args).output().map_err(|e| format!("git: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()))
    }
}

/// The GitHub Pages backend's upload: the verified package goes into `webgames/<id>/` of a RedEngineGames checkout (a site, `catalog.json` included, under `webgames/site`),
/// committed. Returns the path written and whether it was pushed.
pub fn github_upload(repo: &Path, pkg: &Path, v: &Verification, epoch: u64, push: bool) -> Result<(PathBuf, bool), String> {
    if !repo.join(".git").exists() {
        return Err(format!("{} is not a git checkout (clone https://github.com/kevstermcgee/RedEngineGames and pass its path with --repo)", repo.display()));
    }
    let site = repo.join("webgames");
    let meta = write_site(&site, pkg, v, epoch)?;
    git(repo, &["add", "webgames"])?;
    let id = meta["id"].as_str().unwrap_or("game");
    let msg = format!(
        "Browser game {id}: build {} (game {}, engine {})",
        meta["build_id"].as_str().unwrap_or(""),
        meta["game_revision"].as_str().unwrap_or(""),
        meta["engine_revision"].as_str().unwrap_or("")
    );
    let staged = git(repo, &["status", "--porcelain", "--", "webgames"])?;
    if !staged.is_empty() {
        git(repo, &["-c", "user.name=RedEngine publish", "-c", "user.email=publish@redengine.invalid", "commit", "-q", "-m", &msg])?;
    }
    if push {
        git(repo, &["push", "origin", "HEAD"])?;
    }
    Ok((site, push))
}

// ---- the pipeline ----------------------------------------------------------------------------------------------------------------------------------

/// Options for [`publish`].
pub struct Options {
    /// Where it goes.
    pub backend: Backend,
    /// Stop after the browser smoke and the metadata: nothing is uploaded.
    pub dry_run: bool,
    /// Where reports and screenshots go.
    pub out: PathBuf,
    /// A prebuilt player module.
    pub wasm: Option<PathBuf>,
}

fn is_loopback(url: &str) -> bool {
    let host = url.split("://").nth(1).unwrap_or(url).split(['/', ':']).next().unwrap_or("");
    host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host.is_empty()
}

fn rows_failed(rows: &[red2d::script::Row]) -> Vec<String> {
    rows.iter().filter(|r| !r.ok).map(|r| format!("{}: {}", r.name, r.detail)).collect()
}

/// Runs the pipeline for a game file. `progress` is called after each stage.
pub fn publish(game: &Path, opts: &Options, mut progress: impl FnMut(&StageResult)) -> Outcome {
    let mut out = Outcome::new();
    macro_rules! stage {
        ($name:expr, $body:expr) => {{
            let t = Instant::now();
            let r: Result<String, String> = $body;
            let secs = t.elapsed().as_secs_f32();
            match r {
                Ok(d) => out.set($name, true, d, secs),
                Err(e) => {
                    out.set($name, false, e, secs);
                    progress(out.stages.iter().find(|s| s.stage == $name).expect("stage"));
                    return out;
                }
            }
            progress(out.stages.iter().find(|s| s.stage == $name).expect("stage"));
        }};
    }
    // 1 validate
    stage!("validate", {
        let r = game2d::validate(game);
        if r.ok {
            Ok(r.text.lines().next().unwrap_or("").to_string())
        } else {
            Err(r.text)
        }
    });
    // 2 gameplay tests
    stage!("gameplay tests", {
        let r = game2d::verify(game, None);
        if r.ok {
            Ok(r.text.lines().find(|l| l.contains("passed,")).unwrap_or("passed").to_string())
        } else {
            Err(r.text.lines().filter(|l| l.starts_with("FAIL")).collect::<Vec<_>>().join("\n"))
        }
    });
    // 3 + 4 wasm build and static package (one call builds the module and writes the package)
    let pkg_dir = opts.out.join("package");
    let built = {
        let t = Instant::now();
        let wasm_resolved = match &opts.wasm {
            Some(w) => Ok((w.clone(), "given".to_string())),
            None => webpkg::player_wasm(),
        };
        match wasm_resolved {
            Err(e) => {
                out.set("wasm build", false, e, t.elapsed().as_secs_f32());
                progress(&out.stages[2]);
                return out;
            }
            Ok((w, how)) => {
                out.set("wasm build", true, format!("{how}: {}", w.display()), t.elapsed().as_secs_f32());
                progress(&out.stages[2]);
                let t = Instant::now();
                match webpkg::build(game, &pkg_dir, Some(&w)) {
                    Ok(b) => {
                        out.set(
                            "static package",
                            true,
                            format!("package {} in {}", b.manifest["package_id"].as_str().unwrap_or(""), pkg_dir.display()),
                            t.elapsed().as_secs_f32(),
                        );
                        progress(&out.stages[3]);
                        b
                    }
                    Err(e) => {
                        out.set("static package", false, e, t.elapsed().as_secs_f32());
                        progress(&out.stages[3]);
                        return out;
                    }
                }
            }
        }
    };
    let build_id = built.manifest["package_id"].as_str().unwrap_or("").to_string();
    out.build_id = Some(build_id.clone());
    // 5 integrity
    stage!("integrity check", {
        let bad = rows_failed(&webpkg::check(&built.dir));
        if bad.is_empty() {
            out.states.build = true;
            Ok("the package is intact and self-contained".to_string())
        } else {
            Err(bad.join("; "))
        }
    });
    // 6 browser smoke
    let shots = opts.out.join("browser");
    let verification = {
        let t = Instant::now();
        match webverify::verify(Some(&built.dir), None, &shots) {
            Err(e) => {
                out.set("browser smoke", false, e, t.elapsed().as_secs_f32());
                progress(&out.stages[5]);
                return out;
            }
            Ok(v) => {
                let bad = rows_failed(&v.rows);
                let ver = verification_of(&build_id, &v);
                std::fs::write(shots.join("verification.json"), serde_json::to_string_pretty(&ver.to_json()).unwrap_or_default()).ok();
                if bad.is_empty() {
                    out.states.local_browser = true;
                    out.set("browser smoke", true, format!("{} checks in {}", v.rows.len(), v.browser), t.elapsed().as_secs_f32());
                    progress(&out.stages[5]);
                    ver
                } else {
                    out.set("browser smoke", false, bad.join("; "), t.elapsed().as_secs_f32());
                    progress(&out.stages[5]);
                    return out;
                }
            }
        }
    };
    upload_and_confirm(&mut out, &mut progress, &built.dir, &verification, opts);
    out
}

/// Stages 7-10 for a package that has already passed the browser: used by [`publish`] and by `publish --package`.
pub fn upload_and_confirm(out: &mut Outcome, progress: &mut impl FnMut(&StageResult), pkg: &Path, ver: &Verification, opts: &Options) {
    let epoch = now_epoch();
    // 7 metadata
    let t = Instant::now();
    let manifest: Value = std::fs::read_to_string(pkg.join("manifest.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let meta = game_meta(&manifest, ver, epoch, &[]);
    out.set(
        "publication metadata",
        true,
        format!(
            "{} · {} · build {} · engine {}",
            meta["id"].as_str().unwrap_or(""),
            meta["build_timestamp"].as_str().unwrap_or(""),
            meta["build_id"].as_str().unwrap_or(""),
            meta["engine_revision"].as_str().unwrap_or("")
        ),
        t.elapsed().as_secs_f32(),
    );
    progress(&out.stages[6]);
    if opts.dry_run {
        out.set("upload", true, "dry run: nothing uploaded", 0.0);
        out.stages[7].skipped = true;
        out.external_step = Some("run again without --dry-run to upload".into());
        return;
    }
    // 8 upload
    let t = Instant::now();
    let uploaded = match &opts.backend {
        Backend::Local { site, .. } => write_site(site, pkg, ver, epoch).map(|m| (format!("site written to {}", site.display()), site.clone(), m)),
        Backend::GithubPages { repo, push, .. } => github_upload(repo, pkg, ver, epoch, *push).and_then(|(site, pushed)| {
            write_site_meta_only(&site).map(|m| {
                (format!("committed in {}{}", repo.display(), if pushed { " and pushed" } else { " (NOT pushed: nothing has left this machine)" }), site, m)
            })
        }),
    };
    let (detail, location, meta) = match uploaded {
        Ok(x) => x,
        Err(e) => {
            out.set("upload", false, e, t.elapsed().as_secs_f32());
            progress(&out.stages[7]);
            return;
        }
    };
    out.location = Some(location.display().to_string());
    let id = meta["id"].as_str().unwrap_or("").to_string();
    let pushed = matches!(&opts.backend, Backend::GithubPages { push: true, .. }) || matches!(&opts.backend, Backend::Local { .. });
    out.states.upload = pushed;
    out.set("upload", true, detail, t.elapsed().as_secs_f32());
    progress(&out.stages[7]);
    // 9 remote smoke + 10 url
    let public_base = match &opts.backend {
        Backend::Local { base_url, .. } => base_url.clone(),
        Backend::GithubPages { pages_url, push: true, .. } => Some(format!("{}/play", pages_url.trim_end_matches('/'))),
        Backend::GithubPages { push: false, .. } => None,
    };
    let Some(base) = public_base else {
        // No place serves the files: check them over HTTP from here (a loopback server on the site directory), and say that is all this is.
        if let Backend::Local { site, .. } = &opts.backend {
            let t = Instant::now();
            let r = loopback_check(site, &id, ver, &opts.out);
            match r {
                Ok(msg) => {
                    out.set("remote smoke", true, format!("{msg} (served from this machine's loopback; this is not a remote check)"), t.elapsed().as_secs_f32())
                }
                Err(e) => out.set("remote smoke", false, e, t.elapsed().as_secs_f32()),
            }
            progress(&out.stages[8]);
        } else {
            out.set("remote smoke", true, "not run: nothing was pushed, so there is no deployed copy to check", 0.0);
            out.stages[8].skipped = true;
        }
        out.set("url", true, "unavailable: no remote backend published this game", 0.0);
        out.stages[9].skipped = true;
        out.external_step = Some(match &opts.backend {
            Backend::Local { site, .. } => format!(
                "serve {} from a static host (or `red_engine2 web serve {}`) and pass --base-url, or use --backend github-pages",
                site.display(),
                site.display()
            ),
            Backend::GithubPages { repo, .. } => {
                format!("review and push: git -C {} push origin HEAD (or rerun with --push); the site workflow then deploys it", repo.display())
            }
        });
        return;
    };
    let url = format!("{}/games/{id}/", base.trim_end_matches('/'));
    let t = Instant::now();
    match remote_smoke(&url, ver, &opts.out, matches!(&opts.backend, Backend::GithubPages { .. })) {
        Ok(msg) => {
            out.set("remote smoke", true, msg, t.elapsed().as_secs_f32());
            progress(&out.stages[8]);
            out.url = Some(url.clone());
            out.states.remote_playable = !is_loopback(&url);
            out.set("url", true, url, 0.0);
            progress(&out.stages[9]);
        }
        Err(e) => {
            out.set("remote smoke", false, e, t.elapsed().as_secs_f32());
            progress(&out.stages[8]);
        }
    }
}

fn write_site_meta_only(site: &Path) -> Result<Value, String> {
    // Read back the record `write_site` just wrote (the most recently modified game).
    let mut newest: Option<(std::time::SystemTime, Value)> = None;
    if let Ok(rd) = std::fs::read_dir(site.join("games")) {
        for e in rd.flatten() {
            let p = e.path().join("game.json");
            if let (Ok(md), Ok(t)) = (std::fs::metadata(&p), std::fs::read_to_string(&p)) {
                if let (Ok(m), Ok(v)) = (md.modified(), serde_json::from_str::<Value>(&t)) {
                    if newest.as_ref().is_none_or(|(n, _)| m >= *n) {
                        newest = Some((m, v));
                    }
                }
            }
        }
    }
    newest.map(|(_, v)| v).ok_or_else(|| "the site has no game record".to_string())
}

fn verification_of(build_id: &str, v: &webverify::Verified) -> Verification {
    Verification {
        package_id: build_id.to_string(),
        ok: v.rows.iter().all(|r| r.ok),
        browser: v.browser.clone(),
        checks: v.rows.len(),
        audio_claims: v.rows.iter().filter(|r| r.claim == "browser-audio" && r.ok).map(|r| r.name.clone()).collect(),
    }
}

/// Serves the site directory on loopback and runs the browser against `games/<id>/` there.
fn loopback_check(site: &Path, id: &str, ver: &Verification, out: &Path) -> Result<String, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("cannot start a loopback server: {e}"))?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    drop(listener);
    let site = site.to_path_buf();
    std::thread::spawn(move || {
        let _ = webpkg::serve(&site, port, |_| {});
    });
    for _ in 0..50 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    remote_smoke(&format!("http://127.0.0.1:{port}/games/{id}/"), ver, out, false)
}

/// Waits (when the site takes time to deploy) for the URL to serve the build, then runs the browser against it.
fn remote_smoke(url: &str, ver: &Verification, out: &Path, wait_for_deploy: bool) -> Result<String, String> {
    let manifest_url = format!("{}manifest.json", url);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(if wait_for_deploy { 600 } else { 10 });
    let mut last: String;
    loop {
        match http_get(&manifest_url) {
            Ok(body) => match serde_json::from_str::<Value>(&body) {
                Ok(m) if m["package_id"].as_str() == Some(ver.package_id.as_str()) => break,
                Ok(m) => last = format!("serves package {} (waiting for {})", m["package_id"], ver.package_id),
                Err(_) => last = "answered, but manifest.json is not JSON".into(),
            },
            Err(e) => last = e,
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("{manifest_url} never served build {}: {last}", ver.package_id));
        }
        std::thread::sleep(std::time::Duration::from_secs(if wait_for_deploy { 10 } else { 1 }));
    }
    let v = webverify::verify(None, Some(url.trim_end_matches('/')), &out.join("remote"))?;
    let bad = rows_failed(&v.rows);
    if bad.is_empty() {
        Ok(format!("{} checks passed in {} against {url}", v.rows.len(), v.browser))
    } else {
        Err(format!("the deployed copy fails in the browser: {}", bad.join("; ")))
    }
}

/// A minimal HTTP/1.1 GET (http only; the https case goes through `curl`, which every supported machine has).
pub fn http_get(url: &str) -> Result<String, String> {
    if url.starts_with("https://") {
        let o = std::process::Command::new("curl").args(["-fsSL", "--max-time", "20", url]).output().map_err(|e| format!("curl: {e}"))?;
        return if o.status.success() {
            Ok(String::from_utf8_lossy(&o.stdout).to_string())
        } else {
            Err(format!("curl {url}: {}", String::from_utf8_lossy(&o.stderr).trim()))
        };
    }
    use std::io::{Read, Write};
    let rest = url.strip_prefix("http://").ok_or_else(|| format!("{url}: not an http(s) URL"))?;
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, "/".into()));
    let mut s = std::net::TcpStream::connect(host).map_err(|e| format!("{host}: {e}"))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(10))).ok();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").ok_or("malformed response")?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!("{url}: {}", head.lines().next().unwrap_or("")));
    }
    Ok(body.to_string())
}

/// The report as text: one line per stage, then the four states, the URL and any external step.
pub fn render(o: &Outcome) -> String {
    let mut t = String::new();
    for (i, s) in o.stages.iter().enumerate() {
        let mark = if s.skipped {
            "skip"
        } else if s.ok {
            "ok  "
        } else {
            "FAIL"
        };
        t.push_str(&format!("{:>2}. {mark} {:<20} {}\n", i + 1, s.stage, s.detail.lines().next().unwrap_or("")));
    }
    if let Some(f) = o.failed_stage() {
        t.push_str(&format!("\nFAILED at stage {} ({}):\n{}\n", STAGES.iter().position(|s| *s == f.stage).map_or(0, |i| i + 1), f.stage, f.detail));
    }
    let st = &o.states;
    let yn = |b: bool| if b { "yes" } else { "no" };
    t.push_str(&format!(
        "\nBUILD SUCCESS: {}   LOCAL BROWSER SUCCESS: {}   UPLOAD SUCCESS: {}   REMOTE PLAYABLE SUCCESS: {}\n",
        yn(st.build),
        yn(st.local_browser),
        yn(st.upload),
        yn(st.remote_playable)
    ));
    match (&o.url, &o.location) {
        (Some(u), _) => t.push_str(&format!("URL: {u}{}\n", if st.remote_playable { "" } else { "   (loopback only: not reachable by anyone else)" })),
        (None, Some(l)) => t.push_str(&format!("URL: none. PUBLICATION UNAVAILABLE: the files are at {l}\n")),
        (None, None) => t.push_str("URL: none.\n"),
    }
    if let Some(e) = &o.external_step {
        t.push_str(&format!("Remaining external step: {e}\n"));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg_with(dir: &Path, id: &str, build: &str, desc: &str) {
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), "<html></html>").unwrap();
        std::fs::write(dir.join("thumbnail.png"), b"png").unwrap();
        let manifest = json!({
            "schema": webpkg::SCHEMA, "package_id": build, "entry": "index.html",
            "game": {"id": id, "title": "T", "description": desc, "game_revision": "abc", "presentation": "2d", "platforms": ["web"], "networking": "offline", "input": ["keyboard"], "persistence": [], "screen": {"width": 320, "height": 180, "scale": "fit"}},
            "engine": {"revision": "deadbeef", "dirty": false},
            "compat": {"requires": ["WebAssembly"], "optional": [], "networking": "none"},
            "native": {"scenarios": [{"name": "a"}]},
            "files": [{"path": "index.html", "bytes": 13, "sha256": "x"}, {"path": "thumbnail.png", "bytes": 3, "sha256": "y"}],
        });
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    }

    fn ver(build: &str) -> Verification {
        Verification { package_id: build.into(), ok: true, browser: "Chromium 1".into(), checks: 38, audio_claims: vec!["browser audio initialised".into()] }
    }

    #[test]
    fn iso8601_is_right_across_a_leap_day_and_the_epoch() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso8601(1_790_000_000), "2026-09-21T14:13:20Z");
    }

    #[test]
    fn a_site_keeps_every_build_immutable_and_serves_the_newest_at_the_stable_path() {
        let root = std::env::temp_dir().join(format!("re2_site_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (p1, p2, site) = (root.join("p1"), root.join("p2"), root.join("site"));
        pkg_with(&p1, "demo", "1111", "first");
        pkg_with(&p2, "demo", "2222", "second");
        let m1 = write_site(&site, &p1, &ver("1111"), 1_000).unwrap();
        assert_eq!(m1["urls"]["stable"], "games/demo/");
        let m2 = write_site(&site, &p2, &ver("2222"), 2_000).unwrap();
        // Both builds are kept, the stable path is the newest, and the record lists both.
        assert!(site.join("games/demo/builds/1111/manifest.json").is_file() && site.join("games/demo/builds/2222/manifest.json").is_file());
        assert!(std::fs::read_to_string(site.join("games/demo/manifest.json")).unwrap().contains("2222"));
        assert_eq!(m2["builds"].as_array().unwrap().len(), 2);
        assert_eq!(m2["builds"][0]["build_id"], "1111");
        // Re-publishing a build is idempotent and does not duplicate it in the record.
        let m3 = write_site(&site, &p2, &ver("2222"), 3_000).unwrap();
        assert_eq!(m3["builds"].as_array().unwrap().len(), 2);
        // The catalog and the page list the game, with the record's fields.
        let c: Value = serde_json::from_str(&std::fs::read_to_string(site.join("catalog.json")).unwrap()).unwrap();
        assert_eq!(c["schema"], CATALOG_SCHEMA);
        assert_eq!(c["games"][0]["id"], "demo");
        assert_eq!(c["games"][0]["build_id"], "2222");
        assert_eq!(c["games"][0]["compatibility"]["browsers_verified"][0], "Chromium 1");
        assert!(std::fs::read_to_string(site.join("index.html")).unwrap().contains("href=\"games/demo/\""));
        // A different package under an existing build id is refused: builds are immutable.
        pkg_with(&p1, "demo", "2222", "tampered");
        let e = write_site(&site, &p1, &ver("2222"), 4_000).unwrap_err();
        assert!(e.contains("immutable"), "{e}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_verification_for_another_package_is_refused() {
        let root = std::env::temp_dir().join(format!("re2_site2_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        pkg_with(&root.join("p"), "demo", "3333", "x");
        let e = write_site(&root.join("site"), &root.join("p"), &ver("9999"), 1).unwrap_err();
        assert!(e.contains("verification is for package 9999") && e.contains("3333"), "{e}");
        assert!(!root.join("site/games").exists(), "nothing was written");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_record_carries_what_a_catalog_needs_and_claims_no_listening() {
        let manifest = json!({"package_id": "abcd", "game": {"id": "g", "title": "G", "description": "d", "game_revision": "r", "presentation": "2d", "platforms": ["web"], "networking": "offline", "input": ["mouse"], "persistence": ["settings"], "screen": {}},
            "engine": {"revision": "e", "dirty": true}, "compat": {"requires": [], "optional": [], "networking": "n"}, "native": {"scenarios": [1, 2, 3]}});
        let r = game_meta(&manifest, &ver("abcd"), 0, &[]);
        for k in [
            "id",
            "title",
            "description",
            "engine_revision",
            "game_revision",
            "presentation",
            "platforms",
            "input",
            "networking",
            "persistence",
            "thumbnail",
            "build_timestamp",
            "compatibility",
            "urls",
            "builds",
            "verification",
        ] {
            assert!(r.get(k).is_some(), "the record lacks `{k}`");
        }
        assert_eq!(r["verification"]["audio"]["human_listening_verified"], false);
        assert_eq!(r["verification"]["human_playtest"], false);
        assert_eq!(r["verification"]["native"]["scenarios"], 3);
        assert_eq!(r["engine_dirty"], true);
        assert!(Verification::from_json(&ver("abcd").to_json()).is_some_and(|v| v.package_id == "abcd" && v.ok));
    }

    #[test]
    fn the_report_never_claims_more_than_happened() {
        let mut o = Outcome::new();
        o.states.build = true;
        o.states.local_browser = true;
        o.set("validate", true, "ok", 0.0);
        o.location = Some("/tmp/site".into());
        o.external_step = Some("push".into());
        let t = render(&o);
        assert!(
            t.contains("BUILD SUCCESS: yes")
                && t.contains("LOCAL BROWSER SUCCESS: yes")
                && t.contains("UPLOAD SUCCESS: no")
                && t.contains("REMOTE PLAYABLE SUCCESS: no"),
            "{t}"
        );
        assert!(t.contains("URL: none. PUBLICATION UNAVAILABLE") && t.contains("Remaining external step: push"), "{t}");
        assert!(!t.contains("http"), "no URL is invented");
        o.set("browser smoke", false, "boom", 0.0);
        let t = render(&o);
        assert!(t.contains("FAILED at stage 6 (browser smoke)") && t.contains("boom"), "{t}");
        assert!(is_loopback("http://127.0.0.1:8080/games/x/") && !is_loopback("https://example.github.io/x/"));
    }
}
