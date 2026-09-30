//! Hosting a match from inside another program (ADR 0054): how `re2 --host MAP` plays against bots on your own machine with no second process,
//! no console window to close and nothing left running afterwards. It is the core of `red_server` for one map: the same [`Server`], the match flow
//! and the bots that the map's own `match` and `bots` blocks ask for, listening on the loopback by default, on a thread that stops when the
//! [`LocalHost`] is dropped.

use super::join_code::JoinCode;
use super::map_hash;
use super::quic::{QuicServer, QuicServerOptions, ServerIdentity};
use super::server::{raise_timer_resolution, Server, ServerConfig};
use crate::sim::flow::MatchSettings;
use crate::sim::interest::InterestMap;
use crate::sim::match_sim::MatchSim;
use crate::sim::spawns::parse_spawns;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// What to host.
#[derive(Debug, Clone)]
pub struct HostOptions {
    /// The address to listen on: the loopback unless other machines should be able to join.
    pub bind: IpAddr,
    /// The UDP port (`0` picks a free one).
    pub port: u16,
    /// Players (humans included) the match aims for; empty slots are filled with bots. `None` = the map's `bots.fill`, `Some(0)` = no bots.
    pub fill: Option<usize>,
    /// The bots' level, overriding the map's: `rookie`, `easy`, `normal`, `hard`, `nightmare`, or a number from 0 to 1.
    pub bot_skill: Option<String>,
    /// Only use spawn points of this group (`""` = all).
    pub spawn_group: String,
    /// Overrides the map's `match.score_to_win`: team or player kills that end a round (`Some(0)` = no kill limit).
    pub kill_limit: Option<u32>,
    /// Overrides the map's `match.round_secs`: how long a round lasts (`Some(0.0)` = no time limit).
    pub round_secs: Option<f32>,
    /// Host for other machines: QUIC + TLS 1.3 with a saved identity, optionally opening the router's port. `None` = this machine only.
    pub public: Option<PublicOptions>,
}

/// How a hosted game reaches other people.
#[derive(Debug, Clone)]
pub struct PublicOptions {
    /// Where the host's identity (`cert.pem`, `key.pem`) is kept; made on first use so the fingerprint friends pin stays the same.
    pub identity_dir: PathBuf,
    /// The UDP port to try first (the next few are tried if it is busy).
    pub port: u16,
    /// Ask the home router to open the port (UPnP).
    pub upnp: bool,
    /// A join key friends must also know (`None` = none).
    pub key: Option<String>,
}

impl Default for HostOptions {
    fn default() -> Self {
        HostOptions {
            bind: IpAddr::from([127, 0, 0, 1]),
            port: 0,
            fill: None,
            bot_skill: None,
            spawn_group: String::new(),
            kill_limit: None,
            round_secs: None,
            public: None,
        }
    }
}

/// A server running on a background thread of this process.
pub struct LocalHost {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// The identity fingerprint clients pin, when hosting over QUIC.
    fingerprint: Option<String>,
    /// Addresses friends can try: the router's public one (when UPnP worked) first, then this machine's LAN address.
    join_addresses: Vec<String>,
    /// What UPnP said (a line for the screen), when it was asked.
    upnp_note: Option<String>,
    key: Option<String>,
    keeper: Option<JoinHandle<()>>,
}

impl LocalHost {
    /// Loads `map` and starts serving it. The error says what is wrong with the map or the options.
    pub fn start(map: &Path, opts: &HostOptions) -> Result<LocalHost, String> {
        let text = std::fs::read_to_string(map).map_err(|e| format!("cannot read {}: {e}", map.display()))?;
        let scene = crate::schema::parse_scene(&text).map_err(|e| format!("{} is not a valid scene: {}", map.display(), e.join("; ")))?;
        let mut spawns = parse_spawns(&text)?;
        if !opts.spawn_group.is_empty() {
            spawns.retain(|s| s.group == opts.spawn_group);
            if spawns.is_empty() {
                return Err(format!("no spawn points in group '{}'", opts.spawn_group));
            }
        }
        let sim = MatchSim::try_new(&scene, spawns.clone())?;
        let public = opts.public.as_ref();
        let (bind, port) = match public {
            Some(p) => (IpAddr::from([0, 0, 0, 0]), p.port),
            None => (opts.bind, opts.port),
        };
        let mut cfg = ServerConfig::new(SocketAddr::new(bind, port), map_hash(&text));
        cfg.join_key = public.and_then(|p| p.key.clone()).filter(|k| !k.is_empty());
        cfg.bot_fill = opts.fill;
        cfg.bot_level = opts
            .bot_skill
            .as_deref()
            .map(|s| {
                crate::sim::ai::skill::level_from_name(s)
                    .ok_or_else(|| format!("'{s}' is not a bot level (rookie, easy, normal, hard, nightmare, or a number from 0 to 1)"))
            })
            .transpose()?;
        let mut fingerprint = None;
        let mut server = match public {
            None => Server::bind(cfg, sim).map_err(|e| format!("cannot listen on {}:{}: {e}", opts.bind, opts.port))?,
            Some(p) => {
                let identity = load_or_make_identity(&p.identity_dir)?;
                fingerprint = Some(identity.fingerprint());
                // The first free port from the wanted one: a second copy of the game on this machine must not stop a host.
                let mut bound = None;
                let mut last_error = String::new();
                for offset in 0..16u16 {
                    let candidate = SocketAddr::new(bind, p.port.saturating_add(offset));
                    match QuicServer::bind(candidate, &identity, QuicServerOptions { max_connections: 32 }) {
                        Ok(t) => {
                            bound = Some(t);
                            break;
                        }
                        Err(e) => last_error = format!("{candidate}: {e}"),
                    }
                }
                let transport = bound.ok_or_else(|| format!("cannot listen for friends (is a firewall in the way?): {last_error}"))?;
                Server::with_transport(cfg, sim, Box::new(transport)).map_err(|e| format!("cannot start the server: {e}"))?
            }
        };
        if let Some(interest) = InterestMap::parse(&text)? {
            server.set_interest(Some(interest));
        }
        if let Some(mut settings) = MatchSettings::from_scene_text(&text)? {
            if let Some(k) = opts.kill_limit {
                settings.score_to_win = k;
            }
            if let Some(s) = opts.round_secs {
                settings.round_secs = s;
            }
            // The scene is parsed afresh for every round: a rematch starts from the authored map.
            let (text2, spawns2) = (text.clone(), spawns);
            server.enable_flow(settings, move || {
                let scene = crate::schema::parse_scene(&text2).map_err(|e| e.join("; "))?;
                MatchSim::try_new(&scene, spawns2.clone())
            })?;
        }
        let pause = Arc::new(AtomicBool::new(false));
        server.set_pause_flag(pause.clone());
        let bound = server.local_addr().map_err(|e| format!("cannot read the server address: {e}"))?;
        // A server bound to every interface is still reached from this machine by the loopback.
        let addr = SocketAddr::new(if bind.is_unspecified() { IpAddr::from([127, 0, 0, 1]) } else { bind }, bound.port());
        // Friends: the router's public address when UPnP opens the port, and this machine's LAN address.
        let mut join_addresses = Vec::new();
        let mut upnp_note = None;
        let mut keeper = None;
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(p) = public {
            if p.upnp {
                let options = crate::tools::portmap::Options { port: bound.port(), ..Default::default() };
                match crate::tools::portmap::Keeper::start(&options) {
                    Ok((mut k, _report)) => {
                        if let Some(a) = k.found().join_address(bound.port()) {
                            join_addresses.push(a);
                        }
                        upnp_note = Some(match k.found().reachability_warning() {
                            Some(w) => w,
                            None => "the router opened the port for your friends".to_string(),
                        });
                        let flag = stop.clone();
                        keeper = std::thread::Builder::new()
                            .name("red-upnp".into())
                            .spawn(move || {
                                while !flag.load(Ordering::Relaxed) {
                                    std::thread::sleep(std::time::Duration::from_millis(500));
                                    k.tick();
                                }
                                k.stop();
                            })
                            .ok();
                    }
                    Err(e) => {
                        upnp_note = Some(format!(
                            "the router did not open the port ({e}); forward UDP {} to this PC, or friends on your network can still join",
                            bound.port()
                        ))
                    }
                }
            }
            if let Some(lan) = super::upnp::local_ip_towards(IpAddr::from([8, 8, 8, 8])) {
                join_addresses.push(format!("{lan}:{}", bound.port()));
            }
        }
        raise_timer_resolution();
        let flag = stop.clone();
        let thread =
            std::thread::Builder::new().name("red-host".into()).spawn(move || server.run(&flag)).map_err(|e| format!("cannot start the server thread: {e}"))?;
        Ok(LocalHost { addr, stop, pause, thread: Some(thread), fingerprint, join_addresses, upnp_note, key: public.and_then(|p| p.key.clone()), keeper })
    }

    /// The identity fingerprint clients must pin (`sha256:<hex>`), when this host speaks QUIC.
    pub fn fingerprint(&self) -> Option<&str> {
        self.fingerprint.as_deref()
    }

    /// The join codes to give friends: the public address first (if the router opened the port), then the LAN one.
    pub fn join_codes(&self) -> Vec<JoinCode> {
        self.join_addresses.iter().map(|a| JoinCode { address: a.clone(), fingerprint: self.fingerprint.clone(), key: self.key.clone() }).collect()
    }

    /// What the router said about opening the port, for the screen.
    pub fn upnp_note(&self) -> Option<&str> {
        self.upnp_note.as_deref()
    }

    /// The transport settings a client on this machine joins with (pinned to this host's own identity over QUIC, or plain loopback UDP).
    pub fn local_transport(&self) -> Result<super::client::ClientTransportConfig, String> {
        match &self.fingerprint {
            Some(f) => {
                Ok(super::client::ClientTransportConfig::Quic { trust: super::quic::ServerTrust::fingerprint(f)?, server_name: "localhost".to_string() })
            }
            None => Ok(super::client::ClientTransportConfig::DevUdp),
        }
    }

    /// The join key a client on this machine needs (`None` = the host has none).
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    /// The address a client on this machine joins.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// A switch that freezes the match while it is set: the world and the round clock stop, the connection stays up (a game pauses with it).
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        self.pause.clone()
    }
}

impl Drop for LocalHost {
    /// Stops the server (its clients are told) and waits for its thread.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(k) = self.keeper.take() {
            let _ = k.join();
        }
        if let Some(t) = self.thread.take() {
            // The server stops within a tick or two; if it somehow cannot (a blocked write), give up on it rather than hold the program open.
            for _ in 0..300 {
                if t.is_finished() {
                    let _ = t.join();
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

/// The host's saved identity (`cert.pem`, `key.pem` in `dir`), made on first use. A damaged pair is replaced: friends then need the new fingerprint.
fn load_or_make_identity(dir: &Path) -> Result<ServerIdentity, String> {
    let (cert, key) = (dir.join("cert.pem"), dir.join("key.pem"));
    if cert.exists() && key.exists() {
        if let Ok(id) = ServerIdentity::load(&cert, &key) {
            return Ok(id);
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let made = ServerIdentity::generate(&["localhost".to_string()])?;
    std::fs::write(&cert, &made.cert_pem).map_err(|e| format!("cannot write {}: {e}", cert.display()))?;
    std::fs::write(&key, &made.key_pem).map_err(|e| format!("cannot write {}: {e}", key.display()))?;
    Ok(made.identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::bot::{Behavior, Bot, ClientWorld};
    use crate::net::protocol::ROSTER_BOT;
    use crate::player::Character;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn lab() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")
    }

    #[test]
    fn a_local_host_serves_the_map_fills_it_with_bots_and_stops_when_dropped() {
        let host = LocalHost::start(&lab(), &HostOptions { fill: Some(4), spawn_group: "duel".into(), ..Default::default() }).unwrap();
        assert!(host.addr().ip().is_loopback() && host.addr().port() != 0);
        let (_scene, world) = ClientWorld::load(&lab()).unwrap();
        let mut me = Bot::new(host.addr(), Character::Human, world, Behavior::Idle, 0).unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        let mut bots = 0;
        while Instant::now() < end && bots < 3 {
            me.pump(Instant::now());
            bots = me.client.status().map_or(0, |s| s.roster.iter().filter(|e| e.flags & ROSTER_BOT != 0).count());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(bots, 3, "one human and three bots make four players");
        let started = Instant::now();
        drop(host);
        assert!(started.elapsed() < Duration::from_secs(3), "dropping the host stops its thread promptly: {:?}", started.elapsed());
    }

    #[test]
    fn a_paused_host_keeps_everyone_connected_and_sending_snapshots() {
        let host = LocalHost::start(&lab(), &HostOptions { fill: Some(0), spawn_group: "duel".into(), ..Default::default() }).unwrap();
        let (_s, world) = ClientWorld::load(&lab()).unwrap();
        let mut me = Bot::new(host.addr(), Character::Human, world, Behavior::Idle, 0).unwrap();
        let pump_for = |me: &mut Bot, secs: f64| {
            let end = Instant::now() + Duration::from_secs_f64(secs);
            while Instant::now() < end {
                me.pump(Instant::now());
                std::thread::sleep(Duration::from_millis(3));
            }
        };
        pump_for(&mut me, 1.0);
        assert_eq!(me.client.state(), crate::net::client::ConnState::Connected);
        let before = me.client.stats().snapshots;
        host.pause_flag().store(true, Ordering::Relaxed);
        pump_for(&mut me, 4.0); // longer than the 3 s a silent server would be timed out after
        assert_eq!(me.client.state(), crate::net::client::ConnState::Connected, "a paused match does not drop its players");
        assert!(me.client.stats().snapshots > before + 60, "the server keeps sending snapshots while paused: {} -> {}", before, me.client.stats().snapshots);
        host.pause_flag().store(false, Ordering::Relaxed);
        pump_for(&mut me, 0.5);
        assert_eq!(me.client.state(), crate::net::client::ConnState::Connected);
    }

    #[test]
    fn a_public_host_speaks_quic_with_a_saved_identity_that_a_client_pins() {
        let dir = std::env::temp_dir().join(format!("re2_host_identity_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let public = PublicOptions { identity_dir: dir.clone(), port: 0, upnp: false, key: Some("sesame".into()) };
        let opts = HostOptions {
            fill: Some(0),
            spawn_group: "duel".into(),
            kill_limit: Some(5),
            round_secs: Some(30.0),
            public: Some(public.clone()),
            ..Default::default()
        };
        let host = LocalHost::start(&lab(), &opts).unwrap();
        let fingerprint = host.fingerprint().expect("QUIC hosts have an identity").to_string();
        assert!(fingerprint.starts_with("sha256:") && dir.join("cert.pem").exists() && dir.join("key.pem").exists());
        let (_scene, world) = ClientWorld::load(&lab()).unwrap();
        let mut cfg = crate::net::client::ClientConfig::new(host.addr(), 0, world.map_hash, 0);
        cfg.transport = host.local_transport().unwrap();
        cfg.join_key = host.key().map(str::to_string);
        let client = crate::net::client::NetClient::connect_with(cfg).unwrap();
        let mut me = Bot::with_client(client, Character::Human, world, Behavior::Idle).unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end && me.client.state() != crate::net::client::ConnState::Connected {
            me.pump(Instant::now());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(me.client.state(), crate::net::client::ConnState::Connected, "joined over QUIC with the pinned identity and the key");
        assert!(me.client.security().is_secure());
        drop(host);
        // A second host with the same folder keeps the identity (friends do not have to be told a new fingerprint).
        let again = LocalHost::start(&lab(), &HostOptions { fill: Some(0), spawn_group: "duel".into(), public: Some(public), ..Default::default() }).unwrap();
        assert_eq!(again.fingerprint(), Some(fingerprint.as_str()));
        drop(again);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bad_map_or_option_is_an_error_that_says_what_is_wrong() {
        let missing = LocalHost::start(Path::new("no/such/map.json"), &HostOptions::default()).err().unwrap();
        assert!(missing.contains("no/such/map.json"), "{missing}");
        let skill = LocalHost::start(&lab(), &HostOptions { bot_skill: Some("godlike".into()), ..Default::default() }).err().unwrap();
        assert!(skill.contains("godlike") && skill.contains("nightmare"), "{skill}");
        let group = LocalHost::start(&lab(), &HostOptions { spawn_group: "nowhere".into(), ..Default::default() }).err().unwrap();
        assert!(group.contains("nowhere"), "{group}");
    }
}
