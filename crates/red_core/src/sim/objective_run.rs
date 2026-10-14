//! The objective modes inside a running match: feeds [`objective`](super::objective) its actors every tick and carries out what it answers.
//!
//! Split from `match_sim` so the world stays free of mode rules: a mode here only reads positions and deaths, and acts through three
//! doors (revive everyone for a new round, hurt players near the blast, hold players still during the freeze).
//!
//! Used by Killchain alone and offered by nothing: it moves to `games/killchain` (ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or,
//! system audit, docs/analysis/2026-10-14-system-audit.md).

use super::match_sim::MatchSim;
use super::objective::{Actor, Cmd, ObjEvent, BLAST_RADIUS, EVENT_LOG};
use super::ordnance::{FxEvent, FxKind};
use super::shooter::{ModeKind, ShooterGame};
use crate::weapons::Weapon;
use glam::Vec3;

impl ShooterGame {
    /// Everybody on a team, as the objective modes see them.
    fn objective_actors(&self, sim: &MatchSim) -> Vec<Actor> {
        sim.players()
            .filter(|(_, p)| (1..=2).contains(&p.team))
            .map(|(slot, p)| Actor {
                slot: slot as u8,
                team: p.team,
                pos: Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y),
                alive: !p.combat.is_dead(),
                interact: p.interact_held,
            })
            .collect()
    }

    /// One tick of capture the flag or search and destroy (nothing in the other modes).
    pub(super) fn objective_tick(&mut self, sim: &mut MatchSim) {
        let mode = self.arena.cfg.mode;
        if !matches!(mode, ModeKind::Ctf | ModeKind::Snd) {
            return;
        }
        let actors = self.objective_actors(sim);
        let tick = sim.tick();
        let mut events = Vec::new();
        let mut cmds = Vec::new();
        {
            let arena = &mut self.arena;
            let win_rounds = arena.cfg.objective.win_rounds;
            if let Some(ctf) = arena.ctf.as_mut() {
                ctf.step(tick, &actors, &mut arena.points, &mut events);
            }
            if let Some(snd) = arena.snd.as_mut() {
                let won = arena.points.iter().any(|p| *p >= win_rounds);
                cmds = snd.step(tick, &actors, &mut arena.points, won, &mut events);
            }
            for (kind, team, slot) in events {
                let id = arena.next_id();
                if arena.obj_events.len() >= EVENT_LOG {
                    arena.obj_events.remove(0);
                }
                arena.obj_events.push(ObjEvent { id, kind, team, slot });
            }
        }
        for cmd in cmds {
            match cmd {
                Cmd::NewRound => self.snd_new_round(sim),
                Cmd::Explode(at) => self.bomb_blast(sim, at),
            }
        }
    }

    /// Back to the spawns with a fresh kit for everyone, and the floor cleared (a new search and destroy round).
    fn snd_new_round(&mut self, sim: &mut MatchSim) {
        let slots: Vec<usize> = sim.players().map(|(s, _)| s).collect();
        for slot in slots {
            sim.respawn_in(slot, self.me());
        }
        {
            let arena = &mut self.arena;
            arena.dropped.clear();
            arena.projectiles.clear();
            arena.zones.clear();
            for p in &mut arena.pickups {
                p.taken_until = None;
            }
        }
    }

    /// The bomb goes off: a big blast, lethal near the site and fading to nothing at the edge.
    fn bomb_blast(&mut self, sim: &mut MatchSim, at: Vec3) {
        let victims: Vec<(usize, f32)> = sim
            .players()
            .filter(|(_, p)| !p.combat.is_dead())
            .map(|(slot, p)| (slot, (Vec3::new(p.state.pos.x, p.state.foot_y + 1.0, p.state.pos.y) - at).length()))
            .filter(|(_, d)| *d < BLAST_RADIUS)
            .collect();
        for (slot, d) in victims {
            let dmg = (220.0 * (1.0 - d / BLAST_RADIUS).powf(0.8)).round() as u32;
            // Credited to nobody: a bomb is not a kill for anyone's score.
            sim.damage_in(slot, dmg.max(1), slot, Weapon::Frag, false, self.me());
        }
        let now = sim.tick();
        {
            let arena = &mut self.arena;
            let id = arena.next_id();
            if arena.fx.len() >= 6 {
                arena.fx.remove(0);
            }
            arena.fx.push(FxEvent { id, kind: FxKind::Blast, pos: at, size: BLAST_RADIUS * 0.8, tick: now });
        }
    }
}
