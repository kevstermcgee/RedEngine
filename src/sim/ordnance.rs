//! Things that fly and go off: rockets, launched and thrown grenades, smoke clouds, fires, and the sounds-and-flashes they raise
//! (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! Everything here is deterministic simulation on [`MatchSim`]: a projectile moves in whole ticks under gravity, stops at the first thing its
//! path meets (a wall, a prop or a player), and either detonates there (rockets, launcher grenades, incendiaries) or bounces off it (thrown
//! grenades) until its fuse runs out. A blast damages everyone within its radius by distance, unless a wall stands between; a flash blinds
//! whoever could see it; smoke and fire become [`Zone`]s that last a while. Clients draw what the snapshot lists; they decide nothing.

use super::match_sim::MatchSim;
use crate::arsenal::Payload;
use crate::hit::{raycast_shapes, surface_normal};
use crate::sim::clock::{secs_to_ticks, TICK_DT};
use crate::weapons::Weapon;
use glam::Vec3;

/// Most projectiles alive at once.
pub const MAX_PROJECTILES: usize = 24;
/// Most smoke clouds and fires alive at once (the oldest goes first).
pub const MAX_ZONES: usize = 8;
/// Explosions and pops are kept this many ticks so a client that missed a snapshot still sees them.
pub const FX_KEEP_TICKS: u64 = 40;
/// Most effect events kept.
pub const MAX_FX: usize = 8;
const GRAVITY: f32 = 9.81;
/// A thrown grenade loses this share of its speed along the surface normal when it bounces, and this share along the surface.
const BOUNCE_RESTITUTION: f32 = 0.42;
const BOUNCE_FRICTION: f32 = 0.82;
/// Fire hurts every this many ticks.
const FIRE_TICKS: u64 = 10;

/// A rocket or grenade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projectile {
    /// Unique id (how clients tell projectiles apart).
    pub id: u16,
    /// What it is ([`Weapon::Lancer`], [`Weapon::Thumper`] or a grenade).
    pub weapon: Weapon,
    /// Who launched it (slot).
    pub owner: u8,
    /// Where it is.
    pub pos: Vec3,
    /// How fast it moves, m/s.
    pub vel: Vec3,
    /// Ticks since launch.
    pub age: u16,
    /// Ticks until a grenade goes off (`0` = on contact).
    pub fuse: u16,
    /// Lying still on the ground.
    pub resting: bool,
}

/// What a zone is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneKind {
    /// A cloud nobody can see through.
    Smoke,
    /// A fire that burns.
    Fire,
}

/// A smoke cloud or a fire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zone {
    /// Unique id.
    pub id: u16,
    /// What it is.
    pub kind: ZoneKind,
    /// Centre on the ground.
    pub pos: Vec3,
    /// Radius, m.
    pub radius: f32,
    /// The tick it ends.
    pub until: u64,
    /// Who made it.
    pub owner: u8,
    /// Damage per second, fires only.
    pub dps: u16,
}

/// What kind of effect an [`FxEvent`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxKind {
    /// An explosion.
    Blast,
    /// A flashbang going off.
    FlashPop,
    /// A smoke grenade going off.
    SmokePop,
    /// An incendiary bursting.
    FirePop,
}

impl FxKind {
    /// The wire code.
    pub fn to_wire(self) -> u8 {
        match self {
            FxKind::Blast => 0,
            FxKind::FlashPop => 1,
            FxKind::SmokePop => 2,
            FxKind::FirePop => 3,
        }
    }

    /// The inverse of [`to_wire`](Self::to_wire).
    pub fn from_wire(v: u8) -> Option<FxKind> {
        Some(match v {
            0 => FxKind::Blast,
            1 => FxKind::FlashPop,
            2 => FxKind::SmokePop,
            3 => FxKind::FirePop,
            _ => return None,
        })
    }
}

/// Something a client should show and hear once: an explosion, a pop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FxEvent {
    /// Unique id (a client plays each id once).
    pub id: u16,
    /// What happened.
    pub kind: FxKind,
    /// Where.
    pub pos: Vec3,
    /// How big, m.
    pub size: f32,
    /// The tick it happened.
    pub tick: u64,
}

impl MatchSim {
    /// Launches a projectile of `weapon` from `origin` with velocity `vel`.
    pub(super) fn launch(&mut self, weapon: Weapon, owner: usize, origin: Vec3, vel: Vec3) {
        let spec = weapon.kit();
        let Some(arena) = self.arena.as_mut() else { return };
        if arena.projectiles.len() >= MAX_PROJECTILES {
            arena.projectiles.remove(0);
        }
        let id = arena.next_id();
        arena.projectiles.push(Projectile {
            id,
            weapon,
            owner: owner as u8,
            pos: origin,
            vel,
            age: 0,
            fuse: if spec.fuse > 0.0 { secs_to_ticks(spec.fuse).max(1) as u16 } else { 0 },
            resting: false,
        });
    }

    /// Moves every projectile one tick and detonates what is due.
    pub(super) fn step_projectiles(&mut self) {
        let Some(arena) = self.arena.as_ref() else { return };
        if arena.projectiles.is_empty() && arena.zones.is_empty() {
            return;
        }
        let mut goes_off: Vec<(Projectile, Vec3)> = Vec::new();
        let mut i = 0;
        while i < self.arena.as_ref().map_or(0, |a| a.projectiles.len()) {
            let mut pr = self.arena.as_ref().map(|a| a.projectiles[i]).unwrap_or_else(|| unreachable!());
            let spec = pr.weapon.kit();
            pr.age = pr.age.saturating_add(1);
            let mut detonate_at = None;
            if pr.fuse > 0 {
                pr.fuse -= 1;
                if pr.fuse == 0 {
                    detonate_at = Some(pr.pos);
                }
            }
            if detonate_at.is_none() && !pr.resting {
                pr.vel.y -= GRAVITY * spec.gravity * TICK_DT;
                let motion = pr.vel * TICK_DT;
                let dist = motion.length();
                let dir = if dist > 1e-6 { motion / dist } else { Vec3::NEG_Y };
                // A launcher's round clears its owner for the first few ticks; a grenade never collides with the thrower's own body on release.
                let ignore = if pr.age < 8 { pr.owner as usize } else { usize::MAX };
                let hit = self.probe(pr.pos, dir, dist + 0.05, ignore);
                match hit {
                    None => pr.pos += motion,
                    Some(h) => {
                        let at = pr.pos + dir * (h.distance - 0.03).max(0.0);
                        if spec.fuse == 0.0 {
                            // Contact rounds (rockets, launched grenades) go off where they land.
                            detonate_at = Some(at);
                            pr.pos = at;
                        } else {
                            let normal = match h.target {
                                super::interact::RayTarget::Player(_) => -dir,
                                _ => surface_normal(pr.pos, dir, h.distance, &self.hit_shapes),
                            };
                            pr.pos = at;
                            if spec.payload == Payload::Fire && normal.y > 0.3 {
                                detonate_at = Some(at);
                            } else {
                                let vn = normal * pr.vel.dot(normal);
                                let vt = pr.vel - vn;
                                pr.vel = vt * BOUNCE_FRICTION - vn * BOUNCE_RESTITUTION;
                                if normal.y > 0.6 && pr.vel.length() < 1.4 {
                                    pr.vel = Vec3::ZERO;
                                    pr.resting = true;
                                }
                            }
                        }
                    }
                }
            }
            if pr.pos.y < -60.0 {
                if let Some(a) = self.arena.as_mut() {
                    a.projectiles.remove(i);
                }
                continue;
            }
            if let Some(at) = detonate_at {
                goes_off.push((pr, at));
                if let Some(a) = self.arena.as_mut() {
                    a.projectiles.remove(i);
                }
                continue;
            }
            if let Some(a) = self.arena.as_mut() {
                a.projectiles[i] = pr;
            }
            i += 1;
        }
        for (pr, at) in goes_off {
            self.detonate(pr, at);
        }
        self.step_zones();
    }

    fn push_fx(&mut self, kind: FxKind, pos: Vec3, size: f32) {
        let tick = self.tick;
        let Some(a) = self.arena.as_mut() else { return };
        a.fx.retain(|e| tick.saturating_sub(e.tick) < FX_KEEP_TICKS);
        if a.fx.len() >= MAX_FX {
            a.fx.remove(0);
        }
        let id = a.next_id();
        a.fx.push(FxEvent { id, kind, pos, size, tick });
    }

    fn detonate(&mut self, pr: Projectile, at: Vec3) {
        let spec = pr.weapon.kit();
        match spec.payload {
            Payload::Explosive => {
                self.push_fx(FxKind::Blast, at, spec.radius);
                self.blast(at, pr.weapon, pr.owner as usize);
            }
            Payload::Flash => {
                self.push_fx(FxKind::FlashPop, at, spec.radius);
                self.flash(at, pr.weapon);
            }
            Payload::Smoke => {
                self.push_fx(FxKind::SmokePop, at, spec.radius);
                self.add_zone(ZoneKind::Smoke, at, spec.radius, spec.lasts, pr.owner, 0);
            }
            Payload::Fire => {
                self.push_fx(FxKind::FirePop, at, spec.radius);
                self.add_zone(ZoneKind::Fire, at, spec.radius, spec.lasts, pr.owner, spec.blast);
            }
            Payload::None => {}
        }
        self.rules.inject(self.tick, "explosion", Some(pr.owner as usize));
    }

    fn add_zone(&mut self, kind: ZoneKind, pos: Vec3, radius: f32, secs: f32, owner: u8, dps: u16) {
        let until = self.tick + secs_to_ticks(secs) as u64;
        let Some(a) = self.arena.as_mut() else { return };
        if a.zones.len() >= MAX_ZONES {
            a.zones.remove(0);
        }
        let id = a.next_id();
        a.zones.push(Zone { id, kind, pos, radius, until, owner, dps });
    }

    /// Damage of a blast at `at` to every living player by distance (nothing through a wall), and a shove for loose props.
    fn blast(&mut self, at: Vec3, weapon: Weapon, owner: usize) {
        let spec = weapon.kit();
        let radius = spec.radius;
        let victims: Vec<(usize, f32)> = self
            .players()
            .filter(|(_, p)| !p.combat.is_dead())
            .filter_map(|(slot, p)| {
                let body = p.state.character.body();
                let height = if p.crouching { body.crouch_eye + 0.12 } else { body.body_height };
                let centre = Vec3::new(p.state.pos.x, p.state.foot_y + height * 0.5, p.state.pos.y);
                // The nearest point of the body's axis, so a blast at a player's feet still counts as close.
                let nearest = Vec3::new(p.state.pos.x, at.y.clamp(p.state.foot_y + 0.1, p.state.foot_y + height - 0.1), p.state.pos.y);
                let d = (nearest - at).length().min((centre - at).length());
                (d < radius).then(|| {
                    let to = centre - (at + Vec3::Y * 0.1);
                    let len = to.length();
                    let blocked = len > 0.3 && raycast_shapes(at + Vec3::Y * 0.1, to / len, len - 0.15, &self.hit_shapes).is_some();
                    (slot, if blocked { -1.0 } else { d })
                })
            })
            .collect();
        for (slot, d) in victims {
            if d < 0.0 {
                continue;
            }
            let falloff = (1.0 - d / radius).clamp(0.0, 1.0).powf(1.2);
            let dmg = (spec.blast as f32 * falloff).round() as u32;
            if dmg > 0 {
                self.damage_ex(slot, dmg, owner, weapon, false);
            }
        }
        for k in 0..self.props.props().len() {
            let pos = self.props.prop_pose(k).w_axis.truncate();
            let to = pos - at;
            let d = to.length();
            if d < radius && d > 1e-3 {
                let mass = self.props.mass(k).min(6.0);
                self.shove(k, to / d, at, (1.0 - d / radius) * 9.0 * mass);
            }
        }
    }

    /// Blinds everyone who could see a flashbang go off at `at`: longer when close and when looking at it.
    fn flash(&mut self, at: Vec3, weapon: Weapon) {
        let spec = weapon.kit();
        let now = self.tick;
        let mut blinded: Vec<(usize, f32)> = Vec::new();
        for (slot, p) in self.players().filter(|(_, p)| !p.combat.is_dead()) {
            let body = p.state.character.body();
            let eye = Vec3::new(p.state.pos.x, p.state.foot_y + if p.crouching { body.crouch_eye } else { body.stand_eye }, p.state.pos.y);
            let to = at - eye;
            let d = to.length();
            if d > spec.radius || d < 1e-3 {
                continue;
            }
            if raycast_shapes(eye, to / d, d - 0.1, &self.hit_shapes).is_some() {
                continue;
            }
            let (sy, cy) = libm::sincosf(p.state.yaw);
            let (sp, cp) = libm::sincosf(p.state.pitch);
            let look = Vec3::new(sy * cp, sp, -cy * cp);
            let facing = look.dot(to / d);
            let angle = if facing > 0.35 {
                1.0
            } else if facing > -0.35 {
                0.55
            } else {
                0.22
            };
            let secs = spec.lasts * (1.0 - d / spec.radius).clamp(0.0, 1.0).powf(0.6) * angle;
            if secs > 0.25 {
                blinded.push((slot, secs));
            }
        }
        for (slot, secs) in blinded {
            if let Some(p) = self.players[slot].as_mut() {
                let until = now + secs_to_ticks(secs) as u64;
                p.combat.flash_until = p.combat.flash_until.max(until);
                p.combat.flash_total = p.combat.flash_total.max(secs);
            }
        }
    }

    /// Expires zones and lets fires burn whoever stands in them.
    fn step_zones(&mut self) {
        let now = self.tick;
        let Some(a) = self.arena.as_mut() else { return };
        a.zones.retain(|z| z.until > now);
        a.fx.retain(|e| now.saturating_sub(e.tick) < FX_KEEP_TICKS);
        if !now.is_multiple_of(FIRE_TICKS) {
            return;
        }
        let fires: Vec<Zone> = a.zones.iter().copied().filter(|z| z.kind == ZoneKind::Fire).collect();
        for z in fires {
            let burned: Vec<usize> = self
                .players()
                .filter(|(_, p)| !p.combat.is_dead())
                .filter(|(_, p)| {
                    let d = (p.state.pos - glam::Vec2::new(z.pos.x, z.pos.z)).length();
                    d < z.radius && (p.state.foot_y - z.pos.y).abs() < 1.6
                })
                .map(|(slot, _)| slot)
                .collect();
            let per_tick = (z.dps as u64 * FIRE_TICKS / crate::sim::clock::TICK_RATE_HZ as u64).max(1) as u32;
            for slot in burned {
                self.damage_ex(slot, per_tick, z.owner as usize, Weapon::Incendiary, false);
            }
        }
    }

    /// Whether a point is inside a smoke cloud (bots cannot see through one).
    pub fn in_smoke(&self, p: Vec3) -> bool {
        self.arena.as_ref().is_some_and(|a| a.zones.iter().any(|z| z.kind == ZoneKind::Smoke && (z.pos + Vec3::Y * z.radius * 0.5 - p).length() < z.radius))
    }
}
