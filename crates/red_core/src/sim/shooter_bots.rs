//! The loadout shooter's part of a bot's mind ([`super::game::BotMind`]): which weapon to hold, when to aim down the sights, when to throw a
//! grenade, when to reload, which weapon on the floor is worth walking to (never the one a teammate is already heading for), and where to be
//! for capture the flag and search and destroy. A bot's brain asks it where the shooter's rules came in (after aiming and firing; in its
//! movement), with the brain's own random numbers, so bots play exactly as when this was the brain's own code.
//!
//! Used by Killchain alone and offered by nothing: it moves to `games/killchain` (ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or,
//! system audit, docs/analysis/2026-10-14-system-audit.md).

use super::ai::secs;
use super::ai::skill::WeaponProfile;
use super::game::{BotCtx, BotGoal, BotMind};
use super::match_sim::MatchSim;
use super::objective::{BombState, FlagState, DEFUSE_RADIUS};
use super::player::PlayerInput;
use super::shooter::ShooterSim;
use crate::weapons::Weapon;
use glam::Vec3;

/// What a loadout bot remembers between ticks: when it may choose a weapon again, when it means to throw, where it is walking to pick
/// something up and until when that choice holds.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShooterBot {
    select_until: u64,
    nade_until: u64,
    pickup_goal: Option<Vec3>,
    pickup_until: u64,
}

impl BotMind for ShooterBot {
    /// Chooses the weapon to hold, aims down the sights at range, throws grenades at people it can see, and never shoots blind.
    fn weapons(&mut self, sim: &MatchSim, bot: &mut BotCtx, input: &mut PlayerInput) {
        let (slot, me, now) = (bot.slot, bot.me, bot.now);
        let Some(kit) = sim.kit(slot) else { return };
        if me.combat.flash_until > now {
            input.attack = false; // blinded: no shooting at nothing
            input.aim = false;
            return;
        }
        let target = bot.target.and_then(|t| sim.player(t));
        let dist = target.map(|p| (p.state.pos - me.state.pos).length());
        let current = kit.current();
        // Aim down the sights at range, except with weapons that do not gain from it.
        if let Some(d) = dist {
            input.aim = current.is_gun() && d > 13.0 && !matches!(current.class(), crate::arsenal::Class::Shotgun | crate::arsenal::Class::Launcher);
        }
        // Grenades: a lob at an enemy that is in sight and not too close.
        if kit.grenade_count() > 0 {
            if now >= self.nade_until && target.is_some() && dist.is_some_and(|d| (8.0..28.0).contains(&d)) && bot.rng.chance(0.0012) {
                self.nade_until = now + secs(0.9);
            }
            if now < self.nade_until {
                if current.is_grenade() {
                    if let Some(d) = dist {
                        input.pitch = (input.pitch + 0.16 + d * 0.0065).clamp(-1.4, 1.4);
                    }
                    input.attack = kit.busy == 0 && kit.throwing.is_none();
                    input.aim = false;
                    return;
                }
                if now >= self.select_until {
                    input.select = 4;
                    self.select_until = now + 12;
                }
                return;
            }
        }
        if now < self.select_until {
            return;
        }
        // Which gun to hold: one with ammunition; the one whose range suits the fight; the melee weapon when both are dry.
        let usable = |i: usize| kit.guns[i].filter(|g| g.loaded > 0 || g.reserve > 0 || kit.sel == crate::sim::kit::Slot::Gun(i as u8));
        let score = |i: usize| -> Option<f32> {
            let g = usable(i)?;
            let p = WeaponProfile::in_scene(g.weapon, sim);
            let have_rounds = if g.loaded > 0 { 0.0 } else { 40.0 };
            let fit = match dist {
                Some(d) => (p.ideal - d).abs().min(60.0),
                None => -(g.weapon.kit().damage as f32) * 0.1,
            };
            Some(fit + have_rounds)
        };
        let best = (0..2).filter_map(|i| score(i).map(|s| (i, s))).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i);
        let want = match best {
            Some(i) => crate::sim::kit::Slot::Gun(i as u8),
            None => crate::sim::kit::Slot::Melee,
        };
        // A reload in progress is not thrown away for a marginal gain.
        let reloading_useful = kit.reloading() && dist.is_none();
        if want != kit.sel && !reloading_useful && !(current.is_gun() && kit.gun().is_some_and(|g| g.loaded > 0) && dist.is_some_and(|d| d < 35.0)) {
            input.select = match want {
                crate::sim::kit::Slot::Gun(i) => 1 + i,
                _ => 3,
            };
            self.select_until = now + 30;
        }
        // Top up between fights.
        if dist.is_none() && current.is_gun() && kit.gun().is_some_and(|g| g.loaded * 2 < g.weapon.kit().mag && g.reserve > 0) {
            input.reload = bot.seq.is_multiple_of(2);
        }
    }

    /// The nearest weapon worth walking to: better than anything the bot holds, or fitting an empty slot.
    fn pickup(&mut self, sim: &MatchSim, bot: &mut BotCtx) -> Option<Vec3> {
        let (slot, me, now) = (bot.slot, bot.me, bot.now);
        let kit = sim.kit(slot)?;
        let arena = sim.arena()?;
        if now < self.pickup_until && self.pickup_goal.is_none() {
            return None;
        }
        if now < self.pickup_until
            && self.pickup_goal.is_some_and(|goal| {
                arena.pickups.iter().any(|m| m.taken_until.is_none() && (m.spawn.at - goal).length() < 0.1)
                    || arena.dropped.iter().any(|d| (d.pos - goal).length() < 0.1)
            })
        {
            return self.pickup_goal;
        }
        self.pickup_until = now + secs(2.0);
        let held: f32 = kit.guns.iter().flatten().map(|g| weapon_value(g.weapon)).fold(0.0, f32::max);
        let free_gun = kit.guns.iter().any(Option::is_none);
        let free_nade = kit.grenade_count() < crate::sim::kit::MAX_GRENADES;
        let low_ammo = kit.guns.iter().flatten().any(|g| g.reserve < g.weapon.kit().mag);
        let here = Vec3::new(me.state.pos.x, me.state.foot_y, me.state.pos.y);
        let mut best: Option<(f32, Vec3)> = None;
        let mut consider = |weapon: Option<Weapon>, pos: Vec3| {
            let value = match weapon {
                None => {
                    if !low_ammo {
                        return;
                    }
                    5.5
                }
                Some(w) if w.is_melee() => return,
                Some(w) if w.is_grenade() => {
                    if !free_nade {
                        return;
                    }
                    4.0
                }
                Some(w) => {
                    let v = weapon_value(w);
                    if !(free_gun || v > held + 0.5) {
                        return;
                    }
                    v
                }
            };
            let d = (pos - here).length();
            // Prefer a different pickup when a teammate is already heading here or standing beside it.
            // Claims expire with the brain's decision; no reservation state enters the simulation or wire format.
            let congestion = sim
                .players()
                .filter(|(s, p)| *s != slot && p.team == me.team && !p.combat.is_dead())
                .filter(|(s, p)| {
                    (Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y) - pos).length() < 3.0
                        || sim
                            .brain(*s)
                            .and_then(|b| b.mind::<ShooterBot>())
                            .is_some_and(|b| now < b.pickup_until && b.pickup_goal.is_some_and(|g| (g - pos).length() < 2.0))
                })
                .count() as f32;
            let score = value * 10.0 - d * 0.35 - congestion * 35.0;
            if d < 70.0 && best.is_none_or(|b| score > b.0) {
                best = Some((score, pos));
            }
        };
        for m in arena.pickups.iter().filter(|m| m.taken_until.is_none()) {
            consider(m.spawn.weapon, m.spawn.at);
        }
        for d in &arena.dropped {
            consider(Some(d.weapon), d.pos);
        }
        self.pickup_goal = best.map(|b| b.1);
        self.pickup_goal
    }

    /// What the bot in `slot` should do about the objective right now, `None` in the kill modes or when it has nothing to do. It never
    /// overrides a fight: the bot brain asks only when no enemy is in view.
    fn objective(&self, sim: &MatchSim, slot: usize) -> Option<BotGoal> {
        let arena = sim.arena()?;
        let me = sim.player(slot)?;
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
            if let Some(nav) = sim.nav() {
                if let Some(i) = nav.nearest(Vec3::new(lane.x, 0.0, lane.z), 1.0) {
                    lane = nav.node(i).pos;
                }
            }
            if ctf.carried_by(slot as u8).is_some() {
                // Home by the same lane they came by, once past the middle.
                let at = if along > 0.6 { lane } else { mine.home };
                return Some(BotGoal { at, hold: false, urgent: true });
            }
            match mine.state {
                FlagState::Carried { pos, .. } | FlagState::Dropped { pos, .. } => return Some(BotGoal { at: pos, hold: false, urgent: true }),
                FlagState::Home => {}
            }
            if let FlagState::Carried { slot: c, .. } = theirs.state {
                if sim.team_of(c as usize) == team {
                    return Some(BotGoal { at: mine.home + spread(slot), hold: false, urgent: false });
                }
            }
            // One bot in three guards the home flag; the rest go for the other one.
            if slot.is_multiple_of(3) {
                return Some(BotGoal { at: mine.home + spread(slot), hold: false, urgent: false });
            }
            // Not yet in the middle: go by the chosen lane; past it: for the flag.
            return Some(BotGoal { at: if along < 0.4 { lane } else { theirs.pos() }, hold: false, urgent: false });
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
                    Some(BotGoal { at: if on_site { pos } else { site.at }, hold: on_site, urgent: true })
                }
                BombState::Carried { slot: c, .. } => sim.player(c as usize).map(|p| BotGoal {
                    at: Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y) + spread(slot),
                    hold: false,
                    urgent: false,
                }),
                BombState::Dropped(p) => Some(BotGoal { at: p, hold: false, urgent: true }),
                BombState::Planted { pos, .. } => Some(BotGoal { at: pos + spread(slot) * 2.0, hold: false, urgent: false }),
                _ => None,
            };
        }
        match snd.bomb {
            BombState::Planted { pos, .. } => {
                let d = pos - Vec3::new(me.state.pos.x, pos.y, me.state.pos.y);
                let close = d.x * d.x + d.z * d.z < (DEFUSE_RADIUS * 0.8).powi(2);
                Some(BotGoal { at: pos, hold: close, urgent: true })
            }
            BombState::Defused(_) | BombState::Exploded(_) => None,
            _ => {
                let site = site_for(slot);
                Some(BotGoal { at: site.at + spread(slot) * 1.5, hold: false, urgent: false })
            }
        }
    }

    fn reached_pickup(&mut self) {
        self.pickup_until = 0;
    }
}

/// How much a bot wants a gun (a rough tier list: rifles and marksman guns over SMGs over pistols).
fn weapon_value(w: Weapon) -> f32 {
    use crate::arsenal::Class;
    let k = w.kit();
    match k.class {
        Class::Rifle => 9.0 + k.damage as f32 * 0.02,
        Class::Marksman => 8.5,
        Class::Sniper => 8.0 + k.damage as f32 * 0.01,
        Class::Lmg => 7.5,
        Class::Smg => 7.0,
        Class::Shotgun => 6.0,
        Class::Launcher => 6.5,
        Class::Pistol => 3.0 + k.damage as f32 * 0.02,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::shooter::{ShooterGame, BLOCK, GAME};

    fn goal(sim: &MatchSim, slot: usize) -> Option<Vec3> {
        sim.brain(slot).and_then(|b| b.mind::<ShooterBot>()).and_then(|m| m.pickup_goal)
    }

    #[test]
    fn teammates_choose_different_pickups_and_abandon_taken_goals_immediately() {
        crate::schema::define_blocks(&[BLOCK]);
        crate::sim::game::define_games(&[GAME]);
        let scene = crate::schema::parse_scene(
            r#"{
            "camera":{"position":[0,1.7,0],"target":[0,1.7,-5]},
            "shooter":{"start":["pistol","knife"],"pickups":[
                {"weapon":"rifle","at":[-10,0.3,0]}, {"weapon":"rifle","at":[10,0.3,0]}]},
            "objects":[{"id":"floor","type":"plane","size":[100,100]}]
        }"#,
        )
        .unwrap();
        let spawn = crate::sim::spawns::Spawn { id: "s".into(), position: [0.0; 3], yaw_deg: 0.0, group: String::new() };
        let mut sim = MatchSim::new(&scene, vec![spawn]);
        let cfg = crate::sim::ai::BotsConfig::default();
        assert!(sim.add_bot_in_slot_team(0, &cfg.spec(0), 1));
        assert!(sim.add_bot_in_slot_team(1, &cfg.spec(1), 1));
        // With nobody to fight, each bot heads for a pickup in its first thought: the first one's claim sends its teammate to the other.
        sim.tick_once();
        let (first, other) = (goal(&sim, 0).unwrap(), goal(&sim, 1).unwrap());
        assert!((first - other).length() > 10.0, "same-value pickups should split the team");
        sim.game_mut::<ShooterGame>().unwrap().arena.pickups.iter_mut().find(|m| m.spawn.at == other).unwrap().taken_until = Some(500);
        sim.tick_once();
        let next = goal(&sim, 1).unwrap();
        assert_ne!(other, next, "a consumed pickup is abandoned before the two-second decision expires");
    }
}
