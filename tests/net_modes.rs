//! The loadout modes over real UDP: a duel server holds two people and turns a third away, the match status names the mode and the mode's own
//! limit, and free for all has no teams (ADR 2026-10-07-killchain-game-modes-free-for-all-capture-the-flag). Timing-based, so every wait has a
//! generous deadline.

use red_engine2::net::bot::{Behavior, Bot, ClientWorld};
use red_engine2::net::client::{ClientConfig, ConnState, NetClient};
use red_engine2::net::protocol::RejectReason;
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::player::Character;
use red_engine2::sim::flow::MatchSettings;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::shooter::ModeKind;
use red_engine2::sim::spawns::parse_spawns;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const SCENE: &str = r##"{
  "camera": {"position": [0, 1.7, 0], "target": [0, 1.7, -5]},
  "combat": {"respawn_secs": 2, "spawn": "round_robin"},
  "match": {"min_players": 2, "countdown_secs": 0.4, "round_secs": 20, "results_secs": 5, "score_to_win": 10, "join_in_progress": true, "ready_check": true},
  "shooter": {"start": ["pistol", "knife"],
    "flags": [{"team": 1, "at": [-20, 0, 0]}, {"team": 2, "at": [20, 0, 0]}],
    "sites": [{"name": "A", "at": [0, 0, 0], "radius": 4}],
    "objective": {"capture_limit": 5, "win_rounds": 3}},
  "spawns": [
    {"id": "a", "position": [-30, 0, 0], "yaw_deg": 90, "group": "team1"}, {"id": "b", "position": [-30, 0, 4], "yaw_deg": 90, "group": "team1"},
    {"id": "c", "position": [30, 0, 0], "yaw_deg": 270, "group": "team2"}, {"id": "d", "position": [30, 0, 4], "yaw_deg": 270, "group": "team2"}],
  "objects": [{"id": "floor", "type": "plane", "size": [100, 100]}]
}"##;

fn scene_path(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("net_modes_{}_{tag}.json", std::process::id()));
    std::fs::write(&p, SCENE).unwrap();
    p
}

struct TestServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Server>>,
}

impl TestServer {
    fn start(path: &Path, mode: ModeKind, team_size: usize, limit: Option<u32>) -> TestServer {
        let text = std::fs::read_to_string(path).unwrap();
        // The host's choice goes into the scene the sim is built from; the hash is the file's, as a joiner's copy matches.
        let scene_text = red_engine2::sim::shooter::with_overrides(&text, Some(mode), Some(team_size), limit).unwrap();
        let scene = red_engine2::schema::parse_scene(&scene_text).unwrap();
        let spawns = parse_spawns(&scene_text).unwrap();
        let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), red_engine2::net::map_hash(&text));
        cfg.bot_fill = Some(0);
        let mut server = Server::bind(cfg, MatchSim::try_new(&scene, spawns.clone()).unwrap()).unwrap();
        server.set_logger(|_| {});
        let settings = MatchSettings::from_scene_text(&text).unwrap().unwrap();
        server.enable_flow(settings, move || MatchSim::try_new(&scene, spawns.clone())).unwrap();
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = Some(std::thread::spawn(move || {
            server.run(&s2);
            server
        }));
        TestServer { addr, stop, handle }
    }

    fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().unwrap().join().unwrap();
    }
}

fn bot(server: SocketAddr, path: &Path, name: &str) -> Bot {
    let (_scene, world) = ClientWorld::load(path).unwrap();
    let mut cfg = ClientConfig::new(server, 0, world.map_hash, 0);
    cfg.name = name.to_string();
    Bot::with_client(NetClient::connect_with(cfg).unwrap(), Character::Human, world, Behavior::Idle).unwrap()
}

fn drive(bots: &mut [&mut Bot], what: &str, mut done: impl FnMut(&[&mut Bot]) -> bool) {
    let end = Instant::now() + Duration::from_secs(12);
    while Instant::now() < end {
        let now = Instant::now();
        for b in bots.iter_mut() {
            b.pump(now);
        }
        if done(bots) {
            return;
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    panic!("timed out waiting for: {what}");
}

#[test]
fn a_duel_server_holds_two_people_and_turns_a_third_away() {
    let path = scene_path("duel");
    let server = TestServer::start(&path, ModeKind::Tdm, 1, None);
    let (mut a, mut b) = (bot(server.addr, &path, "Ada"), bot(server.addr, &path, "Bo"));
    drive(&mut [&mut a, &mut b], "both in the lobby", |bs| bs.iter().all(|x| x.client.state() == ConnState::Connected && x.client.status().is_some()));
    let mut c = bot(server.addr, &path, "Cy");
    drive(&mut [&mut a, &mut b, &mut c], "the third is refused", |bs| matches!(bs[2].client.state(), ConnState::Rejected(RejectReason::Full)));
    let st = a.client.status().unwrap();
    assert_eq!((st.team_size, st.mode), (1, ModeKind::Tdm.wire()), "the status says it is a duel");
    server.finish();
}

#[test]
fn the_status_names_the_mode_and_the_modes_own_limit() {
    let path = scene_path("ctf");
    let server = TestServer::start(&path, ModeKind::Ctf, 3, Some(7));
    let mut a = bot(server.addr, &path, "Ada");
    drive(&mut [&mut a], "the lobby", |bs| bs[0].client.status().is_some());
    let st = a.client.status().unwrap();
    assert_eq!(st.mode, ModeKind::Ctf.wire());
    assert_eq!(st.team_size, 3);
    assert_eq!(st.kill_limit, 7, "captures to win, from the host's choice, not the map's kill limit of 10");
    server.finish();

    let server = TestServer::start(&path, ModeKind::Snd, 2, None);
    let mut a = bot(server.addr, &path, "Ada");
    drive(&mut [&mut a], "the lobby again", |bs| bs[0].client.status().is_some());
    assert_eq!(a.client.status().unwrap().kill_limit, 3, "rounds to win, from the map's objective block");
    server.finish();
}

#[test]
fn free_for_all_has_no_teams() {
    let path = scene_path("ffa");
    let server = TestServer::start(&path, ModeKind::Ffa, 2, None);
    let (mut a, mut b) = (bot(server.addr, &path, "Ada"), bot(server.addr, &path, "Bo"));
    drive(&mut [&mut a, &mut b], "both in the lobby", |bs| bs.iter().all(|x| x.client.status().is_some_and(|s| s.roster.len() == 2)));
    let st = a.client.status().unwrap();
    assert!(st.roster.iter().all(|e| e.team == 0), "nobody is on a team: {:?}", st.roster);
    assert_eq!(st.mode, ModeKind::Ffa.wire());
    server.finish();
}
