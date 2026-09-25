//! Interactions over **real UDP**: the server decides who picks up a contested prop, every client sees the carried prop move,
//! and a shot fired by one client damages another and kills them — all through the same bounded binary protocol, with the
//! clients only sending buttons.

use red_engine2::net::bot::{Behavior, Bot, BotFrame, ClientWorld};
use red_engine2::net::client::ConnState;
use red_engine2::net::map_hash;
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::player::Character;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

fn lab_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")
}

struct TestServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Server>>,
    barrel_prop: Option<usize>,
    /// Where the barrel starts (props are not sent until something disturbs them).
    barrel_origin: glam::Vec3,
}

impl TestServer {
    fn start(group: &str) -> TestServer {
        let text = std::fs::read_to_string(lab_path()).unwrap();
        let scene = red_engine2::schema::parse_scene(&text).unwrap();
        let mut spawns = parse_spawns(&text).unwrap();
        spawns.retain(|s| s.group == group);
        let sim = MatchSim::new(&scene, spawns);
        let barrel_index = scene.objects.iter().position(|o| o.id == "domino_0").unwrap();
        let barrel_origin = scene.objects[barrel_index].position.sample(0.0);
        let barrel_prop = sim.props().prop_of_object(barrel_index);
        let mut server = Server::bind(ServerConfig::new("127.0.0.1:0".parse().unwrap(), map_hash(&text)), sim).unwrap();
        server.set_logger(|_| {});
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = Some(std::thread::spawn(move || {
            server.run(&s2);
            server
        }));
        TestServer { addr, stop, handle, barrel_prop, barrel_origin }
    }

    fn finish(mut self) -> Server {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().unwrap().join().unwrap()
    }
}

fn bot(addr: SocketAddr, pitch_deg: f32) -> Bot {
    let (_scene, world) = ClientWorld::load(&lab_path()).unwrap();
    let mut b = Bot::new(addr, Character::Human, world, Behavior::Idle, 0).unwrap();
    b.pitch = pitch_deg.to_radians();
    b
}

/// Runs every bot concurrently for `secs`, calling `watch(index, frame)` for each frame of each bot.
fn run_phase(bots: &mut [&mut Bot], secs: f64, watch: &(dyn Fn(usize, &BotFrame) + Sync)) {
    std::thread::scope(|s| {
        for (i, b) in bots.iter_mut().enumerate() {
            s.spawn(move || {
                b.run(Duration::from_secs_f64(secs), |f| {
                    watch(i, f);
                    true
                });
            });
        }
    });
}

fn connect_in_order(bots: &mut [&mut Bot]) {
    for b in bots.iter_mut() {
        b.run(Duration::from_secs_f64(2.0), |f| f.conn != ConnState::Connected);
        assert_eq!(b.client.state(), ConnState::Connected);
    }
}

#[test]
fn a_contested_barrel_goes_to_one_client_and_every_client_sees_it_carried() {
    let server = TestServer::start("props");
    let barrel = server.barrel_prop.expect("domino_0 is a loose prop") as u16;
    let origin = server.barrel_origin;
    // Slot 0 stands at spawn_props_a, slot 1 at spawn_props_b (each aiming at the barrel), slot 2 only watches.
    let (mut a, mut b, mut c) = (bot(server.addr, -40.0), bot(server.addr, -27.5), bot(server.addr, 0.0));
    connect_in_order(&mut [&mut a, &mut b, &mut c]);
    let seen = std::sync::Mutex::new(Vec::<glam::Vec3>::new());
    let watch = |i: usize, f: &BotFrame| {
        if i == 2 {
            if let Some((_, p)) = f.props.iter().find(|(id, _)| *id == barrel) {
                seen.lock().unwrap().push(p.pos);
            }
        }
    };
    run_phase(&mut [&mut a, &mut b, &mut c], 0.5, &watch); // settle
    a.buttons = 8; // both press E at the same moment
    b.buttons = 8;
    run_phase(&mut [&mut a, &mut b, &mut c], 0.15, &watch);
    a.buttons = 0;
    b.buttons = 0;
    run_phase(&mut [&mut a, &mut b, &mut c], 1.0, &watch);
    let server = server.finish();
    let holders: Vec<usize> = (0..8).filter(|s| server.sim().props().held_by(*s).is_some()).collect();
    assert_eq!(holders.len(), 1, "exactly one player ended up holding the barrel: {holders:?}");
    let held = server.sim().props().held_by(holders[0]).unwrap();
    assert_eq!(held as u16, barrel, "and it is the barrel both were aiming at");
    let track = seen.lock().unwrap().clone();
    assert!(track.len() > 20, "the observer received prop updates for it: {}", track.len());
    let travelled = track.iter().map(|p| p.distance(origin)).fold(0.0f32, f32::max);
    assert!(travelled > 0.3, "the observer saw the barrel lifted off its spot and carried: it moved {travelled} m from {origin}");
}

#[test]
fn a_shot_fired_by_one_client_hurts_another_and_four_kill_them() {
    let server = TestServer::start("duel");
    let (mut a, mut b) = (bot(server.addr, 0.0), bot(server.addr, 0.0));
    connect_in_order(&mut [&mut a, &mut b]);
    let victim_hp = std::sync::Mutex::new(Vec::<(u8, bool)>::new());
    let watch = |i: usize, f: &BotFrame| {
        if i == 0 {
            if let Some((_, p)) = f.remote.first() {
                victim_hp.lock().unwrap().push((p.hp, p.dead));
            }
        }
    };
    run_phase(&mut [&mut a, &mut b], 0.4, &watch);
    a.buttons = 64; // switch to the revolver
    run_phase(&mut [&mut a, &mut b], 0.1, &watch);
    a.buttons = 0;
    run_phase(&mut [&mut a, &mut b], 0.6, &watch); // the raise takes 0.34 s
    for _ in 0..4 {
        a.buttons = 16; // fire
        run_phase(&mut [&mut a, &mut b], 0.1, &watch);
        a.buttons = 0;
        run_phase(&mut [&mut a, &mut b], 0.55, &watch); // the cooldown is 0.42 s
    }
    let server = server.finish();
    let hist = server.sim().rules().history();
    let count = |n: &str| hist.iter().filter(|e| e.name == n).count();
    assert_eq!(count("shot"), 4, "{:?}", hist.iter().map(|e| &e.name).collect::<Vec<_>>());
    assert_eq!(count("hit"), 4);
    assert_eq!(count("kill"), 1, "the fourth hit killed");
    let seen = victim_hp.lock().unwrap().clone();
    assert!(seen.iter().any(|(hp, _)| *hp == 75), "the shooter's client saw the victim at 75 hp: {seen:?}");
    assert!(seen.iter().any(|(_, dead)| *dead), "and saw them die");
    assert!(seen.first().is_some_and(|(hp, _)| *hp == 100));
}
