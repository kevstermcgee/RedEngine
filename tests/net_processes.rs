//! Multiplayer across **separate OS processes**: one `red_server` and two `red_bot` clients, each its
//! own process with its own socket, talking over loopback UDP. Everything the in-process tests prove
//! (`tests/net_e2e.rs`), but with nothing shared except the wire — including a client that leaves and
//! comes back with its token, and an authoritative prop one client pushes and the other observes.

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn spawn_server(extra: &[&str]) -> (Child, String, std::thread::JoinHandle<Vec<String>>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_red_server"))
        .args(["--port", "0", "--bind", "127.0.0.1", "--stats-secs", "0", "--map", concat!(env!("CARGO_MANIFEST_DIR"), "/examples/test_lab.json")])
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start red_server");
    let out = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut lines = Vec::new();
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(addr) = line.strip_prefix("LISTENING ") {
                let _ = tx.send(addr.trim().to_string());
            }
            lines.push(line);
        }
        lines
    });
    let addr = rx.recv_timeout(Duration::from_secs(20)).expect("the server printed its address");
    (child, addr, reader)
}

fn spawn_bot(addr: &str, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_red_bot"))
        .args(["--server", addr, "--map", concat!(env!("CARGO_MANIFEST_DIR"), "/examples/test_lab.json")])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start red_bot")
}

/// All JSON lines a bot printed.
fn lines(child: Child) -> Vec<Value> {
    let out = child.wait_with_output().expect("bot finished");
    assert!(out.status.success(), "bot exited with {:?}", out.status);
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn summary(v: &[Value]) -> &Value {
    &v.iter().rev().find(|l| l.get("summary").is_some()).expect("the bot printed a summary")["summary"]
}

fn events(v: &[Value]) -> Vec<String> {
    v.iter().filter_map(|l| l.get("event")).map(|e| e["what"].as_str().unwrap_or("").to_string()).collect()
}

#[test]
fn a_server_process_and_two_client_processes_play_together_and_one_reconnects() {
    let (mut server, addr, server_out) = spawn_server(&["--spawn-group", "props", "--run-for", "16"]);

    // Client A: joins first (spawn_props_a, facing the barrel `domino_0`), walks 2.6 m north into it and then
    // STOPS, so the props have settled long before the two processes take their final readings.
    let a = spawn_bot(&addr, &["--as", "human", "--behavior", "route:1.0,2.8", "--duration", "8", "--report-every", "0.5"]);
    std::thread::sleep(Duration::from_millis(1200));
    // Client B: joins second as Cheddar, stands still, leaves after 3 s and rejoins (with its token) at 5 s.
    let b = spawn_bot(&addr, &["--as", "rat", "--behavior", "idle", "--duration", "8", "--report-every", "0.5", "--leave-after", "3.0", "--rejoin-after", "5.0"]);

    let (la, lb) = (lines(a), lines(b));
    let _ = server.wait();
    let server_lines = server_out.join().unwrap();

    let (sa, sb) = (summary(&la), summary(&lb));
    assert_eq!(sa["conn"], "Connected");
    assert_eq!(sb["conn"], "Connected", "B is back online after leaving");

    // Both joined as distinct players; B came back as the same player id via its token.
    let ev_b = events(&lb);
    let joins: Vec<&String> = ev_b.iter().filter(|e| e.starts_with("connected as player")).collect();
    assert_eq!(joins.len(), 2, "B connected twice (join, then rejoin): {ev_b:?}");
    let id_of = |s: &str| s.split_whitespace().nth(3).unwrap().to_string();
    assert_eq!(id_of(joins[0]), id_of(joins[1]), "resumed as the same player: {joins:?}");
    assert!(ev_b.iter().any(|e| e.starts_with("left")) && ev_b.iter().any(|e| e.starts_with("rejoining")));
    assert_ne!(sa["id"], sb["id"]);

    // A saw B (a rat) come, go and come back: reports with and without a remote player, in that order.
    let seen: Vec<bool> = la.iter().filter_map(|l| l.get("report")).map(|r| !r["remote"].as_array().unwrap().is_empty()).collect();
    let mut runs = Vec::new();
    for s in seen {
        if runs.last() != Some(&s) {
            runs.push(s);
        }
    }
    assert!(runs.windows(3).any(|w| w == [true, false, true]), "A's view of B over time (present, gone, present expected): {runs:?}");

    // The authoritative prop A pushed: both processes report it, at the same place, away from where it was authored.
    let props_a = sa["props"].as_array().unwrap();
    let props_b = sb["props"].as_array().unwrap();
    assert!(!props_a.is_empty() && !props_b.is_empty(), "both saw promoted props: {props_a:?} / {props_b:?}");
    let mut compared = 0;
    for pa in props_a {
        if let Some(pb) = props_b.iter().find(|p| p["id"] == pa["id"]) {
            let d = ((pa["x"].as_f64().unwrap() - pb["x"].as_f64().unwrap()).powi(2) + (pa["y"].as_f64().unwrap() - pb["y"].as_f64().unwrap()).powi(2) + (pa["z"].as_f64().unwrap() - pb["z"].as_f64().unwrap()).powi(2)).sqrt();
            assert!(d < 0.05, "prop {} differs between the two client processes by {d} m", pa["id"]);
            compared += 1;
        }
    }
    assert!(compared >= 1, "the two processes must have at least one prop in common");
    // A's own prediction stayed honest through it all.
    assert!(sa["worst_prediction_correction_m"].as_f64().unwrap() < 0.5, "{}", sa["worst_prediction_correction_m"]);

    let stopped = server_lines.iter().find(|l| l.starts_with("stopped:")).expect("server summary");
    assert!(stopped.contains("2 joins") && stopped.contains("1 resumes"), "{stopped}");
}
