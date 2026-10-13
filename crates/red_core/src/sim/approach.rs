//! Object-based player actions, shared by the two ways a scripted player is driven: `checks.sim` scenarios (the headless simulation) and
//! `re2 --script` / `playtest` (the real client). Both say `approach`, `look_at` and `interact` with an object id instead of world
//! coordinates, durations and angles; this module is the one definition of what those mean, so a step that works in one works in the other.
//!
//! | step | done when | fails when |
//! |---|---|---|
//! | `approach: id` | the player's horizontal gap to the object's footprint is at most `within` (default: 60% of the body's pickup reach) | no closer after [`STUCK_SECS`] (something is in the way), or [`DEFAULT_TIMEOUT_SECS`] / `timeout` pass |
//! | `look_at: id` | at once: yaw and pitch point from the eye at the middle of the object | the object does not exist |
//! | `interact: id` | approach (if not already close), look, press E, and a loose prop is now held (or already was) | the approach fails, or the press took nothing (the reason is named) |
//!
//! The steering is a straight line, like `walk`: around a wall, `walk` to a corner first. Everything here is pure (no sim, no client), so both
//! sides test it the same way.

use glam::{Vec2, Vec3};

/// Seconds an `approach` may take before it fails, unless the step says `timeout`.
pub const DEFAULT_TIMEOUT_SECS: f32 = 12.0;
/// Seconds without getting meaningfully closer after which an `approach` is called stuck.
pub const STUCK_SECS: f32 = 2.0;
/// Metres closer that count as progress for the stuck test.
const PROGRESS: f32 = 0.2;

/// Where an object is right now, as a world box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    /// Lowest corner.
    pub min: Vec3,
    /// Highest corner.
    pub max: Vec3,
}

impl Target {
    /// A fixed object from its world bounds.
    pub fn from_bounds(min: Vec3, max: Vec3) -> Target {
        Target { min, max }
    }

    /// A loose prop: its origin sits on the underside of its box (`RuleProp::origin`), `extents` are the box's side lengths.
    pub fn from_prop(origin: Vec3, extents: Vec3) -> Target {
        let half = Vec3::new(extents.x, 0.0, extents.z) * 0.5;
        Target { min: origin - half, max: origin + half + Vec3::new(0.0, extents.y, 0.0) }
    }

    /// The point `look_at` aims at: the middle of the box.
    pub fn aim_point(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Horizontal distance from `pos` (x, z) to the object's footprint: 0 when the player stands inside it.
    pub fn gap(&self, pos: Vec2) -> f32 {
        let dx = (self.min.x - pos.x).max(0.0).max(pos.x - self.max.x);
        let dz = (self.min.z - pos.y).max(0.0).max(pos.y - self.max.z);
        dx.hypot(dz)
    }

    /// Yaw (radians, 0 looks along -Z, clockwise from above) that faces the middle of the footprint from `pos`.
    pub fn heading(&self, pos: Vec2) -> f32 {
        let c = self.aim_point();
        libm::atan2f(c.x - pos.x, -(c.z - pos.y))
    }
}

/// How close an `approach` gets by default: 60% of the body's pickup reach, so the eye-to-object ray an `interact` casts is well within it.
pub fn default_within(pickup_reach: f32) -> f32 {
    (pickup_reach * 0.6).clamp(0.3, 1.5)
}

/// How close `interact` walks before it presses E: close enough that the *eye-to-object* distance, which is what the pick-up reach is measured in, is at most 80% of
/// `pickup_reach`, never looser than [`default_within`]. On flat ground that is the same ~1.4 m as ever; on a slope, where a low prop sits well below the eye, the walker
/// must come closer, because the vertical part of the distance uses up some of the reach (a prop 1.9 m below the eye is out of reach at 1.4 m horizontally).
pub fn pickup_within(pickup_reach: f32, eye: Vec3, target: &Target) -> f32 {
    let dy = (eye.y - target.aim_point().y).abs();
    let budget = pickup_reach * 0.8;
    let half = 0.5 * (target.max.x - target.min.x).max(target.max.z - target.min.z);
    // Horizontal distance from the eye to the object's middle that keeps the 3-D distance within budget; the gap is measured to its footprint, so take the half-width off.
    let centre = (budget * budget - dy * dy).max(0.0).sqrt();
    (centre - half).clamp(0.3, default_within(pickup_reach))
}

/// `(yaw, pitch)` in radians that point a view at `at` from `eye`.
pub fn aim(eye: Vec3, at: Vec3) -> (f32, f32) {
    let d = at - eye;
    (libm::atan2f(d.x, -d.z), libm::atan2f(d.y, libm::sqrtf(d.x * d.x + d.z * d.z)))
}

/// Why an `approach` gave up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// No progress for [`STUCK_SECS`]: a wall, a prop or a ledge is in the way.
    Stuck,
    /// The step's time ran out.
    Timeout,
}

/// What the player should do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Close enough.
    Arrived,
    /// Keep walking at the object.
    Moving,
    /// Give up.
    Failed(Failure),
}

/// One `approach` in progress: the time it has taken and whether it is still getting closer.
#[derive(Debug, Clone, PartialEq)]
pub struct Approach {
    within: f32,
    timeout: f32,
    elapsed: f32,
    best: f32,
    idle: f32,
}

impl Approach {
    /// Starts an approach that is done at `within` metres and fails after `timeout` seconds.
    pub fn new(within: f32, timeout: f32) -> Approach {
        Approach { within, timeout, elapsed: 0.0, best: f32::INFINITY, idle: 0.0 }
    }

    /// The distance it is done at.
    pub fn within(&self) -> f32 {
        self.within
    }

    /// Seconds spent so far.
    pub fn elapsed(&self) -> f32 {
        self.elapsed
    }

    /// Advances by `dt` seconds with the player now `gap` metres from the object (see [`Target::gap`]).
    pub fn step(&mut self, dt: f32, gap: f32) -> Progress {
        if gap <= self.within {
            return Progress::Arrived;
        }
        self.elapsed += dt;
        if gap < self.best - PROGRESS || gap > self.best + 2.0 {
            // Progress, or the player was moved (a teleport): measure again from here.
            (self.best, self.idle) = (gap, 0.0);
        } else {
            self.idle += dt;
        }
        if self.idle >= STUCK_SECS {
            Progress::Failed(Failure::Stuck)
        } else if self.elapsed >= self.timeout {
            Progress::Failed(Failure::Timeout)
        } else {
            Progress::Moving
        }
    }
}

/// The sentence for a failed `approach`/`interact` step: what, where the player stopped and how far from the goal.
pub fn failure_message(step: &str, id: &str, why: Failure, gap: f32, within: f32, at: Vec2, secs: f32) -> String {
    let cause = match why {
        Failure::Stuck => format!("no closer for {STUCK_SECS:.0} s (something is in the way: `walk` round it first, or `lint`/`walk` the route)"),
        Failure::Timeout => format!("not there after {secs:.0} s (raise `timeout` if the way is long)"),
    };
    format!("{step} `{id}`: stopped at ({:.2}, {:.2}), {gap:.2} m from it, wanted within {within:.2} m: {cause}", at.x, at.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crate_at(x: f32, z: f32) -> Target {
        Target::from_prop(Vec3::new(x, 0.0, z), Vec3::new(0.6, 0.6, 0.6))
    }

    #[test]
    fn gap_is_the_distance_to_the_footprint_and_zero_inside_it() {
        let t = crate_at(0.0, 0.0);
        assert!((t.gap(Vec2::new(2.3, 0.0)) - 2.0).abs() < 1e-5);
        assert!((t.gap(Vec2::new(0.3 + 3.0, 0.3 + 4.0)) - 5.0).abs() < 1e-5, "diagonal to the corner");
        assert_eq!(t.gap(Vec2::new(0.1, -0.1)), 0.0);
    }

    #[test]
    fn a_prop_target_stands_on_its_origin_and_aims_at_its_middle() {
        let t = crate_at(1.0, 2.0);
        assert_eq!(t.aim_point(), Vec3::new(1.0, 0.3, 2.0));
        assert_eq!(t.min.y, 0.0);
    }

    #[test]
    fn aim_and_heading_use_the_engines_yaw_convention() {
        // yaw 0 looks along -Z; a target to the east (+X) is 90 degrees clockwise.
        let (yaw, pitch) = aim(Vec3::new(0.0, 1.6, 0.0), Vec3::new(0.0, 1.6, -5.0));
        assert!(yaw.abs() < 1e-6 && pitch.abs() < 1e-6);
        let (yaw, _) = aim(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0));
        assert!((yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        let (_, pitch) = aim(Vec3::new(0.0, 1.6, 0.0), Vec3::new(0.0, 0.3, -1.3));
        assert!(pitch < 0.0, "a prop on the floor is below the eye");
        assert!((crate_at(5.0, 0.0).heading(Vec2::ZERO) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn default_within_follows_the_body_reach() {
        assert!((default_within(2.3) - 1.38).abs() < 1e-5);
        assert_eq!(default_within(0.9), 0.54);
        assert_eq!(default_within(10.0), 1.5);
    }

    #[test]
    fn an_approach_arrives_gets_stuck_or_times_out() {
        let mut a = Approach::new(1.0, 12.0);
        assert_eq!(a.step(0.1, 5.0), Progress::Moving);
        assert_eq!(a.step(0.1, 4.0), Progress::Moving);
        assert_eq!(a.step(0.1, 0.9), Progress::Arrived);
        // Pressed against a wall: the gap stops shrinking.
        let mut a = Approach::new(1.0, 12.0);
        let mut last = Progress::Moving;
        for _ in 0..40 {
            last = a.step(0.1, 3.0);
            if last != Progress::Moving {
                break;
            }
        }
        assert_eq!(last, Progress::Failed(Failure::Stuck));
        // Always closing, but too slowly for the limit.
        let mut a = Approach::new(1.0, 1.0);
        let (mut gap, mut last) = (50.0, Progress::Moving);
        for _ in 0..20 {
            gap -= 0.5;
            last = a.step(0.1, gap);
            if last != Progress::Moving {
                break;
            }
        }
        assert_eq!(last, Progress::Failed(Failure::Timeout));
        // A teleport (the gap jumps up) restarts the stuck clock instead of failing.
        let mut a = Approach::new(1.0, 30.0);
        for _ in 0..15 {
            a.step(0.1, 3.0);
        }
        assert_eq!(a.step(0.1, 9.0), Progress::Moving);
    }

    #[test]
    fn the_failure_names_the_object_the_place_and_the_distance() {
        let m = failure_message("approach", "parcel_1", Failure::Stuck, 3.2, 1.38, Vec2::new(1.0, -2.0), 4.0);
        assert!(m.contains("`parcel_1`") && m.contains("(1.00, -2.00)") && m.contains("3.20 m") && m.contains("in the way"), "{m}");
        assert!(failure_message("interact", "x", Failure::Timeout, 5.0, 1.0, Vec2::ZERO, 12.0).contains("raise `timeout`"));
    }

    #[test]
    fn interact_walks_closer_when_the_prop_is_far_below_the_eye() {
        let flat = Target::from_prop(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.3, 0.4, 0.3));
        let eye_flat = Vec3::new(0.0, 1.6, 2.0);
        let on_flat = pickup_within(2.3, eye_flat, &flat);
        assert!(on_flat <= default_within(2.3) && on_flat > 0.9, "flat ground keeps about the old distance: {on_flat}");
        let downhill = pickup_within(2.3, Vec3::new(0.0, 1.6 + 0.4, 2.0), &flat);
        assert!(downhill < on_flat - 0.1, "standing 0.4 m higher means coming closer: {downhill} against {on_flat}");
        // At the extreme the vertical distance alone eats the reach: still a sensible minimum, never zero or negative.
        assert_eq!(pickup_within(2.3, Vec3::new(0.0, 9.0, 0.0), &flat), 0.3);
        // A big object's footprint edge is nearer than its middle, so the walker may stop a little farther out.
        let wide = Target::from_prop(Vec3::ZERO, Vec3::new(1.0, 0.4, 1.0));
        assert!(pickup_within(2.3, eye_flat, &wide) <= default_within(2.3));
    }
}
