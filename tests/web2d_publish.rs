//! Publishing, without needing a browser: the gates that must hold before anything is uploaded, the static site's layout and immutability, the GitHub Pages backend against a local bare
//! repository, and the report's honesty (a URL only if one exists; the four successes separate). The browser stage itself is exercised by the `web` CI stage, which publishes
//! the example games for real.

use red_engine2::tools::publish2d::{self, Verification};
use red_engine2::tools::webpkg;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_red_engine2")
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("re2_publish_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str], cwd: &Path) -> (Output, String) {
    let o = Command::new(bin()).args(args).current_dir(cwd).output().expect("start red_engine2");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    (o, text)
}

fn wasm() -> Option<&'static Path> {
    static W: OnceLock<Option<PathBuf>> = OnceLock::new();
    W.get_or_init(|| match webpkg::player_wasm() {
        Ok((p, _)) => Some(p),
        Err(e) if std::env::var("RED2D_WEB_OPTIONAL").as_deref() == Ok("1") => {
            eprintln!("SKIPPED (RED2D_WEB_OPTIONAL=1): {e}");
            None
        }
        Err(e) => panic!("the WebAssembly player cannot be built: {e}"),
    })
    .as_deref()
}

fn package(dir: &Path, edit: impl FnOnce(&mut Value)) -> Option<(PathBuf, Value)> {
    let w = wasm()?;
    let mut g: Value = serde_json::from_str(&std::fs::read_to_string(root().join("examples/2d/coin-dash.game2d.json")).unwrap()).unwrap();
    edit(&mut g);
    let game = dir.join("coin-dash.game2d.json");
    std::fs::write(&game, g.to_string()).unwrap();
    let out = dir.join("pkg");
    let b = webpkg::build(&game, &out, Some(w)).unwrap_or_else(|e| panic!("{e}"));
    Some((out, b.manifest))
}

fn ver(manifest: &Value) -> Verification {
    Verification { package_id: manifest["package_id"].as_str().unwrap().into(), ok: true, browser: "Chromium test".into(), checks: 3, audio_claims: vec![] }
}

// ---- gates ---------------------------------------------------------------------------------------------------------------------------------------

#[test]
fn publishing_a_package_nobody_played_is_refused_and_so_is_one_that_changed_after_it_was() {
    let Some((pkg, m)) = package(&scratch("gate"), |_| {}) else { return };
    let cwd = scratch("gate_cwd");
    let site = cwd.join("site");
    let args = |extra: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = ["publish", "--package", pkg.to_str().unwrap(), "--site", site.to_str().unwrap()].iter().map(|s| s.to_string()).collect();
        v.extend(extra.iter().map(|s| s.to_string()));
        v
    };
    let go = |a: &[String]| run(&a.iter().map(String::as_str).collect::<Vec<_>>(), &cwd);
    // 1. no browser record at all
    let (o, t) = go(&args(&[]));
    assert!(!o.status.success() && t.contains("has not been verified in a browser") && t.contains("web verify --package") && t.contains("refused"), "{t}");
    assert!(!site.exists(), "nothing was uploaded");
    // 2. a record that failed
    let rec = cwd.join("rec");
    std::fs::create_dir_all(&rec).unwrap();
    let mut v = ver(&m);
    v.ok = false;
    std::fs::write(rec.join("verification.json"), v.to_json().to_string()).unwrap();
    let (o, t) = go(&args(&["--out", rec.to_str().unwrap()]));
    assert!(!o.status.success() && t.contains("FAILED") && t.contains("a failing game is not published"), "{t}");
    // 3. a record for a different package (the package changed after it was verified)
    let mut other = ver(&m);
    other.package_id = "0000000000000000".into();
    std::fs::write(rec.join("verification.json"), other.to_json().to_string()).unwrap();
    let (o, t) = go(&args(&["--out", rec.to_str().unwrap()]));
    assert!(!o.status.success() && t.contains("is for package 0000000000000000") && t.contains("changed after it was verified"), "{t}");
    // 4. a tampered package with an honest record is refused by the integrity check
    std::fs::write(rec.join("verification.json"), ver(&m).to_json().to_string()).unwrap();
    let g = pkg.join("assets/game.json");
    let mut bytes = std::fs::read(&g).unwrap();
    bytes.push(b' ');
    std::fs::write(&g, bytes).unwrap();
    let (o, t) = go(&args(&["--out", rec.to_str().unwrap()]));
    assert!(!o.status.success() && t.contains("fails its integrity check") && t.contains("assets/game.json"), "{t}");
    assert!(!site.exists());
}

#[test]
fn a_verified_package_is_published_to_a_local_site_and_the_report_claims_no_url() {
    let Some((pkg, m)) = package(&scratch("local"), |_| {}) else { return };
    let cwd = scratch("local_cwd");
    let rec = cwd.join("rec");
    std::fs::create_dir_all(&rec).unwrap();
    std::fs::write(rec.join("verification.json"), ver(&m).to_json().to_string()).unwrap();
    let site = cwd.join("site");
    // --dry-run uploads nothing.
    let (o, t) = run(&["publish", "--package", pkg.to_str().unwrap(), "--site", site.to_str().unwrap(), "--out", rec.to_str().unwrap(), "--dry-run"], &cwd);
    assert!(o.status.success() && t.contains("dry run: nothing uploaded") && !site.exists(), "{t}");
    // For real. No browser runs here, so the loopback re-check cannot pass; what matters is what is claimed.
    let (_o, t) = run(&["publish", "--package", pkg.to_str().unwrap(), "--site", site.to_str().unwrap(), "--out", rec.to_str().unwrap()], &cwd);
    assert!(site.join("games/coin-dash/builds").join(m["package_id"].as_str().unwrap()).join("manifest.json").is_file(), "{t}");
    assert!(
        site.join("games/coin-dash/manifest.json").is_file()
            && site.join("games/coin-dash/game.json").is_file()
            && site.join("catalog.json").is_file()
            && site.join("index.html").is_file()
    );
    assert!(t.contains("BUILD SUCCESS: yes") && t.contains("LOCAL BROWSER SUCCESS: yes") && t.contains("REMOTE PLAYABLE SUCCESS: no"), "{t}");
    assert!(!t.contains("URL: http") && !t.contains("URL: file"), "no URL is invented for a site nobody serves:\n{t}");
    assert!(t.contains("PUBLICATION UNAVAILABLE") || t.contains("FAILED at stage 9"), "{t}");
    let c: Value = serde_json::from_str(&std::fs::read_to_string(site.join("catalog.json")).unwrap()).unwrap();
    assert_eq!(c["games"][0]["id"], "coin-dash");
    assert_eq!(c["games"][0]["verification"]["browser"]["engine"], "Chromium test");
}

#[test]
fn mistakes_in_the_command_and_the_game_stop_at_the_stage_that_owns_them() {
    let cwd = scratch("stages");
    let game = |g: Value| -> PathBuf {
        let p = cwd.join("g.game2d.json");
        std::fs::write(&p, g.to_string()).unwrap();
        p
    };
    let base: Value = serde_json::from_str(&std::fs::read_to_string(root().join("examples/2d/coin-dash.game2d.json")).unwrap()).unwrap();
    // A game that does not validate stops at stage 1, before any build.
    let mut bad = base.clone();
    bad["capabilities"]["networking"] = json!("authoritative");
    let (o, t) = run(&["publish", game(bad).to_str().unwrap(), "--out", cwd.join("o1").to_str().unwrap()], &cwd);
    assert!(!o.status.success() && t.contains("FAILED at stage 1 (validate)") && t.contains("Browser target cannot use the native UDP transport"), "{t}");
    assert!(t.contains("skip wasm build") && t.contains("BUILD SUCCESS: no") && t.contains("URL: none."), "{t}");
    // A missing asset (a sprite) is a validation failure, with the fix.
    let mut bad = base.clone();
    bad["prefabs"]["coin"]["shape"] = json!({"sprite": "coins"});
    let (o, t) = run(&["publish", game(bad).to_str().unwrap(), "--out", cwd.join("o2").to_str().unwrap()], &cwd);
    assert!(!o.status.success() && t.contains("stage 1 (validate)") && t.contains("no sprite `coins`") && t.contains("did you mean `coin`"), "{t}");
    // A game whose playthrough regressed stops at stage 2.
    let mut bad = base.clone();
    bad["rules"][0]["do"][0] = json!({"add": ["score", 0]});
    let (o, t) = run(&["publish", game(bad).to_str().unwrap(), "--out", cwd.join("o3").to_str().unwrap()], &cwd);
    assert!(!o.status.success() && t.contains("FAILED at stage 2 (gameplay tests)") && t.contains("expected score"), "{t}");
    // Command mistakes.
    let (o, t) = run(&["publish"], &cwd);
    assert!(!o.status.success() && t.contains("give a game file, or --package DIR"), "{t}");
    let (o, t) = run(&["publish", game(base.clone()).to_str().unwrap(), "--backend", "ftp"], &cwd);
    assert!(!o.status.success() && t.contains("use `local` or `github-pages`"), "{t}");
    let (o, t) = run(&["publish", game(base).to_str().unwrap(), "--backend", "github-pages"], &cwd);
    assert!(!o.status.success() && t.contains("--backend github-pages needs --repo") && t.contains("RedEngineGames"), "{t}");
}

// ---- the site and the GitHub Pages backend -----------------------------------------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t.invalid"]).args(args).output().expect("git");
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

#[test]
fn the_pages_backend_commits_pushes_keeps_every_build_and_does_not_recommit_the_same_one() {
    let Some((pkg1, m1)) = package(&scratch("pages1"), |_| {}) else { return };
    let (pkg2, m2) = package(&scratch("pages2"), |g| g["description"] = json!("A second build with another description.")).unwrap();
    assert_ne!(m1["package_id"], m2["package_id"]);
    let tmp = scratch("pages_repo");
    let origin = tmp.join("origin.git");
    assert!(Command::new("git").args(["init", "-q", "--bare", "-b", "main"]).arg(&origin).status().unwrap().success());
    let repo = tmp.join("clone");
    assert!(Command::new("git").args(["clone", "-q"]).arg(&origin).arg(&repo).output().unwrap().status.success());
    std::fs::write(repo.join("README.md"), "games").unwrap();
    git(&repo, &["checkout", "-q", "-B", "main"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    git(&repo, &["push", "-q", "origin", "main"]);

    // Not pushed: committed locally, the remote has nothing new, and the function says so.
    let (site, pushed) = publish2d::github_upload(&repo, &pkg1, &ver(&m1), 1_000, false).unwrap();
    assert!(!pushed && site.ends_with("webgames"));
    assert_eq!(git(&origin, &["rev-list", "--count", "main"]), "1", "nothing left the machine");
    assert_eq!(git(&repo, &["rev-list", "--count", "HEAD"]), "2");
    assert!(git(&repo, &["log", "-1", "--format=%s"]).contains("Browser game coin-dash: build"));
    // Pushed: the remote has it.
    publish2d::github_upload(&repo, &pkg2, &ver(&m2), 2_000, true).unwrap();
    assert_eq!(git(&origin, &["rev-list", "--count", "main"]), "3");
    let listing = git(&origin, &["ls-tree", "-r", "--name-only", "main"]);
    for build in [m1["package_id"].as_str().unwrap(), m2["package_id"].as_str().unwrap()] {
        assert!(listing.contains(&format!("webgames/games/coin-dash/builds/{build}/manifest.json")), "both builds are kept: {listing}");
    }
    assert!(listing.contains("webgames/games/coin-dash/game.json") && listing.contains("webgames/catalog.json"), "{listing}");
    let latest: Value = serde_json::from_str(&std::fs::read_to_string(repo.join("webgames/games/coin-dash/manifest.json")).unwrap()).unwrap();
    assert_eq!(latest["package_id"], m2["package_id"], "the stable path serves the newest build");
    // The same build again changes nothing worth a commit.
    let before = git(&repo, &["rev-list", "--count", "HEAD"]);
    publish2d::github_upload(&repo, &pkg2, &ver(&m2), 2_000, false).unwrap();
    assert_eq!(git(&repo, &["rev-list", "--count", "HEAD"]), before);
    // A path that is not a checkout is refused with what to do.
    let e = publish2d::github_upload(&tmp.join("nowhere"), &pkg1, &ver(&m1), 1, false).unwrap_err();
    assert!(e.contains("not a git checkout") && e.contains("RedEngineGames"), "{e}");
}

#[test]
fn the_pages_generator_patch_lists_browser_games_when_they_exist() {
    // The site generator lives in the RedEngineGames repository; this checks the contract it reads: catalog.json's fields.
    let Some((pkg, m)) = package(&scratch("contract"), |_| {}) else { return };
    let site = scratch("contract_site");
    publish2d::write_site(&site, &pkg, &ver(&m), 3_000).unwrap();
    let c: Value = serde_json::from_str(&std::fs::read_to_string(site.join("catalog.json")).unwrap()).unwrap();
    let g = &c["games"][0];
    for k in [
        "id",
        "title",
        "description",
        "url",
        "thumbnail",
        "presentation",
        "input",
        "build_timestamp",
        "game_revision",
        "engine_revision",
        "compatibility",
        "verification",
    ] {
        assert!(g.get(k).is_some_and(|v| !v.is_null()), "catalog entry lacks `{k}`: {g}");
    }
    assert!(site.join(g["thumbnail"].as_str().unwrap()).is_file(), "the thumbnail path resolves inside the site");
    assert!(site.join(g["url"].as_str().unwrap()).join("index.html").is_file(), "the url resolves to the game");
    assert_eq!(g["build_timestamp"], "1970-01-01T00:50:00Z");
}
