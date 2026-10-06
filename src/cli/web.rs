//! `red_engine2 web ...`: build, check, serve and browser-verify 2D game packages.

use super::*;
use red2d::script::Row;
use red_engine2::tools::{game2d, publish2d, webpkg, webverify};

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
            if let Some(d) = &dir {
                // The record `publish --package` requires: which package was played, and whether every check passed.
                if let Some(id) = std::fs::read_to_string(d.join("manifest.json"))
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .and_then(|m| m["package_id"].as_str().map(str::to_string))
                {
                    let ver = publish2d::Verification {
                        package_id: id,
                        ok: failed == 0,
                        browser: browser.clone(),
                        checks: all.len(),
                        audio_claims: all.iter().filter(|r| r.claim == "browser-audio" && r.ok).map(|r| r.name.clone()).collect(),
                        features: publish2d::features_of(&all),
                    };
                    std::fs::write(out.join("verification.json"), serde_json::to_string_pretty(&ver.to_json()).unwrap_or_default()).ok();
                }
            }
            println!(
                "{} in {browser} ({} checks; screenshots and report.json in {})",
                if failed == 0 { "LOCAL BROWSER SUCCESS" } else { "BROWSER VERIFICATION FAILED" },
                all.len(),
                out.display()
            );
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

/// `publish`: the staged pipeline, or (with `--package`) the upload half for a package a browser already passed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_publish(
    game: Option<&Path>,
    package: Option<&Path>,
    backend: &str,
    site: Option<PathBuf>,
    base_url: Option<String>,
    repo: Option<PathBuf>,
    push: bool,
    pages_url: Option<String>,
    dry_run: bool,
    out: Option<PathBuf>,
    wasm: Option<PathBuf>,
) -> Result<(), String> {
    use publish2d::{Backend, Options};
    let backend = match backend {
        "local" => Backend::Local { site: site.unwrap_or_else(|| PathBuf::from("out/site")), base_url },
        "github-pages" => Backend::GithubPages {
            repo: repo.ok_or("--backend github-pages needs --repo PATH (a checkout of https://github.com/kevstermcgee/RedEngineGames)")?,
            push,
            pages_url: pages_url.unwrap_or_else(|| "https://kevstermcgee.github.io/RedEngineGames".into()),
        },
        other => return Err(format!("--backend {other}: use `local` or `github-pages`")),
    };
    let wasm = wasm.or_else(|| std::env::var("RED2D_WASM").ok().map(PathBuf::from));
    let id_of = |g: &Path| game2d::load(g).map(|(d, _)| d.id.clone());
    let started = std::time::Instant::now();
    let outcome = match (game, package) {
        (Some(g), None) => {
            let out = out.unwrap_or_else(|| PathBuf::from("out/publish").join(id_of(g).unwrap_or_else(|_| "game".into())));
            let opts = Options { backend, dry_run, out, wasm };
            publish2d::publish(g, &opts, |s| {
                println!(
                    "{:>2}. {} {:<20} {}",
                    publish2d::STAGES.iter().position(|n| *n == s.stage).map_or(0, |i| i + 1),
                    if s.ok { "ok  " } else { "FAIL" },
                    s.stage,
                    s.detail.lines().next().unwrap_or("")
                )
            })
        }
        (None, Some(p)) => {
            // Publishing without playing it is refused: the package must carry a passing browser record for exactly this package.
            let manifest: serde_json::Value = std::fs::read_to_string(p.join("manifest.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .ok_or_else(|| format!("{} has no manifest.json: it is not a built package (`red_engine2 web build GAME`)", p.display()))?;
            let id = manifest["package_id"].as_str().unwrap_or("");
            let gid = manifest["game"]["id"].as_str().unwrap_or("game");
            let rec_path = PathBuf::from("out/web-verify").join(p.file_name().unwrap_or_default()).join("verification.json");
            let rec_path = if let Some(o) = &out { o.join("verification.json") } else { rec_path };
            let ver = std::fs::read_to_string(&rec_path)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|v| publish2d::Verification::from_json(&v))
                .ok_or_else(|| format!("this package has not been verified in a browser: no record at {} (run `red_engine2 web verify --package {}` first; publishing an unplayed package is refused)", rec_path.display(), p.display()))?;
            if ver.package_id != id {
                return Err(format!("the browser verification at {} is for package {}, not {id}: the package changed after it was verified; run `web verify --package {}` again", rec_path.display(), ver.package_id, p.display()));
            }
            if !ver.ok {
                return Err(format!(
                    "the browser verification of package {id} FAILED ({}): fix it and `web verify` again; a failing game is not published",
                    rec_path.display()
                ));
            }
            let bad: Vec<String> = webpkg::check(p).into_iter().filter(|r| !r.ok).map(|r| format!("{}: {}", r.name, r.detail)).collect();
            if !bad.is_empty() {
                return Err(format!("the package fails its integrity check: {}", bad.join("; ")));
            }
            let mut o = publish2d::Outcome::default();
            o.states.build = true;
            o.states.local_browser = true;
            o.build_id = Some(id.to_string());
            let opts = Options { backend, dry_run, out: out.unwrap_or_else(|| PathBuf::from("out/publish").join(gid)), wasm };
            publish2d::upload_and_confirm(
                &mut o,
                &mut |s| {
                    println!(
                        "{:>2}. {} {:<20} {}",
                        publish2d::STAGES.iter().position(|n| *n == s.stage).map_or(0, |i| i + 1),
                        if s.ok { "ok  " } else { "FAIL" },
                        s.stage,
                        s.detail.lines().next().unwrap_or("")
                    )
                },
                p,
                &ver,
                &opts,
            );
            o
        }
        _ => return Err("give a game file, or --package DIR (a package `web verify` passed)".into()),
    };
    println!();
    print!("{}", publish2d::render(&outcome));
    println!("({:.1} s)", started.elapsed().as_secs_f32());
    if outcome.ok() {
        Ok(())
    } else {
        Err(String::new())
    }
}

/// `propose`: a plan from an idea.
pub(crate) fn run_propose(
    idea: &str,
    title: Option<String>,
    presentation: Option<String>,
    platforms: Vec<String>,
    inputs: Vec<String>,
    networking: Option<String>,
    session: Option<u32>,
) -> Result<(), String> {
    use red2d::caps::{Input, Networking, Platform, Presentation};
    if idea.trim().is_empty() {
        return Err("say the idea in a few words, like `propose \"a small arcade game where you dodge asteroids\"`".into());
    }
    fn one<T>(what: &str, v: &str, parse: fn(&str) -> Option<T>, names: Vec<&'static str>) -> Result<T, String> {
        parse(v).ok_or_else(|| format!("--{what} `{v}` is not one of {}", names.join(", ")))
    }
    let o = red_engine2::tools::propose::Overrides {
        title,
        presentation: presentation.as_deref().map(|p| one("presentation", p, Presentation::parse, Presentation::names())).transpose()?,
        platforms: platforms.iter().map(|p| one("platform", p, Platform::parse, Platform::names())).collect::<Result<_, _>>()?,
        input: inputs.iter().map(|p| one("input", p, Input::parse, Input::names())).collect::<Result<_, _>>()?,
        networking: networking.as_deref().map(|p| one("networking", p, Networking::parse, Networking::names())).transpose()?,
        session_minutes: session,
    };
    let p = red_engine2::tools::propose::propose(idea, &o);
    if red_engine2::tools::envelope::capturing() {
        println!("{}", serde_json::to_string_pretty(&p.to_json()).unwrap_or_default());
    } else {
        print!("{}", p.render());
    }
    if p.buildable() {
        Ok(())
    } else {
        Err(String::new())
    }
}
