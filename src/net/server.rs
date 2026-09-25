//! The authoritative server: UDP sockets and sessions around a [`MatchSim`].
//!
//! Loop: drain the socket ([`Server::pump`]), run due ticks ([`Server::tick`]); [`Server::run`] does
//! both on a fixed 60 Hz schedule. Each tick the sim advances; every `snapshot_every` ticks each
//! client gets a snapshot of every player plus the props that changed *since that client last
//! acknowledged* (per-client change cursors over `sim::change`), so a lost packet costs a resend of
//! a delta, never a desync. A client silent for [`ServerConfig::client_timeout`] is dropped; its
//! player is remembered for [`ServerConfig::resume_grace`] so a returning client (same token) gets
//! back where it was. Clients only ever send *inputs*; nothing a client says can set a position.

use super::limits::{TokenBucket, HELLOS_PER_SEC, HELLO_BURST, MAX_PARKED};
use super::sessions::{Parked, Session, TokenSource};
use super::snapshots::{player_snaps, props_to_send, room_of_player, visible_players};
use crate::net::protocol::*;
use crate::sim::clock::TICK_RATE_HZ;
use crate::sim::interest::InterestMap;
use crate::sim::match_sim::MatchSim;
use crate::sim::player::PlayerState;
use glam::Vec3;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Server settings.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to listen on (`127.0.0.1:0` picks a free port for tests).
    pub bind: SocketAddr,
    /// A snapshot goes out every this many ticks (2 = 30 Hz).
    pub snapshot_every: u8,
    /// A client silent this long is dropped.
    pub client_timeout: Duration,
    /// A dropped player can be resumed with its token for this long.
    pub resume_grace: Duration,
    /// Hash of the loaded map; joins with a different hash are rejected.
    pub map_hash: u32,
    /// Print a stats line this often (`None` = never).
    pub stats_every: Option<Duration>,
}

impl ServerConfig {
    /// Sensible defaults for a LAN/loopback match.
    pub fn new(bind: SocketAddr, map_hash: u32) -> Self {
        ServerConfig {
            bind,
            snapshot_every: 2,
            client_timeout: Duration::from_millis(3000),
            resume_grace: Duration::from_secs(30),
            map_hash,
            stats_every: None,
        }
    }
}

/// Counters for instrumentation and tests.
#[derive(Debug, Clone, Default)]
pub struct ServerStats {
    /// Ticks run.
    pub ticks: u64,
    /// Sum of sim tick durations, microseconds.
    pub tick_us_total: u64,
    /// Longest single sim tick, microseconds.
    pub tick_us_worst: u64,
    /// Snapshots sent.
    pub snapshots_sent: u64,
    /// Datagrams received (valid or not).
    pub packets_in: u64,
    /// Bytes received.
    pub bytes_in: u64,
    /// Bytes sent.
    pub bytes_out: u64,
    /// Moving-prop records sent in snapshots (with interest management on, far rooms cost nothing).
    pub props_sent: u64,
    /// Player records sent in snapshots.
    pub players_sent: u64,
    /// Datagrams that failed to decode or were oversized.
    pub bad_packets: u64,
    /// Input packets dropped because their session exceeded its packet budget (a flood).
    pub rate_limited: u64,
    /// Hellos dropped because the server-wide join budget was spent (a join flood).
    pub hellos_throttled: u64,
    /// Players joined (fresh).
    pub joins: u64,
    /// Players who resumed with a token.
    pub resumes: u64,
    /// Players who left (Bye or timeout).
    pub leaves: u64,
    /// Of those, timeouts.
    pub timeouts: u64,
}

/// A prop the server nudges periodically so there is always an authoritative moving prop to watch.
struct DemoKick {
    prop: usize,
    dir: f32,
}

/// The server. See the module docs.
pub struct Server {
    socket: UdpSocket,
    cfg: ServerConfig,
    sim: MatchSim,
    sessions: Vec<Session>,
    parked: Vec<Parked>,
    stats: ServerStats,
    kick: Option<DemoKick>,
    out: Vec<u8>,
    changed: Vec<(crate::sim::change::Generation, usize)>,
    players_scratch: Vec<PlayerSnap>,
    visible_scratch: Vec<PlayerSnap>,
    props_scratch: Vec<PropSnap>,
    sent_scratch: Vec<(usize, crate::sim::change::Generation)>,
    interest: Option<InterestMap>,
    tokens: TokenSource,
    hello_bucket: TokenBucket,
    started: Instant,
    log: Box<dyn FnMut(&str) + Send>,
}

impl Server {
    /// Binds the socket and takes ownership of the world.
    pub fn bind(cfg: ServerConfig, sim: MatchSim) -> io::Result<Server> {
        let socket = UdpSocket::bind(cfg.bind)?;
        socket.set_nonblocking(true)?;
        Ok(Server {
            socket,
            cfg,
            sim,
            sessions: Vec::new(),
            parked: Vec::new(),
            stats: ServerStats::default(),
            kick: None,
            out: Vec::with_capacity(MAX_PACKET),
            changed: Vec::new(),
            players_scratch: Vec::new(),
            visible_scratch: Vec::new(),
            props_scratch: Vec::new(),
            sent_scratch: Vec::new(),
            interest: None,
            tokens: TokenSource::new(),
            hello_bucket: TokenBucket::new(HELLOS_PER_SEC, HELLO_BURST, Instant::now()),
            started: Instant::now(),
            log: Box::new(|s| println!("{s}")),
        })
    }

    /// Replaces where log lines go (tests silence it or collect it).
    pub fn set_logger(&mut self, f: impl FnMut(&str) + Send + 'static) {
        self.log = Box::new(f);
    }

    /// The address actually bound (useful with port 0).
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// The authoritative world (read-only).
    pub fn sim(&self) -> &MatchSim {
        &self.sim
    }

    /// Counters.
    pub fn stats(&self) -> &ServerStats {
        &self.stats
    }

    /// Dropped players still remembered for resume (bounded by `limits::MAX_PARKED`).
    pub fn parked_count(&self) -> usize {
        self.parked.len()
    }

    /// Turns on spatial interest management: each client is sent only the players and moving props in its own room and the
    /// rooms within `hops` open portals (see `sim::interest`). Off by default; `red_server` turns it on when the map has zones.
    pub fn set_interest(&mut self, map: Option<InterestMap>) {
        self.interest = map;
    }

    /// Starts recording the match into a [`Trace`](crate::sim::trace::Trace) (before the first tick).
    pub fn start_recording(&mut self, header: crate::sim::trace::Header) -> Result<(), String> {
        self.sim.start_recording(header)
    }

    /// Finishes recording; `None` if it never started.
    pub fn take_trace(&mut self) -> Option<crate::sim::trace::Trace> {
        self.sim.take_trace()
    }

    /// Number of connected clients.
    pub fn client_count(&self) -> usize {
        self.sessions.len()
    }

    /// Every 3 seconds, shove prop `prop` along +X then -X, so a moving authoritative prop exists even
    /// when nobody is touching anything. `prop` is an index into `physics::loose_props`.
    pub fn set_demo_kick(&mut self, prop: usize) {
        self.kick = Some(DemoKick { prop, dir: 1.0 });
    }

    fn say(&mut self, s: String) {
        let t = self.started.elapsed().as_secs_f32();
        (self.log)(&format!("[{t:8.2}s] {s}"));
    }

    fn send(&mut self, to: SocketAddr, msg: &ServerMsg) {
        self.out.clear();
        msg.encode(&mut self.out);
        match self.socket.send_to(&self.out, to) {
            Ok(n) => self.stats.bytes_out += n as u64,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => {} // unreachable peer: the timeout will collect it
        }
    }

    /// Reads every datagram waiting on the socket and acts on it.
    pub fn pump(&mut self, now: Instant) {
        let mut buf = [0u8; 2048];
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((n, addr)) => {
                    self.stats.packets_in += 1;
                    self.stats.bytes_in += n as u64;
                    if n > MAX_PACKET {
                        self.stats.bad_packets += 1;
                        continue;
                    }
                    self.handle(addr, &buf[..n], now);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                // Windows reports a previous send to a closed port as an error on the *next* receive.
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(_) => break,
            }
        }
    }

    fn handle(&mut self, addr: SocketAddr, bytes: &[u8], now: Instant) {
        let msg = match ClientMsg::decode(bytes) {
            Ok(m) => m,
            Err(_) => {
                self.stats.bad_packets += 1;
                return;
            }
        };
        match msg {
            ClientMsg::Hello(h) => self.on_hello(addr, h, now),
            ClientMsg::Input(p) => match self.sessions.iter().position(|s| s.addr == addr) {
                Some(i) => {
                    let (slot, s) = (self.sessions[i].slot, &mut self.sessions[i]);
                    s.last_heard = now;
                    if !s.input_bucket.allow(now) {
                        self.stats.rate_limited += 1;
                        return;
                    }
                    s.last_client_time_ms = p.client_time_ms;
                    s.last_client_packet_at = now;
                    // Acknowledge: every prop pose that snapshot carried is now confirmed for this client.
                    let sent = &s.sent[p.snapshot_ack as usize % 64];
                    if sent.seq == p.snapshot_ack && p.snapshot_ack != 0 {
                        for &(entity, gen) in &sent.props {
                            if s.known.len() <= entity {
                                s.known.resize(entity + 1, crate::sim::change::Generation(0));
                            }
                            s.known[entity] = s.known[entity].max(gen);
                        }
                    }
                    for input in p.inputs {
                        self.sim.push_input(slot, input);
                    }
                }
                // Someone we do not know (the server restarted, or we timed them out): tell them.
                None => self.send(addr, &ServerMsg::Bye),
            },
            ClientMsg::Bye => {
                if let Some(i) = self.sessions.iter().position(|s| s.addr == addr) {
                    self.drop_session(i, now, false);
                }
            }
        }
    }

    /// The Welcome for `s`, or `None` if its player no longer exists (the caller then drops the session).
    fn welcome_for(&self, s: &Session) -> Option<Welcome> {
        let p = self.sim.player(s.slot)?.state;
        Some(Welcome {
            player_id: s.slot as u8,
            token: s.token,
            tick_rate: TICK_RATE_HZ as u16,
            snapshot_every: self.cfg.snapshot_every,
            server_tick: self.sim.tick() as u32,
            spawn: [p.pos.x, p.foot_y, p.pos.y, p.yaw],
            character: character_to_wire(p.character),
        })
    }

    fn on_hello(&mut self, addr: SocketAddr, h: Hello, now: Instant) {
        // A join flood (spoofed sources filling the match with ghosts) is cut off before it costs a reply.
        if !self.hello_bucket.allow(now) {
            self.stats.hellos_throttled += 1;
            return;
        }
        if h.version != PROTOCOL_VERSION {
            return self.send(addr, &ServerMsg::Reject(RejectReason::Version));
        }
        if h.map_hash != self.cfg.map_hash {
            return self.send(addr, &ServerMsg::Reject(RejectReason::WrongMap));
        }
        // A retransmitted Hello from a client we already have: answer again, change nothing.
        if let Some(i) = self.sessions.iter().position(|s| s.addr == addr) {
            self.sessions[i].last_heard = now;
            return match self.welcome_for(&self.sessions[i]) {
                Some(w) => self.send(addr, &ServerMsg::Welcome(w)),
                None => self.drop_session(i, now, false), // its player vanished: free the slot; the client re-joins
            };
        }
        // Resume: the token of a recently dropped player, or of a live session from another address
        // (the client came back from a new port before we noticed the old one had died).
        let mut resumed: Option<(u64, PlayerState)> = None;
        if h.resume_token != 0 {
            if let Some(k) = self.parked.iter().position(|p| p.token == h.resume_token && p.expires > now) {
                let p = self.parked.remove(k);
                resumed = Some((p.token, p.state));
            } else if let Some(i) = self.sessions.iter().position(|s| s.token == h.resume_token) {
                let token = self.sessions[i].token;
                if let Some(state) = self.sim.player(self.sessions[i].slot).map(|p| p.state) {
                    self.drop_session(i, now, false);
                    resumed = Some((token, state));
                }
            }
        }
        let (token, slot, fresh) = match resumed {
            Some((token, state)) => match self.sim.add_player_with(state) {
                Some(slot) => (token, slot, false),
                None => return self.send(addr, &ServerMsg::Reject(RejectReason::Full)),
            },
            None => match self.sim.add_player(character_from_wire(h.character)) {
                Some(slot) => (self.tokens.next(), slot, true),
                None => return self.send(addr, &ServerMsg::Reject(RejectReason::Full)),
            },
        };
        let session = Session::new(addr, slot, token, now);
        let Some(w) = self.welcome_for(&session) else {
            self.sim.remove_player(slot);
            return;
        };
        self.sessions.push(session);
        if fresh {
            self.stats.joins += 1;
        } else {
            self.stats.resumes += 1;
        }
        let who = if w.character == 1 { "rat" } else { "human" };
        self.say(format!("{} {addr} as player {slot} ({who}); {} connected", if fresh { "join" } else { "RESUME" }, self.sessions.len()));
        self.send(addr, &ServerMsg::Welcome(w));
    }

    fn drop_session(&mut self, index: usize, now: Instant, timed_out: bool) {
        let s = self.sessions.remove(index);
        if let Some(state) = self.sim.remove_player(s.slot) {
            self.parked.push(Parked { token: s.token, state, expires: now + self.cfg.resume_grace });
            if self.parked.len() > MAX_PARKED {
                self.parked.remove(0); // a join/leave flood cannot grow this list; the oldest resume is forgotten
            }
        }
        self.stats.leaves += 1;
        if timed_out {
            self.stats.timeouts += 1;
        }
        self.say(format!("{} {} (player {}); {} connected", if timed_out { "timeout" } else { "leave" }, s.addr, s.slot, self.sessions.len()));
    }

    /// Runs one simulation tick and does everything due after it: timeouts, the demo kick, snapshots.
    pub fn tick(&mut self, now: Instant) {
        let t0 = Instant::now();
        self.sim.tick_once();
        for e in self.sim.take_events() {
            let who = e.slot.map(|s| format!(" (player {s})")).unwrap_or_default();
            self.say(format!("event {} by rule {}{who} at tick {}", e.name, e.rule, e.tick));
        }
        let us = t0.elapsed().as_micros() as u64;
        self.stats.ticks += 1;
        self.stats.tick_us_total += us;
        self.stats.tick_us_worst = self.stats.tick_us_worst.max(us);

        let timeout = self.cfg.client_timeout;
        while let Some(i) = self.sessions.iter().position(|s| now.duration_since(s.last_heard) > timeout) {
            self.drop_session(i, now, true);
        }
        self.parked.retain(|p| p.expires > now);

        if let Some(k) = &mut self.kick {
            if self.sim.tick().is_multiple_of(3 * TICK_RATE_HZ as u64) {
                let prop = k.prop;
                let dir = k.dir;
                k.dir = -k.dir;
                let at = self.sim.props().prop_pose(prop).w_axis.truncate();
                // Proportional to the prop's mass: a 4 m/s shove, so a crate and a barrel both really travel.
                let impulse = self.sim.props().mass(prop) * 4.0;
                self.sim.apply_impulse(prop, Vec3::new(dir, 0.0, 0.0), at + Vec3::Y * 0.2, impulse);
            }
        }
        if self.sim.tick().is_multiple_of(self.cfg.snapshot_every.max(1) as u64) {
            self.send_snapshots(now);
        }
        if let Some(every) = self.cfg.stats_every {
            let secs = self.started.elapsed().as_secs_f64();
            if self.sim.tick().is_multiple_of(((every.as_secs_f64() * TICK_RATE_HZ as f64) as u64).max(1)) {
                let s = &self.stats;
                let line = format!(
                    "stats: players {} | tick avg {:.0} us worst {} us | promoted props {} | in {:.1} KB/s out {:.1} KB/s | snapshots {} | bad packets {}",
                    self.sessions.len(),
                    s.tick_us_total as f64 / s.ticks.max(1) as f64,
                    s.tick_us_worst,
                    self.sim.props().dynamic_count(),
                    s.bytes_in as f64 / 1024.0 / secs,
                    s.bytes_out as f64 / 1024.0 / secs,
                    s.snapshots_sent,
                    s.bad_packets
                );
                self.say(line);
            }
        }
    }

    fn send_snapshots(&mut self, now: Instant) {
        player_snaps(&self.sim, &mut self.players_scratch);
        for i in 0..self.sessions.len() {
            let slot = self.sessions[i].slot;
            let room = room_of_player(&self.sim, self.interest.as_ref(), slot);
            visible_players(&self.sim, self.interest.as_ref(), slot, &self.players_scratch, &mut self.visible_scratch);
            let s = &mut self.sessions[i];
            props_to_send(&self.sim, self.interest.as_ref(), room, &s.known, &mut self.changed, &mut self.props_scratch, &mut self.sent_scratch);
            s.snapshot_seq = s.snapshot_seq.wrapping_add(1);
            let seq = s.snapshot_seq;
            let entry = &mut s.sent[seq as usize % 64];
            entry.seq = seq;
            entry.props.clear();
            entry.props.extend_from_slice(&self.sent_scratch);
            let snap = Snapshot {
                seq,
                server_tick: self.sim.tick() as u32,
                ack_input_seq: self.sim.player(slot).map_or(0, |p| p.last_processed_seq),
                echo_time_ms: s.last_client_time_ms,
                echo_hold_ms: now.duration_since(s.last_client_packet_at).as_millis().min(65_535) as u16,
                players: std::mem::take(&mut self.visible_scratch),
                props: std::mem::take(&mut self.props_scratch),
            };
            let addr = s.addr;
            self.stats.players_sent += snap.players.len() as u64;
            self.stats.props_sent += snap.props.len() as u64;
            let msg = ServerMsg::Snapshot(snap);
            self.send(addr, &msg);
            if let ServerMsg::Snapshot(snap) = msg {
                // Take the buffers back so the next client reuses their allocations.
                (self.visible_scratch, self.props_scratch) = (snap.players, snap.props);
            }
            self.stats.snapshots_sent += 1;
        }
        // Close the generation: writes from here on are strictly newer than every snapshot just sent.
        self.sim.props_mut().clock_mut().advance();
    }

    /// Runs the server on a fixed schedule until `stop` is set: drain the socket, run every tick that
    /// is due (catching up after a stall, at most 8 in a row), sleep in short slices.
    pub fn run(&mut self, stop: &AtomicBool) {
        let tick = Duration::from_secs_f64(1.0 / TICK_RATE_HZ as f64);
        let mut next = Instant::now() + tick;
        while !stop.load(Ordering::Relaxed) {
            let now = Instant::now();
            self.pump(now);
            let mut ran = 0;
            while now >= next && ran < 8 {
                self.tick(now);
                next += tick;
                ran += 1;
            }
            if ran == 8 {
                next = Instant::now() + tick;
            }
            let wait = next.saturating_duration_since(Instant::now());
            if wait > Duration::from_micros(1500) {
                std::thread::sleep(Duration::from_micros(500));
            } else {
                std::thread::yield_now();
            }
        }
        // Tell everyone, so clients know at once instead of waiting for a timeout.
        let addrs: Vec<SocketAddr> = self.sessions.iter().map(|s| s.addr).collect();
        for a in addrs {
            self.send(a, &ServerMsg::Bye);
        }
    }
}

/// Asks Windows for a 1 ms timer so `sleep` in [`Server::run`] does not oversleep by ~15 ms. A no-op elsewhere.
pub fn raise_timer_resolution() {
    #[cfg(windows)]
    {
        #[link(name = "winmm")]
        extern "system" {
            fn timeBeginPeriod(period: u32) -> u32;
        }
        // SAFETY: plain FFI call with a valid argument; it only adjusts the process timer resolution.
        unsafe {
            timeBeginPeriod(1);
        }
    }
}
