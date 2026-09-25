//! Multiplayer performance budgets that never flake: packet sizes and per-client bandwidth (bytes are deterministic once time is
//! under the test's control), and a generous server-tick ceiling (a catastrophic-regression gate for the sub-50 ms latency target,
//! not a benchmark). Wall-clock micro-benchmarks live in `benches/sim.rs` (`match/tick`, `net/*`); the allocation budget for the
//! match tick is in `tests/alloc_budget.rs`.
//!
//! Budgets are constants here on purpose: raising one is a decision that shows up in review.

use red_engine2::net::map_hash;
use red_engine2::net::protocol::{
    snapshot_bytes, ClientMsg, Hello, InputPacket, PlayerSnap, PropSnap, ServerMsg, Snapshot, MAX_PACKET, MAX_PLAYERS_PER_SNAPSHOT, MAX_PROPS_PER_SNAPSHOT,
    NO_PROP, PROTOCOL_VERSION,
};
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::sim::interest::InterestMap;
use red_engine2::sim::match_sim::{MatchSim, MAX_PLAYERS};
use red_engine2::sim::player::PlayerInput;
use red_engine2::sim::spawns::parse_spawns;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The worst snapshot a client is ever sent (8 players and 30 prop updates) must fit one datagram with room to spare.
const WORST_SNAPSHOT_BUDGET: usize = 1250;
/// Steady-state bandwidth per client, bytes/s, at 30 snapshots/s: 8 idle players in view, nothing else moving.
const IDLE_BYTES_PER_SEC_PER_CLIENT: f64 = 10_500.0;
/// The same with the worst case (every snapshot full of prop updates).
const WORST_BYTES_PER_SEC_PER_CLIENT: f64 = 40_000.0;
/// Average server tick (all simulation, 8 walking players in the Test Lab), microseconds. 60 Hz allows 16 667; the
/// sub-50 ms latency target leaves the tick well inside that, this is a 10x-regression tripwire on a debug-ish build.
const AVG_TICK_BUDGET_US: f64 = 2_000.0;

fn lab() -> (String, red_engine2::schema::Scene) {
    let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
    let scene = red_engine2::schema::parse_scene(&text).unwrap();
    (text, scene)
}

fn worst_snapshot() -> Snapshot {
    Snapshot {
        seq: 1,
        server_tick: 1,
        ack_input_seq: 1,
        echo_time_ms: 1,
        echo_hold_ms: 0,
        players: vec![
            PlayerSnap { id: 0, character: 0, flags: 0, pos: [1.0; 3], yaw: 0.1, pitch: 0.1, speed: 3.0, vy: 0.0, weapon: 0, held: NO_PROP, hp: 100 };
            MAX_PLAYERS_PER_SNAPSHOT
        ],
        props: vec![PropSnap { id: 0, pos: [1.0; 3], rot: [0.0, 0.0, 0.0, 1.0] }; MAX_PROPS_PER_SNAPSHOT],
    }
}

#[test]
fn the_worst_case_snapshot_and_the_bandwidth_it_implies_are_within_budget() {
    let mut buf = Vec::new();
    ServerMsg::Snapshot(worst_snapshot()).encode(&mut buf);
    assert_eq!(buf.len(), snapshot_bytes(MAX_PLAYERS_PER_SNAPSHOT, MAX_PROPS_PER_SNAPSHOT), "the size formula matches the encoder");
    assert!(buf.len() <= WORST_SNAPSHOT_BUDGET && buf.len() < MAX_PACKET, "worst snapshot is {} bytes (budget {WORST_SNAPSHOT_BUDGET})", buf.len());
    let worst_rate = buf.len() as f64 * 30.0;
    assert!(worst_rate <= WORST_BYTES_PER_SEC_PER_CLIENT, "worst-case bandwidth {worst_rate:.0} B/s per client (budget {WORST_BYTES_PER_SEC_PER_CLIENT})");
    // The client -> server direction: one 4-input packet per tick at 60 Hz.
    let mut inp = Vec::new();
    ClientMsg::Input(InputPacket { snapshot_ack: 1, client_time_ms: 1, inputs: vec![PlayerInput::default(); 4] }).encode(&mut inp);
    assert!(inp.len() as f64 * 60.0 <= 8_000.0, "upstream {} B/s", inp.len() as f64 * 60.0);
    ClientMsg::Hello(Hello { version: PROTOCOL_VERSION, map_hash: 0, character: 0, resume_token: 0 }).encode(&mut inp);
}

/// A server on loopback whose clock the test drives, with `n` synthetic clients that join and send one input per tick.
struct Session {
    server: Server,
    socks: Vec<UdpSocket>,
    addr: SocketAddr,
    now: Instant,
    seq: u32,
}

impl Session {
    fn new(n: usize, interest: bool) -> Session {
        let (text, scene) = lab();
        let sim = MatchSim::new(&scene, parse_spawns(&text).unwrap());
        let mut server = Server::bind(ServerConfig::new("127.0.0.1:0".parse().unwrap(), map_hash(&text)), sim).unwrap();
        server.set_logger(|_| {});
        if interest {
            server.set_interest(InterestMap::parse(&text).unwrap());
        }
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        let socks: Vec<UdpSocket> = (0..n).map(|_| UdpSocket::bind("127.0.0.1:0").unwrap()).collect();
        let mut s = Session { server, socks, addr, now: Instant::now(), seq: 0 };
        let hello = encode(&ClientMsg::Hello(Hello { version: PROTOCOL_VERSION, map_hash: map_hash(&text), character: 0, resume_token: 0 }));
        for sock in &s.socks {
            sock.send_to(&hello, s.addr).unwrap();
            std::thread::sleep(Duration::from_millis(5));
            s.server.pump(s.now);
        }
        assert_eq!(s.server.client_count(), n.min(MAX_PLAYERS));
        s
    }

    /// Runs `secs` of match time; every client sends one input per tick (`walk`: all walk forward, else idle).
    fn run(&mut self, secs: f64, walk: bool) {
        for _ in 0..(secs * 60.0) as u32 {
            self.now += Duration::from_micros(16_667);
            self.seq += 1;
            let p = encode(&ClientMsg::Input(InputPacket {
                snapshot_ack: 0,
                client_time_ms: 0,
                inputs: vec![PlayerInput { seq: self.seq, forward: walk as i8, yaw: 0.3, ..Default::default() }],
            }));
            for s in &self.socks {
                let _ = s.send_to(&p, self.addr);
            }
            self.server.pump(self.now);
            self.server.tick(self.now);
        }
    }
}

fn encode(m: &ClientMsg) -> Vec<u8> {
    let mut b = Vec::new();
    m.encode(&mut b);
    b
}

#[test]
fn eight_idle_players_cost_each_client_under_the_idle_bandwidth_budget() {
    let mut s = Session::new(8, false);
    s.run(1.0, false); // let the players settle
    let before = s.server.stats().bytes_out;
    s.run(4.0, false);
    let rate = (s.server.stats().bytes_out - before) as f64 / 4.0 / 8.0;
    println!("idle: {rate:.0} B/s per client (budget {IDLE_BYTES_PER_SEC_PER_CLIENT}); snapshot = {} B", snapshot_bytes(8, 0));
    assert!(rate <= IDLE_BYTES_PER_SEC_PER_CLIENT, "idle bandwidth {rate:.0} B/s per client exceeds {IDLE_BYTES_PER_SEC_PER_CLIENT}");
    assert!(rate > 5_000.0, "sanity: 30 snapshots/s of 8 players is real traffic ({rate:.0} B/s)");
}

#[test]
fn interest_management_cuts_the_per_client_bandwidth_of_a_spread_out_match() {
    let (mut all, mut near) = (Session::new(6, false), Session::new(6, true));
    for s in [&mut all, &mut near] {
        s.run(1.0, false);
    }
    let (b0, b1) = (all.server.stats().bytes_out, near.server.stats().bytes_out);
    all.run(3.0, false);
    near.run(3.0, false);
    let (rate_all, rate_near) = ((all.server.stats().bytes_out - b0) as f64 / 3.0 / 6.0, (near.server.stats().bytes_out - b1) as f64 / 3.0 / 6.0);
    println!("6 players (4 in the hall, 2 in the props room): {rate_all:.0} B/s per client without interest, {rate_near:.0} with");
    assert!(rate_near < rate_all * 0.8, "interest management must save at least 20%: {rate_near:.0} vs {rate_all:.0}");
}

#[test]
fn the_server_tick_stays_far_inside_the_frame_budget_with_a_full_match_walking() {
    let mut s = Session::new(8, true);
    s.run(5.0, true);
    let st = s.server.stats();
    let avg = st.tick_us_total as f64 / st.ticks.max(1) as f64;
    println!("8 walking players: average tick {avg:.0} us, worst {} us over {} ticks", st.tick_us_worst, st.ticks);
    assert!(avg <= AVG_TICK_BUDGET_US, "average server tick {avg:.0} us exceeds the {AVG_TICK_BUDGET_US} us budget (16 667 us is a whole 60 Hz frame)");
}
