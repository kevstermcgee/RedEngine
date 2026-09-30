//! `red_engine2 servers` through the real binary. **Read-only and non-destructive by design**: it lists, and it asks to act on names and pids that do not exist (which must be refused
//! before anything is sent to systemd or a process), so it can never start, stop or signal a real game server on the machine running the tests. The logic with a fake machine is
//! tested in `tools::servers`; this proves the command is wired, prints JSON under `--json`, and fails cleanly. It is skipped where there is no systemd user session (CI runners, macOS).

use std::process::Command;

fn systemd_user_available() -> bool {
    Command::new("systemctl").args(["--user", "show-environment"]).output().map(|o| o.status.success()).unwrap_or(false)
}

fn run(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_red_engine2")).args(args).output().expect("the binary runs");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
}

#[test]
fn listing_works_and_json_has_the_documented_shape() {
    if !systemd_user_available() {
        eprintln!("skipped: no systemd user session");
        return;
    }
    let (ok, text, err) = run(&["servers"]);
    assert!(ok, "{err}");
    assert!(text.starts_with("NAME") || text.contains("no Red game servers found"), "{text}");
    let (ok, json, err) = run(&["--json", "servers"]);
    assert!(ok, "{err}");
    let v: serde_json::Value = serde_json::from_str(&json).expect("--json prints JSON");
    let data = v.get("data").unwrap_or(&v);
    assert!(data["servers"].is_array() && data["unmanaged"].is_array(), "{json}");
    for s in data["servers"].as_array().unwrap() {
        for key in ["name", "unit", "state", "port", "enabled"] {
            assert!(s.get(key).is_some(), "a server entry has `{key}`: {s}");
        }
        assert!(!s.to_string().to_ascii_lowercase().contains("join_key"), "no secret names in the output");
    }
}

#[test]
fn acting_on_something_that_is_not_a_game_server_is_refused_without_touching_anything() {
    if !systemd_user_available() {
        eprintln!("skipped: no systemd user session");
        return;
    }
    for args in [
        vec!["servers", "start", "definitely-not-a-server-xyz"],
        vec!["servers", "stop", "definitely-not-a-server-xyz", "--yes"],
        vec!["servers", "restart", "definitely-not-a-server-xyz", "--yes"],
        vec!["servers", "status", "definitely-not-a-server-xyz"],
        vec!["servers", "logs", "definitely-not-a-server-xyz"],
    ] {
        let (ok, _, err) = run(&args);
        assert!(!ok, "{args:?} must fail");
        assert!(err.contains("no game server called"), "{args:?}: {err}");
    }
    // pid 1 is not an unmanaged red_server: never signalled, even with --yes.
    let (ok, _, err) = run(&["servers", "stop", "--pid", "1", "--yes"]);
    assert!(!ok && err.contains("not an unmanaged red_server"), "{err}");
    // A stop with neither a name nor a pid is a usage error.
    let (ok, _, _) = run(&["servers", "stop"]);
    assert!(!ok);
}
