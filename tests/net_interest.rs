//! Spatial interest management over real UDP, in the Red Test Lab (six rooms in a row joined by portals): a client hears about
//! its own room and the rooms one open portal away, and nothing else. This proves that entities in irrelevant rooms stop
//! costing snapshot bandwidth, that a client walking into range is told the current state of things that moved while it was
//! away, and that the whole thing is switched by map data (zones + portals), not by hand-written replication sets.

use red_engine2::net::bot::{Behavior, Bot, BotFrame, ClientWorld};
use red_engine2::net::client::ConnState;
use red_engine2::net::map_hash;
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::player::Character;
use red_engine2::sim::interest::InterestMap;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

fn lab_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")
}

struct TestServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Server>>,
}

impl TestServer {
    /// The whole lab (all six spawns: the first four joiners land in the spawn hall, the next two in the props room).
    fn start(interest: bool, kick_barrel: bool) -> TestServer {
        let text = std::fs::read_to_string(lab_path()).unwrap();
        let scene = red_engine2::schema::parse_scene(&text).unwrap();
        let sim = MatchSim::new(&scene, parse_spawns(&text).unwrap());
        let kick = kick_barrel.then(|| {
            let i = scene.objects.iter().position(|o| o.id == "domino_0").unwrap();
            sim.props().prop_of_object(i).unwrap()
        });
        let mut server = Server::bind(ServerConfig::new("127.0.0.1:0".parse().unwrap(), map_hash(&text)), sim).unwrap();
        server.set_logger(|_| {});
        if interest {
            server.set_interest(InterestMap::parse(&text).unwrap());
        }
        if let Some(p) = kick {
            server.set_demo_kick(p);
        }
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = Some(std::thread::spawn(move || {
            server.run(&s2);
            server
        }));
        TestServer { addr, stop, handle }
    }

    fn finish(mut self) -> Server {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().unwrap().join().unwrap()
    }
}

fn bot(addr: SocketAddr, behavior: Behavior) -> Bot {
    let (_scene, world) = ClientWorld::load(&lab_path()).unwrap();
    Bot::new(addr, Character::Human, world, behavior, 0).unwrap()
}

fn connect_in_order(bots: &mut [Bot]) {
    for b in bots.iter_mut() {
        b.run(Duration::from_secs_f64(2.0), |f| f.conn != ConnState::Connected);
        assert_eq!(b.client.state(), ConnState::Connected);
    }
}

fn run_all(bots: &mut [Bot], secs: f64, watch: &(dyn Fn(usize, &BotFrame) + Sync)) {
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

/// Six clients: four idle in the spawn hall, two in the props room, one of which walks into the barrels. Returns what the
/// server sent and, per client, the most props and other players it ever saw at once.
fn session(interest: bool) -> (red_engine2::net::server::ServerStats, Vec<(usize, usize)>) {
    let server = TestServer::start(interest, false);
    let mut bots: Vec<Bot> = (0..5).map(|_| bot(server.addr, Behavior::Idle)).collect();
    bots.push(bot(server.addr, Behavior::Forward { yaw_deg: 0.0, sprint: true })); // slot 5 = spawn_props_b, walks -Z into the barrels
    connect_in_order(&mut bots);
    let peaks = Mutex::new(vec![(0usize, 0usize); 6]);
    run_all(&mut bots, 3.0, &|i, f| {
        let mut p = peaks.lock().unwrap();
        p[i].0 = p[i].0.max(f.props.len());
        p[i].1 = p[i].1.max(f.remote.len());
    });
    let stats = server.finish().stats().clone();
    let peaks = peaks.into_inner().unwrap();
    (stats, peaks)
}

#[test]
fn clients_in_far_rooms_stop_receiving_props_and_players_and_bandwidth_drops() {
    let (off, off_peaks) = session(false);
    let (on, on_peaks) = session(true);
    // Without interest management everyone hears about everything: the four hall clients see the barrels move and all 5 others.
    assert!(off_peaks[..4].iter().all(|(props, players)| *props > 0 && *players == 5), "off: {off_peaks:?}");
    // With it, the spawn hall (three portals from the props room) hears nothing about props and only its own three neighbours.
    assert!(on_peaks[..4].iter().all(|(props, players)| *props == 0 && *players == 3), "on: {on_peaks:?}");
    // The two props-room clients still see the barrels and each other.
    assert!(on_peaks[4..].iter().all(|(props, players)| *props > 0 && *players == 1), "on: {on_peaks:?}");
    // Bandwidth: prop records are what interest management removes; player records shrink too.
    println!(
        "props sent: {} -> {} ; players sent: {} -> {} ; bytes out: {} -> {}",
        off.props_sent, on.props_sent, off.players_sent, on.players_sent, off.bytes_out, on.bytes_out
    );
    assert!(on.props_sent * 3 <= off.props_sent, "prop records: {} with interest vs {} without", on.props_sent, off.props_sent);
    assert!(on.players_sent < off.players_sent);
    assert!(on.bytes_out * 10 < off.bytes_out * 8, "at least 20% fewer bytes with interest: {} vs {}", on.bytes_out, off.bytes_out);
}

#[test]
fn walking_into_range_delivers_the_current_state_of_props_that_moved_while_you_were_away() {
    let server = TestServer::start(true, true); // the server nudges barrel domino_0 every 3 s
                                                // Slot 0 (spawn hall) walks through the doors to the props room; slot 1 stays behind in the hall as the control.
    let route = "-22,-6; -17,0; -14,0.6; -12,1.0; -10,0.2; -7,0; -3.3,0.5; -1,0.3; 1.5,0.4; 4,0.4; 7,0.4";
    let points: Vec<(f32, f32)> =
        route.split(';').map(|p| p.trim().split_once(',').map(|(x, z)| (x.trim().parse().unwrap(), z.trim().parse().unwrap())).unwrap()).collect();
    let mut bots = vec![bot(server.addr, Behavior::Waypoints { points, sprint: false }), bot(server.addr, Behavior::Idle)];
    connect_in_order(&mut bots);
    let trail = Mutex::new(Vec::<(f32, usize)>::new()); // (walker x, props known) per frame
    let control_props = Mutex::new(0usize);
    run_all(&mut bots, 9.0, &|i, f| {
        if i == 0 {
            if let Some(me) = f.me_state {
                trail.lock().unwrap().push((me.pos.x, f.props.len()));
            }
        } else {
            let mut c = control_props.lock().unwrap();
            *c = (*c).max(f.props.len());
        }
    });
    server.finish();
    let trail = trail.into_inner().unwrap();
    let far: usize = trail.iter().filter(|(x, _)| *x < -9.0).map(|(_, n)| *n).max().unwrap_or(0);
    let near: usize = trail.iter().filter(|(x, _)| *x > -6.0).map(|(_, n)| *n).max().unwrap_or(0);
    assert!(trail.iter().any(|(x, _)| *x < -12.0) && trail.iter().any(|(x, _)| *x > -3.0), "the walker really crossed the lab");
    assert_eq!(far, 0, "two or more portals away (hall, clearance): nothing about the barrels is sent");
    assert!(near >= 1, "one portal away (from the vertical room): the barrel that moved while it was away is delivered");
    assert_eq!(*control_props.lock().unwrap(), 0, "the client that stayed in the hall never heard about props");
}
