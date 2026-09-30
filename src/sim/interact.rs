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
//! - **Firearms** (`attack` with a firearm out): a hitscan shot (nine rays for the shotgun) with the firearm's own cooldown and the player's
//!   ammunition (`weapons.ammo`, one supply for every firearm); `reload` refills from the reserve; an empty click is a short cooldown.
//! - **Health**: hits damage a player; at 0 they die (dropping what they carry), and respawn after the scene's respawn delay
//!   ([`RESPAWN_TICKS`](crate::weapons::RESPAWN_TICKS) by default). `pickup`, `drop`, `shot`, `hit`, `kill` and `respawn` are raised as engine events so scene
//!   `rules` can react (score a kill, end a round). Numbers are data: `weapons` and `combat` in the scene.
//! - **Ladder** (`weapons.ladder`, Gun Game): a player carries `ladder[kills]`; a kill hands the killer the next rung at once, a respawn
//!   restores the rung they were on, and weapons cannot be switched by hand.
//! - **Pacing** (`combat`): spawn protection, health regeneration and where the dead reappear (farthest from every living opponent).

use super::combat::{Cooldown, MeleeSwing, WeaponSwitch};
use super::combat_cfg::{SpawnPolicy, REGEN_UNIT};
use super::match_sim::MatchSim;
use super::player::{PlayerInput, PlayerState};
use super::spawns::Spawn;
use crate::hit::raycast_shapes;
use crate::player::Character;
use crate::weapons::{Ammo, Weapon, WeaponConfig, BAT_REACH, DRY_FIRE_COOLDOWN_TICKS, PLAYER_MAX_HP};
use glam::{Vec2, Vec3};

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
    /// The player's ammunition (one supply for every firearm they carry).
    pub ammo: Ammo,
    /// Hit points.
    pub hp: u32,
    /// The tick a dead player respawns on (`None` while alive).
    pub dead_until: Option<u64>,
    /// Players this one has killed.
    pub kills: u32,
    /// Times this player has died.
    pub deaths: u32,
    /// The tick spawn protection ends (damage is ignored before it); `0` = not protected.
    pub protected_until: u64,
    /// The tick this player last took damage (`0` = never): health regeneration starts a while after it.
    pub last_hurt_tick: u64,
    regen_acc: u32,
    /// Feedback counters (wrapping, never reset): shots fired, attacks that damaged someone, times damaged. A client turns increments into
    /// sounds, hit markers and the damage flash; they do not affect play.
    pub shots: u32,
    /// See [`Combat::shots`].
    pub hits: u32,
    /// See [`Combat::shots`].
    pub hurt: u32,
    /// World bearing (yaw convention, radians) from this player toward whoever damaged them last.
    pub hurt_bearing: f32,
    /// Button state on the previous processed input (buttons act on their rising edge): interact, attack, reload, switch.
    prev: [bool; 4],
    /// The loadout (two guns, melee, grenades), in a match whose scene has a `shooter` block; `None` in the classic arena.
    pub kit: Option<super::kit::Kit>,
    /// The tick until which the player is blinded by a flashbang (`0` = not).
    pub flash_until: u64,
    /// How long the blinding flash lasts in total, seconds (for the fade).
    pub flash_total: f32,
    /// Who killed this player, with what, and whether it was a headshot (set on death).
    pub killed_by: Option<(u8, Weapon, bool)>,
    /// Kills that were headshots (wrapping; a counter like `hits`).
    pub headshots: u32,
}

/// How many failed pick-ups a match remembers for its report.
const MAX_PICKUP_MISSES: usize = 8;

impl Combat {
    /// A fresh, alive player carrying the bat.
    pub fn new(cfg: &WeaponConfig) -> Combat {
        Combat {
            weapon: cfg.weapon_for_kills(0),
            swing: MeleeSwing::default(),
            cooldown: Cooldown::default(),
            switch: WeaponSwitch::default(),
            ammo: cfg.ammo,
            hp: PLAYER_MAX_HP,
            dead_until: None,
            kills: 0,
            deaths: 0,
            protected_until: 0,
            last_hurt_tick: 0,
            regen_acc: 0,
            shots: 0,
            hits: 0,
            hurt: 0,
            hurt_bearing: 0.0,
            prev: [false; 4],
            kit: None,
            flash_until: 0,
            flash_total: 0.0,
            killed_by: None,
            headshots: 0,
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
        if let Some(kit) = &self.kit {
            h ^= kit.state_hash();
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // The pacing state joins the fold only while it is in play, so a match that does not use spawn protection or regeneration keeps
        // exactly the checksums it always had (committed traces stay valid).
        if self.protected_until != 0 || self.last_hurt_tick != 0 || self.regen_acc != 0 {
            for v in [self.protected_until, self.last_hurt_tick, self.regen_acc as u64] {
                h ^= v;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }
}

/// Eye position and look direction of a player.
pub(super) fn eye_and_look(state: &PlayerState, crouching: bool) -> (Vec3, Vec3) {
    let body = state.character.body();
    let eye = Vec3::new(state.pos.x, state.foot_y + if crouching { body.crouch_eye } else { body.stand_eye }, state.pos.y);
    let (sy, cy) = libm::sincosf(state.yaw);
    let (sp, cp) = libm::sincosf(state.pitch);
    (eye, Vec3::new(sy * cp, sp, -cy * cp).normalize_or_zero())
}

/// Distance along a unit ray to a player's vertical cylinder (centre on the ground plane, `foot` to `foot + height`), if within `reach`.
pub fn ray_cylinder(origin: Vec3, dir: Vec3, reach: f32, centre: glam::Vec2, radius: f32, foot: f32, height: f32) -> Option<f32> {
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
        self.probe_lagged(origin, dir, reach, ignore, 0)
    }

    /// [`probe`](Self::probe) with the other players where they were `lag` ticks ago: what a shooter whose screen is that far behind the
    /// server actually aimed at (lag compensation, ADR 0053). Fixed geometry and props are judged as they are now.
    pub fn probe_lagged(&self, origin: Vec3, dir: Vec3, reach: f32, ignore: usize, lag: usize) -> Option<RayHit> {
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
            let (pos, foot) = self.rewound(slot, lag).unwrap_or((p.state.pos, p.state.foot_y));
            // A crouching player is a shorter target.
            let height = if p.crouching { body.crouch_eye + 0.12 } else { body.body_height };
            if let Some(d) = ray_cylinder(origin, dir, reach, pos, body.radius, foot, height) {
                consider(RayHit { target: RayTarget::Player(slot), distance: d });
            }
        }
        best
    }

    /// Advances player `slot`'s combat timers one tick: cooldowns, the bat swing (resolving its strike), and the respawn clock.
    pub(super) fn combat_tick(&mut self, slot: usize) {
        let (now, rules) = (self.tick, self.combat_cfg);
        let Some(p) = self.players[slot].as_mut() else { return };
        p.combat.cooldown.tick();
        p.combat.switch.tick();
        if let Some(t) = p.combat.dead_until {
            if now >= t {
                self.respawn(slot);
            }
            return;
        }
        if rules.regen_per_sec > 0 && p.combat.hp < PLAYER_MAX_HP && now >= p.combat.last_hurt_tick + rules.regen_delay_ticks {
            p.combat.regen_acc += rules.regen_per_sec;
            while p.combat.regen_acc >= REGEN_UNIT {
                p.combat.regen_acc -= REGEN_UNIT;
                p.combat.hp = (p.combat.hp + 1).min(PLAYER_MAX_HP);
            }
        }
        if p.combat.kit.is_some() {
            self.kit_tick(slot);
            return;
        }
        if p.combat.swing.tick() {
            self.melee_strike(slot);
        }
    }

    /// The spawn point for a player entering the world in `slot` (`usize::MAX` = a slot nobody holds yet), and advances the round-robin cursor.
    ///
    /// `round_robin` walks the list. `farthest` scores every spawn by its distance to the nearest living opponent (a floor apart counts
    /// extra), minus a penalty when any opponent has a clear line to it, and takes the best; ties keep the round-robin order, so a match
    /// with nobody else in it still rotates through the list.
    pub(super) fn pick_spawn(&mut self, slot: usize) -> Spawn {
        let team = self.team_of(slot);
        self.pick_spawn_team(slot, team)
    }

    /// [`pick_spawn`](Self::pick_spawn) for a player about to join `team`. In a team match a player spawns at their own team's spawn points
    /// (`group` "team1" / "team2"), when the map has any.
    pub(super) fn pick_spawn_team(&mut self, slot: usize, team: u8) -> Spawn {
        if team != 0 {
            let group = format!("team{team}");
            let mine: Vec<usize> = (0..self.spawns.len()).filter(|i| self.spawns[*i].group == group).collect();
            if !mine.is_empty() {
                let start = self.next_spawn % mine.len();
                self.next_spawn += 1;
                return self.pick_from(slot, team, &mine, start);
            }
        }
        let all: Vec<usize> = (0..self.spawns.len()).collect();
        let start = self.next_spawn % all.len();
        self.next_spawn += 1;
        self.pick_from(slot, team, &all, start)
    }

    /// The spawn among `candidates` (indices into the spawn list) for `slot`; `start` is the round-robin position in that list.
    fn pick_from(&mut self, slot: usize, team: u8, candidates: &[usize], start: usize) -> Spawn {
        let n = candidates.len();
        if self.combat_cfg.spawn != SpawnPolicy::Farthest {
            return self.spawns[candidates[start]].clone();
        }
        let others: Vec<(usize, Vec3, Vec3)> = self
            .players()
            .filter(|(s, p)| *s != slot && !p.combat.is_dead() && (team == 0 || p.team != team))
            .map(|(s, p)| {
                let body = p.state.character.body();
                (s, Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y), Vec3::new(p.state.pos.x, p.state.foot_y + body.stand_eye, p.state.pos.y))
            })
            .collect();
        if others.is_empty() {
            return self.spawns[candidates[start]].clone();
        }
        let mut best = (f32::NEG_INFINITY, candidates[start]);
        for k in 0..n {
            let i = candidates[(start + k) % n];
            let at = Vec3::from(self.spawns[i].position);
            let (mut nearest, mut seen) = (f32::INFINITY, false);
            for (other, feet, eye) in &others {
                nearest = nearest.min(Vec2::new(feet.x - at.x, feet.z - at.z).length() + (feet.y - at.y).abs() * 2.0);
                if !seen {
                    let to = at + Vec3::Y * 1.5;
                    let (d, dist) = (to - *eye, (to - *eye).length());
                    seen = dist < 70.0 && dist > 0.01 && self.probe(*eye, d / dist, dist, *other).is_none();
                }
            }
            let score = nearest - if seen { 25.0 } else { 0.0 };
            if score > best.0 {
                best = (score, i);
            }
        }
        self.spawns[best.1].clone()
    }

    fn respawn(&mut self, slot: usize) {
        let s = self.pick_spawn(slot);
        let (cfg, protect, now) = (self.weapons, self.combat_cfg.protect_ticks, self.tick);
        let Some(p) = self.players[slot].as_mut() else { return };
        let character = p.state.character;
        p.state = PlayerState::spawn(s.position[0], s.position[2], s.position[1], s.yaw_deg, character);
        let old = &p.combat;
        let (kills, deaths, shots, hits, hurt, hurt_bearing, headshots) = (old.kills, old.deaths, old.shots, old.hits, old.hurt, old.hurt_bearing, old.headshots);
        p.combat = Combat { kills, deaths, shots, hits, hurt, hurt_bearing, headshots, weapon: cfg.weapon_for_kills(kills), ..Combat::new(&cfg) };
        if let Some(arena) = &self.arena {
            p.combat.kit = Some(arena.cfg.start_kit());
        }
        p.combat.protected_until = if protect > 0 { now + protect } else { 0 };
        let body = character.body();
        let foot = Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y);
        self.props.set_player_slot(slot, foot, body.radius, body.body_height);
        self.sync_kit(slot);
        self.rules.inject(self.tick, "respawn", Some(slot));
    }

    /// Reads player `slot`'s buttons: a button acts on the tick it goes down.
    pub(super) fn handle_actions(&mut self, slot: usize, input: &PlayerInput) {
        let Some(p) = self.players[slot].as_mut() else { return };
        if p.combat.is_dead() {
            p.combat.prev = [false; 4];
            return;
        }
        if p.combat.kit.is_some() {
            self.kit_actions(slot, input);
            return;
        }
        // A peaceful scene has no weapons: the primary button *is* interact, and reload / switch do nothing.
        let peaceful = self.player_tuning.mode.is_peaceful();
        let now =
            if peaceful { [input.interact || input.attack, false, false, false] } else { [input.interact, input.attack, input.reload, input.switch_weapon] };
        let edge: [bool; 4] = std::array::from_fn(|i| now[i] && !p.combat.prev[i]);
        p.combat.prev = now;
        let repeating = input.attack && p.combat.weapon.automatic();
        if edge[0] && !self.interact(slot) && peaceful {
            self.rules.inject(self.tick, "interact", Some(slot));
        }
        if edge[3] {
            self.switch_weapon(slot);
        }
        if edge[2] {
            if let Some(p) = self.players[slot].as_mut().filter(|p| p.combat.weapon.is_firearm()) {
                p.combat.ammo.reload();
            }
        }
        if edge[1] || repeating {
            self.attack(slot);
        }
    }

    /// `E` (or a peaceful scene's primary button): drop what is carried, else pick up what is aimed at. True when something happened.
    fn interact(&mut self, slot: usize) -> bool {
        let Some(p) = self.players[slot].as_ref() else { return false };
        let (eye, look) = eye_and_look(&p.state, p.crouching);
        let body = p.state.character.body();
        if self.props.held_by(slot).is_some() {
            let velocity = crate::sim::player::release_velocity(&p.state, look, self.player_tuning.throw_speed);
            if self.props.drop_held_by(slot, velocity).is_some() {
                self.rules.inject(self.tick, "drop", Some(slot));
                return true;
            }
        } else if let Some(prop) = self.props.pick_target_for(slot, eye, look, body.pickup_reach, &body.carry) {
            if self.props.pick_up_by(slot, prop) {
                if let Some(p) = self.players[slot].as_mut() {
                    p.combat.swing.cancel();
                }
                self.rules.inject(self.tick, "pickup", Some(slot));
                return true;
            }
        } else if self.pickup_misses.len() < MAX_PICKUP_MISSES {
            let why = self.props.why_no_pickup(eye, look, body.pickup_reach, &body.carry);
            self.pickup_misses.push((self.tick, slot, why));
        }
        false
    }

    /// The first few failed pick-ups of this match (`(tick, slot, why)`): what `sim` prints so an empty-handed `interact` is explained.
    pub fn pickup_misses(&self) -> &[(u64, usize, String)] {
        &self.pickup_misses
    }

    fn switch_weapon(&mut self, slot: usize) {
        if self.weapons.has_ladder() {
            return; // on a ladder the weapon in your hands is the one you earned
        }
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
        p.combat.protected_until = 0; // raising a weapon ends spawn protection
        match p.combat.weapon {
            Weapon::Bat => {
                p.combat.swing.start();
                self.rules.inject(self.tick, "swing", Some(slot));
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
                p.combat.shots = p.combat.shots.wrapping_add(1);
                let lag = p.view_lag as usize;
                let (eye, look) = eye_and_look(&p.state, p.crouching);
                self.rules.inject(self.tick, "shot", Some(slot));
                let (mut landed, mut struck_prop) = (false, false);
                for pellet in 0..firearm.pellets() {
                    let dir = firearm.shot_direction(look, pellet);
                    if let Some(hit) = self.probe_lagged(eye, dir, spec.range, slot, lag) {
                        match hit.target {
                            RayTarget::Prop(prop) => {
                                self.shove(prop, dir, eye + dir * hit.distance, spec.impulse / firearm.pellets() as f32);
                                struck_prop = true;
                            }
                            RayTarget::Player(target) => landed |= self.damage(target, firearm.pellet_damage(self.weapons.damage(firearm), pellet), slot),
                            RayTarget::Static => {}
                        }
                    }
                }
                if struck_prop {
                    self.rules.inject(self.tick, "prop_hit", Some(slot));
                }
                if landed {
                    if let Some(p) = self.players[slot].as_mut() {
                        p.combat.hits = p.combat.hits.wrapping_add(1);
                    }
                }
            }
        }
    }

    fn melee_strike(&mut self, slot: usize) {
        let Some(p) = self.players[slot].as_ref() else { return };
        let (eye, look) = eye_and_look(&p.state, p.crouching);
        let lag = p.view_lag as usize;
        let Some(hit) = self.probe_lagged(eye, look, BAT_REACH, slot, lag) else { return };
        match hit.target {
            RayTarget::Prop(prop) => {
                let mass = self.props.mass(prop);
                self.shove(prop, look, eye + look * hit.distance, 6.0 * mass.min(4.0));
                self.rules.inject(self.tick, "prop_hit", Some(slot));
            }
            RayTarget::Player(target) => {
                if self.damage(target, self.weapons.bat_damage, slot) {
                    if let Some(p) = self.players[slot].as_mut() {
                        p.combat.hits = p.combat.hits.wrapping_add(1);
                    }
                }
            }
            RayTarget::Static => {}
        }
    }

    /// `by` damages `target`; at 0 hit points the target dies (dropping what it carries) and `by` scores a kill (and, on a weapon
    /// ladder, climbs a rung). Returns whether any damage landed: `false` for a dead target or one under spawn protection.
    fn damage(&mut self, target: usize, amount: u32, by: usize) -> bool {
        let weapon = self.players.get(by).and_then(Option::as_ref).map_or(Weapon::Bat, |p| p.combat.weapon);
        self.damage_ex(target, amount, by, weapon, false)
    }

    /// [`damage`](Self::damage) with what dealt it and whether it was a headshot. In a team match a bullet or blast never hurts a teammate unless
    /// the scene allows friendly fire (a player's own blast still hurts them).
    pub(super) fn damage_ex(&mut self, target: usize, amount: u32, by: usize, weapon: Weapon, headshot: bool) -> bool {
        let (now, respawn_ticks, cfg, regen) = (self.tick, self.combat_cfg.respawn_ticks, self.weapons, self.combat_cfg.regen_per_sec > 0);
        let friendly_fire = self.arena.as_ref().is_none_or(|a| a.cfg.friendly_fire);
        let (by_team, target_team) = (self.team_of(by), self.team_of(target));
        let same_team = by != target && by_team != 0 && by_team == target_team;
        if same_team && !friendly_fire {
            return false;
        }
        let bearing = match (self.players.get(by).and_then(Option::as_ref), self.players[target].as_ref()) {
            (Some(a), Some(v)) => {
                let d = a.state.pos - v.state.pos;
                libm::atan2f(d.x, -d.y)
            }
            _ => 0.0,
        };
        let Some(t) = self.players[target].as_mut().filter(|t| !t.combat.is_dead()) else { return false };
        if now < t.combat.protected_until {
            return false;
        }
        t.combat.hp = t.combat.hp.saturating_sub(amount);
        if regen {
            t.combat.last_hurt_tick = now; // only tracked when it is used, so it stays out of the checksum otherwise
            t.combat.regen_acc = 0;
        }
        t.combat.hurt = t.combat.hurt.wrapping_add(1);
        t.combat.hurt_bearing = bearing;
        self.rules.inject(now, "hit", Some(by));
        if t.combat.hp > 0 {
            return true;
        }
        t.combat.dead_until = Some(now + respawn_ticks);
        t.combat.deaths += 1;
        t.combat.swing.cancel();
        t.combat.killed_by = Some((by as u8, weapon, headshot));
        if let Some(kit) = t.combat.kit.as_mut() {
            kit.reload_left = 0;
            kit.throwing = None;
            kit.strike_in = 0;
        }
        self.props.drop_held_by(target, Vec3::ZERO);
        if self.arena.is_some() {
            self.kit_drop_all(target);
        }
        let scores = by != target && !same_team;
        if let Some(killer) = self.players.get_mut(by).and_then(Option::as_mut).filter(|_| scores) {
            killer.combat.kills += 1;
            if headshot {
                killer.combat.headshots = killer.combat.headshots.wrapping_add(1);
            }
            if cfg.has_ladder() {
                let (old, next) = (killer.combat.weapon, cfg.weapon_for_kills(killer.combat.kills));
                if next != old {
                    killer.combat.swing.cancel();
                    killer.combat.switch.start(old);
                    killer.combat.weapon = next;
                    killer.combat.ammo = cfg.ammo;
                }
            }
        }
        if let Some(arena) = self.arena.as_mut() {
            if scores && by_team > 0 {
                arena.team_kills[(by_team as usize - 1) & 1] += 1;
            }
            arena.record_kill(super::shooter::KillRecord { tick: now, killer: by as u8, victim: target as u8, weapon, headshot });
        }
        self.rules.inject(now, "kill", Some(by));
        true
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
