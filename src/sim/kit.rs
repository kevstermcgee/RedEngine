//! What a player carries and does in a loadout match: two guns with a magazine and reserve each, a melee weapon, up to two grenades; choosing,
//! firing, reloading, throwing, picking up and dropping (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! The authoritative rules on [`MatchSim`]; a client only sends buttons (`PlayerInput::{attack, aim, reload, interact, drop, select}`). Every
//! number comes from the weapon's row in [`crate::arsenal`]: cadence, magazine, reload time, spread (wider when moving, jumping or spraying,
//! tight when aiming or crouched), damage falloff with range, the headshot multiplier, a knife's backstab. Randomness is a hash of tick, slot and
//! pellet, so a replay fires the same shots.

use super::interact::RayTarget;
use super::match_sim::MatchSim;
use super::player::PlayerInput;
use super::shooter::{Dropped, DROP_LIFETIME_SECS, MAX_DROPPED};
use crate::arsenal::{Class, KitSpec};
use crate::sim::clock::{secs_to_ticks, TICK_DT};
use crate::weapons::{Ammo, Weapon};
use glam::Vec3;

/// Most grenades carried.
pub const MAX_GRENADES: usize = 2;
/// Ticks to bring a weapon up after choosing it.
pub const DRAW_TICKS: u16 = secs_to_ticks(0.36) as u16;
/// Ticks from starting a throw to the grenade leaving the hand.
pub const THROW_TICKS: u16 = secs_to_ticks(0.22) as u16;
/// Ticks a player cannot use a weapon after throwing.
pub const AFTER_THROW_TICKS: u16 = secs_to_ticks(0.7) as u16;
/// Ticks of a dry click.
pub const DRY_TICKS: u16 = secs_to_ticks(0.25) as u16;
/// Seconds a knife's slash takes to land.
const MELEE_WINDUP_SECS: f32 = 0.11;
/// Horizontal distance within which walking over a weapon picks it up, m.
pub const PICKUP_RADIUS: f32 = 1.15;
/// Horizontal distance within which pressing interact picks a weapon up, m.
pub const PICKUP_REACH: f32 = 2.4;
/// Ticks a dropper cannot pick their own drop back up.
const DROP_IGNORE_TICKS: u64 = secs_to_ticks(1.2) as u64;
/// Standing head height above the feet where a head hit begins is `body height - HEAD_DEPTH`, m.
pub const HEAD_DEPTH: f32 = 0.24;

/// A gun and its own ammunition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gun {
    /// Which gun.
    pub weapon: Weapon,
    /// Rounds in the magazine.
    pub loaded: u16,
    /// Rounds in reserve for this gun alone.
    pub reserve: u16,
}

impl Gun {
    /// A new gun with a full magazine and its usual reserve.
    pub fn fresh(weapon: Weapon) -> Gun {
        let k = weapon.kit();
        Gun { weapon, loaded: k.mag, reserve: k.reserve }
    }

    /// The most reserve rounds a player may carry for this gun.
    pub fn reserve_cap(weapon: Weapon) -> u16 {
        let k = weapon.kit();
        k.reserve + k.mag
    }
}

/// Where in the kit the weapon in hand is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// One of the two gun slots.
    Gun(u8),
    /// The melee weapon.
    Melee,
    /// One of the two grenade slots.
    Grenade(u8),
}

impl Slot {
    /// A slot as a byte: `0`, `1` the guns, `2` melee, `3`, `4` the grenades.
    pub fn to_wire(self) -> u8 {
        match self {
            Slot::Gun(i) => i.min(1),
            Slot::Melee => 2,
            Slot::Grenade(i) => 3 + i.min(1),
        }
    }

    /// The inverse of [`to_wire`](Self::to_wire).
    pub fn from_wire(v: u8) -> Slot {
        match v {
            0 | 1 => Slot::Gun(v),
            2 => Slot::Melee,
            3 | 4 => Slot::Grenade(v - 3),
            _ => Slot::Melee,
        }
    }

    const CYCLE: [Slot; 5] = [Slot::Gun(0), Slot::Gun(1), Slot::Melee, Slot::Grenade(0), Slot::Grenade(1)];
}

/// One player's weapons and what they are doing with them.
#[derive(Debug, Clone, PartialEq)]
pub struct Kit {
    /// The two gun slots.
    pub guns: [Option<Gun>; 2],
    /// The melee weapon.
    pub melee: Weapon,
    /// Up to two grenades.
    pub grenades: [Option<Weapon>; MAX_GRENADES],
    /// What is in hand.
    pub sel: Slot,
    /// What was in hand before (quick switch).
    pub last: Slot,
    /// Ticks until the reload finishes (`0` = not reloading).
    pub reload_left: u16,
    /// Ticks until the weapon can be used again (drawing, cycling a bolt, cooling down).
    pub busy: u16,
    /// Ticks until a melee strike lands (`0` = none under way).
    pub strike_in: u16,
    /// Ticks left of the melee swing being animated (`0` = not swinging).
    pub swing: u16,
    /// A grenade being thrown: `(which, underhand, ticks until release)`.
    pub throwing: Option<(Weapon, bool, u16)>,
    /// How much the last shots have spoiled the aim, 0..1.
    pub heat: f32,
    /// Aiming down the sights this tick (from the input).
    pub aiming: bool,
    prev: Prev,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Prev {
    attack: bool,
    reload: bool,
    interact: bool,
    drop: bool,
    select: u8,
}

impl Kit {
    /// A kit holding `weapons` (at most two guns, one melee weapon, two grenades; a missing melee weapon is the combat knife). The first gun is in
    /// hand, else the melee weapon.
    pub fn from_weapons(weapons: &[Weapon]) -> Kit {
        let mut kit = Kit {
            guns: [None; 2],
            melee: Weapon::Knife,
            grenades: [None; MAX_GRENADES],
            sel: Slot::Melee,
            last: Slot::Melee,
            reload_left: 0,
            busy: DRAW_TICKS,
            strike_in: 0,
            swing: 0,
            throwing: None,
            heat: 0.0,
            aiming: false,
            prev: Prev::default(),
        };
        for &w in weapons {
            match w.class() {
                Class::Melee => kit.melee = w,
                Class::Grenade => {
                    if let Some(s) = kit.grenades.iter_mut().find(|s| s.is_none()) {
                        *s = Some(w);
                    }
                }
                _ => {
                    if let Some(s) = kit.guns.iter_mut().find(|s| s.is_none()) {
                        *s = Some(Gun::fresh(w));
                    }
                }
            }
        }
        kit.sel = if kit.guns[0].is_some() { Slot::Gun(0) } else { Slot::Melee };
        kit.last = Slot::Melee;
        kit
    }

    /// The weapon in hand.
    pub fn current(&self) -> Weapon {
        self.weapon_at(self.sel).unwrap_or(self.melee)
    }

    /// The weapon in a slot, if the slot is filled.
    pub fn weapon_at(&self, slot: Slot) -> Option<Weapon> {
        match slot {
            Slot::Gun(i) => self.guns[i as usize & 1].map(|g| g.weapon),
            Slot::Melee => Some(self.melee),
            Slot::Grenade(i) => self.grenades[i as usize & 1],
        }
    }

    /// The gun in hand, if a gun is in hand.
    pub fn gun(&self) -> Option<&Gun> {
        match self.sel {
            Slot::Gun(i) => self.guns[i as usize & 1].as_ref(),
            _ => None,
        }
    }

    fn gun_mut(&mut self) -> Option<&mut Gun> {
        match self.sel {
            Slot::Gun(i) => self.guns[i as usize & 1].as_mut(),
            _ => None,
        }
    }

    /// How many grenades are carried.
    pub fn grenade_count(&self) -> usize {
        self.grenades.iter().flatten().count()
    }

    /// Whether the weapon in hand is reloading.
    pub fn reloading(&self) -> bool {
        self.reload_left > 0
    }

    /// Whether a melee swing is under way (for the animation).
    pub fn swinging(&self) -> bool {
        self.swing > 0
    }

    fn select(&mut self, slot: Slot) -> bool {
        if slot == self.sel || self.weapon_at(slot).is_none() {
            return false;
        }
        self.last = self.sel;
        self.sel = slot;
        self.reload_left = 0;
        self.strike_in = 0;
        self.swing = 0;
        self.throwing = None;
        self.busy = DRAW_TICKS;
        self.heat = 0.0;
        true
    }

    /// The next filled slot in the cycle, `dir` steps (+1 / -1) from the one in hand.
    fn cycled(&self, dir: i32) -> Option<Slot> {
        let start = Slot::CYCLE.iter().position(|s| *s == self.sel).unwrap_or(0) as i32;
        (1..=5).map(|k| Slot::CYCLE[(start + dir * k).rem_euclid(5) as usize]).find(|s| self.weapon_at(*s).is_some() && *s != self.sel)
    }

    /// Takes away the thing in `slot` and returns it as a drop (`None` for an empty slot or the combat knife).
    fn take(&mut self, slot: Slot) -> Option<(Weapon, u16, u16)> {
        match slot {
            Slot::Gun(i) => self.guns[i as usize & 1].take().map(|g| (g.weapon, g.loaded, g.reserve)),
            Slot::Melee if self.melee != Weapon::Knife => {
                let w = std::mem::replace(&mut self.melee, Weapon::Knife);
                Some((w, 0, 0))
            }
            Slot::Melee => None,
            Slot::Grenade(i) => self.grenades[i as usize & 1].take().map(|w| (w, 1, 0)),
        }
    }

    /// Repairs the selection after something left a slot: fall back to the other gun, else melee.
    fn reselect(&mut self) {
        if self.weapon_at(self.sel).is_some() {
            return;
        }
        let next = [Slot::Gun(0), Slot::Gun(1), Slot::Melee].into_iter().find(|s| self.weapon_at(*s).is_some()).unwrap_or(Slot::Melee);
        self.sel = next;
        self.reload_left = 0;
        self.busy = DRAW_TICKS;
    }

    /// A fold of the kit for the match checksum.
    pub fn state_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for g in &self.guns {
            mix(g.map_or(0, |g| g.weapon.wire() as u64 + 1 + ((g.loaded as u64) << 8) + ((g.reserve as u64) << 24)));
        }
        mix(self.melee.wire() as u64);
        for g in &self.grenades {
            mix(g.map_or(0, |w| w.wire() as u64 + 1));
        }
        mix(self.sel.to_wire() as u64 | (self.reload_left as u64) << 8 | (self.busy as u64) << 24 | (self.strike_in as u64) << 40);
        h
    }
}

/// splitmix64.
fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Two numbers in `[0, 1)` from a seed.
fn unit_pair(seed: u64) -> (f32, f32) {
    let a = mix64(seed);
    let b = mix64(a);
    (((a >> 40) as f32) / (1u64 << 24) as f32, ((b >> 40) as f32) / (1u64 << 24) as f32)
}

/// `dir` turned by an angle of up to `cone_deg` in a direction chosen by `u`, `v` (uniform over the disc).
pub fn scatter(dir: Vec3, cone_deg: f32, u: f32, v: f32) -> Vec3 {
    if cone_deg <= 0.0 {
        return dir;
    }
    let right = dir.cross(Vec3::Y).normalize_or_zero();
    let right = if right == Vec3::ZERO { Vec3::X } else { right };
    let up = right.cross(dir).normalize_or_zero();
    let r = cone_deg.to_radians().tan() * v.sqrt();
    let a = u * std::f32::consts::TAU;
    (dir + (right * a.cos() + up * a.sin()) * r).normalize()
}

/// The share of a gun's damage left at `dist` metres.
pub fn falloff(spec: &KitSpec, dist: f32) -> f32 {
    if dist <= spec.effective || spec.range <= spec.effective {
        return 1.0;
    }
    let t = ((dist - spec.effective) / (spec.range - spec.effective)).clamp(0.0, 1.0);
    1.0 + (spec.far_mult - 1.0) * t
}

impl MatchSim {
    /// Whether this match is a loadout match.
    pub fn is_loadout(&self) -> bool {
        self.arena.is_some()
    }

    /// Whether players may be assigned to team 1 or 2: always true in a loadout match, or when the scene's own
    /// `"teams": true` asked for team assignment without a `shooter` block (a hide-and-seek, capture-the-flag or
    /// other asymmetric-role game that still wants `who: team1`/`who: team2` in its rules).
    pub fn teams_enabled(&self) -> bool {
        self.is_loadout() || self.teams_requested
    }

    /// Keeps the legacy view of a kit (the weapon in hand, the ammunition the HUD reads) in step with the kit.
    pub(super) fn sync_kit(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_mut() else { return };
        let Some(kit) = p.combat.kit.as_ref() else { return };
        p.combat.weapon = kit.current();
        p.combat.ammo = match kit.gun() {
            Some(g) => Ammo::Limited { loaded: g.loaded as u32, capacity: g.weapon.kit().mag as u32, reserve: g.reserve as u32 },
            None => Ammo::Infinite,
        };
    }

    /// Reads player `slot`'s buttons for a loadout match.
    pub(super) fn kit_actions(&mut self, slot: usize, input: &PlayerInput) {
        let Some(p) = self.players[slot].as_mut() else { return };
        if p.combat.is_dead() {
            return;
        }
        let Some(kit) = p.combat.kit.as_mut() else { return };
        let prev = kit.prev;
        let edge = |now: bool, before: bool| now && !before;
        kit.prev = Prev { attack: input.attack, reload: input.reload, interact: input.interact, drop: input.drop, select: input.select };
        // Choosing a weapon.
        if input.select != 0 && input.select != prev.select {
            let target = match input.select {
                1 => Some(Slot::Gun(0)),
                2 => Some(Slot::Gun(1)),
                3 => Some(Slot::Melee),
                4 => match kit.sel {
                    Slot::Grenade(0) if kit.grenades[1].is_some() => Some(Slot::Grenade(1)),
                    Slot::Grenade(_) => kit.grenades[0].map(|_| Slot::Grenade(0)),
                    _ => kit.grenades[0].map(|_| Slot::Grenade(0)).or(kit.grenades[1].map(|_| Slot::Grenade(1))),
                },
                5 => Some(kit.last),
                6 => kit.cycled(1),
                7 => kit.cycled(-1),
                _ => None,
            };
            if let Some(t) = target {
                if kit.select(t) {
                    p.combat.protected_until = 0;
                }
            }
        }
        kit.aiming = input.aim && kit.weapon_at(kit.sel).is_some_and(|w| w.is_gun()) && kit.reload_left == 0 && kit.busy == 0;
        let (want_drop, want_interact, want_reload) = (edge(input.drop, prev.drop), edge(input.interact, prev.interact), edge(input.reload, prev.reload));
        let attack_edge = edge(input.attack, prev.attack);
        let (pressed, held) = (attack_edge, input.attack);
        let underhand = input.aim;
        self.sync_kit(slot);
        if want_drop {
            self.kit_drop(slot);
        }
        if want_interact {
            self.kit_interact(slot);
        }
        if want_reload {
            self.kit_start_reload(slot);
        }
        if pressed || held {
            self.kit_attack(slot, pressed, underhand);
        }
        self.sync_kit(slot);
    }

    fn kit_start_reload(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_mut() else { return };
        let Some(kit) = p.combat.kit.as_mut() else { return };
        if kit.reload_left > 0 || kit.throwing.is_some() {
            return;
        }
        let Some(g) = kit.gun().copied() else { return };
        let spec = g.weapon.kit();
        if g.loaded >= spec.mag || g.reserve == 0 {
            return;
        }
        kit.reload_left = spec.reload_ticks() as u16;
        kit.aiming = false;
    }

    fn kit_attack(&mut self, slot: usize, pressed: bool, underhand: bool) {
        let Some(p) = self.players[slot].as_mut() else { return };
        let Some(kit) = p.combat.kit.as_mut() else { return };
        if kit.busy > 0 || kit.throwing.is_some() {
            return;
        }
        let weapon = kit.current();
        let spec = weapon.kit();
        p.combat.protected_until = 0; // raising a weapon ends spawn protection
        match spec.class {
            Class::Melee => {
                if !pressed && !spec.auto {
                    return;
                }
                kit.busy = spec.cooldown_ticks() as u16;
                kit.swing = spec.cooldown_ticks().min(24) as u16;
                kit.strike_in = secs_to_ticks(MELEE_WINDUP_SECS).max(2) as u16;
                self.rules.inject(self.tick, "swing", Some(slot));
            }
            Class::Grenade => {
                if !pressed {
                    return;
                }
                kit.throwing = Some((weapon, underhand, THROW_TICKS));
                kit.reload_left = 0;
            }
            _ => {
                if !pressed && !spec.auto {
                    return;
                }
                if kit.reload_left > 0 {
                    // A shotgun loaded a shell at a time can be fired mid-reload; everything else waits for the magazine.
                    if spec.per_shell && kit.gun().is_some_and(|g| g.loaded > 0) {
                        kit.reload_left = 0;
                    } else {
                        return;
                    }
                }
                let Some(g) = kit.gun_mut() else { return };
                if g.loaded == 0 {
                    if g.reserve > 0 {
                        kit.reload_left = spec.reload_ticks() as u16;
                    } else {
                        kit.busy = DRY_TICKS;
                    }
                    return;
                }
                g.loaded -= 1;
                kit.busy = spec.cooldown_ticks() as u16;
                kit.heat = (kit.heat + if spec.auto { 0.16 } else { 0.35 }).min(1.0);
                self.kit_shoot(slot, weapon);
            }
        }
    }

    /// Fires one trigger pull of the gun in hand (the round is already spent).
    fn kit_shoot(&mut self, slot: usize, weapon: Weapon) {
        let spec = weapon.kit();
        let (now, friendly_fire) = (self.tick, self.arena.as_ref().is_some_and(|a| a.cfg.friendly_fire));
        let Some(p) = self.players[slot].as_mut() else { return };
        p.combat.shots = p.combat.shots.wrapping_add(1);
        let shots = p.combat.shots;
        let (eye, look) = super::interact::eye_and_look(&p.state, p.crouching);
        let (aiming, heat) = (p.combat.kit.as_ref().is_some_and(|k| k.aiming), p.combat.kit.as_ref().map_or(0.0, |k| k.heat));
        let body = p.state.character.body();
        let move_frac = (p.speed / body.sprint_speed.max(0.1)).clamp(0.0, 1.0);
        let airborne = p.state.vy.abs() > 0.05;
        let (crouching, lag, velocity) = (p.crouching, p.view_lag as usize, p.state.velocity);
        self.rules.inject(now, "shot", Some(slot));
        let mut cone = if aiming { spec.spread_ads } else { spec.spread_hip };
        cone += spec.spread_move * move_frac * if aiming { 0.6 } else { 1.0 };
        if crouching {
            cone *= 0.65;
        }
        if airborne {
            cone += 2.0 + spec.spread_move;
        }
        cone += heat * (spec.spread_hip.max(0.8)) * 0.9 * if spec.auto { 1.0 } else { 0.5 };
        let _ = friendly_fire;
        if spec.class == Class::Launcher {
            let (u, v) = unit_pair(now << 20 ^ (slot as u64) << 8 ^ shots as u64);
            let dir = scatter(look, cone, u, v);
            let origin = eye + look * 0.9 - Vec3::Y * 0.12;
            self.launch(weapon, slot, origin, dir * spec.speed + Vec3::new(velocity.x, 0.0, velocity.y) * 0.3);
            return;
        }
        let mut landed: Vec<(usize, f32, bool)> = Vec::new();
        let mut struck_prop = false;
        for pellet in 0..spec.pellets.max(1) {
            let seed = now << 24 ^ (slot as u64) << 16 ^ (shots as u64) << 4 ^ pellet as u64;
            let (u, v) = unit_pair(seed);
            let dir = scatter(look, cone, u, v);
            let Some(hit) = self.probe_lagged(eye, dir, spec.range, slot, lag) else { continue };
            match hit.target {
                RayTarget::Prop(prop) => {
                    self.shove(prop, dir, eye + dir * hit.distance, spec.damage as f32 * 0.4 / spec.pellets as f32);
                    struck_prop = true;
                }
                RayTarget::Player(target) => {
                    let y = eye.y + dir.y * hit.distance;
                    let Some(t) = self.players[target].as_ref() else { continue };
                    let tbody = t.state.character.body();
                    let height = if t.crouching { tbody.crouch_eye + 0.12 } else { tbody.body_height };
                    let head = y - t.state.foot_y >= height - HEAD_DEPTH;
                    let per =
                        spec.damage as f32 / spec.pellets.max(1) as f32 * falloff(&spec, hit.distance) * if head { spec.head_x10 as f32 / 10.0 } else { 1.0 };
                    match landed.iter_mut().find(|(s, _, _)| *s == target) {
                        Some(entry) => {
                            entry.1 += per;
                            entry.2 |= head;
                        }
                        None => landed.push((target, per, head)),
                    }
                }
                RayTarget::Static => {}
            }
        }
        if struck_prop {
            self.rules.inject(now, "prop_hit", Some(slot));
        }
        let mut any = false;
        for (target, amount, head) in landed {
            any |= self.damage_ex(target, amount.round().max(1.0) as u32, slot, weapon, head);
        }
        if any {
            if let Some(p) = self.players[slot].as_mut() {
                p.combat.hits = p.combat.hits.wrapping_add(1);
            }
        }
    }

    /// Advances player `slot`'s kit one tick: drawing, reloading, strikes, throws, aim recovery.
    pub(super) fn kit_tick(&mut self, slot: usize) {
        let now = self.tick;
        let Some(p) = self.players[slot].as_mut() else { return };
        if p.combat.is_dead() {
            return;
        }
        let Some(kit) = p.combat.kit.as_mut() else { return };
        kit.busy = kit.busy.saturating_sub(1);
        kit.swing = kit.swing.saturating_sub(1);
        kit.heat = (kit.heat - 1.6 * TICK_DT).max(0.0);
        if kit.reload_left > 0 {
            kit.reload_left -= 1;
            if kit.reload_left == 0 {
                if let Some(g) = kit.gun_mut() {
                    let spec = g.weapon.kit();
                    if spec.per_shell {
                        if g.reserve > 0 && g.loaded < spec.mag {
                            g.loaded += 1;
                            g.reserve -= 1;
                        }
                        if g.reserve > 0 && g.loaded < spec.mag {
                            kit.reload_left = spec.reload_ticks() as u16;
                        }
                    } else {
                        let n = (spec.mag - g.loaded).min(g.reserve);
                        g.loaded += n;
                        g.reserve -= n;
                    }
                }
            }
        }
        // An empty gun reloads by itself once it can.
        if kit.reload_left == 0 && kit.busy == 0 {
            if let Some(g) = kit.gun().copied() {
                if g.loaded == 0 && g.reserve > 0 {
                    kit.reload_left = g.weapon.kit().reload_ticks() as u16;
                }
            }
        }
        let mut strike = false;
        if kit.strike_in > 0 {
            kit.strike_in -= 1;
            strike = kit.strike_in == 0;
        }
        let mut release = None;
        if let Some((w, under, left)) = kit.throwing {
            if left <= 1 {
                kit.throwing = None;
                kit.busy = AFTER_THROW_TICKS;
                release = Some((w, under));
            } else {
                kit.throwing = Some((w, under, left - 1));
            }
        }
        let _ = now;
        if strike {
            self.kit_melee_strike(slot);
        }
        if let Some((w, under)) = release {
            self.kit_release_grenade(slot, w, under);
        }
        self.sync_kit(slot);
    }

    fn kit_melee_strike(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_ref() else { return };
        let Some(kit) = p.combat.kit.as_ref() else { return };
        let weapon = kit.melee;
        let spec = weapon.kit();
        let (eye, look) = super::interact::eye_and_look(&p.state, p.crouching);
        let lag = p.view_lag as usize;
        let Some(hit) = self.probe_lagged(eye, look, spec.reach, slot, lag) else { return };
        match hit.target {
            RayTarget::Prop(prop) => {
                let mass = self.props.mass(prop);
                self.shove(prop, look, eye + look * hit.distance, 6.0 * mass.min(4.0));
                self.rules.inject(self.tick, "prop_hit", Some(slot));
            }
            RayTarget::Player(target) => {
                let Some(t) = self.players[target].as_ref() else { return };
                // A slash from behind: the victim faces the way the attacker is swinging.
                let (sy, cy) = libm::sincosf(t.state.yaw);
                let facing = glam::Vec2::new(sy, -cy);
                let swing = glam::Vec2::new(look.x, look.z).normalize_or_zero();
                let behind = facing.dot(swing) > 0.5;
                let amount = if weapon == Weapon::Knife && behind { 100 } else { spec.damage as u32 };
                if self.damage_ex(target, amount, slot, weapon, false) {
                    if let Some(p) = self.players[slot].as_mut() {
                        p.combat.hits = p.combat.hits.wrapping_add(1);
                    }
                }
            }
            RayTarget::Static => {}
        }
    }

    fn kit_release_grenade(&mut self, slot: usize, weapon: Weapon, underhand: bool) {
        let Some(p) = self.players[slot].as_mut() else { return };
        let (eye, look) = super::interact::eye_and_look(&p.state, p.crouching);
        let velocity = Vec3::new(p.state.velocity.x, p.state.vy.max(0.0), p.state.velocity.y);
        let Some(kit) = p.combat.kit.as_mut() else { return };
        // The grenade leaves the slot it was thrown from.
        if let Some(s) = kit.grenades.iter_mut().find(|s| **s == Some(weapon)) {
            *s = None;
        }
        kit.reselect();
        let spec = weapon.kit();
        let speed = if underhand { spec.speed * 0.5 } else { spec.speed };
        let lift = if underhand { 1.6 } else { 2.6 };
        let vel = look * speed + Vec3::Y * lift + velocity * 0.6;
        let origin = eye + look * 0.45 - Vec3::Y * 0.12;
        self.rules.inject(self.tick, "throw", Some(slot));
        self.launch(weapon, slot, origin, vel);
    }

    // ---- pickups and drops ------------------------------------------------------------------------------------------------------

    /// Puts `item` into player `slot`'s kit. `swap` lets a full slot trade with the item in hand. Returns what the player had to put down (if any)
    /// and whether the item was taken.
    fn kit_give(&mut self, slot: usize, weapon: Option<Weapon>, loaded: u16, reserve: u16, swap: bool) -> (bool, Option<(Weapon, u16, u16)>) {
        let Some(p) = self.players[slot].as_mut() else { return (false, None) };
        let Some(kit) = p.combat.kit.as_mut() else { return (false, None) };
        let Some(w) = weapon else {
            // An ammunition crate: a magazine more for every gun, up to the cap.
            let mut any = false;
            for g in kit.guns.iter_mut().flatten() {
                let cap = Gun::reserve_cap(g.weapon);
                let add = g.weapon.kit().mag.min(cap.saturating_sub(g.reserve));
                g.reserve += add;
                any |= add > 0;
            }
            return (any, None);
        };
        match w.class() {
            Class::Grenade => {
                if let Some(s) = kit.grenades.iter_mut().find(|s| s.is_none()) {
                    *s = Some(w);
                    return (true, None);
                }
                (false, None)
            }
            Class::Melee => {
                if !swap || kit.melee == w {
                    return (false, None);
                }
                let old = std::mem::replace(&mut kit.melee, w);
                if kit.sel == Slot::Melee {
                    kit.busy = DRAW_TICKS;
                }
                (true, (old != Weapon::Knife).then_some((old, 0, 0)))
            }
            _ => {
                // The same gun again tops up its ammunition.
                if let Some(g) = kit.guns.iter_mut().flatten().find(|g| g.weapon == w) {
                    let cap = Gun::reserve_cap(w);
                    let add = (loaded + reserve).min(cap.saturating_sub(g.reserve));
                    g.reserve += add;
                    return (add > 0, None);
                }
                if let Some(i) = kit.guns.iter().position(|g| g.is_none()) {
                    kit.guns[i] = Some(Gun { weapon: w, loaded, reserve });
                    kit.select(Slot::Gun(i as u8));
                    return (true, None);
                }
                if !swap {
                    return (false, None);
                }
                let i = match kit.sel {
                    Slot::Gun(i) => i as usize & 1,
                    _ => 0,
                };
                let old = kit.guns[i].replace(Gun { weapon: w, loaded, reserve });
                kit.sel = Slot::Gun(i as u8);
                kit.reload_left = 0;
                kit.busy = DRAW_TICKS;
                (true, old.map(|g| (g.weapon, g.loaded, g.reserve)))
            }
        }
    }

    /// Lays a weapon on the floor where player `slot` stands (or `ahead` of them).
    fn put_down(&mut self, slot: usize, item: (Weapon, u16, u16), ahead: bool) {
        let now = self.tick;
        let Some(p) = self.players[slot].as_ref() else { return };
        let (_, look) = super::interact::eye_and_look(&p.state, p.crouching);
        let flat = Vec3::new(look.x, 0.0, look.z).normalize_or_zero();
        let pos = Vec3::new(p.state.pos.x, p.state.foot_y + 0.28, p.state.pos.y) + if ahead { flat * 0.9 } else { Vec3::ZERO };
        self.add_dropped(item, pos, slot as u8, now + DROP_IGNORE_TICKS);
    }

    fn add_dropped(&mut self, item: (Weapon, u16, u16), pos: Vec3, ignore_slot: u8, ignore_until: u64) {
        let now = self.tick;
        let Some(a) = self.arena.as_mut() else { return };
        if a.dropped.len() >= MAX_DROPPED {
            a.dropped.remove(0);
        }
        let id = a.next_id();
        a.dropped.push(Dropped {
            id,
            weapon: item.0,
            loaded: item.1,
            reserve: item.2,
            pos,
            expires: now + secs_to_ticks(DROP_LIFETIME_SECS) as u64,
            ignore_slot,
            ignore_until,
        });
    }

    fn kit_drop(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_mut() else { return };
        let Some(kit) = p.combat.kit.as_mut() else { return };
        let sel = kit.sel;
        let Some(item) = kit.take(sel) else { return };
        kit.reselect();
        self.put_down(slot, item, true);
        self.sync_kit(slot);
    }

    /// Everything a dying player carried falls where they died.
    pub(super) fn kit_drop_all(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_mut() else { return };
        let Some(kit) = p.combat.kit.as_mut() else { return };
        let mut items = Vec::new();
        for s in [Slot::Gun(0), Slot::Gun(1), Slot::Grenade(0), Slot::Grenade(1)] {
            if let Some(item) = kit.take(s) {
                items.push(item);
            }
        }
        if kit.melee != Weapon::Knife {
            items.extend(kit.take(Slot::Melee));
        }
        let (x, y, z) = (p.state.pos.x, p.state.foot_y + 0.28, p.state.pos.y);
        for (k, item) in items.into_iter().enumerate() {
            let a = k as f32 * 1.7;
            self.add_dropped(item, Vec3::new(x + a.cos() * 0.5, y, z + a.sin() * 0.5), u8::MAX, 0);
        }
    }

    /// Interact: pick up the nearest weapon in reach, trading with the gun in hand if both gun slots are full.
    fn kit_interact(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_ref() else { return };
        let here = Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y);
        let now = self.tick;
        let Some(a) = self.arena.as_ref() else { return };
        enum Src {
            Map(usize),
            Dropped(usize),
        }
        let near = |pos: Vec3| {
            let d = (glam::Vec2::new(pos.x, pos.z) - glam::Vec2::new(here.x, here.z)).length();
            (d <= PICKUP_REACH && pos.y - here.y > -0.8 && pos.y - here.y < 2.2).then_some(d)
        };
        let mut best: Option<(f32, Src)> = None;
        for (i, m) in a.pickups.iter().enumerate() {
            if m.taken_until.is_none() {
                if let Some(d) = near(m.spawn.at) {
                    if best.as_ref().is_none_or(|b| d < b.0) {
                        best = Some((d, Src::Map(i)));
                    }
                }
            }
        }
        for (i, d) in a.dropped.iter().enumerate() {
            if d.ignore_slot as usize == slot && now < d.ignore_until {
                continue;
            }
            if let Some(dist) = near(d.pos) {
                if best.as_ref().is_none_or(|b| dist < b.0) {
                    best = Some((dist, Src::Dropped(i)));
                }
            }
        }
        let Some((_, src)) = best else { return };
        let (weapon, loaded, reserve) = match src {
            Src::Map(i) => {
                let m = &a.pickups[i];
                (m.spawn.weapon, m.loaded, m.reserve)
            }
            Src::Dropped(i) => {
                let d = &a.dropped[i];
                (Some(d.weapon), d.loaded, d.reserve)
            }
        };
        let (taken, old) = self.kit_give(slot, weapon, loaded, reserve, true);
        if !taken {
            return;
        }
        self.consume_pickup(match src {
            Src::Map(i) => Ok(i),
            Src::Dropped(i) => Err(i),
        });
        if let Some(old) = old {
            self.put_down(slot, old, false);
        }
        self.rules.inject(self.tick, "pickup", Some(slot));
        self.sync_kit(slot);
    }

    fn consume_pickup(&mut self, which: Result<usize, usize>) {
        let now = self.tick;
        let Some(a) = self.arena.as_mut() else { return };
        match which {
            Ok(i) => {
                let secs = a.pickups[i].spawn.respawn_secs;
                a.pickups[i].taken_until = Some(now + secs_to_ticks(secs) as u64);
            }
            Err(i) => {
                a.dropped.remove(i);
            }
        }
    }

    /// Walking over a weapon picks it up when there is room; expired drops vanish and taken spots come back.
    pub(super) fn kit_pickups_tick(&mut self) {
        let now = self.tick;
        let Some(a) = self.arena.as_mut() else { return };
        a.dropped.retain(|d| d.expires > now);
        for m in a.pickups.iter_mut() {
            if m.taken_until.is_some_and(|t| now >= t) {
                m.taken_until = None;
            }
        }
        for slot in 0..self.players.len() {
            let Some(p) = self.players[slot].as_ref() else { continue };
            if p.combat.is_dead() || p.combat.kit.is_none() {
                continue;
            }
            let here = Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y);
            let close = |pos: Vec3| {
                (glam::Vec2::new(pos.x, pos.z) - glam::Vec2::new(here.x, here.z)).length() <= PICKUP_RADIUS && pos.y - here.y > -0.8 && pos.y - here.y < 2.0
            };
            let Some(a) = self.arena.as_ref() else { return };
            let found = a
                .pickups
                .iter()
                .enumerate()
                .find(|(_, m)| m.taken_until.is_none() && close(m.spawn.at))
                .map(|(i, m)| (Ok(i), m.spawn.weapon, m.loaded, m.reserve))
                .or_else(|| {
                    a.dropped
                        .iter()
                        .enumerate()
                        .find(|(_, d)| !(d.ignore_slot as usize == slot && now < d.ignore_until) && close(d.pos))
                        .map(|(i, d)| (Err(i), Some(d.weapon), d.loaded, d.reserve))
                });
            let Some((which, weapon, loaded, reserve)) = found else { continue };
            // Walking over a weapon only takes it when it fits without trading: a free gun slot, a free grenade slot, or an ammunition crate.
            if weapon.is_some_and(|w| w.is_melee()) {
                continue;
            }
            let (taken, _) = self.kit_give(slot, weapon, loaded, reserve, false);
            if taken {
                self.consume_pickup(which);
                self.rules.inject(now, "pickup", Some(slot));
                self.sync_kit(slot);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kit_is_built_from_a_list_and_holds_two_guns() {
        let kit = Kit::from_weapons(&[Weapon::Pistol, Weapon::Knife, Weapon::Frag]);
        assert_eq!(kit.current(), Weapon::Pistol);
        assert_eq!(kit.melee, Weapon::Knife);
        assert_eq!(kit.grenade_count(), 1);
        assert_eq!(kit.gun().map(|g| (g.loaded, g.reserve)), Some((17, 68)), "a full magazine and the usual reserve, its own");
    }

    #[test]
    fn selection_cycles_over_what_is_carried_only() {
        let mut kit = Kit::from_weapons(&[Weapon::Pistol, Weapon::Rifle, Weapon::Knife, Weapon::Flash]);
        assert_eq!(kit.cycled(1), Some(Slot::Gun(1)));
        kit.select(Slot::Gun(1));
        assert_eq!(kit.cycled(1), Some(Slot::Melee));
        assert_eq!(kit.cycled(-1), Some(Slot::Gun(0)));
        kit.select(Slot::Melee);
        assert_eq!(kit.cycled(1), Some(Slot::Grenade(0)));
        assert!(!kit.select(Slot::Grenade(1)), "an empty slot cannot be chosen");
        assert_eq!(kit.last, Slot::Gun(1));
    }

    #[test]
    fn falloff_is_full_inside_the_effective_range_and_fades_to_the_far_share() {
        let s = Weapon::Shotgun.kit();
        assert_eq!(falloff(&s, 3.0), 1.0);
        assert!((falloff(&s, s.range) - s.far_mult).abs() < 1e-4);
        assert!(falloff(&s, 20.0) < falloff(&s, 12.0));
    }

    #[test]
    fn scatter_stays_inside_the_cone() {
        let dir = Vec3::new(0.3, 0.1, -0.9).normalize();
        for k in 0..200 {
            let (u, v) = unit_pair(k);
            let d = scatter(dir, 3.0, u, v);
            assert!(d.dot(dir) >= 3.0f32.to_radians().cos() - 1e-4);
            assert!((d.length() - 1.0).abs() < 1e-4);
        }
        assert_eq!(scatter(dir, 0.0, 0.3, 0.3), dir);
    }
}
