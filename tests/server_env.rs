//! `red_server` is configured entirely from the environment when run in a container or under a process manager
//! (`RED_MAP`, `RED_PORT`, `RED_BIND`, ...), and refuses malformed values instead of silently using defaults.

use std::process::{Command, Stdio};

fn server() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_red_server"));
    c.env_remove("RED_MAP").env_remove("RED_PORT").env_remove("RED_BIND").env_remove("RED_SPAWN_GROUP").stdout(Stdio::piped()).stderr(Stdio::piped());
    c
}

#[test]
fn everything_can_come_from_the_environment() {
    let out = server()
        .env("RED_MAP", concat!(env!("CARGO_MANIFEST_DIR"), "/examples/test_lab.json"))
        .env("RED_PORT", "0")
        .env("RED_BIND", "127.0.0.1")
        .env("RED_SPAWN_GROUP", "duel")
        .env("RED_STATS_SECS", "0")
        .env("RED_RUN_FOR", "0.5")
        .output()
        .expect("run red_server");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("LISTENING 127.0.0.1:"), "bound where RED_BIND said: {text}");
    assert!(text.contains("stopped:"), "a clean shutdown line: {text}");
}

#[test]
fn a_flag_beats_the_environment_and_a_bad_value_is_an_error() {
    let out = server().env("RED_PORT", "not-a-port").output().expect("run red_server");
    assert_eq!(out.status.code(), Some(2), "malformed RED_PORT must fail loudly");
    assert!(String::from_utf8_lossy(&out.stderr).contains("RED_PORT"), "{}", String::from_utf8_lossy(&out.stderr));

    let out = server()
        .env("RED_MAP", "/definitely/not/a/map.json")
        .args([
            "--map",
            concat!(env!("CARGO_MANIFEST_DIR"), "/examples/test_lab.json"),
            "--port",
            "0",
            "--bind",
            "127.0.0.1",
            "--stats-secs",
            "0",
            "--run-for",
            "0.3",
        ])
        .output()
        .expect("run red_server");
    assert!(out.status.success(), "--map must override RED_MAP: {}", String::from_utf8_lossy(&out.stderr));
}
