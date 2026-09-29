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

#[test]
fn the_server_listens_on_loopback_unless_told_otherwise() {
    // No --bind, no RED_BIND: an unattended run must never open a public socket (an OS firewall prompt would block it until a person clicks).
    let out = server()
        .args(["--map", concat!(env!("CARGO_MANIFEST_DIR"), "/examples/test_lab.json"), "--port", "0", "--stats-secs", "0", "--run-for", "0.3"])
        .output()
        .expect("run red_server");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("LISTENING 127.0.0.1:"), "loopback by default: {text}");
    assert!(text.contains("--public"), "the output says how to host for other machines: {text}");
}

#[test]
fn public_hosting_is_an_explicit_choice() {
    // A throwaway identity for this run (ADR 0044): no key is stored in the repository.
    let dir = std::env::temp_dir().join(format!("red-server-env-{}", red_engine2::crypto::random_u64().unwrap()));
    std::fs::create_dir_all(&dir).unwrap();
    let gen = red_engine2::net::quic::ServerIdentity::generate(&["localhost".into()]).unwrap();
    std::fs::write(dir.join("cert.pem"), &gen.cert_pem).unwrap();
    std::fs::write(dir.join("key.pem"), &gen.key_pem).unwrap();
    let (cert, key) = (dir.join("cert.pem").display().to_string(), dir.join("key.pem").display().to_string());
    let run = |args: &[&str], env_bind: Option<&str>| {
        let mut c = server();
        c.args(["--map", concat!(env!("CARGO_MANIFEST_DIR"), "/examples/test_lab.json"), "--port", "0", "--stats-secs", "0", "--run-for", "0.3"]).args(args);
        if let Some(b) = env_bind {
            c.env("RED_BIND", b);
        }
        let out = c.output().expect("run red_server");
        (out.status.success(), String::from_utf8_lossy(&out.stdout).to_string(), String::from_utf8_lossy(&out.stderr).to_string())
    };
    // Facing the network is explicit (--public or RED_BIND), and it is QUIC with an identity...
    for (args, env_bind) in [(vec!["--public", "--tls-cert", &cert, "--tls-key", &key], None), (vec!["--tls-cert", &cert, "--tls-key", &key], Some("0.0.0.0"))]
    {
        let (ok, text, err) = run(&args, env_bind);
        assert!(ok && text.contains("LISTENING 0.0.0.0:") && text.contains("transport: quic"), "{args:?} / {env_bind:?}: {text}{err}");
    }
    // ...or it refuses to start (fail closed), unless plaintext is explicitly accepted.
    let (ok, text, err) = run(&["--public"], None);
    assert!(!ok && !text.contains("LISTENING") && err.contains("refusing development UDP"), "{text}{err}");
    let (ok, text, _) = run(&["--public", "--insecure-public-udp"], None);
    assert!(ok && text.contains("LISTENING 0.0.0.0:") && text.contains("NOT encrypted"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}
