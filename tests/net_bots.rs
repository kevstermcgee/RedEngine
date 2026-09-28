//! Bots over real UDP (ADR 0044): a server fills empty slots with AI players that show up in every client's snapshots and roster like anyone
//! else; a human joining takes a bot's place and leaving gives it back; and in a match flow the lobby names the bots that will play, the round
//! is fought (and can be won) by them, and the results screen has their scores. Timing-based, so waits are generous.

use red_engine2::net::bot::{Behavior, Bot, ClientWorld};
use red_engine2::net::client::{ClientConfig, ConnState, NetClient};
use red_engine2::net::protocol::{RosterEntry, ROSTER_BOT, ROSTER_READY};
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::player::Character;
use red_engine2::sim::flow::{MatchSettings, Phase};
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

fn lab_text(extra: Value) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for (k, x) in extra.as_object().cloned().unwrap_or_default() {
        v[k] = x;
    }
    v.to_string()
}

struct TestServer {
    addr: SocketAddr,
    map_hash: u32,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Server>>,
    scene_text: String,
}

impl TestServer {
    fn start(extra: Value, flow: Option<MatchSettings>) -> TestServer {
        let text = lab_text(extra);
        let scene = red_engine2::schema::parse_scene(&text).unwrap();
        let mut spawns = parse_spawns(&text).unwrap();
        spawns.retain(|s| s.group == "duel");
        let hash = red_engine2::net::map_hash(&text);
        let mut server = Server::bind(ServerConfig::new("127.0.0.1:0".parse().unwrap(), hash), MatchSim::try_new(&scene, spawns.clone()).unwrap()).unwrap();
        server.set_logger(|_| {});
        if let Some(settings) = flow {
            let t = text.clone();
            server
                .enable_flow(settings, move || {
                    let scene = red_engine2::schema::parse_scene(&t).map_err(|e| e.join("; "))?;
                    MatchSim::try_new(&scene, spawns.clone())
                })
                .unwrap();
        }
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = Some(std::thread::spawn(move || {
            server.run(&s2);
            server
        }));
        TestServer { addr, map_hash: hash, stop, handle, scene_text: text }
    }

    fn human(&self, name: &str, auto_ready: bool) -> Bot {
        let _ = &self.scene_text;
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json");
        let (_scene, mut world) = ClientWorld::load(&path).unwrap();
        world.map_hash = self.map_hash;
        let mut cfg = ClientConfig::new(self.addr, 0, self.map_hash, 0);
        cfg.name = name.to_string();
        let mut b = Bot::with_client(NetClient::connect_with(cfg).unwrap(), Character::Human, world, Behavior::Idle).unwrap();
        b.auto_ready = auto_ready;
        b
    }

    fn finish(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().unwrap().join().unwrap();
    }
}

fn drive(bots: &mut [&mut Bot], what: &str, mut done: impl FnMut(&[&mut Bot]) -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
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
    let logs: Vec<String> = bots.iter().map(|b| format!("{:?}", b.events)).collect();
    panic!("timed out waiting for: {what}\n{}", logs.join("\n"));
}

fn roster(b: &Bot) -> Vec<RosterEntry> {
    b.client.status().map(|s| s.roster.clone()).unwrap_or_default()
}

fn bots_in(r: &[RosterEntry]) -> usize {
    r.iter().filter(|e| e.flags & ROSTER_BOT != 0).count()
}

const CREW: fn() -> Value = || {
    json!({
        "weapons": {"starting": "smg"},
        "bots": {"fill": 4, "skill": "normal", "roster": [
            {"name": "Dusty", "character": "cowboy", "style": "rusher"},
            {"name": "Merlot", "character": "wizard"},
            {"name": "Zorp", "character": "alien", "style": "acrobat"}]}
    })
};

#[test]
fn a_server_fills_empty_slots_with_bots_a_human_takes_a_place_and_gives_it_back() {
    let server = TestServer::start(CREW(), None);
    let mut a = server.human("Ada", false);
    drive(&mut [&mut a], "Ada sees the bots", |bs| {
        let r = roster(bs[0]);
        bs[0].client.state() == ConnState::Connected && r.len() == 4 && bots_in(&r) == 3
    });
    let r = roster(&a);
    let mut names: Vec<&str> = r.iter().filter(|e| e.flags & ROSTER_BOT != 0).map(|e| e.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Dusty", "Merlot", "Zorp"], "the roster names the bots");
    assert!(r.iter().filter(|e| e.flags & ROSTER_BOT != 0).all(|e| e.flags & ROSTER_READY != 0 && e.ping_ms == 0));
    let human = r.iter().find(|e| e.name == "Ada").unwrap();
    assert!(human.id < 4, "humans keep the small ids: Ada is {}", human.id);
    assert!(r.iter().filter(|e| e.flags & ROSTER_BOT != 0).all(|e| e.id > human.id), "bots take the top slots");
    // Their bodies are in the snapshots: three remote players, and they are moving (hunting).
    drive(&mut [&mut a], "three remote players are drawn", |bs| bs[0].frame(Instant::now()).remote.len() == 3);
    let start: Vec<_> = a.frame(Instant::now()).remote.iter().map(|(id, p)| (*id, p.pos)).collect();
    drive(&mut [&mut a], "a bot moves", |bs| {
        bs[0].frame(Instant::now()).remote.iter().any(|(id, p)| start.iter().any(|(sid, sp)| sid == id && (p.pos - *sp).length() > 1.0))
    });

    // A second human takes a bot's place.
    let mut b = server.human("Bo", false);
    drive(&mut [&mut a, &mut b], "Bo joined and one bot left", |bs| {
        let r = roster(bs[0]);
        r.len() == 4 && bots_in(&r) == 2 && r.iter().any(|e| e.name == "Bo")
    });
    // ... and leaving hands the room back.
    b.client.disconnect();
    drop(b);
    drive(&mut [&mut a], "the bot came back", |bs| {
        let r = roster(bs[0]);
        r.len() == 4 && bots_in(&r) == 3
    });
    server.finish();
}

#[test]
fn shots_and_damage_reach_a_client_as_events() {
    // Three SMG bots and one idle human in the duel hall: the bots shoot, the human hears it and is hurt, and the counters on the wire say so.
    let server = TestServer::start(CREW(), None);
    let mut a = server.human("Ada", false);
    drive(&mut [&mut a], "Ada hears the fighting and is shot at", |bs| bs[0].heard_shots >= 5 && bs[0].times_hurt >= 1);
    assert!(a.heard_shots >= 5, "shots fired by other players were reported: {}", a.heard_shots);
    assert!(a.times_hurt >= 1, "damage taken was reported: {}", a.times_hurt);
    assert_eq!(a.landed_hits, 0, "an idle client hits nobody");
    server.finish();
}

#[test]
fn with_no_fill_there_are_no_bots_and_the_config_can_switch_them_off() {
    let extra = json!({"bots": {"fill": 0}});
    let server = TestServer::start(extra, None);
    let mut a = server.human("Ada", false);
    drive(&mut [&mut a], "Ada is alone", |bs| bs[0].client.state() == ConnState::Connected && roster(bs[0]).len() == 1);
    assert_eq!(bots_in(&roster(&a)), 0);
    server.finish();
}

#[test]
fn in_a_match_flow_the_lobby_names_the_bots_and_a_bot_can_win_the_round() {
    let settings =
        MatchSettings { min_players: 1, countdown_secs: 0.4, round_secs: 40.0, results_secs: 30.0, score_to_win: 3, join_in_progress: true, ready_check: true };
    let server = TestServer::start(CREW(), Some(settings));
    let mut a = server.human("Ada", true);
    // The lobby shows the fighters before there is a world.
    drive(&mut [&mut a], "the lobby lists the bots", |bs| {
        let r = roster(bs[0]);
        phase(bs[0]) == Phase::Waiting && bots_in(&r) == 3
    });
    assert!(roster(&a).iter().filter(|e| e.flags & ROSTER_BOT != 0).all(|e| e.flags & ROSTER_READY != 0), "bots are ready, so only the human is waited for");
    // Ada is auto-ready: countdown, round; the bots fight and someone reaches three kills.
    drive(&mut [&mut a], "the round starts with bodies in the world", |bs| phase(bs[0]) == Phase::Playing && bs[0].frame(Instant::now()).remote.len() == 3);
    drive(&mut [&mut a], "the round ends", |bs| phase(bs[0]) == Phase::Results);
    let st = a.client.status().unwrap().clone();
    assert_eq!(st.end_code, 2, "someone reached the score to win (end code {}, roster {:?})", st.end_code, st.roster);
    let top = st.roster.iter().max_by_key(|e| e.score).unwrap();
    assert!(top.score >= 3, "the winner has the score: {top:?}");
    assert_eq!(st.winner, top.id, "the winner is whoever has the top score");
    assert!(st.roster.len() == 4 && bots_in(&st.roster) == 3, "the results still list the bots: {:?}", st.roster);
    server.finish();
}

fn phase(b: &Bot) -> Phase {
    b.client.phase()
}
