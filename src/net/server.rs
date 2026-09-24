//! The authoritative server: UDP sockets and sessions around a [`MatchSim`].
//!
//! Loop: drain the socket ([`Server::pump`]), run due ticks ([`Server::tick`]); [`Server::run`] does
//! both on a fixed 60 Hz schedule. Each tick the sim advances; every `snapshot_every` ticks each
//! client gets a snapshot of every player plus the props that changed *since that client last
//! acknowledged* (per-client change cursors over `sim::change`), so a lost packet costs a resend of
//! a delta, never a desync. A client silent for [`ServerConfig::client_timeout`] is dropped; its
//! player is remembered for [`ServerConfig::resume_grace`] so a returning client (same token) gets
//! back where it was. Clients only ever send *inputs*; nothing a client says can set a position.

use crate::net::protocol::*;
use crate::player::Character;
use crate::sim::change::Generation;
use crate::sim::clock::TICK_RATE_HZ;
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
        ServerConfig { bind, snapshot_every: 2, client_timeout: Duration::from_millis(3000), resume_grace: Duration::from_secs(30), map_hash, stats_every: None }
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
    /// Datagrams that failed to decode or were oversized.
    pub bad_packets: u64,
    /// Players joined (fresh).
    pub joins: u64,
    /// Players who resumed with a token.
    pub resumes: u64,
    /// Players who left (Bye or timeout).
    pub leaves: u64,
    /// Of those, timeouts.
    pub timeouts: u64,
}

#[derive(Clone, Copy)]
struct SentSnap {
    seq: u32,
    gen: Generation,
    complete: bool,
}

struct Session {
    addr: SocketAddr,
    slot: usize,
    token: u64,
    last_heard: Instant,
    snapshot_seq: u32,
    /// Everything stamped at or before this generation is known to have reached the client.
    acked_gen: Generation,
    sent: [Option<SentSnap>; 64],
    last_client_time_ms: u32,
    last_client_packet_at: Instant,
}

struct Parked {
    token: u64,
    state: PlayerState,
    expires: Instant,
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
    changed: Vec<usize>,
    token_counter: u64,
    started: Instant,
    log: Box<dyn FnMut(&str) + Send>,
}

fn char_to_u8(c: Character) -> u8 {
    match c {
        Character::Human => 0,
        Character::Rat => 1,
    }
}

/// `0` human, `1` rat (anything else is a human).
pub fn char_from_u8(v: u8) -> Character {
    if v == 1 {
        Character::Rat
    } else {
        Character::Human
    }
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
            token_counter: 0,
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

    fn new_token(&mut self) -> u64 {
        self.token_counter += 1;
        // Not cryptographic: it only has to be unguessable by accident and distinct per player.
        let mut x = self.token_counter ^ (self.started.elapsed().as_nanos() as u64) ^ 0x9e37_79b9_7f4a_7c15;
        x ^= x >> 30;
        x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^= x >> 31;
        x.max(1)
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
                    s.last_client_time_ms = p.client_time_ms;
                    s.last_client_packet_at = now;
                    // Acknowledge: if that snapshot was complete, everything stamped up to it arrived.
                    if let Some(sent) = s.sent[p.snapshot_ack as usize % 64].filter(|x| x.seq == p.snapshot_ack) {
                        if sent.complete && sent.gen > s.acked_gen {
                            s.acked_gen = sent.gen;
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

    fn welcome_for(&self, s: &Session) -> Welcome {
        let p = self.sim.player(s.slot).expect("session without a player").state;
        Welcome {
            player_id: s.slot as u8,
            token: s.token,
            tick_rate: TICK_RATE_HZ as u16,
            snapshot_every: self.cfg.snapshot_every,
            server_tick: self.sim.tick() as u32,
            spawn: [p.pos.x, p.foot_y, p.pos.y, p.yaw],
            character: char_to_u8(p.character),
        }
    }

    fn on_hello(&mut self, addr: SocketAddr, h: Hello, now: Instant) {
        if h.version != PROTOCOL_VERSION {
            return self.send(addr, &ServerMsg::Reject(RejectReason::Version));
        }
        if h.map_hash != self.cfg.map_hash {
            return self.send(addr, &ServerMsg::Reject(RejectReason::WrongMap));
        }
        // A retransmitted Hello from a client we already have: answer again, change nothing.
        if let Some(i) = self.sessions.iter().position(|s| s.addr == addr) {
            self.sessions[i].last_heard = now;
            let w = self.welcome_for(&self.sessions[i]);
            return self.send(addr, &ServerMsg::Welcome(w));
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
            None => match self.sim.add_player(char_from_u8(h.character)) {
                Some(slot) => (self.new_token(), slot, true),
                None => return self.send(addr, &ServerMsg::Reject(RejectReason::Full)),
            },
        };
        let session = Session {
            addr,
            slot,
            token,
            last_heard: now,
            snapshot_seq: 0,
            acked_gen: Generation(0),
            sent: [None; 64],
            last_client_time_ms: 0,
            last_client_packet_at: now,
        };
        let w = self.welcome_for(&session);
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
                self.sim.props_mut().strike_impulse(prop, Vec3::new(dir, 0.0, 0.0), at + Vec3::Y * 0.2, impulse);
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
        let players: Vec<PlayerSnap> = self
            .sim
            .players()
            .map(|(slot, p)| PlayerSnap {
                id: slot as u8,
                character: char_to_u8(p.state.character),
                flags: p.crouching as u8,
                pos: [p.state.pos.x, p.state.foot_y, p.state.pos.y],
                yaw: p.state.yaw,
                pitch: p.state.pitch,
                speed: p.speed,
                vy: p.state.vy,
            })
            .collect();
        let gen_now = self.sim.props_mut().clock_mut().now();
        for i in 0..self.sessions.len() {
            let (slot, cursor) = (self.sessions[i].slot, self.sessions[i].acked_gen);
            self.changed.clear();
            self.changed.extend(self.sim.props().entities().transforms.iter_changed_since(cursor).map(|(entity_slot, _)| entity_slot));
            let total = self.changed.len();
            let complete = total <= MAX_PROPS_PER_SNAPSHOT;
            // Too many changed to fit: send a rotating window, and do NOT advance the cursor (the rest
            // stays "changed since" and comes in later snapshots).
            let (start, take) = if complete { (0, total) } else { ((self.sessions[i].snapshot_seq as usize * MAX_PROPS_PER_SNAPSHOT) % total, MAX_PROPS_PER_SNAPSHOT) };
            let props: Vec<PropSnap> = (0..take)
                .map(|k| {
                    let entity_slot = self.changed[(start + k) % total];
                    let t = self.sim.props().entities().transforms.get(entity_slot);
                    PropSnap { id: self.sim.props().prop_of_entity(entity_slot) as u16, pos: t.position.to_array(), rot: t.rotation.to_array() }
                })
                .collect();
            let s = &mut self.sessions[i];
            s.snapshot_seq += 1;
            let seq = s.snapshot_seq;
            s.sent[seq as usize % 64] = Some(SentSnap { seq, gen: gen_now, complete });
            let snap = Snapshot {
                seq,
                server_tick: self.sim.tick() as u32,
                ack_input_seq: self.sim.player(slot).map_or(0, |p| p.last_processed_seq),
                echo_time_ms: s.last_client_time_ms,
                echo_hold_ms: now.duration_since(s.last_client_packet_at).as_millis().min(65_535) as u16,
                players: players.clone(),
                props,
            };
            let addr = s.addr;
            self.send(addr, &ServerMsg::Snapshot(snap));
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
