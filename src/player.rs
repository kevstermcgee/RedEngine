//! Player-movement constants and the small physics helpers shared by the live viewer (`re2`)
//! and the offline map-analysis tools (`red_engine2 reach` / `lint` / `plan`).
//!
//! Keeping these in one place is the whole point: the analysis tools answer "can the player
//! actually walk from A to B?" by running the *same* collision/ground-height code the game runs
//! every physics tick, so a map that passes `lint` is playable, not just plausible.

use crate::viewer::{colliders_on_floor, ground_height_at, resolve_collision, Collider2D, GroundCandidates};
use glam::Vec2;

/// Movement/collision/gravity run at this fixed timestep (`sim::clock::TICK_DT`, 60 Hz) in the live viewer.
pub const FIXED_DT: f32 = crate::sim::clock::TICK_DT;

/// Walking speed, m/s.
pub const WALK_SPEED: f32 = 3.2;
/// Sprint speed, m/s.
pub const SPRINT_SPEED: f32 = 6.5;
/// Multiplier applied to speed while crouching.
pub const CROUCH_SPEED_MULT: f32 = 0.5;

/// Radius of the player's collision circle. A doorway must be at least `2 * PLAYER_RADIUS`
/// wide to be walkable; `lint` warns below `MIN_COMFORTABLE_DOOR_WIDTH`.
pub const PLAYER_RADIUS: f32 = 0.35;
/// Doors narrower than this draw a `door` lint warning (the hard minimum is `2 * PLAYER_RADIUS`).
pub const MIN_COMFORTABLE_DOOR_WIDTH: f32 = 0.9;

/// Eye height above the feet when standing, m.
pub const STAND_EYE_HEIGHT: f32 = 1.7;
/// Eye height above the feet when crouched, m.
pub const CROUCH_EYE_HEIGHT: f32 = 1.05;

/// Arcade-ish gravity and jump: apex height = JUMP_SPEED^2 / (2 * GRAVITY) ~= 0.4m.
pub const GRAVITY: f32 = 18.0;
/// Initial upward speed of a jump, m/s.
pub const JUMP_SPEED: f32 = 3.8;
/// Height of a jump's apex, m (derived from `JUMP_SPEED` and `GRAVITY`).
pub const JUMP_HEIGHT: f32 = JUMP_SPEED * JUMP_SPEED / (2.0 * GRAVITY);

/// Headroom a player needs above their feet. The collision body band is 2.0 m tall
/// (`viewer::PLAYER_BAND_MAX_Y`), so anything whose underside is *within* 2.0 m of the floor —
/// a door header, a low beam — blocks walking. Ceilings/headers lower than this above walkable
/// floor are flagged by `lint`, and `wall` refuses door openings shorter than it.
pub const PLAYER_HEADROOM: f32 = 2.05;

/// Applies one movement delta the way the live viewer does: X first, then Z, each followed by a
/// push-out against every collider active at the player's current foot height.
pub fn step_horizontal(colliders: &[Collider2D], pos: Vec2, foot_y: f32, delta: Vec2) -> Vec2 {
    step_horizontal_r(colliders, pos, foot_y, delta, PLAYER_RADIUS)
}

/// [`step_horizontal`] for a body with a different collision `radius` (Cheddar the rat is far
/// narrower than a human, so he fits through gaps a person cannot).
pub fn step_horizontal_r(colliders: &[Collider2D], pos: Vec2, foot_y: f32, delta: Vec2, radius: f32) -> Vec2 {
    let active = colliders_on_floor(colliders, foot_y);
    let mut p = pos;
    p.x += delta.x;
    p = resolve_collision(p, radius, &active);
    p.y += delta.y;
    resolve_collision(p, radius, &active)
}

/// Who the player is. Chosen on the launch screen; each character has its own body numbers
/// ([`BodySpec`]) and model (`crate::characters`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Character {
    /// The ordinary-looking person with a bat.
    Human,
    /// Cheddar, the small brownish-grey lab rat.
    Rat,
}

/// Everything about a body that differs between characters. Gravity and the ground/step rules are
/// shared (see [`vertical_step`]); these are the numbers that make a rat feel like a rat.
#[derive(Debug, Clone, Copy)]
pub struct BodySpec {
    /// Radius of the collision circle, m.
    pub radius: f32,
    /// Eye height standing / crouched, m.
    pub stand_eye: f32,
    /// Eye height crouched, m.
    pub crouch_eye: f32,
    /// Everyday movement speed, m/s.
    pub walk_speed: f32,
    /// Speed while Shift is held and moving forward, m/s.
    pub sprint_speed: f32,
    /// How far behind the character the third-person camera sits, m.
    pub third_person_distance: f32,
    /// Third-person camera lift above the eye, m.
    pub third_person_lift: f32,
    /// Near clip plane, m (a rat's eyes are 15 cm off the floor, so it must be tiny).
    pub near_plane: f32,
    /// Whether the character carries and swings the bat.
    pub has_bat: bool,
    /// Height of the body's collision cylinder (what shoves loose props), m.
    pub body_height: f32,
    /// How far away the character can pick a prop up, m.
    pub pickup_reach: f32,
    /// What the character can lift (see `crate::physics`).
    pub carry: crate::physics::CarryLimits,
    /// How far below eye level a carried prop's centre sits, m.
    pub hold_drop: f32,
}

impl Character {
    /// Both characters, in the order the launch screen shows them.
    pub const ALL: [Character; 2] = [Character::Human, Character::Rat];

    /// The body numbers for this character.
    pub fn body(self) -> BodySpec {
        match self {
            Character::Human => BodySpec {
                radius: PLAYER_RADIUS,
                stand_eye: STAND_EYE_HEIGHT,
                crouch_eye: CROUCH_EYE_HEIGHT,
                walk_speed: WALK_SPEED,
                sprint_speed: SPRINT_SPEED,
                third_person_distance: 3.4,
                third_person_lift: 0.55,
                near_plane: 0.05,
                has_bat: true,
                body_height: 1.75,
                pickup_reach: 2.3,
                carry: crate::physics::HUMAN_CARRY,
                hold_drop: 0.55,
            },
            // Cheddar's ordinary pace *is* a human's sprint: 6.5 m/s, and Shift adds nothing.
            Character::Rat => BodySpec {
                radius: RAT_RADIUS,
                stand_eye: 0.15,
                crouch_eye: 0.11,
                walk_speed: SPRINT_SPEED,
                sprint_speed: SPRINT_SPEED,
                third_person_distance: 1.1,
                third_person_lift: 0.22,
                near_plane: 0.02,
                has_bat: false,
                body_height: 0.17,
                pickup_reach: 0.9,
                carry: crate::physics::RAT_CARRY,
                hold_drop: 0.05,
            },
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Character::Human => "Human",
            Character::Rat => "Cheddar the rat",
        }
    }

    /// Parses `human` / `rat` (also `cheddar`), case-insensitively.
    pub fn parse(s: &str) -> Option<Character> {
        match s.trim().to_ascii_lowercase().as_str() {
            "human" | "person" | "h" => Some(Character::Human),
            "rat" | "cheddar" | "r" => Some(Character::Rat),
            _ => None,
        }
    }
}

/// Radius of Cheddar's collision circle, m: half a body length is 0.2 but he is narrow, and the
/// circle only has to keep him out of walls, so it hugs his width and he can squeeze through
/// gaps a person cannot.
pub const RAT_RADIUS: f32 = 0.12;

/// The vertical half of one physics tick, exactly as the live viewer runs it: optional jump,
/// gravity, and settling onto whatever walkable surface is under `pos` (see
/// [`ground_height_at`]). Returns the new `(foot_y, vertical_velocity)`.
pub fn vertical_step(ground: &GroundCandidates, pos: Vec2, foot_y: f32, vertical_velocity: f32, jump: bool) -> (f32, f32) {
    let ground_now = ground_height_at(ground, pos, foot_y);
    let grounded = foot_y <= ground_now && vertical_velocity <= 0.0;
    let mut vy = vertical_velocity;
    if jump && grounded {
        vy = JUMP_SPEED;
    }
    vy -= GRAVITY * FIXED_DT;
    let mut y = foot_y + vy * FIXED_DT;
    if y <= ground_now {
        y = ground_now;
        vy = 0.0;
    }
    (y, vy)
}
