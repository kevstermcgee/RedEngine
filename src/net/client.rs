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

use crate::net::auth::{join_proof, Direction, SessionKey};
use crate::net::interp::{RemoteWorld, View};
use crate::net::protocol::*;
use crate::sim::clock::TICK_DT;
use crate::sim::flow::Phase;
use crate::sim::player::PlayerInput;
use std::collections::hash_map::RandomState;
use std::collections::VecDeque;
use std::hash::BuildHasher;
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
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
}

impl ClientConfig {
    /// A config for a keyless join with the default name.
    pub fn new(server: SocketAddr, character: u8, map_hash: u32, resume_token: u64) -> Self {
        ClientConfig { server, character, map_hash, resume_token, join_key: None, name: String::new() }
    }
}

/// The client. See the module docs.
pub struct NetClient {
    socket: UdpSocket,
    server: SocketAddr,
    character: u8,
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
    entropy: RandomState,
    entropy_counter: u64,
    recent_inputs: VecDeque<PlayerInput>,
    latest_snapshot_seq: u32,
    world: RemoteWorld,
    stats: ClientStats,
    out: Vec<u8>,
    status: Option<Status>,
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

impl NetClient {
    /// Opens a socket and starts joining `server`. `resume_token` is `0` for a fresh join, or a token
    /// from an earlier session to get that player back.
    pub fn connect(server: SocketAddr, character: u8, map_hash: u32, resume_token: u64) -> io::Result<NetClient> {
        Self::connect_with(ClientConfig::new(server, character, map_hash, resume_token))
    }

    /// Like [`NetClient::connect`], with a join key and a name.
    pub fn connect_with(cfg: ClientConfig) -> io::Result<NetClient> {
        let bind = SocketAddr::new(if cfg.server.is_ipv4() { Ipv4Addr::UNSPECIFIED.into() } else { Ipv6Addr::UNSPECIFIED.into() }, 0);
        let socket = UdpSocket::bind(bind)?;
        socket.set_nonblocking(true)?;
        let now = Instant::now();
        let mut c = NetClient {
            socket,
            server: cfg.server,
            character: cfg.character.min(1),
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
            entropy: RandomState::new(),
            entropy_counter: 0,
            recent_inputs: VecDeque::new(),
            latest_snapshot_seq: 0,
            world: RemoteWorld::default(),
            stats: ClientStats::default(),
            out: Vec::with_capacity(MAX_PACKET),
            status: None,
            ready: false,
            last_lobby: None,
            round_ack: 0,
            timeout: Duration::from_millis(2000),
            hello_interval: Duration::from_millis(250),
            lobby_interval: Duration::from_millis(200),
        };
        c.new_attempt();
        Ok(c)
    }

    /// Starts a fresh handshake: a new nonce, no cookie, no session key.
    fn new_attempt(&mut self) {
        self.entropy_counter = self.entropy_counter.wrapping_add(1);
        self.nonce = self.entropy.hash_one((self.entropy_counter, self.started.elapsed().as_nanos())).max(1);
        self.cookie = 0;
        self.key = None;
        self.last_hello = None;
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

    /// The phase (`Playing` until the server says otherwise, which is what an open-play server means).
    pub fn phase(&self) -> Phase {
        self.status.as_ref().map_or(Phase::Playing, |s| s.phase)
    }

    /// Whether we have a body in the running round (`false` while in the lobby, in the results, or watching a round in progress).
    pub fn in_round(&self) -> bool {
        self.welcome.is_some_and(|w| w.in_round) && self.phase() == Phase::Playing
    }

    /// The character we ask for (`0` human, `1` rat).
    pub fn character(&self) -> u8 {
        self.character
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
        let character = character.min(1);
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

    fn raw_send(&mut self) {
        if let Ok(n) = self.socket.send_to(&self.out, self.server) {
            self.stats.bytes_out += n as u64;
        }
    }

    /// Sends a tagged message; dropped silently while there is no session key yet.
    fn send_signed(&mut self, msg: &ClientMsg) {
        let Some(key) = self.key.clone() else { return };
        self.out.clear();
        msg.encode(&mut self.out);
        key.sign(Direction::ToServer, &mut self.out);
        self.raw_send();
    }

    fn send_hello(&mut self, now: Instant) {
        self.last_hello = Some(now);
        let proof = match (&self.join_key, self.cookie) {
            (Some(k), c) if c != 0 => join_proof(k.as_bytes(), self.nonce, c, self.map_hash, PROTOCOL_VERSION),
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
        self.state = ConnState::Closed;
    }

    /// Receives everything waiting, drives the handshake and reconnect timers, and reports what happened.
    pub fn poll(&mut self, now: Instant) -> Vec<NetEvent> {
        let mut events = Vec::new();
        if self.state == ConnState::Closed || matches!(self.state, ConnState::Rejected(_)) {
            return events;
        }
        let mut buf = [0u8; 2048];
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((n, from)) => {
                    if from != self.server || n > MAX_PACKET {
                        continue;
                    }
                    self.stats.bytes_in += n as u64;
                    self.on_datagram(&buf[..n], now, &mut events);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue, // Windows: an earlier send bounced
                Err(_) => break,
            }
        }
        match self.state {
            ConnState::Connecting | ConnState::Reconnecting => {
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
                    self.new_attempt();
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
        let msg = if is_signed(kind) {
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
                self.key = Some(SessionKey::derive(key.as_bytes(), self.nonce, cookie));
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
                self.welcome = Some(w);
                self.token = w.token;
                self.state = ConnState::Connected;
                self.round_ack = w.round;
                self.recent_inputs.clear();
                self.world.reset();
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
                events.push(NetEvent::Snapshot { own, ack_input_seq: s.ack_input_seq });
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
            ServerMsg::NoSession => {
                // Unauthenticated, so only believed when a live session has gone quiet (a forger cannot make a healthy one restart).
                if self.state == ConnState::Connected && now.duration_since(self.last_heard) > Duration::from_millis(500) {
                    self.state = ConnState::Reconnecting;
                    self.new_attempt();
                    events.push(NetEvent::ServerBye);
                }
            }
            ServerMsg::Bye => {
                // The server is closing (a restart, a shutdown): rejoin with our token.
                self.last_heard = now;
                if self.state == ConnState::Connected {
                    self.state = ConnState::Reconnecting;
                    self.new_attempt();
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
