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
