//! Workshop online, through the supported adapter (a match game's snapshot part and a client's game decoder: `MatchGame::snapshot_part`,
//! `NetClient::set_game_decoder`): a real `Server` and a real client (`net::bot::Bot` on `NetClient`) over the in-memory link on a virtual
//! clock (`net::memnet`), with loss, delay and jitter. The client joins late, is cut off past the server's timeout and comes back, and every
//! part it decodes must be exactly what the server's game encoded at that tick.

use red_engine2::net::bot::{Behavior, Bot, ClientWorld};
use red_engine2::net::client::{ClientConfig, ConnState, NetClient};
use red_engine2::net::memnet::{LinkModel, MemNet};
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::player::Character;
use red_engine2::sim::clock::TICK_RATE_HZ;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use workshop_server::game::WorkshopGame;
use workshop_server::wire::{self, WorkshopView};

mod common;

const STEP: Duration = Duration::from_millis(4);

fn scene_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../content/workshop.json")
}

struct World {
    net: MemNet,
    server: Server,
    epoch: Instant,
    next_tick: Duration,
    /// What the server's game encoded, by the systems' tick.
    sent: BTreeMap<u32, WorkshopView>,
    bot: Option<Bot>,
    server_addr: SocketAddr,
    map_hash: u32,
}

fn view(game: &WorkshopGame) -> WorkshopView {
    let part = wire::encode(game);
    let decoded = wire::decode(&part).unwrap().unwrap();
    (decoded.as_ref() as &dyn std::any::Any).downcast_ref::<WorkshopView>().unwrap().clone()
}

impl World {
    fn new(link: LinkModel) -> World {
        let text = std::fs::read_to_string(scene_path()).unwrap();
        World::with(link, &text, scene_path().parent().unwrap())
    }

    fn with(link: LinkModel, text: &str, dir: &std::path::Path) -> World {
        workshop_server::register();
        let text = text.to_string();
        let scene = red_engine2::scene_blocks::parse_scene_in(&text, Some(dir)).unwrap();
        let server_addr: SocketAddr = "10.0.0.1:27015".parse().unwrap();
        let net = MemNet::new(11, server_addr);
        net.set_link(link);
        let map_hash = red_engine2::net::map_hash(&text);
        let sim = MatchSim::try_new(&scene, parse_spawns(&text).unwrap()).unwrap();
        let mut server = Server::with_transport(ServerConfig::new(server_addr, map_hash), sim, Box::new(net.server_transport())).unwrap();
        server.set_logger(|_| {});
        World { net, server, epoch: Instant::now(), next_tick: Duration::ZERO, sent: BTreeMap::new(), bot: None, server_addr, map_hash }
    }

    fn now(&self) -> Instant {
        self.epoch + self.net.now()
    }

    fn join(&mut self) {
        self.join_scene(&scene_path());
    }

    fn join_scene(&mut self, path: &std::path::Path) {
        let (_scene, world) = ClientWorld::load(path).unwrap();
        let mut client = NetClient::with_transport(
            ClientConfig::new(self.server_addr, 0, self.map_hash, 0),
            Box::new(self.net.client_transport("10.0.1.1:5000".parse().unwrap())),
            self.now(),
        );
        client.set_game_decoder(wire::decode);
        self.bot = Some(Bot::with_client(client, Character::Human, world, Behavior::Idle).unwrap());
    }

    fn step(&mut self) {
        self.net.advance(STEP);
        let now = self.now();
        self.server.pump(now);
        let tick = Duration::from_secs_f64(1.0 / TICK_RATE_HZ as f64);
        while self.next_tick <= self.net.now() {
            self.server.tick(now);
            self.next_tick += tick;
            let g = self.server.sim().game::<WorkshopGame>().unwrap();
            self.sent.insert(g.systems().tick_count() as u32, view(g));
        }
        if let Some(b) = &mut self.bot {
            b.pump(now);
        }
    }

    fn run(&mut self, span: Duration) -> usize {
        let end = self.net.now() + span;
        let mut checked = 0;
        while self.net.now() < end {
            self.step();
            if let Some(v) = self.bot.as_ref().and_then(|b| b.client.game_view::<WorkshopView>()) {
                let sent = self.sent.get(&v.tick).unwrap_or_else(|| panic!("the client shows tick {}, which the server never encoded", v.tick));
                assert_eq!(v, sent, "the client's view of tick {} is not what the server sent", v.tick);
                checked += 1;
            }
        }
        checked
    }
}

#[test]
fn the_workshop_survives_late_join_loss_jitter_and_reconnect() {
    let link = LinkModel { loss_permille: 100, delay: Duration::from_millis(40), jitter: Duration::from_millis(30) };
    let mut w = World::new(link);
    // Production runs for 5 s before anyone joins.
    w.run(Duration::from_secs(5));
    let before_join = w.server.sim().game::<WorkshopGame>().unwrap().systems().tick_count();
    assert!(before_join > 250);
    w.join();
    let checked = w.run(Duration::from_secs(3));
    let bot = w.bot.as_ref().unwrap();
    assert_eq!(bot.client.state(), ConnState::Connected);
    let v = bot.client.game_view::<WorkshopView>().expect("a late joiner gets the workshop's state");
    assert!(checked > 50 && v.tick as u64 > before_join, "{checked} views checked, newest tick {}", v.tick);
    assert!(v.machines.iter().any(|m| m.2 > 0), "the late joiner sees cycles made before it joined: {v:?}");
    // Cut off past the server's 3 s timeout: the client reconnects by itself, and the state it shows is current again.
    let peer: SocketAddr = "10.0.1.1:5000".parse().unwrap();
    w.net.partition(peer, true);
    w.run(Duration::from_secs(4));
    let stale = w.bot.as_ref().unwrap().client.game_view::<WorkshopView>().unwrap().tick;
    w.net.partition(peer, false);
    let checked = w.run(Duration::from_secs(6));
    let bot = w.bot.as_ref().unwrap();
    assert_eq!(bot.client.state(), ConnState::Connected);
    let fresh = bot.client.game_view::<WorkshopView>().unwrap();
    assert!(checked > 50 && fresh.tick > stale + 300, "after the reconnect: {checked} views checked, tick {} (was {stale})", fresh.tick);
    let server_tick = w.server.sim().game::<WorkshopGame>().unwrap().systems().tick_count() as u32;
    assert!(server_tick - fresh.tick < 30, "the view is at most half a second behind the server ({} vs {server_tick})", fresh.tick);
    assert!(w.server.stats().timeouts >= 1, "the partition really did time the client out");
}

/// A snapshot whose game part is malformed is dropped as a bad packet, as a malformed snapshot is.
#[test]
fn a_malformed_part_is_refused() {
    use red_engine2::net::protocol::GameMsg;
    for bytes in [vec![], vec![0u8; 3], vec![255u8; 64]] {
        assert!(wire::decode(&GameMsg { kind: wire::KIND, bytes }).is_err());
    }
    assert!(wire::decode(&GameMsg { kind: 1, bytes: vec![1, 2, 3] }).unwrap().is_none(), "another game's part is not ours");
}

/// The first network limit, measured: a game part over `MAX_GAME_BYTES` does not fit a snapshot. The server leaves it out whole and counts it
/// (rather than cutting it, which would cost the client every snapshot); the client stays connected and in sync on players, without the
/// workshop. 200 machines make a part of about 1.6 KB; the scale probe prints the size at 10, 100 and 1000.
#[test]
fn a_part_too_big_for_a_snapshot_is_left_out_visibly_and_the_client_stays_connected() {
    let (text, dir) = common::workshop(200);
    let path = dir.join("scene.json");
    std::fs::write(&path, &text).unwrap();
    let mut w = World::with(LinkModel::default(), &text, &dir);
    let part = wire::encode(w.server.sim().game::<WorkshopGame>().unwrap()).bytes.len();
    assert!(part > red_engine2::net::protocol::MAX_GAME_BYTES, "{part} bytes");
    w.join_scene(&path);
    w.run(Duration::from_secs(2));
    let bot = w.bot.as_ref().unwrap();
    assert_eq!(bot.client.state(), ConnState::Connected);
    assert!(bot.client.stats().snapshots > 30, "snapshots still arrive: {:?}", bot.client.stats());
    assert!(bot.client.game_view::<WorkshopView>().is_none(), "no workshop state reaches the client");
    assert!(w.server.stats().game_parts_omitted > 30, "{:?}", w.server.stats().game_parts_omitted);
    assert_eq!(bot.client.stats().bad_packets, 0, "nothing arrives cut");
    let _ = std::fs::remove_dir_all(&dir);
}
