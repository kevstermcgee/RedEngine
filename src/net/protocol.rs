//! Wire format: small hand-rolled little-endian binary messages over UDP, no dependencies.
//!
//! Every datagram is `magic:u16, version:u16, kind:u8, body`. Decoding never panics: truncated,
//! oversized or garbage input is a [`DecodeError`] (a test feeds it random bytes). Hard limits
//! ([`MAX_PACKET`], [`MAX_PLAYERS_PER_SNAPSHOT`], [`MAX_PROPS_PER_SNAPSHOT`], [`MAX_INPUTS_PER_PACKET`])
//! keep a hostile packet from making the receiver allocate or loop.
//!
//! Client -> server: [`ClientMsg::Hello`] (join / resume), [`ClientMsg::Input`] (the last few ticks of
//! input, redundantly, so one lost packet loses nothing, plus an acknowledgement of the newest
//! snapshot received), [`ClientMsg::Bye`].
//! Server -> client: [`ServerMsg::Welcome`], [`ServerMsg::Reject`], [`ServerMsg::Snapshot`] (every
//! player, plus the props that changed since the client last acknowledged), [`ServerMsg::Bye`].

use crate::player::Character;
use crate::sim::player::PlayerInput;
use std::fmt;

/// First two bytes of every datagram ("RD").
pub const MAGIC: u16 = 0x5244;
/// Bumped on any incompatible change; a mismatched client is rejected.
pub const PROTOCOL_VERSION: u16 = 2;
/// Largest datagram either side sends or accepts (under a typical 1500-byte MTU).
pub const MAX_PACKET: usize = 1400;
/// Most inputs one packet carries (the newest is last).
pub const MAX_INPUTS_PER_PACKET: usize = 4;
/// Most players in one snapshot.
pub const MAX_PLAYERS_PER_SNAPSHOT: usize = 8;
/// Most props in one snapshot (30 bytes each: fits the packet with the players, 35 bytes each).
pub const MAX_PROPS_PER_SNAPSHOT: usize = 30;

const KIND_HELLO: u8 = 1;
const KIND_INPUT: u8 = 2;
const KIND_C_BYE: u8 = 3;
const KIND_WELCOME: u8 = 16;
const KIND_REJECT: u8 = 17;
const KIND_SNAPSHOT: u8 = 18;
const KIND_S_BYE: u8 = 19;

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
}

impl RejectReason {
    fn to_u8(self) -> u8 {
        match self {
            RejectReason::Version => 1,
            RejectReason::WrongMap => 2,
            RejectReason::Full => 3,
        }
    }
    fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            1 => Ok(RejectReason::Version),
            2 => Ok(RejectReason::WrongMap),
            3 => Ok(RejectReason::Full),
            _ => Err(DecodeError::OutOfRange),
        }
    }
}

/// A join request (or a re-join: a non-zero `resume_token` asks to get the old player back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hello {
    /// Must equal [`PROTOCOL_VERSION`].
    pub version: u16,
    /// Hash of the client's copy of the map (`net::map_hash`).
    pub map_hash: u32,
    /// `0` human, `1` rat.
    pub character: u8,
    /// Token from an earlier [`Welcome`], or `0`.
    pub resume_token: u64,
}

/// A batch of recent inputs and what the client has received so far.
#[derive(Debug, Clone, PartialEq)]
pub struct InputPacket {
    /// Sequence number of the newest snapshot the client received (`0` = none yet).
    pub snapshot_ack: u32,
    /// Client clock, milliseconds (echoed back for round-trip time).
    pub client_time_ms: u32,
    /// Up to [`MAX_INPUTS_PER_PACKET`] inputs, oldest first.
    pub inputs: Vec<PlayerInput>,
}

/// Messages a client sends.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientMsg {
    /// Join or resume.
    Hello(Hello),
    /// Recent inputs + acknowledgement.
    Input(InputPacket),
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

/// Messages a server sends.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMsg {
    /// Joined.
    Welcome(Welcome),
    /// Refused.
    Reject(RejectReason),
    /// World state.
    Snapshot(Snapshot),
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
            }
            ClientMsg::Input(p) => {
                w.header(KIND_INPUT);
                w.u32(p.snapshot_ack);
                w.u32(p.client_time_ms);
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
            KIND_HELLO => ClientMsg::Hello(Hello { version: r.u16()?, map_hash: r.u32()?, character: r.u8()?, resume_token: r.u64()? }),
            KIND_INPUT => {
                let (snapshot_ack, client_time_ms) = (r.u32()?, r.u32()?);
                let n = r.u8()? as usize;
                if n > MAX_INPUTS_PER_PACKET {
                    return Err(DecodeError::OutOfRange);
                }
                let mut inputs = Vec::with_capacity(n);
                for _ in 0..n {
                    inputs.push(get_input(&mut r)?);
                }
                ClientMsg::Input(InputPacket { snapshot_ack, client_time_ms, inputs })
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
            }
            ServerMsg::Reject(r) => {
                w.header(KIND_REJECT);
                w.u8(r.to_u8());
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
                ServerMsg::Welcome(Welcome { player_id, token, tick_rate, snapshot_every, server_tick, spawn, character: r.u8()? })
            }
            KIND_REJECT => ServerMsg::Reject(RejectReason::from_u8(r.u8()?)?),
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

    #[test]
    fn every_message_round_trips() {
        roundtrip_c(ClientMsg::Hello(Hello { version: PROTOCOL_VERSION, map_hash: 0xdead_beef, character: 1, resume_token: u64::MAX }));
        roundtrip_c(ClientMsg::Input(InputPacket { snapshot_ack: 7, client_time_ms: 123456, inputs: vec![input(1), input(2), input(3), input(4)] }));
        roundtrip_c(ClientMsg::Input(InputPacket { snapshot_ack: 0, client_time_ms: 0, inputs: vec![] }));
        roundtrip_c(ClientMsg::Bye);
        roundtrip_s(ServerMsg::Welcome(Welcome {
            player_id: 3,
            token: 42,
            tick_rate: 60,
            snapshot_every: 2,
            server_tick: 999,
            spawn: [1.0, 0.0, -2.5, 1.57],
            character: 0,
        }));
        for r in [RejectReason::Version, RejectReason::WrongMap, RejectReason::Full] {
            roundtrip_s(ServerMsg::Reject(r));
        }
        roundtrip_s(ServerMsg::Bye);
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
        w.u16(0);
        w.u32(1);
        w.u8(0);
        w.u64(0);
        assert!(matches!(ClientMsg::decode(&b), Ok(ClientMsg::Hello(h)) if h.version == 0));
    }
}
