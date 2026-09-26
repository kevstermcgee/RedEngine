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

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use red_engine2::audio::{synth_bat_hit, synth_revolver_shot, synth_weapon_click, Audio};
use red_engine2::characters::{human_object, rat_object, HUMAN_HEIGHT};
use red_engine2::collide::{
    collect_box_colliders_except, collect_ground_candidates_except, colliders_on_floor, resolve_collision, Collider2D, GroundCandidates,
};
use red_engine2::easing::Ease;
use red_engine2::hit::{collect_hit_shapes_where, raycast_shapes, HitShape};
use red_engine2::menu::{self, PauseAction};
use red_engine2::net::bot::ClientWorld;
use red_engine2::net::session::NetSession;
use red_engine2::physics::PropWorld;
use red_engine2::player::{BodySpec, Character, FIXED_DT};
use red_engine2::schema::{Object, ObjectKind, Scene};
use red_engine2::sim::clock::TickClock;
use red_engine2::sim::combat::{Cooldown, MeleeSwing, WeaponSwitch};
use red_engine2::sim::player::{step_player_on, PlayerInput, PlayerState};
use red_engine2::sim::rules::Target;
use red_engine2::sim::rules_run::{RulePlayer, RulesEngine};
use red_engine2::sim::spawns::{parse_spawns, Spawn};
use red_engine2::skeleton::{pose_to_parts, HumanoidRig, PoseSample};
use red_engine2::track::Track;
use red_engine2::viewer::{viewmodel_transform, FpsCamera, FrameOptions, LiveRenderer};
use red_engine2::viewer::{IDLE_PITCH_DEG, IDLE_ROLL_DEG};
use red_engine2::weapons::{
    Ammo, Weapon, DRY_FIRE_COOLDOWN_TICKS, MUZZLE_FLASH_TIME, RECOIL_TIME, REVOLVER_COOLDOWN_TICKS, REVOLVER_IMPULSE, REVOLVER_RANGE, SWING_RECOVER_SECS,
    SWING_STRIKE_SECS, SWING_WINDUP_SECS, SWITCH_SECS,
};
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

mod avatar;
mod events;
mod frame;
mod online;
mod weapons;
mod window;
use window::acquire_frame;
#[cfg(windows)]
mod win;

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
const MELEE_REACH: f32 = red_engine2::weapons::BAT_REACH;

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
/// Online: how many ticks an action button (E, click, wheel, R) is held on the input sent to the server (the server acts on
/// the press, and the redundant input packets make three ticks robust against a lost datagram).
const NET_PULSE_TICKS: u8 = 3;
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
    /// Typing a server address (the online connect form).
    Connect,
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
    cursor_y: f32,
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
    /// Online: ticks left to hold each action button (interact, attack, switch, reload) on the input sent to the server.
    /// The server acts on the press, so a short pulse is one action.
    net_pulse: [u8; 4],
    /// The Escape menu is showing (the mouse is free; single-player is frozen).
    paused: bool,
    pause_hover: Option<PauseAction>,
    cursor: (f32, f32),
    /// `--connect`: the multiplayer session (`None` = single player).
    net: Option<NetSession>,
    /// Address to join, and the client's view of the map, until `start_game` uses them.
    net_server: Option<SocketAddr>,
    net_world: Option<ClientWorld>,
    /// The online screens' state (connect form text, hover, what the overlay shows).
    online: online::OnlineUi,
    /// A session connected from the connect form, waiting for `start_game` to use it.
    pending_net: Option<NetSession>,
    /// `--key` / `RE2_KEY`: the server's join key.
    join_key: Option<String>,
    /// `--name` / `RE2_NAME`: the name shown in the lobby.
    player_name: String,
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
    /// Offline uses the same pure scene-rule state machine as `MatchSim`. Online rule state is
    /// authoritative on the server and is not replicated by protocol v3 yet.
    rules: RulesEngine,
    /// Named targets for the rule `teleport` action.
    spawns: Vec<Spawn>,
    /// Most recent non-terminal rule event, shown briefly by the generic rules HUD.
    rule_event: Option<String>,
    rule_event_until: u64,
    /// Last `(width, height, content)` painted into the offline rules HUD.
    rule_hud_painted: Option<(u32, u32, String)>,
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
        let scene_ammo = scene.weapons.revolver_ammo;
        let rules = RulesEngine::new(scene.rules.clone());
        // A validated scene with authored spawns parses here. The camera fallback below also
        // supports animated/offline scenes whose raw camera is not a constant triple.
        let spawns = std::fs::read_to_string(&scene_path)
            .ok()
            .and_then(|text| parse_spawns(&text).ok())
            .unwrap_or_else(|| vec![Spawn { id: "camera".into(), position: [spawn.x, 0.0, spawn.z], yaw_deg: yaw, group: String::new() }]);
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
            cursor_y: 0.0,
            keys: HashSet::new(),
            grabbed: false,
            grabbed_at: Instant::now(),
            sprint_held: false,
            jump_queued: false,
            attack_queued: false,
            net_pulse: [0; 4],
            paused: false,
            pause_hover: None,
            cursor: (0.0, 0.0),
            net: None,
            net_server,
            net_world,
            online: online::OnlineUi::new("127.0.0.1", "", &std::env::var("RE2_NAME").unwrap_or_default()),
            pending_net: None,
            join_key: std::env::var("RE2_KEY").ok().filter(|k| !k.is_empty()),
            player_name: std::env::var("RE2_NAME").unwrap_or_default(),
            autowalk: std::env::var("RE2_AUTOWALK").ok().filter(|v| !v.is_empty()),
            net_title_at: Instant::now(),
            switch_queued: None,
            shot_cd: Cooldown::default(),
            swing: MeleeSwing::default(),
            switch: WeaponSwitch::default(),
            clock: TickClock::default(),
            rules,
            spawns,
            rule_event: None,
            rule_event_until: 0,
            rule_hud_painted: None,
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
            ammo: scene_ammo,
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
}

/// What the command line asked for.
struct Args {
    scene: PathBuf,
    who: Option<Character>,
    connect: Option<SocketAddr>,
    key: Option<String>,
    name: Option<String>,
}

/// Command line: `re2 [scene.json] [--as human|rat] [--connect HOST:PORT] [--key JOIN_KEY] [--name NAME]` (the character can also
/// come from `RE2_CHARACTER`, the server from `RE2_CONNECT`, the key from `RE2_KEY`, the name from `RE2_NAME`); without a character the
/// launch menu asks, and its PLAY ONLINE button (or the O key) opens a form for the server, key and name.
fn parse_args() -> Args {
    let mut scene = None;
    let mut who = std::env::var("RE2_CHARACTER").ok().and_then(|v| Character::parse(&v));
    let mut connect: Option<String> = std::env::var("RE2_CONNECT").ok().filter(|v| !v.is_empty());
    let (mut key, mut name) = (None::<String>, None::<String>);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--as" || a == "--character" {
            match args.next().as_deref().and_then(Character::parse) {
                Some(c) => who = Some(c),
                None => eprintln!("--as expects `human` or `rat`; showing the menu instead"),
            }
        } else if a == "--connect" {
            connect = args.next();
        } else if a == "--key" {
            key = args.next();
        } else if a == "--name" {
            name = args.next();
        } else if scene.is_none() {
            scene = Some(PathBuf::from(a));
        }
    }
    let connect = connect.map(|c| {
        let c = if c.contains(':') { c } else { format!("{c}:{}", red_engine2::net::DEFAULT_PORT) };
        c.to_socket_addrs().ok().and_then(|mut i| i.next()).unwrap_or_else(|| fail_online(&format!("'{c}' is not a valid HOST:PORT")))
    });
    Args { scene: scene.unwrap_or_else(|| PathBuf::from("examples/room.json")), who, connect, key, name }
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
    let Args { scene: scene_path, who: forced_character, connect, key, name } = parse_args();
    // Online, the client's map is loaded together with its hash and static collision (what the server has).
    let mut net_world = None;
    let loaded = if connect.is_some() {
        ClientWorld::load(&scene_path)
            .map(|(scene, world)| {
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
    if key.is_some() {
        app.join_key = key;
    }
    if let Some(n) = name {
        app.online.form.name = n.clone();
        app.player_name = n;
    }
    event_loop.run_app(&mut app).expect("event loop error");
}
