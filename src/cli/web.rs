//! `red_engine2 web ...`: build, check, serve and browser-verify 2D game packages.

use super::*;
use red2d::script::Row;
use red_engine2::tools::{game2d, webpkg, webverify};

fn rows_text(rows: &[Row]) -> (String, usize) {
    let mut t = String::new();
    let mut failed = 0;
    for r in rows {
        if !r.ok {
            failed += 1;
        }
        t.push_str(&format!("{}  [{}] {}: {}\n", if r.ok { "PASS" } else { "FAIL" }, r.claim, r.name, r.detail));
    }
    (t, failed)
}

/// The default package directory for a game: `out/web/<id>`.
pub(crate) fn default_package_dir(game: &Path) -> Result<PathBuf, String> {
    let (d, _) = game2d::load(game)?;
    Ok(PathBuf::from("out/web").join(&d.id))
}

pub(crate) fn run_web(cmd: WebCmd) -> Result<(), String> {
    match cmd {
        WebCmd::Build { game, out, wasm } => {
            let out = match out {
                Some(o) => o,
                None => default_package_dir(&game)?,
            };
            let wasm = wasm.or_else(|| std::env::var("RED2D_WASM").ok().map(PathBuf::from));
            let started = std::time::Instant::now();
            let built = webpkg::build(&game, &out, wasm.as_deref())?;
            let rows = webpkg::check(&built.dir);
            let (t, failed) = rows_text(&rows);
            print!("{t}");
            let m = &built.manifest;
            println!(
                "{} {} -> {} (package {}, game revision {}, engine {}{}, {} files, {:.1} s)",
                if failed == 0 { "BUILD SUCCESS" } else { "BUILD FAILED" },
                m["game"]["id"].as_str().unwrap_or(""),
                built.dir.display(),
                m["package_id"].as_str().unwrap_or(""),
                m["game"]["game_revision"].as_str().unwrap_or(""),
                m["engine"]["revision"].as_str().unwrap_or(""),
                if m["engine"]["dirty"].as_bool() == Some(true) { "+dirty" } else { "" },
                m["files"].as_array().map_or(0, Vec::len) + 1,
                started.elapsed().as_secs_f32()
            );
            println!("BUILD SUCCESS means the static package exists and is internally consistent. It does not mean the game runs in a browser: `red_engine2 web verify` does that.");
            if failed == 0 {
                Ok(())
            } else {
                Err(String::new())
            }
        }
        WebCmd::Check { dir } => {
            let (t, failed) = rows_text(&webpkg::check(&dir));
            print!("{t}");
            if failed == 0 {
                println!("package is intact");
                Ok(())
            } else {
                Err(format!("{failed} check(s) failed"))
            }
        }
        WebCmd::Serve { dir, port } => {
            if !dir.join("manifest.json").is_file() {
                return Err(format!("{} has no manifest.json: build a package first (`red_engine2 web build GAME`)", dir.display()));
            }
            webpkg::serve(&dir, port, |p| println!("serving {} on http://127.0.0.1:{p}/ (Ctrl-C to stop)", dir.display()))
        }
        WebCmd::SetupBrowser => {
            println!("{}", webverify::setup_browser()?);
            Ok(())
        }
        WebCmd::Verify { game, package, out, url } => {
            // 1. Build (or take) the package. 2. Check it. 3. Run it in the browser.
            let (dir, id) = match (&package, &game, &url) {
                (Some(p), _, None) => (Some(p.clone()), p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                (None, Some(g), None) => {
                    let out = default_package_dir(g)?;
                    webpkg::build(g, &out, std::env::var("RED2D_WASM").ok().map(PathBuf::from).as_deref())?;
                    let id = out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    (Some(out), id)
                }
                (_, _, Some(u)) => (None, u.rsplit('/').find(|s| !s.is_empty()).unwrap_or("remote").to_string()),
                _ => return Err("give a game file, a --package directory, or a --url".into()),
            };
            let mut all = Vec::new();
            if let Some(d) = &dir {
                all.extend(webpkg::check(d));
            }
            let out = out.unwrap_or_else(|| PathBuf::from("out/web-verify").join(&id));
            let v = webverify::verify(dir.as_deref(), url.as_deref(), &out)?;
            let browser = v.browser.clone();
            all.extend(v.rows);
            let (t, failed) = rows_text(&all);
            print!("{t}");
            std::fs::write(out.join("report.json"), serde_json::to_string_pretty(&v.raw).unwrap_or_default()).ok();
            println!("{} in {browser} ({} checks; screenshots and report.json in {})", if failed == 0 { "LOCAL BROWSER SUCCESS" } else { "BROWSER VERIFICATION FAILED" }, all.len(), out.display());
            if failed == 0 {
                println!("PROVEN: the page loads over HTTP, the module initialises, the first frame is pixel-identical to the native renderer, the game's scenarios replay to the native hashes, real key/mouse events change the state, saves survive a reload and bad storage does not break the game, audio starts after a gesture (browser-audio rows), the console is clean.");
                println!("NOT PROVEN: that a human played it, that it is fun, that it sounds right, other browsers or devices (touch and gamepad paths are unverified).");
            } else {
                println!("Only the PASS rows above hold; the FAIL rows say what to fix.");
            }
            if failed == 0 {
                Ok(())
            } else {
                Err(String::new())
            }
        }
    }
}
