//! The static web package: built from a real WebAssembly player, deterministic, self-describing, and tamper-evident. No browser is needed here (`web verify` is the browser half).
//!
//! These build `crates/red2d` for `wasm32-unknown-unknown`. On a machine without that target they FAIL with the one-line fix, unless `RED2D_WEB_OPTIONAL=1` (the same escape hatch
//! the offscreen render test has) turns "cannot build the player" into a loud skip.

use red_engine2::crypto::{hex, sha256};
use red_engine2::tools::{webpkg, webpkg::SCHEMA};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn game() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/2d/coin-dash.game2d.json")
}

fn wasm() -> Option<&'static Path> {
    static W: OnceLock<Option<PathBuf>> = OnceLock::new();
    W.get_or_init(|| match webpkg::player_wasm() {
        Ok((p, _)) => Some(p),
        Err(e) if std::env::var("RED2D_WEB_OPTIONAL").as_deref() == Ok("1") => {
            eprintln!("SKIPPED (RED2D_WEB_OPTIONAL=1): the WebAssembly player cannot be built here:\n{e}");
            None
        }
        Err(e) => panic!("the WebAssembly player cannot be built: {e}"),
    })
    .as_deref()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("re2_pkg_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn built(name: &str) -> Option<(PathBuf, Value)> {
    let w = wasm()?;
    let dir = scratch(name);
    let b = webpkg::build(&game(), &dir, Some(w)).unwrap_or_else(|e| panic!("{e}"));
    Some((dir, b.manifest))
}

fn failed(dir: &Path) -> Vec<String> {
    webpkg::check(dir).into_iter().filter(|r| !r.ok).map(|r| format!("{}: {}", r.name, r.detail)).collect()
}

/// Rewrites manifest hashes after a deliberate edit, so the *other* checks (not the hash check) are what must catch it.
fn rehash(dir: &Path) {
    let mut m: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let mut all = Vec::new();
    let files = m["files"].as_array_mut().unwrap();
    for f in files.iter_mut() {
        let p = f["path"].as_str().unwrap().to_string();
        let b = std::fs::read(dir.join(&p)).unwrap();
        f["bytes"] = json!(b.len());
        f["sha256"] = json!(hex(&sha256(&b)));
    }
    let mut sorted: Vec<(String, String)> =
        files.iter().map(|f| (f["path"].as_str().unwrap().to_string(), f["sha256"].as_str().unwrap().to_string())).collect();
    sorted.sort();
    for (p, h) in sorted {
        all.extend_from_slice(p.as_bytes());
        all.extend_from_slice(h.as_bytes());
    }
    m["package_id"] = json!(&hex(&sha256(&all))[..16]);
    std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&m).unwrap()).unwrap();
}

#[test]
fn a_built_package_is_complete_consistent_and_names_what_it_is() {
    let Some((dir, m)) = built("complete") else { return };
    assert_eq!(failed(&dir), Vec::<String>::new());
    assert_eq!(m["schema"], SCHEMA);
    assert_eq!(m["game"]["id"], "coin-dash");
    assert_eq!(m["game"]["presentation"], "2d");
    assert_eq!(m["game"]["platforms"], json!(["web"]));
    assert_eq!(m["game"]["networking"], "offline");
    assert!(m["game"]["persistence"].as_array().unwrap().iter().any(|p| p == "progress"));
    assert_eq!(m["game"]["screen"], json!({"width": 320, "height": 180, "scale": "fit"}));
    let paths: Vec<&str> = m["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["assets/game.json", "game.wasm", "index.html", "runtime.js", "thumbnail.png"], "sorted, and exactly the declared files");
    assert!(m["native"]["scenarios"].as_array().unwrap().len() >= 3 && m["native"]["initial"]["frame"].as_str().unwrap().len() == 16);
    assert!(m["browser_checks"].as_array().unwrap().iter().any(|c| c["persists"].as_array().is_some_and(|p| !p.is_empty())));
    let text = std::fs::read_to_string(dir.join("manifest.json")).unwrap();
    assert!(
        m.get("built_at").is_none() && !text.contains("timestamp") && !text.contains("T00:") && !text.contains("2026-"),
        "a deterministic package carries no timestamp"
    );
    let html = std::fs::read_to_string(dir.join("index.html")).unwrap();
    assert!(html.contains("<title>Coin Dash</title>") && html.contains("wasm-unsafe-eval") && !html.contains("{{"), "the page is filled in and keeps its CSP");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_same_game_and_engine_give_byte_identical_packages() {
    let Some((a, ma)) = built("det_a") else { return };
    let (b, mb) = built("det_b").unwrap();
    assert_eq!(ma["package_id"], mb["package_id"]);
    for f in ma["files"].as_array().unwrap() {
        let p = f["path"].as_str().unwrap();
        assert_eq!(std::fs::read(a.join(p)).unwrap(), std::fs::read(b.join(p)).unwrap(), "{p} differs between two builds");
    }
    assert_eq!(std::fs::read(a.join("manifest.json")).unwrap(), std::fs::read(b.join("manifest.json")).unwrap());
    let _ = std::fs::remove_dir_all(a);
    let _ = std::fs::remove_dir_all(b);
}

#[test]
fn the_player_module_imports_nothing_exports_the_abi_and_leaks_no_paths() {
    let Some((dir, _)) = built("module") else { return };
    let bytes = std::fs::read(dir.join("game.wasm")).unwrap();
    let info = webpkg::wasm_info(&bytes).unwrap();
    assert!(info.imports.is_empty(), "the module must need nothing from its host: {:?}", info.imports);
    for n in webpkg::ABI {
        assert!(info.exports.iter().any(|e| e == n), "missing export `{n}`");
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for needle in ["/home/", "/Users/", home.as_str(), env!("CARGO_MANIFEST_DIR")] {
        if needle.len() > 3 {
            assert!(!bytes.windows(needle.len()).any(|w| w == needle.as_bytes()), "the module contains the build machine's path `{needle}`");
        }
    }
    assert!(bytes.len() < 1_500_000, "{} bytes: the player should stay small", bytes.len());
    let _ = std::fs::remove_dir_all(dir);
}

// ---- adversarial: a package that was altered, incomplete or unsafe is caught by the directory alone ------------------------------------------------------------

#[test]
fn a_modified_file_a_missing_asset_and_an_extra_file_are_caught() {
    let Some((dir, _)) = built("tamper") else { return };
    let g = dir.join("assets/game.json");
    let original = std::fs::read(&g).unwrap();
    let mut grown = original.clone();
    grown.push(b' ');
    std::fs::write(&g, &grown).unwrap();
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("files and hashes") && m.contains("assets/game.json") && m.contains("manifest says")), "a different size: {f:?}");
    let mut same_size = original.clone();
    same_size[20] = if same_size[20] == b'x' { b'y' } else { b'x' };
    std::fs::write(&g, &same_size).unwrap();
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("files and hashes") && m.contains("assets/game.json") && m.contains("SHA-256")), "same size, different bytes: {f:?}");
    std::fs::write(&g, &original).unwrap();
    std::fs::remove_file(&g).unwrap();
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("assets/game.json: listed but missing")), "a missing web asset: {f:?}");
    std::fs::write(&g, &original).unwrap();
    std::fs::write(dir.join("extra.png"), b"x").unwrap();
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("no undeclared files") && m.contains("extra.png")), "{f:?}");
    std::fs::remove_file(dir.join("extra.png")).unwrap();
    assert!(failed(&dir).is_empty(), "restored, it is intact again");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_absolute_path_an_external_url_or_an_undeclared_fetch_is_refused_even_with_honest_hashes() {
    let Some((dir, _)) = built("paths") else { return };
    let js = dir.join("runtime.js");
    let original = std::fs::read_to_string(&js).unwrap();
    // 1. An absolute source path baked into the page's script.
    std::fs::write(&js, format!("{original}\n// built in /home/someone/src/game\n")).unwrap();
    rehash(&dir);
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("self-contained") && m.contains("/home/")), "{f:?}");
    // 2. An external URL.
    std::fs::write(&js, original.replace("fetch('game.wasm')", "fetch('https://cdn.example.com/game.wasm')")).unwrap();
    rehash(&dir);
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("self-contained") && (m.contains("://") || m.contains("cdn.example.com"))), "{f:?}");
    // 3. A file the page loads that the manifest does not declare.
    std::fs::write(&js, original.replace("fetch('game.wasm')", "fetch('secret/extra.wasm')")).unwrap();
    rehash(&dir);
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("self-contained") && m.contains("secret/extra.wasm") && m.contains("does not declare")), "{f:?}");
    // 4. A runtime that calls something the module does not export.
    std::fs::write(&js, original.replace("x().render()", "x().render_gpu()")).unwrap();
    rehash(&dir);
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("runtime uses the declared ABI") && m.contains("render_gpu")), "{f:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_module_that_imports_the_host_or_is_not_a_module_is_refused() {
    let Some((dir, _)) = built("badwasm") else { return };
    let w = dir.join("game.wasm");
    // A module that imports env.f.
    let imports: Vec<u8> = vec![0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 1, 4, 1, 0x60, 0, 0, 2, 9, 1, 3, b'e', b'n', b'v', 1, b'f', 0, 0];
    std::fs::write(&w, imports).unwrap();
    rehash(&dir);
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("wasm imports nothing") && m.contains("env.f")), "{f:?}");
    assert!(f.iter().any(|m| m.contains("exports the player ABI") && m.contains("missing")), "{f:?}");
    std::fs::write(&w, b"not wasm at all").unwrap();
    rehash(&dir);
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("wasm module") && m.contains("not a WebAssembly module")), "{f:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_game_whose_revision_does_not_match_the_manifest_is_refused() {
    let Some((dir, _)) = built("rev") else { return };
    let g = dir.join("assets/game.json");
    let t = std::fs::read_to_string(&g).unwrap().replace("A top-down arcade dash", "A different description of the dash");
    std::fs::write(&g, t).unwrap();
    rehash(&dir);
    let f = failed(&dir);
    assert!(f.iter().any(|m| m.contains("game revision")), "{f:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn building_refuses_what_it_should_not_touch_or_cannot_deliver() {
    let Some(w) = wasm() else { return };
    // A directory that is not a package is never replaced.
    let precious = scratch("precious");
    std::fs::create_dir_all(&precious).unwrap();
    std::fs::write(precious.join("notes.txt"), "mine").unwrap();
    let e = webpkg::build(&game(), &precious, Some(w)).map(|_| ()).unwrap_err();
    assert!(e.contains("not a web package") && e.contains("refusing to replace"), "{e}");
    assert_eq!(std::fs::read_to_string(precious.join("notes.txt")).unwrap(), "mine");
    let _ = std::fs::remove_dir_all(&precious);
    // A game that does not declare the web platform is not built for it.
    let mut g: Value = serde_json::from_str(&std::fs::read_to_string(game()).unwrap()).unwrap();
    g["capabilities"]["platforms"] = json!(["linux"]);
    let p = std::env::temp_dir().join(format!("re2_noweb_{}.game2d.json", std::process::id()));
    std::fs::write(&p, g.to_string()).unwrap();
    let e = webpkg::build(&p, &scratch("noweb"), Some(w)).map(|_| ()).unwrap_err();
    assert!(e.contains("cannot target `linux`") || e.contains("does not declare the `web` platform"), "{e}");
    let _ = std::fs::remove_file(p);
    // A package is rebuilt in place (the previous one is replaced, not appended to).
    let d = scratch("again");
    webpkg::build(&game(), &d, Some(w)).unwrap();
    std::fs::write(d.join("stale.txt"), "x").unwrap();
    webpkg::build(&game(), &d, Some(w)).unwrap();
    assert!(!d.join("stale.txt").exists());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_directory_that_is_not_a_package_says_so() {
    let d = scratch("nothing");
    std::fs::create_dir_all(&d).unwrap();
    let rows = webpkg::check(&d);
    assert!(rows.len() == 1 && !rows[0].ok && rows[0].detail.contains("not a web package"), "{rows:?}");
    let _ = std::fs::remove_dir_all(d);
}
