//! A refused join leaves a trace (ADR 2026-10-20-a-server-says-why-it-refused-a-join). On 2026-10-20 a protocol-15 server refused every v16 and v17
//! client for days and its journal read "0 joins, 0 bad packets": nothing counted a refused `Hello` and nothing said why. Each reason now bumps exactly
//! its own counter, is written once a minute with the client's protocol version, map hash and a masked address (never a key), and shows in the periodic
//! `stats:` line, which keeps every field it always had. The server's clock is under the test's control, as in `net_auth.rs`; nothing here changes the
//! wire: the replies are the protocol's existing ones, so a v4 client and the deployed protocol-17 relay and server interoperate (the first test proves
//! the client side of that by joining with the unmodified client path).

use red_engine2::net::map_hash;
use red_engine2::net::protocol::*;
use red_engine2::net::refusals::NEEDS_KEY_GRACE;
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::net::testkit::RawClient;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use red_engine2::tools::servers::parse_players;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const KEY: &str = "right-join-key-0001";
const WRONG_KEY: &str = "wrong-join-key-0002";

struct Rig {
    server: Server,
    addr: SocketAddr,
    hash: u32,
    now: Instant,
    nonce: u64,
    log: Arc<Mutex<Vec<String>>>,
}

impl Rig {
    fn new(key: Option<&str>, stats_every: Option<Duration>) -> Rig {
        let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
        let scene = red_engine2::scene_blocks::parse_scene(&text).unwrap();
        let mut spawns = parse_spawns(&text).unwrap();
        spawns.retain(|s| s.group == "duel");
        let hash = map_hash(&text);
        let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), hash);
        cfg.join_key = key.map(str::to_string);
        cfg.stats_every = stats_every;
        let mut server = Server::bind(cfg, MatchSim::try_new(&scene, spawns).unwrap()).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = log.clone();
        server.set_logger(move |l| sink.lock().unwrap().push(l.to_string()));
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        Rig { server, addr, hash, now: Instant::now(), nonce: 900, log }
    }

    fn pump(&mut self) {
        std::thread::sleep(Duration::from_millis(30));
        self.server.pump(self.now);
    }

    /// Runs the server's clock forward, ticking 60 times a second.
    fn advance(&mut self, secs: f64) {
        for _ in 0..(secs * 60.0) as u32 {
            self.now += Duration::from_micros(16_667);
            self.server.pump(self.now);
            self.server.tick(self.now);
        }
    }

    fn client(&mut self, hash: u32, key: Option<&str>) -> RawClient {
        self.nonce += 1;
        RawClient::new(self.addr, hash, key, self.nonce).unwrap()
    }

    fn handshake(&mut self, c: &mut RawClient) -> Result<Welcome, String> {
        let (server, now) = (&mut self.server, self.now);
        c.handshake(|| {
            std::thread::sleep(Duration::from_millis(30));
            server.pump(now);
        })
    }

    /// The log lines about refused joins.
    fn refused_lines(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().filter(|l| l.contains("join refused")).cloned().collect()
    }

    /// `[v, m, k, b, f, o]` as the server counts them.
    fn counts(&self) -> [u64; 6] {
        let s = self.server.stats();
        [s.rejected_version, s.rejected_wrong_map, s.rejected_needs_key, s.bad_keys, s.rejected_full, s.rejected_other]
    }
}

/// The bytes of a Hello that claims `version` (its body says so; the server reads only that from another version's Hello).
fn hello_of_version(version: u16, hash: u32, nonce: u64) -> Vec<u8> {
    let mut b = Vec::new();
    ClientMsg::Hello(Hello { version, map_hash: hash, client_nonce: nonce, ..Hello::default() }).encode(&mut b);
    b
}

#[test]
fn an_honest_join_is_not_a_refusal_and_the_deployed_client_path_still_works() {
    let mut rig = Rig::new(Some(KEY), None);
    let mut c = rig.client(rig.hash, Some(KEY));
    rig.handshake(&mut c).expect("the unchanged client handshake (challenge, proof, welcome) still joins");
    rig.advance(NEEDS_KEY_GRACE.as_secs_f64() + 2.0);
    assert_eq!(rig.counts(), [0; 6], "a handshake that was answered is never counted as one that was not");
    assert!(rig.refused_lines().is_empty(), "{:?}", rig.refused_lines());
}

#[test]
fn a_version_mismatch_bumps_only_its_counter_and_logs_once_naming_which_side_is_old() {
    let mut rig = Rig::new(None, None);
    for (n, version) in [(1u64, PROTOCOL_VERSION - 1), (2, PROTOCOL_VERSION - 1), (3, PROTOCOL_VERSION + 1)] {
        let c = rig.client(rig.hash, None);
        c.send_raw(&hello_of_version(version, rig.hash, 5000 + n));
        rig.pump();
        let mut c = c;
        assert_eq!(c.poll(), vec![ServerMsg::Reject(RejectReason::Version)], "the client still gets the reply the protocol has always sent");
    }
    assert_eq!(rig.counts(), [3, 0, 0, 0, 0, 0], "three version refusals and nothing else");
    let lines = rig.refused_lines();
    assert_eq!(lines.len(), 1, "one line per reason per minute, however many are refused: {lines:?}");
    let line = &lines[0];
    assert!(line.contains("join refused (version)"), "{line}");
    assert!(line.contains(&format!("protocol v{} and this server v{PROTOCOL_VERSION}", PROTOCOL_VERSION - 1)), "names the client's version: {line}");
    assert!(line.contains("the client is the old side"), "{line}");
    assert!(line.contains("peer 127.0.x.x"), "the address is masked to its first two octets: {line}");
    assert!(!line.contains("127.0.0.1"), "{line}");
    // A minute later the next one is written again, and carries the number held back.
    rig.advance(61.0);
    let c = rig.client(rig.hash, None);
    c.send_raw(&hello_of_version(PROTOCOL_VERSION + 1, rig.hash, 7000));
    rig.pump();
    let lines = rig.refused_lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[1].contains("this server is the old side") && lines[1].contains("2 more like it"), "{}", lines[1]);
    assert_eq!(rig.counts()[0], 4);
}

#[test]
fn a_wrong_map_bumps_only_its_counter_and_logs_both_hashes_once() {
    let mut rig = Rig::new(None, None);
    let wrong = rig.hash ^ 0x00ff_00ff;
    for _ in 0..2 {
        let mut c = rig.client(wrong, None);
        let err = rig.handshake(&mut c).expect_err("a client with another map is refused");
        assert_eq!(err, "rejected: WrongMap");
    }
    assert_eq!(rig.counts(), [0, 2, 0, 0, 0, 0]);
    let lines = rig.refused_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("join refused (map)"), "{}", lines[0]);
    assert!(lines[0].contains(&format!("{wrong:08x}")) && lines[0].contains(&format!("{:08x}", rig.hash)), "both hashes: {}", lines[0]);
    assert!(lines[0].contains(&format!("client protocol v{PROTOCOL_VERSION}")), "{}", lines[0]);
}

#[test]
fn a_wrong_join_key_bumps_only_its_counter_logs_once_and_never_the_key() {
    let mut rig = Rig::new(Some(KEY), None);
    for _ in 0..2 {
        let mut c = rig.client(rig.hash, Some(WRONG_KEY));
        let err = rig.handshake(&mut c).expect_err("the wrong key is refused");
        assert_eq!(err, "rejected: BadKey");
    }
    assert_eq!(rig.counts(), [0, 0, 0, 2, 0, 0], "b is the wrong-key counter (`bad_keys`, which existed before)");
    let lines = rig.refused_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("join refused (key)") && lines[0].contains("peer 127.0.x.x"), "{}", lines[0]);
    let everything = rig.log.lock().unwrap().join("\n");
    assert!(!everything.contains(KEY) && !everything.contains(WRONG_KEY), "no key reaches the log:\n{everything}");
}

#[test]
fn a_client_that_gives_up_at_the_key_challenge_is_counted_as_needs_key_once_the_grace_has_passed() {
    let mut rig = Rig::new(Some(KEY), None);
    // What a short code without the key does to the real client: it receives the Challenge ("a key is required"), has none, and stops. The test sends
    // the first Hello and never the second.
    let first = rig.client(rig.hash, None);
    first.send_hello();
    first.send_hello(); // the first Hello repeats until the Challenge arrives: still one handshake
    rig.pump();
    assert_eq!(rig.counts(), [0; 6], "nothing is counted before the grace: the second Hello may still come");
    rig.advance(NEEDS_KEY_GRACE.as_secs_f64() - 2.0);
    assert_eq!(rig.counts(), [0; 6]);
    rig.advance(3.0);
    assert_eq!(rig.counts(), [0, 0, 1, 0, 0, 0], "one handshake stopped at the key challenge");
    let lines = rig.refused_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("join refused (needs key)") && lines[0].contains("without the key part"), "{}", lines[0]);
    rig.advance(30.0);
    assert_eq!(rig.counts()[2], 1, "counted once, not every second");
}

#[test]
fn a_full_match_is_counted_and_logged() {
    let mut rig = Rig::new(None, None);
    let mut held = Vec::new();
    let mut refused = None;
    for _ in 0..(red_engine2::sim::match_sim::MAX_PLAYERS + 1) {
        let mut c = rig.client(rig.hash, None);
        match rig.handshake(&mut c) {
            Ok(_) => held.push(c),
            Err(e) => {
                refused = Some(e);
                break;
            }
        }
    }
    let refused = refused.expect("the match fills up");
    assert_eq!(refused, "rejected: Full");
    assert_eq!(rig.counts(), [0, 0, 0, 0, 1, 0]);
    assert!(rig.refused_lines().iter().any(|l| l.contains("join refused (full)")), "{:?}", rig.refused_lines());
}

#[test]
fn the_stats_line_keeps_every_field_it_had_in_order_and_appends_the_refusals_where_the_status_reader_still_parses_it() {
    let mut rig = Rig::new(None, Some(Duration::from_secs(1)));
    let c = rig.client(rig.hash, None);
    c.send_raw(&hello_of_version(PROTOCOL_VERSION - 1, rig.hash, 4242));
    rig.pump();
    rig.advance(1.0);
    let line = rig.log.lock().unwrap().iter().rev().find(|l| l.contains("stats: players ")).cloned().expect("a stats line after a second");
    let stats = line.split("stats: ").nth(1).unwrap();
    let fields = [
        "players ",
        " | phase ",
        " | tick avg ",
        " us worst ",
        " us | promoted props ",
        " | in ",
        " KB/s out ",
        " KB/s | snapshots ",
        " | bad packets ",
        " bad tags ",
        " | joins ",
        " | rejected v1 m0 k0 b0 f0 o0",
        " | throttled 0",
    ];
    let mut at = 0;
    for f in fields {
        let found = stats[at..].find(f).unwrap_or_else(|| panic!("`{f}` missing or out of order in: {stats}"));
        at += found + f.len();
    }
    assert!(stats.ends_with(" | throttled 0"), "{stats}");
    // The reader `red_engine2 servers` uses on the journal still gets the player count and phase out of the longer line.
    let journal = format!("1790798221.5 host red_server[321]: {line}\n");
    let players = parse_players(&journal, 1790798225, Some(60)).expect("the longer line still parses");
    assert_eq!((players.count, players.phase.as_str()), (0, "playing"), "open play (no lobby) reports its phase as playing");
}
