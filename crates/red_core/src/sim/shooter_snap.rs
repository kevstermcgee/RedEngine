//! The loadout shooter's own part of a snapshot, built from the match ([`ShooterGame::snapshot_part`]): the receiving player's kit and what
//! just happened to them, which map pickups are there, what lies dropped, flies, burns or just went off, and the objective modes' flags,
//! bomb and events. The encoding is `net::arena_wire`'s; the core protocol carries it as a `GameMsg` without reading it.
//!
//! Used by Killchain alone and offered by nothing: it moves to `games/killchain` (ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or,
//! system audit, docs/analysis/2026-10-14-system-audit.md).

use super::match_sim::MatchSim;
use super::shooter::ShooterGame;
use crate::net::arena_wire::{
    ArenaSnap, DroppedSnap, FlagSnap, FxSnap, ObjEventSnap, ObjSnap, OwnKit, ProjSnap, SndSnap, ZoneSnap, MAX_DROPPED_SNAP, MAX_FX_SNAP, MAX_PROJ_SNAP,
    MAX_ZONE_SNAP, PICKUP_MASK_BYTES,
};
use crate::sim::game::MatchGame;

impl ShooterGame {
    /// The loadout match's world for `viewer`: their own kit, which map pickups are there, what lies dropped, flies, burns or just went off.
    pub(super) fn arena_snap(&self, sim: &MatchSim, viewer: usize) -> Option<ArenaSnap> {
        let arena = &self.arena;
        let now = sim.tick();
        let own = sim.player(viewer).and_then(|p| {
            let kit = self.kits.get(viewer)?.as_ref()?;
            let gun = |i: usize| kit.guns[i].map_or((0, 0, 0), |g| (g.weapon.wire() + 1, g.loaded, g.reserve));
            let (killed_by, killed_weapon, killed_head) = match p.combat.killed_by {
                Some((by, w, head)) if p.combat.is_dead() => (by, w.wire(), head),
                _ => (255, 0, false),
            };
            Some(OwnKit {
                sel: kit.sel.to_wire(),
                guns: [gun(0), gun(1)],
                melee: kit.melee.wire(),
                grenades: [kit.grenades[0].map_or(0, |w| w.wire() + 1), kit.grenades[1].map_or(0, |w| w.wire() + 1)],
                reload_left: kit.reload_left,
                busy: kit.busy,
                flash_left: p.combat.flash_until.saturating_sub(now).min(u16::MAX as u64) as u16,
                flash_total: (p.combat.flash_total * 10.0).round().clamp(0.0, 255.0) as u8,
                killed_by,
                killed_weapon,
                killed_head,
                headshots: p.combat.headshots as u8,
                team: p.team,
            })
        });
        let mut pickups = [0u8; PICKUP_MASK_BYTES];
        for (i, m) in arena.pickups.iter().enumerate().take(PICKUP_MASK_BYTES * 8) {
            if m.taken_until.is_none() {
                pickups[i / 8] |= 1 << (i % 8);
            }
        }
        let score = self.team_score().unwrap_or([0; 2]);
        Some(ArenaSnap {
            own,
            obj: self.obj_snap(now),
            team_kills: [score[0].min(u16::MAX as u32) as u16, score[1].min(u16::MAX as u32) as u16],
            pickups,
            dropped: arena.dropped.iter().take(MAX_DROPPED_SNAP).map(|d| DroppedSnap { id: d.id, weapon: d.weapon.wire(), pos: d.pos.to_array() }).collect(),
            projectiles: arena
                .projectiles
                .iter()
                .take(MAX_PROJ_SNAP)
                .map(|p| ProjSnap { id: p.id, weapon: p.weapon.wire(), pos: p.pos.to_array(), vel: p.vel.to_array() })
                .collect(),
            zones: arena
                .zones
                .iter()
                .take(MAX_ZONE_SNAP)
                .map(|z| ZoneSnap {
                    id: z.id,
                    kind: (z.kind == crate::sim::ordnance::ZoneKind::Fire) as u8,
                    pos: z.pos.to_array(),
                    radius_dm: (z.radius * 10.0).round().clamp(0.0, 255.0) as u8,
                    left_ticks: z.until.saturating_sub(now).min(u16::MAX as u64) as u16,
                })
                .collect(),
            fx: arena
                .fx
                .iter()
                .rev()
                .take(MAX_FX_SNAP)
                .rev()
                .map(|f| FxSnap { id: f.id, kind: f.kind.to_wire(), pos: f.pos.to_array(), size_dm: (f.size * 10.0).round().clamp(0.0, 255.0) as u8 })
                .collect(),
        })
    }

    /// The objective modes' state as the wire carries it (empty outside capture the flag and search and destroy).
    fn obj_snap(&self, now: u64) -> ObjSnap {
        use crate::sim::objective::{BombState, FlagState, RoundPhase};
        let arena = &self.arena;
        let left = |t: u64| t.saturating_sub(now).min(u16::MAX as u64) as u16;
        let events: Vec<ObjEventSnap> = arena.obj_events.iter().map(|e| ObjEventSnap { id: e.id, kind: e.kind, team: e.team, slot: e.slot }).collect();
        if let Some(ctf) = &arena.ctf {
            let flag = |f: &crate::sim::objective::Flag| match f.state {
                FlagState::Home => FlagSnap { state: 0, carrier: 255, pos: f.home.to_array(), left_ticks: 0 },
                FlagState::Carried { slot, pos } => FlagSnap { state: 1, carrier: slot, pos: pos.to_array(), left_ticks: 0 },
                FlagState::Dropped { pos, return_at } => FlagSnap { state: 2, carrier: 255, pos: pos.to_array(), left_ticks: left(return_at) },
            };
            return ObjSnap { kind: 1, flags: [flag(&ctf.flags[0]), flag(&ctf.flags[1])], events, ..Default::default() };
        }
        if let Some(snd) = &arena.snd {
            let (phase, left_ticks, winner) = match snd.phase {
                RoundPhase::Freeze { until } => (0, left(until), 0),
                RoundPhase::Live { until } => (1, left(until), 0),
                RoundPhase::Over { until, winner } => (2, left(until), winner),
            };
            let (bomb, carrier, pos, site, fuse_ticks) = match snd.bomb {
                BombState::Carried { slot, pos } => (0, slot, pos, 0, 0),
                BombState::Dropped(pos) => (1, 255, pos, 0, 0),
                BombState::Planted { pos, site, explode_at } => (2, 255, pos, site, left(explode_at)),
                BombState::Defused(pos) => (3, 255, pos, 0, 0),
                BombState::Exploded(pos) => (4, 255, pos, 0, 0),
            };
            let s = SndSnap {
                phase,
                round: snd.round,
                attackers: snd.attackers,
                left_ticks,
                bomb,
                carrier,
                pos: pos.to_array(),
                site,
                fuse_ticks,
                progress: snd.progress(now),
                winner,
            };
            return ObjSnap { kind: 2, snd: s, events, ..Default::default() };
        }
        ObjSnap::default()
    }
}
