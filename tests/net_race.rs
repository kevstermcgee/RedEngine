//! A kart race over the real wire (ADR 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature, protocol v11): a real `Server` on loopback
//! with a raw client. The scene's `race` block must reach the client in every snapshot (phase, countdown, clock), every player's kart block must
//! carry the driver and the race progress, the countdown must hold the kart and the light must release it, and a match without a race must stay
//! exactly as it was (no race header, no kart blocks). Time is under the test's control (`Server::pump(now)` / `tick(now)`), so it is quick.

use red_engine2::net::map_hash;
use red_engine2::net::protocol::{ClientMsg, InputPacket, ServerMsg, Snapshot};
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::net::testkit::RawClient;
use red_engine2::sim::kart::Driver;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::player::PlayerInput;
use red_engine2::sim::spawns::parse_spawns;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const RACE_SCENE: &str = r#"{"camera":{"position":[0,30,60],"target":[0,0,0]},
    "zones":[{"id":"line","rect":[-6,-41,6,-39]},{"id":"east","rect":[39,-6,41,6]},{"id":"south","rect":[-6,39,6,41]},{"id":"west","rect":[-41,-6,-39,6]}],
    "race":{"laps":1,"gates":["line","east","south","west"],"countdown_secs":1},
    "spawns":[{"id":"a","position":[-20,0,-41.5],"yaw_deg":90},{"id":"b","position":[-20,0,-38.5],"yaw_deg":90}],
    "objects":[{"id":"floor","type":"plane","size":[200,200],"position":[0,0.01,0]}]}"#;

struct Rig {
    server: Server,
    addr: SocketAddr,
    hash: u32,
    now: Instant,
}

impl Rig {
    fn new(text: &str) -> Rig {
        let scene = red_engine2::schema::parse_scene(text).unwrap();
        let hash = map_hash(text);
        let mut server =
            Server::bind(ServerConfig::new("127.0.0.1:0".parse().unwrap(), hash), MatchSim::try_new(&scene, parse_spawns(text).unwrap()).unwrap()).unwrap();
        server.set_logger(|_| {});
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        Rig { server, addr, hash, now: Instant::now() }
    }

    fn join(&mut self, nonce: u64) -> RawClient {
        let mut c = RawClient::new(self.addr, self.hash, None, nonce).unwrap();
        let (server, now) = (&mut self.server, self.now);
        c.handshake(|| {
            std::thread::sleep(Duration::from_millis(30));
            server.pump(now);
        })
        .expect("an honest handshake");
        c
    }

    /// Runs `ticks` server ticks; a client holding `forward` throttle sends one input per tick. Returns every snapshot it received, oldest first.
    fn drive(&mut self, c: &mut RawClient, seq: &mut u32, ticks: u32, forward: i8) -> Vec<Snapshot> {
        let mut out = Vec::new();
        for _ in 0..ticks {
            *seq += 1;
            c.send(&ClientMsg::Input(InputPacket { inputs: vec![PlayerInput { seq: *seq, forward, ..Default::default() }], ..Default::default() }));
            std::thread::sleep(Duration::from_millis(1));
            self.now += Duration::from_micros(16_667);
            self.server.pump(self.now);
            self.server.tick(self.now);
            std::thread::sleep(Duration::from_millis(1));
            out.extend(c.poll().into_iter().filter_map(|m| if let ServerMsg::Snapshot(s) = m { Some(s) } else { None }));
        }
        out
    }
}

#[test]
fn a_race_reaches_the_client_with_its_countdown_karts_and_progress() {
    let mut rig = Rig::new(RACE_SCENE);
    let mut c = rig.join(7001);
    let mut seq = 0;
    // The first half second: the light is red. Snapshots say so, and the kart does not move even though the client holds full throttle.
    let early = rig.drive(&mut c, &mut seq, 30, 1);
    assert!(!early.is_empty(), "snapshots arrive");
    let start_x = early[0].players[0].pos[0];
    for s in &early {
        let race = s.race.expect("a race match sends the race header in every snapshot");
        assert_eq!(race.phase, 0, "countdown");
        assert!(race.countdown_ticks > 0);
        assert!((s.players[0].pos[0] - start_x).abs() < 1e-4, "held on the grid");
        let kart = s.players[0].kart.expect("and every player's kart block");
        assert_eq!((kart.driver, kart.lap, kart.next_gate, kart.finished), (Driver::Duck.wire(), 0, 1, false));
    }
    // The light goes green (1 s countdown): the kart accelerates east along its heading.
    let later = rig.drive(&mut c, &mut seq, 150, 1);
    let last = later.last().expect("more snapshots");
    let race = last.race.unwrap();
    assert_eq!(race.phase, 1, "racing");
    assert!(race.race_tick > 30, "the race clock runs: {}", race.race_tick);
    assert!(last.players[0].pos[0] > start_x + 20.0, "the kart drove off: {} -> {}", start_x, last.players[0].pos[0]);
    assert!(last.players[0].speed > 8.0, "at kart pace: {}", last.players[0].speed);
    let kart = last.players[0].kart.unwrap();
    assert_eq!(kart.place, 1, "the only one on the grid is first");
    assert!(kart.boost_ticks == 0 && !kart.finished);
    assert!(later.iter().any(|s| s.race.unwrap().phase == 0 || s.race.unwrap().phase == 1), "phases only ever move forward");
    assert!(later.windows(2).all(|w| w[0].race.unwrap().phase <= w[1].race.unwrap().phase));
}

#[test]
fn a_match_without_a_race_sends_no_race_and_no_kart_blocks() {
    let text = r#"{"camera":{"position":[0,5,10],"target":[0,0,0]},"spawns":[{"id":"a","position":[0,0,0],"yaw_deg":0}],
        "objects":[{"id":"floor","type":"plane","size":[40,40],"position":[0,0.01,0]}]}"#;
    let mut rig = Rig::new(text);
    let mut c = rig.join(7002);
    let mut seq = 0;
    let snaps = rig.drive(&mut c, &mut seq, 60, 1);
    assert!(!snaps.is_empty());
    for s in &snaps {
        assert!(s.race.is_none(), "no race header");
        assert!(s.players.iter().all(|p| p.kart.is_none()), "no kart blocks");
    }
    assert!(snaps.last().unwrap().players[0].speed < 4.0, "and the player still walks");
}
