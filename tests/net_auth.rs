//! Who may join and who sent this datagram (ADR 0028), tested from both sides: a stranger cannot kick, move, replay into or impersonate a
//! session, a join key is proven and never sent, a captured handshake is useless from another address, and a client believes only
//! datagrams whose tag verifies. The server's clock is under the test's control, as in `net_abuse.rs`.

use red_engine2::net::auth::{Direction, SessionKey};
use red_engine2::net::client::{ClientConfig, ConnState, NetClient};
use red_engine2::net::map_hash;
use red_engine2::net::protocol::*;
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::net::testkit::RawClient;
use red_engine2::sim::match_sim::MatchSim;
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
    fn new(key: Option<&str>) -> Rig {
        let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
        let scene = red_engine2::schema::parse_scene(&text).unwrap();
        let mut spawns = parse_spawns(&text).unwrap();
        spawns.retain(|s| s.group == "duel");
        let hash = map_hash(&text);
        let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), hash);
        cfg.join_key = key.map(str::to_string);
        let mut server = Server::bind(cfg, MatchSim::try_new(&scene, spawns).unwrap()).unwrap();
        server.set_logger(|_| {});
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        Rig { server, addr, hash, now: Instant::now(), nonce: 500 }
    }

    fn pump(&mut self) {
        std::thread::sleep(Duration::from_millis(30));
        self.server.pump(self.now);
    }

    fn advance(&mut self, secs: f64) {
        for _ in 0..(secs * 60.0) as u32 {
            self.now += Duration::from_micros(16_667);
            self.server.pump(self.now);
            self.server.tick(self.now);
        }
    }

    fn client(&mut self, key: Option<&str>) -> RawClient {
        self.nonce += 1;
        RawClient::new(self.addr, self.hash, key, self.nonce).unwrap()
    }

    fn join(&mut self, key: Option<&str>) -> (RawClient, Welcome) {
        let mut c = self.client(key);
        let (server, now) = (&mut self.server, self.now);
        let w = c
            .handshake(|| {
                std::thread::sleep(Duration::from_millis(30));
                server.pump(now);
            })
            .expect("handshake");
        (c, w)
    }
}

fn input(seq: u32, forward: i8) -> ClientMsg {
    ClientMsg::Input(InputPacket { inputs: vec![PlayerInput { seq, forward, yaw: 1.5, ..Default::default() }], ..Default::default() })
}

fn position(rig: &Rig, w: &Welcome) -> (f32, f32) {
    let p = rig.server.sim().player(w.player_id as usize).expect("player exists").state;
    (p.pos.x, p.pos.y)
}

#[test]
fn a_stranger_cannot_kick_or_steer_a_player_it_only_knows_the_address_of() {
    let mut rig = Rig::new(None);
    let (victim, w) = rig.join(None);
    let before = position(&rig, &w);
    let mut attacker = rig.client(None);
    // The attacker sends from its own socket: a Bye, and an Input, each with a tag made from a key it invented.
    let fake = SessionKey::derive(b"", 1, 2);
    for msg in [ClientMsg::Bye, input(1, 1)] {
        let mut b = Vec::new();
        msg.encode(&mut b);
        fake.sign(Direction::ToServer, &mut b);
        attacker.send_raw(&b);
    }
    // ...and the same forgeries from the victim's own address (what IP spoofing would give it): the tag is still wrong.
    for msg in [ClientMsg::Bye, input(2, 1)] {
        let mut b = Vec::new();
        msg.encode(&mut b);
        fake.sign(Direction::ToServer, &mut b);
        victim.send_raw(&b);
        let mut untagged = Vec::new();
        msg.encode(&mut untagged);
        victim.send_raw(&untagged);
    }
    rig.pump();
    attacker.poll();
    rig.advance(1.0);
    assert_eq!(rig.server.client_count(), 1, "the victim was not kicked");
    assert_eq!(position(&rig, &w), before, "the victim was not steered");
    assert!(rig.server.stats().bad_tags >= 4, "forgeries from the victim's address fail the tag check: {}", rig.server.stats().bad_tags);
}

#[test]
fn a_packet_recorded_from_one_session_does_nothing_in_another() {
    let mut rig = Rig::new(None);
    let (a, wa) = rig.join(None);
    let (b, wb) = rig.join(None);
    let recorded = a.signed_bytes(&input(1, 1)); // a genuine, correctly tagged packet of A's session
    let (pa, pb) = (position(&rig, &wa), position(&rig, &wb));
    b.send_raw(&recorded); // an eavesdropper replays it from B's address
    rig.pump();
    rig.advance(0.5);
    assert_eq!(position(&rig, &wb), pb, "B did not move: A's tag does not verify under B's key");
    assert!(rig.server.stats().bad_tags >= 1);
    // The genuine packet still works from A.
    a.send_raw(&recorded);
    rig.pump();
    rig.advance(0.5);
    assert_ne!(position(&rig, &wa), pa, "A's own packet is accepted");
}

#[test]
fn flipping_one_bit_of_a_tagged_packet_makes_the_server_drop_it() {
    let mut rig = Rig::new(None);
    let (c, w) = rig.join(None);
    let before = position(&rig, &w);
    let good = c.signed_bytes(&input(1, 1));
    for i in 0..good.len() {
        let mut bad = good.clone();
        bad[i] ^= 0x10;
        c.send_raw(&bad);
    }
    rig.pump();
    rig.advance(0.5);
    assert_eq!(position(&rig, &w), before, "no tampered variant moved the player");
    c.send_raw(&good);
    rig.pump();
    rig.advance(0.5);
    assert_ne!(position(&rig, &w), before, "but the untouched original did");
}

#[test]
fn a_join_key_is_proven_not_sent_and_a_captured_handshake_is_useless_from_another_address() {
    let mut rig = Rig::new(Some("s3cret-join-key"));
    // The wrong key is refused, a missing key is refused (the client learns why from the Challenge), the right key joins.
    let mut wrong = rig.client(Some("nope"));
    let (server, now) = (&mut rig.server, rig.now);
    let r = wrong.handshake(|| {
        std::thread::sleep(Duration::from_millis(30));
        server.pump(now);
    });
    assert_eq!(r, Err("rejected: BadKey".to_string()));
    let mut none = rig.client(None);
    let (server, now) = (&mut rig.server, rig.now);
    let r = none.handshake(|| {
        std::thread::sleep(Duration::from_millis(30));
        server.pump(now);
    });
    assert_eq!(r, Err("rejected: BadKey".to_string()), "a raw client with no key sends a zero proof: refused");
    assert!(none.requires_key, "and the Challenge told it a key is needed");
    let mut good = rig.client(Some("s3cret-join-key"));
    // Capture the second Hello (the one that carries the proof) of the honest join.
    good.send_hello();
    rig.pump();
    good.poll(); // reads the Challenge and sends Hello #2
    let captured = good.hello_bytes();
    assert!(!captured.windows(15).any(|w| w == b"s3cret-join-key"), "the key never appears on the wire");
    rig.pump();
    good.poll();
    assert!(good.welcome.is_some(), "the right key joins");
    // An eavesdropper replays that exact Hello from its own address: the cookie belongs to the honest client's address, so it only
    // earns a fresh Challenge, never a session.
    let joined = rig.server.client_count();
    let mut replay = rig.client(None);
    replay.send_raw(&captured);
    rig.pump();
    let got = replay.poll();
    assert!(matches!(got.first(), Some(ServerMsg::Challenge { .. })), "{got:?}");
    assert_eq!(rig.server.client_count(), joined, "no session for the replayer");
    assert_eq!(rig.server.stats().bad_keys, 2);
}

#[test]
fn a_hello_from_an_older_protocol_version_gets_a_polite_rejection() {
    let mut rig = Rig::new(None);
    let mut c = rig.client(None);
    // A version-2 Hello: magic, version, kind, then its shorter body.
    let mut old = vec![0x44, 0x52, 2, 0, 1, 2, 0];
    old.extend_from_slice(&rig.hash.to_le_bytes());
    old.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0]);
    c.send_raw(&old);
    rig.pump();
    assert_eq!(c.poll(), vec![ServerMsg::Reject(RejectReason::Version)]);
    assert_eq!(rig.server.client_count(), 0);
}

#[test]
fn a_challenge_reply_is_smaller_than_the_hello_that_asked_for_it_so_a_spoofer_gains_nothing() {
    let mut rig = Rig::new(None);
    let c = rig.client(None);
    let hello = c.hello_bytes();
    c.send_hello();
    rig.pump();
    let mut buf = [0u8; 2048];
    c.sock.set_nonblocking(false).unwrap();
    c.sock.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    let (n, _) = c.sock.recv_from(&mut buf).unwrap();
    assert!(n < hello.len(), "Challenge is {n} bytes, Hello is {}: no amplification", hello.len());
}

/// A pretend server on a raw socket, to test what the *client* believes.
struct FakeServer {
    sock: UdpSocket,
}

impl FakeServer {
    fn recv_hello(&self) -> (SocketAddr, Hello) {
        let mut buf = [0u8; 2048];
        self.sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let (n, from) = self.sock.recv_from(&mut buf).expect("the client's Hello");
        match ClientMsg::decode(&buf[..n]).expect("a Hello") {
            ClientMsg::Hello(h) => (from, h),
            other => panic!("expected Hello, got {other:?}"),
        }
    }

    fn send(&self, to: SocketAddr, msg: &ServerMsg, key: Option<&SessionKey>) {
        let mut b = Vec::new();
        msg.encode(&mut b);
        if let Some(k) = key {
            k.sign(Direction::ToClient, &mut b);
        }
        self.sock.send_to(&b, to).unwrap();
    }
}

#[test]
fn a_client_believes_only_datagrams_whose_tag_verifies() {
    let fake = FakeServer { sock: UdpSocket::bind("127.0.0.1:0").unwrap() };
    let server_addr = fake.sock.local_addr().unwrap();
    let mut client = NetClient::connect_with(ClientConfig::new(server_addr, 0, 7, 0)).unwrap();
    let poll = |c: &mut NetClient, ms: u64| {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            c.poll(Instant::now());
            std::thread::sleep(Duration::from_millis(3));
        }
    };
    poll(&mut client, 20); // sends the first Hello
    let (client_addr, h1) = fake.recv_hello();
    assert_eq!(h1.cookie, 0, "the first Hello carries no cookie");
    fake.send(client_addr, &ServerMsg::Challenge { cookie: 0xC0FFEE, requires_key: false }, None);
    poll(&mut client, 50);
    let (_, h2) = fake.recv_hello();
    assert_eq!((h2.cookie, h2.client_nonce), (0xC0FFEE, h1.client_nonce), "the second Hello answers the Challenge");
    let right = SessionKey::derive(b"", h2.client_nonce, 0xC0FFEE);
    let welcome =
        Welcome { player_id: 1, token: 99, tick_rate: 60, snapshot_every: 2, server_tick: 0, spawn: [0.0; 4], character: 0, round: 0, in_round: true };
    // A Welcome with the wrong tag, and one with no tag, do not connect the client.
    fake.send(client_addr, &ServerMsg::Welcome(welcome), Some(&SessionKey::derive(b"", 1, 2)));
    fake.send(client_addr, &ServerMsg::Welcome(welcome), None);
    poll(&mut client, 80);
    assert_eq!(client.state(), ConnState::Connecting, "forged Welcomes are ignored");
    assert!(client.stats().bad_packets >= 2);
    fake.send(client_addr, &ServerMsg::Welcome(welcome), Some(&right));
    poll(&mut client, 80);
    assert_eq!(client.state(), ConnState::Connected, "the genuine Welcome is believed");
    // Unauthenticated hints cannot disturb a healthy session: NoSession, a forged Bye, a forged Reject.
    fake.send(client_addr, &ServerMsg::NoSession, None);
    fake.send(client_addr, &ServerMsg::Reject(RejectReason::Full), None);
    fake.send(client_addr, &ServerMsg::Bye, Some(&SessionKey::derive(b"", 5, 6)));
    poll(&mut client, 100);
    assert_eq!(client.state(), ConnState::Connected);
    // A genuine Bye is obeyed (the client goes back to rejoining).
    fake.send(client_addr, &ServerMsg::Bye, Some(&right));
    poll(&mut client, 100);
    assert_eq!(client.state(), ConnState::Reconnecting);
}

#[test]
fn a_client_with_a_key_will_not_join_a_server_that_does_not_ask_for_one() {
    let fake = FakeServer { sock: UdpSocket::bind("127.0.0.1:0").unwrap() };
    let mut cfg = ClientConfig::new(fake.sock.local_addr().unwrap(), 0, 7, 0);
    cfg.join_key = Some("a-key".to_string());
    let mut client = NetClient::connect_with(cfg).unwrap();
    client.poll(Instant::now());
    let (client_addr, _) = fake.recv_hello();
    fake.send(client_addr, &ServerMsg::Challenge { cookie: 5, requires_key: false }, None);
    std::thread::sleep(Duration::from_millis(50));
    client.poll(Instant::now());
    assert_eq!(client.state(), ConnState::Rejected(RejectReason::ServerIsOpen), "an open server that a keyed client reaches may be an impostor");
}

#[test]
fn a_recorded_lobby_packet_replayed_later_cannot_undo_a_newer_choice() {
    use red_engine2::net::protocol::LobbyCmd;
    let mut rig = Rig::new(None);
    let (mut c, _w) = rig.join(None);
    let ready_at_100 = c.signed_bytes(&ClientMsg::Lobby(LobbyCmd { ready: true, client_time_ms: 100, ..Default::default() }));
    let unready_at_50 = c.signed_bytes(&ClientMsg::Lobby(LobbyCmd { ready: false, client_time_ms: 50, ..Default::default() }));
    c.send_raw(&ready_at_100);
    rig.pump();
    c.send_raw(&unready_at_50); // genuine (correctly tagged) but older: a replay or a reordered packet
    rig.pump();
    assert_eq!(rig.server.client_count(), 1, "still connected: an old packet is ignored, not an error");
    // The server's copy of "ready" is visible in the next Status.
    rig.advance(0.5);
    let mut ready = None;
    for m in c.poll() {
        if let ServerMsg::Status(st) = m {
            ready = st.roster.first().map(|e| e.flags & ROSTER_READY != 0);
        }
    }
    assert_eq!(ready, Some(true), "the newer ready survived the replayed older packet");
}
