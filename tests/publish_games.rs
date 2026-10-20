//! `scripts/publish_games.py export` must never delete what it does not own. The real script is run on a tiny throwaway repository: hand-added files in the managed folders
//! survive, files an earlier export published and this one no longer does are removed (with the folders that leaves empty), a catalog path that tries to escape is ignored,
//! and exporting twice changes nothing. (The engine's `publish-games.yml` pushes the result to `RedEngineGames` on every push to `main` that touches the source; the old script
//! deleted each whole folder first, which would have wiped the hand-added minigames.) Skipped where `python3` is not installed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().map(|o| o.status.success()).unwrap_or(false))
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn tree(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                // Forward slashes and no carriage returns on every platform (Windows prints `games\demo` and Python writes CRLF), so the assertions read the same everywhere.
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                out.push(format!("{rel}={}", fs::read_to_string(&p).unwrap_or_default().trim().replace(['\r', '\n'], " ")));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

/// A throwaway engine repository with the real publisher in it.
fn engine(tag: &str, with_tests_entry: bool) -> PathBuf {
    let root = std::env::temp_dir().join(format!("re2_publish_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::copy(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/publish_games.py"), root.join("scripts/publish_games.py")).unwrap();
    write(&root.join("src_games/run.json"), "the game");
    write(&root.join("src_protos/p.txt"), "a prototype");
    write(&root.join("src_tests/t.txt"), "a test");
    write(&root.join("src_demos/d.txt"), "a demo");
    let tests =
        if with_tests_entry { r#"[{"source":"src_tests/t.txt","destination":"t"}]"# } else { r#"[{"source":"src_demos/d.txt","destination":"other"}]"# };
    write(
        &root.join("games-publish.json"),
        &format!(
            r#"{{"version":1,"source_repository":"x/engine","target_repository":"x/games",
            "collections":{{"games":[{{"source":"src_games/run.json","destination":"demo"}}],"prototypes":[{{"source":"src_protos/p.txt","destination":"p"}}],
              "tests":{tests},"demos":[{{"source":"src_demos/d.txt","destination":"d"}}]}},
            "playables":[{{"slug":"demo","name":"Demo","entry":"games/demo/run.json","files":["games/demo/run.json"],"arguments":[]}}]}}"#
        ),
    );
    root
}

fn export(py: &str, root: &Path, out: &Path) {
    let r = Command::new(py)
        .arg("scripts/publish_games.py")
        .args(["export", "--output"])
        .arg(out)
        .args(["--revision", "abc123"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
}

#[test]
fn an_export_writes_what_it_publishes_removes_only_what_it_published_before_and_leaves_everything_else() {
    let Some(py) = python() else { return };
    let root = engine("keep", true);
    let out = std::env::temp_dir().join(format!("re2_publish_out_{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    // The Games repository as it is today: two hand-added minigames, plus a file a previous export published, and the catalog that says so.
    write(&out.join("games/laser-vault/laser_vault.json"), "hand added");
    write(&out.join("games/laser-vault/README.md"), "readme");
    write(&out.join("games/extra.md"), "also hand added");
    write(&out.join("games/old-game/gone.json"), "published before");
    write(&out.join("tests/old/x.txt"), "published before");
    write(
        &out.join(".games-catalog.json"),
        r#"{"files":[{"path":"games/old-game/gone.json","sha256":"0"},{"path":"tests/old/x.txt","sha256":"0"},{"path":"games/../../victim.txt","sha256":"0"},{"path":"elsewhere/x","sha256":"0"}]}"#,
    );
    let victim = out.parent().unwrap().join("victim.txt");
    write(&victim, "must survive");

    export(py, &root, &out);
    let after = tree(&out);
    let has = |needle: &str| after.iter().any(|l| l.starts_with(needle));
    assert!(
        has("games/demo/run.json=the game") && has("prototypes/p/p.txt=a prototype") && has("tests/t/t.txt=a test") && has("demos/d/d.txt=a demo"),
        "{after:?}"
    );
    assert!(
        has("games/laser-vault/laser_vault.json=hand added") && has("games/laser-vault/README.md=readme") && has("games/extra.md=also hand added"),
        "hand-added files survive: {after:?}"
    );
    assert!(!has("games/old-game") && !has("tests/old"), "files an earlier export published and this one does not are gone: {after:?}");
    assert!(!out.join("games/old-game").exists() && !out.join("tests/old").exists(), "and so are the folders that left empty");
    assert_eq!(fs::read_to_string(&victim).unwrap(), "must survive", "a catalog path that escapes the output is ignored, never followed");
    let catalog = fs::read_to_string(out.join(".games-catalog.json")).unwrap();
    assert!(catalog.contains("abc123") && catalog.contains("games/demo/run.json"));

    // Exporting again changes nothing.
    export(py, &root, &out);
    assert_eq!(tree(&out), after, "idempotent");

    // Drop the `tests` entry from the manifest: its published file is removed next time; the hand-added files still stay.
    let root2 = engine("keep2", false);
    export(py, &root2, &out);
    let later = tree(&out);
    assert!(!later.iter().any(|l| l.starts_with("tests/t/")), "the entry the manifest dropped is removed: {later:?}");
    assert!(later.iter().any(|l| l.starts_with("games/laser-vault/laser_vault.json=hand added")), "{later:?}");
    for d in [&root, &root2, &out] {
        let _ = fs::remove_dir_all(d);
    }
    let _ = fs::remove_file(victim);
}

fn export_status(py: &str, root: &Path, out: &Path) -> (bool, String) {
    let r = Command::new(py)
        .arg("scripts/publish_games.py")
        .args(["export", "--output"])
        .arg(out)
        .args(["--revision", "abc123"])
        .current_dir(root)
        .output()
        .unwrap();
    (r.status.success(), String::from_utf8_lossy(&r.stderr).into_owned())
}

/// A playable's `page` folder (its page on the download site) is published beside the game, to `site/wiki/<slug>/` and `site/thumbs/<slug>.png`, so a game and its
/// page arrive in one pull request; it is held to the Games repository's own page rules (`site/wiki.py`) first; a page made by hand there is never touched; and a page the
/// manifest stops naming is removed like any other published file.
#[test]
fn a_games_page_is_published_with_it_checked_by_the_sites_rules_and_removed_when_dropped() {
    let Some(py) = python() else { return };
    let root = engine("page", true);
    let manifest = fs::read_to_string(root.join("games-publish.json")).unwrap();
    let with_page = manifest.replace(r#""arguments":[]}"#, r#""arguments":[],"page":"pages/demo"}"#);
    assert_ne!(with_page, manifest);
    write(&root.join("games-publish.json"), &with_page);
    write(&root.join("pages/demo/wiki.json"), r#"{"tagline": "a demo"}"#);
    fs::write(root.join("pages/demo/g1.jpg"), b"\xff\xd8\xffpicture").unwrap();
    fs::write(root.join("pages/demo/thumb.png"), b"\x89PNG\r\n\x1a\nthumb").unwrap();

    let out = std::env::temp_dir().join(format!("re2_publish_page_out_{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    // The Games repository's page rules: a page needs a tagline. And a page made there by hand for another game.
    write(&out.join("site/wiki.py"), "def problems(data, author, folder):\n    return [] if data.get('tagline') else ['`tagline` is missing']\n");
    write(&out.join("site/wiki/hand-made/wiki.json"), "made by hand");
    write(&out.join("site/thumbs/hand-made.png"), "made by hand");

    export(py, &root, &out);
    let after = tree(&out);
    let has = |lines: &[String], needle: &str| lines.iter().any(|l| l.starts_with(needle));
    assert!(
        has(&after, r#"site/wiki/demo/wiki.json={"tagline": "a demo"}"#) && has(&after, "site/wiki/demo/g1.jpg") && has(&after, "site/thumbs/demo.png"),
        "{after:?}"
    );
    assert!(!has(&after, "site/wiki/demo/thumb.png"), "the thumbnail goes to site/thumbs only: {after:?}");
    assert!(has(&after, "site/wiki/hand-made/wiki.json=made by hand") && has(&after, "site/thumbs/hand-made.png=made by hand"), "{after:?}");

    // A page the site would refuse is not published, and the export says why.
    write(&root.join("pages/demo/wiki.json"), r#"{"tagline": ""}"#);
    let (ok, err) = export_status(py, &root, &out);
    assert!(!ok && err.contains("`tagline` is missing") && err.contains("pages/demo/wiki.json"), "{err}");
    assert_eq!(tree(&out), after, "nothing was written");

    // A file a page may not hold is refused.
    write(&root.join("pages/demo/wiki.json"), r#"{"tagline": "a demo"}"#);
    write(&root.join("pages/demo/notes.txt"), "scratch");
    let (ok, err) = export_status(py, &root, &out);
    assert!(!ok && err.contains("notes.txt"), "{err}");
    fs::remove_file(root.join("pages/demo/notes.txt")).unwrap();

    // The manifest stops naming the page: what it published is removed, the hand-made page stays.
    write(&root.join("games-publish.json"), &manifest);
    export(py, &root, &out);
    let later = tree(&out);
    assert!(!has(&later, "site/wiki/demo/") && !has(&later, "site/thumbs/demo.png") && !out.join("site/wiki/demo").exists(), "{later:?}");
    assert!(has(&later, "site/wiki/hand-made/wiki.json=made by hand") && has(&later, "site/thumbs/hand-made.png=made by hand"), "{later:?}");
    for d in [&root, &out] {
        let _ = fs::remove_dir_all(d);
    }
}
