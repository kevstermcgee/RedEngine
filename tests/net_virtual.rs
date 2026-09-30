//! The match flow over `net::memnet`: a real `Server` and real `NetClient`s/`Bot`s talking through an in-memory link on a **virtual clock**, with no socket, thread or
//! `sleep`. `net_flow.rs` proves the same lobby -> countdown -> round -> results -> rematch flow over real UDP in real seconds; this is the same logic as a state
//! machine driven by an explicit clock, so it takes milliseconds, and because the link's loss, delay and reordering come from a seed, a run is exactly repeatable
//! (the determinism test below compares two runs event for event) and a scenario can name the conditions it needs (a bad link, a peer that simply vanishes).
//! The real-socket and QUIC suites stay: they prove the OS, the sockets and the TLS handshake, not the game logic.

use red_engine2::net::bot::{Behavior, Bot, ClientWorld};
use red_engine2::net::client::{ClientConfig, ConnState, NetClient};
use red_engine2::net::memnet::{LinkModel, MemNet};
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::player::Character;
use red_engine2::sim::clock::TICK_RATE_HZ;
use red_engine2::sim::flow::{MatchSettings, Phase};
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn lab_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")
}

fn quick() -> MatchSettings {
    MatchSettings { min_players: 2, countdown_secs: 0.4, round_secs: 1.5, results_secs: 0.8, score_to_win: 0, join_in_progress: true, ready_check: true }
}

/// Virtual time between pump calls: 4 ms, finer than a tick, like a busy loop.
const STEP: Duration = Duration::from_millis(4);

struct Sim {
    net: MemNet,
    server: Server,
    epoch: Instant,
    next_tick: Duration,
    bots: Vec<Bot>,
    map_hash: u32,
    server_addr: SocketAddr,
}

impl Sim {
    fn new(seed: u64, settings: MatchSettings, link: LinkModel) -> Sim {
        let text = std::fs::read_to_string(lab_path()).unwrap();
        let scene = red_engine2::schema::parse_scene(&text).unwrap();
        let mut spawns = parse_spawns(&text).unwrap();
        spawns.retain(|s| s.group == "duel");
        let server_addr: SocketAddr = "10.0.0.1:27015".parse().unwrap();
        let net = MemNet::new(seed, server_addr);
        net.set_link(link);
        let map_hash = red_engine2::net::map_hash(&text);
        let cfg = ServerConfig::new(server_addr, map_hash);
        let mut server = Server::with_transport(cfg, MatchSim::try_new(&scene, spawns.clone()).unwrap(), Box::new(net.server_transport())).unwrap();
        server.set_logger(|_| {});
        server.enable_flow(settings, move || MatchSim::try_new(&scene, spawns.clone())).unwrap();
        Sim { net, server, epoch: Instant::now(), next_tick: Duration::ZERO, bots: Vec::new(), map_hash, server_addr }
    }

    fn now(&self) -> Instant {
        self.epoch + self.net.now()
    }

    fn add_bot(&mut self, name: &str, auto_ready: bool) -> usize {
        let n = self.bots.len();
        let addr: SocketAddr = format!("10.0.1.{}:5000", n + 1).parse().unwrap();
        let (_scene, world) = ClientWorld::load(&lab_path()).unwrap();
        let mut cfg = ClientConfig::new(self.server_addr, 0, self.map_hash, 0);
        cfg.name = name.to_string();
        let client = NetClient::with_transport(cfg, Box::new(self.net.client_transport(addr)), self.now());
        let mut bot = Bot::with_client(client, Character::Human, world, Behavior::Forward { yaw_deg: 90.0, sprint: false }).unwrap();
        bot.auto_ready = auto_ready;
        self.bots.push(bot);
        n
    }

    /// One 4 ms of virtual time: the clock moves, the server drains its socket and runs the ticks that are due, then every client pumps.
    fn step(&mut self) {
        self.net.advance(STEP);
        let now = self.now();
        self.server.pump(now);
        let tick = Duration::from_secs_f64(1.0 / TICK_RATE_HZ as f64);
        while self.next_tick <= self.net.now() {
            self.server.tick(now);
            self.next_tick += tick;
        }
        for b in &mut self.bots {
            b.pump(now);
        }
    }

    /// Steps until `done`, failing with the bots' event logs after `limit` of virtual time. Returns the virtual time it took.
    fn drive(&mut self, what: &str, limit: Duration, done: impl Fn(&Sim) -> bool) -> Duration {
        let start = self.net.now();
        while self.net.now() - start < limit {
            self.step();
            if done(self) {
                return self.net.now() - start;
            }
        }
        let logs: Vec<String> = self.bots.iter().map(|b| format!("{:?}", b.events)).collect();
        panic!("after {limit:?} of virtual time: {what}\n{}", logs.join("\n"));
    }

    fn run_for(&mut self, span: Duration) {
        let end = self.net.now() + span;
        while self.net.now() < end {
            self.step();
        }
    }
}

fn phase(b: &Bot) -> Phase {
    b.client.phase()
}

const LIMIT: Duration = Duration::from_secs(12);

#[test]
fn the_whole_match_flow_runs_in_virtual_time_with_the_same_assertions_as_the_real_udp_test() {
    let mut s = Sim::new(1, quick(), LinkModel::default());
    let (a, b) = (s.add_bot("Ada", false), s.add_bot("Bo", true));
    s.drive("both in the lobby", LIMIT, |s| s.bots.iter().all(|x| x.client.state() == ConnState::Connected && x.client.status().is_some()));
    assert_eq!((phase(&s.bots[a]), s.bots[a].client.in_round()), (Phase::Waiting, false), "the lobby is not the world");
    let roster = s.bots[a].client.status().unwrap().roster.clone();
    assert!(roster.iter().any(|e| e.name == "Ada") && roster.iter().any(|e| e.name == "Bo"), "names reach the roster: {roster:?}");

    // Only Bo is ready: nothing starts, however long we wait (here: 600 ms of virtual time, instantly).
    s.drive("Bo's ready shows in the roster", LIMIT, |s| {
        s.bots[a].client.status().is_some_and(|st| st.roster.iter().any(|e| e.name == "Bo" && e.flags & 1 != 0))
    });
    s.run_for(Duration::from_millis(600));
    assert_eq!(phase(&s.bots[a]), Phase::Waiting, "one of two ready is not everyone");

    // Ada readies up: countdown, then the round; both get a body at distinct spawns.
    let now = s.now();
    s.bots[a].client.set_ready(true, now);
    s.drive("the round starts", LIMIT, |s| s.bots.iter().all(|x| phase(x) == Phase::Playing && x.client.in_round() && x.predictor.is_some()));
    let (wa, wb) = (s.bots[a].client.welcome().unwrap(), s.bots[b].client.welcome().unwrap());
    assert_eq!((wa.round, wb.round), (1, 1));
    assert_ne!(wa.player_id, wb.player_id);
    assert!((wa.spawn[0] - wb.spawn[0]).abs() + (wa.spawn[2] - wb.spawn[2]).abs() > 1.0, "distinct spawn points");
    assert!(s.bots[a].events.iter().any(|e| e.1.starts_with("phase countdown round 1")), "the countdown was announced: {:?}", s.bots[a].events);
    s.drive("Ada walks away from her spawn", LIMIT, |s| {
        let now = s.now();
        s.bots[a].frame(now).me.distance(glam::Vec2::new(wa.spawn[0], wa.spawn[2])) > 1.5
    });

    // The round is timed (1.5 s), then the results.
    s.drive("the results", LIMIT, |s| s.bots.iter().all(|x| phase(x) == Phase::Results));
    assert_eq!(s.bots[a].client.status().unwrap().end_code, 1, "ended by the clock");
    assert!(!s.bots[a].client.is_ready() && !s.bots[a].client.in_round());

    // Nobody rematches: back to the lobby with the ready flags clear.
    s.bots[b].auto_ready = false;
    let now = s.now();
    s.bots[b].client.set_ready(false, now);
    s.drive("back in the lobby", LIMIT, |s| s.bots.iter().all(|x| phase(x) == Phase::Waiting));
    s.drive("the roster shows nobody ready", LIMIT, |s| s.bots[a].client.status().is_some_and(|st| st.roster.iter().all(|e| e.flags & 1 == 0)));

    // Rematch: round 2 starts from the authored map.
    let now = s.now();
    s.bots[a].client.set_ready(true, now);
    s.bots[b].client.set_ready(true, now);
    s.drive("round 2", LIMIT, |s| s.bots.iter().all(|x| phase(x) == Phase::Playing && x.client.in_round() && x.client.welcome().is_some_and(|w| w.round == 2)));
    let w2 = s.bots[a].client.welcome().unwrap();
    assert_eq!((w2.player_id, w2.spawn), (wa.player_id, wa.spawn), "same id, same spawn: the world was rebuilt");
    s.drive("round 2 ends", LIMIT, |s| s.bots.iter().all(|x| phase(x) == Phase::Results));
    assert_eq!(s.server.round(), 2);
    assert_eq!(s.server.stats().rounds, 2);
    assert!(s.net.now() > Duration::from_secs(5), "about seven virtual seconds of match...");
}

#[test]
fn a_bad_link_with_loss_delay_and_reordering_still_gets_everyone_into_the_round() {
    let bad = LinkModel { loss_permille: 100, delay: Duration::from_millis(40), jitter: Duration::from_millis(40) };
    let mut s = Sim::new(7, quick(), bad);
    s.add_bot("Ada", true);
    s.add_bot("Bo", true);
    s.drive("the round starts over a lossy, jittery link", Duration::from_secs(20), |s| {
        s.bots.iter().all(|x| phase(x) == Phase::Playing && x.client.in_round())
    });
    let (sent, dropped) = s.net.counts();
    assert!(dropped > 0 && dropped < sent / 4, "the link really lost datagrams: {dropped} of {sent}");
}

/// Everything observable about one run: the virtual time each milestone was reached, every bot's event names, and the exact simulation checksum.
struct RunTrace {
    marks: Vec<Duration>,
    events: Vec<Vec<String>>,
    checksum: u64,
}

fn run_once(seed: u64) -> RunTrace {
    let link = LinkModel { loss_permille: 80, delay: Duration::from_millis(30), jitter: Duration::from_millis(30) };
    let mut s = Sim::new(seed, quick(), link);
    s.add_bot("Ada", true);
    s.add_bot("Bo", true);
    let connected = s.drive("connected", Duration::from_secs(20), |s| s.bots.iter().all(|x| x.client.state() == ConnState::Connected));
    let playing = s.drive("playing", Duration::from_secs(20), |s| s.bots.iter().all(|x| phase(x) == Phase::Playing && x.client.in_round()));
    let results = s.drive("results", Duration::from_secs(20), |s| s.bots.iter().all(|x| phase(x) == Phase::Results));
    RunTrace {
        marks: vec![connected, playing, results],
        events: s.bots.iter().map(|b| b.events.iter().map(|e| e.1.clone()).collect()).collect(),
        checksum: s.server.sim().checksum(),
    }
}

#[test]
fn the_same_seed_replays_the_same_run_exactly_and_another_seed_does_not() {
    let (a, b) = (run_once(11), run_once(11));
    assert_eq!(a.marks, b.marks, "the same milestones at the same virtual instants");
    assert_eq!(a.checksum, b.checksum, "the same simulation, bit for bit");
    assert_eq!(a.events, b.events, "the same events in the same order");
    let c = run_once(12);
    assert!(c.marks != a.marks || c.checksum != a.checksum, "a different seed loses different datagrams and plays a different run");
}

#[test]
fn a_peer_that_simply_vanishes_is_dropped_by_the_server_after_its_timeout_and_the_survivor_carries_on() {
    let mut s = Sim::new(3, MatchSettings { min_players: 1, ..quick() }, LinkModel::default());
    s.add_bot("Ada", false);
    s.add_bot("Bo", false);
    s.drive("both connected", LIMIT, |s| s.server.client_count() == 2 && s.bots.iter().all(|x| x.client.status().is_some()));
    // Bo's link dies without a goodbye (no packets either way): from the server's side, silence.
    s.net.partition("10.0.1.2:5000".parse().unwrap(), true);
    let waited = s.drive("the server times Bo out", Duration::from_secs(30), |s| s.server.client_count() == 1);
    assert!(waited >= Duration::from_secs(1), "not instantly: the server waits out a silent peer ({waited:?})");
    assert!(waited <= Duration::from_secs(15), "but does not wait forever ({waited:?})");
    assert_eq!(s.bots[0].client.state(), ConnState::Connected, "Ada is unaffected");
    // Bo's link comes back: it has to join again, and the server lets it.
    s.net.partition("10.0.1.2:5000".parse().unwrap(), false);
    s.drive("Bo is back", Duration::from_secs(30), |s| s.server.client_count() == 2 && s.bots[1].client.state() == ConnState::Connected);
}
