//! A hostile network against the real [`Server`] on real UDP sockets, with time under the test's control
//! (`Server::pump(now)` / `tick(now)` take the clock as an argument, so a flood and its aftermath run in
//! milliseconds and deterministically). The server must never panic, never let ghosts fill the match, keep its
//! memory bounded, and still serve an honest client afterwards.

use red_engine2::net::map_hash;
use red_engine2::net::protocol::{ClientMsg, InputPacket, MAX_PACKET};
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::net::testkit::RawClient;
use red_engine2::sim::match_sim::{MatchSim, MAX_PLAYERS};
use red_engine2::sim::player::PlayerInput;
use red_engine2::sim::spawns::parse_spawns;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct Rig {
    server: Server,
    addr: SocketAddr,
    hash: u32,
    now: Instant,
    nonce: u64,
}

impl Rig {
    fn new() -> Rig {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json");
        let text = std::fs::read_to_string(&path).unwrap();
        let scene = red_engine2::schema::parse_scene(&text).unwrap();
        let mut spawns = parse_spawns(&text).unwrap();
        spawns.retain(|s| s.group == "duel");
        let hash = map_hash(&text);
        let mut server = Server::bind(ServerConfig::new("127.0.0.1:0".parse().unwrap(), hash), MatchSim::try_new(&scene, spawns).unwrap()).unwrap();
        server.set_logger(|_| {});
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        let t0 = Instant::now();
        Rig { server, addr, hash, now: t0, nonce: 1000 }
    }

    /// Lets the server read what has arrived, at the rig's clock.
    fn pump(&mut self) {
        // Loopback delivery is not instantaneous: give the OS a moment to hand the datagrams over.
        std::thread::sleep(Duration::from_millis(30));
        self.server.pump(self.now);
    }

    /// Advances the rig's clock by `secs`, ticking the server 60 times a second.
    fn advance(&mut self, secs: f64) {
        for _ in 0..(secs * 60.0) as u32 {
            self.now += Duration::from_micros(16_667);
            self.server.pump(self.now);
            self.server.tick(self.now);
        }
    }

    fn client(&mut self) -> RawClient {
        self.nonce += 1;
        RawClient::new(self.addr, self.hash, None, self.nonce).unwrap()
    }

    /// A client that has completed the real handshake.
    fn join(&mut self) -> (RawClient, red_engine2::net::protocol::Welcome) {
        let mut c = self.client();
        let (server, now) = (&mut self.server, self.now);
        let w = c
            .handshake(|| {
                std::thread::sleep(Duration::from_millis(30));
                server.pump(now);
            })
            .expect("an honest handshake");
        (c, w)
    }
}

fn input_packet(seq: u32, forward: i8, yaw: f32) -> ClientMsg {
    ClientMsg::Input(InputPacket { inputs: vec![PlayerInput { seq, forward, yaw, ..Default::default() }], ..Default::default() })
}

struct Xorshift(u64);
impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// An honest client can still join and be welcomed.
fn assert_an_honest_client_can_join(rig: &mut Rig) {
    let (_c, w) = rig.join();
    assert_ne!(w.token, 0);
}

#[test]
fn garbage_and_mutated_packets_never_panic_and_the_server_keeps_serving() {
    let mut rig = Rig::new();
    let (joined, _) = rig.join();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut rng = Xorshift(0x9e37_79b9_7f4a_7c15);
    // Seeds: a Hello, an untagged Input, a genuinely tagged Input (sent from the joined client's own socket so it reaches the tag check).
    let seeds = [
        joined.hello_bytes(),
        {
            let mut b = Vec::new();
            input_packet(1, 1, 0.0).encode(&mut b);
            b
        },
        joined.signed_bytes(&input_packet(2, 1, 0.0)),
        joined.signed_bytes(&ClientMsg::Lobby(Default::default())),
    ];
    let mut sent = 0u64;
    for round in 0..40 {
        for _ in 0..300 {
            let mut p = match rng.next() % 4 {
                0 => (0..(rng.next() % 1500) as usize).map(|_| rng.next() as u8).collect(), // pure noise, up to over-MTU
                1 => seeds[(rng.next() % 4) as usize].clone(),                              // valid
                2 => {
                    let mut m = seeds[(rng.next() % 4) as usize].clone(); // bit-flipped
                    let i = (rng.next() as usize) % m.len();
                    m[i] ^= 1 << (rng.next() % 8);
                    m
                }
                _ => {
                    let mut m = seeds[(rng.next() % 4) as usize].clone(); // truncated or padded
                    let n = (rng.next() as usize) % (m.len() + 8);
                    m.resize(n, 0xAB);
                    m
                }
            };
            p.truncate(MAX_PACKET + 300);
            // Mutations of the tagged packets go out from the joined client's address so they meet the tag check, the rest from a stranger.
            if rng.next().is_multiple_of(2) {
                joined.send_raw(&p);
            } else {
                let _ = sock.send_to(&p, rig.addr);
            }
            sent += 1;
        }
        rig.pump();
        rig.advance(0.05 + round as f64 * 0.001);
    }
    assert!(sent >= 12_000);
    let s = rig.server.stats().clone();
    assert!(s.bad_packets > 1000, "most noise is refused: {}", s.bad_packets);
    assert!(s.bad_tags > 100, "tampered packets from a known session fail the tag check: {}", s.bad_tags);
    assert!(rig.server.client_count() <= MAX_PLAYERS, "sessions stay bounded: {}", rig.server.client_count());
    rig.advance(4.0); // everything the flood created times out
    assert_eq!(rig.server.client_count(), 0, "ghost sessions expire");
    assert_an_honest_client_can_join(&mut rig);
}

#[test]
fn a_join_flood_from_many_sources_is_throttled_and_cannot_hold_the_match_hostage() {
    let mut rig = Rig::new();
    // 300 distinct source addresses all say Hello in the same instant. None of them can finish the handshake (a spoofed source never
    // sees the Challenge), so no session is created at all, and the replies stay within the join budget.
    let clients: Vec<RawClient> = (0..300).map(|_| rig.client()).collect();
    for c in &clients {
        c.send_hello();
    }
    rig.pump();
    rig.pump();
    let s = rig.server.stats().clone();
    assert!(s.hellos_throttled > 100, "the join budget cut the flood off: {} throttled", s.hellos_throttled);
    assert!(s.challenges <= 130, "no more challenges than the budget allows: {}", s.challenges);
    assert_eq!(rig.server.client_count(), 0, "a Hello alone never creates a session: state is only kept for a proven address");
    rig.advance(4.0); // the join budget refills
    assert_an_honest_client_can_join(&mut rig);
}

#[test]
fn resume_memory_is_bounded_however_many_players_come_and_go() {
    let mut rig = Rig::new();
    // 8 joiners at a time, over and over; every one that times out is parked for resume (30 s).
    for wave in 0..20 {
        let mut clients: Vec<RawClient> = (0..MAX_PLAYERS).map(|_| rig.client()).collect();
        for c in &clients {
            c.send_hello();
        }
        rig.pump();
        for c in clients.iter_mut() {
            c.poll(); // reads the Challenge and sends the second Hello
        }
        rig.pump();
        assert!(rig.server.client_count() >= 1, "wave {wave} joined");
        rig.advance(3.5); // they go silent and time out
    }
    assert!(rig.server.stats().leaves > 40, "many players came and went: {}", rig.server.stats().leaves);
    let parked = rig.server.parked_count();
    assert!(parked > 0 && parked <= red_engine2::net::limits::MAX_PARKED, "resume memory is capped: {parked}");
    assert_an_honest_client_can_join(&mut rig);
}

#[test]
fn an_input_flood_is_rate_limited_and_hostile_values_cannot_move_a_player_off_the_map_or_into_nan() {
    let mut rig = Rig::new();
    let (c, w) = rig.join();
    let start = w.spawn;
    // 2000 input packets in one instant: NaN, infinities, absurd yaw, out-of-range strafe/forward, wild seq jumps.
    let wild = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30, 0.0, 1.5];
    let mut rng = Xorshift(42);
    for k in 0..2000u32 {
        let yaw = wild[(rng.next() % wild.len() as u64) as usize];
        let seq = if k % 7 == 0 { rng.next() as u32 } else { k + 1 };
        c.send(&input_packet(seq, 100, yaw));
        // Drain in batches: a real server reads as packets arrive, and Linux's loopback receive buffer (about 200 KB, counted in
        // per-packet overhead) silently drops a 2000-packet burst nobody has read yet, which would make the count below meaningless.
        if k % 100 == 99 {
            rig.pump();
        }
    }
    rig.pump();
    rig.pump();
    assert!(rig.server.stats().rate_limited > 1500, "the packet budget held: {} dropped", rig.server.stats().rate_limited);
    rig.advance(2.0);
    let p = rig.server.sim().player(w.player_id as usize).expect("the player is still connected").state;
    assert!(p.pos.is_finite() && p.foot_y.is_finite() && p.vy.is_finite() && p.yaw.is_finite() && p.pitch.is_finite(), "state stayed finite: {p:?}");
    let moved = ((p.pos.x - start[0]).powi(2) + (p.pos.y - start[2]).powi(2)).sqrt();
    assert!(moved < 40.0, "no teleporting: moved {moved} m");
    assert_an_honest_client_can_join(&mut rig);
}

#[test]
fn a_spoofed_resume_token_does_not_steal_a_player() {
    let mut rig = Rig::new();
    let (_victim, w) = rig.join();
    // An attacker guesses tokens (never the right one): every guess is a fresh join, not a takeover.
    for guess in 1..=50u64 {
        let mut attacker = rig.client();
        attacker.resume_token = guess.wrapping_mul(0x9e37_79b9);
        attacker.send_hello();
        rig.pump();
        attacker.poll();
        rig.pump();
    }
    assert!(rig.server.sim().player(w.player_id as usize).is_some(), "the victim's player still exists");
    assert_eq!(rig.server.stats().resumes, 0, "no wrong token was accepted as a resume");
}
