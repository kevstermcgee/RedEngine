//! `game upgrade plan`/`game upgrade verify` (`src/tools/upgrade.rs`): planning is read-only, candidate
//! regeneration never touches the real project, conflicts are surfaced rather than discarded, and
//! insufficient evidence never produces a passing result. Each test drives the real CLI binary against
//! fixtures scaffolded in a temp directory (pattern from `tests/verify_map.rs`); nothing under this
//! checkout is modified and nothing is pushed.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("re2_game_upgrade_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_red_engine2")).args(args).output().expect("start red_engine2")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn git(cwd: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args);
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn sha(cwd: &Path, rev: &str) -> String {
    let out = Command::new("git").current_dir(cwd).args(["rev-parse", rev]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A small synthetic git checkout standing in for "the engine": two commits, with a `PROTOCOL_VERSION`
/// bump and a `checks` `GROUPS` growth between them, at the real paths our migration detectors read —
/// enough for `resolve_target`/`git_show` without a real multi-minute engine clone+build. Returns
/// `(dir, old_sha, new_sha)`.
fn fake_engine_repo(name: &str) -> (PathBuf, String, String) {
    let dir = scratch(&format!("engine_{name}"));
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(dir.join("src/net")).unwrap();
    std::fs::create_dir_all(dir.join("src/tools")).unwrap();
    // Content includes `name` so distinct fixtures never collide on commit hash (git commits are content-addressed:
    // identical tree + author + committer + message + timestamp would otherwise hash identically across repos).
    std::fs::write(dir.join("MARKER"), name).unwrap();
    std::fs::write(dir.join("src/net/protocol.rs"), "pub const PROTOCOL_VERSION: u16 = 8;\n").unwrap();
    std::fs::write(dir.join("src/tools/verify.rs"), "const GROUPS: [&str; 6] = [\"lint\", \"reach\", \"walk\", \"objects\", \"views\", \"sim\"];\n").unwrap();
    std::fs::write(dir.join("Cargo.lock"), "# lock v1\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "old"]);
    let old = sha(&dir, "HEAD");
    std::fs::write(dir.join("src/net/protocol.rs"), "pub const PROTOCOL_VERSION: u16 = 14;\n").unwrap();
    std::fs::write(
        dir.join("src/tools/verify.rs"),
        "const GROUPS: [&str; 8] = [\"lint\", \"reach\", \"walk\", \"objects\", \"views\", \"sim\", \"perf\", \"nav\"];\n",
    )
    .unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "new"]);
    let new = sha(&dir, "HEAD");
    (dir, old, new)
}

/// A fresh game project scaffolded by the real `new-game` command, its `engine.path` pointed at `engine_dir`.
fn scaffold_game(name: &str, engine_dir: &Path) -> PathBuf {
    let dir = scratch(name);
    let _ = std::fs::remove_dir_all(&dir);
    let out = cli(&["new-game", dir.to_str().unwrap(), "--name", name, "--engine-path", engine_dir.to_str().unwrap()]);
    assert!(out.status.success(), "new-game: {}", text(&out));
    dir
}

fn snapshot(dir: &Path, rel: &[&str]) -> Vec<(String, Vec<u8>)> {
    rel.iter().map(|r| (r.to_string(), std::fs::read(dir.join(r)).unwrap_or_default())).collect()
}

#[test]
fn plan_is_read_only_on_a_standard_blueprint_project() {
    let (engine, _old, _new) = fake_engine_repo("standard");
    let game = scaffold_game("standard", &engine);
    let before = snapshot(&game, &["game.json", "maps/main.json", "blueprints/main.blueprint.json"]);
    let head_before = sha(&engine, "HEAD");

    let out = cli(&["game", "upgrade", "plan", "--dir", game.to_str().unwrap(), "--to", "HEAD", "--engine", engine.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out));
    let rendered = text(&out);
    assert!(rendered.contains("StandardBlueprint"), "{rendered}");
    assert!(!rendered.contains("uncertainties"), "a fresh scaffold has no pre-existing drift: {rendered}");

    let after = snapshot(&game, &["game.json", "maps/main.json", "blueprints/main.blueprint.json"]);
    assert_eq!(before, after, "plan must never edit the project's own files");
    assert_eq!(sha(&engine, "HEAD"), head_before, "plan must never switch the engine checkout it reads from");
}

#[test]
fn plan_surfaces_hand_edited_map_drift_as_an_uncertainty_not_authorization_to_replace() {
    let (engine, _old, new) = fake_engine_repo("handedit");
    let game = scaffold_game("handedit", &engine);
    let map = game.join("maps/main.json");
    let text_before = std::fs::read_to_string(&map).unwrap();
    std::fs::write(&map, text_before.replace("\"height\": 2.8", "\"height\": 3.1")).unwrap();
    let hand_edited = std::fs::read(&map).unwrap();

    let out = cli(&["game", "upgrade", "plan", "--dir", game.to_str().unwrap(), "--to", &new, "--engine", engine.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out));
    let rendered = text(&out);
    assert!(rendered.contains("pre-existing drift"), "{rendered}");
    assert!(!rendered.to_lowercase().contains("clean\n"), "a drifted map must never be called clean: {rendered}");
    assert_eq!(std::fs::read(&map).unwrap(), hand_edited, "plan must never touch the hand-edited map");
}

#[test]
fn plan_reports_uncertainty_when_old_generation_is_unrecoverable() {
    let (engine, _old, new) = fake_engine_repo("nogeneration");
    let game = scaffold_game("nogeneration", &engine);
    std::fs::remove_file(game.join("blueprints/main.blueprint.json")).unwrap();
    // game.json still lists the deleted blueprint; the map is now the only source of truth.
    let out = cli(&["game", "upgrade", "plan", "--dir", game.to_str().unwrap(), "--to", &new, "--engine", engine.to_str().unwrap()]);
    let rendered = text(&out);
    // Either the drift check already names the missing blueprint, or classification could not proceed cleanly —
    // either way this must never read as an authorized, uncertainty-free "clean" upgrade.
    assert!(
        rendered.contains("missing") || rendered.contains("uncertain") || rendered.contains("uncertainties") || !out.status.success(),
        "a project whose generation inputs are gone must not look clean: {rendered}"
    );
}

#[test]
fn classify_flags_a_custom_rust_client_with_a_conflicting_engine_reference() {
    let (engine_a, _old_a, new_a) = fake_engine_repo("client_engine_a");
    let (engine_b, _old_b, _new_b) = fake_engine_repo("client_engine_b");
    let game = scaffold_game("customclient", &engine_a);
    std::fs::create_dir_all(game.join("client")).unwrap();
    std::fs::write(
        game.join("client/Cargo.toml"),
        format!("[package]\nname = \"customclient\"\nversion = \"0.1.0\"\n[dependencies]\nred_engine2 = {{ path = \"{}\" }}\n", engine_b.display()),
    )
    .unwrap();

    // The project's own engine (game.json, engine_a) and its Cargo dependency (engine_b) are two different
    // checkouts: they must disagree on commit even though both repos follow the same old->new content shape.
    let out = cli(&["game", "upgrade", "plan", "--dir", game.to_str().unwrap(), "--to", &new_a, "--engine", engine_a.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out));
    let rendered = text(&out);
    assert!(rendered.contains("CustomRustClient"), "{rendered}");
    assert!(rendered.contains("different commit") && rendered.contains("required_repair"), "{rendered}");
}

#[test]
fn resolving_a_moving_branch_pins_one_sha_and_a_dirty_target_is_refused() {
    let (engine, old, new) = fake_engine_repo("moving");
    let game = scaffold_game("moving", &engine);

    // A branch resolves to today's tip, stored as a fixed sha in the packet.
    let out = cli(&["game", "upgrade", "plan", "--dir", game.to_str().unwrap(), "--to", "main", "--engine", engine.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains(&new), "the branch's current tip must be the resolved target: {}", text(&out));
    assert_ne!(old, new);

    // Dirty engine checkout: refused by name, not silently resolved.
    std::fs::write(engine.join("src/net/protocol.rs"), "pub const PROTOCOL_VERSION: u16 = 99;\n").unwrap();
    let out = cli(&["game", "upgrade", "plan", "--dir", game.to_str().unwrap(), "--to", "main", "--engine", engine.to_str().unwrap()]);
    assert!(!out.status.success(), "a dirty engine checkout must be refused: {}", text(&out));
    assert!(text(&out).contains("uncommitted"), "{}", text(&out));
}

#[test]
fn stale_check_cache_does_not_let_verify_bless_a_changed_map() {
    let (engine, _old, new) = fake_engine_repo("stalecache");
    let game = scaffold_game("stalecache", &engine);

    // Populate game.rs's own CheckCache with a passing result.
    let out = cli(&["game", "check", "--dir", game.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(game.join("out/cache").is_dir(), "game check should have written its cache");

    // Change the map's bytes without touching the blueprint: a cached "passed" must not survive this.
    let map = game.join("maps/main.json");
    std::fs::write(&map, std::fs::read_to_string(&map).unwrap().replace("\"height\": 2.8", "\"height\": 3.1")).unwrap();

    let packet = game.join("out/upgrade/packet.json");
    let plan_out = cli(&[
        "game",
        "upgrade",
        "plan",
        "--dir",
        game.to_str().unwrap(),
        "--to",
        &new,
        "--engine",
        engine.to_str().unwrap(),
        "--out",
        packet.to_str().unwrap(),
    ]);
    assert!(plan_out.status.success(), "{}", text(&plan_out));

    let verify_out = cli(&["game", "upgrade", "verify", packet.to_str().unwrap(), "--dir", game.to_str().unwrap(), "--only", "baseline"]);
    assert!(!verify_out.status.success(), "a stale cache must not make a changed map look healthy: {}", text(&verify_out));
    assert!(text(&verify_out).contains("STALE") || text(&verify_out).to_lowercase().contains("differs"), "{}", text(&verify_out));
}

#[test]
fn an_unknown_only_stage_fails_clearly_instead_of_running_everything() {
    let (engine, _old, new) = fake_engine_repo("unknownonly");
    let game = scaffold_game("unknownonly", &engine);
    let packet = game.join("out/upgrade/packet.json");
    let plan_out = cli(&[
        "game",
        "upgrade",
        "plan",
        "--dir",
        game.to_str().unwrap(),
        "--to",
        &new,
        "--engine",
        engine.to_str().unwrap(),
        "--out",
        packet.to_str().unwrap(),
    ]);
    assert!(plan_out.status.success(), "{}", text(&plan_out));

    // A plausible typo for "baseline": at the review anchor this silently ran the full pipeline instead of
    // failing, which would also have tried (and failed) to build this fixture's non-buildable fake engine.
    let verify_out = cli(&["game", "upgrade", "verify", packet.to_str().unwrap(), "--dir", game.to_str().unwrap(), "--only", "basline"]);
    assert!(!verify_out.status.success(), "{}", text(&verify_out));
    let rendered = text(&verify_out);
    assert!(rendered.contains("unknown") && rendered.contains("basline"), "{rendered}");
    assert!(!rendered.contains("[ok]") && !rendered.contains("[FAIL]"), "no stage actually ran: {rendered}");
}

#[test]
fn verify_refuses_a_packet_whose_baseline_no_longer_matches_the_project() {
    let (engine, old, new) = fake_engine_repo("staleverify");
    let game = scaffold_game("staleverify", &engine);
    let packet = game.join("out/upgrade/packet.json");
    // Plan while the engine checkout is at `new` (where `scaffold_game` leaves it).
    let plan_out = cli(&[
        "game",
        "upgrade",
        "plan",
        "--dir",
        game.to_str().unwrap(),
        "--to",
        &new,
        "--engine",
        engine.to_str().unwrap(),
        "--out",
        packet.to_str().unwrap(),
    ]);
    assert!(plan_out.status.success(), "{}", text(&plan_out));
    assert!(text(&plan_out).contains(&new[..12]), "the packet must record `new` as the baseline: {}", text(&plan_out));

    // The project's own engine checkout moves (exactly what a background `git pull` on a tracked branch would
    // do) between planning and verifying: its recorded baseline no longer matches what the packet was planned
    // against, so every migration verdict in it was computed against a project state that no longer exists.
    git(&engine, &["checkout", "-q", "--detach", &old]);

    let verify_out = cli(&["game", "upgrade", "verify", packet.to_str().unwrap(), "--dir", game.to_str().unwrap(), "--only", "baseline"]);
    assert!(!verify_out.status.success(), "a moved baseline must be refused, not silently verified: {}", text(&verify_out));
    let rendered = text(&verify_out).to_lowercase();
    assert!(rendered.contains("stale") && rendered.contains("plan") && rendered.contains("again"), "{}", text(&verify_out));
}

#[test]
fn an_unreachable_migration_id_is_possible_not_silently_certified_through_the_cli() {
    // docs/upgrade-migrations.json ships with real ids; confirm `plan`'s rendered packet never calls an
    // id "applicable" without a real detector backing it when the evidence cannot be read (unresolved baseline).
    let (engine, _old, new) = fake_engine_repo("unresolved_baseline");
    let game = scaffold_game("unresolved_baseline", &engine);
    // engine.path points somewhere nonexistent so the *current* (baseline) engine cannot be resolved to a sha.
    let game_json = game.join("game.json");
    let text_before = std::fs::read_to_string(&game_json).unwrap();
    std::fs::write(&game_json, text_before.replace(&engine.display().to_string(), "/nonexistent/engine/path")).unwrap();

    let out = cli(&["game", "upgrade", "plan", "--dir", game.to_str().unwrap(), "--to", &new, "--engine", engine.to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out));
    let rendered = text(&out);
    assert!(rendered.contains("unresolved") || rendered.contains("possible:"), "{rendered}");
    assert!(!rendered.contains("[required] protocol-version-lockstep (applicable)"), "no baseline sha means this cannot be certified applicable: {rendered}");
}

#[test]
fn requested_repair_is_confirmed_broken_then_confirmed_fixed() {
    // The exact function `game upgrade verify`'s baseline stage calls (`game::check`) must tell "still broken"
    // from "repaired" for a scripted gameplay regression, which is what proves a requested fix actually landed.
    use red_engine2::tools::blueprint;
    let (engine, _old, _new) = fake_engine_repo("tdd");
    let game = scaffold_game("tdd", &engine);
    let bp_path = game.join("blueprints/main.blueprint.json");
    let mut bp: Value = serde_json::from_str(&std::fs::read_to_string(&bp_path).unwrap()).unwrap();
    bp["scene"] = serde_json::json!({"checks": {"sim": [{
        "name": "a deliberately failing scenario",
        "spawn_group": "duel",
        "players": [{"id": "p1", "character": "human", "spawn": "spawn_hall_1"}],
        "script": [],
        "expect": [{"event": "this_never_happens", "count": 1}]
    }]}});
    std::fs::write(&bp_path, serde_json::to_string_pretty(&bp).unwrap()).unwrap();
    let built = blueprint::compile_in(&bp, bp_path.parent()).expect("blueprint compiles");
    std::fs::write(game.join("maps/main.json"), &built.scene_text).unwrap();

    let cfg = red_engine2::tools::game::load(&game).unwrap();
    let broken = red_engine2::tools::game::check(&cfg, false);
    assert!(broken.failed() > 0, "the planted scenario must fail before any repair: {}", broken.render());

    // The repair: make the expectation satisfiable.
    bp["scene"]["checks"]["sim"][0]["expect"] = serde_json::json!([{"event": "this_never_happens", "count": 0}]);
    std::fs::write(&bp_path, serde_json::to_string_pretty(&bp).unwrap()).unwrap();
    let built = blueprint::compile_in(&bp, bp_path.parent()).expect("blueprint compiles");
    std::fs::write(game.join("maps/main.json"), &built.scene_text).unwrap();
    let fixed = red_engine2::tools::game::check(&cfg, false);
    assert_eq!(fixed.failed(), 0, "after the repair the same check must pass: {}", fixed.render());
}
