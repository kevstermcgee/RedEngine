//! One player's movement for one tick, as a pure function: the *same* code runs in the single-player
//! game, on the authoritative server, and in a networked client's prediction, so they cannot disagree
//! about how a player moves.
//!
//! Input is a [`PlayerInput`] (what the player is asking for this tick, not where they are), so a
//! client can never tell the server a position. The world it moves through is just the static
//! collider list and the ground candidates (`viewer::collect_*`), which the caller builds once.

use crate::player::{step_horizontal_band, vertical_step, BodySpec, Character, CROUCH_SPEED_MULT, FIXED_DT};
use crate::viewer::{Collider2D, GroundCandidates};
use glam::Vec2;

/// Everything about a player the simulation owns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerState {
    /// Planar position (x, z), m.
    pub pos: Vec2,
    /// Height of the feet, m.
    pub foot_y: f32,
    /// Vertical velocity, m/s.
    pub vy: f32,
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
        PlayerState { pos: Vec2::new(x, z), foot_y, vy: 0.0, yaw: yaw_deg.to_radians(), pitch: 0.0, character }
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
}

impl PlayerInput {
    /// Replaces non-finite or out-of-range values (a corrupt or hostile packet) with harmless ones.
    pub fn sanitized(mut self) -> Self {
        self.forward = self.forward.clamp(-1, 1);
        self.strafe = self.strafe.clamp(-1, 1);
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
    let input = input.sanitized();
    state.yaw = input.yaw;
    state.pitch = input.pitch;
    let body: BodySpec = state.character.body();

    // Same convention as `FpsCamera`: forward = (sin yaw, -cos yaw), right = (cos yaw, sin yaw).
    let (sy, cy) = state.yaw.sin_cos();
    let fwd = Vec2::new(sy, -cy);
    let right = Vec2::new(cy, sy);
    let mut dir = fwd * input.forward as f32 + right * input.strafe as f32;

    // Sprinting needs a forward component (no sprinting backward), and crouch always wins.
    let sprinting = input.sprint && input.forward > 0 && !input.crouch && body.sprint_speed > body.walk_speed;
    let mut speed_now = 0.0;
    if dir.length_squared() > 1e-8 {
        dir = dir.normalize();
        speed_now = if input.crouch {
            body.walk_speed * CROUCH_SPEED_MULT
        } else if sprinting {
            body.sprint_speed
        } else {
            body.walk_speed
        };
        state.pos = step_horizontal_band(colliders, state.pos, state.foot_y, dir * speed_now * FIXED_DT, body.radius, body.band_top);
    }
    let (foot_y, vy) = vertical_step(ground, state.pos, state.foot_y, state.vy, input.jump);
    state.foot_y = foot_y;
    state.vy = vy;
    speed_now
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewer::{collect_box_colliders, collect_ground_candidates};
    use std::path::Path;

    fn world() -> (Vec<Collider2D>, GroundCandidates) {
        let scene = crate::load_scene(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
        (collect_box_colliders(&scene), collect_ground_candidates(&scene))
    }

    fn walk(state: &mut PlayerState, forward: i8, ticks: u32, colliders: &[Collider2D], ground: &GroundCandidates) {
        for k in 0..ticks {
            let yaw = state.yaw;
            step_player(state, &PlayerInput { seq: k, forward, yaw, ..Default::default() }, colliders, ground);
        }
    }

    #[test]
    fn walking_moves_at_walk_speed_and_sprint_is_faster() {
        let (c, g) = world();
        let mut s = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human); // facing +X
        walk(&mut s, 1, 60, &c, &g);
        assert!((s.pos.x - (-22.0 + 3.2)).abs() < 0.02, "one second at 3.2 m/s: {}", s.pos.x);
        let mut r = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human);
        for k in 0..30 {
            let yaw = r.yaw;
            step_player(&mut r, &PlayerInput { seq: k, forward: 1, sprint: true, yaw, ..Default::default() }, &c, &g);
        }
        assert!((r.pos.x - (-22.0 + 3.25)).abs() < 0.02, "half a second at 6.5 m/s: {}", r.pos.x);
    }

    #[test]
    fn the_rat_has_its_own_pace_slower_than_a_human_sprint() {
        let (c, g) = world();
        let mut rat = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Rat);
        walk(&mut rat, 1, 30, &c, &g); // half a second
        assert!((rat.pos.x - (-22.0 + crate::player::RAT_SPEED * 0.5)).abs() < 0.03, "{}", rat.pos.x);
        assert!(rat.pos.x - (-22.0) < crate::player::SPRINT_SPEED * 0.5 - 0.5, "well short of a human sprint's half-second distance");
        assert!(rat.pos.x - (-22.0) > crate::player::WALK_SPEED * 0.5 + 0.2, "but brisker than a human's walk");
    }

    #[test]
    fn diagonal_movement_is_not_faster_and_hostile_input_cannot_exceed_full_speed() {
        let (c, g) = world();
        let mut s = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human);
        for k in 0..60 {
            // forward and strafe both "127": sanitised to 1, and diagonals are normalised.
            let yaw = s.yaw;
            step_player(&mut s, &PlayerInput { seq: k, forward: 127, strafe: 127, yaw, ..Default::default() }, &c, &g);
        }
        let moved = (s.pos - Vec2::new(-22.0, -3.0)).length();
        assert!(moved <= 3.2 + 0.02, "moved {moved} m in a second");
        let mut n = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human);
        step_player(&mut n, &PlayerInput { forward: 1, yaw: f32::NAN, pitch: f32::INFINITY, ..Default::default() }, &c, &g);
        assert!(n.pos.is_finite() && n.yaw.is_finite() && n.pitch.is_finite());
    }

    #[test]
    fn walls_stop_a_player_and_the_step_is_deterministic() {
        let (c, g) = world();
        let mut a = PlayerState::spawn(-22.0, -3.0, 0.0, 270.0, Character::Human); // facing -X, into the west wall
        let mut b = a;
        walk(&mut a, 1, 300, &c, &g);
        walk(&mut b, 1, 300, &c, &g);
        assert_eq!(a, b, "same inputs, same result");
        assert!(a.pos.x > -23.9, "stopped by the wall at x = -24: {}", a.pos.x);
    }

    #[test]
    fn the_rat_squeezes_through_a_gap_the_human_cannot() {
        let (c, g) = world();
        let mut rat = PlayerState::spawn(-13.0, -6.5, 0.0, 90.0, Character::Rat);
        walk(&mut rat, 1, 120, &c, &g);
        assert!(rat.pos.x > -11.0, "the rat passes the 0.25 m gap: {}", rat.pos.x);
        let mut human = PlayerState::spawn(-13.0, -6.5, 0.0, 90.0, Character::Human);
        walk(&mut human, 1, 120, &c, &g);
        assert!(human.pos.x < -12.0, "the human does not: {}", human.pos.x);
    }

    #[test]
    fn jumping_leaves_the_ground_and_lands_again() {
        let (c, g) = world();
        let mut s = PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human);
        let yaw = s.yaw;
        step_player(&mut s, &PlayerInput { jump: true, yaw, ..Default::default() }, &c, &g);
        assert!(s.foot_y > 0.0);
        let mut peak = 0.0f32;
        for _ in 0..60 {
            let yaw = s.yaw;
            step_player(&mut s, &PlayerInput { yaw, ..Default::default() }, &c, &g);
            peak = peak.max(s.foot_y);
        }
        assert!(peak > 0.3 && peak < 0.5, "jump apex ~0.4 m: {peak}");
        assert_eq!(s.foot_y, 0.0);
    }
}
