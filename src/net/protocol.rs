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
//! roster, repeated), [`ServerMsg::RuleState`] (the current data-authored game state, repeated), [`ServerMsg::NoSession`], [`ServerMsg::Bye`].
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
/// Bumped on any incompatible change; a mismatched client is rejected. v8: `PlayerSnap::shots` + the per-client [`Feedback`] counters.
/// v9: the weapon numbers on the wire (indices of `weapons::Weapon::ALL`) changed when the silver revolver left the list.
/// v10 (ADR 0044): join proofs use a new domain (bound to the TLS exporter on QUIC), and a snapshot carries only as many props as the
/// client's transport datagram budget allows.
pub const PROTOCOL_VERSION: u16 = 14;
// v14 (ADR 2026-09-30-killchain-loadout-shooter): twelve players; inputs carry two flag bytes (aim, drop, weapon choice); a snapshot of a loadout match
// carries an [`ArenaSnap`] (your kit, pickups, projectiles, smoke, explosions); players carry a team and a stance byte; the lobby chooses a team; the
// status carries team scores and the match limits.
/// Largest message either side accepts, and the development UDP datagram budget (under a typical 1500-byte MTU). On QUIC the budget is
/// the connection's current `max_datagram_size` (about 1150 bytes on a fresh 1200-byte path MTU); messages above it travel on a stream.
pub const MAX_PACKET: usize = 1400;
/// Most inputs one packet carries (the newest is last).
pub const MAX_INPUTS_PER_PACKET: usize = 4;
/// Most players in one snapshot.
pub const MAX_PLAYERS_PER_SNAPSHOT: usize = 12;
/// Most props in one snapshot (30 bytes each: fits players of at most 44 bytes each).
pub const MAX_PROPS_PER_SNAPSHOT: usize = 27;

/// Longest player name, bytes.
pub const MAX_NAME: usize = 16;
/// Longest round-outcome text in a [`Status`], bytes.
pub const MAX_OUTCOME: usize = 32;
/// Most roster entries in a [`Status`] (a match holds at most this many players).
pub const MAX_ROSTER: usize = MAX_PLAYERS_PER_SNAPSHOT;
/// Most scene variables presented by a networked game.
pub const MAX_RULE_VARS: usize = 16;
/// Most hidden objects in a networked game's current state.
pub const MAX_RULE_HIDDEN: usize = 256;
/// Most objects with collision disabled by rules.
pub const MAX_RULE_COLLISION: usize = 64;
/// Longest variable, event or outcome name in network rule presentation, bytes.
pub const MAX_RULE_TEXT: usize = 32;
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
const KIND_RULE_STATE: u8 = 23;

/// The kind byte of a datagram (`None` if it is too short to have one or is not ours).
pub fn peek_kind(bytes: &[u8]) -> Option<u8> {
    (bytes.len() >= 5 && u16::from_le_bytes([bytes[0], bytes[1]]) == MAGIC).then(|| bytes[4])
}

/// Whether datagrams of this kind carry an authentication tag (everything after the handshake).
pub fn is_signed(kind: u8) -> bool {
    matches!(kind, KIND_INPUT | KIND_C_BYE | KIND_LOBBY | KIND_WELCOME | KIND_SNAPSHOT | KIND_S_BYE | KIND_STATUS | KIND_RULE_STATE)
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

/// The largest character byte a client may ask for. Bodies use `0..=5`; in a race the byte is the driver (`sim::kart::Driver::wire`, `0..=7`), and the server
/// clamps it to what the scene means.
pub const MAX_CHOICE: u8 = 7;

/// `0` human, `1` rat: how a [`Character`] travels on the wire.
pub fn character_to_wire(c: Character) -> u8 {
    match c {
        Character::Human => 0,
        Character::Rat => 1,
        Character::Wizard => 2,
        Character::Cowboy => 3,
        Character::Alien => 4,
        Character::Robot => 5,
        Character::Ridgeback => 6,
        Character::Nightfall => 7,
    }
}

/// The inverse of [`character_to_wire`]; anything but `1` is a human (a hostile value gets the default body).
pub fn character_from_wire(v: u8) -> Character {
    Character::ALL.get(v as usize).copied().unwrap_or(Character::Human)
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
    /// The server's TLS identity did not verify against the pinned fingerprint or CA (decided by the client; never retried, never
    /// downgraded to an insecure transport).
    ServerIdentity,
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
            RejectReason::ServerIdentity => 7,
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
            7 => Ok(RejectReason::ServerIdentity),
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
            RejectReason::ServerIdentity => {
                "The server's identity does not match the fingerprint or certificate authority you were given. Not connecting: ask its host."
            }
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
    /// The team the player wants: `0` whichever has room, `1` Ridgeback, `2` Nightfall.
    pub team: u8,
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
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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
    /// Horizontal momentum for prediction, x and z in metres per second.
    pub velocity: [f32; 2],
    /// The weapon in hand (`weapons::Weapon::wire`).
    pub weapon: u8,
    /// The prop this player is carrying ([`NO_PROP`] when none).
    pub held: u16,
    /// Loadout matches only: bits 0-1 team, bit 2 aiming down the sights, bit 3 reloading, bit 4 throwing a grenade, bit 5 blinded. Zero elsewhere.
    pub extra: u8,
    /// Hit points.
    pub hp: u8,
    /// Firearm shots this player has fired (wrapping): a client that sees it grow plays the shot (a sound, a muzzle flash) at their position.
    pub shots: u8,
    /// The player's kart, in a race match; `None` everywhere else.
    pub kart: Option<KartSnap>,
}

/// [`PlayerSnap::flags`] bit: the player is under spawn protection (cannot be hurt yet).
pub const FLAG_PROTECTED: u8 = 8;

/// `PlayerSnap::held` when the player carries nothing.
pub const NO_PROP: u16 = u16::MAX;

/// Wire-only bit of a player's flags byte: a [`KartSnap`] follows the player record (race matches only; other matches cost nothing).
const WIRE_KART: u8 = 0x40;
/// Wire-only bit of the player-count byte: a [`RaceSnap`] follows the counts (race matches only).
const WIRE_RACE: u8 = 0x80;
/// Wire-only bit of the player-count byte: every player carries a stance byte and an [`ArenaSnap`] ends the message (loadout matches only).
const WIRE_ARENA: u8 = 0x40;
/// Bytes a kart block adds to each player in a race snapshot.
pub const KART_BYTES: usize = 13;
/// Bytes the race header adds to a race snapshot, before its hazards (phase, countdown, clock, hazard count).
pub const RACE_HEADER_BYTES: usize = 12;
/// Bytes each hazard adds to a race snapshot.
pub const HAZARD_BYTES: usize = 10;
/// Most hazards a race snapshot carries (the simulation's pool size).
pub const MAX_HAZARDS_PER_SNAPSHOT: usize = 24;

/// A kart's own state in a snapshot (race matches): everything the driver's client needs to predict the kart exactly, and what every client needs to
/// draw and rank it. Boost and spin-out are in ticks, the drift charge in milliseconds, so the values are exact enough that a replay of unacknowledged
/// inputs on top of them reproduces the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KartSnap {
    /// Which of the eight drivers (`sim::kart::Driver::wire`).
    pub driver: u8,
    /// Ticks of boost left.
    pub boost_ticks: u8,
    /// Ticks of spin-out left.
    pub spin_ticks: u8,
    /// `-1` drifting left, `0` not, `1` right.
    pub drift_dir: i8,
    /// Whether the hop/drift button was held on the last processed input.
    pub jump_held: bool,
    /// Whether they have finished the race.
    pub finished: bool,
    /// Drift charge banked, milliseconds.
    pub drift_charge_ms: u16,
    /// Slipstream banked, 0..=200 (a Wolf's bar; 200 = full).
    pub slip: u8,
    /// Laps completed.
    pub lap: u8,
    /// The next gate they must cross.
    pub next_gate: u8,
    /// Their place in the standings, from 1 (`0` = not ranked).
    pub place: u8,
    /// The pickup held (`sim::kart::Item::wire`: 0 none, 1 Mushroom, 2 Acorn, 3 Bubble).
    pub item: u8,
    /// Ticks of Bubble shield left.
    pub shield_ticks: u16,
    /// Ticks until the driver's ability (the Beaver's Build) is ready again.
    pub ability_cooldown: u8,
    /// Whether the item button was held on the last processed input (an item is used on the press).
    pub attack_held: bool,
    /// Whether the ability button was held on the last processed input.
    pub interact_held: bool,
}

/// One hazard on the track in a race snapshot: an Acorn in flight or a plank.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HazardSnap {
    /// `0` Acorn, `1` plank.
    pub kind: u8,
    /// The slot that made it.
    pub owner: u8,
    /// Where it is (x, z).
    pub pos: [f32; 2],
}

impl KartSnap {
    /// The kart memory this describes, for a client to predict from (`sim::kart::KartState`).
    pub fn to_state(&self) -> crate::sim::kart::KartState {
        crate::sim::kart::KartState {
            boost_ticks: self.boost_ticks as u16,
            drift_dir: self.drift_dir,
            drift_charge: self.drift_charge_ms as f32 / 1000.0,
            spin_ticks: self.spin_ticks as u16,
            jump_held: self.jump_held,
            slip_charge: self.slip as f32 / 200.0,
            item: crate::sim::kart::Item::from_wire(self.item).unwrap_or_default(),
            shield_ticks: self.shield_ticks,
            ability_cooldown: self.ability_cooldown as u16,
            attack_held: self.attack_held,
            interact_held: self.interact_held,
        }
    }
}

/// The race as of one snapshot: where it is in its life and the clock.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RaceSnap {
    /// `0` countdown, `1` racing, `2` finished.
    pub phase: u8,
    /// Ticks of countdown left.
    pub countdown_ticks: u16,
    /// Ticks since the light went green.
    pub race_tick: u32,
    /// Which item boxes are ready to be taken (bit `i` = the race's `i`th `item_boxes` zone); a taken box is drawn gone.
    pub boxes_ready: u32,
    /// The Acorns in flight and planks on the track.
    pub hazards: Vec<HazardSnap>,
}

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
#[derive(Debug, Clone, PartialEq, Default)]
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
    /// What the server saw the receiving client do and suffer (wrapping counters a client turns into hit markers, damage flashes, sounds).
    pub fx: Feedback,
    /// Every connected player.
    pub players: Vec<PlayerSnap>,
    /// Props changed since the client's last acknowledged snapshot (possibly a subset; the rest follow).
    pub props: Vec<PropSnap>,
    /// The race, in a race match; `None` everywhere else.
    pub race: Option<RaceSnap>,
    /// The loadout match's world and the receiving player's kit; `None` in every other kind of match.
    pub arena: Option<ArenaSnap>,
}

/// The receiving player's own kit and what just happened to them, in a loadout match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OwnKit {
    /// The slot in hand (`sim::kit::Slot::to_wire`).
    pub sel: u8,
    /// The two gun slots as `(weapon wire + 1, rounds loaded, rounds in reserve)`; weapon `0` = empty.
    pub guns: [(u8, u16, u16); 2],
    /// The melee weapon (wire number).
    pub melee: u8,
    /// The two grenade slots (wire number + 1, `0` = empty).
    pub grenades: [u8; 2],
    /// Ticks until the reload finishes (`0` = not reloading).
    pub reload_left: u16,
    /// Ticks until the weapon can be used again.
    pub busy: u16,
    /// Ticks the screen stays whited out by a flashbang.
    pub flash_left: u16,
    /// Tenths of a second the last flash lasts in total (for the fade).
    pub flash_total: u8,
    /// Who killed the player (`255` = alive, or nobody).
    pub killed_by: u8,
    /// What killed them (wire number).
    pub killed_weapon: u8,
    /// Whether it was a headshot.
    pub killed_head: bool,
    /// Kills of the player's that were headshots (wrapping).
    pub headshots: u8,
    /// The player's team.
    pub team: u8,
}

impl OwnKit {
    /// The weapon in hand, from the slot and the kit.
    pub fn current_weapon(&self) -> Option<crate::weapons::Weapon> {
        use crate::weapons::Weapon;
        match self.sel {
            0 | 1 => {
                let g = self.guns[self.sel as usize].0;
                (g > 0).then(|| Weapon::from_wire(g - 1))
            }
            2 => Some(Weapon::from_wire(self.melee)),
            3 | 4 => {
                let g = self.grenades[self.sel as usize - 3];
                (g > 0).then(|| Weapon::from_wire(g - 1))
            }
            _ => None,
        }
    }
}

/// A weapon lying on the floor where somebody dropped it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DroppedSnap {
    /// Unique id.
    pub id: u16,
    /// Weapon wire number.
    pub weapon: u8,
    /// Where.
    pub pos: [f32; 3],
}

/// A rocket or grenade in flight or at rest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjSnap {
    /// Unique id.
    pub id: u16,
    /// Weapon wire number.
    pub weapon: u8,
    /// Where.
    pub pos: [f32; 3],
    /// How fast it moves, m/s (lets a client carry it between snapshots).
    pub vel: [f32; 3],
}

/// A smoke cloud or fire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneSnap {
    /// Unique id.
    pub id: u16,
    /// `0` smoke, `1` fire.
    pub kind: u8,
    /// Where.
    pub pos: [f32; 3],
    /// Radius, tenths of a metre.
    pub radius_dm: u8,
    /// Ticks until it ends.
    pub left_ticks: u16,
}

/// An explosion or a pop to show and hear once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FxSnap {
    /// Unique id (a client plays each id once).
    pub id: u16,
    /// `sim::ordnance::FxKind::to_wire`.
    pub kind: u8,
    /// Where.
    pub pos: [f32; 3],
    /// Size, tenths of a metre.
    pub size_dm: u8,
}

/// Most entries of each list an [`ArenaSnap`] carries.
pub const MAX_DROPPED_SNAP: usize = 12;
/// See [`MAX_DROPPED_SNAP`].
pub const MAX_PROJ_SNAP: usize = 12;
/// See [`MAX_DROPPED_SNAP`].
pub const MAX_ZONE_SNAP: usize = 6;
/// See [`MAX_DROPPED_SNAP`].
pub const MAX_FX_SNAP: usize = 6;
/// Bytes of the map-pickup availability mask (96 spots).
pub const PICKUP_MASK_BYTES: usize = 12;

/// The part of a loadout match's world a client draws and hears, beside the players.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArenaSnap {
    /// The receiving player's kit (`None` for a spectator).
    pub own: Option<OwnKit>,
    /// Kills per team.
    pub team_kills: [u16; 2],
    /// Bit `i` set: map pickup spot `i` has its item right now.
    pub pickups: [u8; PICKUP_MASK_BYTES],
    /// Dropped weapons.
    pub dropped: Vec<DroppedSnap>,
    /// Projectiles.
    pub projectiles: Vec<ProjSnap>,
    /// Smoke and fire.
    pub zones: Vec<ZoneSnap>,
    /// Explosions and pops of the last moments.
    pub fx: Vec<FxSnap>,
}

impl ArenaSnap {
    /// Bytes this block takes on the wire.
    pub fn wire_bytes(&self) -> usize {
        1 + self.own.map_or(0, |_| OWN_BYTES)
            + 4
            + PICKUP_MASK_BYTES
            + 4
            + self.dropped.len().min(MAX_DROPPED_SNAP) * 9
            + self.projectiles.len().min(MAX_PROJ_SNAP) * 16
            + self.zones.len().min(MAX_ZONE_SNAP) * 12
            + self.fx.len().min(MAX_FX_SNAP) * 10
    }
}

/// Bytes of an [`OwnKit`] on the wire.
pub const OWN_BYTES: usize = 1 + 10 + 1 + 2 + 2 + 2 + 2 + 1 + 1 + 1 + 1 + 1 + 1;

/// Per-client feedback carried by every [`Snapshot`]: counters that only ever grow (wrapping), so a lost snapshot loses nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Feedback {
    /// Attacks of the client's that damaged someone.
    pub hits: u8,
    /// Times the client was damaged.
    pub hurt: u8,
    /// Kills the client scored.
    pub kills: u8,
    /// Where the last damage to the client came from: a world yaw in 1/256 turns (0 = -Z, clockwise from above).
    pub bearing: u8,
    /// While the client is dead: tenths of a second until it respawns (`0` = alive), saturating.
    pub respawn: u8,
}

impl Feedback {
    /// [`Feedback::bearing`] as radians (the yaw convention of the rest of the engine).
    pub fn bearing_rad(&self) -> f32 {
        self.bearing as f32 * (std::f32::consts::TAU / 256.0)
    }

    /// Encodes a world yaw (radians) as a [`Feedback::bearing`] byte.
    pub fn bearing_from_rad(yaw: f32) -> u8 {
        (yaw.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * 256.0) as u32 as u8
    }
}

/// One line of the roster in a [`Status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    /// Player id (the slot used in snapshots).
    pub id: u8,
    /// The player's team (`0` = none, `1`, `2`).
    pub team: u8,
    /// [`ROSTER_READY`] | [`ROSTER_IN_ROUND`] | [`ROSTER_BOT`].
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
/// [`RosterEntry::flags`]: an AI player (always ready, never has a ping).
pub const ROSTER_BOT: u8 = 4;

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
    /// Kills per team (index = team - 1); zeros in a match without teams.
    pub team_score: [u16; 2],
    /// Team kills that end the round (`0` = no kill limit).
    pub kill_limit: u16,
    /// Round length in seconds (`0` = no time limit).
    pub time_limit_secs: u16,
    /// The winning team after a round (`0` = none or a draw).
    pub winner_team: u8,
}

/// `Status::end_code` before any round has ended.
pub const NO_END: u8 = 255;

/// One scene variable in a [`RuleState`].
#[derive(Debug, Clone, PartialEq)]
pub struct RuleVar {
    /// Authored variable name.
    pub name: String,
    /// Authoritative value.
    pub value: f64,
}

/// The complete bounded presentation state of data-authored rules. The server repeats it, so a
/// lost datagram, reconnect, or late join recovers current truth without replaying old events.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleState {
    /// Increasing (wrapping); an older state that arrives late is ignored.
    pub seq: u16,
    /// Match-flow round this state belongs to (`0` in open play).
    pub round: u16,
    /// Server simulation tick represented by this state.
    pub server_tick: u32,
    /// Scene variables, in authored order.
    pub vars: Vec<RuleVar>,
    /// Indices in the shared parsed scene for objects currently hidden.
    pub hidden: Vec<u16>,
    /// Indices of top-level objects whose authored collision is currently disabled.
    pub collision_disabled: Vec<u16>,
    /// Most recent non-terminal event, empty when none exists.
    pub event: String,
    /// Tick at which `event` occurred.
    pub event_tick: u32,
    /// Terminal rule outcome, empty while the game is running.
    pub outcome: String,
}

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
    /// Current data-authored rule presentation state.
    RuleState(RuleState),
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

/// A world coordinate in 5 cm steps (two bytes).
fn q16(v: f32) -> i16 {
    (v * 20.0).round().clamp(-32768.0, 32767.0) as i16
}

fn put_pos(w: &mut W, p: [f32; 3]) {
    for v in p {
        w.u16(q16(v) as u16);
    }
}

fn get_pos(r: &mut R) -> Result<[f32; 3], DecodeError> {
    Ok([r.u16()? as i16 as f32 / 20.0, r.u16()? as i16 as f32 / 20.0, r.u16()? as i16 as f32 / 20.0])
}

fn put_arena(w: &mut W, a: &ArenaSnap) {
    match &a.own {
        Some(o) => {
            w.u8(1);
            w.u8(o.sel);
            for (weapon, loaded, reserve) in o.guns {
                w.u8(weapon);
                w.u16(loaded);
                w.u16(reserve);
            }
            w.u8(o.melee);
            w.u8(o.grenades[0]);
            w.u8(o.grenades[1]);
            w.u16(o.reload_left);
            w.u16(o.busy);
            w.u16(o.flash_left);
            w.u8(o.flash_total);
            w.u8(o.killed_by);
            w.u8(o.killed_weapon);
            w.u8(o.killed_head as u8);
            w.u8(o.headshots);
            w.u8(o.team);
        }
        None => w.u8(0),
    }
    w.u16(a.team_kills[0]);
    w.u16(a.team_kills[1]);
    w.0.extend_from_slice(&a.pickups);
    let (nd, np, nz, nf) =
        (a.dropped.len().min(MAX_DROPPED_SNAP), a.projectiles.len().min(MAX_PROJ_SNAP), a.zones.len().min(MAX_ZONE_SNAP), a.fx.len().min(MAX_FX_SNAP));
    for n in [nd, np, nz, nf] {
        w.u8(n as u8);
    }
    for d in &a.dropped[..nd] {
        w.u16(d.id);
        w.u8(d.weapon);
        put_pos(w, d.pos);
    }
    for p in &a.projectiles[..np] {
        w.u16(p.id);
        w.u8(p.weapon);
        put_pos(w, p.pos);
        for v in p.vel {
            w.u16((v * 10.0).round().clamp(-32768.0, 32767.0) as i16 as u16);
        }
    }
    for z in &a.zones[..nz] {
        w.u16(z.id);
        w.u8(z.kind);
        put_pos(w, z.pos);
        w.u8(z.radius_dm);
        w.u16(z.left_ticks);
    }
    for f in &a.fx[..nf] {
        w.u16(f.id);
        w.u8(f.kind);
        put_pos(w, f.pos);
        w.u8(f.size_dm);
    }
}

fn get_arena(r: &mut R) -> Result<ArenaSnap, DecodeError> {
    let own = if r.u8()? != 0 {
        let sel = r.u8()?;
        let guns = [(r.u8()?, r.u16()?, r.u16()?), (r.u8()?, r.u16()?, r.u16()?)];
        let (melee, g0, g1) = (r.u8()?, r.u8()?, r.u8()?);
        let (reload_left, busy, flash_left, flash_total) = (r.u16()?, r.u16()?, r.u16()?, r.u8()?);
        let (killed_by, killed_weapon, killed_head, headshots, team) = (r.u8()?, r.u8()?, r.u8()? != 0, r.u8()?, r.u8()?.min(2));
        Some(OwnKit {
            sel,
            guns,
            melee,
            grenades: [g0, g1],
            reload_left,
            busy,
            flash_left,
            flash_total,
            killed_by,
            killed_weapon,
            killed_head,
            headshots,
            team,
        })
    } else {
        None
    };
    let team_kills = [r.u16()?, r.u16()?];
    let pickups: [u8; PICKUP_MASK_BYTES] = r.array()?;
    let (nd, np, nz, nf) = (r.u8()? as usize, r.u8()? as usize, r.u8()? as usize, r.u8()? as usize);
    if nd > MAX_DROPPED_SNAP || np > MAX_PROJ_SNAP || nz > MAX_ZONE_SNAP || nf > MAX_FX_SNAP {
        return Err(DecodeError::OutOfRange);
    }
    let mut dropped = Vec::with_capacity(nd);
    for _ in 0..nd {
        dropped.push(DroppedSnap { id: r.u16()?, weapon: r.u8()?, pos: get_pos(r)? });
    }
    let mut projectiles = Vec::with_capacity(np);
    for _ in 0..np {
        let (id, weapon, pos) = (r.u16()?, r.u8()?, get_pos(r)?);
        let vel = [r.u16()? as i16 as f32 / 10.0, r.u16()? as i16 as f32 / 10.0, r.u16()? as i16 as f32 / 10.0];
        projectiles.push(ProjSnap { id, weapon, pos, vel });
    }
    let mut zones = Vec::with_capacity(nz);
    for _ in 0..nz {
        zones.push(ZoneSnap { id: r.u16()?, kind: r.u8()?.min(1), pos: get_pos(r)?, radius_dm: r.u8()?, left_ticks: r.u16()? });
    }
    let mut fx = Vec::with_capacity(nf);
    for _ in 0..nf {
        fx.push(FxSnap { id: r.u16()?, kind: r.u8()?, pos: get_pos(r)?, size_dm: r.u8()? });
    }
    Ok(ArenaSnap { own, team_kills, pickups, dropped, projectiles, zones, fx })
}

fn put_input(w: &mut W, i: &PlayerInput) {
    w.u32(i.seq);
    w.u16(i.flags());
    w.u8(i.forward as u8);
    w.u8(i.strafe as u8);
    w.f32(i.yaw);
    w.f32(i.pitch);
}

fn get_input(r: &mut R) -> Result<PlayerInput, DecodeError> {
    let seq = r.u32()?;
    let flags = r.u16()?;
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
                w.u8(l.team);
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
            KIND_LOBBY => ClientMsg::Lobby(LobbyCmd {
                ready: r.u8()? != 0,
                character: r.u8()?,
                team: r.u8()?.min(2),
                round_ack: r.u16()?,
                client_time_ms: r.u32()?,
                rtt_ms: r.u16()?,
            }),
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
                    w.u8(e.team);
                    w.u8(e.flags);
                    w.u8(e.character);
                    w.u16(e.ping_ms);
                    w.u16(e.score);
                    w.text(&e.name, MAX_NAME);
                }
                w.u16(st.team_score[0]);
                w.u16(st.team_score[1]);
                w.u16(st.kill_limit);
                w.u16(st.time_limit_secs);
                w.u8(st.winner_team);
            }
            ServerMsg::RuleState(st) => {
                w.header(KIND_RULE_STATE);
                w.u16(st.seq);
                w.u16(st.round);
                w.u32(st.server_tick);
                let nv = st.vars.len().min(MAX_RULE_VARS);
                w.u8(nv as u8);
                for v in &st.vars[..nv] {
                    w.text(&v.name, MAX_RULE_TEXT);
                    w.u64(v.value.to_bits());
                }
                let nh = st.hidden.len().min(MAX_RULE_HIDDEN);
                w.u16(nh as u16);
                for &id in &st.hidden[..nh] {
                    w.u16(id);
                }
                let nc = st.collision_disabled.len().min(MAX_RULE_COLLISION);
                w.u16(nc as u16);
                for &id in &st.collision_disabled[..nc] {
                    w.u16(id);
                }
                w.text(&st.event, MAX_RULE_TEXT);
                w.u32(st.event_tick);
                w.text(&st.outcome, MAX_RULE_TEXT);
            }
            ServerMsg::Snapshot(s) => {
                w.header(KIND_SNAPSHOT);
                w.u32(s.seq);
                w.u32(s.server_tick);
                w.u32(s.ack_input_seq);
                w.u32(s.echo_time_ms);
                w.u16(s.echo_hold_ms);
                for b in [s.fx.hits, s.fx.hurt, s.fx.kills, s.fx.bearing, s.fx.respawn] {
                    w.u8(b);
                }
                let np = s.players.len().min(MAX_PLAYERS_PER_SNAPSHOT);
                w.u8(np as u8 | if s.race.is_some() { WIRE_RACE } else { 0 } | if s.arena.is_some() { WIRE_ARENA } else { 0 });
                let nq = s.props.len().min(MAX_PROPS_PER_SNAPSHOT);
                w.u8(nq as u8);
                if let Some(race) = &s.race {
                    w.u8(race.phase);
                    w.u16(race.countdown_ticks);
                    w.u32(race.race_tick);
                    w.u32(race.boxes_ready);
                    let nh = race.hazards.len().min(MAX_HAZARDS_PER_SNAPSHOT);
                    w.u8(nh as u8);
                    for h in &race.hazards[..nh] {
                        w.u8(h.kind);
                        w.u8(h.owner);
                        w.f32(h.pos[0]);
                        w.f32(h.pos[1]);
                    }
                }
                for p in &s.players[..np] {
                    let moving = p.velocity != [0.0; 2];
                    w.u8(p.id);
                    w.u8(p.character);
                    // Bit 7 is a wire-only velocity-presence flag, bit 6 a wire-only kart-block flag.
                    w.u8((p.flags & 0x3f) | if moving { 0x80 } else { 0 } | if p.kart.is_some() { WIRE_KART } else { 0 });
                    for v in p.pos {
                        w.f32(v);
                    }
                    w.f32(p.yaw);
                    w.f32(p.pitch);
                    w.f32(p.speed);
                    w.f32(p.vy);
                    if moving {
                        w.f32(p.velocity[0]);
                        w.f32(p.velocity[1]);
                    }
                    w.u8(p.weapon);
                    w.u16(p.held);
                    w.u8(p.hp);
                    w.u8(p.shots);
                    if s.arena.is_some() {
                        w.u8(p.extra);
                    }
                    if let Some(k) = &p.kart {
                        w.u8(k.driver);
                        w.u8(k.boost_ticks);
                        w.u8(k.spin_ticks);
                        w.u8((k.drift_dir.signum() + 1) as u8
                            | (k.jump_held as u8) << 2
                            | (k.finished as u8) << 3
                            | (k.attack_held as u8) << 4
                            | (k.interact_held as u8) << 5
                            | (k.item & 3) << 6);
                        w.u16(k.drift_charge_ms);
                        w.u8(k.slip);
                        w.u8(k.lap);
                        w.u8(k.next_gate);
                        w.u8(k.place);
                        w.u16(k.shield_ticks);
                        w.u8(k.ability_cooldown);
                    }
                }
                for q in &s.props[..nq] {
                    w.u16(q.id);
                    for v in q.pos.into_iter().chain(q.rot) {
                        w.f32(v);
                    }
                }
                if let Some(a) = &s.arena {
                    put_arena(&mut w, a);
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
                    roster.push(RosterEntry {
                        id: r.u8()?,
                        team: r.u8()?.min(2),
                        flags: r.u8()?,
                        character: r.u8()?,
                        ping_ms: r.u16()?,
                        score: r.u16()?,
                        name: r.text(MAX_NAME)?,
                    });
                }
                let team_score = [r.u16()?, r.u16()?];
                let (kill_limit, time_limit_secs, winner_team) = (r.u16()?, r.u16()?, r.u8()?.min(2));
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
                    team_score,
                    kill_limit,
                    time_limit_secs,
                    winner_team,
                })
            }
            KIND_RULE_STATE => {
                let (seq, round, server_tick) = (r.u16()?, r.u16()?, r.u32()?);
                let nv = r.u8()? as usize;
                if nv > MAX_RULE_VARS {
                    return Err(DecodeError::OutOfRange);
                }
                let mut vars = Vec::with_capacity(nv);
                for _ in 0..nv {
                    vars.push(RuleVar { name: r.text(MAX_RULE_TEXT)?, value: f64::from_bits(r.u64()?) });
                }
                let nh = r.u16()? as usize;
                if nh > MAX_RULE_HIDDEN {
                    return Err(DecodeError::OutOfRange);
                }
                let mut hidden = Vec::with_capacity(nh);
                for _ in 0..nh {
                    hidden.push(r.u16()?);
                }
                let nc = r.u16()? as usize;
                if nc > MAX_RULE_COLLISION {
                    return Err(DecodeError::OutOfRange);
                }
                let mut collision_disabled = Vec::with_capacity(nc);
                for _ in 0..nc {
                    collision_disabled.push(r.u16()?);
                }
                ServerMsg::RuleState(RuleState {
                    seq,
                    round,
                    server_tick,
                    vars,
                    hidden,
                    collision_disabled,
                    event: r.text(MAX_RULE_TEXT)?,
                    event_tick: r.u32()?,
                    outcome: r.text(MAX_RULE_TEXT)?,
                })
            }
            KIND_SNAPSHOT => {
                let (seq, server_tick, ack_input_seq, echo_time_ms) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let echo_hold_ms = r.u16()?;
                let fx = Feedback { hits: r.u8()?, hurt: r.u8()?, kills: r.u8()?, bearing: r.u8()?, respawn: r.u8()? };
                let (np_byte, nq) = (r.u8()?, r.u8()? as usize);
                let (np, has_race, has_arena) = ((np_byte & !(WIRE_RACE | WIRE_ARENA)) as usize, np_byte & WIRE_RACE != 0, np_byte & WIRE_ARENA != 0);
                if np > MAX_PLAYERS_PER_SNAPSHOT || nq > MAX_PROPS_PER_SNAPSHOT {
                    return Err(DecodeError::OutOfRange);
                }
                let race = if has_race {
                    let (phase, countdown_ticks, race_tick, boxes_ready, nh) = (r.u8()?, r.u16()?, r.u32()?, r.u32()?, r.u8()? as usize);
                    if phase > 2 || nh > MAX_HAZARDS_PER_SNAPSHOT {
                        return Err(DecodeError::OutOfRange);
                    }
                    let mut hazards = Vec::with_capacity(nh);
                    for _ in 0..nh {
                        let (kind, owner) = (r.u8()?, r.u8()?);
                        if kind > 1 {
                            return Err(DecodeError::OutOfRange);
                        }
                        hazards.push(HazardSnap { kind, owner, pos: [r.f32()?, r.f32()?] });
                    }
                    Some(RaceSnap { phase, countdown_ticks, race_tick, boxes_ready, hazards })
                } else {
                    None
                };
                let mut players = Vec::with_capacity(np);
                for _ in 0..np {
                    let id = r.u8()?;
                    let character = r.u8()?;
                    let flags = r.u8()?;
                    players.push(PlayerSnap {
                        id,
                        character,
                        flags: flags & 0x3f,
                        pos: [r.f32()?, r.f32()?, r.f32()?],
                        yaw: r.f32()?,
                        pitch: r.f32()?,
                        speed: r.f32()?,
                        vy: r.f32()?,
                        velocity: if flags & 0x80 != 0 { [r.f32()?, r.f32()?] } else { [0.0; 2] },
                        weapon: r.u8()?,
                        held: r.u16()?,
                        hp: r.u8()?,
                        shots: r.u8()?,
                        extra: if has_arena { r.u8()? } else { 0 },
                        kart: if flags & WIRE_KART != 0 {
                            let (driver, boost_ticks, spin_ticks, status) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
                            let (drift_charge_ms, slip, lap, next_gate, place) = (r.u16()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?);
                            let (shield_ticks, ability_cooldown) = (r.u16()?, r.u8()?);
                            if status & 3 > 2 || driver > 7 || slip > 200 {
                                return Err(DecodeError::OutOfRange);
                            }
                            Some(KartSnap {
                                driver,
                                boost_ticks,
                                spin_ticks,
                                drift_dir: (status & 3) as i8 - 1,
                                jump_held: status & 4 != 0,
                                finished: status & 8 != 0,
                                drift_charge_ms,
                                slip,
                                lap,
                                next_gate,
                                place,
                                item: status >> 6,
                                shield_ticks,
                                ability_cooldown,
                                attack_held: status & 16 != 0,
                                interact_held: status & 32 != 0,
                            })
                        } else {
                            None
                        },
                    });
                }
                let mut props = Vec::with_capacity(nq);
                for _ in 0..nq {
                    props.push(PropSnap { id: r.u16()?, pos: [r.f32()?, r.f32()?, r.f32()?], rot: [r.f32()?, r.f32()?, r.f32()?, r.f32()?] });
                }
                let arena = if has_arena { Some(get_arena(&mut r)?) } else { None };
                ServerMsg::Snapshot(Snapshot { seq, server_tick, ack_input_seq, echo_time_ms, echo_hold_ms, fx, players, props, race, arena })
            }
            KIND_S_BYE => ServerMsg::Bye,
            k => return Err(DecodeError::UnknownKind(k)),
        };
        r.finish()?;
        Ok(msg)
    }
}

/// Maximum encoded bytes with `players` players and `props` props (zero velocity saves eight bytes).
pub const fn snapshot_bytes(players: usize, props: usize) -> usize {
    5 + 16 + 2 + 5 + 2 + players * 44 + props * 30
}

/// How many props a snapshot with `players` players may carry within a datagram budget of `budget` bytes (the transport's
/// `max_datagram` minus any tag), never more than [`MAX_PROPS_PER_SNAPSHOT`]. Props left out stay unconfirmed and go next time.
pub const fn snapshot_prop_budget(budget: usize, players: usize) -> usize {
    let room = budget.saturating_sub(snapshot_bytes(players, 0)) / (snapshot_bytes(0, 1) - snapshot_bytes(0, 0));
    if room < MAX_PROPS_PER_SNAPSHOT {
        room
    } else {
        MAX_PROPS_PER_SNAPSHOT
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn snapshots_are_sized_to_the_transport_budget() {
        // UDP (1400 minus the 8-byte tag): the old full snapshot fits.
        assert_eq!(snapshot_prop_budget(1400 - 8, MAX_PLAYERS_PER_SNAPSHOT), MAX_PROPS_PER_SNAPSHOT);
        // QUIC at its guaranteed 1200-byte path (~1150-byte datagrams): 8 players and fewer props, and the result fits.
        for budget in [1100usize, 1150, 1162, 1200] {
            let n = snapshot_prop_budget(budget, MAX_PLAYERS_PER_SNAPSHOT);
            assert!(n < MAX_PROPS_PER_SNAPSHOT && snapshot_bytes(MAX_PLAYERS_PER_SNAPSHOT, n) <= budget, "{budget}: {n}");
            assert!(snapshot_bytes(MAX_PLAYERS_PER_SNAPSHOT, n + 1) > budget);
        }
        assert_eq!(snapshot_prop_budget(0, 8), 0, "no connection, no props");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn snapshots_have_a_default_so_a_new_field_costs_one_line_in_a_test() {
        // Build test snapshots as `Snapshot { server_tick: 3, players: vec![PlayerSnap { id: 1, ..Default::default() }], ..Default::default() }`: adding a wire field
        // then needs no edit in the twenty literals that do not care about it (friction found building Great Outdoors).
        let s = Snapshot { server_tick: 3, players: vec![PlayerSnap { id: 1, ..Default::default() }], ..Default::default() };
        assert_eq!((s.server_tick, s.players.len(), s.players[0].id, s.players[0].kart.is_none(), s.race.is_none()), (3, 1, 1, true, true));
    }

    use super::*;

    fn input(seq: u32) -> PlayerInput {
        PlayerInput {
            seq,
            analog: false,
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
            aim: true,
            drop: false,
            select: 3,
        }
    }

    #[test]
    fn analog_input_keeps_stick_precision_on_the_wire() {
        let mut packet = InputPacket::default();
        packet.inputs.push(PlayerInput { analog: true, forward: 63, strafe: -101, ..Default::default() });
        roundtrip_c(ClientMsg::Input(packet));
    }

    #[test]
    fn every_playable_character_has_a_distinct_wire_identity() {
        for who in Character::ALL {
            assert_eq!(character_from_wire(character_to_wire(who)), who);
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
            team_score: [7, 9],
            kill_limit: 50,
            time_limit_secs: 600,
            winner_team: 2,
            roster: (0..MAX_ROSTER as u8)
                .map(|i| RosterEntry {
                    id: i,
                    team: 1 + i % 2,
                    flags: ROSTER_READY | ROSTER_IN_ROUND,
                    character: i % 2,
                    ping_ms: 20 + i as u16,
                    score: i as u16,
                    name: sanitize_name(&format!("player-with-a-long-name-{i}")),
                })
                .collect(),
        }
    }

    fn full_rule_state() -> RuleState {
        RuleState {
            seq: u16::MAX,
            round: 9,
            server_tick: 123_456,
            vars: (0..MAX_RULE_VARS).map(|i| RuleVar { name: format!("variable_{i:02}_with_long_name"), value: i as f64 + 0.5 }).collect(),
            hidden: (0..MAX_RULE_HIDDEN as u16).collect(),
            collision_disabled: (0..MAX_RULE_COLLISION as u16).collect(),
            event: "collected_the_last_object".into(),
            event_tick: 123_450,
            outcome: "a_wonderful_victory".into(),
        }
    }

    #[test]
    fn a_status_and_a_welcome_fit_a_packet_with_room_for_the_tag() {
        let mut b = Vec::new();
        ServerMsg::Status(full_status()).encode(&mut b);
        assert!(b.len() + super::super::auth::TAG_LEN <= MAX_PACKET, "{} bytes", b.len());
        assert!(b.len() < 400, "a full 12-player roster is {} bytes; keep Status small (it is sent 5x a second to everyone)", b.len());
    }

    #[test]
    fn a_full_rule_state_is_bounded_and_fits_one_datagram() {
        let mut b = Vec::new();
        ServerMsg::RuleState(full_rule_state()).encode(&mut b);
        assert!(b.len() + super::super::auth::TAG_LEN <= MAX_PACKET, "{} bytes", b.len());
        assert_eq!(ServerMsg::decode(&b).unwrap(), ServerMsg::RuleState(full_rule_state()));
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
        roundtrip_c(ClientMsg::Lobby(LobbyCmd { ready: true, character: 1, team: 2, round_ack: 2, client_time_ms: 99, rtt_ms: 31 }));
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
        for r in [
            RejectReason::Version,
            RejectReason::WrongMap,
            RejectReason::Full,
            RejectReason::BadKey,
            RejectReason::NeedsKey,
            RejectReason::ServerIsOpen,
            RejectReason::ServerIdentity,
        ] {
            roundtrip_s(ServerMsg::Reject(r));
        }
        roundtrip_s(ServerMsg::Bye);
        roundtrip_s(ServerMsg::NoSession);
        roundtrip_s(ServerMsg::Challenge { cookie: u64::MAX, requires_key: true });
        roundtrip_s(ServerMsg::Status(full_status()));
        roundtrip_s(ServerMsg::RuleState(full_rule_state()));
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
                velocity: [3.0, -2.0],
                weapon: 1,
                held: 7,
                hp: 80,
                shots: 200 + i,
                extra: 0,
                kart: None,
            })
            .collect();
        let props = (0..MAX_PROPS_PER_SNAPSHOT as u16).map(|i| PropSnap { id: i, pos: [1.0, 2.0, 3.0], rot: [0.0, 0.0, 0.0, 1.0] }).collect();
        let fx = Feedback { hits: 250, hurt: 3, kills: 255, bearing: 128, respawn: 27 };
        roundtrip_s(ServerMsg::Snapshot(Snapshot {
            seq: 5,
            server_tick: 100,
            ack_input_seq: 90,
            echo_time_ms: 77,
            echo_hold_ms: 12,
            fx,
            players,
            props,
            race: None,
            arena: None,
        }));
    }

    #[test]
    fn a_full_snapshot_fits_in_one_packet_and_the_size_formula_is_right() {
        let s = Snapshot {
            seq: 1,
            server_tick: 1,
            ack_input_seq: 1,
            echo_time_ms: 1,
            echo_hold_ms: 0,
            fx: Feedback::default(),
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
                    velocity: [1.0; 2],
                    weapon: 0,
                    held: NO_PROP,
                    hp: 100,
                    shots: 0,
                    extra: 0,
                    kart: None,
                };
                MAX_PLAYERS_PER_SNAPSHOT
            ],
            props: vec![PropSnap { id: 0, pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0] }; MAX_PROPS_PER_SNAPSHOT],
            race: None,
            arena: None,
        };
        let mut b = Vec::new();
        ServerMsg::Snapshot(s).encode(&mut b);
        assert_eq!(b.len(), snapshot_bytes(MAX_PLAYERS_PER_SNAPSHOT, MAX_PROPS_PER_SNAPSHOT));
        assert!(b.len() < MAX_PACKET, "{}", b.len());
    }

    fn race_snapshot(props: usize) -> Snapshot {
        race_snapshot_with(props, 3)
    }

    fn race_snapshot_with(props: usize, hazards: usize) -> Snapshot {
        let players = (0..MAX_PLAYERS_PER_SNAPSHOT as u8)
            .map(|i| PlayerSnap {
                id: i,
                character: 0,
                flags: 0,
                pos: [i as f32, 0.0, -3.0],
                yaw: 1.5,
                pitch: 0.0,
                speed: 20.0,
                vy: 0.0,
                velocity: [20.0, 1.0],
                weapon: 0,
                held: NO_PROP,
                hp: 100,
                shots: 0,
                extra: 0,
                kart: Some(KartSnap {
                    driver: i % 8,
                    boost_ticks: 30 + i,
                    spin_ticks: i * 5,
                    drift_dir: i as i8 % 3 - 1,
                    jump_held: i % 2 == 0,
                    finished: i == 7,
                    drift_charge_ms: 3200 + i as u16,
                    slip: 200 - i,
                    lap: i % 3,
                    next_gate: i,
                    place: i + 1,
                    item: i % 4,
                    shield_ticks: 300 - i as u16,
                    ability_cooldown: 240 - i,
                    attack_held: i % 3 == 0,
                    interact_held: i % 2 == 1,
                }),
            })
            .collect();
        Snapshot {
            seq: 9,
            server_tick: 1234,
            ack_input_seq: 1200,
            echo_time_ms: 5,
            echo_hold_ms: 1,
            fx: Feedback::default(),
            players,
            props: vec![PropSnap { id: 0, pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0] }; props],
            race: Some(RaceSnap {
                phase: 1,
                countdown_ticks: 0,
                race_tick: 4_000_000_000,
                boxes_ready: 0b1010_0101,
                hazards: (0..hazards).map(|i| HazardSnap { kind: (i % 2) as u8, owner: (i % 8) as u8, pos: [i as f32 * 3.5, -40.0 + i as f32] }).collect(),
            }),
            arena: None,
        }
    }

    #[test]
    fn a_race_snapshot_carries_every_kart_and_the_race_and_costs_nothing_elsewhere() {
        roundtrip_s(ServerMsg::Snapshot(race_snapshot(0)));
        // Twelve karts leave room for ten props (the server takes the race overhead out of the props' budget).
        roundtrip_s(ServerMsg::Snapshot(race_snapshot(10)));
        roundtrip_s(ServerMsg::Snapshot(race_snapshot_with(0, 0)));
        roundtrip_s(ServerMsg::Snapshot(race_snapshot_with(0, MAX_HAZARDS_PER_SNAPSHOT)));
        let mut race = Vec::new();
        ServerMsg::Snapshot(race_snapshot(0)).encode(&mut race);
        let mut plain_snapshot = race_snapshot(0);
        plain_snapshot.race = None;
        plain_snapshot.players.iter_mut().for_each(|p| p.kart = None);
        let mut plain = Vec::new();
        ServerMsg::Snapshot(plain_snapshot).encode(&mut plain);
        assert_eq!(race.len() - plain.len(), RACE_HEADER_BYTES + MAX_PLAYERS_PER_SNAPSHOT * KART_BYTES + 3 * HAZARD_BYTES, "exactly the documented overhead");
        assert_eq!(plain.len(), snapshot_bytes(MAX_PLAYERS_PER_SNAPSHOT, 0), "a match without a race is byte for byte what it was");
        // The server takes the overhead out of the props' room, so a race snapshot always fits the transport's datagram.
        for budget in [1200usize, 1252, 1400] {
            // The worst case: eight karts and a full pool of hazards.
            let overhead = RACE_HEADER_BYTES + KART_BYTES * MAX_PLAYERS_PER_SNAPSHOT + HAZARD_BYTES * MAX_HAZARDS_PER_SNAPSHOT;
            let props = snapshot_prop_budget(budget - overhead, MAX_PLAYERS_PER_SNAPSHOT);
            let mut b = Vec::new();
            ServerMsg::Snapshot(race_snapshot_with(props, MAX_HAZARDS_PER_SNAPSHOT)).encode(&mut b);
            assert!(b.len() <= budget, "budget {budget}: {} bytes with {props} props", b.len());
        }
        assert!(race.len() < MAX_PACKET);
    }

    #[test]
    fn a_kart_block_with_impossible_values_is_rejected() {
        let bad = |edit: &dyn Fn(&mut Snapshot)| {
            let mut s = race_snapshot(0);
            edit(&mut s);
            let mut b = Vec::new();
            ServerMsg::Snapshot(s).encode(&mut b);
            assert!(ServerMsg::decode(&b).is_err());
        };
        bad(&|s| s.players[0].kart.as_mut().unwrap().driver = 8);
        bad(&|s| s.players[0].kart.as_mut().unwrap().slip = 201);
        bad(&|s| s.race.as_mut().unwrap().phase = 3);
        bad(&|s| s.race.as_mut().unwrap().hazards[0].kind = 2);
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
            fx: Feedback::default(),
            players: vec![],
            props: vec![PropSnap { id: 1, pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0] }],
            race: None,
            arena: None,
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
        for _ in 0..5 {
            w.u8(0); // the feedback counters
        }
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
