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

use red_engine2::net::client::{ClientConfig, ClientTransportConfig, ConnState, NetClient};
use red_engine2::net::host::{HostOptions, LocalHost, PublicOptions};
use red_engine2::net::join::{JoinError, JoinTarget};
use red_engine2::net::netsim::{profile, LossyProxy};
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
    assert!(shown.starts_with(host.relay_code().unwrap()) && shown.len() > 6, "{shown}");
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
