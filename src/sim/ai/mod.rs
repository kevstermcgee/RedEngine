//! Bots: AI players that live inside a [`MatchSim`] and are driven by exactly what a human sends, a [`PlayerInput`] per tick (ADR 0044).
//!
//! A bot is an ordinary player slot with a [`Brain`] attached. Each tick, before the inputs are consumed, [`MatchSim::run_bots`] asks
//! every brain what it would do and queues that input, so bots move with the same physics, shoot with the same weapons, take the same
//! damage and respawn by the same rules as anyone else, and the recorded trace of a match replays without a brain at all (the inputs are
//! in the trace). A brain is a pure function of the world it is shown and its own seeded random generator: deterministic.
//!
//! What a brain does, in order each tick:
//! 1. **Perceive**: which enemies can it see (a real ray, a field of view, footsteps within hearing distance), whom does it remember,
//!    whom will it fight (nearest, humans a little preferred, sticky).
//! 2. **Aim**: swing toward the target no faster than its skill allows, with a wobble that shrinks the longer it has had the target in
//!    sight and grows with the target's speed. The bot does not know about its own wobble, so it fires "when aligned" and misses honestly.
//! 3. **Fire**: only after its reaction time, only within the weapon's reach, bursts and rests for automatics, fresh pulls for
//!    semi-autos, the bat only in arm's reach.
//! 4. **Move**: fight at the distance its weapon and personality like (close in with a shotgun, hold off with a scout), strafe and jump,
//!    hunt the nearest enemy when none is in sight, and steer around walls and ledges by rolling the *real movement code* forward a few
//!    ticks for candidate headings; stuck detection breaks out of corners. Routes along a `nav` graph come from [`nav`].
//!
//! The pieces: [`skill`] (difficulty, personality, per-weapon profiles), [`nav`] (the waypoint graph a scene may carry), and this file (the
//! brain, [`BotSpec`]/[`BotsConfig`] and the `MatchSim` methods that add, name and run bots).

pub mod kart;
pub mod nav;
pub mod route;
pub mod skill;

use self::route::{Route, Steer};
use self::skill::{level_from_name, Skill, Style, WeaponProfile};
use super::interact::{RayHit, RayTarget};
use super::match_sim::{MatchSim, ServerPlayer, MAX_PLAYERS};
use super::player::{step_player_tuned, PlayerInput, PlayerState};
use crate::player::{Character, FIXED_DT};
use crate::strict::check_keys;
use crate::weapons::Weapon;
use glam::{Vec2, Vec3};
use serde_json::{Map, Value};
use std::f32::consts::{PI, TAU};

/// Everything that makes one bot itself.
#[derive(Debug, Clone, PartialEq)]
pub struct BotSpec {
    /// The name on the scoreboard.
    pub name: String,
    /// Its body: any human-rig character (never the rat, which cannot attack).
    pub character: Character,
    /// Skill level, `0.0` (rookie) to `1.0` (nightmare); see [`skill::PRESETS`].
    pub level: f32,
    /// How it likes to fight.
    pub style: Style,
    /// Pins this bot to team 1 or 2 (`bots.roster[].team`), on top of the match's own auto-balancing. `None` lets
    /// the match assign it a team when teams are in play (`MatchSim::teams_enabled`), or no team otherwise.
    pub team: Option<u8>,
}

/// The scene's `bots` block: how many fighters the match wants and who they are.
///
/// ```json
/// "bots": { "fill": 8, "skill": "normal", "roster": [ { "name": "Dusty", "character": "cowboy", "skill": "hard", "style": "rusher" } ] }
/// ```
///
/// `fill` is the number of players (humans included) the match aims for; empty slots are filled with bots, and a joining human takes a bot's
/// place. `roster` names the first bots; any beyond it are generated (a name from a pool, characters and styles in rotation).
#[derive(Debug, Clone, PartialEq)]
pub struct BotsConfig {
    /// Players the match aims for, `0` = no bots.
    pub fill: usize,
    /// Level for bots that do not name their own.
    pub level: f32,
    /// The named bots, in the order they are used.
    pub roster: Vec<BotSpec>,
}

impl Default for BotsConfig {
    fn default() -> Self {
        BotsConfig { fill: 0, level: 0.5, roster: Vec::new() }
    }
}

/// Keys of the `bots` block.
pub const BOTS_KEYS: &[&str] = &["fill", "skill", "roster"];
/// Keys of one `bots.roster` entry.
pub const BOT_KEYS: &[&str] = &["name", "character", "skill", "style", "team"];

const NAME_POOL: [&str; 24] = [
    "Dusty", "Merlot", "Zorp", "R0-B1", "Hex", "Pixel", "Bolt", "Nova", "Gizmo", "Sprocket", "Rook", "Maverick", "Ghost", "Viper", "Anvil", "Cobalt", "Falcon",
    "Granite", "Harbor", "Iron", "Jackal", "Kestrel", "Lynx", "Onyx",
];
const BOT_BODIES: [Character; 4] = [Character::Cowboy, Character::Wizard, Character::Alien, Character::Robot];
const STYLE_CYCLE: [Style; 4] = [Style::Balanced, Style::Rusher, Style::Sniper, Style::Acrobat];

impl BotsConfig {
    /// The `index`th bot: the roster entry if there is one, else a generated fighter.
    pub fn spec(&self, index: usize) -> BotSpec {
        if let Some(s) = self.roster.get(index) {
            return s.clone();
        }
        let n = index.saturating_sub(self.roster.len());
        BotSpec {
            name: NAME_POOL[index % NAME_POOL.len()].to_string(),
            character: BOT_BODIES[index % BOT_BODIES.len()],
            level: self.level,
            style: STYLE_CYCLE[(n + 1) % STYLE_CYCLE.len()],
            team: None,
        }
    }
}

fn level_of(v: &Value, path: &str, errs: &mut Vec<String>) -> Option<f32> {
    let level = match v {
        Value::String(s) => level_from_name(s),
        Value::Number(n) => n.as_f64().map(|f| f as f32).filter(|l| (0.0..=1.0).contains(l)),
        _ => None,
    };
    if level.is_none() {
        let names: Vec<&str> = skill::PRESETS.iter().map(|p| p.0).collect();
        errs.push(format!("{path}: must be a number from 0 to 1 or one of {}", names.join(", ")));
    }
    level
}

/// Parses a scene's optional `bots` block; every problem is `bots.path: message` with a did-you-mean.
pub fn parse_bots(root: &Map<String, Value>) -> Result<BotsConfig, Vec<String>> {
    let mut cfg = BotsConfig::default();
    let Some(value) = root.get("bots") else { return Ok(cfg) };
    let Some(o) = value.as_object() else {
        return Err(vec!["bots: must be an object like {\"fill\": 8, \"skill\": \"normal\"}".to_string()]);
    };
    let mut errs = Vec::new();
    check_keys(&mut errs, "bots", o, BOTS_KEYS);
    if let Some(v) = o.get("fill") {
        match v.as_u64().filter(|n| (0..=MAX_PLAYERS as u64).contains(n)) {
            Some(n) => cfg.fill = n as usize,
            None => errs.push(format!("bots.fill: must be a whole number from 0 to {MAX_PLAYERS} (the players the match aims for, humans included)")),
        }
    }
    if let Some(v) = o.get("skill") {
        if let Some(l) = level_of(v, "bots.skill", &mut errs) {
            cfg.level = l;
        }
    }
    if let Some(v) = o.get("roster") {
        let Some(list) = v.as_array() else {
            errs.push("bots.roster: must be a list of {name, character, skill, style}".to_string());
            return Err(errs);
        };
        if list.len() > MAX_PLAYERS {
            errs.push(format!("bots.roster: at most {MAX_PLAYERS} bots"));
        }
        for (i, item) in list.iter().enumerate() {
            let path = format!("bots.roster[{i}]");
            let Some(b) = item.as_object() else {
                errs.push(format!("{path}: must be an object like {{\"name\": \"Dusty\", \"character\": \"cowboy\"}}"));
                continue;
            };
            check_keys(&mut errs, &path, b, BOT_KEYS);
            let mut spec = BotsConfig { level: cfg.level, ..Default::default() }.spec(i);
            if let Some(n) = b.get("name") {
                match n.as_str().map(str::trim).filter(|s| !s.is_empty() && s.len() <= 16) {
                    Some(s) => spec.name = s.to_string(),
                    None => errs.push(format!("{path}.name: must be a name of 1 to 16 characters")),
                }
            }
            if let Some(c) = b.get("character") {
                match c.as_str().and_then(Character::parse).filter(|c| *c != Character::Rat) {
                    Some(c) => spec.character = c,
                    None => errs.push(format!("{path}.character: must be human, wizard, cowboy, alien or robot (a rat cannot attack)")),
                }
            }
            if let Some(l) = b.get("skill") {
                if let Some(l) = level_of(l, &format!("{path}.skill"), &mut errs) {
                    spec.level = l;
                }
            }
            if let Some(s) = b.get("style") {
                match s.as_str().and_then(Style::parse) {
                    Some(s) => spec.style = s,
                    None => errs.push(format!("{path}.style: must be balanced, rusher, sniper or acrobat")),
                }
            }
            if let Some(t) = b.get("team") {
                match t.as_u64().filter(|n| *n == 1 || *n == 2) {
                    Some(n) => spec.team = Some(n as u8),
                    None => errs.push(format!("{path}.team: must be 1 or 2")),
                }
            }
            cfg.roster.push(spec);
        }
    }
    if errs.is_empty() {
        Ok(cfg)
    } else {
        Err(errs)
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------
// small deterministic helpers
// ---------------------------------------------------------------------------------------------------------------------------------

/// SplitMix64: tiny, fast, and identical everywhere (no platform maths).
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }

    fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }

    /// Roughly bell-shaped in `[-1, 1]`.
    fn bell(&mut self) -> f32 {
        (self.f32() + self.f32() + self.f32() - 1.5) / 1.5
    }

    fn sign(&mut self) -> f32 {
        if self.f32() < 0.5 {
            -1.0
        } else {
            1.0
        }
    }
}

fn wrap_pi(a: f32) -> f32 {
    let mut a = a % TAU;
    if a > PI {
        a -= TAU;
    } else if a < -PI {
        a += TAU;
    }
    a
}

/// Yaw (radians, 0 = -Z, clockwise from above) of a horizontal direction.
fn yaw_of(d: Vec2) -> f32 {
    libm::atan2f(d.x, -d.y)
}

/// Horizontal unit direction of a yaw.
fn dir_of(yaw: f32) -> Vec2 {
    let (s, c) = libm::sincosf(yaw);
    Vec2::new(s, -c)
}

fn rotate(d: Vec2, angle: f32) -> Vec2 {
    let (s, c) = libm::sincosf(angle);
    Vec2::new(d.x * c - d.y * s, d.x * s + d.y * c)
}

fn look_vec(yaw: f32, pitch: f32) -> Vec3 {
    let (sy, cy) = libm::sincosf(yaw);
    let (sp, cp) = libm::sincosf(pitch);
    Vec3::new(sy * cp, sp, -cy * cp)
}

fn secs(s: f32) -> u64 {
    (s.max(0.0) * crate::sim::clock::TICK_RATE_HZ as f32) as u64
}

// ---------------------------------------------------------------------------------------------------------------------------------
// the brain
// ---------------------------------------------------------------------------------------------------------------------------------

/// Farthest a bot will try to see, metres.
const SIGHT_M: f32 = 100.0;
/// Half of the horizontal field of view, radians (about 105 degrees: wide, so flanking works but not from behind).
const FOV_HALF: f32 = 1.83;
/// Ticks between full looks around (staggered by slot).
const LOOK_EVERY: u64 = 4;
/// Ticks of movement rolled forward when choosing a heading.
const ROLLOUT_TICKS: u32 = 12;
/// Closer than this an enemy is simply seen, metres.
const POINT_BLANK_M: f32 = 1.3;
/// Players do not collide, so bots keep this much room between themselves and anyone else, metres.
const PERSONAL_SPACE_M: f32 = 1.6;

#[derive(Debug, Clone, Copy)]
struct Memory {
    pos: Vec3,
    tick: u64,
}

/// One bot's mind. Create with [`Brain::new`]; [`MatchSim::add_bot`] does that for you.
#[derive(Debug, Clone)]
pub struct Brain {
    spec: BotSpec,
    skill: Skill,
    rng: Rng,
    seq: u32,
    was_dead: bool,
    // perception
    target: Option<usize>,
    target_visible: bool,
    seen_since: u64,
    memory: [Option<Memory>; MAX_PLAYERS],
    last_hp: u32,
    // aim: the bot's intended aim (smoothed toward the target) and the wobble its shots actually carry
    aim_yaw: f32,
    aim_pitch: f32,
    bias: Vec2,
    bias_goal: Vec2,
    bias_next: u64,
    // trigger
    burst_ticks: u32,
    rest_until: u64,
    // movement
    strafe: f32,
    strafe_until: u64,
    hunt_goal: Option<Vec3>,
    hunt_until: u64,
    heading: Vec2,
    heading_until: u64,
    avoid_side: f32,
    jump_at: u64,
    // stuck detection
    probe_pos: Vec2,
    probe_tick: u64,
    stuck_until: u64,
    escape: Vec2,
    /// The last wish direction (diagnostics).
    last_want: Vec2,
    /// The path being followed along the scene's nav graph.
    route: Route,
    // loadout matches: choosing weapons, throwing grenades, going for pickups
    select_until: u64,
    nade_until: u64,
    pickup_goal: Option<Vec3>,
    pickup_until: u64,
}

impl Brain {
    /// A brain for `spec`; `seed` (the slot and any match seed) makes two bots with the same spec still behave differently.
    pub fn new(spec: BotSpec, seed: u64) -> Brain {
        Brain {
            skill: Skill::from_level(spec.level),
            rng: Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D).wrapping_add(0x1234_5678_9ABC_DEF1)),
            spec,
            seq: 0,
            was_dead: true,
            target: None,
            target_visible: false,
            seen_since: 0,
            memory: [None; MAX_PLAYERS],
            last_hp: 100,
            aim_yaw: 0.0,
            aim_pitch: 0.0,
            bias: Vec2::ZERO,
            bias_goal: Vec2::ZERO,
            bias_next: 0,
            burst_ticks: 0,
            rest_until: 0,
            strafe: 1.0,
            strafe_until: 0,
            hunt_goal: None,
            hunt_until: 0,
            heading: Vec2::ZERO,
            heading_until: 0,
            avoid_side: 1.0,
            jump_at: 0,
            probe_pos: Vec2::ZERO,
            probe_tick: 0,
            stuck_until: 0,
            escape: Vec2::ZERO,
            last_want: Vec2::ZERO,
            route: Route::default(),
            select_until: 0,
            nade_until: 0,
            pickup_goal: None,
            pickup_until: 0,
        }
    }

    /// The bot's spec (name, body, level, style).
    pub fn spec(&self) -> &BotSpec {
        &self.spec
    }

    /// One line of what the brain is doing (tracing and debugging): target, goal, heading, stuck state.
    pub fn debug_line(&self, now: u64) -> String {
        format!(
            "target {:?} visible {} hunt {:?} want ({:.2},{:.2}) heading ({:.2},{:.2}) strafe {:+.0} stuck_for {} aim_yaw {:.0} bias ({:.1},{:.1}) deg",
            self.target,
            self.target_visible,
            self.hunt_goal.map(|g| (g.x.round(), g.z.round())),
            self.last_want.x,
            self.last_want.y,
            self.heading.x,
            self.heading.y,
            self.strafe,
            self.stuck_until.saturating_sub(now),
            self.aim_yaw.to_degrees(),
            self.bias.x.to_degrees(),
            self.bias.y.to_degrees()
        )
    }

    /// Who it is fighting right now (a player slot), if anyone.
    pub fn target(&self) -> Option<usize> {
        self.target.filter(|_| self.target_visible)
    }

    fn respawned(&mut self, me: &ServerPlayer, now: u64) {
        self.target = None;
        self.target_visible = false;
        self.memory = [None; MAX_PLAYERS];
        self.aim_yaw = me.state.yaw;
        self.aim_pitch = 0.0;
        self.bias = Vec2::ZERO;
        self.bias_goal = Vec2::ZERO;
        self.burst_ticks = 0;
        self.hunt_goal = None;
        self.heading_until = 0;
        self.stuck_until = 0;
        self.probe_pos = me.state.pos;
        self.probe_tick = now;
        self.last_hp = me.combat.hp;
        self.strafe_until = 0;
        self.route.clear();
    }

    /// One tick of thinking: the input this bot sends for the world as `sim` shows it now.
    pub(crate) fn think(&mut self, sim: &MatchSim, slot: usize) -> PlayerInput {
        self.seq = self.seq.wrapping_add(1);
        let now = sim.tick();
        let Some(me) = sim.player(slot) else { return PlayerInput { seq: self.seq, ..Default::default() } };
        if me.combat.is_dead() {
            self.was_dead = true;
            return PlayerInput { seq: self.seq, yaw: self.aim_yaw, ..Default::default() };
        }
        if self.was_dead {
            self.was_dead = false;
            self.respawned(me, now);
        }
        let weapon = me.combat.weapon;
        let profile = WeaponProfile::of(weapon);
        let body = me.state.character.body();
        let eye = Vec3::new(me.state.pos.x, me.state.foot_y + if me.crouching { body.crouch_eye } else { body.stand_eye }, me.state.pos.y);
        let hurt = me.combat.hp < self.last_hp;
        self.last_hp = me.combat.hp;

        self.perceive(sim, slot, eye, now, hurt);
        let mut input = PlayerInput { seq: self.seq, sprint: true, ..Default::default() };
        let aligned = self.aim(sim, me, eye, now, &profile);
        self.fire(sim, me, now, &profile, aligned, &mut input);
        input.yaw = self.aim_yaw + self.bias.x;
        input.pitch = (self.aim_pitch + self.bias.y).clamp(-1.5, 1.5);
        self.movement(sim, slot, me, now, &profile, &mut input);
        self.kit_management(sim, me, now, &mut input);
        input
    }

    // ---- loadout matches: weapons, grenades, pickups ---------------------------------------------------------------------------

    /// Chooses the weapon to hold, aims down the sights at range, throws grenades at people it can see, and never shoots blind.
    fn kit_management(&mut self, sim: &MatchSim, me: &ServerPlayer, now: u64, input: &mut PlayerInput) {
        let Some(kit) = me.combat.kit.as_ref() else { return };
        if me.combat.flash_until > now {
            input.attack = false; // blinded: no shooting at nothing
            input.aim = false;
            return;
        }
        let target = self.target.filter(|_| self.target_visible).and_then(|t| sim.player(t));
        let dist = target.map(|p| (p.state.pos - me.state.pos).length());
        let current = kit.current();
        // Aim down the sights at range, except with weapons that do not gain from it.
        if let Some(d) = dist {
            input.aim = current.is_gun() && d > 13.0 && !matches!(current.class(), crate::arsenal::Class::Shotgun | crate::arsenal::Class::Launcher);
        }
        // Grenades: a lob at an enemy that is in sight and not too close.
        if kit.grenade_count() > 0 {
            if now >= self.nade_until && target.is_some() && dist.is_some_and(|d| (8.0..28.0).contains(&d)) && self.rng.chance(0.0012) {
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
            let p = WeaponProfile::of(g.weapon);
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
            input.reload = self.seq.is_multiple_of(2);
        }
    }

    /// The nearest weapon worth walking to: better than anything the bot holds, or fitting an empty slot.
    fn pickup_goal(&mut self, sim: &MatchSim, me: &ServerPlayer, now: u64) -> Option<Vec3> {
        let kit = me.combat.kit.as_ref()?;
        let arena = sim.arena()?;
        if now < self.pickup_until {
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
            let score = value * 10.0 - d * 0.35;
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

    // ---- perception -------------------------------------------------------------------------------------------------------------

    fn perceive(&mut self, sim: &MatchSim, slot: usize, eye: Vec3, now: u64, hurt: bool) {
        if !(now + slot as u64).is_multiple_of(LOOK_EVERY) && !hurt {
            return;
        }
        let look = look_vec(self.aim_yaw, self.aim_pitch);
        let memory_ticks = secs(self.skill.memory_secs);
        let mut best: Option<(usize, f32)> = None;
        let mut current_visible = false;
        let my_team = sim.team_of(slot);
        for (s, p) in sim.players() {
            if s == slot || p.combat.is_dead() || (my_team != 0 && p.team == my_team) {
                self.memory[s] = None;
                continue;
            }
            let body = p.state.character.body();
            let feet = Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y);
            let aim_point = feet + Vec3::Y * (body.body_height * 0.55);
            let to = aim_point - eye;
            let dist = to.length().max(0.01);
            let dir = to / dist;
            let in_view = look.dot(dir) > libm::cosf(FOV_HALF) || dist < self.skill.hearing_m;
            // Inside arm's reach nothing can be in the way (and a ray from inside another cylinder is meaningless).
            let visible = dist <= SIGHT_M
                && in_view
                && (dist < POINT_BLANK_M || matches!(sim.probe(eye, dir, dist + 0.6, slot), Some(RayHit { target: RayTarget::Player(t), .. }) if t == s));
            if visible {
                self.memory[s] = Some(Memory { pos: feet, tick: now });
                let human_bonus = if sim.is_bot(s) { 1.0 } else { 1.2 };
                let sticky = if self.target == Some(s) { 1.6 } else { 1.0 };
                let score = human_bonus * sticky / (dist + 4.0);
                if best.is_none_or(|(_, b)| score > b) {
                    best = Some((s, score));
                }
                if self.target == Some(s) {
                    current_visible = true;
                }
            } else if self.memory[s].is_some_and(|m| now.saturating_sub(m.tick) > memory_ticks) {
                self.memory[s] = None;
            }
        }
        match best {
            Some((s, _)) => {
                if self.target != Some(s) || !self.target_visible {
                    self.seen_since = now;
                }
                self.target = Some(s);
                self.target_visible = true;
            }
            None => self.target_visible = current_visible,
        }
        // Shot at by someone unseen: notice the nearest living enemy and turn to face where they are.
        if hurt && !self.target_visible {
            let nearest = sim
                .players()
                .filter(|(s, p)| *s != slot && !p.combat.is_dead() && (my_team == 0 || p.team != my_team))
                .map(|(s, p)| (s, Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y)))
                .min_by(|a, b| (a.1 - eye).length_squared().total_cmp(&(b.1 - eye).length_squared()));
            if let Some((s, pos)) = nearest {
                self.memory[s] = Some(Memory { pos, tick: now });
                self.hunt_goal = Some(pos);
                self.hunt_until = now + secs(4.0);
            }
        }
    }

    // ---- aim --------------------------------------------------------------------------------------------------------------------

    /// Swings the crosshair toward what the bot wants to look at; `true` when it is on the target (ready to shoot).
    fn aim(&mut self, sim: &MatchSim, me: &ServerPlayer, eye: Vec3, now: u64, profile: &WeaponProfile) -> bool {
        let max_step = self.skill.aim_turn_dps.to_radians() * FIXED_DT;
        let gain = self.skill.aim_gain;
        let target = self.target.filter(|_| self.target_visible).and_then(|t| sim.player(t).map(|p| (t, p)));
        let (want_yaw, want_pitch, on_target_tol) = match target {
            Some((_, p)) => {
                let body = p.state.character.body();
                let point = Vec3::new(p.state.pos.x, p.state.foot_y + body.body_height * 0.55, p.state.pos.y);
                let to = point - eye;
                let flat = Vec2::new(to.x, to.z).length().max(0.01);
                let tol = (libm::atanf(0.42 / to.length().max(0.5)) + 0.6f32.to_radians()).clamp(0.9f32.to_radians(), 7.0f32.to_radians());
                (yaw_of(Vec2::new(to.x, to.z)), libm::atan2f(to.y, flat), tol)
            }
            None => {
                // Look where it is going (or hunting); eyes level.
                let d = if self.heading.length_squared() > 0.01 { self.heading } else { dir_of(self.aim_yaw) };
                (yaw_of(d), 0.0, 0.0)
            }
        };
        let step = |current: f32, want: f32, wrap: bool| {
            let d = if wrap { wrap_pi(want - current) } else { want - current };
            current + (d * gain).clamp(-max_step, max_step)
        };
        self.aim_yaw = wrap_pi(step(self.aim_yaw, want_yaw, true));
        self.aim_pitch = step(self.aim_pitch, want_pitch, false).clamp(-1.4, 1.4);
        // The wobble.
        if let Some((_, p)) = target {
            if now >= self.bias_next {
                let seen_for = now.saturating_sub(self.seen_since) as f32 * FIXED_DT;
                let settled = self.skill.aim_floor + (1.0 - self.skill.aim_floor) * libm::expf(-seen_for / self.skill.aim_settle_secs);
                let across = {
                    let to = Vec2::new(p.state.pos.x - me.state.pos.x, p.state.pos.y - me.state.pos.y).normalize_or_zero();
                    let side = Vec2::new(-to.y, to.x);
                    (p.state.velocity.dot(side)).abs()
                };
                let moving = 1.0 + (across / 6.0).min(1.5);
                let sigma = (self.skill.aim_error_deg * profile.steadiness * settled * moving).to_radians();
                self.bias_goal = Vec2::new(self.rng.bell(), self.rng.bell()) * sigma;
                self.bias_next = now + 10 + (self.rng.f32() * 14.0) as u64;
            }
            self.bias += (self.bias_goal - self.bias) * 0.22;
        } else {
            self.bias *= 0.8;
        }
        target.is_some() && wrap_pi(want_yaw - self.aim_yaw).abs() < on_target_tol && (want_pitch - self.aim_pitch).abs() < on_target_tol * 1.5
    }

    // ---- trigger ----------------------------------------------------------------------------------------------------------------

    fn fire(&mut self, sim: &MatchSim, me: &ServerPlayer, now: u64, profile: &WeaponProfile, aligned: bool, input: &mut PlayerInput) {
        if me.combat.weapon.is_firearm() && me.combat.ammo.is_empty() {
            input.reload = self.seq.is_multiple_of(2);
        }
        let Some((_, p)) = self.target.filter(|_| self.target_visible).and_then(|t| sim.player(t).map(|p| (t, p))) else {
            self.burst_ticks = 0;
            return;
        };
        let dist = Vec2::new(p.state.pos.x - me.state.pos.x, p.state.pos.y - me.state.pos.y).length();
        let ready = now >= self.seen_since + secs(self.skill.reaction_secs);
        if !(ready && aligned && dist <= profile.reach) {
            self.burst_ticks = 0;
            return;
        }
        if profile.semi_auto {
            input.attack = self.seq.is_multiple_of(2);
        } else if self.burst_ticks > 0 {
            self.burst_ticks -= 1;
            input.attack = true;
            if self.burst_ticks == 0 {
                self.rest_until = now + secs(self.skill.burst_rest_secs * self.rng.range(0.6, 1.4));
            }
        } else if now >= self.rest_until {
            let cooldown = me.combat.weapon.firearm().map_or(6, |s| s.cooldown_ticks.max(1));
            let shots = self.rng.range(profile.burst.0 as f32, profile.burst.1 as f32 + 0.99) as u32;
            self.burst_ticks = shots.max(1) * cooldown;
            input.attack = true;
        }
    }

    // ---- movement ---------------------------------------------------------------------------------------------------------------

    fn movement(&mut self, sim: &MatchSim, slot: usize, me: &ServerPlayer, now: u64, profile: &WeaponProfile, input: &mut PlayerInput) {
        let pos = me.state.pos;
        let (_, ground) = sim.static_world();
        let floor = crate::collide::ground_height_at(ground, pos, me.state.foot_y);
        let grounded = me.state.vy <= 0.0 && (me.state.foot_y - floor).abs() < 0.05;
        // Stuck detection: asked to move, barely moved in a second.
        if now >= self.probe_tick + 45 {
            let moved = (pos - self.probe_pos).length();
            if moved < 0.5 && self.heading.length_squared() > 0.01 && now >= self.stuck_until && grounded {
                self.stuck_until = now + 40;
                self.escape = rotate(self.heading, self.rng.sign() * self.rng.range(1.2, 2.4));
                self.avoid_side = -self.avoid_side;
                self.jump_at = now;
                self.route.clear();
            }
            self.probe_pos = pos;
            self.probe_tick = now;
        }
        let target = self.target.filter(|_| self.target_visible).and_then(|t| sim.player(t).map(|p| (t, p)));
        let mut want = Vec2::ZERO;
        let mut in_fight = false;
        let mut steer: Option<Steer> = None;
        if now < self.stuck_until {
            want = self.escape;
        } else if let Some((_, p)) = target {
            in_fight = true;
            let to = p.state.pos - pos;
            let dist = to.length().max(0.01);
            let n = to / dist;
            let perp = Vec2::new(-n.y, n.x);
            if now >= self.strafe_until {
                self.strafe = if self.rng.chance(0.75) { -self.strafe } else { self.strafe };
                self.strafe_until = now + secs(self.rng.range(self.skill.strafe_secs.0, self.skill.strafe_secs.1));
            }
            let ideal = profile.ideal * self.spec.style.range_scale();
            let aggression = self.spec.style.aggression();
            let hp_low = me.combat.hp < 35;
            let radial = if profile.melee {
                if dist > 1.6 {
                    1.0
                } else if dist < 0.9 {
                    -0.6
                } else {
                    0.0
                }
            } else if hp_low && dist < ideal * 1.6 {
                -1.0
            } else if dist > ideal * 1.35 {
                0.4 + 0.6 * aggression
            } else if dist < ideal * 0.7 {
                -0.9
            } else {
                0.0
            };
            let lateral = if profile.melee && dist < 3.0 { 0.25 } else { 0.9 };
            want = n * radial + perp * self.strafe * lateral;
            // The enemy is up on another level, or a long way off: getting to them matters more than the perfect firing position, and the
            // way there is the route, not the straight line. Keep shooting while moving.
            let high = (p.state.foot_y - me.state.foot_y).abs() > 2.4;
            if high || dist > 34.0 {
                if let Some(nav) = sim.nav() {
                    let goal = Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y);
                    steer = self.route.steer(nav, &me.state, grounded, goal, now);
                    if let Some(s) = steer {
                        want = s.dir + perp * self.strafe * 0.2;
                    }
                }
            }
        } else if let Some(goal) = self.pickup_goal(sim, me, now).filter(|_| me.combat.kit.is_some()) {
            let to = Vec2::new(goal.x - pos.x, goal.z - pos.y);
            if let Some(nav) = sim.nav() {
                steer = self.route.steer(nav, &me.state, grounded, goal, now);
            }
            want = steer.map_or_else(|| to.normalize_or_zero(), |s| s.dir);
            if to.length() < 0.6 {
                self.pickup_until = 0;
            }
        } else if let Some(goal) = self.hunt_target(sim, slot, me, now) {
            let to = Vec2::new(goal.x - pos.x, goal.z - pos.y);
            if to.length() > 1.5 || (goal.y - me.state.foot_y).abs() > 1.5 {
                if let Some(nav) = sim.nav() {
                    steer = self.route.steer(nav, &me.state, grounded, goal, now);
                }
                want = steer.map_or_else(|| to.normalize_or_zero(), |s| s.dir);
            } else {
                self.hunt_goal = None;
            }
        }
        // Nobody stands inside anybody else (players pass through each other, so bots must make room themselves), but a committed
        // launch or fall is not nudged off its line.
        let committed = steer.is_some_and(|s| s.airborne_leg);
        let mut apart = Vec2::ZERO;
        for (s, p) in sim.players() {
            if s == slot || p.combat.is_dead() || (p.state.foot_y - me.state.foot_y).abs() > 1.5 {
                continue;
            }
            let away = pos - p.state.pos;
            let d = away.length();
            if d < PERSONAL_SPACE_M {
                let push = if d > 0.01 { away / d } else { dir_of(self.rng.f32() * TAU) };
                apart += push * (PERSONAL_SPACE_M - d) / PERSONAL_SPACE_M;
            }
        }
        if apart.length_squared() > 0.0 && !committed {
            want = if want.length_squared() > 0.001 { want.normalize() + apart * 1.5 } else { apart };
        }
        self.last_want = want;
        if want.length_squared() > 0.001 {
            let base = want.normalize();
            if steer.is_some() {
                // The graph has been proven with the real movement: follow it exactly rather than second-guessing with rollouts.
                self.heading = base;
                self.heading_until = now;
            } else if now >= self.heading_until {
                // Roll the real movement forward for a few headings and take the first that gets somewhere.
                self.heading = self.choose_heading(sim, me, base);
                self.heading_until = now + 5 + (self.rng.f32() * 3.0) as u64;
            } else {
                // Keep the previous decision but follow gradual changes of the wish.
                let blend = self.heading + (base - self.heading) * 0.35;
                if blend.length_squared() > 0.001 {
                    self.heading = blend.normalize();
                }
            }
            let d = self.heading;
            // Movement axes are relative to where the bot is looking (the yaw this tick's input carries).
            let fwd = dir_of(input.yaw);
            let right = Vec2::new(-fwd.y, fwd.x);
            input.forward = (d.dot(fwd) * 127.0).round().clamp(-127.0, 127.0) as i8;
            input.strafe = (d.dot(right) * 127.0).round().clamp(-127.0, 127.0) as i8;
            input.analog = true;
        } else {
            self.heading = Vec2::ZERO;
        }
        // Jumping: dodge in fights, hop over what blocks, jump where the route says; never while there is nothing to do.
        let airborne = me.state.vy.abs() > 0.5;
        if !airborne {
            let dodge = in_fight && steer.is_none() && self.rng.chance(self.skill.jump_per_sec * self.spec.style.jumpiness() * FIXED_DT);
            let unstick = now < self.stuck_until && now >= self.jump_at && now < self.jump_at + 3;
            input.jump = dodge || unstick || steer.is_some_and(|s| s.jump);
        }
        input.crouch = in_fight && self.spec.style == Style::Sniper && profile.semi_auto && !profile.melee && !airborne && steer.is_none();
        if input.crouch {
            input.sprint = false;
        }
    }

    /// Where a bot without a fight is heading: what it last saw, else a spot near the nearest living enemy (they hunt).
    fn hunt_target(&mut self, sim: &MatchSim, slot: usize, me: &ServerPlayer, now: u64) -> Option<Vec3> {
        let memory_ticks = secs(self.skill.memory_secs);
        let remembered = self.memory.iter().flatten().filter(|m| now.saturating_sub(m.tick) <= memory_ticks).max_by_key(|m| m.tick).map(|m| m.pos);
        if let Some(p) = remembered {
            return Some(p);
        }
        if self.hunt_goal.is_none() || now >= self.hunt_until {
            let here = Vec3::new(me.state.pos.x, me.state.foot_y, me.state.pos.y);
            let my_team = sim.team_of(slot);
            let mut enemies: Vec<(f32, Vec3)> = sim
                .players()
                .filter(|(s, p)| *s != slot && !p.combat.is_dead() && (my_team == 0 || p.team != my_team))
                .map(|(_, p)| {
                    let at = Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y);
                    ((at - here).length(), at)
                })
                .collect();
            enemies.sort_by(|a, b| a.0.total_cmp(&b.0));
            // Usually the nearest, sometimes the second nearest, so a room of bots does not all arrive from one direction.
            let pick = if enemies.len() > 1 && self.rng.chance(0.3) { 1 } else { 0 };
            self.hunt_goal = enemies.get(pick).map(|e| e.1 + Vec3::new(self.rng.bell() * 6.0, 0.0, self.rng.bell() * 6.0));
            self.hunt_until = now + secs(self.rng.range(2.5, 4.5));
        }
        self.hunt_goal
    }

    /// The first of a few headings around `base` along which the real movement code gets somewhere (or the best of them).
    fn choose_heading(&mut self, sim: &MatchSim, me: &ServerPlayer, base: Vec2) -> Vec2 {
        let nominal = me.state.character.body().sprint_speed.max(sim.player_tuning.sprint_speed) * ROLLOUT_TICKS as f32 * FIXED_DT * 0.55;
        let side = self.avoid_side;
        let mut best = (f32::NEG_INFINITY, base);
        for angle in [0.0, 0.6, -0.6, 1.3, -1.3, 2.2, -2.2] {
            let d = rotate(base, angle * side);
            let (moved, fell, launched) = self.rollout(sim, me, d);
            let bad = fell || launched;
            let score = moved - if bad { nominal * 2.0 } else { 0.0 } - angle.abs() * 0.15;
            if moved >= nominal && !bad {
                return d;
            }
            if score > best.0 {
                best = (score, d);
            }
        }
        best.1
    }

    /// Rolls the movement forward [`ROLLOUT_TICKS`] ticks heading `dir`: `(distance covered, ends well below where it started, gets launched into the air)`.
    /// A bot that is not following the route does not walk off ledges or onto jump pads by accident.
    fn rollout(&self, sim: &MatchSim, me: &ServerPlayer, dir: Vec2) -> (f32, bool, bool) {
        let (colliders, ground) = sim.static_world();
        let mut st: PlayerState = me.state;
        let yaw = yaw_of(dir);
        let input = PlayerInput { forward: 127, analog: true, sprint: true, yaw, ..Default::default() };
        let start = st.pos;
        let y0 = st.foot_y;
        for _ in 0..ROLLOUT_TICKS {
            step_player_tuned(&mut st, &input, colliders, ground, sim.player_tuning, &sim.jump_pads);
        }
        ((st.pos - start).length(), st.foot_y < y0 - 1.1, st.vy > 4.0 || st.foot_y > y0 + 1.6)
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

// ---------------------------------------------------------------------------------------------------------------------------------
// MatchSim: adding, naming and running bots
// ---------------------------------------------------------------------------------------------------------------------------------

impl MatchSim {
    /// Adds a bot in the highest free slot (humans fill from the bottom, so their ids stay small). `None` when the match is full.
    pub fn add_bot(&mut self, spec: &BotSpec) -> Option<usize> {
        let slot = (0..MAX_PLAYERS).rev().find(|s| self.player(*s).is_none())?;
        self.add_bot_in_slot(slot, spec).then_some(slot)
    }

    /// Adds a bot in exactly `slot`; `false` when the slot is taken, out of range, or the spec asks for a body that cannot fight.
    pub fn add_bot_in_slot(&mut self, slot: usize, spec: &BotSpec) -> bool {
        self.add_bot_in_slot_team(slot, spec, 0)
    }

    /// [`add_bot_in_slot`](Self::add_bot_in_slot) for a bot on `team` (`0` = no team): it wears that team's uniform and spawns at its base.
    pub fn add_bot_in_slot_team(&mut self, slot: usize, spec: &BotSpec, team: u8) -> bool {
        if team != 0 {
            let mut spec = spec.clone();
            // The team uniform only replaces a bot's requested look in a real loadout match; a non-shooter teamed
            // scene (hide-and-seek roles, etc.) keeps whatever character the roster asked for.
            if self.is_loadout() {
                spec.character = if team == 1 { Character::Ridgeback } else { Character::Nightfall };
            } else if spec.character == Character::Rat {
                // A fighter cannot be the rat (it cannot attack), the same rule the no-team path below applies.
                return false;
            }
            if !self.add_player_in_slot_team(slot, spec.character, team) {
                return false;
            }
            self.bots[slot] = Some(Box::new(Brain::new(spec, 0xB07 ^ ((slot as u64 + 1) << 8) ^ (self.tick << 16))));
            return true;
        }
        if self.race.is_some() {
            // A race: the bot is a kart driver, on the first animal no one else has taken.
            if !self.add_player_in_slot(slot, Character::Human) {
                return false;
            }
            self.drivers[slot] = self.free_driver(slot);
            let seed = 0xB07 ^ ((slot as u64 + 1) << 8) ^ (self.tick << 16);
            self.kart_bots[slot] = Some(Box::new(kart::KartBrain::new(spec.name.clone(), spec.level, seed)));
            return true;
        }
        if spec.character == Character::Rat || !self.add_player_in_slot(slot, spec.character) {
            return false;
        }
        self.bots[slot] = Some(Box::new(Brain::new(spec.clone(), 0xB07 ^ ((slot as u64 + 1) << 8) ^ (self.tick << 16))));
        true
    }

    /// Whether `slot` is driven by a brain.
    pub fn is_bot(&self, slot: usize) -> bool {
        self.bots.get(slot).is_some_and(Option::is_some) || self.kart_bots.get(slot).is_some_and(Option::is_some)
    }

    /// A bot's name.
    pub fn bot_name(&self, slot: usize) -> Option<&str> {
        if let Some(Some(k)) = self.kart_bots.get(slot) {
            return Some(k.name());
        }
        self.bots.get(slot)?.as_ref().map(|b| b.spec.name.as_str())
    }

    /// Number of bots in the match.
    pub fn bot_count(&self) -> usize {
        self.bots.iter().flatten().count() + self.kart_bots.iter().flatten().count()
    }

    /// The brain of `slot`, for inspection (its target, its spec).
    pub fn brain(&self, slot: usize) -> Option<&Brain> {
        self.bots.get(slot)?.as_deref()
    }

    /// Asks every brain for this tick's input and queues it (recorded like any other input).
    pub(super) fn run_bots(&mut self) {
        for slot in 0..self.kart_bots.len() {
            let Some(mut brain) = self.kart_bots[slot].take() else { continue };
            let input = brain.think(self, slot);
            self.kart_bots[slot] = Some(brain);
            self.push_input(slot, input);
        }
        for slot in 0..self.bots.len() {
            let Some(mut brain) = self.bots[slot].take() else { continue };
            let input = brain.think(self, slot);
            self.bots[slot] = Some(brain);
            self.push_input(slot, input);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(s: &str) -> Map<String, Value> {
        serde_json::from_str::<Value>(s).unwrap().as_object().unwrap().clone()
    }

    #[test]
    fn no_block_means_no_bots() {
        assert_eq!(parse_bots(&root("{}")).unwrap(), BotsConfig::default());
    }

    #[test]
    fn a_roster_names_the_first_bots_and_the_rest_are_generated() {
        let cfg = parse_bots(&root(
            r#"{"bots":{"fill":8,"skill":"hard","roster":[{"name":"Dusty","character":"cowboy","style":"rusher"},{"name":"Zed","skill":0.2,"character":"alien"}]}}"#,
        ))
        .unwrap();
        assert_eq!(cfg.fill, 8);
        assert_eq!((cfg.spec(0).name.as_str(), cfg.spec(0).character, cfg.spec(0).style, cfg.spec(0).level), ("Dusty", Character::Cowboy, Style::Rusher, 0.72));
        assert_eq!((cfg.spec(1).character, cfg.spec(1).level), (Character::Alien, 0.2));
        let g = cfg.spec(5);
        assert!(!g.name.is_empty() && g.level == 0.72 && g.character != Character::Rat, "{g:?}");
        assert_ne!(cfg.spec(4).name, cfg.spec(5).name, "generated names differ");
    }

    #[test]
    fn mistakes_name_the_field_and_the_fix() {
        let e = parse_bots(&root(r#"{"bots":{"fil":8,"skill":"godlike","roster":[{"nam":"x"},{"character":"rat"},{"style":"camper","skill":2}]}}"#))
            .unwrap_err()
            .join("\n");
        assert!(e.contains("bots.fil: unknown field — did you mean `fill`"), "{e}");
        assert!(e.contains("bots.skill: must be a number from 0 to 1 or one of rookie"), "{e}");
        assert!(e.contains("bots.roster[0].nam: unknown field"), "{e}");
        assert!(e.contains("a rat cannot attack"), "{e}");
        assert!(e.contains("bots.roster[2].style: must be balanced") && e.contains("bots.roster[2].skill"), "{e}");
        assert!(parse_bots(&root(r#"{"bots":{"fill":99}}"#)).is_err());
        assert!(parse_bots(&root(r#"{"bots":[]}"#)).is_err());
    }

    #[test]
    fn the_rng_is_uniform_enough_and_repeatable() {
        let mut a = Rng(7);
        let mut b = Rng(7);
        let xs: Vec<f32> = (0..2000).map(|_| a.f32()).collect();
        assert!(xs.iter().zip((0..2000).map(|_| b.f32())).all(|(x, y)| *x == y), "same seed, same stream");
        assert!(xs.iter().all(|x| (0.0..1.0).contains(x)));
        let mean = xs.iter().sum::<f32>() / xs.len() as f32;
        assert!((mean - 0.5).abs() < 0.03, "{mean}");
        let bells: Vec<f32> = (0..2000).map(|_| a.bell()).collect();
        assert!(bells.iter().all(|x| (-1.0..=1.0).contains(x)) && bells.iter().sum::<f32>().abs() / 2000.0 < 0.05);
    }

    #[test]
    fn angle_helpers_agree_with_the_engines_yaw_convention() {
        for deg in [0.0f32, 45.0, 90.0, 180.0, 270.0, 359.0] {
            let yaw = deg.to_radians();
            let d = dir_of(yaw);
            assert!(wrap_pi(yaw_of(d) - yaw).abs() < 1e-4, "{deg}");
        }
        assert!((dir_of(0.0) - Vec2::new(0.0, -1.0)).length() < 1e-5, "yaw 0 looks along -Z");
        assert!((dir_of(PI / 2.0) - Vec2::new(1.0, 0.0)).length() < 1e-5, "yaw 90 looks along +X");
        assert!((wrap_pi(3.0 * PI) - PI).abs() < 1e-4 || (wrap_pi(3.0 * PI) + PI).abs() < 1e-4);
        assert!((rotate(Vec2::X, PI / 2.0) - Vec2::Y).length() < 1e-5);
    }
}
