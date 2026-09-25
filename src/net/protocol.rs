//! Wire format: small hand-rolled little-endian binary messages over UDP, no dependencies.
//!
//! Every datagram is `magic:u16, version:u16, kind:u8, body`. Decoding never panics: truncated,
//! oversized or garbage input is a [`DecodeError`] (a test feeds it random bytes). Hard limits
//! ([`MAX_PACKET`], [`MAX_PLAYERS_PER_SNAPSHOT`], [`MAX_PROPS_PER_SNAPSHOT`], [`MAX_INPUTS_PER_PACKET`])
//! keep a hostile packet from making the receiver allocate or loop.
//!
//! Client -> server: [`ClientMsg::Hello`] (join / resume, with the handshake's cookie and join-key proof), [`ClientMsg::Input`] (the
//! last few ticks of input, redundantly, so one lost packet loses nothing, plus an acknowledgement of the newest snapshot received),
//! [`ClientMsg::Lobby`] (ready / character, repeated: *state*, not events, so a lost packet costs nothing), [`ClientMsg::Bye`].
//! Server -> client: [`ServerMsg::Challenge`], [`ServerMsg::Welcome`], [`ServerMsg::Reject`], [`ServerMsg::Snapshot`] (every
//! player, plus the props that changed since the client last acknowledged), [`ServerMsg::Status`] (the lobby / round state and the
//! roster, repeated), [`ServerMsg::NoSession`], [`ServerMsg::Bye`].
//!
//! Every datagram except `Hello`, `Challenge`, `Reject` and `NoSession` ends with an 8-byte tag (see [`super::auth`], ADR 0028):
//! [`ClientMsg::decode`] and [`ServerMsg::decode`] parse a datagram *whose tag has already been verified and stripped*
//! ([`is_signed`] says which kinds carry one), so nothing in this file trusts a packet the tag has not vouched for.

use super::auth::PROOF_LEN;
use crate::player::Character;
use crate::sim::flow::Phase;
use crate::sim::player::PlayerInput;
use std::fmt;

/// First two bytes of every datagram ("RD").
pub const MAGIC: u16 = 0x5244;
/// Bumped on any incompatible change; a mismatched client is rejected.
pub const PROTOCOL_VERSION: u16 = 3;
/// Largest datagram either side sends or accepts (under a typical 1500-byte MTU).
pub const MAX_PACKET: usize = 1400;
/// Most inputs one packet carries (the newest is last).
pub const MAX_INPUTS_PER_PACKET: usize = 4;
/// Most players in one snapshot.
pub const MAX_PLAYERS_PER_SNAPSHOT: usize = 8;
/// Most props in one snapshot (30 bytes each: fits the packet with the players, 35 bytes each).
pub const MAX_PROPS_PER_SNAPSHOT: usize = 30;

/// Longest player name, bytes.
pub const MAX_NAME: usize = 16;
/// Longest round-outcome text in a [`Status`], bytes.
pub const MAX_OUTCOME: usize = 32;
/// Most roster entries in a [`Status`] (a match holds at most this many players).
pub const MAX_ROSTER: usize = MAX_PLAYERS_PER_SNAPSHOT;
/// `Status::winner` when nobody won (a draw, or a co-operative outcome).
pub const NO_WINNER: u8 = 255;

const KIND_HELLO: u8 = 1;
const KIND_INPUT: u8 = 2;
const KIND_C_BYE: u8 = 3;
const KIND_LOBBY: u8 = 4;
const KIND_WELCOME: u8 = 16;
const KIND_REJECT: u8 = 17;
const KIND_SNAPSHOT: u8 = 18;
const KIND_S_BYE: u8 = 19;
const KIND_CHALLENGE: u8 = 20;
const KIND_STATUS: u8 = 21;
const KIND_NO_SESSION: u8 = 22;

/// The kind byte of a datagram (`None` if it is too short to have one or is not ours).
pub fn peek_kind(bytes: &[u8]) -> Option<u8> {
    (bytes.len() >= 5 && u16::from_le_bytes([bytes[0], bytes[1]]) == MAGIC).then(|| bytes[4])
}

/// Whether datagrams of this kind carry an authentication tag (everything after the handshake).
pub fn is_signed(kind: u8) -> bool {
    matches!(kind, KIND_INPUT | KIND_C_BYE | KIND_LOBBY | KIND_WELCOME | KIND_SNAPSHOT | KIND_S_BYE | KIND_STATUS)
}

/// A player name made safe to show and to send: control characters removed, trimmed, at most [`MAX_NAME`] bytes (cut on a character
/// boundary), and `player` when nothing is left.
pub fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
    let mut out = String::new();
    for c in cleaned.trim().chars() {
        if out.len() + c.len_utf8() > MAX_NAME {
            break;
        }
        out.push(c);
    }
    if out.is_empty() {
        "player".to_string()
    } else {
        out
    }
}

/// Why a datagram could not be decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Not one of ours (wrong magic).
    NotOurs,
    /// Ends before the message does.
    Truncated,
    /// Unknown message kind.
    UnknownKind(u8),
    /// A count or value outside the allowed limits.
    OutOfRange,
    /// Bytes left over after the message.
    TrailingBytes,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::NotOurs => write!(f, "not a Red datagram (magic bytes are not 0x5244)"),
            DecodeError::Truncated => write!(f, "datagram ends before the message does"),
            DecodeError::UnknownKind(k) => write!(f, "unknown message kind {k}"),
            DecodeError::OutOfRange => write!(f, "a count or enum value is outside the protocol's limits"),
            DecodeError::TrailingBytes => write!(f, "extra bytes after the end of the message"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// `0` human, `1` rat: how a [`Character`] travels on the wire.
pub fn character_to_wire(c: Character) -> u8 {
    match c {
        Character::Human => 0,
        Character::Rat => 1,
    }
}

/// The inverse of [`character_to_wire`]; anything but `1` is a human (a hostile value gets the default body).
pub fn character_from_wire(v: u8) -> Character {
    if v == 1 {
        Character::Rat
    } else {
        Character::Human
    }
}

/// Why the server refused a join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// Protocol versions differ.
    Version,
    /// The client loaded a different map file.
    WrongMap,
    /// The match is full.
    Full,
    /// The join key is wrong.
    BadKey,
    /// The server wants a join key and the client has none (decided by the client from the `Challenge`).
    NeedsKey,
    /// The client has a join key but the server asks for none, so the client refuses to trust it (decided by the client).
    ServerIsOpen,
}

impl RejectReason {
    fn to_u8(self) -> u8 {
        match self {
            RejectReason::Version => 1,
            RejectReason::WrongMap => 2,
            RejectReason::Full => 3,
            RejectReason::BadKey => 4,
            RejectReason::NeedsKey => 5,
            RejectReason::ServerIsOpen => 6,
        }
    }
    fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            1 => Ok(RejectReason::Version),
            2 => Ok(RejectReason::WrongMap),
            3 => Ok(RejectReason::Full),
            4 => Ok(RejectReason::BadKey),
            5 => Ok(RejectReason::NeedsKey),
            6 => Ok(RejectReason::ServerIsOpen),
            _ => Err(DecodeError::OutOfRange),
        }
    }

    /// A sentence for the player: what went wrong and what to do.
    pub fn explain(self) -> &'static str {
        match self {
            RejectReason::Version => "The server runs a different version of the game. Update the client or the server.",
            RejectReason::WrongMap => "The server has a different copy of the map. Get the same map file the server loads.",
            RejectReason::Full => "The match is full.",
            RejectReason::BadKey => "The join key is wrong.",
            RejectReason::NeedsKey => "This server needs a join key. Ask its host for it.",
            RejectReason::ServerIsOpen => "You entered a join key but this server does not ask for one, so it may not be the server you meant.",
        }
    }
}

/// A join request (or a re-join: a non-zero `resume_token` asks to get the old player back). Sent twice: first with `cookie == 0`,
/// answered by a [`ServerMsg::Challenge`]; then again carrying the cookie and, for a keyed server, the join-key `proof`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// Must equal [`PROTOCOL_VERSION`].
    pub version: u16,
    /// Hash of the client's copy of the map (`net::map_hash`).
    pub map_hash: u32,
    /// `0` human, `1` rat.
    pub character: u8,
    /// Token from an earlier [`Welcome`], or `0`.
    pub resume_token: u64,
    /// A fresh random number per join attempt; part of the cookie, the proof and the session key.
    pub client_nonce: u64,
    /// The cookie from the server's `Challenge`, or `0` before there is one.
    pub cookie: u64,
    /// `HMAC(join key, ...)` (see [`super::auth::join_proof`]); all zero when the client has no key or has no cookie yet.
    pub proof: [u8; PROOF_LEN],
    /// The name to show in the lobby (sanitized by the receiver).
    pub name: String,
}

impl Default for Hello {
    fn default() -> Self {
        Hello { version: PROTOCOL_VERSION, map_hash: 0, character: 0, resume_token: 0, client_nonce: 0, cookie: 0, proof: [0; PROOF_LEN], name: String::new() }
    }
}

/// A batch of recent inputs and what the client has received so far.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InputPacket {
    /// Sequence number of the newest snapshot the client received (`0` = none yet).
    pub snapshot_ack: u32,
    /// Client clock, milliseconds (echoed back for round-trip time).
    pub client_time_ms: u32,
    /// The newest round whose `Welcome` the client has applied (the server repeats a round's `Welcome` until it sees this).
    pub round_ack: u16,
    /// The client's own smoothed round-trip time, ms (cosmetic: shown as this player's ping in the roster, never trusted for anything else).
    pub rtt_ms: u16,
    /// Up to [`MAX_INPUTS_PER_PACKET`] inputs, oldest first.
    pub inputs: Vec<PlayerInput>,
}

/// What a client wants while it is in the lobby: sent every few hundred ms and whenever it changes, so the server's copy is right
/// even after a lost packet. It doubles as the keep-alive while no `Input` is flowing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LobbyCmd {
    /// Pressed Ready.
    pub ready: bool,
    /// `0` human, `1` rat.
    pub character: u8,
    /// See [`InputPacket::round_ack`].
    pub round_ack: u16,
    /// Client clock, milliseconds (echoed in [`Status`] for round-trip time).
    pub client_time_ms: u32,
    /// See [`InputPacket::rtt_ms`].
    pub rtt_ms: u16,
}

/// Messages a client sends.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientMsg {
    /// Join or resume.
    Hello(Hello),
    /// Recent inputs + acknowledgement.
    Input(InputPacket),
    /// Ready / character while in the lobby (and a keep-alive).
    Lobby(LobbyCmd),
    /// Leaving; the server frees the player immediately.
    Bye,
}

/// The server's answer to a successful [`Hello`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Welcome {
    /// The player's slot / id in snapshots.
    pub player_id: u8,
    /// Secret to resume this player after a disconnect.
    pub token: u64,
    /// Simulation ticks per second.
    pub tick_rate: u16,
    /// A snapshot is sent every this many ticks.
    pub snapshot_every: u8,
    /// The server's tick right now.
    pub server_tick: u32,
    /// Where the player stands: `x, foot_y, z`, and `yaw` (radians).
    pub spawn: [f32; 4],
    /// Character the server gave them (`0` human, `1` rat).
    pub character: u8,
    /// The round this spawn belongs to (`0` in open play). A `Welcome` with a newer round re-places the player at the new spawn.
    pub round: u16,
    /// Whether the player is part of the running round (`false`: watching until the next one).
    pub in_round: bool,
}

/// One player's state in a snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerSnap {
    /// Player id (slot).
    pub id: u8,
    /// `0` human, `1` rat.
    pub character: u8,
    /// Bit 0: crouching, bit 1: swinging the bat, bit 2: dead.
    pub flags: u8,
    /// `x, foot_y, z`.
    pub pos: [f32; 3],
    /// Look yaw, radians.
    pub yaw: f32,
    /// Look pitch, radians.
    pub pitch: f32,
    /// Horizontal speed, m/s (drives the walk animation).
    pub speed: f32,
    /// Vertical velocity, m/s (lets the owner's client replay a jump exactly).
    pub vy: f32,
    /// The weapon in hand (`weapons::Weapon::wire`).
    pub weapon: u8,
    /// The prop this player is carrying ([`NO_PROP`] when none).
    pub held: u16,
    /// Hit points.
    pub hp: u8,
}

/// `PlayerSnap::held` when the player carries nothing.
pub const NO_PROP: u16 = u16::MAX;

/// One prop's pose in a snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropSnap {
    /// Prop id (index in `physics::loose_props`).
    pub id: u16,
    /// World position.
    pub pos: [f32; 3],
    /// World rotation, `x, y, z, w`.
    pub rot: [f32; 4],
}

/// The world as of one server tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Increasing per client; acknowledged by [`InputPacket::snapshot_ack`].
    pub seq: u32,
    /// The server tick this describes.
    pub server_tick: u32,
    /// Newest input sequence the server has processed *for the receiving client* (`0` = none).
    pub ack_input_seq: u32,
    /// The `client_time_ms` of the newest input packet received, echoed for RTT.
    pub echo_time_ms: u32,
    /// How long the server held that packet before sending this snapshot, ms (subtract for RTT).
    pub echo_hold_ms: u16,
    /// Every connected player.
    pub players: Vec<PlayerSnap>,
    /// Props changed since the client's last acknowledged snapshot (possibly a subset; the rest follow).
    pub props: Vec<PropSnap>,
}

/// One line of the roster in a [`Status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    /// Player id (the slot used in snapshots).
    pub id: u8,
    /// [`ROSTER_READY`] | [`ROSTER_IN_ROUND`].
    pub flags: u8,
    /// `0` human, `1` rat.
    pub character: u8,
    /// The player's own reported round-trip time, ms.
    pub ping_ms: u16,
    /// Kills this round.
    pub score: u16,
    /// Display name.
    pub name: String,
}

/// [`RosterEntry::flags`]: the player pressed Ready.
pub const ROSTER_READY: u8 = 1;
/// [`RosterEntry::flags`]: the player is part of the running round.
pub const ROSTER_IN_ROUND: u8 = 2;

/// Where the match is and who is in it. The server sends it to every client a few times a second and whenever something changes; a client
/// keeps the newest (by `seq`). Losing one costs a fifth of a second of staleness, nothing else.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    /// Increasing (wrapping); an older `Status` that arrives late is ignored.
    pub seq: u16,
    /// The lobby / round phase.
    pub phase: Phase,
    /// The current round (`0` = none yet, or open play).
    pub round: u16,
    /// Ticks until the phase changes by itself (`u32::MAX` = no limit).
    pub ticks_left: u32,
    /// Players needed before a countdown can start.
    pub min_players: u8,
    /// Whether the receiving client is part of the running round.
    pub in_round: bool,
    /// The winner's player id after a round ([`NO_WINNER`] otherwise).
    pub winner: u8,
    /// Why the last round ended: `0` rules (see `end_text`), `1` time up, `2` score reached, `3` abandoned, `255` no round has ended.
    pub end_code: u8,
    /// The rule outcome word when `end_code == 0`.
    pub end_text: String,
    /// The `client_time_ms` of the newest packet received from this client (for its round-trip time).
    pub echo_time_ms: u32,
    /// How long the server held that packet, ms.
    pub echo_hold_ms: u16,
    /// Everyone in the match.
    pub roster: Vec<RosterEntry>,
}

/// `Status::end_code` before any round has ended.
pub const NO_END: u8 = 255;

/// Messages a server sends.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMsg {
    /// "Prove you can receive at your address": a cookie to send back, and whether the server wants a join key.
    Challenge {
        /// The cookie (never `0`).
        cookie: u64,
        /// Whether the server was started with a join key.
        requires_key: bool,
    },
    /// Joined.
    Welcome(Welcome),
    /// Refused.
    Reject(RejectReason),
    /// World state.
    Snapshot(Snapshot),
    /// Lobby / round state and the roster.
    Status(Status),
    /// "I have no session for your address" (the server restarted, or timed you out): re-join. Unauthenticated, so a client only
    /// takes it as a hint to re-handshake.
    NoSession,
    /// The server is closing this session.
    Bye,
}

// ---------------------------------------------------------------------------------------------
// codec
// ---------------------------------------------------------------------------------------------

struct W<'a>(&'a mut Vec<u8>);

impl W<'_> {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn header(&mut self, kind: u8) {
        self.u16(MAGIC);
        self.u16(PROTOCOL_VERSION);
        self.u8(kind);
    }
    /// A length-prefixed string, cut to `max` bytes on a character boundary.
    fn text(&mut self, s: &str, max: usize) {
        let mut n = s.len().min(max);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.u8(n as u8);
        self.0.extend_from_slice(&s.as_bytes()[..n]);
    }
}

struct R<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.i.checked_add(n).ok_or(DecodeError::Truncated)?;
        let s = self.b.get(self.i..end).ok_or(DecodeError::Truncated)?;
        self.i = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.array::<1>()?[0])
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.array()?))
    }
    /// A length-prefixed UTF-8 string of at most `max` bytes.
    fn text(&mut self, max: usize) -> Result<String, DecodeError> {
        let n = self.u8()? as usize;
        if n > max {
            return Err(DecodeError::OutOfRange);
        }
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| DecodeError::OutOfRange)
    }
    fn finish(&self) -> Result<(), DecodeError> {
        if self.i == self.b.len() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

/// Reads the datagram header, returning the kind and a reader over the body. The version is *not*
/// checked here: a Hello from an old client must still decode so the server can reject it politely.
fn open(bytes: &[u8]) -> Result<(u8, R<'_>), DecodeError> {
    let mut r = R { b: bytes, i: 0 };
    if r.u16()? != MAGIC {
        return Err(DecodeError::NotOurs);
    }
    let _version = r.u16()?;
    let kind = r.u8()?;
    Ok((kind, r))
}

fn put_input(w: &mut W, i: &PlayerInput) {
    w.u32(i.seq);
    w.u8(i.flags());
    w.u8(i.forward as u8);
    w.u8(i.strafe as u8);
    w.f32(i.yaw);
    w.f32(i.pitch);
}

fn get_input(r: &mut R) -> Result<PlayerInput, DecodeError> {
    let seq = r.u32()?;
    let flags = r.u8()?;
    let (forward, strafe) = (r.u8()? as i8, r.u8()? as i8);
    let (yaw, pitch) = (r.f32()?, r.f32()?);
    Ok(PlayerInput { seq, forward, strafe, yaw, pitch, ..Default::default() }.with_flags(flags).sanitized())
}

impl ClientMsg {
    /// Appends this message to `out` (which the caller clears).
    pub fn encode(&self, out: &mut Vec<u8>) {
        let mut w = W(out);
        match self {
            ClientMsg::Hello(h) => {
                w.header(KIND_HELLO);
                w.u16(h.version);
                w.u32(h.map_hash);
                w.u8(h.character);
                w.u64(h.resume_token);
                w.u64(h.client_nonce);
                w.u64(h.cookie);
                w.0.extend_from_slice(&h.proof);
                w.text(&h.name, MAX_NAME);
            }
            ClientMsg::Lobby(l) => {
                w.header(KIND_LOBBY);
                w.u8(l.ready as u8);
                w.u8(l.character);
                w.u16(l.round_ack);
                w.u32(l.client_time_ms);
                w.u16(l.rtt_ms);
            }
            ClientMsg::Input(p) => {
                w.header(KIND_INPUT);
                w.u32(p.snapshot_ack);
                w.u32(p.client_time_ms);
                w.u16(p.round_ack);
                w.u16(p.rtt_ms);
                let n = p.inputs.len().min(MAX_INPUTS_PER_PACKET);
                w.u8(n as u8);
                for i in &p.inputs[p.inputs.len() - n..] {
                    put_input(&mut w, i);
                }
            }
            ClientMsg::Bye => w.header(KIND_C_BYE),
        }
    }

    /// Decodes one datagram.
    pub fn decode(bytes: &[u8]) -> Result<ClientMsg, DecodeError> {
        let (kind, mut r) = open(bytes)?;
        let msg = match kind {
            KIND_HELLO => {
                let version = r.u16()?;
                if version != PROTOCOL_VERSION {
                    // A client of another protocol version: its Hello layout is unknown, so keep only the version and let the
                    // server say "wrong version" instead of dropping it as garbage.
                    return Ok(ClientMsg::Hello(Hello { version, ..Hello::default() }));
                }
                ClientMsg::Hello(Hello {
                    version,
                    map_hash: r.u32()?,
                    character: r.u8()?,
                    resume_token: r.u64()?,
                    client_nonce: r.u64()?,
                    cookie: r.u64()?,
                    proof: r.array()?,
                    name: r.text(MAX_NAME)?,
                })
            }
            KIND_LOBBY => {
                ClientMsg::Lobby(LobbyCmd { ready: r.u8()? != 0, character: r.u8()?, round_ack: r.u16()?, client_time_ms: r.u32()?, rtt_ms: r.u16()? })
            }
            KIND_INPUT => {
                let (snapshot_ack, client_time_ms, round_ack, rtt_ms) = (r.u32()?, r.u32()?, r.u16()?, r.u16()?);
                let n = r.u8()? as usize;
                if n > MAX_INPUTS_PER_PACKET {
                    return Err(DecodeError::OutOfRange);
                }
                let mut inputs = Vec::with_capacity(n);
                for _ in 0..n {
                    inputs.push(get_input(&mut r)?);
                }
                ClientMsg::Input(InputPacket { snapshot_ack, client_time_ms, round_ack, rtt_ms, inputs })
            }
            KIND_C_BYE => ClientMsg::Bye,
            k => return Err(DecodeError::UnknownKind(k)),
        };
        r.finish()?;
        Ok(msg)
    }
}

impl ServerMsg {
    /// Appends this message to `out` (which the caller clears).
    pub fn encode(&self, out: &mut Vec<u8>) {
        let mut w = W(out);
        match self {
            ServerMsg::Welcome(m) => {
                w.header(KIND_WELCOME);
                w.u8(m.player_id);
                w.u64(m.token);
                w.u16(m.tick_rate);
                w.u8(m.snapshot_every);
                w.u32(m.server_tick);
                for v in m.spawn {
                    w.f32(v);
                }
                w.u8(m.character);
                w.u16(m.round);
                w.u8(m.in_round as u8);
            }
            ServerMsg::Reject(r) => {
                w.header(KIND_REJECT);
                w.u8(r.to_u8());
            }
            ServerMsg::Challenge { cookie, requires_key } => {
                w.header(KIND_CHALLENGE);
                w.u64(*cookie);
                w.u8(*requires_key as u8);
            }
            ServerMsg::NoSession => w.header(KIND_NO_SESSION),
            ServerMsg::Status(st) => {
                w.header(KIND_STATUS);
                w.u16(st.seq);
                w.u8(st.phase.to_wire());
                w.u16(st.round);
                w.u32(st.ticks_left);
                w.u8(st.min_players);
                w.u8(st.in_round as u8);
                w.u8(st.winner);
                w.u8(st.end_code);
                w.text(&st.end_text, MAX_OUTCOME);
                w.u32(st.echo_time_ms);
                w.u16(st.echo_hold_ms);
                let n = st.roster.len().min(MAX_ROSTER);
                w.u8(n as u8);
                for e in &st.roster[..n] {
                    w.u8(e.id);
                    w.u8(e.flags);
                    w.u8(e.character);
                    w.u16(e.ping_ms);
                    w.u16(e.score);
                    w.text(&e.name, MAX_NAME);
                }
            }
            ServerMsg::Snapshot(s) => {
                w.header(KIND_SNAPSHOT);
                w.u32(s.seq);
                w.u32(s.server_tick);
                w.u32(s.ack_input_seq);
                w.u32(s.echo_time_ms);
                w.u16(s.echo_hold_ms);
                let np = s.players.len().min(MAX_PLAYERS_PER_SNAPSHOT);
                w.u8(np as u8);
                let nq = s.props.len().min(MAX_PROPS_PER_SNAPSHOT);
                w.u8(nq as u8);
                for p in &s.players[..np] {
                    w.u8(p.id);
                    w.u8(p.character);
                    w.u8(p.flags);
                    for v in p.pos {
                        w.f32(v);
                    }
                    w.f32(p.yaw);
                    w.f32(p.pitch);
                    w.f32(p.speed);
                    w.f32(p.vy);
                    w.u8(p.weapon);
                    w.u16(p.held);
                    w.u8(p.hp);
                }
                for q in &s.props[..nq] {
                    w.u16(q.id);
                    for v in q.pos.into_iter().chain(q.rot) {
                        w.f32(v);
                    }
                }
            }
            ServerMsg::Bye => w.header(KIND_S_BYE),
        }
    }

    /// Decodes one datagram.
    pub fn decode(bytes: &[u8]) -> Result<ServerMsg, DecodeError> {
        let (kind, mut r) = open(bytes)?;
        let msg = match kind {
            KIND_WELCOME => {
                let (player_id, token, tick_rate, snapshot_every, server_tick) = (r.u8()?, r.u64()?, r.u16()?, r.u8()?, r.u32()?);
                let spawn = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
                let (character, round, in_round) = (r.u8()?, r.u16()?, r.u8()? != 0);
                ServerMsg::Welcome(Welcome { player_id, token, tick_rate, snapshot_every, server_tick, spawn, character, round, in_round })
            }
            KIND_REJECT => ServerMsg::Reject(RejectReason::from_u8(r.u8()?)?),
            KIND_CHALLENGE => ServerMsg::Challenge { cookie: r.u64()?, requires_key: r.u8()? != 0 },
            KIND_NO_SESSION => ServerMsg::NoSession,
            KIND_STATUS => {
                let seq = r.u16()?;
                let phase = Phase::from_wire(r.u8()?).ok_or(DecodeError::OutOfRange)?;
                let (round, ticks_left, min_players, in_round, winner, end_code) = (r.u16()?, r.u32()?, r.u8()?, r.u8()? != 0, r.u8()?, r.u8()?);
                let end_text = r.text(MAX_OUTCOME)?;
                let (echo_time_ms, echo_hold_ms) = (r.u32()?, r.u16()?);
                let n = r.u8()? as usize;
                if n > MAX_ROSTER {
                    return Err(DecodeError::OutOfRange);
                }
                let mut roster = Vec::with_capacity(n);
                for _ in 0..n {
                    roster.push(RosterEntry { id: r.u8()?, flags: r.u8()?, character: r.u8()?, ping_ms: r.u16()?, score: r.u16()?, name: r.text(MAX_NAME)? });
                }
                ServerMsg::Status(Status {
                    seq,
                    phase,
                    round,
                    ticks_left,
                    min_players,
                    in_round,
                    winner,
                    end_code,
                    end_text,
                    echo_time_ms,
                    echo_hold_ms,
                    roster,
                })
            }
            KIND_SNAPSHOT => {
                let (seq, server_tick, ack_input_seq, echo_time_ms) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let echo_hold_ms = r.u16()?;
                let (np, nq) = (r.u8()? as usize, r.u8()? as usize);
                if np > MAX_PLAYERS_PER_SNAPSHOT || nq > MAX_PROPS_PER_SNAPSHOT {
                    return Err(DecodeError::OutOfRange);
                }
                let mut players = Vec::with_capacity(np);
                for _ in 0..np {
                    players.push(PlayerSnap {
                        id: r.u8()?,
                        character: r.u8()?,
                        flags: r.u8()?,
                        pos: [r.f32()?, r.f32()?, r.f32()?],
                        yaw: r.f32()?,
                        pitch: r.f32()?,
                        speed: r.f32()?,
                        vy: r.f32()?,
                        weapon: r.u8()?,
                        held: r.u16()?,
                        hp: r.u8()?,
                    });
                }
                let mut props = Vec::with_capacity(nq);
                for _ in 0..nq {
                    props.push(PropSnap { id: r.u16()?, pos: [r.f32()?, r.f32()?, r.f32()?], rot: [r.f32()?, r.f32()?, r.f32()?, r.f32()?] });
                }
                ServerMsg::Snapshot(Snapshot { seq, server_tick, ack_input_seq, echo_time_ms, echo_hold_ms, players, props })
            }
            KIND_S_BYE => ServerMsg::Bye,
            k => return Err(DecodeError::UnknownKind(k)),
        };
        r.finish()?;
        Ok(msg)
    }
}

/// Bytes of an encoded snapshot with `players` players and `props` props.
pub const fn snapshot_bytes(players: usize, props: usize) -> usize {
    5 + 16 + 2 + 2 + players * 35 + props * 30
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(seq: u32) -> PlayerInput {
        PlayerInput {
            seq,
            forward: 1,
            strafe: -1,
            jump: true,
            sprint: false,
            crouch: true,
            yaw: 1.25,
            pitch: -0.5,
            interact: true,
            attack: false,
            reload: true,
            switch_weapon: true,
        }
    }

    fn roundtrip_c(m: ClientMsg) {
        let mut b = Vec::new();
        m.encode(&mut b);
        assert!(b.len() <= MAX_PACKET);
        assert_eq!(ClientMsg::decode(&b).unwrap(), m);
    }

    fn roundtrip_s(m: ServerMsg) {
        let mut b = Vec::new();
        m.encode(&mut b);
        assert!(b.len() <= MAX_PACKET, "{} bytes", b.len());
        assert_eq!(ServerMsg::decode(&b).unwrap(), m);
    }

    fn full_status() -> Status {
        Status {
            seq: 65_535,
            phase: Phase::Results,
            round: 9,
            ticks_left: u32::MAX,
            min_players: 2,
            in_round: true,
            winner: 3,
            end_code: 0,
            end_text: "victory".to_string(),
            echo_time_ms: 1234,
            echo_hold_ms: 5,
            roster: (0..MAX_ROSTER as u8)
                .map(|i| RosterEntry {
                    id: i,
                    flags: ROSTER_READY | ROSTER_IN_ROUND,
                    character: i % 2,
                    ping_ms: 20 + i as u16,
                    score: i as u16,
                    name: sanitize_name(&format!("player-with-a-long-name-{i}")),
                })
                .collect(),
        }
    }

    #[test]
    fn a_status_and_a_welcome_fit_a_packet_with_room_for_the_tag() {
        let mut b = Vec::new();
        ServerMsg::Status(full_status()).encode(&mut b);
        assert!(b.len() + super::super::auth::TAG_LEN <= MAX_PACKET, "{} bytes", b.len());
        assert!(b.len() < 320, "a full 8-player roster is {} bytes; keep Status small (it is sent 5x a second to everyone)", b.len());
    }

    #[test]
    fn names_are_cleaned_bounded_and_never_empty() {
        assert_eq!(sanitize_name("  Kev\u{7}\n "), "Kev");
        assert_eq!(sanitize_name(""), "player");
        assert_eq!(sanitize_name("\u{0}\u{1}"), "player");
        assert_eq!(sanitize_name("abcdefghijklmnopqrstuvwxyz").len(), MAX_NAME);
        let wide = sanitize_name(&"é".repeat(20));
        assert!(wide.len() <= MAX_NAME && wide.chars().all(|c| c == 'é'), "cut on a character boundary, never mid-character: {wide}");
    }

    #[test]
    fn oversized_or_invalid_text_and_phase_are_refused() {
        let mut b = Vec::new();
        ServerMsg::Status(full_status()).encode(&mut b);
        let mut bad = b.clone();
        bad[5 + 2] = 9; // the phase byte
        assert_eq!(ServerMsg::decode(&bad), Err(DecodeError::OutOfRange));
        let mut h = Vec::new();
        let mut w = W(&mut h);
        w.header(KIND_HELLO);
        w.u16(PROTOCOL_VERSION);
        w.u32(1);
        w.u8(0);
        w.u64(0);
        w.u64(1);
        w.u64(0);
        w.0.extend_from_slice(&[0; PROOF_LEN]);
        w.u8(200); // a name longer than MAX_NAME
        assert_eq!(ClientMsg::decode(&h), Err(DecodeError::OutOfRange));
        let mut h = h[..h.len() - 1].to_vec();
        let mut w = W(&mut h);
        w.u8(2);
        w.0.extend_from_slice(&[0xff, 0xfe]); // not UTF-8
        assert_eq!(ClientMsg::decode(&h), Err(DecodeError::OutOfRange));
    }

    #[test]
    fn every_message_round_trips() {
        roundtrip_c(ClientMsg::Hello(Hello {
            version: PROTOCOL_VERSION,
            map_hash: 0xdead_beef,
            character: 1,
            resume_token: u64::MAX,
            client_nonce: 0x1122_3344_5566_7788,
            cookie: 42,
            proof: [7; PROOF_LEN],
            name: "Ünï the 2nd".to_string(),
        }));
        roundtrip_c(ClientMsg::Input(InputPacket {
            snapshot_ack: 7,
            client_time_ms: 123456,
            round_ack: 3,
            rtt_ms: 48,
            inputs: vec![input(1), input(2), input(3), input(4)],
        }));
        roundtrip_c(ClientMsg::Input(InputPacket::default()));
        roundtrip_c(ClientMsg::Lobby(LobbyCmd { ready: true, character: 1, round_ack: 2, client_time_ms: 99, rtt_ms: 31 }));
        roundtrip_c(ClientMsg::Bye);
        roundtrip_s(ServerMsg::Welcome(Welcome {
            player_id: 3,
            token: 42,
            tick_rate: 60,
            snapshot_every: 2,
            server_tick: 999,
            spawn: [1.0, 0.0, -2.5, 1.57],
            character: 0,
            round: 4,
            in_round: true,
        }));
        for r in [RejectReason::Version, RejectReason::WrongMap, RejectReason::Full, RejectReason::BadKey, RejectReason::NeedsKey, RejectReason::ServerIsOpen] {
            roundtrip_s(ServerMsg::Reject(r));
        }
        roundtrip_s(ServerMsg::Bye);
        roundtrip_s(ServerMsg::NoSession);
        roundtrip_s(ServerMsg::Challenge { cookie: u64::MAX, requires_key: true });
        roundtrip_s(ServerMsg::Status(full_status()));
        let players = (0..MAX_PLAYERS_PER_SNAPSHOT as u8)
            .map(|i| PlayerSnap {
                id: i,
                character: i % 2,
                flags: 1,
                pos: [i as f32, 0.5, -1.0],
                yaw: 0.3,
                pitch: 0.1,
                speed: 3.2,
                vy: -0.5,
                weapon: 1,
                held: 7,
                hp: 80,
            })
            .collect();
        let props = (0..MAX_PROPS_PER_SNAPSHOT as u16).map(|i| PropSnap { id: i, pos: [1.0, 2.0, 3.0], rot: [0.0, 0.0, 0.0, 1.0] }).collect();
        roundtrip_s(ServerMsg::Snapshot(Snapshot { seq: 5, server_tick: 100, ack_input_seq: 90, echo_time_ms: 77, echo_hold_ms: 12, players, props }));
    }

    #[test]
    fn a_full_snapshot_fits_in_one_packet_and_the_size_formula_is_right() {
        let s = Snapshot {
            seq: 1,
            server_tick: 1,
            ack_input_seq: 1,
            echo_time_ms: 1,
            echo_hold_ms: 0,
            players: vec![
                PlayerSnap {
                    id: 0,
                    character: 0,
                    flags: 0,
                    pos: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    speed: 0.0,
                    vy: 0.0,
                    weapon: 0,
                    held: NO_PROP,
                    hp: 100
                };
                MAX_PLAYERS_PER_SNAPSHOT
            ],
            props: vec![PropSnap { id: 0, pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0] }; MAX_PROPS_PER_SNAPSHOT],
        };
        let mut b = Vec::new();
        ServerMsg::Snapshot(s).encode(&mut b);
        assert_eq!(b.len(), snapshot_bytes(MAX_PLAYERS_PER_SNAPSHOT, MAX_PROPS_PER_SNAPSHOT));
        assert!(b.len() < MAX_PACKET, "{}", b.len());
    }

    #[test]
    fn garbage_never_panics_and_is_rejected() {
        // Deterministic xorshift "fuzzing": random bytes, and valid messages with bytes flipped/truncated.
        let mut x = 0x1234_5678_9abc_def0u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..20_000 {
            let n = (next() % 200) as usize;
            let bytes: Vec<u8> = (0..n).map(|_| next() as u8).collect();
            let _ = ClientMsg::decode(&bytes);
            let _ = ServerMsg::decode(&bytes);
        }
        let mut valid = Vec::new();
        ServerMsg::Snapshot(Snapshot {
            seq: 1,
            server_tick: 2,
            ack_input_seq: 3,
            echo_time_ms: 4,
            echo_hold_ms: 5,
            players: vec![],
            props: vec![PropSnap { id: 1, pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0] }],
        })
        .encode(&mut valid);
        for cut in 0..valid.len() {
            assert!(ServerMsg::decode(&valid[..cut]).is_err(), "a truncated packet ({cut} bytes) must not decode");
        }
        for _ in 0..5_000 {
            let mut m = valid.clone();
            let i = (next() as usize) % m.len();
            m[i] ^= 1 << (next() % 8);
            let _ = ServerMsg::decode(&m);
        }
        let mut extra = valid.clone();
        extra.push(0);
        assert_eq!(ServerMsg::decode(&extra), Err(DecodeError::TrailingBytes));
        assert_eq!(ClientMsg::decode(&[0, 0, 0, 0, 1]), Err(DecodeError::NotOurs));
    }

    #[test]
    fn oversized_counts_are_refused_without_allocating_for_them() {
        let mut b = Vec::new();
        let mut w = W(&mut b);
        w.header(KIND_SNAPSHOT);
        for _ in 0..4 {
            w.u32(0);
        }
        w.u16(0);
        w.u8(255);
        w.u8(255);
        assert_eq!(ServerMsg::decode(&b), Err(DecodeError::OutOfRange));
        let mut c = Vec::new();
        let mut w = W(&mut c);
        w.header(KIND_INPUT);
        w.u32(0);
        w.u32(0);
        w.u16(0);
        w.u16(0);
        w.u8(200);
        assert_eq!(ClientMsg::decode(&c), Err(DecodeError::OutOfRange));
    }

    #[test]
    fn a_hello_from_an_old_client_still_decodes_so_it_can_be_rejected() {
        let mut b = Vec::new();
        let mut w = W(&mut b);
        w.u16(MAGIC);
        w.u16(0); // ancient header version
        w.u8(KIND_HELLO);
        w.u16(2); // a version-2 client: shorter body, unknown layout to us
        w.u32(1);
        w.u8(0);
        w.u64(0);
        assert!(matches!(ClientMsg::decode(&b), Ok(ClientMsg::Hello(h)) if h.version == 2));
    }
}
