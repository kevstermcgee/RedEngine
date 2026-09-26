//! The authoritative server: UDP sockets and sessions around a [`MatchSim`].
//!
//! Loop: drain the socket ([`Server::pump`]), run due ticks ([`Server::tick`]); [`Server::run`] does
//! both on a fixed 60 Hz schedule. Each tick the sim advances; every `snapshot_every` ticks each
//! client gets a snapshot of every player plus the props that changed *since that client last
//! acknowledged* (per-client change cursors over `sim::change`), so a lost packet costs a resend of
//! a delta, never a desync. A client silent for [`ServerConfig::client_timeout`] is dropped; its
//! player is remembered for [`ServerConfig::resume_grace`] so a returning client (same token) gets
//! back where it was. Clients only ever send *inputs*; nothing a client says can set a position.
//!
//! **Joining** is a challenge/response (ADR 0028): a `Hello` without a valid address cookie is answered with a `Challenge`; the second
//! `Hello` carries the cookie and, on a server started with a join key, a proof of the key. Every datagram after that is tagged with a
//! per-session HMAC and dropped unread if the tag is wrong ([`super::auth`]).
//!
//! **The match flow** (ADR 0029): with [`Server::enable_flow`] the server runs lobby → countdown → round → results → rematch
//! ([`crate::sim::flow`]). The world is rebuilt from a factory at each countdown, so a rematch starts from the authored map; players keep
//! their id across rounds. Without it the server is in *open play*: join = play, exactly as before.

use super::auth::{proof_matches, CookieJar, Direction, SessionKey};
use super::limits::{TokenBucket, HELLOS_PER_SEC, HELLO_BURST, MAX_PARKED};
use super::sessions::{Parked, Session, TokenSource};
use super::snapshots::{player_snaps, props_to_send, room_of_player, visible_players};
use crate::net::protocol::*;
use crate::sim::clock::TICK_RATE_HZ;
use crate::sim::flow::{EndReason, Flow, FlowEvent, FlowInput, MatchSettings, Phase};
use crate::sim::interest::InterestMap;
use crate::sim::match_sim::{MatchSim, MAX_PLAYERS};
use crate::sim::trace::{Header, Trace};
use glam::Vec3;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A `Status` goes to every client this often (5 Hz), and at once when something changes.
const STATUS_EVERY_TICKS: u64 = (TICK_RATE_HZ / 5) as u64;

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
    /// The join key clients must prove they know (`None` = an open server).
    pub join_key: Option<String>,
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
            join_key: None,
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
    /// Datagrams from a known session whose authentication tag was wrong (forged, corrupted or from another session).
    pub bad_tags: u64,
    /// Input packets dropped because their session exceeded its packet budget (a flood).
    pub rate_limited: u64,
    /// Hellos dropped because the server-wide join budget was spent (a join flood).
    pub hellos_throttled: u64,
    /// Challenges sent (each first Hello of a join attempt).
    pub challenges: u64,
    /// Joins refused for a wrong join key.
    pub bad_keys: u64,
    /// Players joined (fresh).
    pub joins: u64,
    /// Players who resumed with a token.
    pub resumes: u64,
    /// Players who left (Bye or timeout).
    pub leaves: u64,
    /// Of those, timeouts.
    pub timeouts: u64,
    /// Rounds finished (match flow only).
    pub rounds: u64,
    /// `Status` messages sent.
    pub statuses_sent: u64,
    /// Complete rule-state messages sent.
    pub rule_states_sent: u64,
}

/// A prop the server nudges periodically so there is always an authoritative moving prop to watch.
struct DemoKick {
    prop: usize,
    dir: f32,
}

/// What happened in a finished round (handed to [`Server::set_round_hook`]).
#[derive(Debug, Clone)]
pub struct RoundRecord {
    /// The round number.
    pub round: u16,
    /// Why it ended.
    pub reason: EndReason,
    /// The winner's player id: the unique highest score above zero.
    pub winner: Option<u8>,
    /// `(player id, kills)` for everyone who was in the round.
    pub scores: Vec<(u8, u32)>,
    /// The recorded trace of the round, when recording is on.
    pub trace: Option<Trace>,
}

/// Builds a fresh world for a round.
pub type RoundFactory = Box<dyn FnMut() -> Result<MatchSim, String> + Send>;

/// The result the last `Status` carries.
#[derive(Debug, Clone)]
struct LastResult {
    winner: u8,
    end_code: u8,
    end_text: String,
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
    cookies: CookieJar,
    hello_bucket: TokenBucket,
    started: Instant,
    log: Box<dyn FnMut(&str) + Send>,
    flow: Option<Flow>,
    factory: Option<RoundFactory>,
    round_header: Option<Header>,
    round_hook: Option<Box<dyn FnMut(RoundRecord) + Send>>,
    last_result: Option<LastResult>,
    status_dirty: bool,
}

impl Server {
    /// Binds the socket and takes ownership of the world.
    pub fn bind(cfg: ServerConfig, sim: MatchSim) -> io::Result<Server> {
        let socket = UdpSocket::bind(cfg.bind)?;
        socket.set_nonblocking(true)?;
        let entropy = RandomState::new();
        let mut counter = 0u64;
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
            cookies: CookieJar::new(move || {
                counter = counter.wrapping_add(1);
                entropy.hash_one(counter)
            }),
            hello_bucket: TokenBucket::new(HELLOS_PER_SEC, HELLO_BURST, Instant::now()),
            started: Instant::now(),
            log: Box::new(|s| println!("{s}")),
            flow: None,
            factory: None,
            round_header: None,
            round_hook: None,
            last_result: None,
            status_dirty: false,
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

    /// Starts recording the match into a [`Trace`](crate::sim::trace::Trace) (before the first tick). For open play; a match flow
    /// records each round instead ([`Server::set_round_recording`]).
    pub fn start_recording(&mut self, header: Header) -> Result<(), String> {
        self.sim.start_recording(header)
    }

    /// Finishes recording; `None` if it never started.
    pub fn take_trace(&mut self) -> Option<Trace> {
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

    /// Turns on the match flow: lobby, countdown, rounds, results, rematch (see [`crate::sim::flow`]). `factory` builds a fresh world for
    /// each round; it is called once now to prove it works, and that world replaces the current one.
    pub fn enable_flow(&mut self, settings: MatchSettings, mut factory: impl FnMut() -> Result<MatchSim, String> + Send + 'static) -> Result<(), String> {
        if !self.sessions.is_empty() {
            return Err("enable the match flow before anyone connects".to_string());
        }
        self.sim = factory()?;
        self.factory = Some(Box::new(factory));
        self.flow = Some(Flow::new(settings));
        self.status_dirty = true;
        Ok(())
    }

    /// Records every round of the match flow with this header (the trace arrives in the [`RoundRecord`]).
    pub fn set_round_recording(&mut self, header: Header) {
        self.round_header = Some(header);
    }

    /// Calls `f` with the result of every finished round.
    pub fn set_round_hook(&mut self, f: impl FnMut(RoundRecord) + Send + 'static) {
        self.round_hook = Some(Box::new(f));
    }

    /// The lobby / round phase (`Playing` in open play).
    pub fn phase(&self) -> Phase {
        self.flow.as_ref().map_or(Phase::Playing, Flow::phase)
    }

    /// The current round (`0` in open play and before the first countdown).
    pub fn round(&self) -> u16 {
        self.flow.as_ref().map_or(0, Flow::round)
    }

    fn say(&mut self, s: String) {
        let t = self.started.elapsed().as_secs_f32();
        (self.log)(&format!("[{t:8.2}s] {s}"));
    }

    fn raw_send(&mut self, to: SocketAddr) {
        match self.socket.send_to(&self.out, to) {
            Ok(n) => self.stats.bytes_out += n as u64,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => {} // unreachable peer: the timeout will collect it
        }
    }

    /// Sends a message that carries no tag (`Challenge`, `Reject`, `NoSession`).
    fn send_plain(&mut self, to: SocketAddr, msg: &ServerMsg) {
        self.out.clear();
        msg.encode(&mut self.out);
        self.raw_send(to);
    }

    /// Sends a tagged message to session `i`.
    fn send_signed(&mut self, i: usize, msg: &ServerMsg) {
        self.out.clear();
        msg.encode(&mut self.out);
        let s = &self.sessions[i];
        s.key.sign(Direction::ToClient, &mut self.out);
        let addr = s.addr;
        self.raw_send(addr);
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
        let Some(kind) = peek_kind(bytes) else {
            self.stats.bad_packets += 1;
            return;
        };
        if !is_signed(kind) {
            // Only a Hello is legal without a tag.
            match ClientMsg::decode(bytes) {
                Ok(ClientMsg::Hello(h)) => self.on_hello(addr, h, now),
                _ => self.stats.bad_packets += 1,
            }
            return;
        }
        let Some(i) = self.sessions.iter().position(|s| s.addr == addr) else {
            // Someone we do not know (the server restarted, or we timed them out): tell them, within the join budget.
            if self.hello_bucket.allow(now) {
                self.send_plain(addr, &ServerMsg::NoSession);
            }
            return;
        };
        let Some(body) = self.sessions[i].key.verify(Direction::ToServer, bytes) else {
            self.stats.bad_tags += 1;
            return;
        };
        let msg = match ClientMsg::decode(body) {
            Ok(m) => m,
            Err(_) => {
                self.stats.bad_packets += 1;
                return;
            }
        };
        self.sessions[i].last_heard = now;
        match msg {
            ClientMsg::Hello(_) => self.stats.bad_packets += 1, // a Hello never carries a tag
            ClientMsg::Input(p) => self.on_input(i, p, now),
            ClientMsg::Lobby(l) => self.on_lobby(i, l, now),
            ClientMsg::Bye => self.drop_session(i, now, false),
        }
    }

    fn accepts_input(&self) -> bool {
        self.phase() == Phase::Playing
    }

    fn on_input(&mut self, i: usize, p: InputPacket, now: Instant) {
        let accepts = self.accepts_input();
        let s = &mut self.sessions[i];
        if !s.input_bucket.allow(now) {
            self.stats.rate_limited += 1;
            return;
        }
        s.last_client_time_ms = p.client_time_ms;
        s.last_client_packet_at = now;
        s.round_ack = s.round_ack.max(p.round_ack);
        s.rtt_ms = p.rtt_ms.min(9_999);
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
        if s.in_round && accepts {
            let slot = s.slot;
            for input in p.inputs {
                self.sim.push_input(slot, input);
            }
        }
    }

    fn on_lobby(&mut self, i: usize, l: LobbyCmd, now: Instant) {
        let playing = self.phase() == Phase::Playing && self.flow.is_some();
        let s = &mut self.sessions[i];
        if !s.lobby_bucket.allow(now) {
            self.stats.rate_limited += 1;
            return;
        }
        // A lobby packet older than one already seen (a recorded packet replayed, or reordering) must not undo a newer choice.
        if (l.client_time_ms.wrapping_sub(s.last_client_time_ms) as i32) < 0 {
            return;
        }
        s.last_client_time_ms = l.client_time_ms;
        s.last_client_packet_at = now;
        s.round_ack = s.round_ack.max(l.round_ack);
        s.rtt_ms = l.rtt_ms.min(9_999);
        let ready = l.ready && !playing;
        let character = l.character.min(1);
        if ready != s.ready || character != s.character {
            s.ready = ready;
            s.character = character;
            self.status_dirty = true;
        }
    }

    fn current_round(&self) -> u16 {
        self.round()
    }

    /// The Welcome for `s`, or `None` if its player no longer exists (the caller then drops the session).
    fn welcome_for(&self, s: &Session) -> Option<Welcome> {
        let (spawn, character) = if s.in_round {
            let p = self.sim.player(s.slot)?.state;
            ([p.pos.x, p.foot_y, p.pos.y, p.yaw], character_to_wire(p.character))
        } else {
            ([0.0; 4], s.character)
        };
        Some(Welcome {
            player_id: s.slot as u8,
            token: s.token,
            tick_rate: TICK_RATE_HZ as u16,
            snapshot_every: self.cfg.snapshot_every,
            server_tick: self.sim.tick() as u32,
            spawn,
            character,
            round: self.current_round(),
            in_round: s.in_round,
        })
    }

    /// The lowest player id nobody holds. A parked (recently disconnected) player reserves its id so it gets it back on resume, but only
    /// softly: when every free id is reserved, the reservation that would expire soonest is given up, so a crowd of drop-outs cannot lock
    /// newcomers out of the match.
    fn free_slot(&mut self) -> Option<usize> {
        let connected = |sessions: &[Session], slot: usize| sessions.iter().any(|s| s.slot == slot);
        if let Some(slot) = (0..MAX_PLAYERS).find(|slot| !connected(&self.sessions, *slot) && !self.parked.iter().any(|p| p.slot == *slot)) {
            return Some(slot);
        }
        let oldest = self.parked.iter().enumerate().filter(|(_, p)| !connected(&self.sessions, p.slot)).min_by_key(|(_, p)| p.expires).map(|(i, _)| i)?;
        Some(self.parked.remove(oldest).slot)
    }

    fn on_hello(&mut self, addr: SocketAddr, h: Hello, now: Instant) {
        // A join flood (spoofed sources filling the match with ghosts) is cut off before it costs a reply.
        if !self.hello_bucket.allow(now) {
            self.stats.hellos_throttled += 1;
            return;
        }
        if h.version != PROTOCOL_VERSION {
            return self.send_plain(addr, &ServerMsg::Reject(RejectReason::Version));
        }
        // 1. Prove the source address can receive: no valid cookie, no state, just a small Challenge.
        let secs = now.duration_since(self.started).as_secs();
        if !self.cookies.valid(addr, h.client_nonce, h.cookie, secs) {
            self.stats.challenges += 1;
            let cookie = self.cookies.make(addr, h.client_nonce, secs);
            return self.send_plain(addr, &ServerMsg::Challenge { cookie, requires_key: self.cfg.join_key.is_some() });
        }
        // 2. Prove the join key (never sent, only its HMAC).
        let key_bytes = self.cfg.join_key.clone().unwrap_or_default();
        if self.cfg.join_key.is_some() && !proof_matches(key_bytes.as_bytes(), h.client_nonce, h.cookie, h.map_hash, h.version, &h.proof) {
            self.stats.bad_keys += 1;
            return self.send_plain(addr, &ServerMsg::Reject(RejectReason::BadKey));
        }
        if h.map_hash != self.cfg.map_hash {
            return self.send_plain(addr, &ServerMsg::Reject(RejectReason::WrongMap));
        }
        // A retransmitted Hello from a client we already have (same nonce and cookie): answer again, change nothing.
        if let Some(i) = self.sessions.iter().position(|s| s.addr == addr) {
            if self.sessions[i].client_nonce == h.client_nonce && self.sessions[i].cookie == h.cookie {
                self.sessions[i].last_heard = now;
                return match self.welcome_for(&self.sessions[i]) {
                    Some(w) => self.send_signed(i, &ServerMsg::Welcome(w)),
                    None => self.drop_session(i, now, false), // its player vanished: free the slot; the client re-joins
                };
            }
            // The same address with a fresh handshake: the client restarted (or its old handshake expired). It re-joins.
            self.drop_session(i, now, false);
        }
        // Resume: the token of a recently dropped player, or of a live session from another address
        // (the client came back from a new port before we noticed the old one had died).
        if h.resume_token != 0 {
            if let Some(i) = self.sessions.iter().position(|s| s.token == h.resume_token) {
                self.drop_session(i, now, false);
            }
        }
        let resumed = (h.resume_token != 0)
            .then(|| self.parked.iter().position(|p| p.token == h.resume_token && p.expires > now))
            .flatten()
            .map(|k| self.parked.remove(k));
        let Some(slot) = resumed.as_ref().map(|p| p.slot).filter(|s| !self.sessions.iter().any(|x| x.slot == *s)).or_else(|| self.free_slot()) else {
            return self.send_plain(addr, &ServerMsg::Reject(RejectReason::Full));
        };
        let fresh = resumed.is_none();
        let token = resumed.as_ref().map_or_else(|| self.tokens.next(), |p| p.token);
        let character = resumed.as_ref().map_or(h.character.min(1), |p| p.character);
        let name = if h.name.trim().is_empty() { resumed.as_ref().map_or_else(|| sanitize_name(""), |p| p.name.clone()) } else { sanitize_name(&h.name) };
        let key = SessionKey::derive(key_bytes.as_bytes(), h.client_nonce, h.cookie);
        let mut session = Session::new(addr, slot, token, key, h.client_nonce, h.cookie, name, character, now);

        // Does this player get a body right now?
        let joins_world = match &self.flow {
            None => true,
            Some(f) => f.phase() == Phase::Playing && f.settings().join_in_progress,
        };
        if joins_world {
            let round = self.current_round();
            let placed = match resumed.as_ref().and_then(|p| (p.round == round).then_some(p.state).flatten()) {
                Some(state) => self.sim.add_player_at(slot, state),
                None => self.sim.add_player_in_slot(slot, character_from_wire(character)),
            };
            if !placed {
                return self.send_plain(addr, &ServerMsg::Reject(RejectReason::Full));
            }
            session.in_round = true;
        }
        let Some(w) = self.welcome_for(&session) else {
            self.sim.remove_player(slot);
            return;
        };
        session.round_ack = if session.in_round { 0 } else { w.round };
        self.sessions.push(session);
        let last = self.sessions.len() - 1;
        if fresh {
            self.stats.joins += 1;
        } else {
            self.stats.resumes += 1;
        }
        self.status_dirty = true;
        let who = if w.character == 1 { "rat" } else { "human" };
        let name = self.sessions[last].name.clone();
        self.say(format!("{} {addr} as player {slot} '{name}' ({who}); {} connected", if fresh { "join" } else { "RESUME" }, self.sessions.len()));
        self.send_signed(last, &ServerMsg::Welcome(w));
    }

    fn drop_session(&mut self, index: usize, now: Instant, timed_out: bool) {
        let s = self.sessions.remove(index);
        let state = if s.in_round { self.sim.remove_player(s.slot) } else { None };
        let round = self.current_round();
        self.parked.push(Parked {
            token: s.token,
            slot: s.slot,
            state,
            round,
            name: s.name.clone(),
            character: s.character,
            expires: now + self.cfg.resume_grace,
        });
        if self.parked.len() > MAX_PARKED {
            self.parked.remove(0); // a join/leave flood cannot grow this list; the oldest resume is forgotten
        }
        self.stats.leaves += 1;
        if timed_out {
            self.stats.timeouts += 1;
        }
        self.status_dirty = true;
        self.say(format!("{} {} (player {}); {} connected", if timed_out { "timeout" } else { "leave" }, s.addr, s.slot, self.sessions.len()));
    }

    /// Runs one simulation tick and does everything due after it: timeouts, the match flow, the demo kick, snapshots, status.
    pub fn tick(&mut self, now: Instant) {
        let t0 = Instant::now();
        let running = self.accepts_input();
        if running {
            self.sim.tick_once();
            for e in self.sim.take_events() {
                let who = e.slot.map(|s| format!(" (player {s})")).unwrap_or_default();
                self.say(format!("event {} by rule {}{who} at tick {}", e.name, e.rule, e.tick));
            }
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

        self.step_flow();

        if running {
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
        }
        let show_world = self.flow.is_none() || matches!(self.phase(), Phase::Countdown | Phase::Playing);
        if show_world && self.stats.ticks.is_multiple_of(self.cfg.snapshot_every.max(1) as u64) {
            self.send_snapshots(now);
        }
        if self.status_dirty || self.stats.ticks.is_multiple_of(STATUS_EVERY_TICKS) {
            self.send_statuses(now);
            self.status_dirty = false;
        }
        if let Some(every) = self.cfg.stats_every {
            let secs = self.started.elapsed().as_secs_f64();
            if self.stats.ticks.is_multiple_of(((every.as_secs_f64() * TICK_RATE_HZ as f64) as u64).max(1)) {
                let s = &self.stats;
                let line = format!(
                    "stats: players {} | phase {} | tick avg {:.0} us worst {} us | promoted props {} | in {:.1} KB/s out {:.1} KB/s | snapshots {} | bad packets {} bad tags {}",
                    self.sessions.len(),
                    self.phase().name(),
                    s.tick_us_total as f64 / s.ticks.max(1) as f64,
                    s.tick_us_worst,
                    self.sim.props().dynamic_count(),
                    s.bytes_in as f64 / 1024.0 / secs,
                    s.bytes_out as f64 / 1024.0 / secs,
                    s.snapshots_sent,
                    s.bad_packets,
                    s.bad_tags
                );
                self.say(line);
            }
        }
    }

    // ---- the match flow ---------------------------------------------------------------------------------------------------------

    fn step_flow(&mut self) {
        let Some(flow) = &self.flow else { return };
        let connected = self.sessions.len();
        let ready = self.sessions.iter().filter(|s| s.ready).count();
        let in_round = self.sessions.iter().filter(|s| s.in_round).count();
        let best_score = self.sessions.iter().filter(|s| s.in_round).filter_map(|s| self.sim.player(s.slot)).map(|p| p.combat.kills).max().unwrap_or(0);
        let rules_outcome = (flow.phase() == Phase::Playing).then(|| self.sim.rules().ended().map(str::to_string)).flatten();
        let input = FlowInput { connected, ready, in_round, rules_outcome, best_score };
        let Some(event) = self.flow.as_mut().and_then(|f| f.step(&input)) else { return };
        self.status_dirty = true;
        match event {
            FlowEvent::CountdownStarted => self.begin_round_world(),
            FlowEvent::CountdownAborted => {
                self.say("countdown aborted".to_string());
                self.clear_world();
            }
            FlowEvent::RoundStarted => {
                let r = self.round();
                self.say(format!("round {r} started with {} player(s)", self.sessions.len()));
            }
            FlowEvent::RoundEnded(reason) => self.finish_round(reason),
            FlowEvent::ReturnedToLobby => {
                self.clear_world();
                self.say("back in the lobby".to_string());
            }
        }
    }

    /// Builds the round's world and puts every connected player at a spawn, frozen until the countdown ends.
    fn begin_round_world(&mut self) {
        let round = self.round();
        if let Some(factory) = self.factory.as_mut() {
            match factory() {
                Ok(sim) => self.sim = sim,
                Err(e) => self.say(format!("cannot rebuild the world for round {round}: {e}; reusing the old one")),
            }
        }
        for slot in 0..MAX_PLAYERS {
            self.sim.remove_player(slot);
        }
        let mut order: Vec<usize> = (0..self.sessions.len()).collect();
        order.sort_by_key(|&i| self.sessions[i].slot);
        for i in order {
            let (slot, character) = (self.sessions[i].slot, self.sessions[i].character);
            let placed = self.sim.add_player_in_slot(slot, character_from_wire(character));
            let s = &mut self.sessions[i];
            s.in_round = placed;
            s.forget_world();
        }
        if let Some(h) = self.round_header.clone() {
            if let Err(e) = self.sim.start_recording(h) {
                self.say(format!("cannot record round {round}: {e}"));
            }
        }
        self.say(format!("countdown for round {round}: {} player(s)", self.sessions.len()));
        self.send_round_welcomes();
    }

    /// Removes every body from the world (after an aborted countdown or the results).
    fn clear_world(&mut self) {
        for s in self.sessions.iter_mut() {
            if s.in_round {
                self.sim.remove_player(s.slot);
                s.in_round = false;
            }
        }
    }

    fn finish_round(&mut self, reason: EndReason) {
        let round = self.round();
        let mut scores: Vec<(u8, u32)> =
            self.sessions.iter().filter(|s| s.in_round).filter_map(|s| self.sim.player(s.slot).map(|p| (s.slot as u8, p.combat.kills))).collect();
        scores.sort_unstable();
        let top = scores.iter().map(|s| s.1).max().unwrap_or(0);
        let winner = if top > 0 && scores.iter().filter(|s| s.1 == top).count() == 1 { scores.iter().find(|s| s.1 == top).map(|s| s.0) } else { None };
        let (code, text) = reason.to_wire();
        self.last_result = Some(LastResult { winner: winner.unwrap_or(NO_WINNER), end_code: code, end_text: text });
        let trace = self.sim.take_trace();
        for s in self.sessions.iter_mut() {
            s.ready = false; // a rematch needs everyone to press Ready again
        }
        self.stats.rounds += 1;
        self.say(format!("round {round} ended: {}{}", reason.text(), winner.map(|w| format!("; winner: player {w}")).unwrap_or_default()));
        if let Some(hook) = self.round_hook.as_mut() {
            hook(RoundRecord { round, reason, winner, scores, trace });
        }
    }

    /// Sends the current round's `Welcome` to every player in the round who has not yet confirmed it (it is repeated with each `Status`,
    /// so a lost datagram delays the spawn by a fifth of a second, never loses it).
    fn send_round_welcomes(&mut self) {
        let round = self.current_round();
        for i in 0..self.sessions.len() {
            if self.sessions[i].in_round && self.sessions[i].round_ack < round {
                if let Some(w) = self.welcome_for(&self.sessions[i]) {
                    self.send_signed(i, &ServerMsg::Welcome(w));
                }
            }
        }
    }

    fn roster(&self) -> Vec<RosterEntry> {
        let mut r: Vec<RosterEntry> = self
            .sessions
            .iter()
            .map(|s| RosterEntry {
                id: s.slot as u8,
                flags: if s.ready { ROSTER_READY } else { 0 } | if s.in_round { ROSTER_IN_ROUND } else { 0 },
                character: s.character,
                ping_ms: s.rtt_ms,
                score: self.sim.player(s.slot).filter(|_| s.in_round).map_or(0, |p| p.combat.kills.min(u16::MAX as u32) as u16),
                name: s.name.clone(),
            })
            .collect();
        r.sort_by_key(|e| e.id);
        r.truncate(MAX_ROSTER);
        r
    }

    fn send_statuses(&mut self, now: Instant) {
        self.send_round_welcomes();
        if self.sessions.is_empty() {
            return;
        }
        let roster = self.roster();
        let (phase, round, ticks_left, min_players) = match &self.flow {
            Some(f) => (f.phase(), f.round(), f.ticks_left(), f.settings().min_players),
            None => (Phase::Playing, 0, u32::MAX, 1),
        };
        let (winner, end_code, end_text) = match &self.last_result {
            Some(r) => (r.winner, r.end_code, r.end_text.clone()),
            None => (NO_WINNER, NO_END, String::new()),
        };
        let rules = self.sim.rules();
        let vars: Vec<RuleVar> = rules.vars().into_iter().take(MAX_RULE_VARS).map(|(name, value)| RuleVar { name: name.to_string(), value }).collect();
        let hidden: Vec<u16> = rules.hidden().filter_map(|id| self.sim.rule_object_index(id)).take(MAX_RULE_HIDDEN).collect();
        let latest_event = rules.history().iter().rev().find(|e| !e.name.starts_with("end:"));
        let event = latest_event.map_or_else(String::new, |e| e.name.clone());
        let event_tick = latest_event.map_or(0, |e| e.tick.min(u32::MAX as u64) as u32);
        let outcome = rules.ended().unwrap_or_default().to_string();
        let rule_tick = self.sim.tick().min(u32::MAX as u64) as u32;
        for i in 0..self.sessions.len() {
            let s = &mut self.sessions[i];
            s.status_seq = s.status_seq.wrapping_add(1);
            let status_seq = s.status_seq;
            let status = Status {
                seq: status_seq,
                phase,
                round,
                ticks_left,
                min_players,
                in_round: s.in_round,
                winner,
                end_code,
                end_text: end_text.clone(),
                echo_time_ms: s.last_client_time_ms,
                echo_hold_ms: now.duration_since(s.last_client_packet_at).as_millis().min(65_535) as u16,
                roster: roster.clone(),
            };
            self.stats.statuses_sent += 1;
            self.send_signed(i, &ServerMsg::Status(status));
            let rule_state = RuleState {
                seq: status_seq,
                round,
                server_tick: rule_tick,
                vars: vars.clone(),
                hidden: hidden.clone(),
                event: event.clone(),
                event_tick,
                outcome: outcome.clone(),
            };
            self.stats.rule_states_sent += 1;
            self.send_signed(i, &ServerMsg::RuleState(rule_state));
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
            self.stats.players_sent += snap.players.len() as u64;
            self.stats.props_sent += snap.props.len() as u64;
            let msg = ServerMsg::Snapshot(snap);
            self.send_signed(i, &msg);
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
        for i in 0..self.sessions.len() {
            self.send_signed(i, &ServerMsg::Bye);
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
