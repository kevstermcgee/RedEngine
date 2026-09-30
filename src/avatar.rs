//! How other players look and move: the walk cycle, the gun arm raised and following their aim, the bat's swing, the recoil and muzzle flash of
//! a shot, and the fall when they die. Pure (no window, no GPU): [`animate`] poses a pooled avatar [`Object`] from a remote player's
//! interpolated pose and says where the weapon in their hand is ([`RemoteHand`]) for the renderer to draw.
//!
//! The poses are the same ones the local player's own body uses in third person (`bin/re2/avatar.rs` takes its constants from here), so
//! everyone holds a weapon the same way.

use crate::characters::HUMAN_HEIGHT;
use crate::easing::Ease;
use crate::net::interp::PlayerPose;
use crate::player::Character;
use crate::schema::{Object, ObjectKind};
use crate::skeleton::{pose_to_parts, HumanoidRig, PoseSample};
use crate::track::Track;
use crate::weapons::{Weapon, MUZZLE_FLASH_TIME, RECOIL_TIME, SWING_RECOVER_SECS, SWING_STRIKE_SECS, SWING_WINDUP_SECS};
use glam::{Mat4, Quat, Vec3};

/// Walk cycles a second at the human walk speed (faster and slower scale it).
pub const WALK_CYCLES_PER_SEC_AT_WALK_SPEED: f32 = 1.6;
/// Hip swing at full stride, degrees.
pub const HIP_SWING_DEG: f32 = 28.0;
/// How far a knee lifts on the forward stroke, degrees.
pub const KNEE_LIFT_DEG: f32 = 45.0;
/// Knee bend at rest, degrees.
pub const KNEE_REST_DEG: f32 = 4.0;
/// Arm swing at full stride, degrees.
pub const SHOULDER_SWING_DEG: f32 = 20.0;
/// Idle sway of the spine, degrees.
pub const IDLE_SWAY_DEG: f32 = 1.4;
/// Cheddar's gait phase advances this many radians per metre travelled.
pub const RAT_GAIT_RAD_PER_M: f32 = 5.0;
/// Shoulder angle that raises the gun arm level (the pitch of the aim is subtracted from it), degrees.
pub const AIM_SHOULDER_X: f32 = -84.0;
/// Elbow bend of the gun arm, degrees (a shot kicks it further).
pub const AIM_ELBOW_DEG: f32 = 6.0;
/// The bat's pitch at the top of the wind-up, degrees.
pub const WINDUP_PITCH_DEG: f32 = -128.0;
/// The bat's pitch at the end of the strike, degrees.
pub const STRIKE_PITCH_DEG: f32 = 42.0;
/// The bat's pitch as carried, degrees.
pub const BAT_IDLE_PITCH_DEG: f32 = -66.0;
/// Shoulder angle at the top of a swing's wind-up, degrees.
pub const ARM_WINDUP_SHOULDER_X: f32 = 55.0;
/// Shoulder angle at the end of a swing's strike, degrees.
pub const ARM_STRIKE_SHOULDER_X: f32 = -95.0;
/// Elbow bend of the bat arm at rest, degrees.
pub const ARM_IDLE_ELBOW_DEG: f32 = 8.0;
/// Elbow bend at the top of the wind-up, degrees.
pub const ARM_WINDUP_ELBOW_DEG: f32 = 60.0;
/// Elbow bend at the end of the strike, degrees.
pub const ARM_STRIKE_ELBOW_DEG: f32 = 12.0;
/// The forearm bone (`skeleton::pose_to_parts` part index) a weapon is held in: the rig's "left" arm, the character's right hand.
pub const HAND_FOREARM_PART: usize = 3;
/// Seconds a body takes to fall when it dies.
pub const FALL_SECS: f32 = 0.45;
/// Seconds a body takes to stand again when it respawns.
pub const RISE_SECS: f32 = 0.12;

/// The bat's swing at `elapsed` seconds in (`None` = not swinging: `idle`): blends between the wind-up, strike and recovery values.
pub fn swing_blend(elapsed: Option<f32>, idle: f32, windup: f32, strike: f32) -> f32 {
    match elapsed {
        None => idle,
        Some(t) if t < SWING_WINDUP_SECS => idle + (windup - idle) * Ease::Out.apply(t / SWING_WINDUP_SECS),
        Some(t) if t < SWING_WINDUP_SECS + SWING_STRIKE_SECS => windup + (strike - windup) * Ease::In.apply((t - SWING_WINDUP_SECS) / SWING_STRIKE_SECS),
        Some(t) => strike + (idle - strike) * Ease::Out.apply(((t - SWING_WINDUP_SECS - SWING_STRIKE_SECS) / SWING_RECOVER_SECS).min(1.0)),
    }
}

/// Recoil kick right after a shot: 1 at the shot, easing to 0 over [`RECOIL_TIME`].
pub fn recoil_kick(since_shot: f32) -> f32 {
    let f = (since_shot / RECOIL_TIME).clamp(0.0, 1.0);
    (1.0 - f) * (1.0 - f)
}

/// Where a remote player's weapon is and how it is held; the renderer builds the weapon's transform from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RemoteHand {
    /// The weapon in hand.
    pub weapon: Weapon,
    /// The wrist, world space.
    pub wrist: Vec3,
    /// The player's look yaw, radians.
    pub yaw: f32,
    /// The player's look pitch, radians.
    pub pitch: f32,
    /// The bat's pitch, degrees (the swing), when the weapon is the bat.
    pub bat_pitch_deg: f32,
    /// Recoil kick, 1 at a shot easing to 0.
    pub kick: f32,
    /// Muzzle flash brightness, 1 at a shot fading to 0.
    pub flash: f32,
}

/// What an avatar remembers between frames.
#[derive(Debug, Clone, Default)]
pub struct AvatarAnim {
    /// Walk cycle phase, radians.
    pub phase: f32,
    /// `0` standing, `1` lying where it died.
    pub fall: f32,
    /// Seconds into a bat swing.
    pub swing: Option<f32>,
    swinging_before: bool,
    /// Seconds since the last shot (`None` until the first one was seen: nothing to show yet).
    since_shot: Option<f32>,
    last_shots: Option<u8>,
}

impl AvatarAnim {
    /// The muzzle flash brightness right now, 0..1.
    pub fn flash(&self) -> f32 {
        self.since_shot.map_or(0.0, |t| (1.0 - t / MUZZLE_FLASH_TIME).clamp(0.0, 1.0))
    }
}

/// The euler angles (degrees, the order `Object::rotation` uses) of turning by `yaw_deg` about Y after leaning `lean` radians backward.
fn fallen_rotation(yaw_deg: f32, lean: f32) -> Vec3 {
    let q = Quat::from_rotation_y(yaw_deg.to_radians()) * Quat::from_rotation_x(-lean);
    let (x, y, z) = q.to_euler(glam::EulerRot::XYZ);
    Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
}

/// Poses the avatar object `o` (a pooled humanoid or rat wearing `who`) for the remote player `p`: position and facing, the walk cycle, the
/// gun arm or the bat's swing, and the fall of a dead body; `a` carries the animation state and `dt` is the frame time. Returns where the
/// weapon in their hand is (`None` for a rat, and for the dead).
pub fn animate(o: &mut Object, who: Character, p: &PlayerPose, a: &mut AvatarAnim, dt: f32, idle_t: f32) -> Option<RemoteHand> {
    // Shots: the counter moving is a shot. The first sighting is only a baseline.
    if a.last_shots.is_some_and(|last| last != p.shots) {
        a.since_shot = Some(0.0);
    }
    a.last_shots = Some(p.shots);
    if let Some(t) = &mut a.since_shot {
        *t += dt;
    }
    // Swing: the flag rising starts a swing; it runs its own clock.
    if p.swinging && !a.swinging_before {
        a.swing = Some(0.0);
    }
    a.swinging_before = p.swinging;
    if let Some(t) = &mut a.swing {
        *t += dt;
        if *t > SWING_WINDUP_SECS + SWING_STRIKE_SECS + SWING_RECOVER_SECS {
            a.swing = None;
        }
    }
    // Falling and getting back up.
    let step = dt / if p.dead { FALL_SECS } else { RISE_SECS };
    a.fall = if p.dead { (a.fall + step).min(1.0) } else { (a.fall - step).max(0.0) };
    let lean = Ease::Out.apply(a.fall) * std::f32::consts::FRAC_PI_2;

    let yaw_deg = 180.0 - p.yaw.to_degrees();
    o.position = Track::constant(p.pos + Vec3::Y * 0.18 * a.fall);
    o.rotation = Track::constant(if a.fall > 0.0 { fallen_rotation(yaw_deg, lean) } else { Vec3::new(0.0, yaw_deg, 0.0) });
    o.scale = Track::constant(Vec3::ONE);
    match (&mut o.kind, who) {
        (ObjectKind::Humanoid(h), who) if who != Character::Rat => {
            let walk = Character::Human.body().walk_speed;
            let moving = p.speed > 0.05 && !p.dead;
            let (spine_x, l_hip, r_hip, l_knee, r_knee, l_sh, r_sh) = if moving {
                a.phase += dt * p.speed * (WALK_CYCLES_PER_SEC_AT_WALK_SPEED / walk) * std::f32::consts::TAU;
                let ph = a.phase;
                (
                    3.0 * (ph * 2.0).sin(),
                    HIP_SWING_DEG * ph.sin(),
                    -HIP_SWING_DEG * ph.sin(),
                    KNEE_REST_DEG + (KNEE_LIFT_DEG * (-ph).sin()).max(0.0),
                    KNEE_REST_DEG + (KNEE_LIFT_DEG * ph.sin()).max(0.0),
                    -SHOULDER_SWING_DEG * ph.sin(),
                    SHOULDER_SWING_DEG * ph.sin(),
                )
            } else {
                (IDLE_SWAY_DEG * (idle_t * 1.1).sin(), 0.0, 0.0, KNEE_REST_DEG, KNEE_REST_DEG, 0.0, 0.0)
            };
            let weapon = Weapon::from_wire(p.weapon);
            let aiming = !p.dead && weapon.is_firearm();
            let kick = a.since_shot.map_or(0.0, recoil_kick);
            let (l_sh_x, l_elbow) = if p.dead {
                (10.0, 20.0)
            } else if aiming {
                ((AIM_SHOULDER_X - p.pitch.to_degrees() + 14.0 * kick).clamp(-175.0, -20.0), AIM_ELBOW_DEG + 22.0 * kick)
            } else {
                (
                    swing_blend(a.swing, l_sh, ARM_WINDUP_SHOULDER_X, ARM_STRIKE_SHOULDER_X),
                    swing_blend(a.swing, ARM_IDLE_ELBOW_DEG, ARM_WINDUP_ELBOW_DEG, ARM_STRIKE_ELBOW_DEG),
                )
            };
            let head = Vec3::new(p.pitch.to_degrees().clamp(-60.0, 60.0) * -0.5, 0.0, 0.0);
            let l_shoulder = Vec3::new(l_sh_x, 0.0, -6.0);
            let r_shoulder = Vec3::new(r_sh, 0.0, 6.0);
            h.pose.spine = Track::constant(Vec3::new(spine_x, 0.0, 0.0));
            h.pose.head = Track::constant(head);
            h.pose.l_hip = Track::constant(Vec3::new(l_hip, 0.0, 0.0));
            h.pose.r_hip = Track::constant(Vec3::new(r_hip, 0.0, 0.0));
            h.pose.l_knee = Track::constant(l_knee);
            h.pose.r_knee = Track::constant(r_knee);
            h.pose.l_shoulder = Track::constant(l_shoulder);
            h.pose.r_shoulder = Track::constant(r_shoulder);
            h.pose.l_elbow = Track::constant(l_elbow);
            h.pose.r_elbow = Track::constant(KNEE_REST_DEG);
            if p.dead {
                return None;
            }
            // Weld the weapon to the gun arm: solve the same forward kinematics the renderer uses and take the wrist.
            let rig = HumanoidRig::new(HUMAN_HEIGHT, 1.0);
            let sample = PoseSample {
                spine: Vec3::new(spine_x, 0.0, 0.0),
                head: Vec3::ZERO,
                l_shoulder,
                r_shoulder,
                l_elbow,
                r_elbow: KNEE_REST_DEG,
                l_hip: Vec3::new(l_hip, 0.0, 0.0),
                r_hip: Vec3::new(r_hip, 0.0, 0.0),
                l_knee,
                r_knee,
            };
            let forearm = &pose_to_parts(&rig, &sample)[HAND_FOREARM_PART];
            let body_world = Mat4::from_rotation_translation(Quat::from_rotation_y(yaw_deg.to_radians()), p.pos);
            let wrist = (body_world
                * Mat4::from_rotation_translation(forearm.rotation, forearm.center)
                * Mat4::from_translation(Vec3::new(0.0, forearm.length * 0.5, 0.0)))
            .transform_point3(Vec3::ZERO);
            Some(RemoteHand {
                weapon,
                wrist,
                yaw: p.yaw,
                pitch: p.pitch,
                bat_pitch_deg: swing_blend(a.swing, BAT_IDLE_PITCH_DEG, WINDUP_PITCH_DEG, STRIKE_PITCH_DEG),
                kick,
                flash: a.flash(),
            })
        }
        (ObjectKind::Rat(r), Character::Rat) => {
            a.phase += dt * p.speed * RAT_GAIT_RAD_PER_M;
            let stride = (p.speed / Character::Rat.body().sprint_speed).clamp(0.0, 1.0);
            r.gait = Track::constant(a.phase);
            r.stride = Track::constant(stride);
            r.sway = Track::constant(idle_t * 1.3);
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::characters::character_object;

    fn pose(x: f32, dead: bool, weapon: Weapon, shots: u8, swinging: bool) -> PlayerPose {
        PlayerPose {
            pos: Vec3::new(x, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            speed: 0.0,
            character: 0,
            crouching: false,
            swinging,
            dead,
            weapon: weapon.wire(),
            held: crate::net::protocol::NO_PROP,
            hp: 100,
            shots,
            protected: false,
            kart: None,
        }
    }

    fn human() -> Object {
        character_object(Character::Human, "net_human_0")
    }

    #[test]
    fn a_player_holding_a_gun_holds_it_out_and_the_wrist_follows_the_body() {
        let (mut o, mut a) = (human(), AvatarAnim::default());
        let h = animate(&mut o, Character::Human, &pose(0.0, false, Weapon::Rifle, 0, false), &mut a, 0.016, 0.0).expect("a hand");
        assert_eq!(h.weapon, Weapon::Rifle);
        assert!(h.wrist.y > 0.9 && h.wrist.y < 1.6, "the wrist is about chest height: {:?}", h.wrist);
        // Facing yaw 0 is -Z: the gun arm reaches forward, so the wrist is in front of the body's centre line.
        assert!(h.wrist.z < -0.2, "the gun arm points ahead: {:?}", h.wrist);
        let h2 = animate(&mut o, Character::Human, &pose(5.0, false, Weapon::Rifle, 0, false), &mut a, 0.016, 0.0).unwrap();
        assert!((h2.wrist.x - h.wrist.x - 5.0).abs() < 0.05, "it moves with the player: {:?} -> {:?}", h.wrist, h2.wrist);
        // Turned to face +X (yaw 90 degrees) the arm swings round with the body.
        let mut east = pose(0.0, false, Weapon::Rifle, 0, false);
        east.yaw = std::f32::consts::FRAC_PI_2;
        let he = animate(&mut o, Character::Human, &east, &mut a, 0.016, 0.0).unwrap();
        assert!(he.wrist.x > 0.2 && he.wrist.z.abs() < 0.3, "the wrist is ahead of a player facing east: {:?}", he.wrist);
    }

    #[test]
    fn a_shot_kicks_the_gun_and_flashes_once_then_settles() {
        let (mut o, mut a) = (human(), AvatarAnim::default());
        let idle = animate(&mut o, Character::Human, &pose(0.0, false, Weapon::Smg, 7, false), &mut a, 0.016, 0.0).unwrap();
        assert_eq!((idle.flash, idle.kick), (0.0, 0.0), "the first sighting of the counter is a baseline, not a shot");
        let shot = animate(&mut o, Character::Human, &pose(0.0, false, Weapon::Smg, 8, false), &mut a, 0.016, 0.0).unwrap();
        assert!(shot.flash > 0.7 && shot.kick > 0.8, "{shot:?}");
        let later = animate(&mut o, Character::Human, &pose(0.0, false, Weapon::Smg, 8, false), &mut a, 0.5, 0.0).unwrap();
        assert_eq!((later.flash, later.kick), (0.0, 0.0), "flash and recoil are over half a second later");
        // Ten shots between two snapshots are one flash, not a crash.
        let burst = animate(&mut o, Character::Human, &pose(0.0, false, Weapon::Smg, 18, false), &mut a, 0.016, 0.0).unwrap();
        assert!(burst.flash > 0.7);
    }

    #[test]
    fn the_bat_swings_through_its_arc_when_the_swing_flag_rises() {
        let (mut o, mut a) = (human(), AvatarAnim::default());
        let idle = animate(&mut o, Character::Human, &pose(0.0, false, Weapon::Bat, 0, false), &mut a, 0.016, 0.0).unwrap();
        assert_eq!(idle.bat_pitch_deg, BAT_IDLE_PITCH_DEG);
        let mut seen = Vec::new();
        for _ in 0..30 {
            let h = animate(&mut o, Character::Human, &pose(0.0, false, Weapon::Bat, 0, true), &mut a, 0.016, 0.0).unwrap();
            seen.push(h.bat_pitch_deg);
        }
        assert!(seen.iter().any(|p| *p < BAT_IDLE_PITCH_DEG - 30.0), "the bat is raised in the wind-up: {seen:?}");
        assert!(seen.iter().any(|p| *p > 0.0), "and comes down through the strike");
        // The flag staying up does not restart the swing: it ends after 0.36 s (22 frames) and rests.
        assert!((seen[29] - BAT_IDLE_PITCH_DEG).abs() < 5.0, "settled: {}", seen[29]);
        assert!(swing_blend(None, 1.0, 2.0, 3.0) == 1.0);
    }

    #[test]
    fn the_dead_fall_over_and_lose_their_weapon_then_stand_when_they_respawn() {
        let (mut o, mut a) = (human(), AvatarAnim::default());
        assert!(animate(&mut o, Character::Human, &pose(0.0, true, Weapon::Rifle, 0, false), &mut a, 0.016, 0.0).is_none(), "a dead player holds nothing");
        for _ in 0..40 {
            animate(&mut o, Character::Human, &pose(0.0, true, Weapon::Rifle, 0, false), &mut a, 0.016, 0.0);
        }
        assert_eq!(a.fall, 1.0);
        // Lying down: the body's up axis is horizontal, and it rests a little above the floor.
        let r = o.rotation.sample(0.0);
        let q = Quat::from_euler(glam::EulerRot::XYZ, r.x.to_radians(), r.y.to_radians(), r.z.to_radians());
        assert!((q * Vec3::Y).y.abs() < 0.02, "lying flat: up is {:?}", q * Vec3::Y);
        assert!(o.position.sample(0.0).y > 0.1);
        for _ in 0..10 {
            animate(&mut o, Character::Human, &pose(0.0, false, Weapon::Rifle, 0, false), &mut a, 0.016, 0.0);
        }
        assert_eq!(a.fall, 0.0, "back on their feet");
        let r = o.rotation.sample(0.0);
        assert_eq!((r.x, r.z), (0.0, 0.0), "upright again");
    }

    #[test]
    fn walking_cycles_the_legs_and_a_rat_gets_no_weapon() {
        let (mut o, mut a) = (human(), AvatarAnim::default());
        let mut moving = pose(0.0, false, Weapon::Bat, 0, false);
        moving.speed = 6.0;
        for _ in 0..30 {
            animate(&mut o, Character::Human, &moving, &mut a, 0.016, 0.0);
        }
        assert!(a.phase > 1.0, "the cycle advanced with distance: {}", a.phase);
        let mut rat = character_object(Character::Rat, "net_rat_0");
        assert!(animate(&mut rat, Character::Rat, &moving, &mut AvatarAnim::default(), 0.016, 0.0).is_none());
    }
}
