//! The client side of the connection: handshake, redundant input sending, snapshot receiving,
//! automatic reconnect. No window or GPU: the graphical game and the headless `red_bot` share it.
//!
//! Call [`NetClient::poll`] often (every frame) and act on the [`NetEvent`]s it returns; call
//! [`NetClient::send_input`] once per simulation tick. If the server goes silent the client keeps
//! retrying its `Hello` with the old token, so a restarted or briefly unreachable server picks the
//! player back up ([`NetEvent::Connected`] fires again, with the resumed state).

use crate::net::interp::{RemoteWorld, View};
use crate::net::protocol::*;
use crate::sim::clock::TICK_DT;
use crate::sim::player::PlayerInput;
use std::collections::VecDeque;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, UdpSocket};
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
    /// Joined or resumed: the state the server has us in.
    Connected(Welcome),
    /// A snapshot arrived: our own state in it (to reconcile prediction), and the input sequence it acknowledges.
    Snapshot {
        /// Our player's entry in the snapshot.
        own: Option<PlayerSnap>,
        /// Newest input the server has processed for us.
        ack_input_seq: u32,
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
}

/// The client. See the module docs.
pub struct NetClient {
    socket: UdpSocket,
    server: SocketAddr,
    character: u8,
    map_hash: u32,
    state: ConnState,
    welcome: Option<Welcome>,
    token: u64,
    started: Instant,
    last_hello: Option<Instant>,
    last_heard: Instant,
    recent_inputs: VecDeque<PlayerInput>,
    latest_snapshot_seq: u32,
    world: RemoteWorld,
    stats: ClientStats,
    out: Vec<u8>,
    /// Silence longer than this while connected counts as a lost connection.
    pub timeout: Duration,
    /// How often an unanswered Hello is resent.
    pub hello_interval: Duration,
}

impl NetClient {
    /// Opens a socket and starts joining `server`. `resume_token` is `0` for a fresh join, or a token
    /// from an earlier session to get that player back.
    pub fn connect(server: SocketAddr, character: u8, map_hash: u32, resume_token: u64) -> io::Result<NetClient> {
        let bind: SocketAddr = if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }.parse().expect("literal");
        let socket = UdpSocket::bind(bind)?;
        socket.set_nonblocking(true)?;
        let now = Instant::now();
        Ok(NetClient {
            socket,
            server,
            character,
            map_hash,
            state: ConnState::Connecting,
            welcome: None,
            token: resume_token,
            started: now,
            last_hello: None,
            last_heard: now,
            recent_inputs: VecDeque::new(),
            latest_snapshot_seq: 0,
            world: RemoteWorld::default(),
            stats: ClientStats::default(),
            out: Vec::with_capacity(MAX_PACKET),
            timeout: Duration::from_millis(2000),
            hello_interval: Duration::from_millis(250),
        })
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

    fn send(&mut self, msg: &ClientMsg) {
        self.out.clear();
        msg.encode(&mut self.out);
        if let Ok(n) = self.socket.send_to(&self.out, self.server) {
            self.stats.bytes_out += n as u64;
        }
    }

    fn send_hello(&mut self, now: Instant) {
        self.last_hello = Some(now);
        let h = Hello { version: PROTOCOL_VERSION, map_hash: self.map_hash, character: self.character, resume_token: self.token };
        self.send(&ClientMsg::Hello(h));
    }

    /// Sends this tick's input (with the last few, so one lost packet loses nothing). Ignored unless connected.
    pub fn send_input(&mut self, input: PlayerInput, now: Instant) {
        if self.state != ConnState::Connected {
            return;
        }
        self.recent_inputs.push_back(input);
        while self.recent_inputs.len() > MAX_INPUTS_PER_PACKET {
            self.recent_inputs.pop_front();
        }
        let p = InputPacket { snapshot_ack: self.latest_snapshot_seq, client_time_ms: now.duration_since(self.started).as_millis() as u32, inputs: self.recent_inputs.iter().copied().collect() };
        self.send(&ClientMsg::Input(p));
    }

    /// Leaves politely (the server frees our player at once) and closes.
    pub fn disconnect(&mut self) {
        if matches!(self.state, ConnState::Connected | ConnState::Reconnecting | ConnState::Connecting) {
            for _ in 0..3 {
                self.send(&ClientMsg::Bye);
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
                    if let Ok(msg) = ServerMsg::decode(&buf[..n]) {
                        self.on_message(msg, now, &mut events);
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue, // Windows: an earlier send bounced
                Err(_) => break,
            }
        }
        match self.state {
            ConnState::Connecting | ConnState::Reconnecting => {
                if self.last_hello.is_none_or(|t| now.duration_since(t) >= self.hello_interval) {
                    self.send_hello(now);
                }
            }
            ConnState::Connected if now.duration_since(self.last_heard) > self.timeout => {
                self.state = ConnState::Reconnecting;
                self.last_hello = None;
                events.push(NetEvent::Disconnected);
            }
            _ => {}
        }
        events
    }

    fn on_message(&mut self, msg: ServerMsg, now: Instant, events: &mut Vec<NetEvent>) {
        self.last_heard = now;
        match msg {
            ServerMsg::Welcome(w) => {
                let already = self.state == ConnState::Connected && self.welcome.is_some_and(|o| o.player_id == w.player_id && o.token == w.token);
                if already {
                    return; // a duplicate answer to a retransmitted Hello
                }
                self.welcome = Some(w);
                self.token = w.token;
                self.state = ConnState::Connected;
                self.latest_snapshot_seq = 0;
                self.recent_inputs.clear();
                self.world.reset();
                self.stats.connects += 1;
                events.push(NetEvent::Connected(w));
            }
            ServerMsg::Reject(r) => {
                self.state = ConnState::Rejected(r);
                events.push(NetEvent::Rejected(r));
            }
            ServerMsg::Snapshot(s) => {
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
                if s.echo_time_ms != 0 || s.ack_input_seq != 0 {
                    let now_ms = now.duration_since(self.started).as_millis() as i64;
                    let rtt = (now_ms - s.echo_time_ms as i64 - s.echo_hold_ms as i64).max(0) as f32;
                    self.stats.rtt_ms = if self.stats.rtt_ms == 0.0 { rtt } else { self.stats.rtt_ms * 0.9 + rtt * 0.1 };
                }
                let me = self.my_id();
                let own = s.players.iter().find(|p| Some(p.id) == me).copied();
                events.push(NetEvent::Snapshot { own, ack_input_seq: s.ack_input_seq });
            }
            ServerMsg::Bye => {
                // The server forgot us (restart / timeout) or is closing: rejoin with our token.
                if self.state == ConnState::Connected {
                    self.state = ConnState::Reconnecting;
                    self.last_hello = None;
                }
                events.push(NetEvent::ServerBye);
            }
        }
    }
}

/// Seconds per simulation tick (what a client paces its input sending by).
pub const TICK_SECS: f64 = TICK_DT as f64;
