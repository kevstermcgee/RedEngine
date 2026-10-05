//! `red_engine2 web build | check | serve`: a 2D game as a static, deterministic, self-checking web package.
//!
//! ```text
//! index.html          the page (title, description, start screen), no external references
//! runtime.js          the browser side: loop, canvas, input, localStorage, Web Audio (no dependencies)
//! audio-worker.js     renders the music loop off the main thread (same module, same samples)
//! game.wasm           the engine player: the same for every game, imports nothing (no env, files, clock or network)
//! assets/game.json    the game, verbatim
//! thumbnail.png       a frame from the game's own playthrough
//! manifest.json       what is here (SHA-256 of every file), what the game declares, the engine and game revisions, the native hash of every scenario
//! ```
//!
//! The package is deterministic: no timestamp, no absolute path, files in a fixed order, so the same game and the same engine build give the same bytes and the same `package_id`.
//! [`check`] re-derives all of that from the directory alone (hashes, declared assets, no absolute paths, the wasm's imports and exports, the game parsing to the revision the
//! manifest names), so a package can be validated by someone who has neither the source checkout nor the game's source.
//!
//! "Built" and "valid" say nothing about a browser: `web verify` (`webverify.rs`) loads the package in a real headless browser. Compiling to WebAssembly is not browser support.

use crate::crypto::{hex, sha256};
use crate::tools::game2d;
use red2d::render;
use red2d::script::{self, Row};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The manifest schema name.
pub const SCHEMA: &str = "red2d-web-package/1";
const RUNTIME_JS: &str = include_str!("../../crates/red2d/web/runtime.js");
const AUDIO_WORKER_JS: &str = include_str!("../../crates/red2d/web/audio-worker.js");
const SERVICE_WORKER_JS: &str = include_str!("../../crates/red2d/web/sw.js");
const INDEX_HTML: &str = include_str!("../../crates/red2d/web/index.html");

/// Functions the runtime calls on the module; the module must export every one (and the runtime must use no other).
pub const ABI: &[&str] = &[
    "alloc",
    "dealloc",
    "init",
    "error",
    "step",
    "render",
    "view_w",
    "view_h",
    "frame_ptr",
    "frame_len",
    "key",
    "action",
    "pointer",
    "click",
    "window",
    "layout_x",
    "layout_y",
    "layout_w",
    "layout_h",
    "to_view",
    "view_x",
    "view_y",
    "snapshot",
    "save_take",
    "save_load",
    "scenarios",
    "sounds_take",
    "sound_pcm",
    "music_pcm",
    "music_on",
    "sample_rate",
    "out_ptr",
    "out_len",
];

/// What was built.
pub struct Built {
    /// The package directory.
    pub dir: PathBuf,
    /// The manifest as written.
    pub manifest: Value,
}

/// The engine checkout that holds `crates/red2d` (for building the player and templates): `RED2D_ENGINE_ROOT`, the checkout this binary was built from, or one above the cwd/exe.
pub fn engine_root() -> Result<PathBuf, String> {
    let ok = |p: &Path| p.join("crates/red2d/Cargo.toml").is_file();
    let mut tried = Vec::new();
    if let Ok(r) = std::env::var("RED2D_ENGINE_ROOT") {
        let p = PathBuf::from(r);
        if ok(&p) {
            return Ok(p);
        }
        tried.push(p);
    }
    let built_from = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if ok(&built_from) {
        return Ok(built_from);
    }
    tried.push(built_from);
    let mut starts = vec![std::env::current_dir().unwrap_or_default()];
    if let Ok(e) = std::env::current_exe() {
        starts.push(e);
    }
    for s in starts {
        for a in s.ancestors() {
            if ok(a) {
                return Ok(a.to_path_buf());
            }
        }
    }
    Err(format!(
        "cannot find the engine checkout (a directory with crates/red2d): set RED2D_ENGINE_ROOT, or set RED2D_WASM to a prebuilt game player (tried {})",
        tried.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    ))
}

fn cargo_path() -> PathBuf {
    if Command::new("cargo").arg("--version").output().is_ok() {
        return PathBuf::from("cargo");
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = Path::new(&home).join(".cargo/bin/cargo");
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from("cargo")
}

/// The game player module: `RED2D_WASM` if set, else built from `crates/red2d` with `--profile web`.
pub fn player_wasm() -> Result<(PathBuf, String), String> {
    if let Ok(p) = std::env::var("RED2D_WASM") {
        let p = PathBuf::from(p);
        return if p.is_file() { Ok((p, "RED2D_WASM".into())) } else { Err(format!("RED2D_WASM={} is not a file", p.display())) };
    }
    let root = engine_root()?;
    let target_dir = std::env::var("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|_| root.join("target")).join("web2d");
    let cargo_home = std::env::var("CARGO_HOME").ok().or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.cargo"))).unwrap_or_default();
    let mut flags = format!("--remap-path-prefix={}=/red2d", root.display());
    if !cargo_home.is_empty() {
        flags.push_str(&format!(" --remap-path-prefix={cargo_home}=/cargo"));
    }
    let out = Command::new(cargo_path())
        .current_dir(&root)
        .env("RUSTFLAGS", flags)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .args(["build", "--profile", "web", "--target", "wasm32-unknown-unknown", "-p", "red2d", "--target-dir"])
        .arg(&target_dir)
        .output()
        .map_err(|e| format!("could not run cargo: {e} (install Rust from rustup.rs, or set RED2D_WASM to a prebuilt player)"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let hint = if err.contains("can't find crate for `core`") || err.contains("can't find crate for core") || err.contains("target may not be installed") {
            "\nfix: `rustup target add wasm32-unknown-unknown`"
        } else {
            ""
        };
        let tail: Vec<&str> = err.lines().rev().take(25).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!("building the WebAssembly player failed:\n{}{hint}", tail.join("\n")));
    }
    let wasm = target_dir.join("wasm32-unknown-unknown/web/red2d.wasm");
    if !wasm.is_file() {
        return Err(format!("cargo succeeded but {} is missing", wasm.display()));
    }
    Ok((wasm, "built from crates/red2d (--profile web)".into()))
}

// ---- reading a module ------------------------------------------------------------------------------------------------------------------------------

/// What a WebAssembly module imports and exports.
#[derive(Debug, Clone, Default)]
pub struct WasmInfo {
    /// Imported items as `module.name`.
    pub imports: Vec<String>,
    /// Exported names.
    pub exports: Vec<String>,
}

fn leb(b: &[u8], i: &mut usize) -> Result<usize, String> {
    let (mut v, mut shift) = (0usize, 0);
    loop {
        let byte = *b.get(*i).ok_or("truncated module")?;
        *i += 1;
        v |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift > 35 {
            return Err("bad LEB128".into());
        }
    }
}

fn name(b: &[u8], i: &mut usize) -> Result<String, String> {
    let n = leb(b, i)?;
    let s = b.get(*i..*i + n).ok_or("truncated name")?;
    *i += n;
    Ok(String::from_utf8_lossy(s).to_string())
}

/// Reads a module's import and export sections.
pub fn wasm_info(b: &[u8]) -> Result<WasmInfo, String> {
    if b.len() < 8 || &b[0..4] != b"\0asm" {
        return Err("not a WebAssembly module (missing \\0asm header)".into());
    }
    let mut info = WasmInfo::default();
    let mut i = 8;
    while i < b.len() {
        let id = b[i];
        i += 1;
        let size = leb(b, &mut i)?;
        let end = i + size;
        if end > b.len() {
            return Err("truncated section".into());
        }
        match id {
            2 => {
                let mut j = i;
                let n = leb(b, &mut j)?;
                for _ in 0..n {
                    let m = name(b, &mut j)?;
                    let f = name(b, &mut j)?;
                    info.imports.push(format!("{m}.{f}"));
                    let kind = *b.get(j).ok_or("truncated import")?;
                    j += 1;
                    match kind {
                        0 => {
                            leb(b, &mut j)?;
                        }
                        1 => {
                            j += 1;
                            let flag = *b.get(j).ok_or("truncated")?;
                            j += 1;
                            leb(b, &mut j)?;
                            if flag & 1 == 1 {
                                leb(b, &mut j)?;
                            }
                        }
                        2 => {
                            let flag = *b.get(j).ok_or("truncated")?;
                            j += 1;
                            leb(b, &mut j)?;
                            if flag & 1 == 1 {
                                leb(b, &mut j)?;
                            }
                        }
                        _ => {
                            j += 2;
                        }
                    }
                }
            }
            7 => {
                let mut j = i;
                let n = leb(b, &mut j)?;
                for _ in 0..n {
                    info.exports.push(name(b, &mut j)?);
                    j += 1;
                    leb(b, &mut j)?;
                }
            }
            _ => {}
        }
        i = end;
    }
    Ok(info)
}

// ---- building ------------------------------------------------------------------------------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn input_blurb(d: &red2d::game::GameDef) -> String {
    use red2d::caps::Input;
    let mut parts = Vec::new();
    for i in &d.caps.input {
        match i {
            Input::Keyboard => parts.push("keyboard (arrows or WASD, Space, Enter)"),
            Input::Mouse => parts.push("mouse"),
            Input::Gamepad => parts.push("gamepad"),
            Input::Touch => {}
        }
    }
    if parts.is_empty() {
        "Play with the keyboard or mouse.".to_string()
    } else {
        format!("Play with {}.", parts.join(", "))
    }
}

/// What the start card says on a phone.
fn touch_blurb(d: &red2d::game::GameDef) -> String {
    use red2d::caps::Input;
    if !d.caps.input.contains(&Input::Touch) {
        return "This game is made for a keyboard or mouse and does not have touch controls.".to_string();
    }
    if d.controls.visible() {
        "Play with the on-screen controls below the game.".to_string()
    } else {
        "Tap and drag the picture to play.".to_string()
    }
}

/// The manifest's record of the phone controller.
fn controls_json(d: &red2d::game::GameDef) -> Value {
    let c = &d.controls;
    json!({
        "layout": c.layout.name(),
        "visible": c.visible() && d.caps.input.contains(&red2d::caps::Input::Touch),
        "declared": c.declared,
        "buttons": c.buttons.iter().map(|b| json!({"id": b.id, "label": b.label, "action": b.action})).collect::<Vec<_>>(),
        "pause": c.pause,
    })
}

fn engine_revision(root: &Path) -> (String, bool) {
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let rev = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = git(&["status", "--porcelain", "--", "crates/red2d", "src", "Cargo.toml", "Cargo.lock"]).is_some_and(|s| !s.is_empty());
    (rev, dirty)
}

/// An app icon: the middle square of a frame of the game, scaled up in whole-number-free nearest steps to 80% of a `size` square on the game's background colour (the
/// 10% margin keeps it inside the safe zone of a maskable icon). Integer arithmetic only, so it is byte-identical on every machine.
fn icon_png(frame: &render::Frame, size: u32, bg: [u8; 4]) -> Result<Vec<u8>, String> {
    let side = frame.w.min(frame.h);
    let (ox, oy) = ((frame.w - side) / 2, (frame.h - side) / 2);
    let inner = size * 8 / 10;
    let margin = (size - inner) / 2;
    let mut img = image::RgbaImage::from_pixel(size, size, image::Rgba(bg));
    for y in 0..inner {
        for x in 0..inner {
            let (sx, sy) = (ox + (u64::from(x) * u64::from(side) / u64::from(inner)) as u32, oy + (u64::from(y) * u64::from(side) / u64::from(inner)) as u32);
            let i = ((sy * frame.w + sx) * 4) as usize;
            img.put_pixel(margin + x, margin + y, image::Rgba([frame.rgba[i], frame.rgba[i + 1], frame.rgba[i + 2], 255]));
        }
    }
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| format!("icon: {e}"))?;
    Ok(png)
}

fn hex_color(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// The web app manifest that makes the page installable.
fn web_app_manifest(d: &red2d::game::GameDef) -> Value {
    let short: String = d.title.chars().take(12).collect();
    json!({
        "name": d.title,
        "short_name": short,
        "description": d.description,
        "id": "./",
        "start_url": "./index.html",
        "scope": "./",
        "display": "standalone",
        "orientation": "any",
        "background_color": hex_color(d.view.background),
        "theme_color": "#0b0d12",
        "categories": ["games"],
        "icons": [
            {"src": "icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any maskable"},
            {"src": "icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any maskable"},
        ],
    })
}

/// Builds the package for a game into `out` (replacing a previous package there, never any other directory).
pub fn build(game: &Path, out: &Path, wasm_override: Option<&Path>) -> Result<Built, String> {
    let (def, text) = game2d::load(game)?;
    if !def.caps.platforms.contains(&red2d::caps::Platform::Web) {
        return Err(format!(
            "{}: this game does not declare the `web` platform (platforms: {}): add \"web\" to capabilities.platforms to build it for the browser",
            game.display(),
            def.caps.platforms.iter().map(|p| p.name()).collect::<Vec<_>>().join(", ")
        ));
    }
    let (wasm_path, wasm_how) = match wasm_override {
        Some(p) => (p.to_path_buf(), "given".to_string()),
        None => player_wasm()?,
    };
    let wasm = std::fs::read(&wasm_path).map_err(|e| format!("{}: {e}", wasm_path.display()))?;
    let root = engine_root().ok();
    let (rev, dirty) = root.as_deref().map(engine_revision).unwrap_or(("unknown".into(), false));

    // The game's own native playthroughs: the hashes the browser must reproduce.
    let mut scenarios = Vec::new();
    for sc in &def.scenarios {
        let (r, _) = script::run_scenario(&def, sc, None);
        scenarios.push(json!({"name": sc.name, "ticks": r.ticks, "hash": r.hash, "smoke": sc.smoke}));
    }
    let sim0 = red2d::sim::Sim::new(def.clone(), 1);
    let f0 = render::render(&sim0);
    let initial = json!({"state_hash": sim0.hash_hex(), "frame": f0.stats(def.view.background).hash, "seed": 1});

    // Thumbnail: 40% into the smoke scenario (else the first, else the opening scene).
    let sc = def.scenarios.iter().find(|s| s.smoke).or(def.scenarios.first());
    let thumb_sim = match sc {
        Some(s) => {
            let total = script::run_scenario(&def, s, None).0.ticks;
            script::run_scenario(&def, s, Some(total * 2 / 5)).1
        }
        None => sim0,
    };
    let tf = render::render(&thumb_sim);
    let mut png = Vec::new();
    {
        let img = image::RgbaImage::from_raw(tf.w, tf.h, tf.rgba).ok_or("internal: frame size")?;
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| format!("thumbnail: {e}"))?;
    }

    let html = INDEX_HTML
        .replace("{{TITLE}}", &esc(&def.title))
        .replace("{{DESCRIPTION}}", &esc(&def.description))
        .replace("{{ID}}", &esc(&def.id))
        .replace("{{WIDTH}}", &def.view.width.to_string())
        .replace("{{HEIGHT}}", &def.view.height.to_string())
        .replace("{{INPUT}}", &esc(&input_blurb(&def)))
        .replace("{{TOUCH}}", &esc(&touch_blurb(&def)));
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("assets/game.json".into(), text.clone().into_bytes()),
        ("audio-worker.js".into(), AUDIO_WORKER_JS.as_bytes().to_vec()),
        ("game.wasm".into(), wasm),
        ("index.html".into(), html.into_bytes()),
        ("runtime.js".into(), RUNTIME_JS.as_bytes().to_vec()),
        ("thumbnail.png".into(), png),
        ("icon-192.png".into(), icon_png(&render::render(&thumb_sim), 192, def.view.background)?),
        ("icon-512.png".into(), icon_png(&render::render(&thumb_sim), 512, def.view.background)?),
        ("manifest.webmanifest".into(), (serde_json::to_string_pretty(&web_app_manifest(&def)).unwrap_or_default() + "\n").into_bytes()),
        ("sw.js".into(), SERVICE_WORKER_JS.as_bytes().to_vec()),
    ];
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let listing: Vec<Value> = files.iter().map(|(p, b)| json!({"path": p, "bytes": b.len(), "sha256": hex(&sha256(b))})).collect();
    let package_id = {
        let mut all = Vec::new();
        for f in &listing {
            all.extend_from_slice(f["path"].as_str().unwrap_or("").as_bytes());
            all.extend_from_slice(f["sha256"].as_str().unwrap_or("").as_bytes());
        }
        hex(&sha256(&all))[..16].to_string()
    };
    let c = &def.caps;
    let names = |v: Vec<&str>| v.into_iter().map(String::from).collect::<Vec<_>>();
    let manifest = json!({
        "schema": SCHEMA,
        "package_id": package_id,
        "entry": "index.html",
        "game": {
            "id": def.id, "title": def.title, "description": def.description, "game_revision": def.rev,
            "presentation": c.presentation.name(),
            "platforms": names(c.platforms.iter().map(|p| p.name()).collect()),
            "networking": c.networking.name(),
            "input": names(c.input.iter().map(|i| i.name()).collect()),
            "persistence": names(c.persistence.iter().map(|p| p.name()).collect()),
            "controls": controls_json(&def),
            "distribution": names(c.distribution.iter().map(|d| d.name()).collect()),
            "install": {"web_app": true, "offline": true, "icons": ["icon-192.png", "icon-512.png"], "native_installer": false},
            "screen": {"width": def.view.width, "height": def.view.height, "scale": if def.view.scale == red2d::game::Scale::Integer { "integer" } else { "fit" }},
        },
        "engine": {"revision": rev, "dirty": dirty, "player": "red2d", "player_version": env!("CARGO_PKG_VERSION"), "player_build": wasm_how},
        "compat": {"requires": ["WebAssembly", "Canvas 2D"], "optional": ["localStorage (saves)", "Web Audio (sound)", "Gamepad API"], "networking": "none: the game never touches the network after loading"},
        "native": {"initial": initial, "scenarios": scenarios},
        "browser_checks": def.browser.iter().map(|b| json!({"name": b.name, "keys": b.keys, "click": b.click, "ms": b.ms, "changes": b.changes, "persists": b.persists})).collect::<Vec<_>>(),
        "files": listing,
    });

    // Replace only a previous package (or an empty/missing directory).
    if out.exists() {
        let is_pkg =
            std::fs::read_to_string(out.join("manifest.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).is_some_and(|m| m["schema"] == SCHEMA);
        let empty = std::fs::read_dir(out).map(|mut d| d.next().is_none()).unwrap_or(false);
        if !is_pkg && !empty {
            return Err(format!("{} exists and is not a web package: refusing to replace it (choose another --out)", out.display()));
        }
        std::fs::remove_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    for (p, b) in &files {
        let path = out.join(p);
        std::fs::create_dir_all(path.parent().unwrap_or(out)).map_err(|e| format!("{}: {e}", path.display()))?;
        std::fs::write(&path, b).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    std::fs::write(out.join("manifest.json"), serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n").map_err(|e| format!("manifest.json: {e}"))?;
    Ok(Built { dir: out.to_path_buf(), manifest })
}

// ---- checking ------------------------------------------------------------------------------------------------------------------------------------

fn row(ok: bool, name: &str, detail: impl Into<String>) -> Row {
    Row { ok, claim: "package", name: name.to_string(), detail: detail.into() }
}

/// Checks a package directory using nothing but the directory.
pub fn check(dir: &Path) -> Vec<Row> {
    let mut rows = Vec::new();
    let mpath = dir.join("manifest.json");
    let Ok(mtext) = std::fs::read_to_string(&mpath) else {
        return vec![row(false, "manifest", format!("{} is missing: this is not a web package", mpath.display()))];
    };
    let Ok(m) = serde_json::from_str::<Value>(&mtext) else {
        return vec![row(false, "manifest", "manifest.json is not valid JSON")];
    };
    rows.push(row(m["schema"] == SCHEMA, "manifest schema", format!("{}", m["schema"])));
    let listed: Vec<(String, u64, String)> = m["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|f| (f["path"].as_str().unwrap_or("").to_string(), f["bytes"].as_u64().unwrap_or(0), f["sha256"].as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default();

    // Every listed file is there, intact; nothing else is.
    let mut bad = Vec::new();
    let mut listed_paths = BTreeSet::new();
    for (p, n, h) in &listed {
        listed_paths.insert(p.clone());
        if p.starts_with('/') || p.contains("..") || p.contains('\\') || p.contains(':') {
            bad.push(format!("{p}: not a relative path"));
            continue;
        }
        match std::fs::read(dir.join(p)) {
            Ok(b) => {
                if b.len() as u64 != *n {
                    bad.push(format!("{p}: {} bytes, manifest says {n}", b.len()));
                } else if hex(&sha256(&b)) != *h {
                    bad.push(format!("{p}: contents differ from the manifest's SHA-256 (modified after the build)"));
                }
            }
            Err(_) => bad.push(format!("{p}: listed but missing")),
        }
    }
    rows.push(row(
        bad.is_empty() && !listed.is_empty(),
        "files and hashes",
        if bad.is_empty() { format!("{} file(s), every size and SHA-256 matches", listed.len()) } else { bad.join("; ") },
    ));
    let mut extra = Vec::new();
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut v: Vec<_> = rd.flatten().collect();
            v.sort_by_key(|e| e.file_name());
            for e in v {
                let p = e.path();
                if p.is_dir() {
                    walk(base, &p, out);
                } else if let Ok(r) = p.strip_prefix(base) {
                    out.push(r.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let mut present = Vec::new();
    walk(dir, dir, &mut present);
    for p in present {
        if p != "manifest.json" && !listed_paths.contains(&p) {
            extra.push(p);
        }
    }
    rows.push(row(
        extra.is_empty(),
        "no undeclared files",
        if extra.is_empty() { "every file in the directory is listed".to_string() } else { format!("not in the manifest: {}", extra.join(", ")) },
    ));

    // package_id
    let mut all = Vec::new();
    let mut sorted = listed.clone();
    sorted.sort();
    for (p, _, h) in &sorted {
        all.extend_from_slice(p.as_bytes());
        all.extend_from_slice(h.as_bytes());
    }
    let id = hex(&sha256(&all))[..16].to_string();
    rows.push(row(m["package_id"] == id.as_str(), "package id", format!("{} (recomputed {id})", m["package_id"])));

    // The player module.
    match std::fs::read(dir.join("game.wasm")) {
        Ok(b) => match wasm_info(&b) {
            Ok(info) => {
                rows.push(row(
                    info.imports.is_empty(),
                    "wasm imports nothing",
                    if info.imports.is_empty() {
                        "no env, filesystem, clock, network or JavaScript imports: the module is a pure function of its inputs".to_string()
                    } else {
                        format!("imports {}", info.imports.join(", "))
                    },
                ));
                let missing: Vec<&str> = ABI.iter().copied().chain(["memory"]).filter(|n| !info.exports.iter().any(|e| e == n)).collect();
                rows.push(row(
                    missing.is_empty(),
                    "wasm exports the player ABI",
                    if missing.is_empty() { format!("{} exports", info.exports.len()) } else { format!("missing {}", missing.join(", ")) },
                ));
                let leaks: Vec<&str> =
                    ["/home/", "/Users/", "C:\\Users", "/root/"].into_iter().filter(|n| b.windows(n.len()).any(|w| w == n.as_bytes())).collect();
                rows.push(row(
                    leaks.is_empty(),
                    "wasm has no build-machine paths",
                    if leaks.is_empty() { "none of /home/, /Users/, C:\\Users, /root/".to_string() } else { format!("contains {}", leaks.join(", ")) },
                ));
            }
            Err(e) => rows.push(row(false, "wasm module", e)),
        },
        Err(e) => rows.push(row(false, "wasm module", format!("game.wasm: {e}"))),
    }

    // Text files: no absolute paths, no external references, only declared fetches.
    let mut problems = Vec::new();
    for p in ["index.html", "runtime.js", "audio-worker.js", "sw.js", "manifest.webmanifest", "assets/game.json", "manifest.json"] {
        let Ok(t) = std::fs::read_to_string(dir.join(p)) else { continue };
        for pat in ["/home/", "/Users/", "C:\\\\", "C:/", "file://", "localhost", "127.0.0.1"] {
            if t.contains(pat) && !(p == "runtime.js" && pat == "C:/") {
                problems.push(format!("{p} contains `{pat}`"));
            }
        }
        if p == "index.html" || p == "runtime.js" || p == "audio-worker.js" || p == "sw.js" {
            for attr in ["src=\"", "href=\"", "fetch('", "new Worker('", "register('"] {
                let mut rest = t.as_str();
                while let Some(i) = rest.find(attr) {
                    rest = &rest[i + attr.len()..];
                    let end = rest.find(['"', '\'']).unwrap_or(0);
                    let target = &rest[..end];
                    if target.is_empty() || target == "#" {
                        continue;
                    }
                    if target.starts_with('/') || target.contains("://") {
                        problems.push(format!("{p} refers to `{target}`: only relative, packaged files are allowed"));
                    } else if target != "manifest.json" && !listed_paths.contains(target) {
                        problems.push(format!("{p} loads `{target}`, which the manifest does not declare"));
                    }
                }
            }
        }
    }
    rows.push(row(
        problems.is_empty(),
        "self-contained",
        if problems.is_empty() { "no absolute paths, no external URLs, every loaded file is declared".to_string() } else { problems.join("; ") },
    ));

    // The runtime calls only what the module exports.
    if let Ok(runtime) = std::fs::read_to_string(dir.join("runtime.js")) {
        let js = format!("{runtime}\n{}", std::fs::read_to_string(dir.join("audio-worker.js")).unwrap_or_default());
        let mut used = BTreeSet::new();
        let mut rest = js.as_str();
        while let Some(i) = rest.find("x().") {
            rest = &rest[i + 4..];
            let n: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if rest[n.len()..].starts_with('(') {
                used.insert(n);
            }
        }
        let unknown: Vec<&String> = used.iter().filter(|n| !ABI.contains(&n.as_str())).collect();
        rows.push(row(
            unknown.is_empty(),
            "runtime uses the declared ABI",
            if unknown.is_empty() {
                format!("{} calls, all in the ABI", used.len())
            } else {
                format!("calls {}", unknown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
            },
        ));
    }

    // The game parses and is the game the manifest names.
    match std::fs::read_to_string(dir.join("assets/game.json")) {
        Ok(t) => match red2d::game::parse(&t) {
            Ok(d) => {
                rows.push(row(
                    d.rev == m["game"]["game_revision"].as_str().unwrap_or(""),
                    "game revision",
                    format!("assets/game.json is revision {}; manifest says {}", d.rev, m["game"]["game_revision"]),
                ));
                let web = d.caps.platforms.iter().any(|p| p.name() == "web");
                rows.push(row(
                    web && d.id == m["game"]["id"].as_str().unwrap_or(""),
                    "game identity and target",
                    format!("`{}`, platforms {}", d.id, d.caps.platforms.iter().map(|p| p.name()).collect::<Vec<_>>().join("+")),
                ));
            }
            Err(e) => rows.push(row(false, "game parses", e.join("; "))),
        },
        Err(e) => rows.push(row(false, "game parses", format!("assets/game.json: {e}"))),
    }
    // The web app manifest and icons make the page installable.
    match std::fs::read_to_string(dir.join("manifest.webmanifest")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(w) => {
            let mut bad = Vec::new();
            for k in ["name", "start_url", "scope", "display", "icons", "background_color"] {
                if w.get(k).is_none() {
                    bad.push(format!("missing `{k}`"));
                }
            }
            for k in ["start_url", "scope"] {
                if w[k].as_str().is_some_and(|u| u.starts_with('/') || u.contains("://")) {
                    bad.push(format!("`{k}` must be relative (a package can live at any path)"));
                }
            }
            let mut sizes = Vec::new();
            for icon in w["icons"].as_array().into_iter().flatten() {
                let src = icon["src"].as_str().unwrap_or("");
                match std::fs::read(dir.join(src)) {
                    Ok(b) if b.len() > 24 && b.starts_with(&[0x89, b'P', b'N', b'G']) => {
                        let (pw, ph) = (u32::from_be_bytes([b[16], b[17], b[18], b[19]]), u32::from_be_bytes([b[20], b[21], b[22], b[23]]));
                        sizes.push(pw);
                        if icon["sizes"].as_str() != Some(&format!("{pw}x{ph}")) {
                            bad.push(format!("{src} is {pw}x{ph} but the manifest says {}", icon["sizes"]));
                        }
                    }
                    _ => bad.push(format!("icon {src} is missing or not a PNG")),
                }
            }
            if !(sizes.contains(&192) && sizes.contains(&512)) {
                bad.push("an installable app needs a 192x192 and a 512x512 icon".to_string());
            }
            if !dir.join("sw.js").is_file() {
                bad.push("no sw.js (a service worker with a fetch handler is what makes it work offline)".to_string());
            }
            rows.push(row(
                bad.is_empty(),
                "installable web app",
                if bad.is_empty() { "manifest.webmanifest, icons 192 and 512, sw.js".to_string() } else { bad.join("; ") },
            ));
        }
        None => rows.push(row(false, "installable web app", "manifest.webmanifest is missing or not JSON")),
    }
    // A thumbnail that is a real PNG.
    match std::fs::read(dir.join("thumbnail.png")) {
        Ok(b) => rows.push(row(b.starts_with(&[0x89, b'P', b'N', b'G']), "thumbnail", format!("{} bytes", b.len()))),
        Err(e) => rows.push(row(false, "thumbnail", e.to_string())),
    }
    rows
}

// ---- serving ------------------------------------------------------------------------------------------------------------------------------------

/// The content type of a package file.
pub fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "css" => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Serves a package directory on 127.0.0.1 until the process is stopped; returns only on a bind error.
pub fn serve(dir: &Path, port: u16, announce: impl Fn(u16)) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};
    let l = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
    announce(l.local_addr().map(|a| a.port()).unwrap_or(port));
    let dir = dir.to_path_buf();
    for conn in l.incoming().flatten() {
        let dir = dir.clone();
        std::thread::spawn(move || {
            let mut conn = conn;
            let mut line = String::new();
            if BufReader::new(&conn).read_line(&mut line).is_err() {
                return;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").split(['?', '#']).next().unwrap_or("/").to_string();
            let rel = path.trim_start_matches('/');
            let rel = if rel.is_empty() { "index.html" } else { rel };
            let ok = !rel.contains("..") && !rel.contains('\\');
            let body = if ok { std::fs::read(dir.join(rel)).ok() } else { None };
            let _ = match body {
                Some(b) => {
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                        mime(rel),
                        b.len()
                    );
                    conn.write_all(head.as_bytes()).and_then(|_| conn.write_all(&b))
                }
                None => conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found"),
            };
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_calls_exactly_the_abi_the_module_documents() {
        let mut used = BTreeSet::new();
        let both = format!("{RUNTIME_JS}\n{AUDIO_WORKER_JS}");
        let mut rest = both.as_str();
        while let Some(i) = rest.find("x().") {
            rest = &rest[i + 4..];
            let n: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if rest[n.len()..].starts_with('(') {
                used.insert(n);
            }
        }
        let abi: BTreeSet<String> = ABI.iter().map(|s| s.to_string()).collect();
        let unused: Vec<&String> = abi.difference(&used).collect();
        let unknown: Vec<&String> = used.difference(&abi).collect();
        assert!(unknown.is_empty(), "runtime.js calls exports that are not in ABI: {unknown:?}");
        // `dealloc`, `pointer` etc. are all used by the runtime; an export nobody calls is dead surface and a reason to remove it.
        assert!(unused.is_empty(), "ABI entries the runtime never calls: {unused:?}");
        // And every one exists in src/web.rs.
        let web = include_str!("../../crates/red2d/src/web.rs");
        for n in ABI {
            assert!(web.contains(&format!("pub extern \"C\" fn {n}(")), "web.rs does not export `{n}`");
        }
    }

    #[test]
    fn a_module_reader_finds_imports_and_exports() {
        // (module (import "env" "f" (func)) (func) (export "g" (func 1)))
        let m: Vec<u8> = vec![
            0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, // header
            1, 4, 1, 0x60, 0, 0, // type section: one () -> ()
            2, 9, 1, 3, b'e', b'n', b'v', 1, b'f', 0, 0, // import env.f
            3, 2, 1, 0, // function section
            7, 5, 1, 1, b'g', 0, 1, // export g = func 1
        ];
        let i = wasm_info(&m).unwrap();
        assert_eq!((i.imports.as_slice(), i.exports.as_slice()), (&["env.f".to_string()][..], &["g".to_string()][..]));
        assert!(wasm_info(b"nope nope nope").is_err());
        assert!(wasm_info(&m[..m.len() - 3]).is_err(), "truncation is an error, not a panic");
    }

    #[test]
    fn mime_types_serve_wasm_as_wasm() {
        assert_eq!(mime("game.wasm"), "application/wasm");
        assert_eq!(mime("assets/game.json"), "application/json");
    }
}
