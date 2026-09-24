//! Red Engine 2 — a first-person, walk-around viewer for a red_engine2 scene, forked from
//! the original Red Engine to be the base for an online prop hunt game.
//!
//! `re2 [scene.json]` opens a window, drops you inside the scene at the camera's
//! default position, and lets you walk around and look at things: WASD or the arrow keys to
//! move, the mouse to look, Shift to sprint forward, Space for a small jump, Ctrl to crouch,
//! F to toggle borderless fullscreen / maximized, click to (re)capture the mouse, Escape to
//! release it, Q to toggle between first- and third-person view. On launch a menu asks whether to
//! play the Human or Cheddar the rat (`--as human|rat` or `RE2_CHARACTER` skips it). The human
//! holds a bat; left-click swings it, and anything the swing actually touches gets logged, thunks
//! and flashes — a swing through empty air is silent. Hitting things is the seeker's primary
//! action on objects; the crosshair turns gold when something is within bat reach. Cheddar is
//! small and moves at a human's sprint speed all the time. E picks up (and drops) a loose prop —
//! see `red_engine2::physics`. (Right-click is reserved for the hider's "choose an object to
//! replicate", then R — not built yet.)

// A game window shouldn't drag a console window along with it. `windows_subsystem = "windows"`
// stops Windows creating one; `win::attach_console` then re-attaches to the *parent* terminal
// when there is one, so `cargo run` / `RE2_STATS=1` / panics still print where you launched it.
#![cfg_attr(windows, windows_subsystem = "windows")]

use red_engine2::audio::{synth_bat_hit, synth_revolver_shot, synth_weapon_click, Audio};
use red_engine2::net::bot::ClientWorld;
use red_engine2::net::session::NetSession;
use red_engine2::sim::clock::TickClock;
use red_engine2::sim::player::{step_player, PlayerInput, PlayerState};
use red_engine2::sim::combat::{Cooldown, MeleeSwing, WeaponSwitch};
use red_engine2::weapons::{
    Ammo, Weapon, DRY_FIRE_COOLDOWN_TICKS, MUZZLE_FLASH_TIME, RECOIL_TIME, REVOLVER_AMMO, REVOLVER_COOLDOWN_TICKS, REVOLVER_IMPULSE, REVOLVER_RANGE, SWING_RECOVER_SECS,
    SWING_STRIKE_SECS, SWING_WINDUP_SECS, SWITCH_SECS,
};
use red_engine2::viewer::{IDLE_PITCH_DEG, IDLE_ROLL_DEG};
use red_engine2::characters::{human_object, rat_object, HUMAN_HEIGHT};
use red_engine2::easing::Ease;
use red_engine2::hit::{collect_hit_shapes_where, raycast_shapes, HitShape};
use red_engine2::physics::PropWorld;
use red_engine2::menu::{self, PauseAction};
use red_engine2::player::{BodySpec, Character, FIXED_DT};
use red_engine2::schema::{Object, ObjectKind, Scene};
use red_engine2::skeleton::{pose_to_parts, HumanoidRig, PoseSample};
use red_engine2::track::Track;
use red_engine2::viewer::{
    collect_box_colliders_except, collect_ground_candidates_except, colliders_on_floor, viewmodel_transform, Collider2D, FpsCamera, FrameOptions, GroundCandidates, LiveRenderer,
    resolve_collision,
};
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use std::collections::HashSet;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

// Movement/collision/gravity run on a fixed 60Hz timestep, decoupled from the render frame
// rate (see `App::fixed_update` / `App::update`) — a deterministic step size regardless of
// frame-time variance avoids the jitter a variable-dt physics step reads as under any load
// spike, and avoids a single large clamped dt letting the player tunnel partway into a thin
// collider before the next push-out. The render frame then interpolates between the last two
// completed physics states instead of snapping to whichever one just finished.
// (the movement constants themselves live in `red_engine2::player`, shared with the offline
// map-analysis tools so `lint`/`reach` simulate exactly this physics)
const CROUCH_TRANSITION_TIME: f32 = 0.12;
const MOUSE_SENSITIVITY: f32 = 0.0025;
/// Mouse motion is ignored this long after the cursor is captured (see `App::grabbed_at`).
const MOUSE_SETTLE_SECS: f32 = 0.35;

const BASE_FOV_DEG: f32 = 90.0;
// A game-y widened FOV while sprinting reads as speed even before the eye adjusts to how fast
// the walls are sliding by; it also smoothly signals when sprint actually kicks in vs. Shift
// being held but disallowed (crouching, or not moving forward).
const SPRINT_FOV_BOOST_DEG: f32 = 8.0;
const FOV_TRANSITION_TIME: f32 = 0.15;


// Hitting things with the bat is the seeker's main way to act on objects (a struck object makes a
// sound but does not change colour); `E` picks up / drops loose props; right-click is reserved for the
// hider's "pick an object to replicate" (then `R`), not built yet.

// Bat viewmodel: idle pose and swing animation, both expressed as a pitch (rotation about
// the camera's local right axis, tipping the bar up/down) plus a forward lunge, in the
// camera-local frame `viewmodel_transform` expects (+X right, +Y up, +Z forward). Roll (rotation
// about the bar's own axis) stays fixed — it just leans the bat across the view for a
// less "straight ahead" held pose (the bat leans in across the view).
const VM_RIGHT: f32 = 0.10;
const VM_DOWN: f32 = 0.125;
const VM_FORWARD: f32 = 0.30;
const WINDUP_PITCH_DEG: f32 = -128.0;
const STRIKE_PITCH_DEG: f32 = 42.0;
const STRIKE_LUNGE: f32 = 0.16;

// Swing phase durations (seconds) and the melee reach used for the hit-detection raycast fired
// once per swing, at the start of the strike phase.
// Derived from the simulation's whole-tick timings (`weapons::SWING_*_TICKS`): the animation
// plays exactly the swing the simulation runs.
const SWING_WINDUP: f32 = SWING_WINDUP_SECS;
const SWING_STRIKE: f32 = SWING_STRIKE_SECS;
const SWING_RECOVER: f32 = SWING_RECOVER_SECS;
const MELEE_REACH: f32 = 2.2;

// Player body model ("skin"): a default humanoid rig standing in for the player, matched to
// STAND_EYE_HEIGHT's implied stature. Its own scale is toggled between this and a
// near-invisible value to fake per-view-mode visibility (see `update_player_body`), since the
// renderer has no per-object visibility flag to hide it in first person instead.
const PLAYER_HEIGHT: f32 = HUMAN_HEIGHT;
const PLAYER_BUILD: f32 = 1.0;
const HIDDEN_SCALE: f32 = 0.0005;
/// Cheddar's gait phase advances this many radians per metre travelled (a quick scurry).
const RAT_GAIT_RAD_PER_M: f32 = 5.0;

// Procedural walk cycle for the player body: leg/arm swing amplitude and a cycle rate defined
// relative to WALK_SPEED so sprinting/crouch-walking scale the animation's tempo with actual
// speed instead of playing at a fixed rate regardless of how fast the player is moving.
const WALK_CYCLES_PER_SEC_AT_WALK_SPEED: f32 = 1.6;
const HIP_SWING_DEG: f32 = 28.0;
const KNEE_LIFT_DEG: f32 = 45.0;
const KNEE_REST_DEG: f32 = 4.0;
const SHOULDER_SWING_DEG: f32 = 20.0;
const IDLE_SWAY_DEG: f32 = 1.4;

// Third-person camera: pulled back and up from the player's eye point, orbiting with the same
// yaw/pitch mouse look as first person. `THIRD_PERSON_CAM_RADIUS` is only pushed out of wall
// colliders it ends up inside (see `resolve_collision`'s use below) — it doesn't raycast for a
// wall standing *between* the player and the camera, so a camera clipping through a thin wall
// from the far side is a known limitation of this simple a collision model.
const THIRD_PERSON_CAM_RADIUS: f32 = 0.25;

// Third-person hand attachment: the bat is held in the character's anatomical right hand, which
// the humanoid rig calls its *left* arm (the rig faces +Z, so its "right" side is +X, the
// character's left). The swing animation drives that same arm. The grip sits at the end of that
// forearm bone (`skeleton::pose_to_parts`, part index 3) and follows it through the walk cycle
// and the swing; the bat's *orientation* is built from the body's yaw plus the very same
// pitch/roll the first-person viewmodel uses, so both views show one motion (the bone's own roll
// about its length is arbitrary, so it can't be used for orientation).
const BAT_FOREARM_PART: usize = 3;

// Swing pose for the bat arm in third person (shoulder raise/swing on the local
// right axis, elbow bend), driven by the same windup/strike/recover phases as the first-person
// viewmodel's pitch (see `weapon_transform`) so both views read as the same motion.
const ARM_WINDUP_SHOULDER_X: f32 = 55.0;
const ARM_STRIKE_SHOULDER_X: f32 = -95.0;
const ARM_IDLE_ELBOW_DEG: f32 = 8.0;
const ARM_WINDUP_ELBOW_DEG: f32 = 60.0;
const ARM_STRIKE_ELBOW_DEG: f32 = 12.0;
/// Seconds the lower-and-raise animation takes when scrolling between the bat and the revolver
/// (whole ticks in the simulation, `weapons::SWITCH_TICKS`).
const SWITCH_TIME: f32 = SWITCH_SECS;
/// Scroll lines needed to change weapon (a notch of a wheel is one line; touchpads send fractions).
const SCROLL_LINES_PER_SWITCH: f32 = 1.0;
/// Revolver viewmodel rest pose in the camera's frame (right, down, forward), and its recoil kick.
const GUN_RIGHT: f32 = 0.16;
const GUN_DOWN: f32 = 0.17;
const GUN_FORWARD: f32 = 0.40;
/// The gun angles in toward the crosshair by this much, so the player sees its left side (cylinder and all).
const GUN_YAW_DEG: f32 = -8.0;
const GUN_IDLE_PITCH_DEG: f32 = -3.0;
const GUN_RECOIL_PITCH_DEG: f32 = -17.0;
const GUN_RECOIL_BACK: f32 = 0.06;
/// Arm pose for aiming the revolver in third person (shoulder raised to level, elbow nearly straight).
const AIM_SHOULDER_X: f32 = -84.0;
const AIM_ELBOW_DEG: f32 = 6.0;
/// Arm pose while carrying a prop (both arms forward, elbows bent).
const CARRY_SHOULDER_X: f32 = -72.0;
const CARRY_ELBOW_DEG: f32 = 38.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    FirstPerson,
    ThirdPerson,
}

/// Builds the player's own body for `who`, added to the scene at runtime rather than authored in
/// the scene JSON, since it represents the player rather than the room. Its transform and pose are
/// rewritten every frame by `update_player_body`; the values here are just the resting pose.
fn build_player_object(who: Character) -> Object {
    let mut o = match who {
        Character::Human => human_object("player_body"),
        Character::Rat => rat_object("player_body"),
    };
    o.scale = Track::constant(Vec3::splat(HIDDEN_SCALE));
    o.collide = true;
    o
}

struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// Draws the map; built once a character is chosen (its scene includes the player's body).
    live: Option<LiveRenderer>,
    /// Draws the launch menu's 3-D backdrop until then.
    menu: Option<LiveRenderer>,
}

/// Whether the launch menu or the game itself is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Choosing a character.
    Menu,
    /// In the map.
    Playing,
}

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<GpuState>,
    scene: Scene,
    scene_path: PathBuf,
    colliders: Vec<Collider2D>,
    ground: GroundCandidates,
    /// Every solid leaf shape a swing can strike (see `red_engine2::hit`), excluding the player.
    hit_shapes: Vec<HitShape>,
    /// Loose props (pick up with E, drop, knock over); built when the game starts.
    props: Option<PropWorld>,
    /// The loose prop under the crosshair that this character can pick up.
    pickup_target: Option<usize>,
    /// The player's planar velocity, m/s (a dropped prop inherits it).
    player_vel: Vec3,
    camera: FpsCamera,
    phase: Phase,
    /// Who the player is (and, in the menu, which card is highlighted).
    character: Character,
    body: BodySpec,
    /// `--as` / `RE2_CHARACTER`: skip the menu.
    forced_character: Option<Character>,
    menu_scene: Scene,
    /// What the menu overlay currently shows `(width, height, selected)`, to repaint only on change.
    menu_painted: Option<(u32, u32, Character)>,
    cursor_x: f32,
    keys: HashSet<KeyCode>,
    grabbed: bool,
    /// When the mouse was last captured. Capturing recenters the cursor, which delivers one big
    /// spurious motion delta — without ignoring input briefly the camera spins away from the
    /// spawn heading the moment the window opens.
    grabbed_at: Instant,
    sprint_held: bool,
    jump_queued: bool,
    /// Left click waiting for the next simulation tick (swing or shot).
    attack_queued: bool,
    /// The Escape menu is showing (the mouse is free; single-player is frozen).
    paused: bool,
    pause_hover: Option<PauseAction>,
    cursor: (f32, f32),
    /// `--connect`: the multiplayer session (`None` = single player).
    net: Option<NetSession>,
    /// Address to join, and the client's view of the map, until `start_game` uses them.
    net_server: Option<SocketAddr>,
    net_world: Option<ClientWorld>,
    /// Debug: `RE2_AUTOWALK=forward|circle[:deg/s]` walks by itself (for unattended multi-window demos).
    autowalk: Option<String>,
    net_title_at: Instant,
    /// Scroll-wheel weapon switch (`+1`/`-1`) waiting for the next tick.
    switch_queued: Option<i32>,
    /// The weapon in hand (or being switched to). Human only.
    weapon: Weapon,
    /// Lower-and-raise animation between weapons: `(from, elapsed seconds)`.
    switching: Option<(Weapon, f32)>,
    /// Scroll-wheel lines accumulated toward the next switch.
    scroll_accum: f32,
    /// The revolver's ammunition (infinite for now, see `weapons::REVOLVER_AMMO`).
    ammo: Ammo,
    /// Shot-to-shot delay, in ticks.
    shot_cd: Cooldown,
    /// The bat swing (simulation state, in ticks). `swing_timer` below is its render-side mirror.
    swing: MeleeSwing,
    /// The weapon switch (simulation state). `switching` below is its render-side mirror.
    switch: WeaponSwitch,
    /// The fixed 60 Hz clock: frames push real time in, ticks come out.
    clock: TickClock,
    /// Seconds since the last shot (drives the recoil kick); starts settled.
    since_shot: f32,
    /// Seconds of muzzle flash left.
    flash_left: f32,
    /// The player's eye this frame (the origin of swings and shots).
    eye: Vec3,
    shot_sound: Vec<f32>,
    click_sound: Vec<f32>,
    /// Seconds into the current bat swing (incl. the fraction of the next tick), or `None` when idle/holding.
    /// Recomputed every frame from `swing`; only the animation reads it.
    swing_timer: Option<f32>,
    target_index: Option<usize>,
    /// Fixed-timestep physics state (see [`FIXED_DT`]): the authoritative planar position after
    /// the most recently completed physics step, and the one before it, so the actual rendered
    /// frame can interpolate between them instead of drawing exactly on a physics step boundary.
    physics_pos: Vec2,
    prev_physics_pos: Vec2,
    foot_y: f32,
    prev_foot_y: f32,
    vertical_velocity: f32,
    /// This frame's walking speed (0 when standing still), captured from the last fixed physics
    /// step that ran so the walk-cycle animation (which runs once per rendered frame, not once
    /// per physics step) knows how fast to play.
    last_move_speed: f32,
    eye_height: f32,
    fov_deg: f32,
    view_mode: ViewMode,
    /// Index into `scene.objects` of the player's own body (see `build_player_object`), so
    /// `update_player_body` can mutate its transform/pose in place each frame.
    player_object_index: usize,
    /// Radians accumulated while walking, driving the procedural walk-cycle pose.
    walk_phase: f32,
    /// The third-person hand-held bat's current world transform (see
    /// `update_player_body`), recomputed each frame from the right forearm bone.
    hand_prop_transform: Mat4,
    /// `None` when no audio output device is available — playback is just skipped rather than
    /// erroring, so a missing sound card doesn't take the game down with it.
    audio: Option<Audio>,
    /// Synthesized once at startup and replayed on every hit rather than re-synthesized each
    /// time (cheap either way at this length, but there's no reason to redo fixed work).
    hit_sound: Vec<f32>,
    /// Debug: `RE2_VIEW=third` starts in third person (for screenshots).
    debug_third_person: bool,
    /// Debug: `RE2_FREEZE_SHOT=<seconds since the shot>` holds the revolver's recoil/flash there.
    freeze_shot: Option<f32>,
    /// Debug: `RE2_FREEZE_SWING=<seconds>` holds the swing animation at that time (for screenshots).
    freeze_swing: Option<f32>,
    start: Instant,
    last_frame: Instant,
    /// `RE2_STATS=1`: print average frame time / FPS every couple of seconds (for measuring
    /// how a map performs without attaching a profiler).
    stats: Option<FrameStats>,
}

struct FrameStats {
    window_start: Instant,
    frames: u32,
    worst_ms: f32,
}

impl App {
    fn new(scene: Scene, scene_path: PathBuf, forced_character: Option<Character>, net_server: Option<SocketAddr>, net_world: Option<ClientWorld>) -> Self {
        // The player's body is added by `start_game` once the character is chosen.
        let player_object_index = scene.objects.len();
        let character = forced_character.unwrap_or(Character::Human);
        let body = character.body();

        let spawn = scene.camera.position.sample(0.0);
        let target = scene.camera.target.sample(0.0);
        let yaw = (target.x - spawn.x).atan2(-(target.z - spawn.z)).to_degrees();
        let mut camera = FpsCamera::new(Vec3::new(spawn.x, body.stand_eye, spawn.z), yaw);
        camera.fov_deg = BASE_FOV_DEG;
        // Debug: `RE2_PITCH=<degrees>` starts looking up (+) / down (-), for screenshots.
        if let Some(deg) = std::env::var("RE2_PITCH").ok().and_then(|v| v.parse::<f32>().ok()) {
            camera.pitch = deg.to_radians();
        }
        App {
            window: None,
            gpu: None,
            scene,
            scene_path,
            colliders: Vec::new(),
            ground: GroundCandidates::default(),
            hit_shapes: Vec::new(),
            props: None,
            pickup_target: None,
            player_vel: Vec3::ZERO,
            camera,
            phase: Phase::Menu,
            character,
            body,
            forced_character,
            menu_scene: menu::menu_scene(),
            menu_painted: None,
            cursor_x: 0.0,
            keys: HashSet::new(),
            grabbed: false,
            grabbed_at: Instant::now(),
            sprint_held: false,
            jump_queued: false,
            attack_queued: false,
            paused: false,
            pause_hover: None,
            cursor: (0.0, 0.0),
            net: None,
            net_server,
            net_world,
            autowalk: std::env::var("RE2_AUTOWALK").ok().filter(|v| !v.is_empty()),
            net_title_at: Instant::now(),
            switch_queued: None,
            shot_cd: Cooldown::default(),
            swing: MeleeSwing::default(),
            switch: WeaponSwitch::default(),
            clock: TickClock::default(),
            swing_timer: None,
            target_index: None,
            physics_pos: Vec2::new(spawn.x, spawn.z),
            prev_physics_pos: Vec2::new(spawn.x, spawn.z),
            foot_y: 0.0,
            prev_foot_y: 0.0,
            vertical_velocity: 0.0,
            last_move_speed: 0.0,
            eye_height: body.stand_eye,
            fov_deg: BASE_FOV_DEG,
            view_mode: ViewMode::FirstPerson,
            player_object_index,
            walk_phase: 0.0,
            hand_prop_transform: Mat4::from_scale(Vec3::splat(HIDDEN_SCALE)),
            audio: Audio::new(),
            hit_sound: synth_bat_hit(),
            weapon: Weapon::Bat,
            switching: None,
            scroll_accum: 0.0,
            ammo: REVOLVER_AMMO,
            since_shot: RECOIL_TIME,
            flash_left: 0.0,
            eye: Vec3::ZERO,
            shot_sound: synth_revolver_shot(),
            click_sound: synth_weapon_click(),
            debug_third_person: std::env::var("RE2_VIEW").is_ok_and(|v| v == "third"),
            freeze_shot: std::env::var("RE2_FREEZE_SHOT").ok().and_then(|v| v.parse().ok()),
            freeze_swing: std::env::var("RE2_FREEZE_SWING").ok().and_then(|v| v.parse().ok()),
            start: Instant::now(),
            last_frame: Instant::now(),
            stats: std::env::var_os("RE2_STATS").map(|_| FrameStats { window_start: Instant::now(), frames: 0, worst_ms: 0.0 }),
        }
    }

    /// The seeker's primary (and only) action on objects. Runs once per bat swing, at the start
    /// of the strike phase, and only when the swing's ray struck real geometry within
    /// `MELEE_REACH` (see `red_engine2::hit`): logs it and plays the impact thunk (the object itself
    /// does not change colour). A swing that touches nothing never gets here, so it is silent.
    fn hit_with(&mut self, object_index: usize) {
        let id = self.scene.objects[object_index].id.clone();
        println!("Hit '{id}' with the bat!");
        if let Some(audio) = &self.audio {
            audio.play(&self.hit_sound);
        }
    }

    /// Blends between `idle`, `windup`, and `strike` values across the current swing's three
    /// phases — `idle` itself when not swinging. Shared by the first-person weapon's pitch, the
    /// third-person arm's shoulder/elbow pose, and the weapon's forward lunge, each of which
    /// just plugs in different endpoint values for the same windup/strike/recover curve.
    fn swing_blend(&self, idle: f32, windup: f32, strike: f32) -> f32 {
        match self.swing_timer {
            None => idle,
            Some(elapsed) if elapsed < SWING_WINDUP => {
                let f = Ease::Out.apply(elapsed / SWING_WINDUP);
                idle + (windup - idle) * f
            }
            Some(elapsed) if elapsed < SWING_WINDUP + SWING_STRIKE => {
                let f = Ease::In.apply((elapsed - SWING_WINDUP) / SWING_STRIKE);
                windup + (strike - windup) * f
            }
            Some(elapsed) => {
                let f = Ease::Out.apply(((elapsed - SWING_WINDUP - SWING_STRIKE) / SWING_RECOVER).min(1.0));
                strike + (idle - strike) * f
            }
        }
    }

    /// The bat viewmodel's current world transform: an idle held pose, or mid-swing pose
    /// interpolated by elapsed time through windup/strike/recover. Pitch is rotation about the
    /// camera's local right axis (tipping the bar up/down); roll (about the bar's own axis, so
    /// the shaft itself doesn't visibly change) stays fixed to keep the bat leaning across
    /// the view.
    fn weapon_transform(&self) -> Mat4 {
        // The bat viewmodel is camera-attached, not bound to the body rig's hand bone, so in
        // third person it would just float in front of the (now distant) camera. Shrinking it
        // away reuses the same "scale to near-nothing" hide trick as the player body's own
        // first-person visibility toggle rather than adding a second code path.
        if self.view_mode == ViewMode::ThirdPerson || !self.body.has_bat || self.carrying() {
            return Mat4::from_scale(Vec3::splat(HIDDEN_SCALE));
        }
        // Scrolling between weapons lowers the old one and raises the new one.
        let dip = self.switch_dip();
        if self.shown_weapon() == Weapon::Revolver {
            let k = self.recoil_kick();
            let local_rotation = Mat4::from_rotation_y(GUN_YAW_DEG.to_radians()) * Mat4::from_rotation_x((GUN_IDLE_PITCH_DEG + GUN_RECOIL_PITCH_DEG * k + 25.0 * dip).to_radians());
            let local_offset = Vec3::new(GUN_RIGHT, -GUN_DOWN - 0.30 * dip, GUN_FORWARD - GUN_RECOIL_BACK * k);
            return viewmodel_transform(&self.camera, local_offset, local_rotation);
        }
        let pitch_deg = self.swing_blend(IDLE_PITCH_DEG, WINDUP_PITCH_DEG, STRIKE_PITCH_DEG);
        let lunge = self.swing_blend(0.0, 0.0, STRIKE_LUNGE);
        let local_rotation = Mat4::from_rotation_z(IDLE_ROLL_DEG.to_radians()) * Mat4::from_rotation_x((pitch_deg + 30.0 * dip).to_radians());
        let local_offset = Vec3::new(VM_RIGHT, -VM_DOWN - 0.35 * dip, VM_FORWARD + lunge);
        viewmodel_transform(&self.camera, local_offset, local_rotation)
    }

    /// The weapon drawn right now: during a switch the old one until the halfway point, then the new.
    fn shown_weapon(&self) -> Weapon {
        match self.switching {
            Some((from, t)) if t < SWITCH_TIME * 0.5 => from,
            _ => self.weapon,
        }
    }

    /// 0..1..0 over a weapon switch (how far the viewmodel is lowered), 0 when not switching.
    fn switch_dip(&self) -> f32 {
        self.switching.map_or(0.0, |(_, t)| (std::f32::consts::PI * (t / SWITCH_TIME).clamp(0.0, 1.0)).sin())
    }

    /// Recoil kick right after a shot, 1 -> 0 (eased) over `RECOIL_TIME`.
    fn recoil_kick(&self) -> f32 {
        let f = (self.since_shot / RECOIL_TIME).clamp(0.0, 1.0);
        (1.0 - f) * (1.0 - f)
    }

    /// Scroll wheel: switch between the bat and the revolver (human only, not while carrying).
    fn on_scroll(&mut self, lines: f32) {
        if self.net.is_some() {
            return; // weapons are not networked yet
        }
        if !self.body.has_bat || self.carrying() || self.switch.is_active() || self.switch_queued.is_some() {
            return;
        }
        self.scroll_accum += lines;
        if self.scroll_accum.abs() >= SCROLL_LINES_PER_SWITCH {
            self.switch_queued = Some(if self.scroll_accum > 0.0 { 1 } else { -1 });
            self.scroll_accum = 0.0;
        }
    }

    /// Simulation tick: performs the queued weapon switch.
    fn begin_switch(&mut self, dir: i32) {
        if !self.body.has_bat || self.carrying() || self.switch.is_active() {
            return;
        }
        let to = self.weapon.cycle(dir);
        self.switch.start(self.weapon);
        self.weapon = to;
        self.swing.cancel();
        println!("Weapon: {}", to.name());
        if to == Weapon::Revolver {
            if let Some(audio) = &self.audio {
                audio.play(&self.click_sound);
            }
        }
    }

    /// Simulation tick: one revolver shot at the crosshair (hitscan) from the tick's eye. Infinite ammo for now.
    fn fire_revolver(&mut self) {
        if !self.shot_cd.ready() || self.switch.is_active() || self.carrying() {
            return;
        }
        if !self.ammo.try_fire() {
            self.shot_cd.start(DRY_FIRE_COOLDOWN_TICKS);
            if let Some(audio) = &self.audio {
                audio.play(&self.click_sound);
            }
            return;
        }
        self.shot_cd.start(REVOLVER_COOLDOWN_TICKS);
        self.since_shot = 0.0;
        self.flash_left = MUZZLE_FLASH_TIME;
        if let Some(audio) = &self.audio {
            audio.play(&self.shot_sound);
        }
        let dir = self.camera.forward();
        let eye = self.tick_eye();
        if let Some((object_index, distance, loose)) = self.probe(eye, REVOLVER_RANGE) {
            if let (Some(prop), Some(props)) = (loose, self.props.as_mut()) {
                props.strike_impulse(prop, dir, eye + dir * distance, REVOLVER_IMPULSE);
            }
            println!("Shot '{}' at {:.1} m", self.scene.objects[object_index].id, distance);
        }
    }

    /// Escape: free the mouse and show the pause menu (Resume / Quit).
    fn enter_pause(&mut self) {
        self.paused = true;
        self.pause_hover = None;
        self.keys.clear(); // no key-release events arrive while the menu has the mouse
        self.sprint_held = false;
        self.set_grab(false);
        self.repaint_pause();
    }

    /// Hides the pause menu and takes the mouse back.
    fn leave_pause(&mut self) {
        self.paused = false;
        self.pause_hover = None;
        if let Some(live) = self.gpu.as_mut().and_then(|g| g.live.as_mut()) {
            live.overlay.hide();
        }
        self.set_grab(true);
    }

    /// Redraws the pause menu overlay (after it opens, the window resizes or the hover changes).
    fn repaint_pause(&mut self) {
        let map = self.scene_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let status = self.net.as_ref().map(|n| n.status.clone());
        let hover = self.pause_hover;
        if let Some(gpu) = self.gpu.as_mut() {
            let (w, h) = (gpu.config.width, gpu.config.height);
            if let Some(live) = gpu.live.as_mut() {
                live.overlay.set(&gpu.device, &gpu.queue, w, h, &menu::paint_pause(w, h, &map, status.as_deref(), hover));
            }
        }
    }

    fn set_grab(&mut self, grabbed: bool) {
        let Some(window) = &self.window else { return };
        if grabbed {
            let ok = window.set_cursor_grab(CursorGrabMode::Locked).is_ok()
                || window.set_cursor_grab(CursorGrabMode::Confined).is_ok();
            if ok {
                window.set_cursor_visible(false);
                self.grabbed = true;
                self.grabbed_at = Instant::now();
            }
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
            self.grabbed = false;
        }
    }

    /// Toggles borderless fullscreen (covers the whole monitor, no taskbar/decorations) against
    /// the maximized windowed state the app launches in. Not exclusive fullscreen — that
    /// involves a display video-mode switch, which is unnecessary here and would fight the
    /// "fit whatever screen it's on" launch behavior.
    fn toggle_fullscreen(&self) {
        let Some(window) = &self.window else { return };
        if window.fullscreen().is_some() {
            window.set_fullscreen(None);
            window.set_maximized(true);
        } else {
            window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        }
    }

    fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::FirstPerson => ViewMode::ThirdPerson,
            ViewMode::ThirdPerson => ViewMode::FirstPerson,
        };
    }

    /// Updates the player body's world transform and pose for this frame: facing/position from
    /// `planar_pos`/`yaw_deg` (the pre-third-person-pullback player position — the character
    /// moves, the camera just watches it from farther away in third person), a walk cycle driven
    /// by `speed` when moving, and a subtle idle sway otherwise. The right arm's shoulder/elbow
    /// are additionally blended toward the swing pose (via `swing_blend`) whenever a bat
    /// swing is in progress, so a melee attack visibly moves the arm instead of just the weapon.
    /// Also computes `self.hand_prop_transform` — the third-person bat rigidly welded to
    /// that same right forearm bone — and flips both the body's and the hand prop's scale
    /// between `HIDDEN_SCALE` and life-size depending on `self.view_mode`, since there's no
    /// per-object render-visibility flag to hide the player's own body in first person instead.
    fn update_player_body(&mut self, planar_pos: Vec2, yaw_deg: f32, speed: f32, dt: f32) {
        let third_person = self.view_mode == ViewMode::ThirdPerson;
        let scale = if third_person { Vec3::ONE } else { Vec3::splat(HIDDEN_SCALE) };
        let body_pos = Vec3::new(planar_pos.x, self.foot_y, planar_pos.y);
        {
            let body = &mut self.scene.objects[self.player_object_index];
            body.position = Track::constant(body_pos);
            body.rotation = Track::constant(Vec3::new(0.0, yaw_deg, 0.0));
            body.scale = Track::constant(scale);
        }
        match self.character {
            Character::Human => self.pose_human(body_pos, yaw_deg, speed, dt, scale),
            Character::Rat => self.pose_rat(speed, dt),
        }
    }

    /// Cheddar's gait: legs and body bob follow distance travelled, the tail streams when he
    /// runs and idles with a gentle sway when he stops.
    fn pose_rat(&mut self, speed: f32, dt: f32) {
        self.walk_phase += dt * speed * RAT_GAIT_RAD_PER_M;
        let stride = (speed / self.body.sprint_speed).clamp(0.0, 1.0);
        let (gait, sway) = (self.walk_phase, self.start.elapsed().as_secs_f32() * 1.3);
        if let ObjectKind::Rat(r) = &mut self.scene.objects[self.player_object_index].kind {
            r.gait = Track::constant(gait);
            r.stride = Track::constant(stride);
            r.sway = Track::constant(sway);
        }
        self.hand_prop_transform = Mat4::from_scale(Vec3::splat(HIDDEN_SCALE));
    }

    /// The human's walk cycle and bat swing (see `update_player_body`'s notes above): the walk
    /// cycle is driven by `speed`, the right arm blends toward the swing pose, and the
    /// third-person bat is welded to the right forearm bone.
    fn pose_human(&mut self, body_pos: Vec3, yaw_deg: f32, speed: f32, dt: f32, scale: Vec3) {
        if speed > 0.0 {
            self.walk_phase += dt * speed * (WALK_CYCLES_PER_SEC_AT_WALK_SPEED / self.body.walk_speed) * std::f32::consts::TAU;
        }

        let (spine_x, l_hip_x, r_hip_x, l_knee, r_knee, l_sh_x, r_sh_x) = if speed > 0.0 {
            let ph = self.walk_phase;
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
            let idle_t = self.start.elapsed().as_secs_f32();
            (IDLE_SWAY_DEG * (idle_t * 1.1).sin(), 0.0, 0.0, KNEE_REST_DEG, KNEE_REST_DEG, 0.0, 0.0)
        };

        // The right arm additionally blends toward the bat's windup/strike pose during a
        // swing — `r_sh_x` (this frame's walk-cycle value) is the blend's idle endpoint, so a
        // swing mid-stride recovers back into whatever the walk cycle is doing by then rather
        // than snapping to a fixed rest angle.
        let carrying = self.carrying();
        let aiming = !carrying && self.shown_weapon() == Weapon::Revolver;
        let kick = self.recoil_kick();
        // Carrying: both arms forward, elbows bent, holding the prop out in front of the chest.
        // Aiming the revolver: the gun arm out level (following where the player looks), kicking on a shot.
        let aim_shoulder = (AIM_SHOULDER_X - self.camera.pitch.to_degrees() + 14.0 * kick).clamp(-175.0, -20.0);
        let l_sh_x_final = if carrying {
            CARRY_SHOULDER_X
        } else if aiming {
            aim_shoulder
        } else {
            self.swing_blend(l_sh_x, ARM_WINDUP_SHOULDER_X, ARM_STRIKE_SHOULDER_X)
        };
        let l_elbow_final = if carrying {
            CARRY_ELBOW_DEG
        } else if aiming {
            AIM_ELBOW_DEG + 22.0 * kick
        } else {
            self.swing_blend(ARM_IDLE_ELBOW_DEG, ARM_WINDUP_ELBOW_DEG, ARM_STRIKE_ELBOW_DEG)
        };
        let r_sh_x = if carrying { CARRY_SHOULDER_X } else { r_sh_x };
        let r_elbow = if carrying { CARRY_ELBOW_DEG } else { KNEE_REST_DEG };

        let third_person = self.view_mode == ViewMode::ThirdPerson && !carrying;
        let l_shoulder = Vec3::new(l_sh_x_final, 0.0, -6.0);
        let r_shoulder = Vec3::new(r_sh_x, 0.0, 6.0);
        if let ObjectKind::Humanoid(h) = &mut self.scene.objects[self.player_object_index].kind {
            h.pose.spine = Track::constant(Vec3::new(spine_x, 0.0, 0.0));
            h.pose.l_hip = Track::constant(Vec3::new(l_hip_x, 0.0, 0.0));
            h.pose.r_hip = Track::constant(Vec3::new(r_hip_x, 0.0, 0.0));
            h.pose.l_knee = Track::constant(l_knee);
            h.pose.r_knee = Track::constant(r_knee);
            h.pose.l_shoulder = Track::constant(l_shoulder);
            h.pose.r_shoulder = Track::constant(r_shoulder);
            h.pose.l_elbow = Track::constant(l_elbow_final);
            h.pose.r_elbow = Track::constant(r_elbow);
        }

        // Weld the third-person bat to the right forearm bone: run the same forward-kinematic
        // solve the renderer uses (`skeleton::pose_to_parts`) with this frame's exact pose, take
        // the forearm bone's world transform, and place the grip at its far (wrist) end. The
        // forearm's own rotation (from `capsule_between`'s `Quat::from_rotation_arc(Y, dir)`) only
        // pins its local Y axis to the elbow-to-wrist direction — roll around that axis is
        // otherwise arbitrary, so `HAND_GRIP_ROLL_DEG` is a fixed fudge rather than a derived
        // value; it reads fine in practice since the arm doesn't twist much in this rig.
        self.hand_prop_transform = if third_person {
            let rig = HumanoidRig::new(PLAYER_HEIGHT, PLAYER_BUILD);
            let pose = PoseSample {
                spine: Vec3::new(spine_x, 0.0, 0.0),
                head: Vec3::ZERO,
                l_shoulder,
                r_shoulder,
                l_elbow: l_elbow_final,
                r_elbow: KNEE_REST_DEG,
                l_hip: Vec3::new(l_hip_x, 0.0, 0.0),
                r_hip: Vec3::new(r_hip_x, 0.0, 0.0),
                l_knee,
                r_knee,
            };
            let forearm = &pose_to_parts(&rig, &pose)[BAT_FOREARM_PART];
            let body_world = Mat4::from_scale_rotation_translation(scale, Quat::from_rotation_y(yaw_deg.to_radians()), body_pos);
            let hand_bone_world = body_world
                * Mat4::from_rotation_translation(forearm.rotation, forearm.center)
                * Mat4::from_translation(Vec3::new(0.0, forearm.length * 0.5, 0.0));
            let wrist = hand_bone_world.transform_point3(Vec3::ZERO);
            let (fwd, right) = (self.camera.forward_flat(), self.camera.right_flat());
            let basis = Mat4::from_cols(right.extend(0.0), Vec3::Y.extend(0.0), fwd.extend(0.0), Vec4::new(0.0, 0.0, 0.0, 1.0));
            if aiming {
                // The revolver aims where the player looks (pitch tips the barrel, recoil kicks it up).
                Mat4::from_translation(wrist) * basis * Mat4::from_rotation_x((-self.camera.pitch) + (-0.30 * kick))
            } else {
                let pitch_deg = self.swing_blend(IDLE_PITCH_DEG, WINDUP_PITCH_DEG, STRIKE_PITCH_DEG);
                Mat4::from_translation(wrist) * basis * Mat4::from_rotation_z(IDLE_ROLL_DEG.to_radians()) * Mat4::from_rotation_x(pitch_deg.to_radians())
            }
        } else {
            Mat4::from_scale(Vec3::splat(HIDDEN_SCALE))
        };
    }

    /// What the player is asking for this tick, from the held keys (or the `RE2_AUTOWALK` debug script).
    fn build_input(&mut self) -> PlayerInput {
        if let Some(mode) = self.autowalk.clone() {
            // "circle" or "circle:DEGREES_PER_SECOND" turns while walking; anything else walks straight.
            if let Some(rate) = mode.strip_prefix("circle") {
                let dps = rate.trim_start_matches(':').parse::<f32>().unwrap_or(40.0);
                self.camera.yaw += dps.to_radians() * FIXED_DT;
            }
            return PlayerInput { forward: 1, sprint: false, yaw: self.camera.yaw, pitch: self.camera.pitch, ..Default::default() };
        }
        let held = |a: KeyCode, b: KeyCode| self.keys.contains(&a) || self.keys.contains(&b);
        let axis = |pos: bool, neg: bool| pos as i8 - neg as i8;
        PlayerInput {
            seq: 0,
            forward: axis(held(KeyCode::KeyW, KeyCode::ArrowUp), held(KeyCode::KeyS, KeyCode::ArrowDown)),
            strafe: axis(held(KeyCode::KeyD, KeyCode::ArrowRight), held(KeyCode::KeyA, KeyCode::ArrowLeft)),
            jump: std::mem::take(&mut self.jump_queued),
            sprint: self.sprint_held,
            crouch: held(KeyCode::ControlLeft, KeyCode::ControlRight),
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
        }
    }

    /// One fixed-size (`FIXED_DT`) physics step: movement/collision + jump/gravity, sampling
    /// currently-held input fresh (input state doesn't change within a rendered frame between
    /// steps). Snapshots the pre-step planar position/foot height into `prev_physics_pos`/
    /// `prev_foot_y` first, so `update` can interpolate between them for the actual rendered
    /// frame instead of drawing exactly on whichever physics step boundary just landed.
    fn fixed_step_physics(&mut self) {
        self.prev_physics_pos = self.physics_pos;
        self.prev_foot_y = self.foot_y;

        // Movement is the shared pure function `sim::player::step_player` (the single-player game, the
        // authoritative server and a client's prediction all run it). Online, the predictor owns the state
        // and also sends the input to the server.
        let input = self.build_input();
        let mut st = PlayerState { pos: self.physics_pos, foot_y: self.foot_y, vy: self.vertical_velocity, yaw: self.camera.yaw, pitch: self.camera.pitch, character: self.character };
        self.last_move_speed = 0.0;
        if let Some(net) = self.net.as_mut() {
            if let Some(s2) = net.step_local(input, Instant::now()) {
                st = s2;
                self.last_move_speed = net.last_speed;
            }
        } else {
            self.last_move_speed = step_player(&mut st, &input, &self.colliders, &self.ground);
        }
        self.physics_pos = st.pos;
        self.foot_y = st.foot_y;
        self.vertical_velocity = st.vy;

        // Loose props: the player's body shoves what it walks into, then the world steps.
        let d = (self.physics_pos - self.prev_physics_pos) / FIXED_DT;
        self.player_vel = Vec3::new(d.x, 0.0, d.y);
        if let Some(props) = &mut self.props {
            props.set_player(Vec3::new(self.physics_pos.x, self.foot_y, self.physics_pos.y), self.body.radius, self.body.body_height);
            props.step();
        }

        self.fixed_step_combat();
    }

    /// The player's eye at the latest completed tick (the origin of this tick's swings and shots).
    /// Unlike `self.eye` it does not depend on the render frame.
    fn tick_eye(&self) -> Vec3 {
        Vec3::new(self.physics_pos.x, self.foot_y + self.eye_height, self.physics_pos.y)
    }

    /// Weapon logic for one tick: advance the timers (an action queued on tick T first advances on
    /// tick T+1), resolve a landing bat strike, then perform the input queued since the last tick.
    fn fixed_step_combat(&mut self) {
        if self.net.is_some() {
            self.attack_queued = false;
            self.switch_queued = None;
            return; // weapons are not networked yet
        }
        let strike = self.swing.tick();
        self.shot_cd.tick();
        self.switch.tick();

        if strike {
            let eye = self.tick_eye();
            if let Some((object_index, distance, loose)) = self.melee_probe(eye) {
                // A loose prop gets knocked, too: light things fly, heavy ones shuffle.
                if let (Some(prop), Some(props)) = (loose, self.props.as_mut()) {
                    let dir = self.camera.forward();
                    props.strike(prop, dir, eye + dir * distance);
                }
                self.hit_with(object_index);
            }
        }

        if let Some(dir) = self.switch_queued.take() {
            self.begin_switch(dir);
        }
        if std::mem::take(&mut self.attack_queued) && self.body.has_bat && !self.carrying() && !self.switch.is_active() {
            match self.weapon {
                Weapon::Bat => {
                    self.swing.start();
                }
                Weapon::Revolver => self.fire_revolver(),
            }
        }
    }

    /// True while carrying a prop.
    fn carrying(&self) -> bool {
        self.props.as_ref().is_some_and(|p| p.held().is_some())
    }

    /// E: drop what is carried (it keeps the player's momentum), else pick up the prop under the crosshair.
    fn interact(&mut self) {
        if self.net.is_some() {
            return; // pick-up is not networked yet
        }
        let toss = self.camera.forward_flat() * 1.0;
        let Some(props) = self.props.as_mut() else { return };
        if props.held().is_some() {
            props.drop_held(self.player_vel + toss);
        } else if let Some(p) = self.pickup_target {
            props.pick_up(p);
            self.swing.cancel();
            self.swing_timer = None;
        }
    }

    /// What a swing from `eye` along the camera would strike: fixed geometry (exact shapes) or a
    /// loose prop, whichever is nearer. `(scene object index, distance, loose-prop index)`.
    fn melee_probe(&self, eye: Vec3) -> Option<(usize, f32, Option<usize>)> {
        self.probe(eye, MELEE_REACH)
    }

    /// [`melee_probe`](Self::melee_probe) for any reach (a bullet travels 80 m).
    fn probe(&self, eye: Vec3, reach: f32) -> Option<(usize, f32, Option<usize>)> {
        let dir = self.camera.forward();
        let fixed = raycast_shapes(eye, dir, reach, &self.hit_shapes).map(|h| (h.object_index, h.distance, None));
        let loose = self
            .props
            .as_ref()
            .and_then(|p| p.ray_props(eye, dir, reach).map(|(prop, d)| (p.props()[prop].object_index, d, Some(prop))));
        match (fixed, loose) {
            (Some(f), Some(l)) => Some(if l.1 < f.1 { l } else { f }),
            (f, l) => f.or(l),
        }
    }

    fn update(&mut self, dt: f32) {
        // Online, the connection must be serviced even when the window is not focused (or the server
        // would time us out), so the simulation keeps ticking; offline it pauses like before.
        if !self.grabbed && self.net.is_none() {
            return;
        }
        if let Some(net) = self.net.as_mut() {
            let now = Instant::now();
            net.poll(now);
            if let Some(st) = net.teleport.take() {
                // Joined, or resumed after a reconnect: go where the server says.
                self.physics_pos = st.pos;
                self.prev_physics_pos = st.pos;
                self.foot_y = st.foot_y;
                self.prev_foot_y = st.foot_y;
                self.vertical_velocity = 0.0;
                self.camera.yaw = st.yaw;
            }
            // Reconciliation may have nudged the predicted state; carry the difference through so the
            // interpolation below does not lurch (the visual offset is added back after it).
            if let Some(st) = net.state() {
                let delta = st.pos - self.physics_pos;
                self.prev_physics_pos += delta;
                self.physics_pos = st.pos;
                self.foot_y = st.foot_y;
                self.vertical_velocity = st.vy;
            }
            if self.net_title_at.elapsed().as_secs_f32() > 1.0 {
                self.net_title_at = Instant::now();
                if let Some(window) = &self.window {
                    window.set_title(&format!("Red Engine 2 — {} — {} — {}", self.scene_path.display(), self.character.name(), net.status));
                }
            }
        }

        // Accumulate real time and drain it in fixed-size chunks (the standard "fix your
        // timestep" pattern) — capped so a long stall (window drag, debugger pause) resumes from
        // where it left off instead of trying to replay minutes of physics in one frame.
        self.clock.push_time(dt);
        while self.clock.next_tick().is_some() {
            self.fixed_step_physics();
        }
        let alpha = self.clock.alpha();
        let mut planar_pos = self.prev_physics_pos.lerp(self.physics_pos, alpha);
        if let Some(net) = &self.net {
            planar_pos += net.visual_offset();
        }
        let foot_y = self.prev_foot_y + (self.foot_y - self.prev_foot_y) * alpha;

        let crouching = self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight);
        let forward_held = self.keys.contains(&KeyCode::KeyW) || self.keys.contains(&KeyCode::ArrowUp);
        let back_held = self.keys.contains(&KeyCode::KeyS) || self.keys.contains(&KeyCode::ArrowDown);
        let sprinting = self.sprint_held && forward_held && !back_held && !crouching && self.body.sprint_speed > self.body.walk_speed;

        // Crouch: blend the eye height toward its target instead of snapping, so the camera
        // doesn't jump-cut when Ctrl is pressed/released.
        let target_eye_height = if crouching { self.body.crouch_eye } else { self.body.stand_eye };
        let blend = (dt / CROUCH_TRANSITION_TIME).min(1.0);
        self.eye_height += (target_eye_height - self.eye_height) * blend;

        // Update the player's own body (position/facing/pose) from the interpolated
        // (pre-third-person-pullback) planar position, then place the camera: directly at the
        // eye in first person, or pulled back behind/above it in third person. This order
        // matters — the body must be placed before `self.camera.position` is potentially
        // overwritten by the third-person pullback below.
        let body_yaw_deg = 180.0 - self.camera.yaw.to_degrees();
        self.update_player_body(planar_pos, body_yaw_deg, self.last_move_speed, dt);

        let anchor = Vec3::new(planar_pos.x, foot_y + self.eye_height, planar_pos.y);
        self.eye = anchor;
        // Cosmetic timers run on render time; everything that decides a hit is in `fixed_step_combat`.
        self.since_shot += dt;
        self.flash_left = (self.flash_left - dt).max(0.0);
        // The animation reads mirrors of the tick-based swing/switch state, smoothed by `alpha`.
        self.swing_timer = self.swing.elapsed_secs(alpha);
        self.switching = self.switch.elapsed_secs(alpha);
        self.camera.position = match self.view_mode {
            ViewMode::FirstPerson => anchor,
            ViewMode::ThirdPerson => {
                let desired = anchor - self.camera.forward() * self.body.third_person_distance + Vec3::Y * self.body.third_person_lift;
                let active = colliders_on_floor(&self.colliders, foot_y);
                let cam_radius = THIRD_PERSON_CAM_RADIUS.min(self.body.radius * 0.7);
                let clamped = resolve_collision(Vec2::new(desired.x, desired.z), cam_radius, &active);
                Vec3::new(clamped.x, desired.y, clamped.y)
            }
        };

        // Sprint FOV kick, blended the same way as the crouch height.
        let target_fov = if sprinting { BASE_FOV_DEG + SPRINT_FOV_BOOST_DEG } else { BASE_FOV_DEG };
        let fov_blend = (dt / FOV_TRANSITION_TIME).min(1.0);
        self.fov_deg += (target_fov - self.fov_deg) * fov_blend;
        self.camera.fov_deg = self.fov_deg;

        // Loose props: keep a carried one in front of the player, write every prop's physics pose into
        // the scene, and see what the crosshair could pick up.
        if let Some(props) = &mut self.props {
            if let Some(h) = props.held() {
                let pose = props.hold_pose(h, anchor, self.camera.forward(), self.body.radius, self.body.hold_drop, foot_y);
                props.set_held_pose(pose);
            }
            props.sync_scene(&mut self.scene);
            self.pickup_target = props.pick_target(anchor, self.camera.forward(), self.body.pickup_reach, &self.body.carry);
        }

        // What's the crosshair aimed at, within bat reach? (Drives the crosshair's gold "you
        // could hit this" state.) Tested against the objects' real shapes, not bounding boxes, so
        // it is gold only where a swing would actually connect. Only the human has a bat. The ray
        // starts at the player's eye (`anchor`), not the camera: in third person the camera hangs
        // metres behind the player, and testing from there "hit" things behind them.
        let reach = if self.shown_weapon() == Weapon::Revolver { REVOLVER_RANGE } else { MELEE_REACH };
        self.target_index = if self.body.has_bat && !self.carrying() { self.probe(anchor, reach).map(|(o, _, _)| o) } else { None };

        if let Some(t) = self.freeze_shot {
            self.since_shot = t;
            self.flash_left = if t < MUZZLE_FLASH_TIME { MUZZLE_FLASH_TIME * 0.9 } else { 0.0 };
        }
        if let Some(t) = self.freeze_swing {
            self.swing_timer = Some(t);
        }
        if let Some(net) = self.net.as_mut() {
            // Other players and the server's props, interpolated, into the scene the renderer draws.
            net.update_scene(&mut self.scene, Instant::now(), dt);
        }
    }

    fn draw(&mut self) {
        let weapon_transform = self.weapon_transform();
        let carrying = self.carrying();
        let shown_weapon = self.shown_weapon();
        let muzzle_flash = (self.flash_left / MUZZLE_FLASH_TIME).clamp(0.0, 1.0);
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(live) = gpu.live.as_mut() else { return };
        let Some((surface_tex, reconfigure)) = acquire_frame(&gpu.surface, &gpu.device, &gpu.config) else { return };
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let t = if self.scene.duration > 0.0 { self.start.elapsed().as_secs_f32() % self.scene.duration } else { 0.0 };
        let opts = FrameOptions {
            crosshair: true,
            viewmodel: !carrying,
            pickup: self.pickup_target.is_some(),
            weapon: shown_weapon,
            muzzle_flash,
        };
        live.render_ex(
            &gpu.device,
            &gpu.queue,
            &self.scene,
            t,
            &self.camera,
            &view,
            self.target_index.is_some(),
            weapon_transform,
            self.hand_prop_transform,
            opts,
        );
        gpu.queue.present(surface_tex);
        if reconfigure {
            gpu.surface.configure(&gpu.device, &gpu.config);
        }
    }

    /// One frame of the launch menu: the turning models behind, the text and panels over them.
    fn menu_frame(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(menu_live) = gpu.menu.as_mut() else { return };
        let Some((surface_tex, reconfigure)) = acquire_frame(&gpu.surface, &gpu.device, &gpu.config) else { return };
        let (w, h) = (gpu.config.width, gpu.config.height);
        let t = self.start.elapsed().as_secs_f32();
        menu::animate(&mut self.menu_scene, w as f32 / h as f32, t, self.character);
        let key = (w, h, self.character);
        if self.menu_painted != Some(key) {
            let map = self.scene_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            menu_live.overlay.set(&gpu.device, &gpu.queue, w, h, &menu::paint(w, h, self.character, &map));
            self.menu_painted = Some(key);
        }
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let hidden = Mat4::from_scale(Vec3::splat(HIDDEN_SCALE));
        menu_live.render_ex(
            &gpu.device,
            &gpu.queue,
            &self.menu_scene,
            t,
            &menu::menu_camera(),
            &view,
            false,
            hidden,
            hidden,
            FrameOptions { crosshair: false, viewmodel: false, pickup: false, ..FrameOptions::default() },
        );
        gpu.queue.present(surface_tex);
        if reconfigure {
            gpu.surface.configure(&gpu.device, &gpu.config);
        }
    }

    /// Leaves the menu: adds the chosen character's body to the map, builds everything that
    /// depends on it (colliders, hit shapes, the renderer) and captures the mouse.
    fn start_game(&mut self, who: Character) {
        self.character = who;
        self.body = who.body();
        self.player_object_index = self.scene.objects.len();
        self.scene.objects.push(build_player_object(who));
        // Loose props (chairs, crates, apples...) live in the rigid-body world, not in the static
        // collider lists: they move.
        let online = self.net_server.is_some();
        let (props, loose) = if let (Some(addr), Some(world)) = (self.net_server, self.net_world.take()) {
            // Online: the server owns the props; this client only draws them and predicts its own walking.
            let loose: HashSet<usize> = world.prop_objects.iter().copied().collect();
            self.colliders = world.colliders.clone();
            self.ground = world.ground.clone();
            let mut session = match NetSession::connect(addr, who, world, 0) {
                Ok(s) => s,
                Err(e) => fail_online(&format!("cannot open a network socket: {e}")),
            };
            session.add_avatar_pool(&mut self.scene); // before the renderer takes its meshes from the scene
            match session.wait_connected(5.0) {
                Ok(st) => {
                    self.physics_pos = st.pos;
                    self.prev_physics_pos = st.pos;
                    self.foot_y = st.foot_y;
                    self.prev_foot_y = st.foot_y;
                    self.camera.yaw = st.yaw;
                    println!("Joined {addr} as player {} at ({:.1}, {:.1}).", session.client.my_id().unwrap_or(0), st.pos.x, st.pos.y);
                }
                Err(e) => fail_online(&e),
            }
            self.net = Some(session);
            (None, loose)
        } else {
            let props = PropWorld::new(&self.scene, Some(self.player_object_index));
            let loose = props.movable_indices();
            println!("{} loose props (pick up with E).", loose.len());
            self.colliders = collect_box_colliders_except(&self.scene, &loose);
            self.ground = collect_ground_candidates_except(&self.scene, &loose);
            (Some(props), loose)
        };
        let _ = online;
        // The player's own body is in `scene.objects` so the renderer can draw it, but the bat must
        // never be able to hit it (e.g. looking down at your own feet), so it is skipped here.
        let player_index = self.player_object_index;
        self.hit_shapes = collect_hit_shapes_where(&self.scene, |i| i != player_index && !loose.contains(&i));
        self.props = props;
        self.eye_height = self.body.stand_eye;
        self.camera.position.y = self.body.stand_eye;
        self.camera.near = self.body.near_plane;
        if self.debug_third_person {
            self.view_mode = ViewMode::ThirdPerson;
        }
        // Debug: `RE2_WEAPON=revolver` starts with the revolver in hand (for screenshots).
        if who == Character::Human && std::env::var("RE2_WEAPON").is_ok_and(|v| v.eq_ignore_ascii_case("revolver")) {
            self.weapon = Weapon::Revolver;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.live = Some(LiveRenderer::new(&gpu.device, gpu.config.format, &self.scene, gpu.config.width, gpu.config.height));
            gpu.menu = None;
        }
        self.phase = Phase::Playing;
        println!("Playing as {}.", who.name());
        if let Some(window) = &self.window {
            window.set_title(&format!("Red Engine 2 — {} — {}", self.scene_path.display(), who.name()));
        }
        self.last_frame = Instant::now();
        self.net_title_at = Instant::now();
        self.set_grab(true);
    }

    /// Menu keyboard: 1 / 2 pick and start, arrows / A / D move the highlight, Enter or Space starts.
    fn menu_key(&mut self, code: KeyCode, event_loop: &ActiveEventLoop) {
        match code {
            KeyCode::Digit1 | KeyCode::Numpad1 => self.start_game(Character::Human),
            KeyCode::Digit2 | KeyCode::Numpad2 => self.start_game(Character::Rat),
            KeyCode::ArrowLeft | KeyCode::KeyA => self.character = Character::Human,
            KeyCode::ArrowRight | KeyCode::KeyD => self.character = Character::Rat,
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.start_game(self.character),
            KeyCode::Escape => event_loop.exit(),
            _ => {}
        }
    }
}

/// Gets the next swapchain frame (reconfiguring the surface if it went stale), plus whether it
/// should be reconfigured after presenting; `None` when there is no frame to draw this time.
fn acquire_frame(surface: &wgpu::Surface<'static>, device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> Option<(wgpu::SurfaceTexture, bool)> {
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(t) => Some((t, false)),
        wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some((t, true)),
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Validation => None,
        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
            surface.configure(device, config);
            None
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(format!("Red Engine 2 — {}", self.scene_path.display()))
            // Maximized (not exclusive fullscreen) so it snaps to whatever monitor it opens on
            // at that monitor's native work area — centered and taskbar-aware, unlike a fixed
            // inner size that could land off-center on a different-resolution display. The
            // inner size below is only the fallback if the window is ever un-maximized.
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .with_maximized(true);
        // Debug: `RE2_WINDOW=x,y,w,h` places a plain window (to tile two clients side by side).
        let attrs = match std::env::var("RE2_WINDOW").ok().map(|v| v.split(',').filter_map(|n| n.trim().parse::<i32>().ok()).collect::<Vec<_>>()) {
            Some(v) if v.len() == 4 => attrs
                .with_maximized(false)
                .with_inner_size(winit::dpi::PhysicalSize::new(v[2].max(320) as u32, v[3].max(240) as u32))
                .with_position(winit::dpi::PhysicalPosition::new(v[0], v[1])),
            _ => attrs,
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("failed to create window"));

        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).expect("failed to create GPU surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no compatible GPU adapter found (Red Engine 2 needs Vulkan, DX12, or Metal)");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("red-engine-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .expect("failed to create GPU device");

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        // With a forced character the game starts at once; otherwise the launch menu (whose
        // 3-D backdrop is its own tiny scene) shows first.
        let menu = self.forced_character.is_none().then(|| LiveRenderer::new(&device, format, &self.menu_scene, config.width, config.height));
        self.gpu = Some(GpuState { surface, device, queue, config, live: None, menu });
        self.window = Some(window);
        self.last_frame = Instant::now();
        if let Some(who) = self.forced_character {
            self.start_game(who);
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.config.width = size.width.max(1);
                    gpu.config.height = size.height.max(1);
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    for r in [gpu.live.as_mut(), gpu.menu.as_mut()].into_iter().flatten() {
                        r.resize(&gpu.device, gpu.config.width, gpu.config.height);
                    }
                }
                if self.paused {
                    self.repaint_pause();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.phase == Phase::Menu => {
                if let (PhysicalKey::Code(code), ElementState::Pressed) = (event.physical_key, event.state) {
                    self.menu_key(code, event_loop);
                }
            }
            WindowEvent::CursorMoved { position, .. } if self.phase == Phase::Menu => {
                self.cursor_x = position.x as f32;
                if let Some(gpu) = &self.gpu {
                    self.character = menu::character_at(gpu.config.width, self.cursor_x);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if self.phase == Phase::Menu => {
                if let Some(gpu) = &self.gpu {
                    let who = menu::character_at(gpu.config.width, self.cursor_x);
                    self.start_game(who);
                }
            }
            WindowEvent::CursorMoved { position, .. } if self.paused => {
                self.cursor = (position.x as f32, position.y as f32);
                let hover = self.gpu.as_ref().and_then(|g| menu::pause_action_at(g.config.width, g.config.height, self.cursor.0, self.cursor.1));
                if hover != self.pause_hover {
                    self.pause_hover = hover;
                    self.repaint_pause();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed && !event.repeat {
                        if code == KeyCode::Escape {
                            if self.paused {
                                self.leave_pause();
                            } else {
                                self.enter_pause();
                            }
                            return;
                        }
                        if self.paused && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
                            self.leave_pause();
                            return;
                        }
                    }
                    if self.paused {
                        return; // the menu owns the keyboard
                    }
                    if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight) {
                        self.sprint_held = event.state == ElementState::Pressed;
                    }
                    if code == KeyCode::Space && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.jump_queued = true;
                    }
                    if code == KeyCode::KeyF && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.toggle_fullscreen();
                    }
                    if code == KeyCode::KeyQ && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.toggle_view_mode();
                    }
                    if code == KeyCode::KeyE && event.state == ElementState::Pressed && !self.keys.contains(&code) && self.grabbed {
                        self.interact();
                    }
                    match event.state {
                        ElementState::Pressed => {
                            self.keys.insert(code);
                        }
                        ElementState::Released => {
                            self.keys.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if self.paused {
                    let hit = self.gpu.as_ref().and_then(|g| menu::pause_action_at(g.config.width, g.config.height, self.cursor.0, self.cursor.1));
                    match hit {
                        Some(PauseAction::Resume) => self.leave_pause(),
                        Some(PauseAction::Quit) => event_loop.exit(),
                        None => {}
                    }
                } else if !self.grabbed {
                    self.set_grab(true);
                } else {
                    // Acted on by the next simulation tick (`fixed_step_combat`).
                    self.attack_queued = true;
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.phase == Phase::Playing && self.grabbed => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                self.on_scroll(lines);
            }
            WindowEvent::Focused(false) => {
                // No key-release events arrive while unfocused: forget held keys so we do not walk on alone.
                self.keys.clear();
                self.sprint_held = false;
                self.set_grab(false);
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let raw_dt = (now - self.last_frame).as_secs_f32();
                let dt = raw_dt.min(0.1);
                self.last_frame = now;
                if let Some(st) = &mut self.stats {
                    st.frames += 1;
                    st.worst_ms = st.worst_ms.max(raw_dt * 1000.0);
                    let elapsed = st.window_start.elapsed().as_secs_f32();
                    if elapsed >= 2.0 {
                        println!("[stats] {:.0} fps  (avg {:.2} ms, worst {:.1} ms)", st.frames as f32 / elapsed, elapsed * 1000.0 / st.frames as f32, st.worst_ms);
                        *st = FrameStats { window_start: Instant::now(), frames: 0, worst_ms: 0.0 };
                    }
                }
                match self.phase {
                    Phase::Menu => self.menu_frame(),
                    Phase::Playing => {
                        self.update(dt);
                        self.draw();
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device_id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            if self.grabbed && self.grabbed_at.elapsed().as_secs_f32() > MOUSE_SETTLE_SECS {
                self.camera.look(dx as f32 * MOUSE_SENSITIVITY, -dy as f32 * MOUSE_SENSITIVITY);
            }
        }
    }
}

/// Windows glue for running as a GUI-subsystem process (no console of its own).
#[cfg(windows)]
mod win {
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn GetStdHandle(which: u32) -> isize;
        fn SetStdHandle(which: u32, handle: isize) -> i32;
    }
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;

    /// Re-attaches to the launching terminal (if any) and points stdout/stderr at it, unless they
    /// were already redirected to a file/pipe. Returns whether a parent console was found.
    pub fn attach_console() -> bool {
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                return false;
            }
            for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                let h = GetStdHandle(which);
                if h == 0 || h == -1 {
                    if let Ok(con) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
                        SetStdHandle(which, con.as_raw_handle() as isize);
                        std::mem::forget(con);
                    }
                }
            }
        }
        true
    }

    /// A modal error box, for failures that have nowhere else to be printed.
    pub fn message_box(title: &str, text: &str) {
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        unsafe {
            MessageBoxW(0, wide(text).as_ptr(), wide(title).as_ptr(), 0x10); // MB_ICONERROR
        }
    }
}

/// What the command line asked for.
struct Args {
    scene: PathBuf,
    who: Option<Character>,
    connect: Option<SocketAddr>,
}

/// Command line: `re2 [scene.json] [--as human|rat] [--connect HOST:PORT]` (the character can also come
/// from `RE2_CHARACTER`, the server from `RE2_CONNECT`); without a character the launch menu asks.
fn parse_args() -> Args {
    let mut scene = None;
    let mut who = std::env::var("RE2_CHARACTER").ok().and_then(|v| Character::parse(&v));
    let mut connect: Option<String> = std::env::var("RE2_CONNECT").ok().filter(|v| !v.is_empty());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--as" || a == "--character" {
            match args.next().as_deref().and_then(Character::parse) {
                Some(c) => who = Some(c),
                None => eprintln!("--as expects `human` or `rat`; showing the menu instead"),
            }
        } else if a == "--connect" {
            connect = args.next();
        } else if scene.is_none() {
            scene = Some(PathBuf::from(a));
        }
    }
    let connect = connect.map(|c| {
        let c = if c.contains(':') { c } else { format!("{c}:{}", red_engine2::net::DEFAULT_PORT) };
        c.to_socket_addrs().ok().and_then(|mut i| i.next()).unwrap_or_else(|| fail_online(&format!("'{c}' is not a valid HOST:PORT")))
    });
    Args { scene: scene.unwrap_or_else(|| PathBuf::from("examples/room.json")), who, connect }
}

/// Reports a fatal online-mode problem (message box when there is no console) and exits.
fn fail_online(msg: &str) -> ! {
    eprintln!("multiplayer: {msg}");
    #[cfg(windows)]
    win::message_box("Red Engine 2", &format!("Could not join the game:\n{msg}"));
    std::process::exit(2);
}

fn main() {
    #[cfg(windows)]
    let has_console = win::attach_console();
    env_logger::init();
    let Args { scene: scene_path, who: forced_character, connect } = parse_args();
    // Online, the client's map is loaded together with its hash and static collision (what the server has).
    let mut net_world = None;
    let loaded = if connect.is_some() {
        ClientWorld::load(&scene_path).map(|(scene, world)| {
            net_world = Some(world);
            scene
        })
        .map_err(|e| vec![e])
    } else {
        red_engine2::load_scene(&scene_path)
    };
    let scene = loaded.unwrap_or_else(|errs| {
        eprintln!("failed to load scene {}:", scene_path.display());
        for e in &errs {
            eprintln!("  {e}");
        }
        #[cfg(windows)]
        if !has_console {
            let list: Vec<String> = errs.iter().map(|e| format!("  {e}")).collect();
            win::message_box("Red Engine 2", &format!("Failed to load scene {}:\n{}", scene_path.display(), list.join("\n")));
        }
        std::process::exit(1);
    });

    println!("Red Engine 2 — {}", scene_path.display());
    println!("Pick Human (1) or Cheddar the rat (2) on the launch screen; `--as human|rat` skips it.");
    println!("WASD / arrow keys to walk, mouse to look, Shift to sprint forward, Space to jump, Ctrl to crouch.");
    println!("Human: left-click swings the bat. Cheddar: small, and always as fast as a human sprinting.");
    println!("Q to toggle first-/third-person view, F to toggle fullscreen / maximized.");
    println!("Click the window to capture the mouse, Escape to release it.");

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    if let Some(addr) = connect {
        println!("Online: will join {addr} once you pick a character. Weapons and pick-up are not networked yet.");
    }
    let mut app = App::new(scene, scene_path, forced_character, connect, net_world);
    event_loop.run_app(&mut app).expect("event loop error");
}
