//! The production transport end to end (ADR 0044): a real `Server` on QUIC + TLS 1.3 and real clients, over loopback, some through the
//! seeded lossy/reordering proxy. Every identity here is generated for the test run; no key is stored in the repository.
//!
//! Covered: two scripted players completing a rules game under loss and reordering; a wrong server identity (fails closed, never falls
//! back); unauthorized joins; malformed application messages; the connection limit and the bounded inbound queue; reconnect after an
//! outage; and shutdown. Measurements (round trip, bytes, datagram sizes) are printed for the ADR.

use red_engine2::net::bot::{Behavior, Bot, ClientWorld};
use red_engine2::net::client::{ClientConfig, ConnState, NetClient};
use red_engine2::net::netsim::{profile, LossyProxy};
use red_engine2::net::protocol::RejectReason;
use red_engine2::net::quic::{QuicClient, QuicServer, QuicServerOptions, ServerIdentity, ServerTrust, INBOUND_QUEUE};
use red_engine2::net::server::{Server, ServerConfig, ServerStats};
use red_engine2::net::transport::{ClientTransport, Security, ServerTransport, TransportStats, TransportStatus};
use red_engine2::player::Character;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const SCENE: &str = r##"{"camera":{"position":[-6,1.7,0],"target":[0,1,0]},
 "zones":[{"id":"goal","rect":[4,-2,6,2],"y":0}],
 "spawns":[{"id":"a","position":[-6,0,-1],"yaw_deg":90},{"id":"b","position":[-6,0,1],"yaw_deg":90}],
 "vars":{"coins":0},
 "rules":[
  {"id":"take_coin","when":{"enter":{"object":"coin","pad":0.4}},"once":true,"do":[{"add":["coins",1]},{"hide":"coin"},{"emit":"coin"}]},
  {"id":"win","when":{"enter":{"zone":"goal"}},"if":"coins >= 1","do":[{"emit":"victory"},{"end":"victory"}]}],
 "objects":[{"id":"floor","type":"plane","size":[20,10],"position":[0,0.01,0]},
            {"id":"coin","type":"cylinder","radius":0.25,"height":0.08,"position":[0,0.45,0],"collide":false}]}"##;

fn scene_file() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("red-net-quic-{}", red_engine2::crypto::random_u64().unwrap()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("coin_race.json");
    std::fs::write(&p, SCENE).unwrap();
    p
}

struct Running {
    addr: SocketAddr,
    fingerprint: String,
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Server>,
}

impl Running {
    fn finish(self) -> Server {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.join().unwrap()
    }
}

fn start(identity: &ServerIdentity, bind: &str, key: Option<&str>, opts: QuicServerOptions) -> Running {
    let scene = red_engine2::schema::parse_scene(SCENE).unwrap();
    let mut cfg = ServerConfig::new(bind.parse().unwrap(), red_engine2::net::map_hash(SCENE));
    cfg.join_key = key.map(str::to_string);
    let transport = QuicServer::bind(bind.parse().unwrap(), identity, opts).unwrap();
    let fingerprint = transport.fingerprint().to_string();
    let mut server = Server::with_transport(cfg, MatchSim::new(&scene, parse_spawns(SCENE).unwrap()), Box::new(transport)).unwrap();
    server.set_logger(|_| {});
    assert_eq!(server.security(), Security::Quic);
    let addr = server.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let handle = std::thread::spawn(move || {
        server.run(&s2);
        server
    });
    Running { addr, fingerprint, stop, handle }
}

fn identity() -> ServerIdentity {
    ServerIdentity::generate(&["localhost".into()]).unwrap().identity
}

fn quic_cfg(to: SocketAddr, fingerprint: &str, character: u8) -> ClientConfig {
    ClientConfig::quic(to, character, red_engine2::net::map_hash(SCENE), ServerTrust::fingerprint(fingerprint).unwrap(), "localhost")
}

fn poll_until(client: &mut NetClient, secs: f64, mut done: impl FnMut(&NetClient) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    while Instant::now() < end {
        client.poll(Instant::now());
        if done(client) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    false
}

fn describe(s: &ServerStats, t: &TransportStats, secs: f64) -> String {
    format!(
        "server: {} joins {} resumes, in {:.1} KB/s out {:.1} KB/s, {} snapshots, {} bad packets | transport: {} connections, {} refused, {} failed handshakes, {} queue drops, {} stream messages",
        s.joins,
        s.resumes,
        s.bytes_in as f64 / 1024.0 / secs,
        s.bytes_out as f64 / 1024.0 / secs,
        s.snapshots_sent,
        s.bad_packets,
        t.connections_accepted,
        t.connections_refused,
        t.handshakes_failed,
        t.queue_dropped,
        t.sent_on_stream
    )
}

#[test]
fn two_scripted_players_finish_a_rules_game_over_quic_under_loss_and_reordering() {
    let id = identity();
    let server = start(&id, "127.0.0.1:0", Some("a long random join key"), QuicServerOptions::default());
    let bad = profile("bad").unwrap(); // 5% bursty loss, 50 ms +- 25 ms each way (reordering), 1% duplicates
    let map = scene_file();
    let mut proxies = Vec::new();
    let mut bots = Vec::new();
    for (i, route) in [vec![(-6.0, -1.0), (0.0, 0.0), (5.0, 0.0)], vec![(-6.0, 1.0), (0.0, 1.0), (5.0, 1.0)]].into_iter().enumerate() {
        let proxy = LossyProxy::start(server.addr, bad, 7 + i as u64).unwrap();
        let (_scene, world) = ClientWorld::load(&map).unwrap();
        let mut cfg = quic_cfg(proxy.addr, &server.fingerprint, 0);
        cfg.join_key = Some("a long random join key".into());
        cfg.name = format!("bot{i}");
        let client = NetClient::connect_with(cfg).unwrap();
        assert_eq!(client.security(), Security::Quic);
        bots.push(Bot::with_client(client, Character::Human, world, Behavior::Waypoints { points: route, sprint: false }).unwrap());
        proxies.push(proxy);
    }
    let t0 = Instant::now();
    let mut seen_each_other = [false; 2];
    let mut max_rtt: f32 = 0.0;
    while t0.elapsed() < Duration::from_secs(25) {
        let now = Instant::now();
        for (i, b) in bots.iter_mut().enumerate() {
            b.pump(now);
            seen_each_other[i] |= !b.frame(now).remote.is_empty();
            max_rtt = max_rtt.max(b.client.stats().rtt_ms);
        }
        if bots.iter().all(|b| b.client.rule_state().is_some_and(|s| s.outcome == "victory")) {
            break;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    let secs = t0.elapsed().as_secs_f64();
    for (i, b) in bots.iter().enumerate() {
        let st = b.client.rule_state().unwrap_or_else(|| panic!("bot {i} has no rule state: {:?}", b.events));
        assert_eq!(st.outcome, "victory", "bot {i}: {:?}", b.events);
        assert_eq!(st.vars.iter().find(|v| v.name == "coins").map(|v| v.value), Some(1.0));
        assert!(!st.hidden.is_empty(), "the coin is hidden for bot {i}");
        assert_eq!(b.client.stats().connects, 1, "bot {i} never had to reconnect: {:?}", b.events);
    }
    assert!(seen_each_other.iter().all(|s| *s), "each player saw the other: {seen_each_other:?}");
    let rtts: Vec<f32> = bots.iter().map(|b| b.client.stats().rtt_ms).collect();
    let snaps: Vec<(u64, u64)> = bots.iter().map(|b| (b.client.stats().snapshots, b.client.stats().snapshots_missed)).collect();
    for mut b in bots {
        b.client.disconnect();
    }
    let reports: Vec<_> = proxies.into_iter().map(|p| p.finish()).collect();
    let server = server.finish();
    let (s, t) = (server.stats().clone(), server.transport_stats());
    eprintln!("MEASURE quic+bad: finished in {secs:.2} s; client rtt {rtts:?} ms (worst {max_rtt:.0}); snapshots (received, missed) {snaps:?}");
    eprintln!("MEASURE quic+bad: {}", describe(&s, &t, secs));
    for r in &reports {
        eprintln!("MEASURE proxy: received {:?} dropped {:?} duplicated {:?} (loss {:.1}%)", r.received, r.dropped, r.duplicated, r.loss_fraction() * 100.0);
        assert!(r.dropped[0] + r.dropped[1] > 0, "the proxy really dropped datagrams");
    }
    assert_eq!(s.joins, 2);
    assert_eq!(s.bad_keys, 0);
    assert_eq!(t.handshakes_failed, 0);
}

#[test]
fn a_wrong_server_identity_is_refused_and_never_downgraded() {
    let server = start(&identity(), "127.0.0.1:0", None, QuicServerOptions::default());
    let impostor_pin = identity(); // the fingerprint the player was given belongs to another server
    let mut client = NetClient::connect_with(quic_cfg(server.addr, &impostor_pin.fingerprint(), 0)).unwrap();
    assert!(poll_until(&mut client, 5.0, |c| matches!(c.state(), ConnState::Rejected(_))), "the client gave up");
    assert_eq!(client.state(), ConnState::Rejected(RejectReason::ServerIdentity));
    assert!(client.transport_error().is_some_and(|e| e.contains("identity mismatch")), "{:?}", client.transport_error());
    assert_eq!(client.security(), Security::Quic, "still QUIC: nothing fell back to UDP");
    // It stays refused: polling again does not retry.
    assert!(!poll_until(&mut client, 1.0, |c| c.state() != ConnState::Rejected(RejectReason::ServerIdentity)));
    // A CA-based client that does not trust this self-signed certificate is refused the same way.
    let dir = std::env::temp_dir().join(format!("red-ca-{}", red_engine2::crypto::random_u64().unwrap()));
    std::fs::create_dir_all(&dir).unwrap();
    let other = ServerIdentity::generate(&["localhost".into()]).unwrap();
    std::fs::write(dir.join("ca.pem"), &other.cert_pem).unwrap();
    let cfg = ClientConfig {
        transport: red_engine2::net::client::ClientTransportConfig::Quic {
            trust: ServerTrust::roots_file(&dir.join("ca.pem")).unwrap(),
            server_name: "localhost".into(),
            relay_claim: None,
        },
        ..ClientConfig::new(server.addr, 0, red_engine2::net::map_hash(SCENE), 0)
    };
    let mut ca_client = NetClient::connect_with(cfg).unwrap();
    assert!(poll_until(&mut ca_client, 5.0, |c| c.state() == ConnState::Rejected(RejectReason::ServerIdentity)), "{:?}", ca_client.transport_error());
    let server = server.finish();
    assert_eq!(server.stats().joins, 0, "no session was created for either");
}

#[test]
fn joins_need_the_key_and_a_key_never_travels_or_verifies_elsewhere() {
    let server = start(&identity(), "127.0.0.1:0", Some("correct horse battery staple"), QuicServerOptions::default());
    let mut none = NetClient::connect_with(quic_cfg(server.addr, &server.fingerprint, 0)).unwrap();
    assert!(poll_until(&mut none, 5.0, |c| matches!(c.state(), ConnState::Rejected(_))));
    assert_eq!(none.state(), ConnState::Rejected(RejectReason::NeedsKey));
    let mut wrong_cfg = quic_cfg(server.addr, &server.fingerprint, 0);
    wrong_cfg.join_key = Some("wrong".into());
    let mut wrong = NetClient::connect_with(wrong_cfg).unwrap();
    assert!(poll_until(&mut wrong, 5.0, |c| matches!(c.state(), ConnState::Rejected(_))));
    assert_eq!(wrong.state(), ConnState::Rejected(RejectReason::BadKey));
    let mut right_cfg = quic_cfg(server.addr, &server.fingerprint, 0);
    right_cfg.join_key = Some("correct horse battery staple".into());
    let mut right = NetClient::connect_with(right_cfg).unwrap();
    assert!(poll_until(&mut right, 5.0, |c| c.state() == ConnState::Connected), "the right key joins");
    right.disconnect();
    let server = server.finish();
    assert_eq!(server.stats().bad_keys, 1);
    assert_eq!(server.stats().joins, 1);
}

#[test]
fn malformed_application_messages_are_counted_and_do_not_disturb_a_real_player() {
    let server = start(&identity(), "127.0.0.1:0", None, QuicServerOptions::default());
    let mut raw = QuicClient::connect(server.addr, "localhost", ServerTrust::fingerprint(&server.fingerprint).unwrap()).unwrap();
    let t0 = Instant::now();
    while raw.status() != TransportStatus::Ready {
        assert!(t0.elapsed() < Duration::from_secs(5), "raw QUIC client connects");
        std::thread::sleep(Duration::from_millis(2));
    }
    // Garbage, a wrong magic, a Hello cut short, an Input with no session, and a huge count field: all inside a valid QUIC connection.
    let mut junk: Vec<Vec<u8>> = vec![vec![0xff; 40], b"XX\x08\x00\x01".to_vec(), vec![0x44, 0x52, 8, 0, 1, 3], vec![0x44, 0x52, 8, 0, 2, 0, 0]];
    let mut bomb = vec![0x44, 0x52, 8, 0, 2];
    bomb.extend_from_slice(&[0xff; 64]);
    junk.push(bomb);
    for _ in 0..50 {
        for j in &junk {
            let _ = raw.send(j);
        }
    }
    let mut good = NetClient::connect_with(quic_cfg(server.addr, &server.fingerprint, 0)).unwrap();
    assert!(poll_until(&mut good, 5.0, |c| c.state() == ConnState::Connected), "a real player still joins");
    assert!(poll_until(&mut good, 3.0, |c| c.stats().snapshots > 10), "and receives snapshots");
    good.disconnect();
    raw.close();
    let server = server.finish();
    assert!(server.stats().bad_packets >= 100, "malformed messages were counted and dropped: {}", server.stats().bad_packets);
    assert_eq!(server.stats().joins, 1);
}

#[test]
fn connections_and_the_inbound_queue_are_bounded() {
    // At most two connections at once: the rest are refused by the transport before costing a session.
    let server = start(&identity(), "127.0.0.1:0", None, QuicServerOptions { max_connections: 2 });
    let trust = || ServerTrust::fingerprint(&server.fingerprint).unwrap();
    let mut clients: Vec<QuicClient> = (0..5).map(|_| QuicClient::connect(server.addr, "localhost", trust()).unwrap()).collect();
    std::thread::sleep(Duration::from_millis(1500));
    let ready = clients.iter().filter(|c| c.status() == TransportStatus::Ready).count();
    for c in clients.iter_mut() {
        c.close();
    }
    let server = server.finish();
    let t = server.transport_stats();
    eprintln!("MEASURE limit: {ready} of 5 connected; {}", describe(server.stats(), &t, 1.5));
    assert_eq!(ready, 2, "exactly the limit connected");
    assert!(t.connections_refused >= 3, "{t:?}");

    // A flood of datagrams that nobody reads waits in a bounded queue; the excess is dropped and counted, never buffered without limit.
    let id = identity();
    let mut q = QuicServer::bind("127.0.0.1:0".parse().unwrap(), &id, QuicServerOptions::default()).unwrap();
    let mut flooder = QuicClient::connect(q.local_addr().unwrap(), "localhost", ServerTrust::fingerprint(&id.fingerprint()).unwrap()).unwrap();
    let t0 = Instant::now();
    while flooder.status() != TransportStatus::Ready {
        assert!(t0.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(2));
    }
    for i in 0..20_000u32 {
        let _ = flooder.send(&i.to_le_bytes());
        if i % 500 == 0 {
            std::thread::sleep(Duration::from_millis(1)); // let the paths drain so the flood reaches the server's queue
        }
    }
    std::thread::sleep(Duration::from_millis(800));
    let mut buf = [0u8; 64];
    let mut queued = 0;
    while q.recv(&mut buf).is_some() {
        queued += 1;
    }
    let st = q.stats();
    eprintln!("MEASURE queue: {queued} datagrams held (cap {INBOUND_QUEUE}), {} dropped", st.queue_dropped);
    assert!(queued <= INBOUND_QUEUE, "{queued}");
    assert!(st.queue_dropped > 0, "{st:?}");
    flooder.close();
}

#[test]
fn a_player_reconnects_after_an_outage_and_resumes_their_slot() {
    let server = start(&identity(), "127.0.0.1:0", None, QuicServerOptions::default());
    let proxy = LossyProxy::start(server.addr, profile("lan").unwrap(), 3).unwrap();
    let mut client = NetClient::connect_with(quic_cfg(proxy.addr, &server.fingerprint, 0)).unwrap();
    assert!(poll_until(&mut client, 5.0, |c| c.state() == ConnState::Connected));
    let (id, token) = (client.my_id(), client.token());
    proxy.set_blackhole(true); // the network goes away for longer than both timeouts
    assert!(poll_until(&mut client, 5.0, |c| c.state() == ConnState::Reconnecting), "the client noticed");
    std::thread::sleep(Duration::from_millis(1500));
    proxy.set_blackhole(false);
    let t0 = Instant::now();
    assert!(poll_until(&mut client, 12.0, |c| c.state() == ConnState::Connected && c.stats().connects == 2), "reconnected over a new QUIC connection");
    eprintln!("MEASURE reconnect: back {:.2} s after the network returned", t0.elapsed().as_secs_f64());
    assert_eq!((client.my_id(), client.token()), (id, token), "same player, same token");
    client.disconnect();
    proxy.finish();
    let server = server.finish();
    assert_eq!(server.stats().resumes, 1, "the server resumed the parked player");
}

#[test]
fn shutdown_tells_clients_and_closes_every_connection_promptly() {
    let server = start(&identity(), "127.0.0.1:0", None, QuicServerOptions::default());
    let mut client = NetClient::connect_with(quic_cfg(server.addr, &server.fingerprint, 0)).unwrap();
    assert!(poll_until(&mut client, 5.0, |c| c.state() == ConnState::Connected));
    let t0 = Instant::now();
    server.stop.store(true, Ordering::Relaxed);
    let server = server.handle.join().unwrap();
    let stopped = t0.elapsed();
    assert!(stopped < Duration::from_secs(3), "the server stopped in {stopped:?}");
    assert!(poll_until(&mut client, 3.0, |c| c.state() == ConnState::Reconnecting), "the client learned");
    assert!(t0.elapsed() < client.timeout, "the client learned from the server, not from its own timeout ({:?})", t0.elapsed());
    eprintln!("MEASURE shutdown: server stopped in {:.0} ms; client knew within {:.0} ms", stopped.as_secs_f64() * 1000.0, t0.elapsed().as_secs_f64() * 1000.0);
    drop(server);
    client.disconnect();
}
