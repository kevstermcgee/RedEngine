//! Publishing into a RedEngineGames checkout must never touch work it does not own (docs/PUBLISHING_2D.md, "Git safety").
//!
//! Each test builds a throw-away repository with a bare `origin`, puts it in an awkward state (unrelated staged files, a half-staged file, another game's edits, leftovers of a failed
//! run, a rejected push), publishes browser games into it with synthetic packages (no WebAssembly or browser needed, so these always run) and then proves two things: the publication
//! commit holds exactly the game's files, and everything else in the checkout is byte-for-byte and index-for-index what it was.

use red_engine2::tools::publish2d::{self, Verification};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("re2_gitiso_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn git_try(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=dev", "-c", "user.email=dev@dev.invalid"]).args(args).output().expect("git");
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim_end().to_string())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    git_try(dir, args).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

/// A synthetic package: the files a publication copies, without a browser or a build.
fn package(dir: &Path, id: &str, build: &str, desc: &str) -> (PathBuf, Value) {
    let pkg = dir.join(format!("pkg_{id}_{build}"));
    std::fs::create_dir_all(pkg.join("assets")).unwrap();
    std::fs::write(pkg.join("index.html"), format!("<html>{id} {build}</html>")).unwrap();
    std::fs::write(pkg.join("thumbnail.png"), b"png").unwrap();
    std::fs::write(pkg.join("assets/game.json"), format!("{{\"id\":\"{id}\",\"build\":\"{build}\"}}")).unwrap();
    let manifest = json!({
        "schema": "red2d-web-package/1", "package_id": build, "entry": "index.html",
        "game": {"id": id, "title": id, "description": desc, "game_revision": "abc", "presentation": "2d", "platforms": ["web"], "networking": "offline", "input": ["keyboard"], "persistence": [], "screen": {"width": 320, "height": 180, "scale": "fit"}},
        "engine": {"revision": "deadbeef", "dirty": false},
        "compat": {"requires": ["WebAssembly"], "optional": [], "networking": "none"},
        "native": {"scenarios": [{"name": "a"}]},
        "files": [{"path": "index.html", "bytes": 1, "sha256": "x"}, {"path": "thumbnail.png", "bytes": 3, "sha256": "y"}, {"path": "assets/game.json", "bytes": 1, "sha256": "z"}],
    });
    std::fs::write(pkg.join("manifest.json"), manifest.to_string()).unwrap();
    (pkg, manifest)
}

fn ver(manifest: &Value) -> Verification {
    Verification {
        package_id: manifest["package_id"].as_str().unwrap().into(),
        ok: true,
        browser: "Chromium test".into(),
        checks: 3,
        ..Verification::default()
    }
}

struct Repo {
    root: PathBuf,
    origin: PathBuf,
    tmp: PathBuf,
}

/// A checkout with one published game (`first`), its own unrelated files committed, and a bare origin it is in sync with.
fn repo(name: &str) -> Repo {
    let tmp = scratch(name);
    let origin = tmp.join("origin.git");
    assert!(Command::new("git").args(["init", "-q", "--bare", "-b", "main"]).arg(&origin).status().unwrap().success());
    let root = tmp.join("clone");
    assert!(Command::new("git").args(["clone", "-q"]).arg(&origin).arg(&root).output().unwrap().status.success());
    git(&root, &["checkout", "-q", "-B", "main"]);
    std::fs::write(root.join("README.md"), "games\n").unwrap();
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(root.join("notes/todo.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "init"]);
    git(&root, &["push", "-q", "-u", "origin", "main"]);
    let r = Repo { root, origin, tmp };
    let (pkg, m) = package(&r.tmp, "first", "1111", "the first game");
    publish2d::github_upload(&r.root, &pkg, &ver(&m), 1_000, true).unwrap();
    r
}

/// Everything in a checkout a publication must not change: the index entries and the working files of every path outside `webgames/`, plus the staged diff of those.
fn unrelated_state(root: &Path) -> String {
    let mut out = String::new();
    out += &git(root, &["ls-files", "-s", "--", ".", ":(exclude)webgames"]);
    out += "\n--status--\n";
    out += &git(root, &["status", "--porcelain=v1", "-uall", "--", ".", ":(exclude)webgames"]);
    out += "\n--staged--\n";
    out += &git(root, &["diff", "--cached", "--", ".", ":(exclude)webgames"]);
    out += "\n--unstaged--\n";
    out += &git(root, &["diff", "--", ".", ":(exclude)webgames"]);
    for e in walk(root) {
        let rel = e.strip_prefix(root).unwrap().to_string_lossy().to_string();
        if rel.starts_with(".git") || rel.starts_with("webgames") {
            continue;
        }
        out += &format!("\n== {rel}: {:?}", std::fs::read(&e).unwrap());
    }
    out
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        if p.is_dir() {
            v.extend(walk(&p));
        } else {
            v.push(p);
        }
    }
    v.sort();
    v
}

/// The files the newest commit changed.
fn committed_files(root: &Path) -> Vec<String> {
    git(root, &["show", "--name-only", "--format=", "HEAD"]).lines().map(str::to_string).collect()
}

fn publish_second(r: &Repo, id: &str, build: &str, push: bool) -> Result<publish2d::GithubPublication, String> {
    let (pkg, m) = package(&r.tmp, id, build, "another");
    publish2d::github_upload(&r.root, &pkg, &ver(&m), 2_000, push)
}

fn only_game_files(files: &[String], id: &str) {
    assert!(!files.is_empty(), "the publication committed something");
    for f in files {
        assert!(
            f.starts_with(&format!("webgames/games/{id}/")) || f == "webgames/catalog.json" || f == "webgames/index.html",
            "the publication commit holds `{f}`, which is not this game's: {files:?}"
        );
    }
}

#[test]
fn a_clean_repository_gets_one_commit_with_only_the_game_and_ends_clean() {
    let r = repo("clean");
    let before = git(&r.root, &["rev-parse", "HEAD"]);
    let p = publish_second(&r, "second", "2222", true).unwrap();
    assert!(p.commit.is_some() && p.pushed);
    assert_eq!(git(&r.root, &["rev-parse", "HEAD~1"]), before, "exactly one commit on top");
    only_game_files(&committed_files(&r.root), "second");
    assert_eq!(git(&r.root, &["status", "--porcelain", "-uall"]), "", "the checkout is clean: the index and the working tree follow the commit");
    assert_eq!(git(&r.origin, &["rev-parse", "main"]), git(&r.root, &["rev-parse", "HEAD"]), "and it reached origin");
    assert!(git(&r.root, &["log", "-1", "--format=%s"]).starts_with("Browser game second: build 2222"));
    // A normal `git commit -a` / `git commit` afterwards must not undo the publication (the index is in step with HEAD).
    git(&r.root, &["commit", "-q", "--allow-empty", "-m", "later"]);
    assert!(r.root.join("webgames/games/second/manifest.json").is_file());
    assert_eq!(
        git(&r.root, &["ls-tree", "-r", "--name-only", "HEAD", "webgames/games/second"]).lines().count(),
        9,
        "the stable copy (manifest + 3 files), the build copy (the same), the record"
    );
}

#[test]
fn an_unrelated_unstaged_file_stays_unstaged_and_uncommitted() {
    let r = repo("unstaged");
    std::fs::write(r.root.join("README.md"), "games, edited\n").unwrap();
    std::fs::write(r.root.join("scratch.txt"), "untracked\n").unwrap();
    let before = unrelated_state(&r.root);
    publish_second(&r, "second", "2222", true).unwrap();
    only_game_files(&committed_files(&r.root), "second");
    assert_eq!(unrelated_state(&r.root), before);
    assert_eq!(git(&r.root, &["status", "--porcelain"]), " M README.md\n?? scratch.txt");
}

#[test]
fn an_unrelated_staged_file_is_not_in_the_publication_commit_and_stays_staged() {
    let r = repo("staged");
    std::fs::write(r.root.join("engine_notes.md"), "staged by another AI\n").unwrap();
    git(&r.root, &["add", "engine_notes.md"]);
    std::fs::write(r.root.join("README.md"), "games, staged edit\n").unwrap();
    git(&r.root, &["add", "README.md"]);
    let before = unrelated_state(&r.root);
    assert!(before.contains("A  engine_notes.md") && before.contains("M  README.md"), "{before}");
    publish_second(&r, "second", "2222", true).unwrap();
    let files = committed_files(&r.root);
    only_game_files(&files, "second");
    assert!(!files.iter().any(|f| f.contains("engine_notes") || f == "README.md"), "{files:?}");
    assert_eq!(unrelated_state(&r.root), before, "still staged, byte for byte");
    // What the other AI staged is still exactly what it will commit.
    assert_eq!(git(&r.root, &["diff", "--cached", "--name-only"]), "README.md\nengine_notes.md");
}

#[test]
fn a_partially_staged_file_keeps_its_staged_and_unstaged_halves() {
    let r = repo("partial");
    let todo = r.root.join("notes/todo.txt");
    std::fs::write(&todo, "ONE\ntwo\nthree\nfour\n").unwrap();
    git(&r.root, &["add", "notes/todo.txt"]);
    std::fs::write(&todo, "ONE\ntwo\nthree\nFOUR\n").unwrap();
    let before = unrelated_state(&r.root);
    assert!(before.contains("MM notes/todo.txt"), "{before}");
    publish_second(&r, "second", "2222", false).unwrap();
    only_game_files(&committed_files(&r.root), "second");
    assert_eq!(unrelated_state(&r.root), before);
    assert_eq!(git(&r.root, &["show", ":notes/todo.txt"]), "ONE\ntwo\nthree\nfour", "the staged half is intact");
    assert_eq!(std::fs::read_to_string(&todo).unwrap(), "ONE\ntwo\nthree\nFOUR\n", "the unstaged half is intact");
}

#[test]
fn several_games_published_in_a_row_each_get_their_own_commit_and_the_catalog_lists_all() {
    let r = repo("several");
    std::fs::write(r.root.join("wip.txt"), "someone's work\n").unwrap();
    git(&r.root, &["add", "wip.txt"]);
    let before = unrelated_state(&r.root);
    let base = git(&r.root, &["rev-parse", "HEAD"]);
    for (id, build) in [("second", "2222"), ("third", "3333"), ("fourth", "4444")] {
        publish_second(&r, id, build, id != "third").unwrap();
        only_game_files(&committed_files(&r.root), id);
    }
    assert_eq!(git(&r.root, &["rev-list", "--count", &format!("{base}..HEAD")]), "3", "one commit per game");
    assert_eq!(unrelated_state(&r.root), before);
    let catalog: Value = serde_json::from_str(&std::fs::read_to_string(r.root.join("webgames/catalog.json")).unwrap()).unwrap();
    let ids: Vec<&str> = catalog["games"].as_array().unwrap().iter().map(|g| g["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["first", "fourth", "second", "third"]);
    assert_eq!(git(&r.root, &["status", "--porcelain", "-uall"]), "A  wip.txt");
}

#[test]
fn another_games_uncommitted_edits_are_not_published_and_not_touched() {
    let r = repo("othergame");
    // Somebody is editing the already published game by hand, and has staged a new game's files.
    let other_record = r.root.join("webgames/games/first/game.json");
    std::fs::write(&other_record, "{\"id\":\"first\",\"title\":\"HALF EDITED\"}\n").unwrap();
    std::fs::create_dir_all(r.root.join("webgames/games/draft")).unwrap();
    std::fs::write(r.root.join("webgames/games/draft/game.json"), "{\"id\":\"draft\",\"title\":\"not ready\"}\n").unwrap();
    git(&r.root, &["add", "webgames/games/draft"]);
    let edited = std::fs::read(&other_record).unwrap();
    let staged_before = git(&r.root, &["diff", "--cached"]);
    publish_second(&r, "second", "2222", true).unwrap();
    let files = committed_files(&r.root);
    only_game_files(&files, "second");
    assert_eq!(std::fs::read(&other_record).unwrap(), edited, "the other game's edit is still in the working tree");
    assert_eq!(git(&r.root, &["diff", "--cached", "--", "webgames/games/draft"]), staged_before);
    let catalog = git(&r.root, &["show", "HEAD:webgames/catalog.json"]);
    assert!(
        !catalog.contains("HALF EDITED") && !catalog.contains("not ready") && !catalog.contains("draft"),
        "the committed catalog lists only committed games:\n{catalog}"
    );
    assert!(catalog.contains("\"second\"") && catalog.contains("\"first\""));
    assert!(git(&r.root, &["status", "--porcelain"]).contains(" M webgames/games/first/game.json"));
}

#[test]
fn leftovers_of_a_publication_that_stopped_halfway_are_cleared_when_identical_and_refused_when_not() {
    let r = repo("leftover");
    std::fs::write(r.root.join("unrelated.txt"), "keep\n").unwrap();
    git(&r.root, &["add", "unrelated.txt"]);
    // Halfway through an earlier run: the new game's own directory was written into the working tree (untracked, same timestamp, so the same bytes as the real run will
    // produce) and nothing was committed.
    let (pkg, m) = package(&r.tmp, "second", "2222", "another");
    let ghost = r.tmp.join("ghost");
    publish2d::write_site(&ghost, &pkg, &ver(&m), 2_000).unwrap();
    copy_dir(&ghost.join("games/second"), &r.root.join("webgames/games/second"));
    let before_unrelated = unrelated_state(&r.root);
    publish_second(&r, "second", "2222", true).expect("identical leftovers are cleared and the publication succeeds");
    only_game_files(&committed_files(&r.root), "second");
    assert_eq!(unrelated_state(&r.root), before_unrelated);
    assert_eq!(git(&r.root, &["status", "--porcelain", "-uall"]), "A  unrelated.txt");
    // A leftover that is NOT what would be published stops the run, names the file and the way out, and changes nothing.
    let stale = r.root.join("webgames/games/third/index.html");
    std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
    std::fs::write(&stale, "an older, different build\n").unwrap();
    let head = git(&r.root, &["rev-parse", "HEAD"]);
    let status = git(&r.root, &["status", "--porcelain", "-uall"]);
    let e = publish_second(&r, "third", "3333", true).unwrap_err();
    assert!(e.contains("webgames/games/third/index.html") && e.contains("untracked") && e.contains("git -C") && e.contains("nothing was changed"), "{e}");
    assert_eq!(git(&r.root, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&r.root, &["status", "--porcelain", "-uall"]), status);
    assert_eq!(std::fs::read_to_string(&stale).unwrap(), "an older, different build\n");
    assert_eq!(git(&r.origin, &["rev-parse", "main"]), head, "nothing was pushed");
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        if e.path().is_dir() {
            copy_dir(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

#[test]
fn a_failed_push_takes_the_commit_back_and_leaves_everything_as_it_was() {
    let r = repo("failpush");
    std::fs::write(r.root.join("README.md"), "dirty\n").unwrap();
    std::fs::write(r.root.join("staged.txt"), "staged\n").unwrap();
    git(&r.root, &["add", "staged.txt"]);
    // origin rejects every push.
    let hook = r.origin.join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'rejected by test' >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let head = git(&r.root, &["rev-parse", "HEAD"]);
    let before = unrelated_state(&r.root);
    let webgames_before = git(&r.root, &["ls-files", "-s", "webgames"]);
    let e = publish_second(&r, "second", "2222", true).unwrap_err();
    assert!(e.contains("rejected by test") && e.contains("nothing was published") && e.contains("taken back out"), "{e}");
    assert_eq!(git(&r.root, &["rev-parse", "HEAD"]), head, "the branch is where it was");
    assert_eq!(unrelated_state(&r.root), before);
    assert_eq!(git(&r.root, &["ls-files", "-s", "webgames"]), webgames_before, "the index is as it was");
    assert!(!r.root.join("webgames/games/second").exists(), "the working tree has no half-published game");
    // And the next attempt, with the push working again, just succeeds.
    std::fs::remove_file(&hook).unwrap();
    publish_second(&r, "second", "2222", true).unwrap();
    assert_eq!(git(&r.origin, &["rev-parse", "main"]), git(&r.root, &["rev-parse", "HEAD"]));
    assert_eq!(unrelated_state(&r.root), before);
}

#[test]
fn staged_changes_inside_this_games_own_files_stop_the_run_with_the_recovery_command() {
    let r = repo("ownstaged");
    let record = r.root.join("webgames/games/first/game.json");
    std::fs::write(&record, "{\"hand\":\"edited\"}\n").unwrap();
    git(&r.root, &["add", "webgames/games/first/game.json"]);
    let (pkg, m) = package(&r.tmp, "first", "1112", "an update of the first game");
    let head = git(&r.root, &["rev-parse", "HEAD"]);
    let e = publish2d::github_upload(&r.root, &pkg, &ver(&m), 3_000, false).unwrap_err();
    assert!(
        e.contains("webgames/games/first/game.json") && e.contains("staged with other content") && e.contains("git -C") && e.contains("nothing was changed"),
        "{e}"
    );
    assert_eq!(git(&r.root, &["rev-parse", "HEAD"]), head);
    assert_eq!(std::fs::read_to_string(&record).unwrap(), "{\"hand\":\"edited\"}\n");
    assert_eq!(git(&r.root, &["diff", "--cached", "--name-only"]), "webgames/games/first/game.json");
}

#[test]
fn unsafe_repository_states_are_refused_before_anything_is_written() {
    // Unpushed commits would be published along with the game.
    let r = repo("unpushed");
    std::fs::write(r.root.join("local.txt"), "x\n").unwrap();
    git(&r.root, &["add", "local.txt"]);
    git(&r.root, &["commit", "-q", "-m", "a local commit nobody reviewed"]);
    let head = git(&r.root, &["rev-parse", "HEAD"]);
    let e = publish_second(&r, "second", "2222", true).unwrap_err();
    assert!(e.contains("1 commit(s)") && e.contains("origin/main") && e.contains("without --push") && e.contains("Nothing was changed"), "{e}");
    assert_eq!(git(&r.root, &["rev-parse", "HEAD"]), head);
    assert!(!r.root.join("webgames/games/second").exists());
    // Without --push the same state is fine: a local commit on top of it, the unpushed one untouched.
    publish_second(&r, "second", "2222", false).unwrap();
    assert_eq!(git(&r.root, &["rev-parse", "HEAD~1"]), head);
    // A detached HEAD, a merge in progress, a repository without a commit.
    let r = repo("detached");
    git(&r.root, &["checkout", "-q", "--detach"]);
    let e = publish_second(&r, "second", "2222", false).unwrap_err();
    assert!(e.contains("detached HEAD") && e.contains("switch main"), "{e}");
    let r = repo("merging");
    std::fs::write(r.root.join(".git/MERGE_HEAD"), git(&r.root, &["rev-parse", "HEAD"])).unwrap();
    let e = publish_second(&r, "second", "2222", false).unwrap_err();
    assert!(e.contains("middle of a merge") && e.contains("abort"), "{e}");
    let empty = scratch("empty");
    assert!(Command::new("git").args(["init", "-q"]).arg(&empty).status().unwrap().success());
    let (pkg, m) = package(&empty, "second", "2222", "x");
    let e = publish2d::github_upload(&empty, &pkg, &ver(&m), 1, false).unwrap_err();
    assert!(e.contains("no commit yet") && e.contains("--allow-empty"), "{e}");
}

#[test]
fn publishing_the_same_build_twice_commits_nothing_the_second_time() {
    let r = repo("idempotent");
    publish_second(&r, "second", "2222", true).unwrap();
    let head = git(&r.root, &["rev-parse", "HEAD"]);
    let again = publish_second(&r, "second", "2222", true).unwrap();
    assert!(again.commit.is_none() && again.changed.is_empty());
    assert_eq!(git(&r.root, &["rev-parse", "HEAD"]), head);
}

// ---- a publication that was interrupted, and one that is stopped by the machine ------------------------------------------------------------------------

#[test]
fn a_publication_committed_but_not_pushed_is_pushed_by_the_next_run_with_the_new_game() {
    let r = repo("interrupted");
    std::fs::write(r.root.join("elsewhere.txt"), "somebody's staged work\n").unwrap();
    git(&r.root, &["add", "elsewhere.txt"]);
    let before = unrelated_state(&r.root);
    // The first run committed and was killed before (or did not ask for) the push.
    publish_second(&r, "second", "2222", false).unwrap();
    assert_ne!(git(&r.origin, &["rev-parse", "main"]), git(&r.root, &["rev-parse", "HEAD"]));
    // The next run, with --push, recognises that commit as a publication, publishes the new game and sends both.
    publish_second(&r, "third", "3333", true).unwrap();
    assert_eq!(git(&r.origin, &["rev-parse", "main"]), git(&r.root, &["rev-parse", "HEAD"]), "everything is on origin");
    let listing = git(&r.origin, &["ls-tree", "-r", "--name-only", "main"]);
    assert!(listing.contains("webgames/games/second/") && listing.contains("webgames/games/third/") && !listing.contains("elsewhere"), "{listing}");
    assert_eq!(unrelated_state(&r.root), before, "the staged file is still only staged");
}

#[test]
fn another_git_process_holding_the_index_stops_the_run_cleanly_and_the_retry_works() {
    let r = repo("locked");
    let head = git(&r.root, &["rev-parse", "HEAD"]);
    let before = unrelated_state(&r.root);
    std::fs::write(r.root.join(".git/index.lock"), "").unwrap();
    let e = publish_second(&r, "second", "2222", true).unwrap_err();
    assert!(e.contains("index.lock") && e.contains("Nothing was committed and nothing was changed"), "{e}");
    assert_eq!(git(&r.root, &["rev-parse", "HEAD"]), head);
    assert!(!r.root.join("webgames/games/second").exists(), "no half-published game in the working tree");
    std::fs::remove_file(r.root.join(".git/index.lock")).unwrap();
    assert_eq!(unrelated_state(&r.root), before);
    publish_second(&r, "second", "2222", true).unwrap();
    assert_eq!(git(&r.origin, &["rev-parse", "main"]), git(&r.root, &["rev-parse", "HEAD"]));
}

/// Sets a directory's modification time. Windows refuses `File::open(dir)` + `set_modified` ("Access is denied"): a directory handle needs backup semantics and write access.
fn set_dir_modified(dir: &std::path::Path, when: std::time::SystemTime) {
    #[cfg(windows)]
    let f = {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        std::fs::OpenOptions::new().write(true).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(dir).unwrap()
    };
    #[cfg(not(windows))]
    let f = std::fs::File::open(dir).unwrap();
    f.set_modified(when).unwrap();
}

#[test]
fn scratch_left_by_a_crashed_run_is_swept_and_never_shows_in_git_status() {
    let r = repo("crashed");
    let old = r.root.join(".git/redengine-publish-1-1");
    std::fs::create_dir_all(old.join("work/webgames")).unwrap();
    std::fs::write(old.join("work/webgames/leftover.txt"), "x").unwrap();
    // Make it look an hour and a half old.
    set_dir_modified(&old, std::time::SystemTime::now() - std::time::Duration::from_secs(5400));
    assert_eq!(git(&r.root, &["status", "--porcelain", "-uall"]), "", "scratch lives inside .git: status never sees it");
    publish_second(&r, "second", "2222", false).unwrap();
    assert!(!old.exists(), "the stale scratch of a crashed run was removed");
    assert!(
        std::fs::read_dir(r.root.join(".git")).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with("redengine-publish-")),
        "and this run left none"
    );
    assert_eq!(git(&r.root, &["status", "--porcelain", "-uall"]), "");
}

#[cfg(unix)]
#[test]
fn a_branch_that_refuses_to_move_leaves_the_checkout_as_it_was() {
    use std::os::unix::fs::PermissionsExt;
    let r = repo("refused");
    std::fs::write(r.root.join("README.md"), "dirty\n").unwrap();
    let head = git(&r.root, &["rev-parse", "HEAD"]);
    let before = unrelated_state(&r.root);
    let webgames_before = git(&r.root, &["ls-files", "-s", "webgames"]);
    // The ref update is vetoed (what would happen if somebody else's commit had moved the branch in the meantime).
    let hook = r.root.join(".git/hooks/reference-transaction");
    std::fs::write(&hook, "#!/bin/sh\n[ \"$1\" = prepared ] && { echo 'vetoed by the test' >&2; exit 1; }\nexit 0\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let e = publish_second(&r, "second", "2222", false).unwrap_err();
    assert!(e.contains("moved while publishing") && e.contains("nothing was committed"), "{e}");
    std::fs::remove_file(&hook).unwrap();
    assert_eq!(git(&r.root, &["rev-parse", "HEAD"]), head);
    assert_eq!(unrelated_state(&r.root), before);
    assert_eq!(git(&r.root, &["ls-files", "-s", "webgames"]), webgames_before, "the index follows HEAD again");
    assert!(!r.root.join("webgames/games/second").exists(), "the working tree has no half-published game");
    publish_second(&r, "second", "2222", false).unwrap();
    only_game_files(&committed_files(&r.root), "second");
}
