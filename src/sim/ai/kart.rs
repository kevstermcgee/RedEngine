//! Kart bots for Great Outdoors: a driver brain that races.
//!
//! Like the fighters in `sim::ai`, a [`KartBrain`] is an ordinary player slot with a mind attached: each tick, before inputs are consumed, it looks at the
//! world exactly as a human could and queues one [`PlayerInput`] (throttle and steering on the stick axes, hop/drift on `jump`, the item on `attack`, the
//! driver's ability on `interact`). It drives through the same `step_kart` as everyone, so a bot cannot do what a person cannot, and a recorded trace
//! replays without a brain at all (the inputs are in it). A brain is a pure function of the world it is shown and its own seeded generator.
//!
//! What it does each tick:
//! 1. **Follow the line**: the course's `race.line` points, or the gate centres if the track has none, in order, with a look-ahead that grows with speed;
//!    a point it has passed is skipped, and one it keeps missing is dropped by the stuck recovery below.
//! 2. **Set the speed** from the corner ahead (the angle between where it is heading and where the line turns next) and its skill: a rookie leaves
//!    speed on the table, a nightmare takes the corners at the limit.
//! 3. **Steer** towards the look-ahead point on the analog stick, smoothed (a slower reaction at low skill) and with a wobble that shrinks with skill.
//! 4. **Drift** long corners once it is good enough: a hop while steering at speed starts one, held until the corner opens, paying the boost.
//! 5. **Use what it holds**: a Mushroom on a straight, an Acorn at a kart ahead in its lane, a Bubble when an Acorn is coming or it has held it too long;
//!    the Beaver lays a plank in front of a rival close behind.
//! 6. **Dodge hazards** on its line by aiming a few metres to the side of them.
//! 7. **Recover**: wedged against something it backs up, turning the other way, and rejoins the line.

use crate::sim::items::HazardKind;
use crate::sim::kart::{Ability, Item, DRIFT_MIN_SPEED};
use crate::sim::match_sim::{MatchSim, MAX_PLAYERS};
use crate::sim::player::PlayerInput;
use glam::Vec2;

/// Ticks a bot may sit nearly still before it decides it is stuck.
const STUCK_TICKS: u32 = 50;
/// Ticks it reverses out of a jam.
const REVERSE_TICKS: u32 = 55;
/// Longest a bot holds a drift, ticks.
const MAX_DRIFT_TICKS: u32 = 150;
/// How long it holds a Bubble before using it anyway, ticks.
const BUBBLE_HOLD_TICKS: u32 = 240;

/// One bot's mind.
#[derive(Debug, Clone)]
pub struct KartBrain {
    name: String,
    /// Skill, `0.0` (rookie) to `1.0` (nightmare).
    level: f32,
    rng: u64,
    /// The racing line, built from the course on the first tick.
    path: Vec<Vec2>,
    target: usize,
    steer: f32,
    stuck_ticks: u32,
    reverse_ticks: u32,
    reverse_steer: f32,
    drift_ticks: u32,
    item_age: u32,
    last: Vec2,
    /// The line point it was wedged at last time; being wedged at the same one again sends it back a point.
    stuck_at: Option<usize>,
    /// Heading back to a point it had already "passed": only reaching it counts, not passing it.
    backtrack: bool,
}

impl KartBrain {
    /// A brain called `name` at skill `level` (`0..=1`); `seed` (the slot and the tick it joined) makes two bots of the same skill differ.
    pub fn new(name: impl Into<String>, level: f32, seed: u64) -> KartBrain {
        KartBrain {
            name: name.into(),
            level: level.clamp(0.0, 1.0),
            rng: seed | 1,
            path: Vec::new(),
            target: 0,
            steer: 0.0,
            stuck_ticks: 0,
            reverse_ticks: 0,
            reverse_steer: 1.0,
            drift_ticks: 0,
            item_age: 0,
            last: Vec2::ZERO,
            stuck_at: None,
            backtrack: false,
        }
    }

    /// The bot's name on the scoreboard.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its skill, `0..=1`.
    pub fn level(&self) -> f32 {
        self.level
    }

    /// The index of the line point it is steering for (diagnostics).
    pub fn target(&self) -> usize {
        self.target
    }

    /// A uniform number in `0..1` from the brain's own generator (xorshift64*).
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        ((self.rng.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32) / (1u64 << 24) as f32
    }

    /// What this bot does this tick, from what the world shows it.
    pub fn think(&mut self, sim: &MatchSim, slot: usize) -> PlayerInput {
        let seq = sim.tick() as u32 + 1;
        let idle = PlayerInput { seq, ..Default::default() };
        let (Some(p), Some(race), Some(kart), Some(driver)) = (sim.player(slot), sim.race(), sim.kart(slot), sim.driver(slot)) else { return idle };
        if !race.can_drive() {
            return idle;
        }
        if self.path.is_empty() {
            let course = race.course();
            self.path = if course.line.len() >= 2 { course.line.clone() } else { course.gates.iter().map(|g| g.center()).collect() };
            self.target = 0;
            self.last = p.state.pos;
        }
        let n = self.path.len();
        let spec = driver.spec();
        let (sin, cos) = p.state.yaw.sin_cos();
        let (fwd, right) = (Vec2::new(sin, -cos), Vec2::new(cos, sin));
        let pos = p.state.pos;
        let speed = p.state.velocity.dot(fwd);

        // Move on from a point once reached or passed.
        let reach = (speed * 0.45).clamp(6.0, 18.0);
        for _ in 0..n {
            let prev = self.path[(self.target + n - 1) % n];
            let point = self.path[self.target];
            let along = point - prev;
            let passed = !self.backtrack && along.length_squared() > 1e-6 && (point - pos).dot(along) < 0.0;
            if (point - pos).length() < reach || passed {
                self.backtrack = false;
                self.target = (self.target + 1) % n;
            } else {
                break;
            }
        }

        // Stuck: nearly still for a while while the light is green. Back out the other way, then carry on.
        if (pos - self.last).length() < 0.02 && speed.abs() < 1.5 && kart.spin_ticks == 0 {
            self.stuck_ticks += 1;
        } else {
            self.stuck_ticks = 0;
        }
        self.last = pos;
        if self.stuck_ticks > STUCK_TICKS && self.reverse_ticks == 0 {
            self.reverse_ticks = REVERSE_TICKS;
            // Wedged twice in a row while heading for the same point (it overshot a gap and is pushing at a wall the point is behind): go back to the
            // previous point and approach again.
            if self.stuck_at == Some(self.target) {
                self.target = (self.target + n - 1) % n;
                self.backtrack = true;
                self.stuck_at = None;
            } else {
                self.stuck_at = Some(self.target);
            }
            self.reverse_steer = if self.steer >= 0.0 { -1.0 } else { 1.0 };
            self.stuck_ticks = 0;
        }
        if self.reverse_ticks > 0 {
            self.reverse_ticks -= 1;
            return PlayerInput { seq, analog: true, forward: -127, strafe: (self.reverse_steer * 127.0) as i8, ..Default::default() };
        }

        // Where to aim: the look-ahead point, nudged sideways round a hazard sitting on the line.
        let mut aim = self.path[self.target] - pos;
        for h in sim.hazards().iter() {
            if h.kind == HazardKind::Acorn && h.owner as usize == slot {
                continue;
            }
            let rel = h.pos - pos;
            let (ahead, side) = (rel.dot(fwd), rel.dot(right));
            if (3.0..15.0).contains(&ahead) && side.abs() < 2.2 && h.kind == HazardKind::Plank {
                aim -= right * (if side >= 0.0 { 1.0 } else { -1.0 }) * 3.5;
            }
        }
        let to = aim.normalize_or_zero();
        let angle_err = to.dot(right).atan2(to.dot(fwd));

        // Speed for the corner beyond the point it is heading for, and for its skill.
        let next_dir = (self.path[(self.target + 1) % n] - self.path[self.target]).normalize_or_zero();
        let sharpness = to.dot(next_dir).clamp(-1.0, 1.0).acos();
        let corner = 1.0 - (sharpness / 1.6).clamp(0.0, 0.55);
        // Catch-up: the leader eases off a little and the tail pushes a little, so a pack of equally good drivers on karts of different speeds stays a pack
        // (and there is someone near enough to throw an Acorn at). A driver at nightmare level has none of it, which is what balance measurements use.
        let standings = race.standings();
        let place = standings.iter().position(|row| row.player == slot).unwrap_or(0);
        let fraction = place as f32 / (standings.len().max(2) - 1) as f32;
        let catch_up = if self.level < 0.95 { 1.05 - 0.10 * fraction } else { 1.0 };
        let pace = (0.80 + 0.20 * self.level) * catch_up;
        let boosting = kart.boost_ticks > 0;
        let desired = spec.top_speed * corner * pace * if boosting { spec.boost_mult } else { 1.0 };
        let forward: i8 = if speed < desired - 1.0 {
            127
        } else if speed > desired + 3.0 {
            -80
        } else {
            70
        };

        // Steering: proportional to the angle, smoothed by skill, with a wobble that a good driver does not have.
        let wobble = (self.random() - 0.5) * 0.24 * (1.0 - self.level);
        let want = if to.dot(fwd) < 0.0 { angle_err.signum() } else { (angle_err * 1.7).clamp(-1.0, 1.0) };
        let alpha = 0.35 + 0.55 * self.level;
        self.steer += (want + wobble - self.steer) * alpha;
        let strafe = (self.steer.clamp(-1.0, 1.0) * 127.0) as i8;

        // Drift a long corner once good enough: hop while steering at speed, hold until the corner opens.
        let jump = if kart.drift_dir != 0 {
            self.drift_ticks += 1;
            angle_err.abs() > 0.14 && self.drift_ticks < MAX_DRIFT_TICKS
        } else {
            self.drift_ticks = 0;
            self.level >= 0.4 && angle_err.abs() > 0.45 && speed > DRIFT_MIN_SPEED * 1.4 && !kart.jump_held && kart.spin_ticks == 0
        };

        // Items and the driver's ability, each on a fresh press.
        self.item_age = if kart.item == Item::None { 0 } else { self.item_age + 1 };
        let mut attack = false;
        let mut interact = false;
        let rivals = (0..MAX_PLAYERS).filter(|s| *s != slot).filter_map(|s| sim.player(s).map(|q| q.state.pos - pos));
        let (mut nearest_ahead, mut nearest_behind): (Option<Vec2>, Option<Vec2>) = (None, None);
        for rel in rivals {
            let (ahead, side) = (rel.dot(fwd), rel.dot(right));
            // An Acorn homes onto a kart inside a cone ahead of it (`sim::items`), so that is what a bot waits for: a rival ahead and roughly in front.
            if ahead > 0.0 && ahead / rel.length().max(1e-3) > 0.9 && nearest_ahead.is_none_or(|b| b.length() > rel.length()) {
                nearest_ahead = Some(rel);
            }
            if ahead < 0.0 && side.abs() < 3.0 && nearest_behind.is_none_or(|b| b.length() > rel.length()) {
                nearest_behind = Some(rel);
            }
        }
        let acorn_coming = sim.hazards().iter().any(|h| {
            let rel = h.pos - pos;
            h.kind == HazardKind::Acorn && h.owner as usize != slot && rel.dot(fwd) < 0.0 && rel.dot(fwd) > -25.0 && rel.dot(right).abs() < 3.0
        });
        let reaction_ok = self.random() < 0.25 + 0.75 * self.level;
        if !kart.attack_held && reaction_ok {
            attack = match kart.item {
                Item::Mushroom => angle_err.abs() < 0.12 && !boosting && speed > 6.0,
                Item::Acorn => nearest_ahead.is_some_and(|r| (6.0..38.0).contains(&r.dot(fwd))),
                Item::Bubble => acorn_coming || self.item_age > BUBBLE_HOLD_TICKS,
                Item::None => false,
            };
        }
        if spec.ability == Ability::Build && !kart.interact_held && kart.ability_cooldown == 0 && reaction_ok {
            interact = nearest_behind.is_some_and(|r| (3.0..14.0).contains(&-r.dot(fwd)));
        }

        PlayerInput { seq, analog: true, forward, strafe, jump, attack, interact, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generator_is_deterministic_and_spread_across_zero_to_one() {
        let (mut a, mut b) = (KartBrain::new("a", 0.5, 7), KartBrain::new("b", 0.5, 7));
        let xs: Vec<f32> = (0..2000).map(|_| a.random()).collect();
        assert_eq!(xs, (0..2000).map(|_| b.random()).collect::<Vec<f32>>());
        assert!(xs.iter().all(|x| (0.0..1.0).contains(x)));
        let mean = xs.iter().sum::<f32>() / xs.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "mean {mean}");
        assert_ne!(KartBrain::new("c", 0.5, 8).random(), KartBrain::new("c", 0.5, 7).random(), "the seed matters");
    }

    #[test]
    fn a_brain_reports_its_name_and_a_clamped_level() {
        let b = KartBrain::new("Pip", 1.7, 1);
        assert_eq!((b.name(), b.level(), b.target()), ("Pip", 1.0, 0));
        assert_eq!(KartBrain::new("x", -3.0, 1).level(), 0.0);
    }
}
