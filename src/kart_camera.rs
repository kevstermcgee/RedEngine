//! The third-person chase camera for kart racing (Great Outdoors), as a pure state machine: no window, no GPU, testable.
//!
//! The camera hangs behind and above the kart. It follows the kart's *heading* with a lag, so a turn swings the view round rather than snapping it, and
//! while drifting it leans towards the direction the kart is actually travelling (the tail slides out, the view looks into the slide). At speed it pulls
//! back and widens the field of view; a boost pulls a little harder. Its output is an eye, a yaw and a pitch in the same convention as the first-person
//! camera (yaw 0 looks along -Z, increasing clockwise seen from above), so the client can hand it straight to the renderer.

use glam::{Vec2, Vec3};

/// Distance behind the kart at rest, m.
pub const BASE_DISTANCE: f32 = 5.2;
/// Extra distance at top speed, m.
pub const SPEED_DISTANCE: f32 = 2.0;
/// Height of the eye above the kart's feet, m.
pub const HEIGHT: f32 = 2.7;
/// How far ahead of the kart (m) and how high (m) the camera looks.
pub const LOOK_AHEAD: f32 = 3.0;
/// Height of the point the camera looks at, above the kart's feet, m.
pub const LOOK_HEIGHT: f32 = 1.0;
/// Field of view at rest, degrees.
pub const BASE_FOV: f32 = 68.0;
/// Extra field of view at top speed, degrees, and again when boosting.
pub const SPEED_FOV: f32 = 9.0;
/// Extra field of view while boosting, degrees.
pub const BOOST_FOV: f32 = 6.0;
/// How fast the view turns to follow the heading, per second (higher = tighter).
const FOLLOW_RATE: f32 = 5.0;
/// How much of the way towards the travel direction the view leans while drifting (0..1).
const DRIFT_LEAN: f32 = 0.45;
/// How fast the distance and field of view settle, per second.
const SETTLE_RATE: f32 = 4.0;

/// What the camera needs to know about the kart it follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KartView {
    /// Ground position (x, z).
    pub pos: Vec2,
    /// Height of the kart's feet.
    pub foot_y: f32,
    /// Heading, radians (the kart's `PlayerState.yaw`).
    pub heading: f32,
    /// Velocity (x, z), m/s.
    pub velocity: Vec2,
    /// The kart's top speed, m/s (so the pull-back scales to the driver).
    pub top_speed: f32,
    /// Drifting.
    pub drifting: bool,
    /// Boosting.
    pub boosting: bool,
}

/// Where the camera is, in the first-person camera's terms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// Eye position.
    pub eye: Vec3,
    /// Yaw, radians (0 looks along -Z, increasing clockwise seen from above).
    pub yaw: f32,
    /// Pitch, radians (negative looks down).
    pub pitch: f32,
    /// Vertical field of view, degrees.
    pub fov_deg: f32,
}

/// The chase camera's memory between frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChaseCamera {
    yaw: f32,
    distance: f32,
    fov: f32,
    started: bool,
}

impl Default for ChaseCamera {
    fn default() -> Self {
        ChaseCamera { yaw: 0.0, distance: BASE_DISTANCE, fov: BASE_FOV, started: false }
    }
}

/// The shortest signed angle from `a` to `b`, radians, in `-PI..=PI`.
fn angle_delta(a: f32, b: f32) -> f32 {
    let d = (b - a).rem_euclid(std::f32::consts::TAU);
    if d > std::f32::consts::PI {
        d - std::f32::consts::TAU
    } else {
        d
    }
}

impl ChaseCamera {
    /// A camera that will start directly behind the kart on its first update.
    pub fn new() -> ChaseCamera {
        ChaseCamera::default()
    }

    /// The direction the view is currently facing, radians.
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Advances by `dt` seconds towards the kart and returns where the camera is now.
    pub fn update(&mut self, kart: &KartView, dt: f32) -> CameraPose {
        let dt = dt.clamp(0.0, 0.25);
        let speed = kart.velocity.length();
        let speed_frac = (speed / kart.top_speed.max(1.0)).clamp(0.0, 1.3);

        // Where the view wants to face: the heading, leaned towards the direction of travel while drifting (only when actually moving).
        let mut want = kart.heading;
        if kart.drifting && speed > 2.0 {
            let travel = kart.velocity.x.atan2(-kart.velocity.y);
            want += angle_delta(kart.heading, travel) * DRIFT_LEAN;
        }
        if !self.started {
            self.yaw = want;
            self.started = true;
        } else {
            self.yaw += angle_delta(self.yaw, want) * (1.0 - (-FOLLOW_RATE * dt).exp());
        }

        let want_distance = BASE_DISTANCE + SPEED_DISTANCE * speed_frac.min(1.0) + if kart.boosting { 0.8 } else { 0.0 };
        let want_fov = BASE_FOV + SPEED_FOV * speed_frac.min(1.0) + if kart.boosting { BOOST_FOV } else { 0.0 };
        let settle = 1.0 - (-SETTLE_RATE * dt).exp();
        self.distance += (want_distance - self.distance) * settle;
        self.fov += (want_fov - self.fov) * settle;

        let (sin, cos) = self.yaw.sin_cos();
        let forward = Vec2::new(sin, -cos);
        let ground = kart.pos - forward * self.distance;
        let eye = Vec3::new(ground.x, kart.foot_y + HEIGHT, ground.y);
        let target_ground = kart.pos + forward * LOOK_AHEAD;
        let target = Vec3::new(target_ground.x, kart.foot_y + LOOK_HEIGHT, target_ground.y);
        let to = target - eye;
        let yaw = to.x.atan2(-to.z);
        let pitch = to.y.atan2(Vec2::new(to.x, to.z).length());
        CameraPose { eye, yaw, pitch, fov_deg: self.fov }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    fn kart(heading: f32, speed: f32) -> KartView {
        let (sin, cos) = heading.sin_cos();
        KartView { pos: Vec2::new(10.0, -5.0), foot_y: 0.0, heading, velocity: Vec2::new(sin, -cos) * speed, top_speed: 24.0, drifting: false, boosting: false }
    }

    #[test]
    fn it_starts_directly_behind_and_above_looking_down_the_kart() {
        let mut cam = ChaseCamera::new();
        let k = kart(FRAC_PI_2, 0.0); // facing +X
        let pose = cam.update(&k, 1.0 / 60.0);
        assert!((pose.eye.x - (10.0 - BASE_DISTANCE)).abs() < 0.05 && (pose.eye.z + 5.0).abs() < 0.05, "behind (west of) a kart facing east: {:?}", pose.eye);
        assert!((pose.eye.y - HEIGHT).abs() < 1e-4);
        assert!((pose.yaw - FRAC_PI_2).abs() < 0.05, "and looks east: {}", pose.yaw);
        assert!(pose.pitch < 0.0, "down at the kart, not up: {}", pose.pitch);
    }

    #[test]
    fn a_turn_swings_the_view_round_with_a_lag_and_then_catches_up() {
        let mut cam = ChaseCamera::new();
        cam.update(&kart(0.0, 10.0), 0.016);
        let turned = kart(FRAC_PI_2, 10.0);
        let first = cam.update(&turned, 0.016);
        assert!(first.yaw > 0.0 && first.yaw < FRAC_PI_2 * 0.5, "a frame after a 90 degree turn the view has barely moved: {}", first.yaw);
        for _ in 0..90 {
            cam.update(&turned, 0.016);
        }
        assert!((cam.yaw() - FRAC_PI_2).abs() < 0.03, "and within a second and a half it is round: {}", cam.yaw());
    }

    #[test]
    fn it_turns_the_short_way_round_through_the_seam() {
        let mut cam = ChaseCamera::new();
        cam.update(&kart(PI - 0.1, 10.0), 0.016);
        cam.update(&kart(-PI + 0.1, 10.0), 0.016);
        assert!(cam.yaw().abs() > PI - 0.3, "from just under PI to just over -PI is a small step, not a whole turn: {}", cam.yaw());
    }

    #[test]
    fn speed_and_boost_pull_the_camera_back_and_widen_the_view() {
        let settle = |k: KartView| {
            let mut cam = ChaseCamera::new();
            let mut pose = cam.update(&k, 0.016);
            for _ in 0..120 {
                pose = cam.update(&k, 0.016);
            }
            (pose, (pose.eye - Vec3::new(k.pos.x, pose.eye.y, k.pos.y)).length())
        };
        let (slow, slow_d) = settle(kart(0.0, 0.0));
        let (fast, fast_d) = settle(kart(0.0, 24.0));
        let (boost, boost_d) = settle(KartView { boosting: true, ..kart(0.0, 24.0) });
        assert!(fast_d > slow_d + 1.5 && boost_d > fast_d + 0.5, "{slow_d} < {fast_d} < {boost_d}");
        assert!(fast.fov_deg > slow.fov_deg + 6.0 && boost.fov_deg > fast.fov_deg + 4.0, "{} {} {}", slow.fov_deg, fast.fov_deg, boost.fov_deg);
        assert!((slow.fov_deg - BASE_FOV).abs() < 0.01);
    }

    #[test]
    fn a_drift_leans_the_view_towards_where_the_kart_is_really_going() {
        let straight = |drifting: bool| {
            // Heading 0 (north) but sliding towards the east.
            let k = KartView { velocity: Vec2::new(10.0, 0.0), drifting, ..kart(0.0, 0.0) };
            let mut cam = ChaseCamera::new();
            cam.update(&KartView { velocity: Vec2::ZERO, ..k }, 0.016);
            for _ in 0..120 {
                cam.update(&k, 0.016);
            }
            cam.yaw()
        };
        assert!(straight(false).abs() < 0.01, "not drifting: the view follows the heading");
        assert!(straight(true) > 0.5 && straight(true) < FRAC_PI_2, "drifting: it leans towards the slide: {}", straight(true));
    }

    #[test]
    fn it_is_deterministic_and_survives_odd_frame_times() {
        let run = || {
            let mut cam = ChaseCamera::new();
            let mut last = cam.update(&kart(0.3, 12.0), 0.016);
            for dt in [0.0, 0.016, 5.0, 0.001, f32::MIN_POSITIVE, 0.033] {
                last = cam.update(&kart(1.0, 20.0), dt);
            }
            last
        };
        assert_eq!(run(), run());
        let p = run();
        assert!(p.eye.is_finite() && p.yaw.is_finite() && p.pitch.is_finite() && p.fov_deg.is_finite());
    }
}
