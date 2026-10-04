//! Player-movement constants and the small physics helpers shared by the live viewer (`re2`)
//! and the offline map-analysis tools (`red_engine2 reach` / `lint` / `plan`).
//!
//! Keeping these in one place is the whole point: the analysis tools answer "can the player
//! actually walk from A to B?" by running the *same* collision/ground-height code the game runs
//! every physics tick, so a map that passes `lint` is playable, not just plausible.

use crate::collide::{colliders_on_floor_h, ground_height_at, resolve_collision, Collider2D, GroundCandidates, PLAYER_BAND_MAX_Y};
use glam::Vec2;

/// Per-scene tuning for a human player's first-person movement and view. These values are part of
/// the map, so the authoritative server and every predicting client use the same numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerTuning {
    /// When set, this scene is a single-character game: clients skip selection and the server
    /// rejects attempts to switch to another built-in body.
    pub character: Option<Character>,
    /// Vertical field of view in degrees.
    pub fov_deg: f32,
    /// Ordinary movement speed, m/s.
    pub walk_speed: f32,
    /// Forward speed while sprint is held, m/s.
    pub sprint_speed: f32,
    /// Multiplier applied to `walk_speed` while crouched.
    pub crouch_multiplier: f32,
    /// Initial upward speed of a normal jump, m/s.
    pub jump_speed: f32,
    /// Downward acceleration, m/s².
    pub gravity: f32,
    /// Ground acceleration per second; zero preserves legacy immediate movement.
    pub acceleration: f32,
    /// Acceleration along the wish direction while airborne.
    pub air_acceleration: f32,
    /// Ground friction per second for momentum movement.
    pub friction: f32,
    /// Maximum accumulated horizontal speed for momentum movement, m/s.
    pub max_speed: f32,
    /// Speed a released prop gets along the look direction on top of the holder's own velocity, m/s
    /// (`sim::player::release_velocity`).
    pub throw_speed: f32,
    /// Whether the scene is an arena (weapons, combat) or a peaceful one (empty hands, no fighting, a click interacts).
    pub mode: PlayerMode,
    /// Whether the game opens behind the character (`player.view`: `"third"`) instead of looking out of their eyes (`"first"`, the default). The player can still switch with Q.
    pub third_person: bool,
    /// Seconds the picture takes to fade in from black when play begins (`player.fade_in`; 0 = no fade).
    pub fade_in: f32,
    /// How far the world reaches: the looping axis and the walkable limits (the scene's `world` block).
    pub expanse: crate::expanse::Expanse,
}

/// The scene's stance on fighting (`player.mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerMode {
    /// The classic game: a weapon in hand, a click attacks, combat and its HUD exist.
    #[default]
    Arena,
    /// Empty hands. No weapon is allocated or drawn, no attack, recoil or swing sound, no crosshair or viewmodel, and the primary click
    /// means `interact` (pick up / drop / activate) instead of swinging. The server ignores attack, reload and weapon-switch input.
    Peaceful,
}

impl PlayerMode {
    /// Parses `arena` / `peaceful`.
    pub fn parse(s: &str) -> Option<PlayerMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "arena" | "combat" => Some(PlayerMode::Arena),
            "peaceful" | "peace" => Some(PlayerMode::Peaceful),
            _ => None,
        }
    }

    /// Whether weapons exist in this scene.
    pub fn is_peaceful(self) -> bool {
        self == PlayerMode::Peaceful
    }
}

impl Default for PlayerTuning {
    fn default() -> Self {
        PlayerTuning {
            character: None,
            fov_deg: 90.0,
            walk_speed: WALK_SPEED,
            sprint_speed: SPRINT_SPEED,
            crouch_multiplier: CROUCH_SPEED_MULT,
            jump_speed: JUMP_SPEED,
            gravity: GRAVITY,
            acceleration: 0.0,
            air_acceleration: 1.0,
            friction: 6.0,
            max_speed: 20.0,
            throw_speed: THROW_SPEED,
            mode: PlayerMode::Arena,
            third_person: false,
            fade_in: 0.0,
            expanse: crate::expanse::Expanse::default(),
        }
    }
}

/// An authored vertical launcher. A player touching its top surface is launched identically in
/// offline play, server simulation, replay, and client prediction.
#[derive(Debug, Clone, PartialEq)]
pub struct JumpPad {
    pub id: String,
    /// Centre in the horizontal X/Z plane.
    pub center: Vec2,
    /// Width/depth in meters.
    pub size: Vec2,
    /// Height of the surface the player's feet touch.
    pub foot_y: f32,
    /// Initial upward speed, m/s.
    pub launch_speed: f32,
}

impl JumpPad {
    /// Whether a grounded player at `pos`/`foot_y` is touching this pad.
    pub fn touches(&self, pos: Vec2, foot_y: f32) -> bool {
        let half = self.size * 0.5;
        (pos.x - self.center.x).abs() <= half.x && (pos.y - self.center.y).abs() <= half.y && (foot_y - self.foot_y).abs() <= 0.18
    }
}

/// Movement/collision/gravity run at this fixed timestep (`sim::clock::TICK_DT`, 60 Hz) in the live viewer.
pub const FIXED_DT: f32 = crate::sim::clock::TICK_DT;

/// Walking speed, m/s.
pub const WALK_SPEED: f32 = 3.2;
/// Speed a released prop gets along the look on top of the holder's own velocity, m/s (`player.throw_speed`).
pub const THROW_SPEED: f32 = 1.0;
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
/// (`collide::PLAYER_BAND_MAX_Y`), so anything whose underside is *within* 2.0 m of the floor —
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
    step_horizontal_band(colliders, pos, foot_y, delta, radius, PLAYER_BAND_MAX_Y)
}

/// [`step_horizontal_r`] for a body whose top is `band_top` m above its feet: only colliders reaching
/// below that height block, so a rat (0.25 m) runs under a table whose top is 0.78 m up.
pub fn step_horizontal_band(colliders: &[Collider2D], pos: Vec2, foot_y: f32, delta: Vec2, radius: f32, band_top: f32) -> Vec2 {
    let active = colliders_on_floor_h(colliders, foot_y, band_top);
    let mut p = pos;
    p.x += delta.x;
    p = resolve_collision(p, radius, &active);
    p.y += delta.y;
    resolve_collision(p, radius, &active)
}

/// Who the player is. A scene policy or a command-line override selects it (there is no in-game picker);
/// each character has its own body numbers ([`BodySpec`]) and model (`crate::characters`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Character {
    /// The ordinary-looking person with a bat.
    Human,
    /// Cheddar, the small brownish-grey lab rat.
    Rat,
    /// Star-hatted spellcaster using the human movement rig.
    Wizard,
    /// Frontier explorer using the human movement rig.
    Cowboy,
    /// Antenna-bearing visitor using the human movement rig.
    Alien,
    /// Workshop robot using the human movement rig.
    Robot,
    /// A Ridgeback soldier (team 1: army green and tan) on the human rig.
    Ridgeback,
    /// A Nightfall soldier (team 2: navy and black) on the human rig.
    Nightfall,
    /// A too-tall, too-thin, featureless figure on the human rig — a monster, not a costume.
    Hollow,
    /// A boy of about nine in a red-orange jumper and a yellow scarf, smiling: a picture-book child (big round head, sturdy limbs) on the human rig, and slower, smaller, with no bat.
    Boy,
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
    /// How far above its feet the body blocks against overhead geometry, m: anything whose underside
    /// is higher than this can be walked under (a person 2.0, Cheddar 0.25).
    pub band_top: f32,
    /// How far away the character can pick a prop up, m.
    pub pickup_reach: f32,
    /// What the character can lift (see `crate::physics`).
    pub carry: crate::physics::CarryLimits,
    /// How far below eye level a carried prop's centre sits, m.
    pub hold_drop: f32,
}

impl Character {
    /// Every character, in wire-code order.
    pub const ALL: [Character; 10] = [
        Character::Human,
        Character::Rat,
        Character::Wizard,
        Character::Cowboy,
        Character::Alien,
        Character::Robot,
        Character::Ridgeback,
        Character::Nightfall,
        Character::Hollow,
        Character::Boy,
    ];

    /// The body numbers for this character.
    pub fn body(self) -> BodySpec {
        match self {
            Character::Human
            | Character::Wizard
            | Character::Cowboy
            | Character::Alien
            | Character::Robot
            | Character::Ridgeback
            | Character::Nightfall
            | Character::Hollow => BodySpec {
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
                band_top: PLAYER_BAND_MAX_Y,
                pickup_reach: 2.3,
                carry: crate::physics::HUMAN_CARRY,
                hold_drop: 0.55,
            },
            // The boy wanders and runs at a child's pace, sees the world from a child's height, and carries nothing heavier than a stick.
            Character::Boy => BodySpec {
                radius: 0.28,
                stand_eye: 1.18,
                crouch_eye: 0.8,
                walk_speed: 2.9,
                sprint_speed: 5.4,
                third_person_distance: 3.1,
                third_person_lift: 0.5,
                near_plane: 0.05,
                has_bat: false,
                body_height: 1.35,
                band_top: 1.4,
                pickup_reach: 1.6,
                carry: crate::physics::HUMAN_CARRY,
                hold_drop: 0.42,
            },
            // Cheddar has one pace, `RAT_SPEED`, and Shift adds nothing.
            Character::Rat => BodySpec {
                radius: RAT_RADIUS,
                stand_eye: 0.15,
                crouch_eye: 0.11,
                walk_speed: RAT_SPEED,
                sprint_speed: RAT_SPEED,
                third_person_distance: 1.1,
                third_person_lift: 0.22,
                near_plane: 0.02,
                has_bat: false,
                body_height: 0.17,
                band_top: RAT_BAND_TOP,
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
            Character::Wizard => "Wizard",
            Character::Cowboy => "Cowboy",
            Character::Alien => "Alien",
            Character::Robot => "Robot",
            Character::Ridgeback => "Ridgeback soldier",
            Character::Nightfall => "Nightfall soldier",
            Character::Hollow => "The Hollow",
            Character::Boy => "The boy",
        }
    }

    /// Parses `human` / `rat` (also `cheddar`), case-insensitively.
    pub fn parse(s: &str) -> Option<Character> {
        match s.trim().to_ascii_lowercase().as_str() {
            "human" | "person" | "h" => Some(Character::Human),
            "rat" | "cheddar" | "r" => Some(Character::Rat),
            "wizard" => Some(Character::Wizard),
            "cowboy" => Some(Character::Cowboy),
            "alien" => Some(Character::Alien),
            "robot" => Some(Character::Robot),
            "ridgeback" => Some(Character::Ridgeback),
            "nightfall" => Some(Character::Nightfall),
            "hollow" => Some(Character::Hollow),
            "boy" | "child" | "kid" => Some(Character::Boy),
            _ => None,
        }
    }
}

/// Cheddar's one running pace, m/s: brisk next to a person's walk (3.2), well under a person's sprint (6.5).
pub const RAT_SPEED: f32 = 4.0;
/// How far above the floor Cheddar's body reaches, m: a tabletop, a platform or any overhead geometry
/// whose underside is higher than this can be run under.
pub const RAT_BAND_TOP: f32 = 0.25;

/// Radius of Cheddar's collision circle, m: half a body length is 0.2 but he is narrow, and the
/// circle only has to keep him out of walls, so it hugs his width and he can squeeze through
/// gaps a person cannot.
pub const RAT_RADIUS: f32 = 0.12;

/// The vertical half of one physics tick, exactly as the live viewer runs it: optional jump,
/// gravity, and settling onto whatever walkable surface is under `pos` (see
/// [`ground_height_at`]). Returns the new `(foot_y, vertical_velocity)`.
pub fn vertical_step(ground: &GroundCandidates, pos: Vec2, foot_y: f32, vertical_velocity: f32, jump: bool) -> (f32, f32) {
    vertical_step_on(ground, None, pos, foot_y, vertical_velocity, jump)
}

/// [`vertical_step`] with an optional extra floor under the feet (the top of a loose prop the player stands on, found by
/// `PropWorld::floor_under`): the higher of it and the static ground is what they stand on, and they can jump off it.
pub fn vertical_step_on(ground: &GroundCandidates, extra_floor: Option<f32>, pos: Vec2, foot_y: f32, vertical_velocity: f32, jump: bool) -> (f32, f32) {
    vertical_step_on_tuned(ground, extra_floor, pos, foot_y, vertical_velocity, jump, JUMP_SPEED, GRAVITY)
}

/// [`vertical_step_on`] with authored jump and gravity values.
pub fn vertical_step_on_tuned(
    ground: &GroundCandidates,
    extra_floor: Option<f32>,
    pos: Vec2,
    foot_y: f32,
    vertical_velocity: f32,
    jump: bool,
    jump_speed: f32,
    gravity: f32,
) -> (f32, f32) {
    let ground_now = ground_height_at(ground, pos, foot_y).max(extra_floor.unwrap_or(f32::NEG_INFINITY));
    let grounded = foot_y <= ground_now && vertical_velocity <= 0.0;
    let mut vy = vertical_velocity;
    if jump && grounded {
        vy = jump_speed;
    }
    vy -= gravity * FIXED_DT;
    let mut y = foot_y + vy * FIXED_DT;
    if y <= ground_now {
        y = ground_now;
        vy = 0.0;
    }
    (y, vy)
}
