//! Kart movement for one tick, as a pure function: the driving model of Great Outdoors (ADR 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature).
//!
//! A kart is a movement *mode* of the existing player, not a second simulation. [`step_kart`] takes the same [`PlayerState`] and [`PlayerInput`] as
//! `sim::player::step_player_tuned` and is meant to be called from the same three places (the authoritative `MatchSim`, the client's prediction, the bots'
//! rollouts), so a kart drives identically online and offline. Input needs no new fields: `forward` is throttle (+) and brake or reverse (-), `strafe` is
//! steering (+ right), `jump` is hop and drift, and the caller handles `attack` (use the pickup) and `interact` (the driver's signature ability).
//!
//! Heading is `PlayerState.yaw` (same convention as the first-person camera: forward = `(sin yaw, -cos yaw)`); `velocity` is the real motion, so a drift is
//! the velocity pointing away from the heading. What the tick needs to remember beyond `PlayerState` is a small [`KartState`]. Each of the eight [`Driver`]s
//! is a row of numbers ([`KartSpec`], like the firearm rows), so tuning a kart is editing data.

use crate::collide::{ground_height_at, Collider2D, GroundCandidates};
use crate::player::{step_horizontal_band, FIXED_DT};
use crate::sim::player::{PlayerInput, PlayerState};
use glam::Vec2;

/// Radius of a kart's collision circle, m (a person's is 0.35).
pub const KART_RADIUS: f32 = 0.6;
/// Height of a kart's collision band above the ground, m: low walls and kerbs above this do not stop it.
pub const KART_BAND_TOP: f32 = 1.0;
/// Gravity while airborne, m/s^2 (higher than a walker's: karts hop, they do not float).
pub const KART_GRAVITY: f32 = 22.0;
/// Below this forward speed the wheels do not steer, and a drift cannot start, m/s.
pub const DRIFT_MIN_SPEED: f32 = 8.0;
/// Speed at which steering reaches its full rate, m/s.
const STEER_REF_SPEED: f32 = 8.0;
/// How quickly sideways sliding is killed on the ground, per second, times the kart's grip.
const GRIP_RATE: f32 = 9.0;
/// Rolling deceleration with no throttle, m/s^2.
const COAST_DECEL: f32 = 5.0;
/// Deceleration when above the current top speed (a boost ended, or the kart left the road), m/s^2.
const OVER_TOP_DECEL: f32 = 8.0;
/// Acceleration multiplier while boosting.
const BOOST_ACCEL_MULT: f32 = 2.2;
/// Turning rate multiplier while drifting, and the share of it the driver can steer in or out of the turn.
const DRIFT_TURN_MULT: f32 = 1.35;
/// Grip multiplier while drifting: the kart slides.
const DRIFT_GRIP_MULT: f32 = 0.3;
/// Drift charge (seconds of good drifting) needed for the three boost tiers, and the boost each earns, ticks.
const DRIFT_TIERS: [(f32, u16); 3] = [(1.0, 30), (2.0, 60), (3.2, 90)];
/// Spin-out rotation, radians per second.
const SPIN_RATE: f32 = 12.0;
/// Fraction of speed kept each tick the kart scrapes a wall.
const WALL_SCRAPE: f32 = 0.9;
/// Steering multiplier in the air (a glider keeps most of it).
const AIR_STEER: f32 = 0.4;
/// Gravity multiplier while gliding.
const GLIDE_GRAVITY: f32 = 0.35;
/// How long a Mushroom boosts, ticks (1.5 s).
pub const MUSHROOM_BOOST_TICKS: u16 = 90;
/// How long a Bubble lasts if nothing hits it, ticks (5 s).
pub const BUBBLE_TICKS: u16 = 300;
/// The Beaver's Build cooldown, ticks (4 s).
pub const BUILD_COOLDOWN_TICKS: u16 = 240;

/// A pickup a kart can hold (one at a time). `attack` uses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Item {
    /// Nothing held.
    #[default]
    None,
    /// A speed burst for the holder.
    Mushroom,
    /// A nut thrown ahead that spins out the first kart it hits.
    Acorn,
    /// A shield that absorbs the next hit.
    Bubble,
}

impl Item {
    /// The item's number on the wire (`0` = nothing).
    pub fn wire(self) -> u8 {
        match self {
            Item::None => 0,
            Item::Mushroom => 1,
            Item::Acorn => 2,
            Item::Bubble => 3,
        }
    }

    /// The item for a wire number, if it is one.
    pub fn from_wire(n: u8) -> Option<Item> {
        match n {
            0 => Some(Item::None),
            1 => Some(Item::Mushroom),
            2 => Some(Item::Acorn),
            3 => Some(Item::Bubble),
            _ => None,
        }
    }

    /// The item's name as shown to players.
    pub fn name(self) -> &'static str {
        match self {
            Item::None => "none",
            Item::Mushroom => "Mushroom",
            Item::Acorn => "Acorn",
            Item::Bubble => "Bubble",
        }
    }
}

/// What a kart step asks the simulation to do in the world (the kart step itself only changes the kart): the pool of hazards is the simulation's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KartEvents {
    /// Throw an Acorn ahead of the kart.
    pub throw_acorn: bool,
    /// Lay a plank behind the kart (the Beaver's Build).
    pub lay_plank: bool,
}

/// What a surface under the kart does to it. The caller looks this up from the map (ground type, zone or water).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Surface {
    /// The track.
    #[default]
    Road,
    /// Grass and dirt beside the track.
    Dirt,
    /// Mud.
    Mud,
    /// Shallow water.
    Water,
}

/// A driver's signature: what makes their kart different beyond the numbers. Most are encoded in the [`KartSpec`] fields; this names them for tools,
/// the HUD and the systems that need the rest (bumps, drafting, building).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ability {
    /// Duck: water does not slow the kart.
    Float,
    /// Bunny: a higher hop, and hop-drifts charge faster.
    Hop,
    /// Deer: drifts charge much faster.
    DriftBoost,
    /// Coyote: no penalty on dirt, so the off-road shortcuts are his.
    Dirt,
    /// Hawk: holding jump in the air glides with lift and keeps steering.
    Glide,
    /// Bear: bumps spin other karts out, and he is not spun himself.
    Bulldoze,
    /// Wolf: driving in a leader's slipstream charges a boost ([`slipstream_gain`]).
    Slipstream,
    /// Beaver: a wooden kart, immune to mud and water, that lays planks behind it (built by the pickup and hazard system).
    Build,
}

/// The numbers of one kart. Speeds are m/s, accelerations m/s^2, `steer_rate` degrees per second at full lock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KartSpec {
    /// Top speed on the road, m/s.
    pub top_speed: f32,
    /// Acceleration with full throttle, m/s^2.
    pub accel: f32,
    /// Braking deceleration, m/s^2.
    pub brake: f32,
    /// Top reverse speed, m/s.
    pub reverse_speed: f32,
    /// Steering rate at full lock and full speed, degrees per second.
    pub steer_rate: f32,
    /// 0..1: how hard the tyres grip. Low grip slides more.
    pub grip: f32,
    /// Relative mass: heavier karts lose less speed in a bump and push lighter ones aside.
    pub mass: f32,
    /// Top-speed multiplier on dirt.
    pub dirt: f32,
    /// Top-speed multiplier in mud.
    pub mud: f32,
    /// Top-speed multiplier in water.
    pub water: f32,
    /// Top-speed multiplier while boosting.
    pub boost_mult: f32,
    /// How fast a drift charges (1.0 = normal).
    pub drift_rate: f32,
    /// Upward speed of a hop, m/s.
    pub hop_speed: f32,
    /// The driver's signature.
    pub ability: Ability,
}

impl Surface {
    /// The surface a scene names (`dirt`, `mud`, `water`, `road`; any case).
    pub fn parse(name: &str) -> Option<Surface> {
        match name.trim().to_ascii_lowercase().as_str() {
            "road" => Some(Surface::Road),
            "dirt" => Some(Surface::Dirt),
            "mud" => Some(Surface::Mud),
            "water" => Some(Surface::Water),
            _ => None,
        }
    }

    /// The word a scene uses for it.
    pub fn name(self) -> &'static str {
        match self {
            Surface::Road => "road",
            Surface::Dirt => "dirt",
            Surface::Mud => "mud",
            Surface::Water => "water",
        }
    }
}

impl KartSpec {
    /// The top-speed multiplier of `surface` for this kart.
    pub fn surface_mult(&self, surface: Surface) -> f32 {
        match surface {
            Surface::Road => 1.0,
            Surface::Dirt => self.dirt,
            Surface::Mud => self.mud,
            Surface::Water => self.water,
        }
    }
}

/// One of the eight drivers of Great Outdoors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Driver {
    /// Balanced; floats over water.
    Duck,
    /// Quick off the line, hops.
    Bunny,
    /// Highest top speed, drifts well.
    Deer,
    /// Fine on dirt.
    Coyote,
    /// Light, glides off jumps.
    Hawk,
    /// Heavy; bumps others aside.
    Bear,
    /// Draws a boost from slipstreams.
    Wolf,
    /// Wooden kart; builds.
    Beaver,
}

impl Driver {
    /// All eight, in wire order.
    pub const ALL: [Driver; 8] = [Driver::Duck, Driver::Bunny, Driver::Deer, Driver::Coyote, Driver::Hawk, Driver::Bear, Driver::Wolf, Driver::Beaver];

    /// The driver's name as shown to players.
    pub fn name(self) -> &'static str {
        match self {
            Driver::Duck => "Duck",
            Driver::Bunny => "Bunny",
            Driver::Deer => "Deer",
            Driver::Coyote => "Coyote",
            Driver::Hawk => "Hawk",
            Driver::Bear => "Bear",
            Driver::Wolf => "Wolf",
            Driver::Beaver => "Beaver",
        }
    }

    /// The driver's number on the wire (its index in [`Driver::ALL`]).
    pub fn wire(self) -> u8 {
        Driver::ALL.iter().position(|d| *d == self).unwrap_or(0) as u8
    }

    /// The driver for a wire number, if it is one.
    pub fn from_wire(n: u8) -> Option<Driver> {
        Driver::ALL.get(n as usize).copied()
    }

    /// The driver whose [`Driver::name`] is `name` (any case).
    pub fn parse(name: &str) -> Option<Driver> {
        Driver::ALL.into_iter().find(|d| d.name().eq_ignore_ascii_case(name.trim()))
    }

    /// This driver's kart numbers: the starting point in the ADR's table, tuned by playing.
    pub fn spec(self) -> KartSpec {
        let base = KartSpec {
            top_speed: 24.0,
            accel: 12.0,
            brake: 26.0,
            reverse_speed: 6.0,
            steer_rate: 90.0,
            grip: 0.8,
            mass: 1.0,
            dirt: 0.85,
            mud: 0.6,
            water: 0.55,
            boost_mult: 1.3,
            drift_rate: 1.0,
            hop_speed: 4.0,
            ability: Ability::Float,
        };
        match self {
            Driver::Duck => KartSpec { water: 1.0, ability: Ability::Float, ..base },
            Driver::Bunny => KartSpec {
                top_speed: 22.0,
                accel: 16.0,
                steer_rate: 110.0,
                grip: 0.75,
                mass: 0.8,
                drift_rate: 1.2,
                hop_speed: 5.5,
                ability: Ability::Hop,
                ..base
            },
            Driver::Deer => KartSpec { top_speed: 27.0, accel: 10.0, steer_rate: 85.0, mass: 0.9, drift_rate: 1.3, ability: Ability::DriftBoost, ..base },
            Driver::Coyote => KartSpec { top_speed: 25.0, steer_rate: 90.0, grip: 0.7, mass: 0.9, dirt: 1.0, ability: Ability::Dirt, ..base },
            Driver::Hawk => KartSpec { top_speed: 25.0, accel: 11.0, mass: 0.7, ability: Ability::Glide, ..base },
            Driver::Bear => KartSpec {
                top_speed: 22.0,
                accel: 9.0,
                steer_rate: 70.0,
                grip: 0.85,
                mass: 1.6,
                drift_rate: 0.8,
                hop_speed: 3.0,
                ability: Ability::Bulldoze,
                ..base
            },
            Driver::Wolf => KartSpec { top_speed: 25.0, steer_rate: 92.0, grip: 0.78, ability: Ability::Slipstream, ..base },
            Driver::Beaver => KartSpec {
                top_speed: 21.0,
                accel: 9.0,
                steer_rate: 78.0,
                grip: 0.85,
                mass: 1.3,
                dirt: 1.0,
                mud: 1.0,
                water: 1.0,
                drift_rate: 0.9,
                ability: Ability::Build,
                ..base
            },
        }
    }
}

/// What a kart remembers from tick to tick beyond its [`PlayerState`]. Everything here is replicated and predicted.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct KartState {
    /// Ticks of boost left.
    pub boost_ticks: u16,
    /// `-1` drifting left, `1` drifting right, `0` not drifting.
    pub drift_dir: i8,
    /// Seconds of good drifting banked, spent as a boost when the drift ends.
    pub drift_charge: f32,
    /// Ticks left spinning out: the driver has no control.
    pub spin_ticks: u16,
    /// Was `jump` down last tick (a hop happens on the press).
    pub jump_held: bool,
    /// Slipstream banked, 0..1 (Wolf): a full bar becomes a boost.
    pub slip_charge: f32,
    /// The pickup held.
    pub item: Item,
    /// Ticks of Bubble shield left.
    pub shield_ticks: u16,
    /// Ticks until the driver's ability can be used again (the Beaver's Build).
    pub ability_cooldown: u16,
    /// Was `attack` down last tick (an item is used on the press).
    pub attack_held: bool,
    /// Was `interact` down last tick (the driver's ability is used on the press).
    pub interact_held: bool,
}

impl KartState {
    /// Starts a spin-out of `ticks` (a hit): the driver loses control, any drift is lost, the boost is not. A Bubble absorbs the hit instead (and
    /// pops). Returns whether the kart actually spun out.
    pub fn spin_out(&mut self, ticks: u16) -> bool {
        if self.shield_ticks > 0 {
            self.shield_ticks = 0;
            return false;
        }
        self.spin_ticks = self.spin_ticks.max(ticks);
        self.drift_dir = 0;
        self.drift_charge = 0.0;
        true
    }

    /// Adds `ticks` of boost (a pickup, a boost pad, a drift), keeping the longer of the two.
    pub fn boost(&mut self, ticks: u16) {
        self.boost_ticks = self.boost_ticks.max(ticks);
    }

    /// Banks slipstream for a Wolf: `gain` is [`slipstream_gain`] over `dt`; a full bar becomes 1 s of boost. Other drivers ignore it.
    pub fn draft(&mut self, spec: &KartSpec, gain: f32, dt: f32) {
        if spec.ability != Ability::Slipstream {
            return;
        }
        self.slip_charge = if gain > 0.0 { self.slip_charge + gain * dt } else { (self.slip_charge - 0.5 * dt).max(0.0) };
        if self.slip_charge >= 1.0 {
            self.slip_charge = 0.0;
            self.boost(60);
        }
    }
}

/// Advances one kart by one fixed tick (`FIXED_DT`): steering, throttle and braking, grip and drifting, hop and glide, boost, spin-out, walls.
/// `surface` is what is under the kart (the caller looks it up). Returns the speed after the tick, m/s (drives wheel and engine animation).
/// Items and abilities are handled by [`step_kart_ex`]; this is the same step with their world effects dropped.
pub fn step_kart(
    state: &mut PlayerState,
    kart: &mut KartState,
    input: &PlayerInput,
    spec: &KartSpec,
    surface: Surface,
    colliders: &[Collider2D],
    ground: &GroundCandidates,
) -> f32 {
    step_kart_ex(state, kart, input, spec, surface, colliders, ground).0
}

/// [`step_kart`] plus the pickups: on the press of `attack` the held item is used (a Mushroom boosts and a Bubble shields the kart itself, right here, so
/// a client predicts them exactly; an Acorn is returned as an event for the simulation to throw), and on the press of `interact` the Beaver's Build
/// asks for a plank behind the kart (once its cooldown has run). Nothing is used while spinning out. Returns the speed and those world events.
pub fn step_kart_ex(
    state: &mut PlayerState,
    kart: &mut KartState,
    input: &PlayerInput,
    spec: &KartSpec,
    surface: Surface,
    colliders: &[Collider2D],
    ground: &GroundCandidates,
) -> (f32, KartEvents) {
    let input = input.sanitized();
    let mut events = KartEvents::default();
    let attack_pressed = input.attack && !kart.attack_held;
    let interact_pressed = input.interact && !kart.interact_held;
    kart.attack_held = input.attack;
    kart.interact_held = input.interact;
    if kart.spin_ticks == 0 {
        if attack_pressed {
            match std::mem::take(&mut kart.item) {
                Item::Mushroom => kart.boost(MUSHROOM_BOOST_TICKS),
                Item::Bubble => kart.shield_ticks = BUBBLE_TICKS,
                Item::Acorn => events.throw_acorn = true,
                Item::None => {}
            }
        }
        if interact_pressed && spec.ability == Ability::Build && kart.ability_cooldown == 0 {
            events.lay_plank = true;
            kart.ability_cooldown = BUILD_COOLDOWN_TICKS;
        }
    }
    kart.shield_ticks = kart.shield_ticks.saturating_sub(1);
    kart.ability_cooldown = kart.ability_cooldown.saturating_sub(1);
    let scale = if input.analog { 127.0 } else { 1.0 };
    let mut throttle = input.forward as f32 / scale;
    let mut steer = input.strafe as f32 / scale;
    let jump_pressed = input.jump && !kart.jump_held;
    kart.jump_held = input.jump;

    // A spun-out kart ignores its driver and turns on the spot while it slows.
    let spinning = kart.spin_ticks > 0;
    if spinning {
        kart.spin_ticks -= 1;
        throttle = 0.0;
        steer = 0.0;
        state.yaw += SPIN_RATE * FIXED_DT;
    }

    let floor = ground_height_at(ground, state.pos, state.foot_y);
    let grounded = state.vy <= 0.0 && state.foot_y - floor < 0.05;

    // Hop on the press; a hop while steering at speed starts a drift.
    let (sy, cy) = libm::sincosf(state.yaw);
    let mut fwd = Vec2::new(sy, -cy);
    let mut speed_f = state.velocity.dot(fwd);
    if grounded && jump_pressed && !spinning {
        state.vy = spec.hop_speed;
        if speed_f > DRIFT_MIN_SPEED && steer.abs() > 0.2 {
            kart.drift_dir = if steer > 0.0 { 1 } else { -1 };
            kart.drift_charge = 0.0;
        }
    }
    // A drift ends when the button is released (paying out its boost) or the kart is too slow to keep it.
    if kart.drift_dir != 0 && (!input.jump || speed_f < DRIFT_MIN_SPEED * 0.5) {
        if !input.jump && speed_f >= DRIFT_MIN_SPEED * 0.5 {
            for (need, ticks) in DRIFT_TIERS.iter().rev() {
                if kart.drift_charge >= *need {
                    kart.boost(*ticks);
                    break;
                }
            }
        }
        kart.drift_dir = 0;
        kart.drift_charge = 0.0;
    }
    let drifting = kart.drift_dir != 0;

    let boosting = kart.boost_ticks > 0;
    if boosting {
        kart.boost_ticks -= 1;
    }
    let top = spec.top_speed * spec.surface_mult(surface) * if boosting { spec.boost_mult } else { 1.0 };
    let accel = spec.accel * if boosting { BOOST_ACCEL_MULT } else { 1.0 };

    // Throttle, brake, reverse, coast. Only the tyres on the ground can push.
    if grounded {
        if throttle > 0.0 {
            if speed_f < top {
                speed_f = (speed_f + accel * throttle * FIXED_DT).min(top);
            } else {
                speed_f = (speed_f - OVER_TOP_DECEL * FIXED_DT).max(top);
            }
        } else if throttle < 0.0 {
            if speed_f > 0.5 {
                speed_f = (speed_f + spec.brake * throttle * FIXED_DT).max(0.0);
            } else {
                speed_f = (speed_f + accel * 0.6 * throttle * FIXED_DT).max(-spec.reverse_speed);
            }
        } else if speed_f > 0.0 {
            speed_f = (speed_f - COAST_DECEL * FIXED_DT).max(0.0);
        } else {
            speed_f = (speed_f + COAST_DECEL * FIXED_DT).min(0.0);
        }
        if speed_f > top {
            speed_f = (speed_f - OVER_TOP_DECEL * FIXED_DT).max(top);
        }
    }

    // Steering: none while stopped, full from a walking pace, less in the air; a drift locks the kart into its turn and lets the driver tighten or open it.
    let mut turn = steer;
    if drifting {
        let d = kart.drift_dir as f32;
        turn = d * (0.6 + 0.4 * (steer * d).clamp(-1.0, 1.0)) * DRIFT_TURN_MULT;
        if grounded {
            kart.drift_charge += spec.drift_rate * FIXED_DT * (0.5 + 0.5 * steer.abs());
        }
    }
    let air = if grounded {
        1.0
    } else if spec.ability == Ability::Glide && input.jump {
        0.8
    } else {
        AIR_STEER
    };
    let speed_factor = (speed_f.abs() / STEER_REF_SPEED).min(1.0);
    let direction = if speed_f >= 0.0 { 1.0 } else { -1.0 };
    state.yaw += turn * spec.steer_rate.to_radians() * speed_factor * air * direction * FIXED_DT;

    // The kart now points somewhere new: keep its velocity, split it along the new heading, and let the tyres pull the sideways part away.
    let (sy, cy) = libm::sincosf(state.yaw);
    fwd = Vec2::new(sy, -cy);
    let lateral_now = state.velocity - fwd * state.velocity.dot(fwd);
    let mut lateral = lateral_now;
    if grounded {
        let grip = spec.grip * GRIP_RATE * if drifting { DRIFT_GRIP_MULT } else { 1.0 };
        lateral *= libm::expf(-grip * FIXED_DT);
    }
    state.velocity = fwd * speed_f + lateral;

    // Move, sliding along walls; a scrape costs speed.
    let intended = state.velocity * FIXED_DT;
    if intended.length_squared() > 0.0 {
        let before = state.pos;
        state.pos = step_horizontal_band(colliders, state.pos, state.foot_y, intended, KART_RADIUS, KART_BAND_TOP);
        let actual = state.pos - before;
        let mut blocked = false;
        for axis in 0..2 {
            if (actual[axis] - intended[axis]).abs() > 0.001 {
                state.velocity[axis] = actual[axis] / FIXED_DT;
                blocked = true;
            }
        }
        if blocked {
            state.velocity *= WALL_SCRAPE;
        }
    }

    // Vertical: gravity (less while gliding), landing on the highest surface under the kart.
    let gliding = !grounded && spec.ability == Ability::Glide && input.jump && state.vy <= 0.0;
    state.vy -= KART_GRAVITY * if gliding { GLIDE_GRAVITY } else { 1.0 } * FIXED_DT;
    state.foot_y += state.vy * FIXED_DT;
    let floor = ground_height_at(ground, state.pos, state.foot_y);
    if state.foot_y <= floor {
        state.foot_y = floor;
        state.vy = 0.0;
    }
    (state.velocity.length(), events)
}

/// The result of two karts touching: velocity changes and position pushes for each, and how long each spins out.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Bump {
    /// Change to A's velocity.
    pub dv_a: Vec2,
    /// Change to B's velocity.
    pub dv_b: Vec2,
    /// Move A by this to end the overlap.
    pub push_a: Vec2,
    /// Move B by this to end the overlap.
    pub push_b: Vec2,
    /// Spin-out ticks for A.
    pub spin_a: u16,
    /// Spin-out ticks for B.
    pub spin_b: u16,
}

/// Approach speed above which a Bear's bulldoze spins the other kart out, m/s.
const BULLDOZE_MIN_APPROACH: f32 = 3.0;
/// Spin-out length of a bulldoze, ticks.
const BULLDOZE_SPIN_TICKS: u16 = 45;

/// What happens when kart A and kart B overlap, or `None` if they do not. Momentum is exchanged by mass with some bounce; a Bear spins the other kart out
/// on a hard hit and takes only a third of the rebound himself. The caller applies the result (positions, velocities, [`KartState::spin_out`]).
pub fn kart_bump(pos_a: Vec2, vel_a: Vec2, spec_a: &KartSpec, pos_b: Vec2, vel_b: Vec2, spec_b: &KartSpec) -> Option<Bump> {
    let delta = pos_b - pos_a;
    let dist = delta.length();
    let min = 2.0 * KART_RADIUS;
    if dist >= min {
        return None;
    }
    let n = if dist > 1e-6 { delta / dist } else { Vec2::X };
    let (ma, mb) = (spec_a.mass, spec_b.mass);
    let overlap = min - dist;
    let mut bump = Bump { push_a: -n * overlap * (mb / (ma + mb)), push_b: n * overlap * (ma / (ma + mb)), ..Default::default() };
    let approach = (vel_a - vel_b).dot(n);
    if approach > 0.0 {
        let j = (1.0 + 0.6) * approach / (1.0 / ma + 1.0 / mb);
        bump.dv_a = -n * j / ma;
        bump.dv_b = n * j / mb;
        if approach > BULLDOZE_MIN_APPROACH {
            if spec_a.ability == Ability::Bulldoze && spec_b.ability != Ability::Bulldoze {
                bump.spin_b = BULLDOZE_SPIN_TICKS;
                bump.dv_a /= 3.0;
            } else if spec_b.ability == Ability::Bulldoze && spec_a.ability != Ability::Bulldoze {
                bump.spin_a = BULLDOZE_SPIN_TICKS;
                bump.dv_b /= 3.0;
            }
        }
    }
    Some(bump)
}

/// Slipstream: how fast (bar per second) a follower at `follower` heading along `follower_fwd` draws from a leader at `leader`. It needs the leader
/// 2-12 m ahead and within 1.8 m of the follower's line; nothing otherwise.
pub fn slipstream_gain(follower: Vec2, follower_fwd: Vec2, leader: Vec2) -> f32 {
    let to = leader - follower;
    let ahead = to.dot(follower_fwd);
    if !(2.0..=12.0).contains(&ahead) {
        return 0.0;
    }
    let side = (to - follower_fwd * ahead).length();
    if side > 1.8 {
        return 0.0;
    }
    0.5 * (1.0 - (ahead - 2.0) / 10.0) + 0.25
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Character;

    fn at(x: f32, z: f32, yaw_deg: f32) -> PlayerState {
        PlayerState::spawn(x, z, 0.0, yaw_deg, Character::Human)
    }

    fn drive(driver: Driver, ticks: u32, mut input: impl FnMut(u32) -> PlayerInput) -> (PlayerState, KartState) {
        let (mut s, mut k) = (at(0.0, 0.0, 0.0), KartState::default());
        let (spec, ground) = (driver.spec(), GroundCandidates::default());
        for t in 0..ticks {
            step_kart(&mut s, &mut k, &input(t), &spec, Surface::Road, &[], &ground);
        }
        (s, k)
    }

    fn throttle() -> PlayerInput {
        PlayerInput { forward: 1, ..Default::default() }
    }

    #[test]
    fn every_driver_reaches_its_top_speed_and_never_exceeds_it() {
        for d in Driver::ALL {
            let (s, _) = drive(d, 600, |_| throttle());
            let top = d.spec().top_speed;
            assert!((s.velocity.length() - top).abs() < 0.05, "{}: {} vs top {top}", d.name(), s.velocity.length());
            assert!(s.pos.y < -100.0, "{} drove forward (-Z)", d.name());
        }
    }

    #[test]
    fn the_table_matches_the_adr_and_the_drivers_are_distinct() {
        let mut names = std::collections::HashSet::new();
        for (i, d) in Driver::ALL.into_iter().enumerate() {
            assert!(names.insert(d.name()), "unique names");
            assert_eq!((d.wire() as usize, Driver::from_wire(d.wire())), (i, Some(d)));
            assert_eq!(Driver::parse(&d.name().to_uppercase()), Some(d));
        }
        assert_eq!(Driver::from_wire(8), None);
        let by = |f: fn(&KartSpec) -> f32| Driver::ALL.into_iter().max_by(|a, b| f(&a.spec()).total_cmp(&f(&b.spec()))).unwrap();
        assert_eq!(by(|s| s.top_speed), Driver::Deer, "Deer is the fastest");
        assert_eq!(by(|s| s.accel), Driver::Bunny, "Bunny is the quickest off the line");
        assert_eq!(by(|s| s.mass), Driver::Bear, "Bear is the heaviest");
        assert_eq!(by(|s| s.steer_rate), Driver::Bunny);
        assert_eq!(by(|s| s.hop_speed), Driver::Bunny);
        assert_eq!(Driver::Duck.spec().water, 1.0);
        assert_eq!(Driver::Coyote.spec().dirt, 1.0);
        let beaver = Driver::Beaver.spec();
        assert_eq!((beaver.mud, beaver.water, beaver.dirt), (1.0, 1.0, 1.0), "the wooden kart shrugs off the wet and the dirt");
        for d in Driver::ALL {
            let s = d.spec();
            assert!((18.0..=30.0).contains(&s.top_speed) && (0.5..=2.0).contains(&s.mass) && (0.5..=1.0).contains(&s.grip), "{} is in range", d.name());
        }
    }

    #[test]
    fn steering_needs_speed_and_turns_toward_the_stick() {
        let right = PlayerInput { forward: 1, strafe: 1, ..Default::default() };
        let (still, _) = drive(Driver::Duck, 60, |_| PlayerInput { strafe: 1, ..Default::default() });
        assert_eq!(still.yaw, 0.0, "a stopped kart cannot steer");
        let (moving, _) = drive(Driver::Duck, 120, |t| if t < 60 { throttle() } else { right });
        assert!(moving.yaw > 0.5, "steering right turns the heading clockwise: {}", moving.yaw);
        let left = PlayerInput { forward: 1, strafe: -1, ..Default::default() };
        let (l, _) = drive(Driver::Duck, 120, |t| if t < 60 { throttle() } else { left });
        assert!((l.yaw + moving.yaw).abs() < 0.02, "left mirrors right: {} vs {}", l.yaw, moving.yaw);
        let half = PlayerInput { analog: true, forward: 127, strafe: 64, ..Default::default() };
        let (h, _) = drive(Driver::Duck, 120, |t| if t < 60 { throttle() } else { half });
        assert!((h.yaw / moving.yaw - 0.5).abs() < 0.1, "half stick turns about half as fast: {} vs {}", h.yaw, moving.yaw);
    }

    #[test]
    fn braking_stops_and_then_reverses_slowly() {
        let brake = PlayerInput { forward: -1, ..Default::default() };
        let (s, _) = drive(Driver::Bear, 300, |t| if t < 200 { throttle() } else { brake });
        assert!(s.velocity.length() > 0.5 && s.velocity.dot(Vec2::new(0.0, 1.0)) > 0.0, "reversing (+Z) after stopping: {:?}", s.velocity);
        assert!(s.velocity.length() <= Driver::Bear.spec().reverse_speed + 0.01, "reverse is capped");
        let (coast, _) = drive(Driver::Bear, 600, |t| if t < 200 { throttle() } else { PlayerInput::default() });
        assert!(coast.velocity.length() < 0.01, "with no throttle the kart rolls to a stop");
    }

    #[test]
    fn off_road_slows_by_driver() {
        let ground = GroundCandidates::default();
        let top_on = |d: Driver, surface: Surface| {
            let (mut s, mut k, spec) = (at(0.0, 0.0, 0.0), KartState::default(), d.spec());
            for _ in 0..900 {
                step_kart(&mut s, &mut k, &throttle(), &spec, surface, &[], &ground);
            }
            s.velocity.length()
        };
        assert!(top_on(Driver::Deer, Surface::Dirt) < top_on(Driver::Deer, Surface::Road) - 3.0, "dirt slows the Deer");
        assert!((top_on(Driver::Coyote, Surface::Dirt) - top_on(Driver::Coyote, Surface::Road)).abs() < 0.05, "the Coyote does not mind dirt");
        assert!(top_on(Driver::Bunny, Surface::Water) < top_on(Driver::Duck, Surface::Water) - 5.0, "the Duck floats where the Bunny wades");
        assert!((top_on(Driver::Duck, Surface::Water) - Driver::Duck.spec().top_speed).abs() < 0.05);
        assert!((top_on(Driver::Beaver, Surface::Mud) - Driver::Beaver.spec().top_speed).abs() < 0.05, "the Beaver's wooden kart ignores mud");
        assert!(top_on(Driver::Wolf, Surface::Mud) < Driver::Wolf.spec().top_speed * 0.7);
    }

    #[test]
    fn a_wall_stops_the_kart_without_tunnelling() {
        let wall = Collider2D { min: Vec2::new(-5.0, -60.0), max: Vec2::new(5.0, -58.0), min_y: 0.0, max_y: 3.0 };
        let (mut s, mut k, spec, ground) = (at(0.0, 0.0, 0.0), KartState::default(), Driver::Deer.spec(), GroundCandidates::default());
        let mut closest = f32::MAX;
        for _ in 0..900 {
            step_kart(&mut s, &mut k, &throttle(), &spec, Surface::Road, &[wall], &ground);
            closest = closest.min(s.pos.y - wall.max.y);
        }
        assert!(closest >= KART_RADIUS - 0.01, "never inside the wall (nearest {closest})");
        assert!(s.velocity.length() < 3.0, "pinned against it the kart has no speed left: {}", s.velocity.length());
    }

    #[test]
    fn a_hop_leaves_the_ground_once_and_lands() {
        let (mut s, mut k, spec, ground) = (at(0.0, 0.0, 0.0), KartState::default(), Driver::Bunny.spec(), GroundCandidates::default());
        let hop = PlayerInput { forward: 1, jump: true, ..Default::default() };
        step_kart(&mut s, &mut k, &hop, &spec, Surface::Road, &[], &ground);
        assert!(s.foot_y > 0.0 && s.vy > 0.0, "the press hops");
        let mut peak = 0.0f32;
        for _ in 0..90 {
            step_kart(&mut s, &mut k, &hop, &spec, Surface::Road, &[], &ground); // still held: no second hop
            peak = peak.max(s.foot_y);
        }
        assert_eq!((s.foot_y, s.vy), (0.0, 0.0), "landed");
        assert!(peak > 0.2 && peak < 1.5, "a hop, not a jump: {peak}");
        let (mut b, mut bk, bear, _) = (at(0.0, 0.0, 0.0), KartState::default(), Driver::Bear.spec(), 0);
        step_kart(&mut b, &mut bk, &hop, &bear, Surface::Road, &[], &ground);
        assert!(b.vy < s.vy.max(spec.hop_speed), "the Bear hops lower than the Bunny");
    }

    #[test]
    fn a_long_drift_pays_a_boost_and_a_short_one_does_not() {
        let ground = GroundCandidates::default();
        let run = |driver: Driver, drift_ticks: u32| {
            let (mut s, mut k, spec) = (at(0.0, 0.0, 0.0), KartState::default(), driver.spec());
            for _ in 0..240 {
                step_kart(&mut s, &mut k, &throttle(), &spec, Surface::Road, &[], &ground);
            }
            let first = PlayerInput { forward: 1, strafe: 1, jump: true, ..Default::default() };
            for _ in 0..drift_ticks {
                step_kart(&mut s, &mut k, &first, &spec, Surface::Road, &[], &ground);
            }
            assert_eq!(k.drift_dir, 1, "a hop while steering at speed starts a drift");
            step_kart(&mut s, &mut k, &PlayerInput { forward: 1, ..Default::default() }, &spec, Surface::Road, &[], &ground);
            k
        };
        assert_eq!(run(Driver::Duck, 20).boost_ticks, 0, "under a second: nothing banked");
        // The boost is awarded on the release tick, which also spends its first tick: tier 1 is 30 ticks, tier 3 is 90.
        assert_eq!(run(Driver::Duck, 100).boost_ticks, 29, "a long drift boosts (tier 1)");
        assert_eq!(run(Driver::Duck, 260).boost_ticks, 89, "the longest drifts earn the top tier");
        assert!(run(Driver::Deer, 70).boost_ticks > run(Driver::Beaver, 70).boost_ticks, "the Deer charges faster than the Beaver");
        // The boost really is faster than the top speed.
        let (mut s, mut k, spec) = (at(0.0, 0.0, 0.0), KartState::default(), Driver::Duck.spec());
        for _ in 0..240 {
            step_kart(&mut s, &mut k, &throttle(), &spec, Surface::Road, &[], &ground);
        }
        assert!((s.velocity.length() - spec.top_speed).abs() < 0.05, "at top speed before the boost");
        k.boost(60);
        for _ in 0..50 {
            step_kart(&mut s, &mut k, &throttle(), &spec, Surface::Road, &[], &ground);
        }
        assert!(s.velocity.length() > spec.top_speed + 1.0, "boosting beats top speed: {}", s.velocity.length());
    }

    #[test]
    fn a_spin_out_takes_the_wheel_for_a_while() {
        let (mut s, mut k, spec, ground) = (at(0.0, 0.0, 0.0), KartState::default(), Driver::Wolf.spec(), GroundCandidates::default());
        for _ in 0..240 {
            step_kart(&mut s, &mut k, &throttle(), &spec, Surface::Road, &[], &ground);
        }
        let yaw = s.yaw;
        k.spin_out(45);
        for _ in 0..45 {
            step_kart(&mut s, &mut k, &throttle(), &spec, Surface::Road, &[], &ground);
        }
        assert!((s.yaw - yaw).abs() > 6.0, "it turned round on itself: {}", s.yaw - yaw);
        assert_eq!(k.spin_ticks, 0);
        let (_, speed) = (0, step_kart(&mut s, &mut k, &throttle(), &spec, Surface::Road, &[], &ground));
        assert!(speed.is_finite());
    }

    #[test]
    fn the_hawk_glides_and_others_fall() {
        let ground = GroundCandidates::default();
        let airtime = |driver: Driver| {
            let (mut s, mut k, spec) = (at(0.0, 0.0, 0.0), KartState::default(), driver.spec());
            s.foot_y = 5.0;
            let hold = PlayerInput { forward: 1, jump: true, ..Default::default() };
            let mut ticks = 0;
            while s.foot_y > 0.0 && ticks < 1000 {
                step_kart(&mut s, &mut k, &hold, &spec, Surface::Road, &[], &ground);
                ticks += 1;
            }
            ticks
        };
        assert!(airtime(Driver::Hawk) > airtime(Driver::Duck) * 3 / 2, "the Hawk stays up longer: {} vs {}", airtime(Driver::Hawk), airtime(Driver::Duck));
    }

    #[test]
    fn bumps_trade_momentum_by_mass_and_the_bear_bulldozes() {
        let (duck, bear, bunny) = (Driver::Duck.spec(), Driver::Bear.spec(), Driver::Bunny.spec());
        assert_eq!(kart_bump(Vec2::ZERO, Vec2::ZERO, &duck, Vec2::new(5.0, 0.0), Vec2::ZERO, &duck), None, "apart: no bump");
        let a = Vec2::ZERO;
        let b = Vec2::new(1.0, 0.0);
        let head_on = kart_bump(a, Vec2::new(10.0, 0.0), &duck, b, Vec2::ZERO, &duck).unwrap();
        assert!(head_on.dv_a.x < 0.0 && head_on.dv_b.x > 0.0, "A is slowed, B is shoved on");
        assert!((duck.mass * head_on.dv_a + duck.mass * head_on.dv_b).length() < 1e-4, "equal masses conserve momentum");
        assert!(head_on.push_a.x < 0.0 && head_on.push_b.x > 0.0, "the overlap is undone");
        let (mut spin_a, mut spin_b) = (head_on.spin_a, head_on.spin_b);
        assert_eq!((spin_a, spin_b), (0, 0), "ordinary karts do not spin each other");
        let heavy = kart_bump(a, Vec2::new(10.0, 0.0), &bear, b, Vec2::ZERO, &bunny).unwrap();
        assert_eq!(heavy.spin_b, 45, "the Bear spins the Bunny out");
        assert_eq!(heavy.spin_a, 0);
        assert!(heavy.dv_b.x > head_on.dv_b.x, "and shoves a light kart harder than an equal one would");
        let reverse = kart_bump(a, Vec2::ZERO, &bunny, b, Vec2::new(-10.0, 0.0), &bear).unwrap();
        assert_eq!((reverse.spin_a, reverse.spin_b), (45, 0), "it works whichever side the Bear is on");
        spin_a += reverse.spin_a;
        spin_b += reverse.spin_b;
        assert_eq!(spin_a + spin_b, 45);
        assert!(kart_bump(a, Vec2::new(2.0, 0.0), &bear, b, Vec2::ZERO, &bunny).unwrap().spin_b == 0, "a gentle nudge does not spin anyone");
        assert!(kart_bump(a, Vec2::ZERO, &duck, a, Vec2::ZERO, &duck).is_some(), "exactly on top of each other still separates");
    }

    #[test]
    fn a_wolf_banks_a_boost_from_a_slipstream_and_others_do_not() {
        let (wolf, duck) = (Driver::Wolf.spec(), Driver::Duck.spec());
        let fwd = Vec2::new(0.0, -1.0);
        assert_eq!(slipstream_gain(Vec2::ZERO, fwd, Vec2::new(0.0, 1.0)), 0.0, "behind: nothing");
        assert_eq!(slipstream_gain(Vec2::ZERO, fwd, Vec2::new(0.0, -20.0)), 0.0, "too far");
        assert_eq!(slipstream_gain(Vec2::ZERO, fwd, Vec2::new(0.0, -1.0)), 0.0, "too close");
        assert_eq!(slipstream_gain(Vec2::ZERO, fwd, Vec2::new(4.0, -6.0)), 0.0, "off to the side");
        let near = slipstream_gain(Vec2::ZERO, fwd, Vec2::new(0.0, -3.0));
        assert!(near > slipstream_gain(Vec2::ZERO, fwd, Vec2::new(0.0, -10.0)) && near > 0.0, "closer draws more");
        let (mut w, mut d) = (KartState::default(), KartState::default());
        for _ in 0..600 {
            w.draft(&wolf, near, FIXED_DT);
            d.draft(&duck, near, FIXED_DT);
        }
        assert!(w.boost_ticks > 0 || w.slip_charge > 0.0, "the Wolf drafted");
        assert_eq!((d.boost_ticks, d.slip_charge), (0, 0.0), "the Duck does not");
        let mut lone = KartState { slip_charge: 0.8, ..Default::default() };
        for _ in 0..600 {
            lone.draft(&wolf, 0.0, FIXED_DT);
        }
        assert_eq!(lone.slip_charge, 0.0, "the bar drains when nobody is ahead");
    }

    #[test]
    fn the_same_inputs_give_bit_identical_karts() {
        // Server, client prediction and bots must agree exactly: a scripted run with steering, drifting, hops and a wall, twice.
        let wall = Collider2D { min: Vec2::new(30.0, -400.0), max: Vec2::new(32.0, 0.0), min_y: 0.0, max_y: 3.0 };
        for d in Driver::ALL {
            let mut runs = Vec::new();
            for _ in 0..2 {
                let (mut s, mut k, spec, ground) = (at(0.0, 0.0, 0.0), KartState::default(), d.spec(), GroundCandidates::default());
                for t in 0..1500u32 {
                    let input = PlayerInput {
                        seq: t,
                        forward: if t % 500 < 450 { 1 } else { -1 },
                        strafe: ((t / 40) % 3) as i8 - 1,
                        jump: t % 200 < 90,
                        ..Default::default()
                    };
                    step_kart(&mut s, &mut k, &input, &spec, Surface::Road, &[wall], &ground);
                }
                runs.push((s, k));
            }
            assert_eq!(runs[0], runs[1], "{} is deterministic", d.name());
            assert!(runs[0].0.pos.is_finite() && runs[0].0.velocity.is_finite());
        }
    }

    fn tick_ex(driver: Driver, kart: &mut KartState, state: &mut PlayerState, input: PlayerInput) -> KartEvents {
        step_kart_ex(state, kart, &input, &driver.spec(), Surface::Road, &[], &GroundCandidates::default()).1
    }

    #[test]
    fn a_pickup_is_used_on_the_press_and_only_once() {
        let press = PlayerInput { forward: 1, attack: true, ..Default::default() };
        let release = PlayerInput { forward: 1, ..Default::default() };
        // Mushroom: boosts the kart itself, right in the step, and is gone.
        let (mut s, mut k) = (at(0.0, 0.0, 0.0), KartState { item: Item::Mushroom, ..Default::default() });
        assert_eq!(tick_ex(Driver::Duck, &mut k, &mut s, press), KartEvents::default());
        assert_eq!((k.item, k.boost_ticks), (Item::None, MUSHROOM_BOOST_TICKS - 1));
        // Holding the button does not use the next one; releasing and pressing again does.
        k.item = Item::Mushroom;
        tick_ex(Driver::Duck, &mut k, &mut s, press);
        assert_eq!(k.item, Item::Mushroom, "still held down from the first press");
        tick_ex(Driver::Duck, &mut k, &mut s, release);
        tick_ex(Driver::Duck, &mut k, &mut s, press);
        assert_eq!(k.item, Item::None);
        // Bubble: a shield that lasts, and pops on the first hit.
        let (mut s, mut k) = (at(0.0, 0.0, 0.0), KartState { item: Item::Bubble, ..Default::default() });
        tick_ex(Driver::Duck, &mut k, &mut s, press);
        assert_eq!((k.item, k.shield_ticks), (Item::None, BUBBLE_TICKS - 1));
        assert!(!k.spin_out(45), "the bubble absorbs the hit");
        assert_eq!((k.spin_ticks, k.shield_ticks), (0, 0), "and pops");
        assert!(k.spin_out(45) && k.spin_ticks == 45, "the next hit lands");
        // Acorn: the kart asks for a throw; the world does the rest.
        let (mut s, mut k) = (at(0.0, 0.0, 0.0), KartState { item: Item::Acorn, ..Default::default() });
        assert!(tick_ex(Driver::Duck, &mut k, &mut s, press).throw_acorn);
        assert_eq!(k.item, Item::None);
        assert!(!tick_ex(Driver::Duck, &mut k, &mut s, release).throw_acorn);
        // No item: nothing happens. A shield runs out by itself.
        let (mut s, mut k) = (at(0.0, 0.0, 0.0), KartState::default());
        assert_eq!(tick_ex(Driver::Duck, &mut k, &mut s, press), KartEvents::default());
        k.shield_ticks = 2;
        tick_ex(Driver::Duck, &mut k, &mut s, release);
        tick_ex(Driver::Duck, &mut k, &mut s, release);
        assert_eq!(k.shield_ticks, 0);
    }

    #[test]
    fn nothing_is_used_while_spinning_out() {
        let press = PlayerInput { forward: 1, attack: true, ..Default::default() };
        let (mut s, mut k) = (at(0.0, 0.0, 0.0), KartState { item: Item::Mushroom, spin_ticks: 5, ..Default::default() });
        tick_ex(Driver::Duck, &mut k, &mut s, press);
        assert_eq!((k.item, k.boost_ticks), (Item::Mushroom, 0), "the press during a spin is lost, and the item is kept");
    }

    #[test]
    fn only_the_beaver_builds_and_its_cooldown_limits_it() {
        let build = PlayerInput { forward: 1, interact: true, ..Default::default() };
        let release = PlayerInput { forward: 1, ..Default::default() };
        let (mut s, mut k) = (at(0.0, 0.0, 0.0), KartState::default());
        assert!(!tick_ex(Driver::Bear, &mut k, &mut s, build).lay_plank, "only the Beaver builds");
        let (mut s, mut k) = (at(0.0, 0.0, 0.0), KartState::default());
        assert!(tick_ex(Driver::Beaver, &mut k, &mut s, build).lay_plank);
        assert_eq!(k.ability_cooldown, BUILD_COOLDOWN_TICKS - 1);
        tick_ex(Driver::Beaver, &mut k, &mut s, release);
        assert!(!tick_ex(Driver::Beaver, &mut k, &mut s, build).lay_plank, "still cooling down");
        for _ in 0..BUILD_COOLDOWN_TICKS {
            tick_ex(Driver::Beaver, &mut k, &mut s, release);
        }
        assert_eq!(k.ability_cooldown, 0);
        assert!(tick_ex(Driver::Beaver, &mut k, &mut s, build).lay_plank, "ready again");
    }
}
