//! One player's movement for one tick, as a pure function: the *same* code runs in the single-player
//! game, on the authoritative server, and in a networked client's prediction, so they cannot disagree
//! about how a player moves.
//!
//! Input is a [`PlayerInput`] (what the player is asking for this tick, not where they are), so a
//! client can never tell the server a position. The world it moves through is just the static
//! collider list and the ground candidates (`collide::collect_*`), which the caller builds once.

use crate::collide::{Collider2D, GroundCandidates};
use crate::player::{step_horizontal_band, vertical_step_on, vertical_step_on_tuned, BodySpec, Character, JumpPad, PlayerTuning, CROUCH_SPEED_MULT, FIXED_DT};
use glam::{Vec2, Vec3};

/// The velocity a carried prop leaves the hand with: the holder's own motion (horizontal `velocity` and vertical `vy`)
/// plus `throw_speed` (the scene's `player.throw_speed`, 1 m/s by default) along the look direction, pitch included. The
/// offline client and the authoritative server both call this, so a throw is the same game everywhere
/// (ADR 2026-09-29-one-release-velocity). Standing still it is a plain toss; sprinting and looking up it is a throw.
pub fn release_velocity(state: &PlayerState, look: Vec3, throw_speed: f32) -> Vec3 {
    Vec3::new(state.velocity.x, state.vy, state.velocity.y) + look.normalize_or_zero() * throw_speed
}

/// Everything about a player the simulation owns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerState {
    /// Planar position (x, z), m.
    pub pos: Vec2,
    /// Height of the feet, m.
    pub foot_y: f32,
    /// Vertical velocity, m/s.
    pub vy: f32,
    /// Horizontal momentum, replicated for exact client reconciliation.
    pub velocity: Vec2,
    /// Look direction, radians (0 = looking along -Z, increasing clockwise seen from above).
    pub yaw: f32,
    /// Look pitch, radians.
    pub pitch: f32,
    /// Which body they have.
    pub character: Character,
}

impl PlayerState {
    /// A player standing at `(x, z)` on the ground facing `yaw_deg`.
    pub fn spawn(x: f32, z: f32, foot_y: f32, yaw_deg: f32, character: Character) -> Self {
        PlayerState { pos: Vec2::new(x, z), foot_y, vy: 0.0, velocity: Vec2::ZERO, yaw: yaw_deg.to_radians(), pitch: 0.0, character }
    }
}

/// One tick of what a player is asking for. Wish directions are `-1 / 0 / 1` on each axis, so no
/// input can ask for more than full speed.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlayerInput {
    /// Client-assigned, increasing by one per tick; the server acknowledges the newest it processed.
    pub seq: u32,
    /// `+1` forward, `-1` back.
    pub forward: i8,
    /// `+1` right, `-1` left.
    pub strafe: i8,
    /// Signed stick axes in -127..127 instead of digital -1..1; carried in flag bit 7.
    pub analog: bool,
    /// Jump this tick (only takes effect when grounded).
    pub jump: bool,
    /// Sprint requested (needs forward, not backward, not crouching).
    pub sprint: bool,
    /// Crouch held.
    pub crouch: bool,
    /// Look yaw, radians.
    pub yaw: f32,
    /// Look pitch, radians.
    pub pitch: f32,
    /// Interact held (`E`): picks up the prop under the crosshair, or drops the carried one. Acts on the press (rising edge).
    pub interact: bool,
    /// Primary action held (left click): swing the bat / fire the firearm in hand. Acts on the press.
    pub attack: bool,
    /// Reload held (`R`). Acts on the press.
    pub reload: bool,
    /// Switch-weapon held (mouse wheel / `Q`). Acts on the press.
    pub switch_weapon: bool,
    /// Aim held (right mouse / left trigger): aim down the sights, or throw a grenade underhand. Loadout matches only.
    pub aim: bool,
    /// Drop the weapon in hand (`G`). Acts on the press. Loadout matches only.
    pub drop: bool,
    /// Choose a weapon directly: `0` nothing, `1` and `2` the firearm slots, `3` melee, `4` grenade (again: the next one), `5` the previous
    /// weapon, `6` next, `7` previous in the cycle. Acts on a change to a new non-zero value. Loadout matches only.
    pub select: u8,
}

impl PlayerInput {
    /// The action buttons as bits for the wire and traces: bit 0 jump, 1 sprint, 2 crouch, 3 interact, 4 attack, 5 reload, 6 switch,
    /// 7 analog, 8 aim, 9 drop, 10-12 the direct weapon choice.
    pub fn flags(&self) -> u16 {
        (self.jump as u16)
            | ((self.sprint as u16) << 1)
            | ((self.crouch as u16) << 2)
            | ((self.interact as u16) << 3)
            | ((self.attack as u16) << 4)
            | ((self.reload as u16) << 5)
            | ((self.switch_weapon as u16) << 6)
            | ((self.analog as u16) << 7)
            | ((self.aim as u16) << 8)
            | ((self.drop as u16) << 9)
            | (((self.select & 7) as u16) << 10)
    }

    /// The inverse of [`flags`](Self::flags) applied to `self` (movement axes and look are left as they are).
    pub fn with_flags(mut self, f: u16) -> Self {
        self.jump = f & 1 != 0;
        self.sprint = f & 2 != 0;
        self.crouch = f & 4 != 0;
        self.interact = f & 8 != 0;
        self.attack = f & 16 != 0;
        self.reload = f & 32 != 0;
        self.switch_weapon = f & 64 != 0;
        self.analog = f & 128 != 0;
        self.aim = f & 256 != 0;
        self.drop = f & 512 != 0;
        self.select = ((f >> 10) & 7) as u8;
        self
    }

    /// Replaces non-finite or out-of-range values (a corrupt or hostile packet) with harmless ones.
    pub fn sanitized(mut self) -> Self {
        let limit = if self.analog { 127 } else { 1 };
        self.forward = self.forward.clamp(-limit, limit);
        self.strafe = self.strafe.clamp(-limit, limit);
        if !self.yaw.is_finite() {
            self.yaw = 0.0;
        }
        if !self.pitch.is_finite() {
            self.pitch = 0.0;
        }
        self.pitch = self.pitch.clamp(-1.56, 1.56);
        self
    }
}

/// Advances `state` by one [`FIXED_DT`] tick under `input` through the static world, returning the
/// horizontal speed it moved at (m/s, 0 when standing still) for animation.
pub fn step_player(state: &mut PlayerState, input: &PlayerInput, colliders: &[Collider2D], ground: &GroundCandidates) -> f32 {
    step_player_on(state, input, colliders, ground, None)
}

/// [`step_player`] with scene-authored human movement and jump pads.
pub fn step_player_tuned(
    state: &mut PlayerState,
    input: &PlayerInput,
    colliders: &[Collider2D],
    ground: &GroundCandidates,
    tuning: PlayerTuning,
    jump_pads: &[JumpPad],
) -> f32 {
    step_player_on_tuned(state, input, colliders, ground, None, tuning, jump_pads)
}

/// [`step_player`] with an optional extra floor under the feet (single-player: the top of a loose prop the player stands on;
/// the server and prediction pass `None`, so online you still cannot stand on a crate).
pub fn step_player_on(state: &mut PlayerState, input: &PlayerInput, colliders: &[Collider2D], ground: &GroundCandidates, extra_floor: Option<f32>) -> f32 {
    step_player_on_tuned(state, input, colliders, ground, extra_floor, PlayerTuning::default(), &[])
}

/// [`step_player_on`] with scene-authored human movement and jump pads.
pub fn step_player_on_tuned(
    state: &mut PlayerState,
    input: &PlayerInput,
    colliders: &[Collider2D],
    ground: &GroundCandidates,
    extra_floor: Option<f32>,
    tuning: PlayerTuning,
    jump_pads: &[JumpPad],
) -> f32 {
    let input = input.sanitized();
    state.yaw = input.yaw;
    state.pitch = input.pitch;
    let body: BodySpec = state.character.body();

    // Same convention as `FpsCamera`: forward = (sin yaw, -cos yaw), right = (cos yaw, sin yaw).
    let (sy, cy) = libm::sincosf(state.yaw);
    let fwd = Vec2::new(sy, -cy);
    let right = Vec2::new(cy, sy);
    let mut dir = fwd * input.forward as f32 + right * input.strafe as f32;
    let strength = if input.analog { (dir.length() / 127.0).min(1.0) } else { 1.0 };

    // Sprinting needs a forward component (no sprinting backward), and crouch always wins.
    let sprinting = input.sprint && input.forward > 0 && !input.crouch && body.sprint_speed > body.walk_speed;
    let mut speed_now = 0.0;
    if dir.length_squared() > 1e-8 {
        dir = dir.normalize();
        let (walk_speed, sprint_speed, crouch_multiplier) = if state.character != Character::Rat {
            (tuning.walk_speed, tuning.sprint_speed, tuning.crouch_multiplier)
        } else {
            (body.walk_speed, body.sprint_speed, CROUCH_SPEED_MULT)
        };
        speed_now = if input.crouch {
            walk_speed * crouch_multiplier
        } else if sprinting {
            sprint_speed
        } else {
            walk_speed
        } * strength;
    }
    if tuning.acceleration > 0.0 && state.character != Character::Rat {
        let floor = crate::collide::ground_height_at(ground, state.pos, state.foot_y).max(extra_floor.unwrap_or(f32::NEG_INFINITY));
        let grounded = state.vy <= 0.0 && (state.foot_y - floor).abs() < 0.05;
        if grounded {
            let speed = state.velocity.length();
            if speed > 0.0 {
                state.velocity *= ((speed - speed.max(1.0) * tuning.friction * FIXED_DT).max(0.0)) / speed;
            }
        }
        if speed_now > 0.0 {
            let acceleration = if grounded { tuning.acceleration } else { tuning.air_acceleration };
            let add = (speed_now - state.velocity.dot(dir)).max(0.0);
            state.velocity += dir * add.min(acceleration * speed_now * FIXED_DT);
        }
        state.velocity = state.velocity.clamp_length_max(tuning.max_speed);
    } else {
        state.velocity = dir * speed_now;
    }
    let before = state.pos;
    let intended = state.velocity * FIXED_DT;
    if intended.length_squared() > 0.0 {
        // In a generated world the trees near the player are walls too (looked up per tick, from the generator).
        state.pos = match ground.procgen() {
            Some(world) => step_horizontal_band(
                &world.colliders_near(state.pos, 2.0 + intended.length(), colliders),
                state.pos,
                state.foot_y,
                intended,
                body.radius,
                body.band_top,
            ),
            None => step_horizontal_band(colliders, state.pos, state.foot_y, intended, body.radius, body.band_top),
        };
    }
    let actual = state.pos - before;
    // Remove blocked components instead of banking velocity into a wall.
    for axis in 0..2 {
        if (actual[axis] - intended[axis]).abs() > 0.001 {
            state.velocity[axis] = actual[axis] / FIXED_DT;
        }
    }
    // The edge of the world: the walkable limits stop the player (and the momentum into them), and a looping axis carries them across
    // the seam. This is the one place the loop and the limits are applied, so the single-player game, the authoritative server and a
    // client's prediction cannot disagree about where the world ends.
    let mut actual = actual;
    if !tuning.expanse.is_plain() {
        let (bounded, hit) = tuning.expanse.clamp_bounds(state.pos);
        for axis in 0..2 {
            if hit[axis] {
                state.velocity[axis] = 0.0;
                actual[axis] = bounded[axis] - before[axis];
            }
        }
        state.pos = bounded;
    }
    if tuning.acceleration > 0.0 {
        speed_now = actual.length() / FIXED_DT;
    }
    let (foot_y, mut vy) = if state.character != Character::Rat {
        vertical_step_on_tuned(ground, extra_floor, state.pos, state.foot_y, state.vy, input.jump, tuning.jump_speed, tuning.gravity)
    } else {
        vertical_step_on(ground, extra_floor, state.pos, state.foot_y, state.vy, input.jump)
    };
    if state.character != Character::Rat && state.vy <= 0.0 {
        if let Some(pad) = jump_pads.iter().find(|pad| pad.touches(state.pos, foot_y)) {
            vy = pad.launch_speed;
        }
    }
    state.foot_y = foot_y;
    state.vy = vy;
    // Wrap last: the vertical step above looked the ground up at the pre-wrap position, which is the same ground (it is periodic).
    state.pos = tuning.expanse.wrap_pos(state.pos);
    speed_now
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_wrapped_axis_carries_the_player_across_and_bounds_stop_them() {
        use crate::expanse::{Axis, Bounds, Expanse, Wrap};
        let expanse = Expanse { wrap: Some(Wrap { axis: Axis::Z, min: -100.0, max: 100.0 }), bounds: Bounds { x: Some((-10.0, 10.0)), z: None } };
        let tuning = PlayerTuning { expanse, acceleration: 12.0, walk_speed: 9.0, sprint_speed: 9.0, max_speed: 20.0, ..Default::default() };
        let ground = GroundCandidates::default();
        // Walk +Z (yaw 180 degrees looks along +Z) for 30 s: 270 m is more than one loop, and the position never leaves the period.
        let mut s = PlayerState::spawn(0.0, 90.0, 0.0, 180.0, Character::Human);
        let mut crossed = false;
        for k in 0..1800 {
            let before = s.pos.y;
            step_player_on_tuned(&mut s, &PlayerInput { seq: k, forward: 1, yaw: 180f32.to_radians(), ..Default::default() }, &[], &ground, None, tuning, &[]);
            assert!((-100.0..100.0).contains(&s.pos.y), "z stays in the period: {}", s.pos.y);
            crossed |= s.pos.y < before - 100.0;
        }
        assert!(crossed, "the player crossed the seam");
        assert!(s.velocity.length() > 8.0, "and kept their speed through it: {}", s.velocity.length());
        // Walk +X into the limit: stopped at 10, no momentum banked into the invisible wall.
        let mut w = PlayerState::spawn(0.0, 0.0, 0.0, 90.0, Character::Human);
        for k in 0..600 {
            step_player_on_tuned(&mut w, &PlayerInput { seq: k, forward: 1, yaw: 90f32.to_radians(), ..Default::default() }, &[], &ground, None, tuning, &[]);
        }
        assert!((w.pos.x - 10.0).abs() < 1e-4 && w.velocity.x.abs() < 1e-4, "{:?} {:?}", w.pos, w.velocity);
        // Identical inputs, identical result (server and prediction must agree bit for bit).
        let (mut a, mut b) = (PlayerState::spawn(0.0, 99.0, 0.0, 180.0, Character::Human), PlayerState::spawn(0.0, 99.0, 0.0, 180.0, Character::Human));
        for k in 0..200 {
            let input = PlayerInput { seq: k, forward: 1, strafe: (k % 3) as i8 - 1, yaw: 180f32.to_radians(), ..Default::default() };
            step_player_on_tuned(&mut a, &input, &[], &ground, None, tuning, &[]);
            step_player_on_tuned(&mut b, &input, &[], &ground, None, tuning, &[]);
        }
        assert_eq!(a, b);
    }

    use super::*;

    #[test]
    fn arena_momentum_survives_airborne_release_and_ground_friction_stops_it() {
        let ground = GroundCandidates::default();
        let tuning = PlayerTuning { acceleration: 12.0, air_acceleration: 2.0, walk_speed: 9.0, sprint_speed: 9.0, ..Default::default() };
        let mut state = PlayerState::spawn(0.0, 0.0, 0.0, 0.0, Character::Human);
        for _ in 0..60 {
            step_player_on_tuned(&mut state, &PlayerInput { forward: 1, ..Default::default() }, &[], &ground, None, tuning, &[]);
        }
        assert!((state.velocity.length() - 9.0).abs() < 0.01);
        step_player_on_tuned(&mut state, &PlayerInput { jump: true, ..Default::default() }, &[], &ground, None, tuning, &[]);
        let airborne_speed = state.velocity.length();
        for _ in 0..10 {
            step_player_on_tuned(&mut state, &PlayerInput::default(), &[], &ground, None, tuning, &[]);
        }
        assert!((state.velocity.length() - airborne_speed).abs() < 0.01);
        for _ in 0..180 {
            step_player_on_tuned(&mut state, &PlayerInput::default(), &[], &ground, None, tuning, &[]);
        }
        assert!(state.velocity.length() < 0.01);
    }

    #[test]
    fn air_strafing_adds_bounded_momentum() {
        let ground = GroundCandidates::default();
        let tuning = PlayerTuning { acceleration: 12.0, air_acceleration: 2.0, max_speed: 14.0, walk_speed: 9.0, ..Default::default() };
        let mut state = PlayerState::spawn(0.0, 0.0, 100.0, 0.0, Character::Human);
        state.velocity = Vec2::new(0.0, -9.0);
        for _ in 0..30 {
            step_player_on_tuned(&mut state, &PlayerInput { strafe: 1, ..Default::default() }, &[], &ground, None, tuning, &[]);
        }
        assert!(state.velocity.x > 1.0 && state.velocity.y < -8.9);
        assert!(state.velocity.length() > 9.0 && state.velocity.length() <= 14.001);
    }

    #[test]
    fn a_released_prop_inherits_the_holders_motion_plus_a_throw_along_the_look() {
        use super::{release_velocity, PlayerState};
        use glam::{Vec2, Vec3};
        let mut state = PlayerState::spawn(0.0, 0.0, 0.0, 90.0, crate::player::Character::Human);
        let level = Vec3::X;
        // Standing still: a plain toss of `throw_speed` along the look.
        assert_eq!(release_velocity(&state, level, 1.0), Vec3::new(1.0, 0.0, 0.0));
        // Sprinting at 8 m/s: the prop keeps that momentum, plus the toss.
        state.velocity = Vec2::new(8.0, 0.0);
        assert_eq!(release_velocity(&state, level, 1.0), Vec3::new(9.0, 0.0, 0.0));
        // Jumping adds the vertical velocity; looking up 45 degrees at throw speed 6 gives a real upward throw.
        state.vy = 2.0;
        let up45 = Vec3::new(0.5f32.sqrt(), 0.5f32.sqrt(), 0.0);
        let v = release_velocity(&state, up45, 6.0);
        assert!((v.x - (8.0 + 6.0 * 0.5f32.sqrt())).abs() < 1e-5 && (v.y - (2.0 + 6.0 * 0.5f32.sqrt())).abs() < 1e-5 && v.z == 0.0, "{v}");
        // The look direction is normalised, so a long vector is not a stronger throw.
        assert_eq!(release_velocity(&state, level * 50.0, 1.0), release_velocity(&state, level, 1.0));
    }
}
