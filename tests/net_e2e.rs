//! End-to-end multiplayer over **real UDP sockets** (loopback): an authoritative server thread and
//! several headless clients running the real prediction/interpolation stack.
//!
//! These prove, without a window: two clients join, move and see each other smoothly; a client can
//! disconnect and reconnect (gracefully and by timing out) and gets its player back; an authoritative
//! physics prop pushed by one player is observed identically by another; and all of it keeps working
//! through a lossy, laggy link. (Separate OS processes and real windows are covered by
//! `tests/net_processes.rs` and `scripts/multiplayer_demo.ps1`.)
//!
//! Timing-based, so assertions are tolerant; each test runs its own server on its own port.

use glam::Vec2;
use red_engine2::net::bot::{Behavior, Bot, BotFrame, ClientWorld};
use red_engine2::net::client::ConnState;
use red_engine2::net::server::{Server, ServerConfig};
use red_engine2::player::Character;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

fn lab_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")
}

struct TestServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Server>>,
}

impl TestServer {
    fn start(group: &str, timeout_ms: u64, kick: Option<&str>) -> TestServer {
        let path = lab_path();
        let text = std::fs::read_to_string(&path).unwrap();
        let scene = red_engine2::schema::parse_scene(&text).unwrap();
        let mut spawns = parse_spawns(&text).unwrap();
        spawns.retain(|s| s.group == group);
        let sim = MatchSim::new(&scene, spawns);
        let kick_prop = kick.map(|id| {
            let obj = scene.objects.iter().position(|o| o.id == id).unwrap();
            sim.props().prop_of_object(obj).expect("kick target must be a loose prop")
        });
        let mut cfg = ServerConfig::new("127.0.0.1:0".parse().unwrap(), red_engine2::net::map_hash(&text));
        cfg.client_timeout = Duration::from_millis(timeout_ms);
        let mut server = Server::bind(cfg, sim).unwrap();
        server.set_logger(|_| {});
        if let Some(p) = kick_prop {
            server.set_demo_kick(p);
        }
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), server.local_addr().unwrap().port());
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = Some(std::thread::spawn(move || {
            server.run(&s2);
            server
        }));
        TestServer { addr, stop, handle }
    }

    fn finish(mut self) -> Server {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().unwrap().join().unwrap()
    }
}

fn bot(server: SocketAddr, who: Character, behavior: Behavior, token: u64) -> Bot {
    let (_scene, world) = ClientWorld::load(&lab_path()).unwrap();
    Bot::new(server, who, world, behavior, token).unwrap()
}

/// Runs `bot` for `secs`, recording a frame every ~10 ms.
fn run_bot(bot: &mut Bot, secs: f64) -> Vec<BotFrame> {
    let mut frames = Vec::new();
    let mut last = -1.0;
    bot.run(Duration::from_secs_f64(secs), |f| {
        if f.t - last >= 0.010 {
            frames.push(f.clone());
            last = f.t;
        }
        true
    });
    frames
}

/// Runs `bot` until it is connected (so the order in which test clients join, and therefore which
/// spawn each one gets, is deterministic).
fn join(bot: &mut Bot) {
    bot.run(Duration::from_secs(4), |f| f.conn != ConnState::Connected || f.me_state.is_none());
    assert_eq!(bot.client.state(), ConnState::Connected, "{:?}", bot.events);
}

fn last_remote(frames: &[BotFrame], id: u8) -> Option<Vec2> {
    frames.iter().rev().find_map(|f| f.remote.iter().find(|(i, _)| *i == id).map(|(_, p)| Vec2::new(p.pos.x, p.pos.z)))
}

// ---------------------------------------------------------------------------------------------

#[test]
fn two_clients_join_move_and_see_each_other_smoothly() {
    let server = TestServer::start("duel", 3000, None);
    let mut a = bot(server.addr, Character::Human, Behavior::Forward { yaw_deg: 0.0, sprint: false }, 0);
    join(&mut a);
    let mut b = bot(server.addr, Character::Human, Behavior::Idle, 0);
    join(&mut b);
    // (A joined first: spawn_a at (-22, -3) facing B; B is at spawn_b (-18, -3).)
    let (fa, fb) = std::thread::scope(|s| {
        let ha = s.spawn(|| run_bot(&mut a, 4.0));
        let hb = s.spawn(|| run_bot(&mut b, 4.0));
        (ha.join().unwrap(), hb.join().unwrap())
    });
    assert!(a.events.iter().any(|e| e.1.starts_with("connected as player")), "{:?}", a.events);
    assert!(b.events.iter().any(|e| e.1.starts_with("connected as player")), "{:?}", b.events);
    let (ida, idb) = (a.client.my_id().unwrap(), b.client.my_id().unwrap());
    assert_ne!(ida, idb, "distinct player ids");

    // A sees B where B stands (spawn_b: -18, -3).
    let seen_b = last_remote(&fa, idb).expect("A never saw B");
    assert!(seen_b.distance(Vec2::new(-18.0, -3.0)) < 0.05, "A sees B at {seen_b:?}");

    // B sees A walk north (toward -Z) smoothly: monotonic, no jumps.
    let path: Vec<Vec2> = fb.iter().filter_map(|f| f.remote.iter().find(|(i, _)| *i == ida).map(|(_, p)| Vec2::new(p.pos.x, p.pos.z))).collect();
    assert!(path.len() > 100, "B saw A in only {} frames", path.len());
    let mut worst_step = 0.0f32;
    for w in path.windows(2) {
        assert!(w[1].y <= w[0].y + 1e-3, "A appeared to move backwards: {:?} -> {:?}", w[0], w[1]);
        worst_step = worst_step.max(w[0].distance(w[1]));
    }
    assert!(worst_step < 0.08, "largest per-frame step {worst_step} m (3.2 m/s at ~100 fps is 0.03)");
    let travelled = path.first().unwrap().distance(*path.last().unwrap());
    assert!(travelled > 3.0, "A really did walk: {travelled} m");

    // Authoritative truth: A ended where the server says, B saw it, and A's own prediction never had to jump.
    let server = server.finish();
    let truth = server.sim().player(ida as usize).expect("A is still connected").state.pos;
    assert!(path.last().unwrap().distance(truth) < 0.06, "B sees {:?}, server says {truth:?}", path.last());
    let pa = a.predictor.as_ref().unwrap();
    assert!(pa.state.pos.distance(truth) < 0.05, "A's prediction {:?} vs server {truth:?}", pa.state.pos);
    assert!(pa.worst_correction < 0.05, "prediction should have matched the server (worst correction {} m)", pa.worst_correction);
    assert!(a.client.stats().rtt_ms < 50.0, "loopback rtt {} ms", a.client.stats().rtt_ms);
    assert_eq!(server.stats().joins, 2);
}

#[test]
fn a_client_can_disconnect_and_reconnect_gracefully_and_by_timing_out() {
    let server = TestServer::start("duel", 700, None);
    let mut a = bot(server.addr, Character::Human, Behavior::Idle, 0);
    join(&mut a);
    let mut b = bot(server.addr, Character::Human, Behavior::Forward { yaw_deg: 0.0, sprint: false }, 0);
    join(&mut b);
    let addr = server.addr;

    let (fa, b_positions) = std::thread::scope(|s| {
        let ha = s.spawn(|| run_bot(&mut a, 9.0)); // A watches the whole time
        let hb = s.spawn(move || {
            let mut positions = Vec::new();
            // B walks for a second, then leaves politely.
            let f = run_bot(&mut b, 1.5);
            b.behavior = Behavior::Idle;
            run_bot(&mut b, 0.3);
            let (token, id) = (b.client.token(), b.client.my_id().unwrap());
            let left_at = b.predictor.as_ref().unwrap().state.pos;
            positions.push(("left", left_at));
            b.client.disconnect();
            drop(b);
            std::thread::sleep(Duration::from_millis(1200)); // gone for a while: A must see the player vanish
            // Reconnect with the token: same player, same place.
            let mut b2 = bot(addr, Character::Human, Behavior::Idle, token);
            let f2 = run_bot(&mut b2, 1.2);
            let resumed = b2.predictor.as_ref().unwrap().state.pos;
            positions.push(("resumed", resumed));
            assert_eq!(b2.client.my_id(), Some(id), "resumed as the same player id");
            assert_eq!(b2.client.token(), token, "and the same token");
            // Now vanish WITHOUT saying goodbye (a crash / unplugged cable): the server must time us out.
            let addr2 = addr;
            drop(b2); // no Bye sent
            std::thread::sleep(Duration::from_millis(1600)); // > the 700 ms timeout
            let mut b3 = bot(addr2, Character::Human, Behavior::Idle, token);
            run_bot(&mut b3, 1.2);
            positions.push(("resumed after timeout", b3.predictor.as_ref().unwrap().state.pos));
            let _ = (f, f2);
            positions
        });
        (ha.join().unwrap(), hb.join().unwrap())
    });

    // What A saw of B over time: present, gone (Bye), present (resumed), gone (silent timeout), present (resumed).
    let idb = 1u8; // A is the first to join (slot 0), B the second
    let pattern: Vec<bool> = fa.iter().map(|f| f.remote.iter().any(|(i, _)| *i == idb)).collect();
    let mut runs: Vec<bool> = Vec::new();
    for p in pattern {
        if runs.last() != Some(&p) {
            runs.push(p);
        }
    }
    // ... and once more at the very end: the last reconnected client is dropped without a goodbye when its
    // thread finishes, so the server times it out and A correctly sees it vanish.
    assert_eq!(runs, vec![false, true, false, true, false, true, false], "A's view of B over time (starts before B's first snapshot): {runs:?}");

    // Resuming restores the position B had, both times.
    let left = b_positions[0].1;
    for (label, p) in &b_positions[1..] {
        assert!(p.distance(left) < 0.1, "{label}: {p:?} should be where B left ({left:?})");
    }
    let server = server.finish();
    assert!(server.stats().resumes >= 2, "resumes: {}", server.stats().resumes);
    assert!(server.stats().timeouts >= 1, "the silent client must be timed out: {}", server.stats().timeouts);
    assert_eq!(server.stats().joins, 2, "only two fresh joins; the rest were resumes");
}

#[test]
fn a_prop_pushed_by_one_player_moves_authoritatively_and_the_other_sees_the_same() {
    let server = TestServer::start("props", 3000, None);
    let (scene, world) = ClientWorld::load(&lab_path()).unwrap();
    let domino0 = world.prop_objects.iter().position(|&o| scene.objects[o].id == "domino_0").unwrap() as u16;
    // Player 1 spawns at (1.0, 5.4) facing -Z, 1.4 m from domino_0: walking forward pushes it.
    let mut pusher = bot(server.addr, Character::Human, Behavior::Forward { yaw_deg: 0.0, sprint: false }, 0);
    join(&mut pusher);
    let mut watcher = bot(server.addr, Character::Human, Behavior::Idle, 0);
    join(&mut watcher);

    let (fp, fw) = std::thread::scope(|s| {
        let h1 = s.spawn(|| {
            let f = run_bot(&mut pusher, 2.0);
            pusher.behavior = Behavior::Idle;
            let mut g = run_bot(&mut pusher, 3.5);
            let mut all = f;
            all.append(&mut g);
            all
        });
        let h2 = s.spawn(|| run_bot(&mut watcher, 5.5));
        (h1.join().unwrap(), h2.join().unwrap())
    });

    let server = server.finish();
    let truth = server.sim().props().prop_pose(domino0 as usize).w_axis;
    let authored = scene.objects[world.prop_objects[domino0 as usize]].position.sample(0.0);
    assert!((truth.truncate() - authored).length() > 0.3, "the server moved the barrel: {:?} vs authored {authored:?}", truth.truncate());
    assert!(server.sim().props().dynamic_count() >= 1);

    // The watcher (who never touched anything) saw it move, in many intermediate positions...
    let seen: Vec<glam::Vec3> = fw.iter().filter_map(|f| f.props.iter().find(|(i, _)| *i == domino0).map(|(_, p)| p.pos)).collect();
    assert!(seen.len() > 50, "the watcher saw the barrel in {} frames", seen.len());
    let distinct = seen.windows(2).filter(|w| w[0].distance(w[1]) > 0.005).count();
    assert!(distinct > 10, "it was seen *moving* ({distinct} distinct steps), not teleporting");
    // ... and both clients end up agreeing with the authoritative pose.
    for (who, frames) in [("pusher", &fp), ("watcher", &fw)] {
        let last = frames.iter().rev().find_map(|f| f.props.iter().find(|(i, _)| *i == domino0).map(|(_, p)| p.pos)).unwrap_or_else(|| panic!("{who} never saw the barrel"));
        assert!(last.distance(truth.truncate()) < 0.03, "{who} sees the barrel at {last:?}, server says {:?}", truth.truncate());
    }
    // Every promoted prop agrees, not just the one we watched.
    for (who, frames) in [("pusher", &fp), ("watcher", &fw)] {
        let last = frames.last().unwrap();
        for (id, pose) in &last.props {
            let t = server.sim().props().prop_pose(*id as usize).w_axis.truncate();
            assert!(pose.pos.distance(t) < 0.05, "{who}: prop {id} at {:?}, server {t:?}", pose.pos);
        }
    }
}

#[test]
fn the_server_can_keep_a_prop_moving_with_nobody_touching_it() {
    // Kick a barrel every 3 s; a single idle client just watches it travel.
    let server = TestServer::start("duel", 3000, Some("domino_4"));
    let (scene, world) = ClientWorld::load(&lab_path()).unwrap();
    let id = world.prop_objects.iter().position(|&o| scene.objects[o].id == "domino_4").unwrap() as u16;
    let mut watcher = bot(server.addr, Character::Human, Behavior::Idle, 0);
    let frames = run_bot(&mut watcher, 5.5);
    let server = server.finish();
    let seen: Vec<glam::Vec3> = frames.iter().filter_map(|f| f.props.iter().find(|(i, _)| *i == id).map(|(_, p)| p.pos)).collect();
    assert!(seen.len() > 20, "saw the kicked barrel in {} frames", seen.len());
    let span = seen.iter().map(|p| p.distance(seen[0])).fold(0.0f32, f32::max);
    assert!(span > 0.3, "the barrel travelled {span} m by itself");
    let last = seen.last().unwrap();
    assert!(last.distance(server.sim().props().prop_pose(id as usize).w_axis.truncate()) < 0.05);
}

#[test]
fn the_server_rejects_the_wrong_map_and_a_full_match_and_survives_garbage() {
    let server = TestServer::start("duel", 3000, None);
    // Wrong map hash: rejected, not joined.
    let (_s, mut world) = ClientWorld::load(&lab_path()).unwrap();
    world.map_hash ^= 1;
    let mut wrong = Bot::new(server.addr, Character::Human, world, Behavior::Idle, 0).unwrap();
    run_bot(&mut wrong, 0.6);
    assert_eq!(wrong.client.state(), ConnState::Rejected(red_engine2::net::protocol::RejectReason::WrongMap));

    // Garbage datagrams, truncated packets and oversized packets do not hurt it.
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    for n in [0usize, 1, 5, 40, 1399, 1500, 2000] {
        let junk: Vec<u8> = (0..n).map(|i| (i * 31 % 251) as u8).collect();
        let _ = sock.send_to(&junk, server.addr);
    }
    let _ = sock.send_to(&[0x44, 0x52, 1, 0, 2, 0xff, 0xff], server.addr); // right magic, nonsense body

    // Fill the match (8 players), then a ninth is refused; a real client still plays fine.
    let mut bots: Vec<Bot> = (0..8).map(|_| bot(server.addr, Character::Human, Behavior::Idle, 0)).collect();
    std::thread::scope(|s| {
        for b in bots.iter_mut() {
            s.spawn(move || run_bot(b, 1.0));
        }
    });
    assert!(bots.iter().all(|b| b.client.state() == ConnState::Connected), "{:?}", bots.iter().map(|b| b.client.state()).collect::<Vec<_>>());
    let mut ninth = bot(server.addr, Character::Human, Behavior::Idle, 0);
    run_bot(&mut ninth, 0.6);
    assert_eq!(ninth.client.state(), ConnState::Rejected(red_engine2::net::protocol::RejectReason::Full));
    let server = server.finish();
    assert!(server.stats().bad_packets >= 5, "garbage was counted, not crashed on: {}", server.stats().bad_packets);
    assert_eq!(server.client_count(), 8);
}

// ---------------------------------------------------------------------------------------------
// A lossy, laggy link
// ---------------------------------------------------------------------------------------------

/// A UDP proxy between one client and the server that drops a fraction of datagrams and delays the
/// rest by `delay +- jitter`, in both directions. Returns the address the client should talk to.
fn lossy_proxy(server: SocketAddr, loss: f64, delay_ms: u64, jitter_ms: u64, stop: Arc<AtomicBool>) -> (SocketAddr, JoinHandle<()>) {
    let front = UdpSocket::bind("127.0.0.1:0").unwrap(); // the client talks to this
    let back = UdpSocket::bind("127.0.0.1:0").unwrap(); // and the server sees this
    front.set_nonblocking(true).unwrap();
    back.set_nonblocking(true).unwrap();
    let front_addr = front.local_addr().unwrap();
    let h = std::thread::spawn(move || {
        let mut rng = 0x2545_f491_4f6c_dd1du64;
        let mut rand = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng >> 11) as f64 / (1u64 << 53) as f64
        };
        let mut client: Option<SocketAddr> = None;
        // (release time, to_server?, bytes)
        let mut queue: Vec<(Instant, bool, Vec<u8>)> = Vec::new();
        let mut buf = [0u8; 2048];
        while !stop.load(Ordering::Relaxed) {
            while let Ok((n, from)) = front.recv_from(&mut buf) {
                client = Some(from);
                if rand() >= loss {
                    let d = delay_ms as f64 + (rand() - 0.5) * 2.0 * jitter_ms as f64;
                    queue.push((Instant::now() + Duration::from_micros((d.max(0.0) * 1000.0) as u64), true, buf[..n].to_vec()));
                }
            }
            while let Ok((n, _)) = back.recv_from(&mut buf) {
                if rand() >= loss {
                    let d = delay_ms as f64 + (rand() - 0.5) * 2.0 * jitter_ms as f64;
                    queue.push((Instant::now() + Duration::from_micros((d.max(0.0) * 1000.0) as u64), false, buf[..n].to_vec()));
                }
            }
            let now = Instant::now();
            let mut i = 0;
            while i < queue.len() {
                if queue[i].0 <= now {
                    let (_, to_server, bytes) = queue.swap_remove(i);
                    if to_server {
                        let _ = back.send_to(&bytes, server);
                    } else if let Some(c) = client {
                        let _ = front.send_to(&bytes, c);
                    }
                } else {
                    i += 1;
                }
            }
            std::thread::sleep(Duration::from_micros(500));
        }
    });
    (front_addr, h)
}

#[test]
fn it_still_works_through_15_percent_loss_and_40_ms_latency_with_jitter() {
    let server = TestServer::start("props", 3000, None);
    let stop = Arc::new(AtomicBool::new(false));
    let (pa, ha) = lossy_proxy(server.addr, 0.15, 40, 15, stop.clone());
    let (pb, hb) = lossy_proxy(server.addr, 0.15, 40, 15, stop.clone());
    let (scene, world) = ClientWorld::load(&lab_path()).unwrap();
    let domino0 = world.prop_objects.iter().position(|&o| scene.objects[o].id == "domino_0").unwrap() as u16;
    let mut pusher = bot(pa, Character::Human, Behavior::Forward { yaw_deg: 0.0, sprint: false }, 0);
    join(&mut pusher);
    let mut watcher = bot(pb, Character::Human, Behavior::Idle, 0);
    join(&mut watcher);
    let (fp, fw) = std::thread::scope(|s| {
        let h1 = s.spawn(|| {
            let mut all = run_bot(&mut pusher, 2.5);
            pusher.behavior = Behavior::Idle;
            all.append(&mut run_bot(&mut pusher, 4.0));
            all
        });
        let h2 = s.spawn(|| run_bot(&mut watcher, 6.5));
        (h1.join().unwrap(), h2.join().unwrap())
    });
    stop.store(true, Ordering::Relaxed);
    ha.join().unwrap();
    hb.join().unwrap();
    let server = server.finish();
    let truth = server.sim().props().prop_pose(domino0 as usize).w_axis.truncate();
    let authored = scene.objects[world.prop_objects[domino0 as usize]].position.sample(0.0);
    assert!((truth - authored).length() > 0.3, "the barrel was pushed");

    // Loss and lag cost RTT and an occasional resend, but not correctness.
    let last_seen = |frames: &[BotFrame]| frames.iter().rev().find_map(|f| f.props.iter().find(|(i, _)| *i == domino0).map(|(_, p)| p.pos)).expect("never saw the barrel");
    assert!(last_seen(&fw).distance(truth) < 0.05, "watcher {:?} vs server {truth:?}", last_seen(&fw));
    assert!(last_seen(&fp).distance(truth) < 0.05, "pusher {:?} vs server {truth:?}", last_seen(&fp));
    for (who, b) in [("pusher", &pusher), ("watcher", &watcher)] {
        let s = b.client.stats();
        assert!(s.rtt_ms > 60.0 && s.rtt_ms < 200.0, "{who} rtt {} ms (expect ~80 + jitter)", s.rtt_ms);
        assert!(s.snapshots_missed > 0, "{who}: the link really was lossy ({} missed of {})", s.snapshots_missed, s.snapshots);
        assert_eq!(b.client.state(), ConnState::Connected, "{who} stayed connected");
    }
    // The pusher predicted its own walk through 80 ms of round trip and 15% loss and ended on the server's position.
    let truth_p = server.sim().player(pusher.client.my_id().unwrap() as usize).unwrap().state.pos;
    let pred = pusher.predictor.as_ref().unwrap();
    assert!(pred.state.pos.distance(truth_p) < 0.1, "prediction {:?} vs server {truth_p:?}", pred.state.pos);
    // And the watcher still saw the other player (the pusher) move without teleporting.
    let path: Vec<Vec2> = fw.iter().filter_map(|f| f.remote.iter().find(|(i, _)| Some(*i) == pusher.client.my_id()).map(|(_, p)| Vec2::new(p.pos.x, p.pos.z))).collect();
    let worst = path.windows(2).map(|w| w[0].distance(w[1])).fold(0.0f32, f32::max);
    assert!(path.len() > 100 && worst < 0.25, "watcher saw the pusher in {} frames, worst step {worst}", path.len());
}
