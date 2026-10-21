//! The advertised multiplayer workflow, end to end: HOST makes a game and shows a code; a friend types that code into JOIN and plays.
//!
//! Nothing here is hand-wired. The host is `LocalHost` started from `PublicOptions::for_friends` (what the HOST button builds, with only the
//! interface, port and router-mapping overridden so a test opens nothing to the network); the code a friend gets is `LocalHost::share_codes`;
//! the join is `JoinTarget::parse` + `client_config` (what the JOIN button runs) feeding the real `NetClient` over real QUIC, through a real
//! `RelayServer` on the loopback. That is the path `src/bin/re2/kc/app.rs` takes, minus the window, and it is the path that was broken: the
//! relay code resolved a host but dropped the join key the host demanded, so every "HOST, then JOIN with the code" ended in `NeedsKey`.
//!
//! Covered: host -> short-code join; several joiners; wrong, mistyped and expired codes; a wrong, missing and a right key; direct joining;
//! reconnect (a player coming back with their token, and a network outage the relay rides out); the host leaving and the session ending;
//! transport failures (no relay, a relay that vouches for no identity, a wrong identity, a relay that dies mid-game).
//!
//! And the join that does not give up (ADR 2026-10-20-join-by-short-code-survives-a-wrong-map-and-a-slow-relay), through `JoinTask` as the window runs
//! it: a host that plays another map than the first one tried, every cause of failure worded for the player, a lost `Resolve`, a lost `Claim`, a slow link
//! that makes the repeated claims arrive after the pairing, and the bytes of a released client against the current relay.

use red_engine2::net::client::{ClientConfig, ClientTransportConfig, ConnState, NetClient};
use red_engine2::net::host::{HostOptions, LocalHost, PublicOptions};
use red_engine2::net::join::{JoinError, JoinTarget};
use red_engine2::net::join_task::{JoinCause, JoinFailure, JoinPlan, JoinTask, Joined, MapChoice};
use red_engine2::net::netsim::{profile, LinkProfile, LossyProxy};
use red_engine2::net::protocol::RejectReason;
use red_engine2::net::quic::{ServerIdentity, ServerTrust};
use red_engine2::net::relay::{code_to_string, ShortJoin};
use red_engine2::net::relay_server::{HostBridge, RelayServer, RelayServerOptions, ResolveError};
use red_engine2::net::transport::Security;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const SCENE: &str = r##"{"camera":{"position":[-6,1.7,0],"target":[0,1,0]},
 "spawns":[{"id":"a","position":[-6,0,-3],"yaw_deg":90},{"id":"b","position":[-6,0,-1],"yaw_deg":90},
           {"id":"c","position":[-6,0,1],"yaw_deg":90},{"id":"d","position":[-6,0,3],"yaw_deg":90}],
 "objects":[{"id":"floor","type":"plane","size":[20,10],"position":[0,0.01,0]}]}"##;

const LOOPBACK: [u8; 4] = [127, 0, 0, 1];

/// A relay on the loopback, serving on its own thread until dropped.
struct TestRelay {
    addr: SocketAddr,
    server: Arc<RelayServer>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TestRelay {
    fn start(options: RelayServerOptions) -> TestRelay {
        let server = Arc::new(RelayServer::bind(RelayServerOptions { bind: SocketAddr::from((LOOPBACK, 0)), ..options }).unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let (s, f) = (server.clone(), stop.clone());
        let thread = Some(std::thread::spawn(move || s.run(&f)));
        TestRelay { addr: server.local_addr(), server, stop, thread }
    }

    fn address(&self) -> String {
        self.addr.to_string()
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.stop();
    }
}

fn relay_options() -> RelayServerOptions {
    RelayServerOptions { housekeeping_tick: Duration::from_millis(50), ..Default::default() }
}

/// One hosted game and the relay it registered with.
struct Lobby {
    relay: TestRelay,
    host: Option<LocalHost>,
    map_hash: u32,
    dir: PathBuf,
}

/// How the host's join key is set up.
#[derive(Clone)]
enum Key {
    /// Exactly what HOST does: a fresh generated key.
    Generated,
    /// A host that asks for no key at all.
    None,
}

fn scratch_dir(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("red-join-flow-{what}-{}", red_engine2::crypto::random_u64().unwrap()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// HOST, as the button does it (`PublicOptions::for_friends`), registering with `relay`. Only what would open the machine to a network is overridden.
fn host_options(dir: &std::path::Path, relay: &TestRelay, key: Key) -> HostOptions {
    let mut public = PublicOptions::for_friends(dir.join("identity"), Some(relay.address())).unwrap();
    public.bind = Some(IpAddr::from(LOOPBACK));
    public.port = 0;
    public.upnp = false;
    if matches!(key, Key::None) {
        public.key = None;
    }
    HostOptions { fill: Some(0), public: Some(public), ..Default::default() }
}

fn lobby_with(options: RelayServerOptions, key: Key) -> Lobby {
    let relay = TestRelay::start(options);
    let dir = scratch_dir("lobby");
    let map = dir.join("map.json");
    std::fs::write(&map, SCENE).unwrap();
    let host = LocalHost::start(&map, &host_options(&dir, &relay, key)).unwrap();
    // HOST returns at once and the relay answers on a thread of its own (a down relay must not freeze the window): the lobby has its online code a
    // moment later, which is when a friend can be given it.
    wait_for("the relay to give the host its code", || host.relay_code());
    Lobby { relay, host: Some(host), map_hash: red_engine2::net::map_hash(SCENE), dir }
}

fn lobby() -> Lobby {
    lobby_with(relay_options(), Key::Generated)
}

impl Lobby {
    fn host(&self) -> &LocalHost {
        self.host.as_ref().expect("the host is still running")
    }

    /// What the lobby screen shows first.
    fn advertised_code(&self) -> String {
        self.host().share_codes().first().cloned().expect("a hosted game with a relay advertises a code")
    }

    /// What the JOIN button does with the text a friend typed, up to the first packet.
    fn join_config(&self, typed: &str) -> Result<ClientConfig, JoinError> {
        join_config(typed, Some(&self.relay.address()), self.map_hash)
    }

    /// A friend typing `typed` into JOIN.
    fn join(&self, typed: &str) -> NetClient {
        NetClient::connect_with(self.join_config(typed).unwrap()).unwrap()
    }

    fn direct_code(&self, key: Option<&str>) -> String {
        let host = self.host();
        let mut code = format!("{}#{}", host.addr(), host.fingerprint().unwrap());
        if let Some(k) = key {
            code.push('#');
            code.push_str(k);
        }
        code
    }

    fn drop_host(&mut self) {
        self.host = None;
    }
}

impl Drop for Lobby {
    fn drop(&mut self) {
        self.host = None;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn join_config(typed: &str, relay: Option<&str>, map_hash: u32) -> Result<ClientConfig, JoinError> {
    join_config_within(typed, relay, map_hash, Duration::from_secs(5))
}

fn join_config_within(typed: &str, relay: Option<&str>, map_hash: u32, wait: Duration) -> Result<ClientConfig, JoinError> {
    let mut cfg = JoinTarget::parse(typed)?.client_config(relay, wait)?;
    cfg.map_hash = map_hash;
    cfg.name = "Friend".to_string();
    Ok(cfg)
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

fn joined(client: &mut NetClient) -> bool {
    poll_until(client, 10.0, |c| c.state() == ConnState::Connected)
}

fn rejected_with(client: &mut NetClient) -> Option<RejectReason> {
    poll_until(client, 10.0, |c| matches!(c.state(), ConnState::Rejected(_)));
    match client.state() {
        ConnState::Rejected(r) => Some(r),
        _ => None,
    }
}

#[test]
fn what_host_shows_is_what_join_accepts() {
    let lobby = lobby();
    let shown = lobby.advertised_code();
    let host = lobby.host();
    // The advertised code is the relay's short code *and* the host's admission key, and it parses back to exactly that.
    let key = host.key().expect("HOST always asks for a key").to_string();
    let parsed = ShortJoin::parse(&shown).expect("the lobby's first code is a relay code");
    assert_eq!(code_to_string(&parsed.code), host.relay_code().unwrap());
    assert_eq!(parsed.key.as_deref(), Some(key.as_str()));
    assert!(shown.starts_with(&host.relay_code().unwrap()) && shown.len() > 6, "{shown}");
    // The direct codes are still offered after it, and carry the same key.
    assert!(host.share_codes().iter().skip(1).all(|c| c.contains("#sha256:") && c.ends_with(&key)), "{:?}", host.share_codes());
    // The host is a real QUIC server with an identity, never development UDP.
    assert!(host.fingerprint().is_some_and(|f| f.starts_with("sha256:")));
    // The config JOIN builds from it is QUIC pinned to that identity, claims through the relay, and carries the key.
    let cfg = lobby.join_config(&shown).unwrap();
    assert!(matches!(cfg.transport, ClientTransportConfig::Quic { relay_claim: Some(_), .. }));
    assert_eq!(cfg.server, lobby.relay.addr, "a relayed client dials the relay, not the host");
    assert_eq!(cfg.join_key.as_deref(), Some(key.as_str()));
}

#[test]
fn host_then_short_code_join_works_for_the_host_and_several_friends() {
    let lobby = lobby();
    let code = lobby.advertised_code();

    // The host's own player, exactly as `start_hosted` connects it.
    let host = lobby.host();
    let mut own = ClientConfig::new(host.addr(), 0, lobby.map_hash, 0);
    own.transport = host.local_transport().unwrap();
    own.join_key = host.key().map(str::to_string);
    let mut own = NetClient::connect_with(own).unwrap();
    assert!(joined(&mut own), "the host's own player joins");

    // Three friends type the code: as shown, in lower case with spaces, and with no grouping.
    let typed = [code.clone(), code.to_ascii_lowercase().replace('-', " "), code.replace('-', "")];
    let mut friends: Vec<NetClient> = typed.iter().map(|t| lobby.join(t)).collect();
    for (i, f) in friends.iter_mut().enumerate() {
        assert!(joined(f), "friend {i} typed {:?}: {:?}", typed[i], f.transport_error());
        assert_eq!(f.security(), Security::Quic, "joined over QUIC, never plaintext");
    }
    let mut ids: Vec<u8> = friends.iter().map(|f| f.my_id().unwrap()).chain(own.my_id()).collect();
    for f in friends.iter_mut() {
        assert!(poll_until(f, 5.0, |c| c.stats().snapshots > 5), "the match is streaming to them");
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 4, "four different players: {ids:?}");
    assert_eq!(lobby.relay.server.paired_count(), 3, "the relay carries the three friends; the host's own player is local");
    for f in friends.iter_mut().chain(std::iter::once(&mut own)) {
        f.disconnect();
    }
}

#[test]
fn a_host_that_asks_for_no_key_is_joined_by_the_bare_code() {
    let lobby = lobby_with(relay_options(), Key::None);
    let shown = lobby.advertised_code();
    assert_eq!(shown, lobby.host().relay_code().unwrap(), "no key, so nothing after the six characters");
    let mut friend = lobby.join(&shown);
    assert!(joined(&mut friend));
    friend.disconnect();
}

#[test]
fn the_wrong_missing_or_misread_admission_key_is_refused_clearly() {
    let lobby = lobby();
    let code = lobby.host().relay_code().unwrap().to_string();
    let real = lobby.host().key().unwrap().to_string();

    // Right rendezvous code, wrong key.
    let mut wrong = lobby.join(&format!("{code}-ABCD-EFGH-JKMN"));
    assert_eq!(rejected_with(&mut wrong), Some(RejectReason::BadKey));
    assert!(RejectReason::BadKey.explain().contains("join key"));

    // The six characters alone: anyone who guessed or overheard the code, but never had the key.
    let mut bare = lobby.join(&code);
    assert_eq!(rejected_with(&mut bare), Some(RejectReason::NeedsKey));
    assert!(RejectReason::NeedsKey.explain().contains("whole code"), "{}", RejectReason::NeedsKey.explain());

    // The key with one character changed.
    let mut off_by_one = real.clone();
    off_by_one.replace_range(0..1, if real.starts_with('A') { "B" } else { "A" });
    let mut near = lobby.join(&format!("{code}-{off_by_one}"));
    assert_eq!(rejected_with(&mut near), Some(RejectReason::BadKey));

    // None of that cost a real player their place, and the right code still works afterwards.
    let mut right = lobby.join(&lobby.advertised_code());
    assert!(joined(&mut right));
    right.disconnect();
}

#[test]
fn a_wrong_mistyped_or_unregistered_code_fails_before_any_connection() {
    let lobby = lobby();
    let real = lobby.host().relay_code().unwrap().to_string();
    // Same shape as a code, but nobody hosts it.
    let other = if real.starts_with('A') { "BCDEFG" } else { "ACDEFG" };
    assert!(matches!(lobby.join_config(other).unwrap_err(), JoinError::Relay(ResolveError::CodeNotLive)));
    assert!(lobby.join_config(other).unwrap_err().to_string().contains("not live"));
    // Not a code at all: said before the network is touched.
    assert!(matches!(lobby.join_config("not a code").unwrap_err(), JoinError::BadCode(_)));
    assert!(matches!(lobby.join_config("").unwrap_err(), JoinError::BadCode(_)));
    // Relays switched off on this machine, with a relay code typed.
    assert_eq!(join_config(&lobby.advertised_code(), None, lobby.map_hash).unwrap_err(), JoinError::RelaysOff);
}

#[test]
fn a_code_stops_working_when_its_lease_runs_out() {
    // A relay that forgets a host after 400 ms without news. The host's keepalive is a minute; this host never gets to send one.
    let lobby = lobby_with(RelayServerOptions { registration_lease: Duration::from_millis(400), ..relay_options() }, Key::Generated);
    let code = lobby.advertised_code();
    assert!(lobby.join_config(&code).is_ok(), "live while fresh");
    std::thread::sleep(Duration::from_millis(1000));
    assert!(matches!(lobby.join_config(&code).unwrap_err(), JoinError::Relay(ResolveError::CodeNotLive)), "expired");
}

#[test]
fn direct_joining_with_the_long_code_still_works_and_still_needs_the_key() {
    let lobby = lobby();
    let key = lobby.host().key().unwrap().to_string();
    // The long form: HOST:PORT#fingerprint#key.
    let mut direct = NetClient::connect_with({
        let mut cfg = join_config(&lobby.direct_code(Some(&key)), None, lobby.map_hash).unwrap();
        cfg.name = "Direct".into();
        cfg
    })
    .unwrap();
    assert!(joined(&mut direct), "{:?}", direct.transport_error());
    assert_eq!(direct.security(), Security::Quic);
    // Without the key, or with another, the same address and the right identity still get nowhere.
    let mut no_key = NetClient::connect_with(join_config(&lobby.direct_code(None), None, lobby.map_hash).unwrap()).unwrap();
    assert_eq!(rejected_with(&mut no_key), Some(RejectReason::NeedsKey));
    let mut bad_key = NetClient::connect_with(join_config(&lobby.direct_code(Some("NOTTHEKEY")), None, lobby.map_hash).unwrap()).unwrap();
    assert_eq!(rejected_with(&mut bad_key), Some(RejectReason::BadKey));
    // And the address without the identity is not even tried: nothing would be encrypted or checked.
    let bare = format!("{}", lobby.host().addr());
    let on_loopback = join_config(&bare, None, lobby.map_hash).unwrap();
    assert!(matches!(on_loopback.transport, ClientTransportConfig::DevUdp), "loopback is this machine: development UDP, as before");
    assert_eq!(JoinTarget::parse("203.0.113.7:27015").unwrap().client_config(None, Duration::from_millis(50)).unwrap_err(), JoinError::NoIdentity);
    direct.disconnect();
}

#[test]
fn a_player_who_leaves_and_comes_back_with_their_token_gets_their_place() {
    let lobby = lobby();
    let code = lobby.advertised_code();
    let mut first = lobby.join(&code);
    assert!(joined(&mut first));
    let (id, token) = (first.my_id().unwrap(), first.token());
    assert_ne!(token, 0);
    first.disconnect();
    assert_eq!(first.state(), ConnState::Closed, "leaving is explicit: this client does not come back by itself");

    // Back through the relay: a fresh resolve of the same live code, the old token. Reconnecting is a new join that asks to be resumed.
    let mut cfg = lobby.join_config(&code).unwrap();
    cfg.resume_token = token;
    let mut again = NetClient::connect_with(cfg).unwrap();
    assert!(joined(&mut again));
    assert_eq!((again.my_id().unwrap(), again.token()), (id, token), "same player, same token");
    again.disconnect();
}

#[test]
fn a_network_outage_longer_than_the_hosts_patience_but_inside_the_relays_is_ridden_out() {
    // The host gives up on a silent player after 3 s and parks their place for resuming; the relay keeps the pairing for its (here 30 s,
    // by default ten minute) patience, counted from the last traffic. A friend whose network comes back inside it reconnects by themselves
    // over the same relay pairing, with no new code and no new JOIN, and is the same player.
    let lobby = lobby_with(RelayServerOptions { pair_idle_timeout: Duration::from_secs(30), ..relay_options() }, Key::Generated);
    let proxy = LossyProxy::start(lobby.relay.addr, profile("lan").unwrap(), 11).unwrap();
    let mut cfg = lobby.join_config(&lobby.advertised_code()).unwrap();
    cfg.server = proxy.addr; // the friend's network sits between them and the relay
    let mut friend = NetClient::connect_with(cfg).unwrap();
    assert!(joined(&mut friend));
    let (id, token) = (friend.my_id().unwrap(), friend.token());
    proxy.set_blackhole(true);
    assert!(poll_until(&mut friend, 8.0, |c| c.state() == ConnState::Reconnecting), "the client noticed the outage");
    poll_until(&mut friend, 5.0, |_| false); // well past the host's 3 s: their place is parked, not held
    proxy.set_blackhole(false);
    assert!(poll_until(&mut friend, 20.0, |c| c.state() == ConnState::Connected && c.stats().connects == 2), "and came back by itself");
    assert_eq!((friend.my_id().unwrap(), friend.token()), (id, token), "as the same player");
    friend.disconnect();
    proxy.finish();
}

#[test]
fn the_host_leaving_ends_the_session_for_everyone_and_retires_the_code() {
    let mut lobby = lobby();
    let code = lobby.advertised_code();
    let mut a = lobby.join(&code);
    let mut b = lobby.join(&code);
    assert!(joined(&mut a) && joined(&mut b));
    // Someone leaving on their own changes nothing for the others.
    a.disconnect();
    assert_eq!(a.state(), ConnState::Closed);
    assert!(poll_until(&mut b, 2.0, |c| c.stats().snapshots > 20) && b.state() == ConnState::Connected);

    lobby.drop_host();
    let t0 = Instant::now();
    assert!(poll_until(&mut b, 5.0, |c| c.state() == ConnState::Reconnecting), "the remaining player is told the game ended");
    assert!(t0.elapsed() < Duration::from_secs(4), "told by the server, not by waiting out a timeout: {:?}", t0.elapsed());
    // The code is retired at once: a latecomer is told so, instead of being left waiting on a connection to nobody.
    let end = Instant::now() + Duration::from_secs(5);
    let late = loop {
        match lobby.join_config(&code) {
            Err(e) => break e,
            Ok(_) => assert!(Instant::now() < end, "the relay kept resolving a host that had left"),
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(matches!(late, JoinError::Relay(ResolveError::CodeNotLive)), "{late:?}");
    b.disconnect();
}

#[test]
fn transport_failures_are_named_and_never_downgrade() {
    let lobby = lobby();
    let code = lobby.advertised_code();

    // No relay answering at all.
    let nobody = UdpSocketGuard::bind();
    let err = join_config_within(&code, Some(&nobody.addr.to_string()), lobby.map_hash, Duration::from_millis(600)).unwrap_err();
    assert!(matches!(err, JoinError::Relay(ResolveError::NoAnswer(_))), "{err:?}");
    assert!(err.to_string().contains("did not answer"), "{err}");

    // A relay that vouches for no identity: a host registered without one. The client refuses before dialing anything.
    let stop = Arc::new(AtomicBool::new(false));
    let (_bridge, plain_code) = HostBridge::start(&lobby.relay.address(), "127.0.0.1:9".parse().unwrap(), None, stop.clone()).unwrap();
    let err = lobby.join_config(&code_to_string(&plain_code)).unwrap_err();
    assert_eq!(err, JoinError::HostNotVerifiable);

    // An identity that is not the host's (a relay swapping in someone else): the client fails closed and says so.
    let mut cfg = lobby.join_config(&code).unwrap();
    let impostor = ServerIdentity::generate(&["localhost".into()]).unwrap().identity;
    if let ClientTransportConfig::Quic { trust, .. } = &mut cfg.transport {
        *trust = ServerTrust::fingerprint(&impostor.fingerprint()).unwrap();
    }
    let mut client = NetClient::connect_with(cfg).unwrap();
    assert_eq!(rejected_with(&mut client), Some(RejectReason::ServerIdentity));
    assert_eq!(client.security(), Security::Quic, "still QUIC: nothing fell back to plaintext");
    stop.store(true, Ordering::Relaxed);
}

#[test]
fn a_relay_that_dies_mid_game_is_a_reconnect_not_a_crash_and_the_host_stays_hosted() {
    let mut lobby = lobby();
    let mut friend = lobby.join(&lobby.advertised_code());
    assert!(joined(&mut friend));
    lobby.relay.stop();
    assert!(poll_until(&mut friend, 12.0, |c| c.state() == ConnState::Reconnecting), "the client noticed: {:?}", friend.state());
    assert_ne!(friend.state(), ConnState::Connected);
    // The host itself is untouched: its own player and a direct join still work.
    let key = lobby.host().key().unwrap().to_string();
    let mut direct = NetClient::connect_with(join_config(&lobby.direct_code(Some(&key)), None, lobby.map_hash).unwrap()).unwrap();
    assert!(joined(&mut direct), "friends who can reach the host directly are unaffected by the relay");
    direct.disconnect();
    friend.disconnect();
}

// ---- the join that does not give up -----------------------------------------------------------------------------------------------

/// The host's arena (`map.json` in the lobby's folder) and two copies of it that differ by a byte: a different hash each, so a host playing one
/// refuses a client that loaded another with `WrongMap`.
fn maps_in(lobby: &Lobby) -> (MapChoice, MapChoice, MapChoice) {
    let right = MapChoice { label: "RIGHT".into(), path: lobby.dir.join("map.json") };
    let write = |name: &str, scene: String| {
        let path = lobby.dir.join(name);
        std::fs::write(&path, scene).unwrap();
        MapChoice { label: name.to_uppercase(), path }
    };
    (right, write("wrong_a.json", SCENE.replace("[20,10]", "[21,10]")), write("wrong_b.json", SCENE.replace("[20,10]", "[22,10]")))
}

fn plan(target: &str, relay: Option<&str>, maps: Vec<MapChoice>) -> JoinPlan {
    JoinPlan::new(JoinTarget::parse(target).unwrap(), relay.map(str::to_string), "Friend", maps)
}

/// What the window does with a join: asks the task once a frame until it has an answer.
fn run_join(mut task: JoinTask, secs: u64) -> Result<Joined, JoinFailure> {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(r) = task.poll() {
            return r;
        }
        assert!(Instant::now() < end, "the join did not finish in {secs} s (it said: {})", task.status());
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn start(plan: JoinPlan) -> JoinTask {
    JoinTask::start(plan, "release 4, protocol 17")
}

#[test]
fn a_relayed_join_where_the_first_maps_tried_are_wrong_still_gets_in_with_a_fresh_claim_for_each() {
    // The host plays RIGHT. The joiner has selected another map, then another: the first two attempts are refused with WrongMap. Before, the claim of
    // the first attempt was reused for the second (the relay accepts a claim once, from one socket), so the second never paired and the whole join ended
    // "no answer from the server within 6 s". Now every attempt asks the relay again.
    let lobby = lobby();
    let (right, wrong_a, wrong_b) = maps_in(&lobby);
    let code = lobby.advertised_code();
    let joined = run_join(start(plan(&code, Some(&lobby.relay.address()), vec![wrong_a, wrong_b, right.clone()])), 30).expect("joined on the third map");
    assert_eq!((joined.map_index, joined.map.as_path()), (2, right.path.as_path()));
    assert_eq!(joined.session.client.state(), ConnState::Connected);
    assert_eq!(joined.session.client.security(), Security::Quic);
    assert_eq!(lobby.relay.server.paired_count(), 3, "three attempts, three claims, three pairings: none of them reused");
}

#[test]
fn a_direct_join_tries_the_next_map_too_and_a_host_that_matches_none_is_told_so() {
    let lobby = lobby();
    let (right, wrong_a, wrong_b) = maps_in(&lobby);
    let key = lobby.host().key().unwrap().to_string();
    let direct = lobby.direct_code(Some(&key));
    let joined = run_join(start(plan(&direct, None, vec![wrong_a.clone(), right])), 30).expect("joined on the second map");
    assert_eq!(joined.map_index, 1);
    drop(joined);
    let failure = run_join(start(plan(&direct, None, vec![wrong_a, wrong_b])), 30).err().expect("no map matches");
    assert_eq!(failure.cause, JoinCause::Refused(RejectReason::WrongMap));
    assert!(failure.text.contains("map") && failure.text.contains("same release"), "{}", failure.text);
}

#[test]
fn a_host_that_refuses_is_worded_for_what_it_refused_and_is_not_asked_again() {
    let lobby = lobby();
    let (right, wrong_a, _) = maps_in(&lobby);
    let code = lobby.host().relay_code().unwrap().to_string();
    let relay = lobby.relay.address();
    let started = Instant::now();
    // The right rendezvous code and a wrong key: refused on the first map, and the other maps are not tried (it was never about the map).
    let wrong_key = run_join(start(plan(&format!("{code}-ABCD-EFGH-JKMN"), Some(&relay), vec![right.clone(), wrong_a.clone()])), 30).err().unwrap();
    assert_eq!(wrong_key.cause, JoinCause::Refused(RejectReason::BadKey));
    assert_eq!(wrong_key.text, RejectReason::BadKey.explain());
    let no_key = run_join(start(plan(&code, Some(&relay), vec![right, wrong_a])), 30).err().unwrap();
    assert_eq!(no_key.cause, JoinCause::Refused(RejectReason::NeedsKey));
    assert!(no_key.text.contains("whole code"), "{}", no_key.text);
    assert!(started.elapsed() < Duration::from_secs(15), "refusals are answers, not waits: {:?}", started.elapsed());
}

#[test]
fn every_way_a_join_fails_before_the_host_answers_has_its_own_cause_and_words() {
    let lobby = lobby();
    let (right, ..) = maps_in(&lobby);
    let maps = || vec![right.clone()];
    let code = "H3PQXR-K7Q2-MZ4P-WTXA";
    let fast = |mut p: JoinPlan| {
        p.resolve_timeout = Duration::from_millis(600);
        p.connect_timeout = Duration::from_millis(500);
        p
    };
    let fail = |p: JoinPlan| run_join(start(fast(p)), 30).err().expect("a failure");

    // The relay's name does not resolve.
    let f = fail(plan(code, Some("relay.invalid:28016"), maps()));
    assert_eq!(f.cause, JoinCause::NameLookup);
    assert!(f.text.contains("relay.invalid") && f.text.contains("online"), "{}", f.text);
    // The relay is switched off (the machine answers that nothing listens).
    let closed = {
        let s = std::net::UdpSocket::bind((IpAddr::from(LOOPBACK), 0)).unwrap();
        s.local_addr().unwrap()
    };
    let f = fail(plan(code, Some(&closed.to_string()), maps()));
    assert_eq!(f.cause, JoinCause::RelayDown);
    assert!(f.text.contains("switched off") && f.text.contains("long join code"), "{}", f.text);
    // The relay does not answer at all.
    let silent = UdpSocketGuard::bind();
    let f = fail(plan(code, Some(&silent.addr.to_string()), maps()));
    assert_eq!(f.cause, JoinCause::RelayNoAnswer);
    assert!(f.text.contains("did not answer") && f.text.contains("blocks UDP"), "{}", f.text);
    // The relay is fine and nobody holds the code.
    let f = fail(plan(code, Some(&lobby.relay.address()), maps()));
    assert_eq!(f.cause, JoinCause::CodeNotLive);
    assert!(f.text.contains("not live") && f.text.contains("lobby"), "{}", f.text);
    // The host answers nothing (a direct code to a socket that never speaks): asked twice, then said.
    let quiet = UdpSocketGuard::bind();
    let identity = format!("sha256:{}", "ab".repeat(32));
    let f = fail(plan(&format!("{}#{identity}#K7Q2MZ4P", quiet.addr), None, maps()));
    assert_eq!(f.cause, JoinCause::HostNoAnswer);
    assert!(f.text.contains(&quiet.addr.to_string()) && f.text.contains("asked twice") && f.text.contains("firewall"), "{}", f.text);
    // Not a code, relays off, and a long code with no identity are said before anything is sent.
    assert_eq!(fail(plan("203.0.113.9:27015", None, maps())).cause, JoinCause::NoIdentity);
    assert_eq!(fail(plan(code, None, maps())).cause, JoinCause::RelaysOff);
}

#[test]
fn joining_never_holds_up_the_window_and_can_be_walked_away_from() {
    // A relay that never answers: the task is started and polled the way a frame does it, and none of that waits for the network.
    let lobby = lobby();
    let (right, ..) = maps_in(&lobby);
    let silent = UdpSocketGuard::bind();
    let code = lobby.advertised_code();
    let started = Instant::now();
    let mut task = start(plan(&code, Some(&silent.addr.to_string()), vec![right]));
    for _ in 0..1000 {
        assert!(task.poll().is_none(), "nothing yet, and asking is free");
    }
    assert!(started.elapsed() < Duration::from_millis(250), "start and a thousand polls took {:?}", started.elapsed());
    let status = wait_for("the screen to be told what is happening", || Some(task.status()).filter(|s| s.contains("relay")));
    assert!(status.contains("Asking the relay"), "{status}");
    task.cancel();
    let end = Instant::now() + Duration::from_secs(10);
    let outcome = loop {
        if let Some(r) = task.poll() {
            break r;
        }
        assert!(Instant::now() < end, "a cancelled join must stop");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(outcome.err().map(|f| f.cause), Some(JoinCause::Cancelled));
}

fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_resolve_that_was_lost_is_asked_again_and_the_join_goes_on() {
    // The link to the relay drops everything for the first half second (Wi-Fi taking a breath): the first question is lost, the second gets through.
    let lobby = lobby();
    let (right, ..) = maps_in(&lobby);
    let proxy = LossyProxy::start(lobby.relay.addr, profile("lan").unwrap(), 5).unwrap();
    proxy.set_blackhole(true);
    let task = start(plan(&lobby.advertised_code(), Some(&proxy.addr.to_string()), vec![right]));
    std::thread::sleep(Duration::from_millis(450));
    proxy.set_blackhole(false);
    let joined = run_join(task, 30).expect("joined after the first question was lost");
    assert_eq!(joined.session.client.state(), ConnState::Connected);
    let report = proxy.finish();
    assert!(report.dropped[0] >= 1, "something was lost on the way to the relay: {report:?}");
}

#[test]
fn a_claim_that_was_lost_is_repeated_and_the_handshake_completes() {
    // The claim and the first handshake packets go into a dead link; it comes back before the claim's repeats stop. Without repeats the relay never
    // pairs this client and the handshake can only time out.
    let lobby = lobby();
    let proxy = LossyProxy::start(lobby.relay.addr, profile("lan").unwrap(), 6).unwrap();
    let mut cfg = lobby.join_config(&lobby.advertised_code()).unwrap();
    cfg.server = proxy.addr;
    proxy.set_blackhole(true);
    let mut friend = NetClient::connect_with(cfg).unwrap();
    std::thread::sleep(Duration::from_millis(450));
    proxy.set_blackhole(false);
    assert!(joined(&mut friend), "{:?}", friend.transport_error());
    assert_eq!(lobby.relay.server.paired_count(), 1);
    assert!(poll_until(&mut friend, 5.0, |c| c.stats().snapshots > 5), "the match streams to them");
    friend.disconnect();
    let report = proxy.finish();
    assert!(report.dropped[0] >= 2, "the claim was sent more than once into the dead link: {report:?}");
}

#[test]
fn claims_repeated_after_the_pairing_do_no_harm_on_a_slow_link() {
    // 150 ms each way: the handshake takes several round trips, so the claim's repeats (at 250 ms, 600 ms, ...) reach the relay *after* it paired this
    // client. They are forwarded to the host as nine meaningless bytes, which its game server drops; the handshake and the match carry on.
    let lobby = lobby();
    let slow = LinkProfile { name: "slow", loss: 0.0, burst: 1.0, delay_ms: 150.0, jitter_ms: 0.0, duplicate: 0.0 };
    let proxy = LossyProxy::start(lobby.relay.addr, slow, 7).unwrap();
    let mut cfg = lobby.join_config(&lobby.advertised_code()).unwrap();
    cfg.server = proxy.addr;
    let mut friend = NetClient::connect_with(cfg).unwrap();
    assert!(joined(&mut friend), "{:?}", friend.transport_error());
    assert!(poll_until(&mut friend, 8.0, |c| c.stats().snapshots > 20 && c.state() == ConnState::Connected));
    friend.disconnect();
    let report = proxy.finish();
    assert!(report.received[0] >= 4, "claim copies and handshake packets went through the link: {report:?}");
}

#[test]
fn the_bytes_a_released_client_speaks_still_work_against_the_current_relay() {
    // A client of release 4 (protocol 17) written out by hand, byte for byte, with no help from this repository's message types: Register, Resolve and
    // Claim as they have always been on the wire. The relay on the always-on machine is this code's cousin, so this is what keeps both working.
    let relay = TestRelay::start(relay_options());
    let host = std::net::UdpSocket::bind((IpAddr::from(LOOPBACK), 0)).unwrap();
    let client = std::net::UdpSocket::bind((IpAddr::from(LOOPBACK), 0)).unwrap();
    for s in [&host, &client] {
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    }
    let mut buf = [0u8; 256];

    let fingerprint = format!("sha256:{}", "cd".repeat(32));
    let register = [&[1u8, 71][..], fingerprint.as_bytes()].concat();
    host.send_to(&register, relay.addr).unwrap();
    let (n, _) = host.recv_from(&mut buf).unwrap();
    assert_eq!((n, buf[0]), (7, 2), "Registered: tag 2 and the six characters");
    let code = buf[1..7].to_vec();
    assert!(code.iter().all(u8::is_ascii_alphanumeric));

    client.send_to(&[&[3u8][..], &code].concat(), relay.addr).unwrap();
    let (n, _) = client.recv_from(&mut buf).unwrap();
    assert_eq!((n, buf[0], buf[1]), (81, 4, 71), "Resolved: tag 4, the 71-byte identity, then an 8-byte claim token");
    assert_eq!(&buf[2..73], fingerprint.as_bytes());
    let token = buf[73..81].to_vec();

    // The claim, twice (a client of this release repeats it): the relay pairs once.
    let claim = [&[6u8][..], &token].concat();
    client.send_to(&claim, relay.addr).unwrap();
    client.send_to(&claim, relay.addr).unwrap();
    wait_for("the pairing", || (relay.server.paired_count() == 1).then_some(()));
    client.send_to(b"hello", relay.addr).unwrap();
    // The host gets id-framed datagrams (four bytes of pairing id, then what the client sent): the repeated claim, then the greeting.
    let mut payloads = Vec::new();
    loop {
        let (n, from) = host.recv_from(&mut buf).unwrap();
        assert_eq!(from, relay.addr);
        payloads.push(buf[4..n].to_vec());
        if &buf[4..n] == b"hello" {
            break;
        }
    }
    assert_eq!(payloads, vec![claim, b"hello".to_vec()]);
    assert_eq!(relay.server.paired_count(), 1);

    // A code nobody holds: tag 5.
    let other = std::net::UdpSocket::bind((IpAddr::from(LOOPBACK), 0)).unwrap();
    other.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    other.send_to(&[&[3u8][..], b"AAAAAA"].concat(), relay.addr).unwrap();
    let (n, _) = other.recv_from(&mut buf).unwrap();
    assert_eq!(&buf[..n], &[5], "CodeNotFound");
}

/// A bound UDP socket that never answers: stands in for a relay that is down.
struct UdpSocketGuard {
    addr: SocketAddr,
    _socket: std::net::UdpSocket,
}

impl UdpSocketGuard {
    fn bind() -> UdpSocketGuard {
        let socket = std::net::UdpSocket::bind((IpAddr::from(LOOPBACK), 0)).unwrap();
        UdpSocketGuard { addr: socket.local_addr().unwrap(), _socket: socket }
    }
}
