//! The loadout shooter's own part of a snapshot (ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or): the receiving player's kit, the
//! map's pickups, what was dropped, what flies, smoke and fire, explosions, and the objective modes' flags, bomb and events. It travels as the
//! [`GameMsg`] of kind [`KIND`], which the core protocol carries without reading ([`ArenaSnap::to_game_msg`], [`ArenaSnap::from_game_msg`]); each
//! player's stance byte is its [`super::protocol::PlayerSnap::extra`]. Until protocol v16 these were the protocol's own types.
//!
//! Used by Killchain alone and offered by nothing: it moves to `games/killchain` with the shooter (ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or,
//! system audit, docs/analysis/2026-10-14-system-audit.md).

use super::protocol::{DecodeError, GameMsg, R, W};

/// [`GameMsg::kind`] of the arena.
pub const KIND: u8 = 1;

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
    /// Capture the flag or search and destroy state (empty in the other modes).
    pub obj: ObjSnap,
}

/// One flag in a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FlagSnap {
    /// `0` at its base, `1` carried, `2` lying on the ground.
    pub state: u8,
    /// The carrier's player id (`255` = nobody).
    pub carrier: u8,
    /// Where it is.
    pub pos: [f32; 3],
    /// A dropped flag: ticks until it goes home by itself.
    pub left_ticks: u16,
}

/// Search and destroy in a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SndSnap {
    /// `0` freeze, `1` live, `2` round over.
    pub phase: u8,
    /// The round number, from 1.
    pub round: u8,
    /// The attacking team.
    pub attackers: u8,
    /// Ticks until the phase changes (the round clock while live).
    pub left_ticks: u16,
    /// `0` carried, `1` on the ground, `2` planted, `3` defused, `4` exploded.
    pub bomb: u8,
    /// The carrier's player id (`255` = nobody).
    pub carrier: u8,
    /// Where the bomb is.
    pub pos: [f32; 3],
    /// A planted bomb: the site's index.
    pub site: u8,
    /// A planted bomb: ticks until it goes off.
    pub fuse_ticks: u16,
    /// Plant or defuse progress, 0-255, for the HUD.
    pub progress: u8,
    /// After a round: the winning team.
    pub winner: u8,
}

/// An objective event (a flag taken, the bomb planted...) a client plays once per id.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ObjEventSnap {
    /// Unique, growing id.
    pub id: u16,
    /// What happened (`sim::objective::ev`).
    pub kind: u8,
    /// The team it concerns.
    pub team: u8,
    /// The player it concerns (`255` = nobody).
    pub slot: u8,
}

/// The objective modes' part of an [`ArenaSnap`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ObjSnap {
    /// `0` none, `1` capture the flag, `2` search and destroy.
    pub kind: u8,
    /// Capture the flag: the two flags.
    pub flags: [FlagSnap; 2],
    /// Search and destroy.
    pub snd: SndSnap,
    /// The last few events.
    pub events: Vec<ObjEventSnap>,
}

/// Most objective events a snapshot carries.
pub const MAX_OBJ_EVENTS: usize = 4;

impl ObjSnap {
    /// Bytes this block takes on the wire.
    pub fn wire_bytes(&self) -> usize {
        1 + match self.kind {
            1 => 2 * 10,
            2 => 18,
            _ => 0,
        } + if self.kind == 0 { 0 } else { 1 + self.events.len().min(MAX_OBJ_EVENTS) * 5 }
    }
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
            + self.obj.wire_bytes()
    }
}

/// Bytes of an [`OwnKit`] on the wire.
pub const OWN_BYTES: usize = 1 + 10 + 1 + 2 + 2 + 2 + 2 + 1 + 1 + 1 + 1 + 1 + 1;

impl ArenaSnap {
    /// The snapshot part that carries it.
    pub fn to_game_msg(&self) -> GameMsg {
        let mut bytes = Vec::with_capacity(self.wire_bytes());
        put_arena(&mut W(&mut bytes), self);
        GameMsg { kind: KIND, bytes }
    }

    /// The arena a snapshot part carries: `Ok(None)` for another game's part, an error for a malformed one (as a malformed snapshot is).
    pub fn from_game_msg(g: &GameMsg) -> Result<Option<ArenaSnap>, DecodeError> {
        if g.kind != KIND {
            return Ok(None);
        }
        let mut r = R { b: &g.bytes, i: 0 };
        let a = get_arena(&mut r)?;
        r.finish()?;
        Ok(Some(a))
    }
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
    put_obj(w, &a.obj);
}

fn put_obj(w: &mut W, o: &ObjSnap) {
    w.u8(o.kind);
    match o.kind {
        1 => {
            for f in &o.flags {
                w.u8(f.state);
                w.u8(f.carrier);
                put_pos(w, f.pos);
                w.u16(f.left_ticks);
            }
        }
        2 => {
            let s = &o.snd;
            for b in [s.phase, s.round, s.attackers] {
                w.u8(b);
            }
            w.u16(s.left_ticks);
            w.u8(s.bomb);
            w.u8(s.carrier);
            put_pos(w, s.pos);
            w.u8(s.site);
            w.u16(s.fuse_ticks);
            w.u8(s.progress);
            w.u8(s.winner);
        }
        _ => return,
    }
    let n = o.events.len().min(MAX_OBJ_EVENTS);
    w.u8(n as u8);
    for e in &o.events[..n] {
        w.u16(e.id);
        w.u8(e.kind);
        w.u8(e.team);
        w.u8(e.slot);
    }
}

fn get_obj(r: &mut R) -> Result<ObjSnap, DecodeError> {
    let kind = r.u8()?;
    let mut o = ObjSnap { kind, ..Default::default() };
    match kind {
        0 => return Ok(o),
        1 => {
            for f in o.flags.iter_mut() {
                *f = FlagSnap { state: r.u8()?.min(2), carrier: r.u8()?, pos: get_pos(r)?, left_ticks: r.u16()? };
            }
        }
        2 => {
            let (phase, round, attackers) = (r.u8()?.min(2), r.u8()?, r.u8()?.clamp(1, 2));
            let left_ticks = r.u16()?;
            let (bomb, carrier, pos) = (r.u8()?.min(4), r.u8()?, get_pos(r)?);
            o.snd = SndSnap {
                phase,
                round,
                attackers,
                left_ticks,
                bomb,
                carrier,
                pos,
                site: r.u8()?,
                fuse_ticks: r.u16()?,
                progress: r.u8()?,
                winner: r.u8()?.min(2),
            };
        }
        _ => return Err(DecodeError::OutOfRange),
    }
    let n = r.u8()? as usize;
    if n > MAX_OBJ_EVENTS {
        return Err(DecodeError::OutOfRange);
    }
    for _ in 0..n {
        o.events.push(ObjEventSnap { id: r.u16()?, kind: r.u8()?, team: r.u8()?.min(2), slot: r.u8()? });
    }
    Ok(o)
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
    let obj = get_obj(r)?;
    Ok(ArenaSnap { own, team_kills, pickups, dropped, projectiles, zones, fx, obj })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocol::{PlayerSnap, ServerMsg, Snapshot, MAX_PACKET};

    fn roundtrip_s(m: ServerMsg) {
        let mut b = Vec::new();
        m.encode(&mut b);
        assert!(b.len() <= MAX_PACKET, "{} bytes", b.len());
        assert_eq!(ServerMsg::decode(&b).unwrap(), m);
    }

    #[test]
    fn an_arena_travels_as_its_games_part_and_comes_back_whole() {
        let a = ArenaSnap { team_kills: [3, 5], ..Default::default() };
        let g = a.to_game_msg();
        assert_eq!(g.kind, KIND);
        assert_eq!(g.bytes.len(), a.wire_bytes(), "the size accounting matches the bytes written");
        assert_eq!(ArenaSnap::from_game_msg(&g).unwrap(), Some(a));
        assert_eq!(ArenaSnap::from_game_msg(&GameMsg { kind: KIND + 1, bytes: vec![9] }).unwrap(), None, "another game's part is not ours");
        let mut cut = g.clone();
        cut.bytes.pop();
        assert!(ArenaSnap::from_game_msg(&cut).is_err(), "a cut part is refused");
        let mut long = g;
        long.bytes.push(0);
        assert!(ArenaSnap::from_game_msg(&long).is_err(), "trailing bytes are refused");
    }

    #[test]
    fn a_game_part_costs_its_bytes_and_three_and_a_byte_per_player() {
        let players: Vec<PlayerSnap> = (0..5).map(|id| PlayerSnap { id, ..Default::default() }).collect();
        let plain = Snapshot { server_tick: 3, players: players.clone(), ..Default::default() };
        let arena = ArenaSnap { team_kills: [1, 2], ..Default::default() };
        let with = Snapshot { game: Some(arena.to_game_msg()), ..plain.clone() };
        let (mut a, mut b) = (Vec::new(), Vec::new());
        ServerMsg::Snapshot(plain).encode(&mut a);
        ServerMsg::Snapshot(with.clone()).encode(&mut b);
        assert_eq!(b.len() - a.len(), arena.wire_bytes() + 3 + players.len(), "the part, its kind and length, and each player's game byte");
        roundtrip_s(ServerMsg::Snapshot(with));
    }

    #[test]
    fn objective_state_round_trips_and_costs_nothing_in_the_kill_modes() {
        let snap = |obj: ObjSnap| {
            ServerMsg::Snapshot(Snapshot {
                server_tick: 9,
                players: vec![PlayerSnap { id: 1, ..Default::default() }],
                game: Some(ArenaSnap { obj, ..Default::default() }.to_game_msg()),
                ..Default::default()
            })
        };
        let ctf = ObjSnap {
            kind: 1,
            flags: [
                FlagSnap { state: 0, carrier: 255, pos: [-30.0, 0.0, 1.5], left_ticks: 0 },
                FlagSnap { state: 1, carrier: 4, pos: [12.5, 0.0, -3.0], left_ticks: 0 },
            ],
            snd: SndSnap::default(),
            events: vec![ObjEventSnap { id: 7, kind: 1, team: 2, slot: 4 }, ObjEventSnap { id: 8, kind: 4, team: 1, slot: 0 }],
        };
        let snd = ObjSnap {
            kind: 2,
            flags: Default::default(),
            snd: SndSnap {
                phase: 1,
                round: 3,
                attackers: 2,
                left_ticks: 4000,
                bomb: 2,
                carrier: 255,
                pos: [4.0, 0.0, 9.5],
                site: 1,
                fuse_ticks: 1200,
                progress: 128,
                winner: 0,
            },
            events: vec![ObjEventSnap { id: 2, kind: 7, team: 2, slot: 5 }],
        };
        roundtrip_s(snap(ctf.clone()));
        roundtrip_s(snap(snd.clone()));
        roundtrip_s(snap(ObjSnap::default()));
        // The size accounting matches the bytes written, and the kill modes pay one byte.
        for o in [ctf, snd, ObjSnap::default()] {
            let mut buf = Vec::new();
            put_obj(&mut W(&mut buf), &o);
            assert_eq!(buf.len(), o.wire_bytes(), "{o:?}");
        }
        assert_eq!(ObjSnap::default().wire_bytes(), 1);
        // An unknown kind is refused, not trusted.
        let mut bad = Vec::new();
        W(&mut bad).u8(3);
        assert!(get_obj(&mut R { b: &bad, i: 0 }).is_err());
    }
}
