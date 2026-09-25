//! Deterministic trace / replay / checksum diagnosis, end to end: a recording of a scripted match and of a **real
//! multiplayer server session** replays clean with no renderer or socket, a tampered recording is caught at the right
//! tick with a readable diff, and a committed fixture trace keeps the whole thing honest on every CI platform.

use red_engine2::net::bot::{Behavior, Bot, ClientWorld};
use red_engine2::net::map_hash;
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::player::Character;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::replay::{compare_traces, describe, replay};
use red_engine2::sim::scenario::RecordOptions;
use red_engine2::sim::spawns::parse_spawns;
use red_engine2::sim::trace::{Entry, Header, Trace};
use red_engine2::tools::simrun;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn coin_run_trace(checkpoint_every: u32, dump_every: u32) -> Trace {
    let opts = RecordOptions { map_hash: 0, checkpoint_every, dump_every };
    let (report, trace) = simrun::run(&root().join("recipes/coin_run.json"), None, Some("collect all three"), Some(opts)).expect("run scenario");
    assert!(report.all_passed(), "{}", report.render());
    trace.expect("a trace was recorded")
}

fn replay_of(trace: &Trace) -> red_engine2::sim::replay::ReplayReport {
    let loaded = simrun::load(&root().join("recipes/coin_run.json")).expect("load");
    replay(trace, &loaded.scene, &loaded.spawns).expect("replay")
}

#[test]
fn a_recorded_scenario_replays_clean_and_events_match() {
    let trace = coin_run_trace(1, 60);
    assert!(trace.final_tick > 300 && trace.checkpoints.len() as u64 == trace.final_tick, "one checkpoint per tick");
    assert_eq!(trace.events.iter().filter(|e| e.name == "coin").count(), 3);
    let report = replay_of(&trace);
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(report.ticks, trace.final_tick);
    assert_eq!(report.compared as u64, trace.final_tick);
}

#[test]
fn a_trace_survives_the_json_file_format() {
    let trace = coin_run_trace(6, 60);
    let text = serde_json::to_string(&trace.to_json()).unwrap();
    let back = Trace::from_json(&serde_json::from_str(&text).unwrap()).unwrap();
    assert_eq!(back, trace);
    assert!(replay_of(&back).is_clean());
}

#[test]
fn a_tampered_input_is_found_at_the_first_divergent_tick_with_a_diff() {
    let mut trace = coin_run_trace(1, 1);
    // Change the look direction of one input at tick 100: the player walks a slightly different line from then on.
    let mut tampered_tick = None;
    for e in trace.entries.iter_mut() {
        if let Entry::Input { tick, input, .. } = e {
            if *tick >= 100 && tampered_tick.is_none() {
                input.yaw += 0.3;
                tampered_tick = Some(*tick);
            }
        }
    }
    let at = tampered_tick.expect("an input at or after tick 100");
    let report = replay_of(&trace);
    assert!(!report.is_clean());
    let d = report.exact.as_ref().expect("an exact divergence");
    assert_eq!(d.tick, at + 1, "the checkpoint right after the tampered input is the first to differ (recorded at tick {at})");
    assert_eq!(d.components, ["players"], "only the player moved differently at first");
    let c = report.coarse.as_ref().expect("the game itself diverges, not just float noise");
    assert!(c.tick >= d.tick);
    let text = describe(d);
    assert!(text.contains("first divergence at tick") && text.contains("players differ"), "{text}");
    assert!(d.detail.iter().any(|l| l.contains("player 0")), "the diff names the player: {:?}", d.detail);
}

#[test]
fn comparing_two_traces_finds_where_they_split() {
    let a = coin_run_trace(1, 60);
    assert!(compare_traces(&a, &a.clone()).is_none());
    let mut b = a.clone();
    b.checkpoints[200].players ^= 1;
    let d = compare_traces(&a, &b).expect("a divergence");
    assert_eq!(d.tick, a.checkpoints[200].tick);
    assert_eq!(d.components, ["players"]);
}

#[test]
fn a_different_tick_rate_or_map_is_reported_not_silently_replayed() {
    let mut trace = coin_run_trace(6, 60);
    trace.header.tick_rate = 30;
    let loaded = simrun::load(&root().join("recipes/coin_run.json")).unwrap();
    assert!(replay(&trace, &loaded.scene, &loaded.spawns).unwrap_err().contains("30 ticks/s"));
}

/// The committed fixture: a trace recorded once and replayed on every platform CI runs on. If a deliberate change to
/// the simulation makes it diverge, re-record it: `RE2_BLESS_TRACE=1 cargo test --test sim_replay`.
#[test]
fn the_committed_fixture_trace_replays_on_this_platform() {
    let path = root().join("tests/fixtures/coin_run.trace.json");
    if std::env::var_os("RE2_BLESS_TRACE").is_some() {
        let mut trace = coin_run_trace(6, 60);
        trace.header.scene = "recipes/coin_run.json".to_string();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string(&trace.to_json()).unwrap()).unwrap();
    }
    let outcome = simrun::replay_file(&path, Some(&root().join("recipes/coin_run.json"))).expect("replay the fixture");
    let r = &outcome.report;
    assert!(r.game_agrees(), "the game diverges from the committed recording (re-bless if intended):\n{}", outcome.render());
    let here = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
    if r.note.is_none() {
        assert!(r.exact.is_none(), "same platform ({here}) but not bit-identical:\n{}", outcome.render());
    }
    println!("fixture replay: {}", outcome.render());
}

// ---------------------------------------------------------------------------------------------
// A real multiplayer server session, recorded and replayed
// ---------------------------------------------------------------------------------------------

#[test]
fn a_real_udp_server_session_records_a_trace_that_replays_clean() {
    let path = root().join("examples/test_lab.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let scene = red_engine2::schema::parse_scene(&text).unwrap();
    let all_spawns = parse_spawns(&text).unwrap();
    let mut spawns = all_spawns.clone();
    spawns.retain(|s| s.group == "props");
    let sim = MatchSim::new(&scene, spawns);
    let barrel = scene.objects.iter().position(|o| o.id == "domino_0").and_then(|i| sim.props().prop_of_object(i));
    let mut server = Server::bind(ServerConfig::new("127.0.0.1:0".parse().unwrap(), map_hash(&text)), sim).unwrap();
    server.set_logger(|_| {});
    server.start_recording(Header::new(map_hash(&text), 0, "props", 2, 30)).unwrap();
    if let Some(p) = barrel {
        server.set_demo_kick(p); // a server-applied impulse: it must be recorded and replayed too
    }
    let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let handle = std::thread::spawn(move || {
        server.run(&s2);
        server
    });
    let (_scene, world) = ClientWorld::load(&path).unwrap();
    let mut a = Bot::new(addr, Character::Human, world, Behavior::Forward { yaw_deg: 0.0, sprint: false }, 0).unwrap();
    let (_scene, world) = ClientWorld::load(&path).unwrap();
    let mut b = Bot::new(addr, Character::Rat, world, Behavior::Circle { turn_deg_per_sec: 90.0 }, 0).unwrap();
    std::thread::scope(|s| {
        s.spawn(|| a.run(Duration::from_secs_f64(2.5), |_| true));
        s.spawn(|| b.run(Duration::from_secs_f64(2.5), |_| true));
    });
    drop((a, b));
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);
    let mut server = handle.join().unwrap();
    let trace = server.take_trace().expect("the server recorded");
    assert!(trace.final_tick > 150, "ticks: {}", trace.final_tick);
    assert!(trace.entries.iter().filter(|e| matches!(e, Entry::Join { .. })).count() >= 2, "both players joined");
    assert!(trace.entries.iter().any(|e| matches!(e, Entry::Input { .. })), "inputs were recorded");
    let report = replay(&trace, &scene, &all_spawns).expect("replay");
    assert!(report.is_clean(), "the replay of a real server session must match it exactly: {report:?}");
    assert!(report.compared > 50);
}
