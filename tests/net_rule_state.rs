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
    let scene = red_engine2::scene_blocks::parse_scene(&text).unwrap();
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

/// Quiet Compass (Idea Forge 2026-10-15, F1): the server sends at most `MAX_RULE_VARS` variables, so a HUD counter on a variable declared after the
/// sixteenth showed 0 online while it counted offline. The variables the HUD reads go first; the online HUD equals the one drawn from every variable.
#[test]
fn a_hud_counter_on_a_variable_declared_after_the_sixteenth_counts_online() {
    use red_engine2::net::protocol::MAX_RULE_VARS;
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/test_lab.json")).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&source).unwrap();
    let names: Vec<String> = (0..20).map(|i| format!("v{i:02}")).collect();
    value["vars"] = serde_json::Value::Object(names.iter().map(|n| (n.clone(), json!(0))).collect());
    value["hud"] = json!({"show_rules_vars": false});
    value["ui"] = json!({
        "title": "Late counters",
        "counters": [{"var": "v17", "label": "Crumble", "of": 9}, {"var": "v02", "label": "Early"}],
        "objective": [{"if": "v19 >= 5", "text": "Leave ({v18} left)"}, {"text": "Wait"}]
    });
    value["rules"] = json!([{
        "id": "count",
        "when": {"after": 0.05},
        "once": true,
        "do": [{"set": ["v17", 7]}, {"set": ["v19", 5]}, {"set": ["v18", 3]}, {"set": ["v02", 2]}, {"end": "victory"}]
    }]);
    let text = serde_json::to_string(&value).unwrap();
    let scene = red_engine2::scene_blocks::parse_scene(&text).unwrap();
    assert!(scene.rules.var_names.len() > MAX_RULE_VARS, "the scene must declare more variables than a networked match sends");
    let spawns = parse_spawns(&text).unwrap();
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

    let mut client = NetClient::connect(addr, 0, hash, 0).unwrap();
    wait_for_finished_state(&mut [&mut client]);
    let state = client.rule_state().unwrap().clone();
    stop.store(true, Ordering::Relaxed);
    handle.join().unwrap();

    assert!(state.vars.len() <= MAX_RULE_VARS);
    let got = |n: &str| state.vars.iter().find(|v| v.name == n).map(|v| v.value);
    assert_eq!((got("v17"), got("v18"), got("v19"), got("v02")), (Some(7.0), Some(3.0), Some(5.0), Some(2.0)), "sent: {:?}", state.vars);
    // What the online client draws from the server's variables is what a client holding all twenty would draw.
    let mut all: Vec<(String, f64)> = names.iter().map(|n| (n.clone(), 0.0)).collect();
    for (n, v) in [("v17", 7.0), ("v19", 5.0), ("v18", 3.0), ("v02", 2.0)] {
        all.iter_mut().find(|(name, _)| name == n).unwrap().1 = v;
    }
    let texts = |vars: &[(&str, f64)]| -> Vec<String> {
        let layout = red_engine2::ui::game::rules_overlay(&scene, 960, 540, vars, None, None, true);
        layout.widgets.iter().filter_map(|w| w.text.clone()).collect()
    };
    let online: Vec<(&str, f64)> = state.vars.iter().map(|v| (v.name.as_str(), v.value)).collect();
    let everything: Vec<(&str, f64)> = all.iter().map(|(n, v)| (n.as_str(), *v)).collect();
    assert_eq!(texts(&online), texts(&everything));
    assert!(texts(&online).iter().any(|t| t.contains("7 / 9")), "{:?}", texts(&online));
    // The bug, for the record: the first sixteen in declaration order (what the server used to send) draw the counter at 0 and the wrong objective.
    let old = texts(&everything[..MAX_RULE_VARS]);
    assert!(old.iter().any(|t| t.contains("0 / 9")) && old.iter().any(|t| t == "WAIT"), "{old:?}");
}
