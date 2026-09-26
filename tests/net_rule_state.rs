//! Rule presentation is a repeated authoritative state snapshot: peers agree, and a late joiner
//! receives variables, visibility and outcome without replaying the event that produced them.

use red_engine2::net::client::NetClient;
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn wait_for_finished_state(clients: &mut [&mut NetClient]) {
    let end = Instant::now() + Duration::from_secs(4);
    while Instant::now() < end {
        let now = Instant::now();
        for client in clients.iter_mut() {
            client.poll(now);
        }
        if clients.iter().all(|client| client.rule_state().is_some_and(|s| s.outcome == "victory")) {
            return;
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    panic!("clients did not receive the terminal rule state");
}

#[test]
fn peers_and_a_late_joiner_recover_the_same_complete_rule_state() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/test_lab.json")).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&source).unwrap();
    value["vars"] = json!({"score": 0});
    value["rules"] = json!([{
        "id": "finish",
        "when": {"after": 0.05},
        "once": true,
        "do": [
            {"add": ["score", 3]},
            {"hide": "floor_spawn_hall"},
            {"collision": ["floor_spawn_hall", false]},
            {"emit": "collected"},
            {"end": "victory"}
        ]
    }]);
    let text = serde_json::to_string(&value).unwrap();
    let scene = red_engine2::schema::parse_scene(&text).unwrap();
    let spawns = parse_spawns(&text).unwrap();
    let hidden_index = red_engine2::schema::object_ids(&scene.objects).iter().position(|id| id == "floor_spawn_hall").unwrap() as u16;
    let hash = red_engine2::net::map_hash(&text);
    let mut server = Server::bind(ServerConfig::new("127.0.0.1:0".parse().unwrap(), hash), MatchSim::new(&scene, spawns)).unwrap();
    server.set_logger(|_| {});
    let addr = server.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let handle = std::thread::spawn(move || {
        server.run(&stop2);
        server
    });

    let mut a = NetClient::connect(addr, 0, hash, 0).unwrap();
    let mut b = NetClient::connect(addr, 1, hash, 0).unwrap();
    wait_for_finished_state(&mut [&mut a, &mut b]);
    assert_eq!(a.rule_state(), b.rule_state(), "simultaneous clients must present identical state");

    let state = a.rule_state().unwrap();
    assert_eq!(state.vars.iter().find(|v| v.name == "score").map(|v| v.value), Some(3.0));
    assert_eq!(state.hidden, [hidden_index]);
    assert_eq!(state.collision_disabled, [hidden_index]);
    assert_eq!(state.event, "collected");

    let mut late = NetClient::connect(addr, 0, hash, 0).unwrap();
    wait_for_finished_state(&mut [&mut late]);
    let late_state = late.rule_state().unwrap();
    assert_eq!(late_state.vars, state.vars);
    assert_eq!(late_state.hidden, state.hidden);
    assert_eq!(late_state.collision_disabled, state.collision_disabled);
    assert_eq!(late_state.outcome, state.outcome);

    stop.store(true, Ordering::Relaxed);
    let server = handle.join().unwrap();
    assert!(server.stats().rule_states_sent >= 3);
}
