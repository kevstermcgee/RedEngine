//! Authoritative interactions: pick-up and drop, melee, hitscan, cooldowns, damage, death and respawn.
//!
//! Everything a player can *do* to the world besides walk lives here as rules on [`MatchSim`], with no window, GPU or
//! socket: the server, the headless scenario runner and a replay all run exactly this. A client only ever sends
//! *buttons* (`PlayerInput::interact/attack/reload/switch_weapon`); what they do, who wins a contested prop, how much
//! damage lands and when a player respawns is decided here.
//!
//! - **Pick-up / drop** (`interact`): the prop under the crosshair within the character's reach that its carry limits
//!   allow, never one somebody else already holds (contention is settled in slot order, deterministically). The carried prop
//!   follows the holder's look each tick and is published like any moving prop, so every client sees it move.
//! - **Bat** (`attack`, humans): a swing whose strike lands after the windup ticks; it shoves a prop or damages a player.
//! - **Revolver** (`attack` with the revolver out): a hitscan shot with a cooldown and ammo (`weapons.revolver.ammo`);
//!   `reload` refills from the reserve; an empty click is a short cooldown.
//! - **Health**: hits damage a player; at 0 they die (dropping what they carry), and respawn after
//!   [`RESPAWN_TICKS`]. `pickup`, `drop`, `shot`, `hit`, `kill` and `respawn` are raised as engine events so scene
//!   `rules` can react (score a kill, end a round). Numbers are data: `weapons` in the scene.

use super::combat::{Cooldown, MeleeSwing, WeaponSwitch};
use super::match_sim::MatchSim;
use super::player::{PlayerInput, PlayerState};
use crate::hit::raycast_shapes;
use crate::player::Character;
use crate::weapons::{Ammo, Weapon, WeaponConfig, BAT_REACH, DRY_FIRE_COOLDOWN_TICKS, PLAYER_MAX_HP, RESPAWN_TICKS};
use glam::Vec3;

/// What a ray met first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RayTarget {
    /// Fixed geometry (a wall, the floor, furniture that is not loose).
    Static,
    /// A loose prop (index in `physics::loose_props`).
    Prop(usize),
    /// Another player (slot).
    Player(usize),
}

/// A ray's first hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// What it hit.
    pub target: RayTarget,
    /// Distance from the ray origin, m.
    pub distance: f32,
}

/// One player's combat state: weapon, timers, ammo, health and score.
#[derive(Debug, Clone)]
pub struct Combat {
    /// The weapon in hand.
    pub weapon: Weapon,
    swing: MeleeSwing,
    cooldown: Cooldown,
    switch: WeaponSwitch,
    /// The revolver's ammunition.
    pub ammo: Ammo,
    /// Hit points.
    pub hp: u32,
    /// The tick a dead player respawns on (`None` while alive).
    pub dead_until: Option<u64>,
    /// Players this one has killed.
    pub kills: u32,
    /// Times this player has died.
    pub deaths: u32,
    /// Button state on the previous processed input (buttons act on their rising edge): interact, attack, reload, switch.
    prev: [bool; 4],
}

impl Combat {
    /// A fresh, alive player carrying the bat.
    pub fn new(cfg: &WeaponConfig) -> Combat {
        Combat {
            weapon: Weapon::Bat,
            swing: MeleeSwing::default(),
            cooldown: Cooldown::default(),
            switch: WeaponSwitch::default(),
            ammo: cfg.revolver_ammo,
            hp: PLAYER_MAX_HP,
            dead_until: None,
            kills: 0,
            deaths: 0,
            prev: [false; 4],
        }
    }

    /// Whether the player is dead (waiting to respawn).
    pub fn is_dead(&self) -> bool {
        self.dead_until.is_some()
    }

    /// Whether a bat swing is under way.
    pub fn is_swinging(&self) -> bool {
        !self.swing.is_idle()
    }

    /// A fold of everything here that the rules of the game depend on (goes into the match checksum).
    pub fn state_hash(&self) -> u64 {
        let (loaded, reserve) = match self.ammo {
            Ammo::Infinite => (u32::MAX, 0),
            Ammo::Limited { loaded, reserve, .. } => (loaded, reserve),
        };
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [
            self.weapon.wire() as u64,
            self.hp as u64,
            self.dead_until.map_or(0, |t| t + 1),
            self.kills as u64,
            self.deaths as u64,
            loaded as u64,
            reserve as u64,
            !self.cooldown.ready() as u64,
            self.swing.is_idle() as u64,
            self.switch.is_active() as u64,
        ] {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
}

/// Eye position and look direction of a player.
fn eye_and_look(state: &PlayerState, crouching: bool) -> (Vec3, Vec3) {
    let body = state.character.body();
    let eye = Vec3::new(state.pos.x, state.foot_y + if crouching { body.crouch_eye } else { body.stand_eye }, state.pos.y);
    let (sy, cy) = libm::sincosf(state.yaw);
    let (sp, cp) = libm::sincosf(state.pitch);
    (eye, Vec3::new(sy * cp, sp, -cy * cp).normalize_or_zero())
}

/// Distance along a unit ray to a player's vertical cylinder, if within `reach`.
fn ray_cylinder(origin: Vec3, dir: Vec3, reach: f32, centre: glam::Vec2, radius: f32, foot: f32, height: f32) -> Option<f32> {
    let (ox, oz) = (origin.x - centre.x, origin.z - centre.y);
    let a = dir.x * dir.x + dir.z * dir.z;
    if a < 1e-8 {
        return None;
    }
    let b = 2.0 * (ox * dir.x + oz * dir.z);
    let c = ox * ox + oz * oz - radius * radius;
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let t0 = (-b - root) / (2.0 * a);
    let t = if t0 >= 0.0 { t0 } else { (-b + root) / (2.0 * a) };
    if !(0.0..=reach).contains(&t) {
        return None;
    }
    let y = origin.y + dir.y * t;
    (y >= foot && y <= foot + height).then_some(t)
}

impl MatchSim {
    /// The nearest thing a ray from `origin` along `dir` meets within `reach`, ignoring player `ignore` (the shooter):
    /// fixed geometry (exact shapes), a loose prop, or another living player.
    pub fn probe(&self, origin: Vec3, dir: Vec3, reach: f32, ignore: usize) -> Option<RayHit> {
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }
        let mut best = raycast_shapes(origin, dir, reach, &self.hit_shapes).map(|h| RayHit { target: RayTarget::Static, distance: h.distance });
        let mut consider = |hit: RayHit| {
            if best.is_none_or(|b| hit.distance < b.distance) {
                best = Some(hit);
            }
        };
        if let Some((prop, d)) = self.props.ray_props(origin, dir, reach) {
            consider(RayHit { target: RayTarget::Prop(prop), distance: d });
        }
        for (slot, p) in self.players().filter(|(s, p)| *s != ignore && !p.combat.is_dead()) {
            let body = p.state.character.body();
            if let Some(d) = ray_cylinder(origin, dir, reach, p.state.pos, body.radius, p.state.foot_y, body.body_height) {
                consider(RayHit { target: RayTarget::Player(slot), distance: d });
            }
        }
        best
    }

    /// Advances player `slot`'s combat timers one tick: cooldowns, the bat swing (resolving its strike), and the respawn clock.
    pub(super) fn combat_tick(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_mut() else { return };
        p.combat.cooldown.tick();
        p.combat.switch.tick();
        if let Some(t) = p.combat.dead_until {
            if self.tick >= t {
                self.respawn(slot);
            }
            return;
        }
        if p.combat.swing.tick() {
            self.melee_strike(slot);
        }
    }

    fn respawn(&mut self, slot: usize) {
        let s = self.spawns[self.next_spawn % self.spawns.len()].clone();
        self.next_spawn += 1;
        let cfg = self.weapons;
        let Some(p) = self.players[slot].as_mut() else { return };
        let character = p.state.character;
        p.state = PlayerState::spawn(s.position[0], s.position[2], s.position[1], s.yaw_deg, character);
        p.combat = Combat { kills: p.combat.kills, deaths: p.combat.deaths, ..Combat::new(&cfg) };
        let body = character.body();
        let foot = Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y);
        self.props.set_player_slot(slot, foot, body.radius, body.body_height);
        self.rules.inject(self.tick, "respawn", Some(slot));
    }

    /// Reads player `slot`'s buttons: a button acts on the tick it goes down.
    pub(super) fn handle_actions(&mut self, slot: usize, input: &PlayerInput) {
        let Some(p) = self.players[slot].as_mut() else { return };
        if p.combat.is_dead() {
            p.combat.prev = [false; 4];
            return;
        }
        let now = [input.interact, input.attack, input.reload, input.switch_weapon];
        let edge: [bool; 4] = std::array::from_fn(|i| now[i] && !p.combat.prev[i]);
        p.combat.prev = now;
        if edge[0] {
            self.interact(slot);
        }
        if edge[3] {
            self.switch_weapon(slot);
        }
        if edge[2] {
            if let Some(p) = self.players[slot].as_mut().filter(|p| p.combat.weapon.is_firearm()) {
                p.combat.ammo.reload();
            }
        }
        if edge[1] {
            self.attack(slot);
        }
    }

    fn interact(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_ref() else { return };
        let (eye, look) = eye_and_look(&p.state, p.crouching);
        let body = p.state.character.body();
        if self.props.held_by(slot).is_some() {
            let flat = Vec3::new(look.x, 0.0, look.z).normalize_or_zero();
            if self.props.drop_held_by(slot, flat).is_some() {
                self.rules.inject(self.tick, "drop", Some(slot));
            }
        } else if let Some(prop) = self.props.pick_target_for(slot, eye, look, body.pickup_reach, &body.carry) {
            if self.props.pick_up_by(slot, prop) {
                if let Some(p) = self.players[slot].as_mut() {
                    p.combat.swing.cancel();
                }
                self.rules.inject(self.tick, "pickup", Some(slot));
            }
        }
    }

    fn switch_weapon(&mut self, slot: usize) {
        let carrying = self.props.held_by(slot).is_some();
        let Some(p) = self.players[slot].as_mut() else { return };
        if !p.state.character.body().has_bat || carrying || p.combat.switch.is_active() {
            return;
        }
        p.combat.switch.start(p.combat.weapon);
        p.combat.weapon = p.combat.weapon.cycle(1);
        p.combat.swing.cancel();
    }

    fn attack(&mut self, slot: usize) {
        let carrying = self.props.held_by(slot).is_some();
        let Some(p) = self.players[slot].as_mut() else { return };
        if !p.state.character.body().has_bat || carrying || p.combat.switch.is_active() {
            return;
        }
        match p.combat.weapon {
            Weapon::Bat => {
                p.combat.swing.start();
            }
            firearm => {
                let Some(spec) = firearm.firearm() else { return };
                if !p.combat.cooldown.ready() {
                    return;
                }
                if !p.combat.ammo.try_fire() {
                    p.combat.cooldown.start(DRY_FIRE_COOLDOWN_TICKS);
                    return;
                }
                p.combat.cooldown.start(spec.cooldown_ticks);
                let (eye, look) = eye_and_look(&p.state, p.crouching);
                self.rules.inject(self.tick, "shot", Some(slot));
                if let Some(hit) = self.probe(eye, look, spec.range, slot) {
                    match hit.target {
                        RayTarget::Prop(prop) => self.apply_impulse(prop, look, eye + look * hit.distance, spec.impulse),
                        RayTarget::Player(target) => self.damage(target, self.weapons.damage(firearm), slot),
                        RayTarget::Static => {}
                    }
                }
            }
        }
    }

    fn melee_strike(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_ref() else { return };
        let (eye, look) = eye_and_look(&p.state, p.crouching);
        let Some(hit) = self.probe(eye, look, BAT_REACH, slot) else { return };
        match hit.target {
            RayTarget::Prop(prop) => {
                let mass = self.props.mass(prop);
                self.apply_impulse(prop, look, eye + look * hit.distance, 6.0 * mass.min(4.0));
            }
            RayTarget::Player(target) => self.damage(target, self.weapons.bat_damage, slot),
            RayTarget::Static => {}
        }
    }

    /// `by` damages `target`; at 0 hit points the target dies (dropping what it carries) and `by` scores a kill.
    fn damage(&mut self, target: usize, amount: u32, by: usize) {
        let Some(t) = self.players[target].as_mut().filter(|t| !t.combat.is_dead()) else { return };
        t.combat.hp = t.combat.hp.saturating_sub(amount);
        self.rules.inject(self.tick, "hit", Some(by));
        if t.combat.hp > 0 {
            return;
        }
        t.combat.dead_until = Some(self.tick + RESPAWN_TICKS);
        t.combat.deaths += 1;
        t.combat.swing.cancel();
        self.props.drop_held_by(target, Vec3::ZERO);
        if let Some(killer) = self.players.get_mut(by).and_then(Option::as_mut) {
            killer.combat.kills += 1;
        }
        self.rules.inject(self.tick, "kill", Some(by));
    }

    /// Keeps the prop `slot` carries in front of them (called every tick, before the physics step).
    pub(super) fn update_held(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_ref() else { return };
        let Some(prop) = self.props.held_by(slot) else { return };
        let (eye, look) = eye_and_look(&p.state, p.crouching);
        let body = p.state.character.body();
        let pose = self.props.hold_pose(prop, eye, look, body.radius, body.hold_drop, p.state.foot_y);
        self.props.set_held_pose_for(slot, pose);
    }

    /// A character's weapon numbers come from the scene (see [`WeaponConfig`]).
    pub fn weapon_config(&self) -> &WeaponConfig {
        &self.weapons
    }

    /// Whether `character` can attack at all (a rat cannot).
    pub fn can_attack(character: Character) -> bool {
        character.body().has_bat
    }
}
