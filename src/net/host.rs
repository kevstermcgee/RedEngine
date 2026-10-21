//! Hosting a match from inside another program (ADR 0054): how `re2 --host MAP` plays against bots on your own machine with no second process,
//! no console window to close and nothing left running afterwards. It is the core of `red_server` for one map: the same [`Server`], the match flow
//! and the bots that the map's own `match` and `bots` blocks ask for, listening on the loopback by default, on a thread that stops when the
//! [`LocalHost`] is dropped.

use super::join_code::JoinCode;
use super::map_hash;
use super::quic::{QuicServer, QuicServerOptions, ServerIdentity};
use super::relay_server::RegisterTiming;
use super::server::{raise_timer_resolution, Server, ServerConfig};
use crate::sim::flow::MatchSettings;
use crate::sim::interest::InterestMap;
use crate::sim::match_sim::MatchSim;
use crate::sim::spawns::parse_spawns;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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
    /// The scene to play when the game changed the map's text (its mode, its team size: Killchain writes them with the shooter's
    /// `with_overrides`); `None` = the map file as it is. The map's hash, interest and match settings still come from the file.
    pub scene_text: Option<String>,
    /// Host for other machines: QUIC + TLS 1.3 with a saved identity, optionally opening the router's port. `None` = this machine only.
    pub public: Option<PublicOptions>,
}

/// How a hosted game reaches other people.
#[derive(Debug, Clone)]
pub struct PublicOptions {
    /// The interface to listen on: `None` = every interface (friends on the same network and a router's forwarded port reach it), `Some(ip)` =
    /// only that one. A host reached only through a relay can use the loopback and open nothing to the network at all (and so does a test).
    pub bind: Option<IpAddr>,
    /// Where the host's identity (`cert.pem`, `key.pem`) is kept; made on first use so the fingerprint friends pin stays the same.
    pub identity_dir: PathBuf,
    /// The UDP port to try first (the next few are tried if it is busy).
    pub port: u16,
    /// Ask the home router to open the port (UPnP).
    pub upnp: bool,
    /// Ask this gateway instead of searching the network for one (`portmap::Options::router`): a test points it at the loopback so asking opens nothing
    /// on any real router. `None` (what HOST uses) searches.
    pub upnp_router: Option<IpAddr>,
    /// A join key friends must also know (`None` = none). Never sent over the network: clients prove they know it inside the handshake.
    pub key: Option<String>,
    /// Register with a `red_relay` (docs/HOSTING.md) at this `HOST:PORT` instead of relying on UPnP/port
    /// forwarding: a friend then joins with a short code rather than an address at all, and this works even
    /// behind carrier-grade NAT (ADR 0031: nothing UPnP does can fix that; a relay sidesteps it instead of trying
    /// to). A hostname (a DuckDNS name, say) works here, re-resolved on every use — not just a literal address.
    pub relay: Option<String>,
}

impl PublicOptions {
    /// The settings the HOST button uses: listen for friends, ask the router to open the port, require a fresh random join key (so a guessed or
    /// leaked relay code alone is never enough to get in), and register with `relay` when there is one. `identity_dir` keeps the host's
    /// identity between runs so the fingerprint friends pin stays the same. `Err` only if the OS random number generator fails: a host
    /// without a key would be open to anyone holding its code, so it does not start instead.
    pub fn for_friends(identity_dir: PathBuf, relay: Option<String>) -> Result<PublicOptions, String> {
        Ok(PublicOptions {
            bind: None,
            identity_dir,
            port: super::DEFAULT_PORT,
            upnp: true,
            upnp_router: None,
            key: Some(super::relay::generate_join_key()?),
            relay,
        })
    }
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
            scene_text: None,
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
    /// This machine's address on the local network (`HOST:PORT`): friends in the same house can join there.
    lan_address: Option<String>,
    /// The router's UPnP answer (the public address when it opened the port, or what went wrong), when it was asked. It is asked on a thread of its
    /// own, because a router that never answers would otherwise keep the window frozen for seconds.
    upnp: Arc<Mutex<UpnpState>>,
    key: Option<String>,
    keeper: Option<JoinHandle<()>>,
    /// Registration with the relay, when `PublicOptions::relay` was set. It runs on its own thread so a relay that
    /// does not answer never freezes the screen; the state is polled through `relay_code` / `relay_note`.
    relay: Arc<Mutex<RelayState>>,
}

/// Where registering with the relay stands.
enum RelayState {
    /// No relay was asked for.
    Off,
    /// The first Register is in flight.
    Pending,
    /// Registered. The bridge must live as long as the host (its forwarding threads hold their own handles, but dropping it early would send the relay a
    /// goodbye); it also knows the code the relay *currently* holds this host under, which a relay that restarted changes. `seen_changes` is how many
    /// changes the lobby has been told about and `changed_at` when the last one happened.
    Ready { bridge: super::relay_server::HostBridge, seen_changes: u32, changed_at: Option<Instant> },
    /// The relay did not answer, with a line for the screen. The host keeps asking in the background ([`RELAY_RETRY_EVERY`]): a relay that is started
    /// five minutes later still gives this game a code.
    Failed(String),
}

/// What the router said to UPnP.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UpnpState {
    /// UPnP was not asked for.
    Off,
    /// The router is being asked.
    Pending,
    /// The router opened the port: its public address (when it says), and a line about it (a warning when that address cannot be reached from outside).
    Opened { public: Option<String>, note: String },
    /// The router would not (or did not answer): the line for the screen.
    Declined(String),
}

/// How long a host whose relay did not answer waits before it asks again.
pub const RELAY_RETRY_EVERY: Duration = Duration::from_secs(20);
/// How long the lobby keeps saying that the relay's code changed.
const CODE_CHANGE_NOTICE: Duration = Duration::from_secs(90);

/// How a code in [`LocalHost::share_entries`] reaches a friend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeKind {
    /// The relay's short code (with the join key): works from anywhere the relay can be reached.
    Online,
    /// The public address of the router when UPnP opened the port: works from anywhere, but only while that port stays open.
    Direct,
    /// This machine's address on the local network: only for friends in the same house.
    SameNetwork,
}

/// One code to give friends, with what to say about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareCode {
    /// How it reaches a friend.
    pub kind: CodeKind,
    /// A few words for the screen.
    pub label: &'static str,
    /// The code itself, exactly as a friend must type or paste it: its case is part of it (a fingerprint and a key are), so a screen must never change it.
    pub code: String,
}

impl LocalHost {
    /// Loads `map` and starts serving it. The error says what is wrong with the map or the options.
    pub fn start(map: &Path, opts: &HostOptions) -> Result<LocalHost, String> {
        let text = std::fs::read_to_string(map).map_err(|e| format!("cannot read {}: {e}", map.display()))?;
        // The game's choice of mode and team size is in the scene it passes; the hash and the interest map still come from the file.
        let scene_text = opts.scene_text.clone().unwrap_or_else(|| text.clone());
        let scene = crate::scene_blocks::parse_scene(&scene_text).map_err(|e| format!("{} is not a valid scene: {}", map.display(), e.join("; ")))?;
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
            Some(p) => (p.bind.unwrap_or(IpAddr::from([0, 0, 0, 0])), p.port),
            None => (opts.bind, opts.port),
        };
        let mut cfg = ServerConfig::new(SocketAddr::new(bind, port), map_hash(&text));
        let cfg_key = public.and_then(|p| p.key.clone()).filter(|k| !k.is_empty());
        cfg.join_key = cfg_key.clone();
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
            let (text2, spawns2) = (scene_text.clone(), spawns);
            server.enable_flow(settings, move || {
                let scene = crate::scene_blocks::parse_scene(&text2).map_err(|e| e.join("; "))?;
                MatchSim::try_new(&scene, spawns2.clone())
            })?;
        }
        let pause = Arc::new(AtomicBool::new(false));
        server.set_pause_flag(pause.clone());
        let bound = server.local_addr().map_err(|e| format!("cannot read the server address: {e}"))?;
        // A server bound to every interface is still reached from this machine by the loopback.
        let addr = SocketAddr::new(if bind.is_unspecified() { IpAddr::from([127, 0, 0, 1]) } else { bind }, bound.port());
        // Friends: the router's public address when UPnP opens the port (asked on its own thread), and this machine's LAN address.
        let mut lan_address = None;
        let mut keeper = None;
        let upnp = Arc::new(Mutex::new(UpnpState::Off));
        let stop = Arc::new(AtomicBool::new(false));
        if let Some(p) = public {
            if p.upnp {
                *upnp.lock().unwrap_or_else(|e| e.into_inner()) = UpnpState::Pending;
                let options = crate::net::portmap::Options { port: bound.port(), router: p.upnp_router, ..Default::default() };
                keeper = spawn_upnp(options, stop.clone(), upnp.clone());
                if keeper.is_none() {
                    *upnp.lock().unwrap_or_else(|e| e.into_inner()) = UpnpState::Declined("could not ask the router (no thread to do it on)".to_string());
                }
            }
            if let Some(lan) = super::upnp::local_ip_towards(IpAddr::from([8, 8, 8, 8])) {
                lan_address = Some(format!("{lan}:{}", bound.port()));
            }
        }
        let relay_state = Arc::new(Mutex::new(RelayState::Off));
        if let Some(relay) = public.and_then(|p| p.relay.clone()) {
            *relay_state.lock().unwrap_or_else(|p| p.into_inner()) = RelayState::Pending;
            let (state, fp, stop) = (relay_state.clone(), fingerprint.clone(), stop.clone());
            let spawned = std::thread::Builder::new()
                .name("red-relay-register".into())
                .spawn(move || register_with_relay(&relay, addr, fp, &stop, &state, RegisterTiming::default(), RELAY_RETRY_EVERY));
            if spawned.is_err() {
                *relay_state.lock().unwrap_or_else(|p| p.into_inner()) = RelayState::Failed("NO ONLINE CODE: COULD NOT START THE RELAY LINK".to_string());
            }
        }
        raise_timer_resolution();
        let flag = stop.clone();
        let thread =
            std::thread::Builder::new().name("red-host".into()).spawn(move || server.run(&flag)).map_err(|e| format!("cannot start the server thread: {e}"))?;
        Ok(LocalHost { addr, stop, pause, thread: Some(thread), fingerprint, lan_address, upnp, key: cfg_key, keeper, relay: relay_state })
    }

    /// The identity fingerprint clients must pin (`sha256:<hex>`), when this host speaks QUIC.
    pub fn fingerprint(&self) -> Option<&str> {
        self.fingerprint.as_deref()
    }

    /// The direct join codes to give friends: the public address first (if the router opened the port), then the LAN one.
    pub fn join_codes(&self) -> Vec<JoinCode> {
        self.direct_addresses().into_iter().map(|(_, a)| JoinCode { address: a, fingerprint: self.fingerprint.clone(), key: self.key.clone() }).collect()
    }

    /// The addresses a direct code can name, best first, each with how far it reaches.
    fn direct_addresses(&self) -> Vec<(CodeKind, String)> {
        let mut out = Vec::new();
        if let UpnpState::Opened { public: Some(a), .. } = &*self.upnp.lock().unwrap_or_else(|e| e.into_inner()) {
            out.push((CodeKind::Direct, a.clone()));
        }
        out.extend(self.lan_address.iter().map(|a| (CodeKind::SameNetwork, a.clone())));
        out
    }

    /// What the router said about opening the port, for the screen (`None` while it is still being asked, or when it was not).
    pub fn upnp_note(&self) -> Option<String> {
        match &*self.upnp.lock().unwrap_or_else(|e| e.into_inner()) {
            UpnpState::Opened { note, .. } => Some(note.clone()),
            UpnpState::Declined(note) => Some(note.clone()),
            UpnpState::Off | UpnpState::Pending => None,
        }
    }

    /// True while the router is being asked to open the port.
    pub fn upnp_pending(&self) -> bool {
        matches!(&*self.upnp.lock().unwrap_or_else(|e| e.into_inner()), UpnpState::Pending)
    }

    /// The six-character code the relay resolves (`PublicOptions::relay`, registered and accepted). This is the public half: friends are
    /// given [`share_codes`](Self::share_codes), which carry the admission key as well. `None` while registration is still in flight or after it failed.
    /// It follows the relay: a relay that restarted has forgotten this host and hands out a new code at the next keepalive, which shows here.
    pub fn relay_code(&self) -> Option<String> {
        match &*self.relay.lock().unwrap_or_else(|p| p.into_inner()) {
            RelayState::Ready { bridge, .. } => Some(super::relay::code_to_string(&bridge.code())),
            _ => None,
        }
    }

    /// How many times the relay has changed this host's code since it registered (`0` = never). A screen showing the code compares it with what it showed.
    pub fn relay_code_changes(&self) -> u32 {
        match &*self.relay.lock().unwrap_or_else(|p| p.into_inner()) {
            RelayState::Ready { bridge, .. } => bridge.code_changes(),
            _ => 0,
        }
    }

    /// Everything to show the host to give to friends, best first: the relay code **with the admission key** (`H3PQXR-K7Q2-MZ4P-WTXA`), when
    /// registering with a relay worked, then the direct codes (`HOST:PORT#sha256:...#key`) for friends on the same network or behind a
    /// forwarded port. Each one on its own is enough to join; none is joinable without the key this host requires.
    pub fn share_codes(&self) -> Vec<String> {
        self.share_entries().into_iter().map(|e| e.code).collect()
    }

    /// [`share_codes`](Self::share_codes) with a label each, so a screen can say what every code is for. The text of a code is never to be altered
    /// (not upper-cased, not cut): a fingerprint and a key are case-sensitive.
    pub fn share_entries(&self) -> Vec<ShareCode> {
        // A key a short code cannot carry (punctuation) would make a code nobody could type back: leave the relay form out, the direct codes still work.
        let carriable = self.key.as_deref().is_none_or(super::relay::ShortJoin::can_carry);
        let relay = self
            .relay_code()
            .as_deref()
            .and_then(super::relay::parse_code)
            .filter(|_| carriable)
            .map(|code| super::relay::ShortJoin { code, key: self.key.clone() }.format())
            .map(|code| ShareCode { kind: CodeKind::Online, label: "Online code (works from anywhere)", code });
        let direct = self.direct_addresses().into_iter().map(|(kind, a)| ShareCode {
            kind,
            label: if kind == CodeKind::Direct { "Direct code (router port open)" } else { "Same network only" },
            code: JoinCode { address: a, fingerprint: self.fingerprint.clone(), key: self.key.clone() }.format(),
        });
        relay.into_iter().chain(direct).collect()
    }

    /// What asking the relay said, when it was asked and did not succeed (a line for the screen).
    pub fn relay_note(&self) -> Option<String> {
        match &*self.relay.lock().unwrap_or_else(|p| p.into_inner()) {
            RelayState::Failed(note) => Some(note.clone()),
            _ => None,
        }
    }

    /// True while the relay has been asked for the first time and has not answered yet.
    pub fn relay_pending(&self) -> bool {
        matches!(&*self.relay.lock().unwrap_or_else(|p| p.into_inner()), RelayState::Pending)
    }

    /// The line the lobby shows the host about how friends can reach them, or `None` when there is nothing to say.
    ///
    /// While the relay is being asked: that it is. When it did not answer: that, and what the router said (the UPnP trouble is only worth the host's
    /// attention then, because with a relay code friends do not need the router's port at all). When the relay changed the code: that, for a while.
    /// With no relay at all (`RE2_RELAY=off`): what the router said.
    pub fn lobby_notice(&self) -> Option<String> {
        let upnp = self.upnp_note();
        let mut relay = self.relay.lock().unwrap_or_else(|p| p.into_inner());
        match &mut *relay {
            RelayState::Off => upnp,
            RelayState::Pending => Some("CONNECTING TO THE RELAY...".to_string()),
            RelayState::Failed(note) => Some(match upnp {
                Some(u) => format!("{note} {u}"),
                None => note.clone(),
            }),
            RelayState::Ready { bridge, seen_changes, changed_at } => {
                let changes = bridge.code_changes();
                if changes != *seen_changes {
                    *seen_changes = changes;
                    *changed_at = Some(Instant::now());
                }
                changed_at
                    .filter(|t| t.elapsed() < CODE_CHANGE_NOTICE)
                    .map(|_| "THE ONLINE CODE CHANGED (THE RELAY WAS RESTARTED): FRIENDS NEED THE NEW ONE".to_string())
            }
        }
    }

    /// The transport settings a client on this machine joins with (pinned to this host's own identity over QUIC, or plain loopback UDP).
    pub fn local_transport(&self) -> Result<super::client::ClientTransportConfig, String> {
        match &self.fingerprint {
            Some(f) => Ok(super::client::ClientTransportConfig::Quic {
                trust: super::quic::ServerTrust::fingerprint(f)?,
                server_name: "localhost".to_string(),
                relay_claim: None,
            }),
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
        // The router thread removes the mapping when it stops; a router that is still being asked (seconds) is not waited for, the thread finishes the job.
        if let Some(k) = self.keeper.take() {
            for _ in 0..200 {
                if k.is_finished() {
                    let _ = k.join();
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
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

/// Registers with the relay on a thread of its own and keeps trying (every [`RELAY_RETRY_EVERY`]) until it works or the host stops: a relay that was down
/// when the game started still gives it a code once it is up. `state` says where it stands.
fn register_with_relay(
    relay: &str,
    addr: SocketAddr,
    fingerprint: Option<String>,
    stop: &Arc<AtomicBool>,
    state: &Arc<Mutex<RelayState>>,
    timing: RegisterTiming,
    retry_every: Duration,
) {
    let set = |s: RelayState| *state.lock().unwrap_or_else(|p| p.into_inner()) = s;
    loop {
        match super::relay_server::HostBridge::start_with(relay, addr, fingerprint.clone(), stop.clone(), super::relay_server::DEFAULT_HOST_KEEPALIVE, timing) {
            Ok((bridge, _code)) => {
                set(RelayState::Ready { bridge, seen_changes: 0, changed_at: None });
                return;
            }
            Err(e) => {
                eprintln!("relay: '{relay}' did not answer: {e}");
                set(RelayState::Failed(format!(
                    "NO ONLINE CODE YET: THE RELAY DID NOT ANSWER. TRYING AGAIN EVERY {} S. THE ADDRESS BELOW STILL WORKS ON YOUR NETWORK.",
                    retry_every.as_secs().max(1)
                )));
            }
        }
        let until = Instant::now() + retry_every;
        while Instant::now() < until {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if stop.load(Ordering::Relaxed) {
            return;
        }
    }
}

/// A port mapping the router opened: what a friend can be told about it, and the upkeep it needs while the game runs.
trait PortMapping: Send {
    /// The router's public `HOST:PORT` for this game (when it says) and a warning when that address cannot be reached from outside.
    fn describe(&self, port: u16) -> (Option<String>, Option<String>);
    /// Renews the lease when it is due.
    fn tick(&mut self);
    /// Removes the mapping.
    fn stop(self: Box<Self>);
}

impl PortMapping for crate::net::portmap::Keeper {
    fn describe(&self, port: u16) -> (Option<String>, Option<String>) {
        (self.found().join_address(port), self.found().reachability_warning())
    }
    fn tick(&mut self) {
        crate::net::portmap::Keeper::tick(self);
    }
    fn stop(self: Box<Self>) {
        crate::net::portmap::Keeper::stop(*self);
    }
}

/// Asks the router to open the port on a thread of its own, records the answer in `state`, then renews the mapping until `stop` and removes it.
fn spawn_upnp(options: crate::net::portmap::Options, stop: Arc<AtomicBool>, state: Arc<Mutex<UpnpState>>) -> Option<JoinHandle<()>> {
    let port = options.port;
    let ask = move || -> Result<Box<dyn PortMapping>, String> {
        crate::net::portmap::Keeper::start(&options).map(|(k, _report)| Box::new(k) as Box<dyn PortMapping>).map_err(|e| e.to_string())
    };
    spawn_upnp_with(ask, port, stop, state)
}

/// [`spawn_upnp`] with the asking given (a test asks nothing and answers as it likes).
fn spawn_upnp_with(
    ask: impl FnOnce() -> Result<Box<dyn PortMapping>, String> + Send + 'static,
    port: u16,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<UpnpState>>,
) -> Option<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("red-upnp".into())
        .spawn(move || {
            let set = |s: UpnpState| *state.lock().unwrap_or_else(|p| p.into_inner()) = s;
            match ask() {
                Ok(mut mapping) => {
                    let (public, warning) = mapping.describe(port);
                    set(UpnpState::Opened { public, note: warning.unwrap_or_else(|| "the router opened the port for your friends".to_string()) });
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(500));
                        mapping.tick();
                    }
                    mapping.stop();
                }
                Err(e) => set(UpnpState::Declined(format!(
                    "the router did not open the port ({e}); forward UDP {port} to this PC, or friends on your network can still join"
                ))),
            }
        })
        .ok()
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
        let public = PublicOptions {
            bind: Some(IpAddr::from([127, 0, 0, 1])),
            identity_dir: dir.clone(),
            port: 0,
            upnp: false,
            upnp_router: None,
            key: Some("sesame".into()),
            relay: None,
        };
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
    fn the_host_button_always_asks_for_a_fresh_typable_key_and_serves_the_relay() {
        let dir = std::path::PathBuf::from("identity");
        let a = PublicOptions::for_friends(dir.clone(), Some("relay.example:28016".into())).unwrap();
        let b = PublicOptions::for_friends(dir.clone(), None).unwrap();
        let key = a.key.as_deref().expect("a hosted game is never open to whoever holds its code");
        assert_eq!(key.len(), crate::net::relay::JOIN_KEY_LEN);
        assert!(crate::net::relay::ShortJoin::can_carry(key), "a key that fits in the code a friend types");
        assert_ne!(a.key, b.key, "a different key for every hosted game");
        assert_eq!((a.relay.as_deref(), b.relay.as_deref()), (Some("relay.example:28016"), None));
        assert!(a.bind.is_none() && a.upnp, "HOST listens for friends on every interface and asks the router; a test overrides both");
        assert_eq!(a.port, crate::net::DEFAULT_PORT);
    }

    #[test]
    fn a_relay_that_never_answers_does_not_hold_up_hosting_and_ends_as_a_note() {
        let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let dir = std::env::temp_dir().join(format!("re2_host_deadrelay_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let public = PublicOptions {
            bind: Some(IpAddr::from([127, 0, 0, 1])),
            identity_dir: dir.clone(),
            port: 0,
            upnp: false,
            upnp_router: None,
            key: None,
            relay: Some(silent.local_addr().unwrap().to_string()),
        };
        let started = Instant::now();
        let host = LocalHost::start(&lab(), &HostOptions { fill: Some(0), spawn_group: "duel".into(), public: Some(public), ..Default::default() }).unwrap();
        assert!(started.elapsed() < Duration::from_secs(5), "start returned after {:?}: the Register must not block it", started.elapsed());
        assert!(host.relay_pending() && host.relay_code().is_none());
        assert_eq!(host.lobby_notice().as_deref(), Some("CONNECTING TO THE RELAY..."), "the lobby says what is going on while it waits");
        assert!(host.share_entries().iter().all(|e| e.kind != CodeKind::Online), "no online code until the relay gives one");
        let end = Instant::now() + Duration::from_secs(30);
        while host.relay_pending() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!host.relay_pending() && host.relay_code().is_none());
        let note = host.relay_note().expect("a failure leaves a line for the screen");
        assert!(note.contains("RELAY") && note.contains("TRYING AGAIN"), "{note}");
        assert_eq!(host.lobby_notice().as_deref(), Some(note.as_str()));
        drop(host);
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
    // ---- the lobby's view of the relay and the router ----

    const FP: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A host with nothing running, in the given relay and router states: only the accessors the lobby reads.
    fn bare_host(relay: RelayState, upnp: UpnpState) -> LocalHost {
        LocalHost {
            addr: "127.0.0.1:27015".parse().unwrap(),
            stop: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
            thread: None,
            fingerprint: Some(FP.to_string()),
            lan_address: Some("192.168.0.5:27015".to_string()),
            upnp: Arc::new(Mutex::new(upnp)),
            key: Some("K7Q2MZ4PWTXA".to_string()),
            keeper: None,
            relay: Arc::new(Mutex::new(relay)),
        }
    }

    fn declined() -> UpnpState {
        UpnpState::Declined(
            "the router did not open the port (UDP 27015 is already mapped to 192.168.0.133:27015: leaving it alone); forward UDP 27015 to this PC".into(),
        )
    }

    /// A relay serving on a thread of its own until the returned flag is set; its socket is closed when the thread has ended.
    fn run_relay(bind: SocketAddr) -> (SocketAddr, Arc<AtomicBool>, JoinHandle<()>) {
        let relay = super::super::relay_server::RelayServer::bind(super::super::relay_server::RelayServerOptions {
            bind,
            housekeeping_tick: Duration::from_millis(20),
            ..Default::default()
        })
        .unwrap();
        let (addr, stop) = (relay.local_addr(), Arc::new(AtomicBool::new(false)));
        let flag = stop.clone();
        (addr, stop, std::thread::spawn(move || relay.run(&flag)))
    }

    fn wait_for<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
        let end = Instant::now() + Duration::from_secs(secs);
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_router_is_asked_on_its_own_thread_and_its_answer_reaches_the_lobby_later() {
        struct Fake(Arc<AtomicBool>);
        impl PortMapping for Fake {
            fn describe(&self, port: u16) -> (Option<String>, Option<String>) {
                (Some(format!("203.0.113.9:{port}")), None)
            }
            fn tick(&mut self) {}
            fn stop(self: Box<Self>) {
                self.0.store(true, Ordering::Relaxed);
            }
        }
        let (state, stop, removed) = (Arc::new(Mutex::new(UpnpState::Pending)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let started = Instant::now();
        let remove = removed.clone();
        let thread = spawn_upnp_with(
            move || {
                std::thread::sleep(Duration::from_millis(400)); // a router that takes its time
                Ok(Box::new(Fake(remove)) as Box<dyn PortMapping>)
            },
            27015,
            stop.clone(),
            state.clone(),
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_millis(200), "asking the router returned at once: {:?}", started.elapsed());
        assert_eq!(*state.lock().unwrap(), UpnpState::Pending);
        let opened = wait_for("the router's answer", 5, || match &*state.lock().unwrap() {
            UpnpState::Pending => None,
            other => Some(other.clone()),
        });
        assert_eq!(opened, UpnpState::Opened { public: Some("203.0.113.9:27015".into()), note: "the router opened the port for your friends".into() });
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
        assert!(removed.load(Ordering::Relaxed), "the mapping is removed when the host stops");
    }

    #[test]
    fn a_router_that_declines_leaves_a_line_that_says_what_to_do() {
        let (state, stop) = (Arc::new(Mutex::new(UpnpState::Pending)), Arc::new(AtomicBool::new(false)));
        let thread = spawn_upnp_with(
            || Err("UDP 27015 is already mapped to 192.168.0.133:27015 ('red-engine 27015'): leaving it alone".to_string()),
            27015,
            stop,
            state.clone(),
        )
        .unwrap();
        thread.join().unwrap();
        let UpnpState::Declined(note) = state.lock().unwrap().clone() else { panic!("declined") };
        assert!(note.contains("did not open the port") && note.contains("leaving it alone") && note.contains("forward UDP 27015"), "{note}");
    }

    #[test]
    fn the_routers_complaint_is_only_shown_when_there_is_no_online_code() {
        let relay_down = RelayState::Failed("NO ONLINE CODE YET: THE RELAY DID NOT ANSWER. TRYING AGAIN EVERY 20 SECONDS".into());
        // The relay is being asked: that, and nothing about the router.
        assert_eq!(bare_host(RelayState::Pending, declined()).lobby_notice().as_deref(), Some("CONNECTING TO THE RELAY..."));
        // The relay did not answer: both lines, because now the router's port is what friends have.
        let both = bare_host(relay_down, declined()).lobby_notice().unwrap();
        assert!(both.contains("NO ONLINE CODE YET") && both.contains("did not open the port"), "{both}");
        // No relay at all (RE2_RELAY=off): the router's line is all there is to say.
        assert!(bare_host(RelayState::Off, declined()).lobby_notice().unwrap().contains("did not open the port"));
        // A relay that gave a code: the router's trouble is not the host's business, so the lobby does not show it.
        let (addr, stop, thread) = run_relay("127.0.0.1:0".parse().unwrap());
        let quit = Arc::new(AtomicBool::new(false));
        let (bridge, _) = super::super::relay_server::HostBridge::start(&addr.to_string(), "127.0.0.1:9".parse().unwrap(), None, quit.clone()).unwrap();
        let ready = bare_host(RelayState::Ready { bridge, seen_changes: 0, changed_at: None }, declined());
        assert_eq!(ready.lobby_notice(), None);
        quit.store(true, Ordering::Relaxed);
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn every_code_is_offered_with_what_it_is_for_and_in_its_own_case() {
        let (addr, stop, thread) = run_relay("127.0.0.1:0".parse().unwrap());
        let quit = Arc::new(AtomicBool::new(false));
        let (bridge, code) =
            super::super::relay_server::HostBridge::start(&addr.to_string(), "127.0.0.1:9".parse().unwrap(), Some(FP.into()), quit.clone()).unwrap();
        let upnp = UpnpState::Opened { public: Some("203.0.113.9:27015".into()), note: "the router opened the port for your friends".into() };
        let host = bare_host(RelayState::Ready { bridge, seen_changes: 0, changed_at: None }, upnp);
        let entries = host.share_entries();
        assert_eq!(entries.iter().map(|e| e.kind).collect::<Vec<_>>(), [CodeKind::Online, CodeKind::Direct, CodeKind::SameNetwork]);
        assert_eq!(entries[0].code, format!("{}-K7Q2-MZ4P-WTXA", super::super::relay::code_to_string(&code)));
        assert_eq!(entries[1].code, format!("203.0.113.9:27015#{FP}#K7Q2MZ4PWTXA"), "the identity stays lower case and the key as it is");
        assert_eq!(entries[2].code, format!("192.168.0.5:27015#{FP}#K7Q2MZ4PWTXA"));
        assert!(entries.iter().all(|e| !e.label.is_empty()));
        assert_eq!(host.share_codes(), entries.iter().map(|e| e.code.clone()).collect::<Vec<_>>());
        for e in &entries {
            assert!(super::super::join::JoinTarget::parse(&e.code).is_ok(), "what the lobby lists is what JOIN accepts: {}", e.code);
        }
        quit.store(true, Ordering::Relaxed);
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
    }

    #[test]
    fn a_relay_that_restarts_gives_the_lobby_a_new_code_and_a_line_that_says_so() {
        let (addr, stop, thread) = run_relay("127.0.0.1:0".parse().unwrap());
        let quit = Arc::new(AtomicBool::new(false));
        let (bridge, original) = super::super::relay_server::HostBridge::start_with_keepalive(
            &addr.to_string(),
            "127.0.0.1:9".parse().unwrap(),
            Some(FP.into()),
            quit.clone(),
            Duration::from_millis(100),
        )
        .unwrap();
        let host = bare_host(RelayState::Ready { bridge, seen_changes: 0, changed_at: None }, UpnpState::Off);
        let first = host.share_codes()[0].clone();
        assert!(first.starts_with(&super::super::relay::code_to_string(&original)) && host.lobby_notice().is_none() && host.relay_code_changes() == 0);
        // The relay goes away and comes back empty on the same port.
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
        let (_, stop2, thread2) = run_relay(addr);
        let now = wait_for("the lobby to follow the relay", 5, || host.relay_code().filter(|c| *c != super::super::relay::code_to_string(&original)));
        assert_ne!(host.share_codes()[0], first, "the code on show is the new one");
        assert!(host.share_codes()[0].starts_with(&now));
        assert_eq!(host.relay_code_changes(), 1);
        let notice = host.lobby_notice().expect("the host is told");
        assert!(notice.contains("CHANGED") && notice.contains("NEW"), "{notice}");
        quit.store(true, Ordering::Relaxed);
        stop2.store(true, Ordering::Relaxed);
        thread2.join().unwrap();
    }

    #[test]
    fn a_relay_that_is_started_late_still_gives_a_host_that_waited_its_code() {
        // The relay is not there when the game starts (the first tries fail and the lobby says it will keep trying), and is there a moment later.
        let free = {
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            socket.local_addr().unwrap()
        };
        let (state, stop) = (Arc::new(Mutex::new(RelayState::Pending)), Arc::new(AtomicBool::new(false)));
        let timing = RegisterTiming { wait: Duration::from_millis(100), tries: 2 };
        let thread = {
            let (state, stop) = (state.clone(), stop.clone());
            std::thread::spawn(move || {
                register_with_relay(&free.to_string(), "127.0.0.1:9".parse().unwrap(), None, &stop, &state, timing, Duration::from_millis(200))
            })
        };
        let note = wait_for("the first failure", 5, || match &*state.lock().unwrap() {
            RelayState::Failed(n) => Some(n.clone()),
            _ => None,
        });
        assert!(note.contains("TRYING AGAIN"), "{note}");
        let (_, relay_stop, relay_thread) = run_relay(free);
        wait_for("a code from the late relay", 10, || matches!(&*state.lock().unwrap(), RelayState::Ready { .. }).then_some(()));
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
        relay_stop.store(true, Ordering::Relaxed);
        relay_thread.join().unwrap();
    }
}
