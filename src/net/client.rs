//! The client side of the connection: handshake, redundant input sending, snapshot receiving, the lobby, automatic reconnect. No
//! window or GPU: the graphical game and the headless `red_bot` share it.
//!
//! Call [`NetClient::poll`] often (every frame) and act on the [`NetEvent`]s it returns; call
//! [`NetClient::send_input`] once per simulation tick. If the server goes silent the client keeps
//! retrying its `Hello` with the old token, so a restarted or briefly unreachable server picks the
//! player back up ([`NetEvent::Connected`] fires again, with the resumed state).
//!
//! The handshake (ADR 0028): `Hello` → the server's `Challenge` (a cookie proving our address) → `Hello` again with the cookie and, if
//! we hold a join key, its proof → a `Welcome` signed with the session key both sides now derive. Everything after that is tagged, and a
//! datagram whose tag does not verify is dropped here without being read.
//!
//! In a match flow ([`crate::sim::flow`]) the server also sends a [`Status`] (phase, timer, roster) that [`NetClient::status`] keeps, and
//! a new `Welcome` at each round start ([`NetEvent::Connected`] again: put the player at the new spawn). [`NetClient::set_ready`] and
//! [`NetClient::set_character`] are *state*, repeated until the server's roster shows them, so a lost packet costs nothing.
//! [`RuleState`] is likewise a repeated complete presentation snapshot: variables, visibility, recent event and outcome recover after loss,
//! reconnect and late join without replaying transient commands.

use crate::net::auth::{join_proof, join_proof_bound, Direction, SessionKey};
use crate::net::happenings::{Happenings, Watcher};
use crate::net::interp::{RemoteWorld, View};
use crate::net::protocol::*;
use crate::net::quic::{QuicClient, ServerTrust};
use crate::net::transport::{ClientTransport, Security, TransportStatus, UdpClient};
use crate::sim::clock::TICK_DT;
use crate::sim::flow::Phase;
use crate::sim::player::PlayerInput;
use std::collections::VecDeque;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

/// Where the connection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    /// Hello sent, waiting for Welcome.
    Connecting,
    /// Playing.
    Connected,
    /// The server stopped answering; retrying with the old token.
    Reconnecting,
    /// The server refused us (will not retry).
    Rejected(RejectReason),
    /// We left.
    Closed,
}

/// Something that happened, returned by [`NetClient::poll`].
#[derive(Debug, Clone, PartialEq)]
pub enum NetEvent {
    /// Joined or resumed, or a new round placed us: the state the server has us in.
    Connected(Welcome),
    /// A snapshot arrived: our own state in it (to reconcile prediction), and the input sequence it acknowledges.
    Snapshot {
        /// Our player's entry in the snapshot.
        own: Option<PlayerSnap>,
        /// Newest input the server has processed for us.
        ack_input_seq: u32,
        /// The race, in a race match (whether the light is green decides whether our input counts).
        race: Option<RaceSnap>,
    },
    /// The lobby / round phase or the round number changed (read [`NetClient::status`] for the details).
    PhaseChanged {
        /// The new phase.
        phase: Phase,
        /// The round it belongs to.
        round: u16,
    },
    /// The server went quiet (a reconnect attempt begins).
    Disconnected,
    /// The server refused the join.
    Rejected(RejectReason),
    /// The server told us it is shutting down or does not know us.
    ServerBye,
    /// A snapshot reported something new: shots other players fired, and what we did and suffered (hits landed, damage taken, kills).
    Happened(Happenings),
}

/// Connection statistics.
#[derive(Debug, Clone, Default)]
pub struct ClientStats {
    /// Snapshots received.
    pub snapshots: u64,
    /// Snapshots the acknowledgement numbers say were skipped (lost or reordered away).
    pub snapshots_missed: u64,
    /// Bytes received / sent.
    pub bytes_in: u64,
    /// Bytes sent.
    pub bytes_out: u64,
    /// Smoothed round-trip time, milliseconds.
    pub rtt_ms: f32,
    /// Times a (re)connect completed.
    pub connects: u32,
    /// Datagrams dropped because their authentication tag was wrong or they could not be read.
    pub bad_packets: u64,
}

/// Which transport a client uses (ADR 0044). There is no automatic choice and no fallback: a QUIC client that cannot verify the server
/// stops; it never retries over UDP.
#[derive(Clone)]
pub enum ClientTransportConfig {
    /// Development UDP: authenticated, **not encrypted**, no server identity. Loopback tools, tests, trusted LANs.
    DevUdp,
    /// QUIC + TLS 1.3, the server verified by `trust` for `server_name`.
    Quic {
        /// How the server's certificate is checked.
        trust: ServerTrust,
        /// The name the certificate is for (any name works with a pinned fingerprint).
        server_name: String,
    },
}

impl ClientTransportConfig {
    /// The transport a command line asked for (`--server-fingerprint`, `--server-ca` + `--server-name`, `--dev-udp`). With none of them a
    /// loopback server gets development UDP (what local tools and `play-local` run) and any other server is refused: a client never
    /// talks plaintext to the network unless told to, and never falls back from QUIC.
    pub fn choose(
        server: SocketAddr,
        fingerprint: Option<&str>,
        ca: Option<&std::path::Path>,
        server_name: Option<&str>,
        dev_udp: bool,
    ) -> Result<Self, String> {
        let name = server_name.map(str::to_string).unwrap_or_else(|| server.ip().to_string());
        match (fingerprint, ca, dev_udp) {
            (Some(_), Some(_), _) => Err("give either --server-fingerprint or --server-ca, not both".to_string()),
            (Some(_), _, true) | (_, Some(_), true) => Err("--dev-udp cannot be combined with a server identity: choose one transport".to_string()),
            (Some(f), None, false) => Ok(ClientTransportConfig::Quic { trust: ServerTrust::fingerprint(f)?, server_name: name }),
            (None, Some(path), false) => Ok(ClientTransportConfig::Quic { trust: ServerTrust::roots_file(path)?, server_name: name }),
            (None, None, true) => Ok(ClientTransportConfig::DevUdp),
            (None, None, false) if server.ip().is_loopback() => Ok(ClientTransportConfig::DevUdp),
            (None, None, false) => Err(format!(
                "{server} is not on this machine: pass the server's --server-fingerprint (it prints one at start) or --server-ca, so the \
                 connection is encrypted and the server verified; --dev-udp joins a development server in plaintext (trusted LAN only)"
            )),
        }
    }
}

impl std::fmt::Debug for ClientTransportConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientTransportConfig::DevUdp => f.write_str("DevUdp"),
            ClientTransportConfig::Quic { server_name, .. } => write!(f, "Quic {{ server_name: {server_name:?} }}"),
        }
    }
}

/// Everything needed to join a server.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// The server's address.
    pub server: SocketAddr,
    /// `0` human, `1` rat.
    pub character: u8,
    /// Hash of our copy of the map (`net::map_hash`).
    pub map_hash: u32,
    /// A token from an earlier session to resume, or `0`.
    pub resume_token: u64,
    /// The join key, if the server has one.
    pub join_key: Option<String>,
    /// The name to show in the lobby.
    pub name: String,
    /// The transport. [`ClientConfig::new`] picks development UDP (loopback tools and tests); production clients set
    /// [`ClientTransportConfig::Quic`] ([`ClientConfig::quic`]).
    pub transport: ClientTransportConfig,
}

impl ClientConfig {
    /// A config for a keyless join with the default name over **development UDP** (not encrypted).
    pub fn new(server: SocketAddr, character: u8, map_hash: u32, resume_token: u64) -> Self {
        ClientConfig { server, character, map_hash, resume_token, join_key: None, name: String::new(), transport: ClientTransportConfig::DevUdp }
    }

    /// A config for a keyless join over QUIC, the server verified by `trust`.
    pub fn quic(server: SocketAddr, character: u8, map_hash: u32, trust: ServerTrust, server_name: &str) -> Self {
        ClientConfig {
            transport: ClientTransportConfig::Quic { trust, server_name: server_name.to_string() },
            ..ClientConfig::new(server, character, map_hash, 0)
        }
    }
}

/// The client. See the module docs.
pub struct NetClient {
    transport: Box<dyn ClientTransport>,
    /// The transport encrypts and authenticates every datagram (QUIC): no Red tags.
    secure: bool,
    /// Why the transport gave up (a server identity that did not verify).
    transport_error: Option<String>,
    /// The Challenge arrived: messages after the handshake may be sent.
    session_ready: bool,
    last_reconnect: Option<Instant>,
    server: SocketAddr,
    character: u8,
    /// The team we ask for (`0` = whichever has room).
    team: u8,
    map_hash: u32,
    name: String,
    join_key: Option<String>,
    state: ConnState,
    welcome: Option<Welcome>,
    token: u64,
    started: Instant,
    last_hello: Option<Instant>,
    last_heard: Instant,
    /// This attempt's random number, its cookie (0 until the Challenge) and the key derived from both.
    nonce: u64,
    cookie: u64,
    requires_key: bool,
    key: Option<SessionKey>,
    recent_inputs: VecDeque<PlayerInput>,
    latest_snapshot_seq: u32,
    world: RemoteWorld,
    /// Turns the counters in consecutive snapshots into [`NetEvent::Happened`].
    watcher: Watcher,
    /// The loadout match's world and our kit as of the newest snapshot (`None` in any other kind of match).
    arena: Option<crate::net::protocol::ArenaSnap>,
    /// The last seconds of the match, for the killcam.
    recorder: crate::killcam::Recorder,
    /// Tenths of a second until we respawn, from the newest snapshot (`0` = alive).
    respawn_tenths: u8,
    stats: ClientStats,
    out: Vec<u8>,
    status: Option<Status>,
    rule_state: Option<RuleState>,
    ready: bool,
    last_lobby: Option<Instant>,
    /// The round of the newest Welcome we applied (echoed to the server so it stops repeating it).
    round_ack: u16,
    /// Silence longer than this while connected counts as a lost connection.
    pub timeout: Duration,
    /// How often an unanswered Hello is resent.
    pub hello_interval: Duration,
    /// How often the lobby state is resent while not playing.
    pub lobby_interval: Duration,
}

/// The local address a client socket binds to in order to talk to `server`: loopback when the server is on loopback (tests, bots, `play-local`,
/// `net-test`), the wildcard otherwise. A loopback socket never raises the OS firewall prompt, so unattended runs cannot stall on it.
pub fn local_bind_for(server: SocketAddr) -> SocketAddr {
    let ip: std::net::IpAddr = match (server.ip().is_loopback(), server.is_ipv4()) {
        (true, true) => Ipv4Addr::LOCALHOST.into(),
        (true, false) => Ipv6Addr::LOCALHOST.into(),
        (false, true) => Ipv4Addr::UNSPECIFIED.into(),
        (false, false) => Ipv6Addr::UNSPECIFIED.into(),
    };
    SocketAddr::new(ip, 0)
}

impl NetClient {
    /// Opens a **development UDP** socket and starts joining `server` (loopback tools and tests; production uses
    /// [`NetClient::connect_with`] and [`ClientConfig::quic`]). `resume_token` is `0` for a fresh join, or a token from an earlier
    /// session to get that player back.
    pub fn connect(server: SocketAddr, character: u8, map_hash: u32, resume_token: u64) -> io::Result<NetClient> {
        Self::connect_with(ClientConfig::new(server, character, map_hash, resume_token))
    }

    /// Like [`NetClient::connect`], with a join key and a name.
    pub fn connect_with(cfg: ClientConfig) -> io::Result<NetClient> {
        let transport: Box<dyn ClientTransport> = match &cfg.transport {
            ClientTransportConfig::DevUdp => Box::new(UdpClient::connect(cfg.server)?),
            ClientTransportConfig::Quic { trust, server_name } => Box::new(QuicClient::connect(cfg.server, server_name, trust.clone())?),
        };
        Ok(Self::with_transport(cfg, transport, Instant::now()))
    }

    /// A client over a transport the caller built (for instance [`crate::net::memnet`]'s in-memory link), starting its clocks at `now`. `cfg.transport` is ignored:
    /// the transport *is* the choice. The handshake is the same; only the bytes travel differently.
    pub fn with_transport(cfg: ClientConfig, transport: Box<dyn ClientTransport>, now: Instant) -> NetClient {
        let mut c = NetClient {
            secure: transport.security().is_secure(),
            transport,
            transport_error: None,
            session_ready: false,
            last_reconnect: None,
            server: cfg.server,
            character: cfg.character.min(MAX_CHOICE),
            team: 0,
            map_hash: cfg.map_hash,
            name: sanitize_name(&cfg.name),
            join_key: cfg.join_key.filter(|k| !k.is_empty()),
            state: ConnState::Connecting,
            welcome: None,
            token: cfg.resume_token,
            started: now,
            last_hello: None,
            last_heard: now,
            nonce: 0,
            cookie: 0,
            requires_key: false,
            key: None,
            recent_inputs: VecDeque::new(),
            latest_snapshot_seq: 0,
            world: RemoteWorld::default(),
            watcher: Watcher::default(),
            respawn_tenths: 0,
            arena: None,
            recorder: crate::killcam::Recorder::new(),
            stats: ClientStats::default(),
            out: Vec::with_capacity(MAX_PACKET),
            status: None,
            rule_state: None,
            ready: false,
            last_lobby: None,
            round_ack: 0,
            timeout: Duration::from_millis(2000),
            hello_interval: Duration::from_millis(250),
            lobby_interval: Duration::from_millis(200),
        };
        c.new_attempt();
        c
    }

    /// Starts a fresh handshake: a new nonce, no cookie, no session key. The nonce is a freshness value, not a secret; it still comes
    /// from the OS CSPRNG (with a clock-derived fallback only if the OS has none, which cannot make a proof weaker: the key is the secret).
    fn new_attempt(&mut self) {
        self.nonce = crate::crypto::random_u64().unwrap_or_else(|_| (self.started.elapsed().as_nanos() as u64).max(1));
        self.cookie = 0;
        self.key = None;
        self.session_ready = false;
        self.last_hello = None;
    }

    /// A fresh handshake on a fresh transport connection (the server went silent, restarted or said goodbye).
    fn restart(&mut self, now: Instant) {
        self.new_attempt();
        self.transport.reconnect();
        self.last_reconnect = Some(now);
    }

    /// The server this client talks to.
    pub fn server(&self) -> SocketAddr {
        self.server
    }

    /// Which transport protects this connection.
    pub fn security(&self) -> Security {
        self.transport.security()
    }

    /// Why the transport refused to connect (a server identity that did not verify), if it did.
    pub fn transport_error(&self) -> Option<&str> {
        self.transport_error.as_deref()
    }

    /// Where the connection is.
    pub fn state(&self) -> ConnState {
        self.state
    }

    /// Our player id (`None` until welcomed).
    pub fn my_id(&self) -> Option<u8> {
        self.welcome.map(|w| w.player_id)
    }

    /// The token to resume this player later (`0` until welcomed).
    pub fn token(&self) -> u64 {
        self.token
    }

    /// The latest Welcome.
    pub fn welcome(&self) -> Option<Welcome> {
        self.welcome
    }

    /// Statistics.
    pub fn stats(&self) -> &ClientStats {
        &self.stats
    }

    /// Whether the server said it wants a join key (known after its Challenge).
    pub fn server_requires_key(&self) -> bool {
        self.requires_key
    }

    /// The newest lobby / round status (`None` until the first one arrives).
    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    /// Newest complete authoritative rule presentation state.
    pub fn rule_state(&self) -> Option<&RuleState> {
        self.rule_state.as_ref()
    }

    /// Seconds until we respawn (`0.0` while alive), as of the newest snapshot.
    pub fn respawn_in_secs(&self) -> f32 {
        self.respawn_tenths as f32 * 0.1
    }

    /// The phase (`Playing` until the server says otherwise, which is what an open-play server means).
    pub fn phase(&self) -> Phase {
        self.status.as_ref().map_or(Phase::Playing, |s| s.phase)
    }

    /// Whether we have a body in the running round (`false` while in the lobby, in the results, or watching a round in progress).
    pub fn in_round(&self) -> bool {
        self.welcome.is_some_and(|w| w.in_round) && self.phase() == Phase::Playing
    }

    /// The character we ask for (`0` human, `1` rat; in a race the driver, `Driver::wire`).
    pub fn character(&self) -> u8 {
        self.character
    }

    /// The team we ask for (`0` = whichever has room). The server's answer is our roster entry's team.
    pub fn team(&self) -> u8 {
        self.team
    }

    /// Asks for a team (`1` or `2`; `0` for whichever has room) for the next round; repeated to the server until it shows in the roster. Sent at once.
    pub fn set_team(&mut self, team: u8, now: Instant) {
        let team = team.min(2);
        if self.team != team {
            self.team = team;
            self.send_lobby(now);
        }
    }

    /// The loadout match's world and our kit as of the newest snapshot.
    pub fn arena(&self) -> Option<&crate::net::protocol::ArenaSnap> {
        self.arena.as_ref()
    }

    /// The last seconds of the match as snapshots showed them (the killcam cuts its replay from this).
    pub fn recorder(&self) -> &crate::killcam::Recorder {
        &self.recorder
    }

    /// Whether we asked to be ready.
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Presses or releases Ready; repeated to the server until it shows in the roster. Sent at once.
    pub fn set_ready(&mut self, ready: bool, now: Instant) {
        if self.ready != ready {
            self.ready = ready;
            self.send_lobby(now);
        }
    }

    /// Chooses `0` human or `1` rat for the next spawn. Sent at once.
    pub fn set_character(&mut self, character: u8, now: Instant) {
        let character = character.min(MAX_CHOICE);
        if self.character != character {
            self.character = character;
            self.send_lobby(now);
        }
    }

    /// Seconds since this client was created (the client's own clock for interpolation).
    pub fn local_secs(&self, now: Instant) -> f64 {
        now.duration_since(self.started).as_secs_f64()
    }

    /// Other players and props, interpolated for drawing at `now`.
    pub fn view(&self, now: Instant) -> View {
        self.world.view(self.local_secs(now), self.my_id())
    }

    /// Snapshots applied to the remote world.
    pub fn remote_world(&self) -> &RemoteWorld {
        &self.world
    }

    /// Configure interpolation for the locally validated map's movement speeds.
    pub fn set_movement_profile(&mut self, tuning: crate::player::PlayerTuning, pads: &[crate::player::JumpPad]) {
        self.world.set_movement_profile(tuning, pads);
    }

    fn raw_send(&mut self) {
        if let Ok(n) = self.transport.send(&self.out) {
            self.stats.bytes_out += n as u64;
        }
    }

    /// Sends a message of the session (tagged on development UDP); dropped silently before the handshake's Challenge.
    fn send_signed(&mut self, msg: &ClientMsg) {
        if !self.session_ready {
            return;
        }
        self.out.clear();
        msg.encode(&mut self.out);
        if let Some(key) = &self.key {
            key.sign(Direction::ToServer, &mut self.out);
        }
        self.raw_send();
    }

    fn send_hello(&mut self, now: Instant) {
        if self.transport.status() != TransportStatus::Ready {
            return; // QUIC still handshaking: the Hello goes once the connection (and its identity) is established
        }
        self.last_hello = Some(now);
        let proof = match (&self.join_key, self.cookie) {
            (Some(k), c) if c != 0 => {
                if self.secure {
                    // Bound to this TLS connection: useless if captured or relayed to another one.
                    let Some(binding) = self.transport.channel_binding() else { return };
                    join_proof_bound(k.as_bytes(), &binding, self.nonce, c, self.map_hash, PROTOCOL_VERSION)
                } else {
                    join_proof(k.as_bytes(), self.nonce, c, self.map_hash, PROTOCOL_VERSION)
                }
            }
            _ => [0; crate::net::auth::PROOF_LEN],
        };
        let h = Hello {
            version: PROTOCOL_VERSION,
            map_hash: self.map_hash,
            character: self.character,
            resume_token: self.token,
            client_nonce: self.nonce,
            cookie: self.cookie,
            proof,
            name: self.name.clone(),
        };
        self.out.clear();
        ClientMsg::Hello(h).encode(&mut self.out);
        self.raw_send();
    }

    fn send_lobby(&mut self, now: Instant) {
        if self.state != ConnState::Connected {
            return;
        }
        self.last_lobby = Some(now);
        let l = LobbyCmd {
            ready: self.ready,
            character: self.character,
            team: self.team,
            round_ack: self.round_ack,
            client_time_ms: now.duration_since(self.started).as_millis() as u32,
            rtt_ms: self.stats.rtt_ms.min(9_999.0) as u16,
        };
        self.send_signed(&ClientMsg::Lobby(l));
    }

    /// Sends this tick's input (with the last few, so one lost packet loses nothing). Ignored unless we have a body in a running round.
    pub fn send_input(&mut self, input: PlayerInput, now: Instant) {
        if self.state != ConnState::Connected || !self.in_round() {
            return;
        }
        self.recent_inputs.push_back(input);
        while self.recent_inputs.len() > MAX_INPUTS_PER_PACKET {
            self.recent_inputs.pop_front();
        }
        let p = InputPacket {
            snapshot_ack: self.latest_snapshot_seq,
            client_time_ms: now.duration_since(self.started).as_millis() as u32,
            round_ack: self.round_ack,
            rtt_ms: self.stats.rtt_ms.min(9_999.0) as u16,
            inputs: self.recent_inputs.iter().copied().collect(),
        };
        self.send_signed(&ClientMsg::Input(p));
    }

    /// Leaves politely (the server frees our player at once) and closes.
    pub fn disconnect(&mut self) {
        if matches!(self.state, ConnState::Connected | ConnState::Reconnecting | ConnState::Connecting) {
            for _ in 0..3 {
                self.send_signed(&ClientMsg::Bye);
            }
        }
        self.transport.close(); // QUIC: CONNECTION_CLOSE, so the server ends the session at once even if the Bye datagrams are lost
        self.state = ConnState::Closed;
    }

    /// Receives everything waiting, drives the handshake and reconnect timers, and reports what happened.
    pub fn poll(&mut self, now: Instant) -> Vec<NetEvent> {
        let mut events = Vec::new();
        if self.state == ConnState::Closed || matches!(self.state, ConnState::Rejected(_)) {
            return events;
        }
        // Fail closed: a server whose identity did not verify is never retried, and never retried over another transport.
        if let TransportStatus::Failed(why) = self.transport.status() {
            self.transport_error = Some(why);
            self.state = ConnState::Rejected(RejectReason::ServerIdentity);
            events.push(NetEvent::Rejected(RejectReason::ServerIdentity));
            return events;
        }
        // QUIC told us the connection closed (the server shut down or dropped us): rejoin now instead of waiting for the timeout.
        if self.state == ConnState::Connected && self.secure && self.transport.status() == TransportStatus::Connecting {
            self.state = ConnState::Reconnecting;
            self.restart(now);
            self.last_heard = now;
            events.push(NetEvent::ServerBye);
        }
        let mut buf = vec![0u8; crate::net::quic::MAX_STREAM_MESSAGE];
        while let Some(n) = self.transport.recv(&mut buf) {
            if n > crate::net::quic::MAX_STREAM_MESSAGE {
                continue;
            }
            self.stats.bytes_in += n as u64;
            self.on_datagram(&buf[..n], now, &mut events);
        }
        match self.state {
            ConnState::Connecting | ConnState::Reconnecting => {
                // A QUIC connection that dropped while (re)joining gets a new one every couple of seconds (UDP: nothing to do).
                if self.transport.status() == TransportStatus::Connecting && self.last_reconnect.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(2))
                {
                    self.transport.reconnect();
                    self.last_reconnect = Some(now);
                }
                if self.last_hello.is_none_or(|t| now.duration_since(t) >= self.hello_interval) {
                    // A cookie is only good for about 20 s: a handshake that has not finished by then starts over.
                    if self.cookie != 0 && now.duration_since(self.last_heard) > Duration::from_secs(8) {
                        self.new_attempt();
                    }
                    self.send_hello(now);
                }
            }
            ConnState::Connected => {
                if now.duration_since(self.last_heard) > self.timeout {
                    self.state = ConnState::Reconnecting;
                    self.restart(now);
                    self.last_heard = now;
                    events.push(NetEvent::Disconnected);
                } else if !self.in_round() && self.last_lobby.is_none_or(|t| now.duration_since(t) >= self.lobby_interval) {
                    self.send_lobby(now); // the lobby's keep-alive and the repeated ready/character state
                }
            }
            _ => {}
        }
        events
    }

    fn on_datagram(&mut self, bytes: &[u8], now: Instant, events: &mut Vec<NetEvent>) {
        let Some(kind) = peek_kind(bytes) else {
            self.stats.bad_packets += 1;
            return;
        };
        let msg = if is_signed(kind) && !self.secure {
            // Only a datagram whose tag verifies is ever parsed.
            let body = self.key.as_ref().and_then(|k| k.verify(Direction::ToClient, bytes));
            match body.map(ServerMsg::decode) {
                Some(Ok(m)) => m,
                _ => {
                    self.stats.bad_packets += 1;
                    return;
                }
            }
        } else {
            match ServerMsg::decode(bytes) {
                Ok(m) => m,
                Err(_) => {
                    self.stats.bad_packets += 1;
                    return;
                }
            }
        };
        self.on_message(msg, now, events);
    }

    fn on_message(&mut self, msg: ServerMsg, now: Instant, events: &mut Vec<NetEvent>) {
        let handshaking = matches!(self.state, ConnState::Connecting | ConnState::Reconnecting);
        match msg {
            ServerMsg::Challenge { cookie, requires_key } => {
                if !handshaking || cookie == 0 {
                    return;
                }
                self.requires_key = requires_key;
                if requires_key && self.join_key.is_none() {
                    self.state = ConnState::Rejected(RejectReason::NeedsKey);
                    events.push(NetEvent::Rejected(RejectReason::NeedsKey));
                    return;
                }
                if !requires_key && self.join_key.is_some() {
                    self.state = ConnState::Rejected(RejectReason::ServerIsOpen);
                    events.push(NetEvent::Rejected(RejectReason::ServerIsOpen));
                    return;
                }
                self.cookie = cookie;
                self.last_heard = now;
                let key = self.join_key.clone().unwrap_or_default();
                // Development UDP tags every datagram with a session key; QUIC's connection already protects them.
                self.key = (!self.secure).then(|| SessionKey::derive(key.as_bytes(), self.nonce, cookie));
                self.session_ready = true;
                self.send_hello(now); // straight away: the second Hello, carrying the cookie and the proof
            }
            ServerMsg::Welcome(w) => {
                self.last_heard = now;
                let same_place =
                    self.welcome.is_some_and(|o| o.player_id == w.player_id && o.token == w.token && o.round == w.round && o.in_round == w.in_round);
                if self.state == ConnState::Connected && same_place {
                    return; // a duplicate answer to a retransmitted Hello, or a repeated round Welcome
                }
                let first = self.state != ConnState::Connected;
                let new_round = self.welcome.is_some_and(|old| old.round != w.round);
                self.welcome = Some(w);
                if new_round {
                    self.rule_state = None;
                }
                self.token = w.token;
                self.state = ConnState::Connected;
                self.round_ack = w.round;
                self.recent_inputs.clear();
                self.world.reset();
                self.watcher.reset(); // counters restart with a new world: the next snapshot is a baseline
                self.respawn_tenths = 0;
                self.arena = None;
                self.recorder.clear();
                if first {
                    self.latest_snapshot_seq = 0;
                    self.stats.connects += 1;
                }
                events.push(NetEvent::Connected(w));
                self.send_lobby(now); // tell the server we have this round's Welcome
            }
            ServerMsg::Reject(r) => {
                if handshaking {
                    self.state = ConnState::Rejected(r);
                    events.push(NetEvent::Rejected(r));
                }
            }
            ServerMsg::Snapshot(s) => {
                self.last_heard = now;
                if self.state != ConnState::Connected {
                    return;
                }
                if s.seq > self.latest_snapshot_seq {
                    self.stats.snapshots_missed += (s.seq - self.latest_snapshot_seq - 1) as u64;
                    self.latest_snapshot_seq = s.seq;
                }
                let local = self.local_secs(now);
                self.world.apply(&s, local);
                self.stats.snapshots += 1;
                self.note_rtt(now, s.echo_time_ms, s.echo_hold_ms, s.ack_input_seq != 0);
                let me = self.my_id();
                let own = s.players.iter().find(|p| Some(p.id) == me).copied();
                self.respawn_tenths = s.fx.respawn;
                self.recorder.record(&s);
                self.arena.clone_from(&s.arena);
                let happened = self.watcher.observe(&s, me);
                events.push(NetEvent::Snapshot { own, ack_input_seq: s.ack_input_seq, race: s.race });
                if !happened.is_empty() {
                    events.push(NetEvent::Happened(happened));
                }
            }
            ServerMsg::Status(st) => {
                self.last_heard = now;
                if self.state != ConnState::Connected {
                    return;
                }
                if self.status.as_ref().is_some_and(|old| (st.seq.wrapping_sub(old.seq) as i16) <= 0) {
                    return; // an older status arriving late
                }
                self.note_rtt(now, st.echo_time_ms, st.echo_hold_ms, false);
                let changed = self.status.as_ref().is_none_or(|old| old.phase != st.phase || old.round != st.round);
                let (phase, round) = (st.phase, st.round);
                self.status = Some(st);
                if changed && matches!(phase, Phase::Playing | Phase::Results) {
                    self.ready = false; // a rematch needs Ready pressed again; a stale flag must not skip the results
                }
                if changed {
                    events.push(NetEvent::PhaseChanged { phase, round });
                }
            }
            ServerMsg::RuleState(st) => {
                self.last_heard = now;
                if self.state != ConnState::Connected {
                    return;
                }
                if self.welcome.is_none_or(|w| w.round != st.round) {
                    return; // delayed state from the previous round, or a new round whose Welcome has not arrived yet
                }
                if self.rule_state.as_ref().is_some_and(|old| (st.seq.wrapping_sub(old.seq) as i16) <= 0) {
                    return;
                }
                self.rule_state = Some(st);
            }
            ServerMsg::NoSession => {
                // Unauthenticated, so only believed when a live session has gone quiet (a forger cannot make a healthy one restart).
                if self.state == ConnState::Connected && now.duration_since(self.last_heard) > Duration::from_millis(500) {
                    self.state = ConnState::Reconnecting;
                    self.restart(now);
                    events.push(NetEvent::ServerBye);
                }
            }
            ServerMsg::Bye => {
                // The server is closing (a restart, a shutdown): rejoin with our token.
                self.last_heard = now;
                if self.state == ConnState::Connected {
                    self.state = ConnState::Reconnecting;
                    self.restart(now);
                }
                events.push(NetEvent::ServerBye);
            }
        }
    }

    fn note_rtt(&mut self, now: Instant, echo_time_ms: u32, echo_hold_ms: u16, acked_input: bool) {
        if echo_time_ms == 0 && !acked_input {
            return;
        }
        let now_ms = now.duration_since(self.started).as_millis() as i64;
        let rtt = (now_ms - echo_time_ms as i64 - echo_hold_ms as i64).max(0) as f32;
        self.stats.rtt_ms = if self.stats.rtt_ms == 0.0 { rtt } else { self.stats.rtt_ms * 0.9 + rtt * 0.1 };
    }
}

/// Seconds per simulation tick (what a client paces its input sending by).
pub const TICK_SECS: f64 = TICK_DT as f64;

#[cfg(test)]
mod bind_tests {
    use super::local_bind_for;
    use std::net::SocketAddr;

    fn bind(server: &str) -> SocketAddr {
        local_bind_for(server.parse().unwrap())
    }

    #[test]
    fn a_loopback_server_gets_a_loopback_socket() {
        assert_eq!(bind("127.0.0.1:27015"), "127.0.0.1:0".parse().unwrap());
        assert_eq!(bind("[::1]:27015"), "[::1]:0".parse().unwrap());
    }

    #[test]
    fn a_remote_server_gets_the_wildcard() {
        assert_eq!(bind("203.0.113.9:27015"), "0.0.0.0:0".parse().unwrap());
        assert_eq!(bind("[2001:db8::1]:27015"), "[::]:0".parse().unwrap());
    }
}
