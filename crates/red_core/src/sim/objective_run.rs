//! The objective modes inside a running match: feeds [`objective`](super::objective) its actors every tick and carries out what it answers.
//!
//! Split from `match_sim` so the world stays free of mode rules: a mode here only reads positions and deaths, and acts through three
//! doors (revive everyone for a new round, hurt players near the blast, hold players still during the freeze).

use super::match_sim::MatchSim;
use super::objective::{Actor, BombState, Cmd, FlagState, ObjEvent, BLAST_RADIUS, DEFUSE_RADIUS, EVENT_LOG};
use super::ordnance::{FxEvent, FxKind};
use super::shooter::ModeKind;
use crate::weapons::Weapon;
use glam::Vec3;

impl MatchSim {
    /// Whether players are held in place (the freeze before a search and destroy round).
    pub fn input_locked(&self) -> bool {
        self.arena.as_ref().and_then(|a| a.snd.as_ref()).is_some_and(|s| s.locked())
    }

    /// Whether the dead stay dead for now (search and destroy: one life per round).
    pub(super) fn respawn_blocked(&self) -> bool {
        self.arena.as_ref().is_some_and(|a| a.cfg.mode == ModeKind::Snd)
    }

    /// Everybody on a team, as the objective modes see them.
    fn objective_actors(&self) -> Vec<Actor> {
        self.players()
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
    pub(super) fn objective_tick(&mut self) {
        let mode = self.arena.as_ref().map_or(ModeKind::Tdm, |a| a.cfg.mode);
        if !matches!(mode, ModeKind::Ctf | ModeKind::Snd) {
            return;
        }
        let actors = self.objective_actors();
        let tick = self.tick;
        let mut events = Vec::new();
        let mut cmds = Vec::new();
        {
            let Some(arena) = self.arena.as_mut() else { return };
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
                Cmd::NewRound => self.snd_new_round(),
                Cmd::Explode(at) => self.bomb_blast(at),
            }
        }
    }

    /// Back to the spawns with a fresh kit for everyone, and the floor cleared (a new search and destroy round).
    fn snd_new_round(&mut self) {
        let slots: Vec<usize> = self.players().map(|(s, _)| s).collect();
        for slot in slots {
            self.respawn(slot);
        }
        if let Some(arena) = self.arena.as_mut() {
            arena.dropped.clear();
            arena.projectiles.clear();
            arena.zones.clear();
            for p in &mut arena.pickups {
                p.taken_until = None;
            }
        }
    }

    /// The bomb goes off: a big blast, lethal near the site and fading to nothing at the edge.
    fn bomb_blast(&mut self, at: Vec3) {
        let victims: Vec<(usize, f32)> = self
            .players()
            .filter(|(_, p)| !p.combat.is_dead())
            .map(|(slot, p)| (slot, (Vec3::new(p.state.pos.x, p.state.foot_y + 1.0, p.state.pos.y) - at).length()))
            .filter(|(_, d)| *d < BLAST_RADIUS)
            .collect();
        for (slot, d) in victims {
            let dmg = (220.0 * (1.0 - d / BLAST_RADIUS).powf(0.8)).round() as u32;
            // Credited to nobody: a bomb is not a kill for anyone's score.
            self.damage_ex(slot, dmg.max(1), slot, Weapon::Frag, false);
        }
        let now = self.tick;
        if let Some(arena) = self.arena.as_mut() {
            let id = arena.next_id();
            if arena.fx.len() >= 6 {
                arena.fx.remove(0);
            }
            arena.fx.push(FxEvent { id, kind: FxKind::Blast, pos: at, size: BLAST_RADIUS * 0.8, tick: now });
        }
    }
}

/// Where an objective-minded bot wants to be, and whether it should be holding Interact there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjGoal {
    /// Where to go (the ground there).
    pub at: Vec3,
    /// Hold Interact now (planting or defusing).
    pub hold: bool,
    /// Carrying the flag or bomb, or racing to take back one's own: worth running past a fight.
    pub urgent: bool,
}

impl MatchSim {
    /// What the bot in `slot` should do about the objective right now, `None` in the kill modes or when it has nothing to do. It never
    /// overrides a fight: the bot brain asks only when no enemy is in view.
    pub fn bot_objective_goal(&self, slot: usize) -> Option<ObjGoal> {
        let arena = self.arena.as_ref()?;
        let me = self.player(slot)?;
        if me.combat.is_dead() || !(1..=2).contains(&me.team) {
            return None;
        }
        let team = me.team;
        let pos = Vec3::new(me.state.pos.x, me.state.foot_y, me.state.pos.y);
        // A little spread so teammates do not all stand on one spot.
        let spread = |i: usize| {
            let a = i as f32 * 2.399;
            Vec3::new(a.cos() * 3.0, 0.0, a.sin() * 3.0)
        };
        if let Some(ctf) = &arena.ctf {
            let (mine, theirs) = (&ctf.flags[(team - 1) as usize], &ctf.flags[(2 - team) as usize]);
            // Raiders split between lanes: a point beside the middle of the line between the two bases, on the side their slot picks. A raider
            // who has not yet passed the middle heads there first, so two teams' raiders do not all meet head-on in the shortest lane.
            let axis = theirs.home - mine.home;
            let len = axis.length().max(1.0);
            let (dir, side) = (axis / len, Vec3::new(-axis.z, 0.0, axis.x) / len);
            let along = (pos - mine.home).dot(dir) / len; // 0 at my base, 1 at theirs
            let mut lane = mine.home + axis * 0.5 + side * if slot.is_multiple_of(2) { 26.0 } else { -26.0 };
            // A waypoint must be somewhere a bot can walk to: snap it to the nearest node of the map's walkable graph.
            if let Some(nav) = self.nav() {
                if let Some(i) = nav.nearest(Vec3::new(lane.x, 0.0, lane.z), 1.0) {
                    lane = nav.node(i).pos;
                }
            }
            if ctf.carried_by(slot as u8).is_some() {
                // Home by the same lane they came by, once past the middle.
                let at = if along > 0.6 { lane } else { mine.home };
                return Some(ObjGoal { at, hold: false, urgent: true });
            }
            match mine.state {
                FlagState::Carried { pos, .. } | FlagState::Dropped { pos, .. } => return Some(ObjGoal { at: pos, hold: false, urgent: true }),
                FlagState::Home => {}
            }
            if let FlagState::Carried { slot: c, .. } = theirs.state {
                if self.team_of(c as usize) == team {
                    return Some(ObjGoal { at: mine.home + spread(slot), hold: false, urgent: false });
                }
            }
            // One bot in three guards the home flag; the rest go for the other one.
            if slot.is_multiple_of(3) {
                return Some(ObjGoal { at: mine.home + spread(slot), hold: false, urgent: false });
            }
            // Not yet in the middle: go by the chosen lane; past it: for the flag.
            return Some(ObjGoal { at: if along < 0.4 { lane } else { theirs.pos() }, hold: false, urgent: false });
        }
        let snd = arena.snd.as_ref()?;
        if snd.locked() {
            return None;
        }
        let site_for = |i: usize| snd.sites()[i % snd.sites().len()];
        if team == snd.attackers {
            return match snd.bomb {
                BombState::Carried { slot: c, .. } if c as usize == slot => {
                    let site = *snd.sites().iter().min_by(|a, b| (a.at - pos).length().total_cmp(&(b.at - pos).length()))?;
                    let d = site.at - pos;
                    let on_site = d.x * d.x + d.z * d.z < (site.radius * 0.7).powi(2);
                    Some(ObjGoal { at: if on_site { pos } else { site.at }, hold: on_site, urgent: true })
                }
                BombState::Carried { slot: c, .. } => self.player(c as usize).map(|p| ObjGoal {
                    at: Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y) + spread(slot),
                    hold: false,
                    urgent: false,
                }),
                BombState::Dropped(p) => Some(ObjGoal { at: p, hold: false, urgent: true }),
                BombState::Planted { pos, .. } => Some(ObjGoal { at: pos + spread(slot) * 2.0, hold: false, urgent: false }),
                _ => None,
            };
        }
        match snd.bomb {
            BombState::Planted { pos, .. } => {
                let d = pos - Vec3::new(me.state.pos.x, pos.y, me.state.pos.y);
                let close = d.x * d.x + d.z * d.z < (DEFUSE_RADIUS * 0.8).powi(2);
                Some(ObjGoal { at: pos, hold: close, urgent: true })
            }
            BombState::Defused(_) | BombState::Exploded(_) => None,
            _ => {
                let site = site_for(slot);
                Some(ObjGoal { at: site.at + spread(slot) * 1.5, hold: false, urgent: false })
            }
        }
    }
}
