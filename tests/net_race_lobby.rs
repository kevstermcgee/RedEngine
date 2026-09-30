//! The race lobby over real UDP (ADR 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature): in a race the lobby's character byte is
//! the animal. Two clients ask for animals (one asks for one that is already taken), ready up, and the round's karts carry the animals they got.
//! Timing-based, so every wait has a generous deadline.

use red_engine2::net::client::{ClientConfig, ConnState, NetClient};
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::sim::flow::{MatchSettings, Phase};
use red_engine2::sim::kart::Driver;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RACE_SCENE: &str = r#"{"camera":{"position":[0,30,60],"target":[0,0,0]},
    "zones":[{"id":"line","rect":[-6,-41,6,-39]},{"id":"east","rect":[39,-6,41,6]},{"id":"south","rect":[-6,39,6,41]},{"id":"west","rect":[-41,-6,-39,6]}],
    "race":{"laps":1,"gates":["line","east","south","west"],"countdown_secs":1},
    "spawns":[{"id":"a","position":[-20,0,-41.5],"yaw_deg":90},{"id":"b","position":[-20,0,-38.5],"yaw_deg":90}],
    "objects":[{"id":"floor","type":"plane","size":[200,200],"position":[0,0.01,0]}]}"#;

fn client(addr: SocketAddr, hash: u32, name: &str, animal: Driver) -> NetClient {
    let mut cfg = ClientConfig::new(addr, animal.wire(), hash, 0);
    cfg.name = name.to_string();
    NetClient::connect_with(cfg).unwrap()
}

fn pump(clients: &mut [&mut NetClient], what: &str, done: impl Fn(&[&mut NetClient]) -> bool) {
    let end = Instant::now() + Duration::from_secs(12);
    while Instant::now() < end {
        for c in clients.iter_mut() {
            c.poll(Instant::now());
        }
        if done(clients) {
            return;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    panic!("timed out waiting for: {what}");
}

#[test]
fn the_lobby_chooses_the_animal_and_a_duplicate_gets_the_next_free_one() {
    let scene = red_engine2::schema::parse_scene(RACE_SCENE).unwrap();
    let spawns = parse_spawns(RACE_SCENE).unwrap();
    let hash = red_engine2::net::map_hash(RACE_SCENE);
    let mut server = Server::bind(ServerConfig::new("127.0.0.1:0".parse().unwrap(), hash), MatchSim::try_new(&scene, spawns.clone()).unwrap()).unwrap();
    server.set_logger(|_| {});
    let sp2 = spawns.clone();
    let settings =
        MatchSettings { min_players: 2, countdown_secs: 0.3, round_secs: 0.0, results_secs: 1.0, score_to_win: 0, join_in_progress: false, ready_check: true };
    server.enable_flow(settings, move || MatchSim::try_new(&red_engine2::schema::parse_scene(RACE_SCENE).map_err(|e| e.join("; "))?, sp2.clone())).unwrap();
    let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let handle = std::thread::spawn(move || {
        server.run(&stop2);
        server
    });

    // Ada asks for the Bear, Bo asks for the Bear too.
    let mut a = client(addr, hash, "Ada", Driver::Bear);
    let mut b = client(addr, hash, "Bo", Driver::Bear);
    pump(&mut [&mut a, &mut b], "both in the lobby", |cs| {
        cs.iter().all(|c| c.state() == ConnState::Connected && c.status().is_some_and(|s| s.roster.len() == 2))
    });
    let roster = a.status().unwrap().roster.clone();
    assert!(roster.iter().all(|e| e.character == Driver::Bear.wire()), "a race lobby carries the animal, not a body: {roster:?}");

    // Bo changes his mind to the Wolf; a request beyond the eight is clamped to the last (the Beaver).
    b.set_character(Driver::Wolf.wire(), Instant::now());
    pump(&mut [&mut a, &mut b], "Bo's Wolf shows in the roster", |cs| {
        cs[0].status().is_some_and(|s| s.roster.iter().any(|e| e.name == "Bo" && e.character == Driver::Wolf.wire()))
    });
    b.set_character(Driver::Bear.wire(), Instant::now()); // back to the Bear: a duplicate of Ada's

    a.set_ready(true, Instant::now());
    b.set_ready(true, Instant::now());
    pump(&mut [&mut a, &mut b], "the round starts", |cs| cs.iter().all(|c| c.phase() == Phase::Playing && c.in_round()));

    stop.store(true, Ordering::Relaxed);
    let server = handle.join().unwrap();
    let (ia, ib) = (a.my_id().unwrap() as usize, b.my_id().unwrap() as usize);
    assert_eq!(server.sim().driver(ia), Some(Driver::Bear), "the lower slot keeps the animal both asked for");
    assert_ne!(server.sim().driver(ib), Some(Driver::Bear), "the other gets a free one");
    assert!(server.sim().driver(ib).is_some());
}
