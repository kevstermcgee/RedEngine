//! The match flow end to end over real UDP (ADR 0029): clients join a lobby, choose a character, ready up, watch a countdown, play a
//! timed round, see the results, and rematch; a late joiner waits for the next round; a leaver aborts a countdown; a keyed server
//! refuses a client without the key. Timing-based, so assertions are tolerant and every wait has a generous deadline.

use red_engine2::net::bot::{Behavior, Bot, ClientWorld};
use red_engine2::net::client::{ClientConfig, ConnState, NetClient};
use red_engine2::net::server::{RoundRecord, Server, ServerConfig};
use red_engine2::player::Character;
use red_engine2::sim::flow::{EndReason, MatchSettings, Phase};
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

fn lab_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")
}

struct TestServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Server>>,
    rounds: Arc<Mutex<Vec<RoundRecord>>>,
}

impl TestServer {
    fn start(settings: MatchSettings, key: Option<&str>) -> TestServer {
        let text = std::fs::read_to_string(lab_path()).unwrap();
        let scene = red_engine2::schema::parse_scene(&text).unwrap();
        let mut spawns = parse_spawns(&text).unwrap();
        spawns.retain(|s| s.group == "duel");
        let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), red_engine2::net::map_hash(&text));
        cfg.join_key = key.map(str::to_string);
        let mut server = Server::bind(cfg, MatchSim::try_new(&scene, spawns.clone()).unwrap()).unwrap();
        server.set_logger(|_| {});
        server.enable_flow(settings, move || MatchSim::try_new(&scene, spawns.clone())).unwrap();
        let rounds = Arc::new(Mutex::new(Vec::new()));
        let sink = rounds.clone();
        server.set_round_hook(move |r| sink.lock().unwrap().push(r));
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = Some(std::thread::spawn(move || {
            server.run(&s2);
            server
        }));
        TestServer { addr, stop, handle, rounds }
    }

    fn finish(mut self) -> Server {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().unwrap().join().unwrap()
    }
}

fn quick() -> MatchSettings {
    MatchSettings { min_players: 2, countdown_secs: 0.4, round_secs: 1.5, results_secs: 0.8, score_to_win: 0, join_in_progress: true, ready_check: true }
}

fn bot(server: SocketAddr, name: &str, auto_ready: bool) -> Bot {
    let (_scene, world) = ClientWorld::load(&lab_path()).unwrap();
    let mut cfg = ClientConfig::new(server, 0, world.map_hash, 0);
    cfg.name = name.to_string();
    let mut b = Bot::with_client(NetClient::connect_with(cfg).unwrap(), Character::Human, world, Behavior::Forward { yaw_deg: 90.0, sprint: false }).unwrap();
    b.auto_ready = auto_ready;
    b
}

/// Pumps every bot until `done` says so (or 12 s pass, which fails the test with the bots' event logs).
fn drive(bots: &mut [&mut Bot], what: &str, mut done: impl FnMut(&[&mut Bot]) -> bool) {
    let end = Instant::now() + Duration::from_secs(12);
    while Instant::now() < end {
        let now = Instant::now();
        for b in bots.iter_mut() {
            b.pump(now);
        }
        if done(bots) {
            return;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    let logs: Vec<String> = bots.iter().map(|b| format!("{:?}", b.events)).collect();
    panic!("timed out waiting for: {what}\n{}", logs.join("\n"));
}

fn phase(b: &Bot) -> Phase {
    b.client.phase()
}

#[test]
fn lobby_countdown_round_results_rematch_and_the_next_round_starts_from_the_authored_map() {
    let server = TestServer::start(quick(), None);
    let mut a = bot(server.addr, "Ada", false);
    let mut b = bot(server.addr, "Bo", true);
    drive(&mut [&mut a, &mut b], "both in the lobby", |bs| bs.iter().all(|x| x.client.state() == ConnState::Connected && x.client.status().is_some()));
    assert_eq!((phase(&a), a.client.in_round()), (Phase::Waiting, false), "the lobby is not the world");
    let roster = a.client.status().unwrap().roster.clone();
    assert_eq!(roster.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().len(), 2);
    assert!(roster.iter().any(|e| e.name == "Ada") && roster.iter().any(|e| e.name == "Bo"), "names reach the roster: {roster:?}");

    // Only Bo is ready: nothing starts, however long we wait.
    let t = Instant::now();
    drive(&mut [&mut a, &mut b], "Bo's ready shows in the roster", |bs| {
        bs[0].client.status().is_some_and(|s| s.roster.iter().any(|e| e.name == "Bo" && e.flags & 1 != 0))
    });
    while t.elapsed() < Duration::from_millis(600) {
        a.pump(Instant::now());
        b.pump(Instant::now());
        std::thread::sleep(Duration::from_millis(3));
    }
    assert_eq!(phase(&a), Phase::Waiting, "one of two ready is not everyone");

    // Ada readies up: countdown, then the round; both get a body at distinct spawns.
    a.client.set_ready(true, Instant::now());
    drive(&mut [&mut a, &mut b], "the round starts", |bs| bs.iter().all(|x| phase(x) == Phase::Playing && x.client.in_round() && x.predictor.is_some()));
    let (wa, wb) = (a.client.welcome().unwrap(), b.client.welcome().unwrap());
    assert_eq!((wa.round, wb.round), (1, 1));
    assert_ne!(wa.player_id, wb.player_id);
    assert!((wa.spawn[0] - wb.spawn[0]).abs() + (wa.spawn[2] - wb.spawn[2]).abs() > 1.0, "distinct spawn points");
    assert!(a.events.iter().any(|e| e.1.starts_with("phase countdown round 1")), "the countdown was announced: {:?}", a.events);
    // Playing: movement is real.
    drive(&mut [&mut a, &mut b], "Ada walks away from her spawn", |bs| {
        bs[0].frame(Instant::now()).me.distance(glam::Vec2::new(wa.spawn[0], wa.spawn[2])) > 1.5
    });

    // The round is timed: 1.5 s, then the results.
    drive(&mut [&mut a, &mut b], "the results", |bs| bs.iter().all(|x| phase(x) == Phase::Results));
    let st = a.client.status().unwrap().clone();
    assert_eq!(st.end_code, 1, "ended by the clock");
    assert!(!a.client.is_ready(), "a stale Ready must not skip the results");
    assert!(!a.client.in_round(), "no inputs during the results");
    // Nobody rematches: after results_secs the lobby returns and the ready flags are clear (Bo's auto-ready pressed Ready during the
    // results, so let go of it first: a vote cast during the results is kept, which is the point of the rematch).
    b.auto_ready = false;
    b.client.set_ready(false, Instant::now());
    drive(&mut [&mut a, &mut b], "back in the lobby", |bs| bs.iter().all(|x| phase(x) == Phase::Waiting));
    drive(&mut [&mut a, &mut b], "the roster shows nobody ready", |bs| bs[0].client.status().is_some_and(|s| s.roster.iter().all(|e| e.flags & 1 == 0)));

    // Rematch: round 2 starts from the authored map, not from where they stood.
    a.client.set_ready(true, Instant::now());
    b.client.set_ready(true, Instant::now());
    drive(&mut [&mut a, &mut b], "round 2", |bs| {
        bs.iter().all(|x| phase(x) == Phase::Playing && x.client.in_round() && x.client.welcome().is_some_and(|w| w.round == 2))
    });
    let w2 = a.client.welcome().unwrap();
    assert_eq!((w2.player_id, w2.spawn), (wa.player_id, wa.spawn), "same id, same spawn: the world was rebuilt");
    let m = a.frame(Instant::now()).me;
    assert!(m.distance(glam::Vec2::new(w2.spawn[0], w2.spawn[2])) < 1.0, "the predictor was placed at the new spawn, not left where round 1 ended");
    drive(&mut [&mut a, &mut b], "round 2 ends", |bs| bs.iter().all(|x| phase(x) == Phase::Results));

    let records = server.rounds.lock().unwrap().clone();
    assert_eq!(records.len(), 2, "both rounds were reported to the hook");
    assert_eq!((records[0].round, records[0].reason.clone()), (1, EndReason::TimeUp));
    assert_eq!(records[1].round, 2);
    assert_eq!(server.finish().stats().rounds, 2);
}

#[test]
fn everyone_pressing_ready_during_the_results_rematches_at_once() {
    let server = TestServer::start(MatchSettings { results_secs: 30.0, ..quick() }, None);
    let (mut a, mut b) = (bot(server.addr, "A", true), bot(server.addr, "B", true));
    drive(&mut [&mut a, &mut b], "round 1 results", |bs| bs.iter().all(|x| phase(x) == Phase::Results));
    // Results last 30 s, but both bots press Ready again (auto_ready) and the flow skips the wait.
    drive(&mut [&mut a, &mut b], "round 2 starts long before the results would have ended", |bs| {
        bs.iter().all(|x| phase(x) == Phase::Playing && x.client.welcome().is_some_and(|w| w.round == 2))
    });
    let rounds = server.rounds.lock().unwrap().len();
    assert_eq!(rounds, 1, "round 1 was recorded once");
    drop(server.finish());
}

#[test]
fn a_late_joiner_watches_until_the_next_round_when_join_in_progress_is_off() {
    let server = TestServer::start(MatchSettings { join_in_progress: false, round_secs: 2.0, ..quick() }, None);
    let (mut a, mut b) = (bot(server.addr, "A", true), bot(server.addr, "B", true));
    drive(&mut [&mut a, &mut b], "round 1 is running", |bs| bs.iter().all(|x| phase(x) == Phase::Playing && x.client.in_round()));
    let mut late = bot(server.addr, "Late", true);
    drive(&mut [&mut a, &mut b, &mut late], "the latecomer is connected and told to watch", |bs| {
        bs[2].client.state() == ConnState::Connected && bs[2].client.status().is_some()
    });
    assert_eq!(phase(&late), Phase::Playing);
    assert!(!late.client.in_round(), "no body in a round that is already running");
    assert!(late.predictor.is_none(), "so nothing to predict");
    // The next round includes them: they were ready (auto) and everyone rematches.
    drive(&mut [&mut a, &mut b, &mut late], "round 2 with three players", |bs| {
        bs.iter().all(|x| x.client.in_round() && x.client.welcome().is_some_and(|w| w.round == 2))
    });
    drop(server.finish());
}

#[test]
fn a_player_leaving_during_the_countdown_aborts_it_and_the_round_number_is_not_burned() {
    let server = TestServer::start(MatchSettings { countdown_secs: 5.0, ..quick() }, None);
    let (mut a, mut b) = (bot(server.addr, "A", true), bot(server.addr, "B", true));
    drive(&mut [&mut a, &mut b], "the countdown", |bs| bs.iter().all(|x| phase(x) == Phase::Countdown));
    let w = a.client.welcome().unwrap();
    assert!(w.in_round && w.round == 1, "in the countdown everyone already stands at a spawn: {w:?}");
    b.client.disconnect();
    drive(&mut [&mut a], "back to waiting", |bs| phase(bs[0]) == Phase::Waiting);
    assert_eq!(a.client.status().unwrap().round, 0, "the aborted countdown did not use up round 1");
    drop(server.finish());
}

#[test]
fn a_keyed_server_admits_the_right_key_and_refuses_a_missing_one_with_an_explanation() {
    let server = TestServer::start(quick(), Some("correct horse battery staple"));
    let (_s, world) = ClientWorld::load(&lab_path()).unwrap();
    let join = |key: Option<&str>| {
        let mut cfg = ClientConfig::new(server.addr, 0, world.map_hash, 0);
        cfg.join_key = key.map(str::to_string);
        let mut c = NetClient::connect_with(cfg).unwrap();
        let end = Instant::now() + Duration::from_secs(4);
        while Instant::now() < end && matches!(c.state(), ConnState::Connecting) {
            c.poll(Instant::now());
            std::thread::sleep(Duration::from_millis(3));
        }
        c.state()
    };
    assert_eq!(join(Some("correct horse battery staple")), ConnState::Connected);
    assert_eq!(join(None), ConnState::Rejected(red_engine2::net::protocol::RejectReason::NeedsKey));
    assert_eq!(join(Some("wrong")), ConnState::Rejected(red_engine2::net::protocol::RejectReason::BadKey));
    let server = server.finish();
    assert_eq!(server.stats().bad_keys, 1);
}
