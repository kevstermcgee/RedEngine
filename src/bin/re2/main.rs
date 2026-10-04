//! Red Engine 2 — a first-person, walk-around viewer for a red_engine2 scene, forked from
//! the original Red Engine to be the base for an online prop hunt game.
//!
//! `re2 [scene.json]` opens a window, drops you inside the scene at the camera's
//! default position, and lets you walk around and look at things: WASD or the arrow keys to
//! move, the mouse to look, Shift to sprint forward, Space for a small jump, Ctrl to crouch,
//! F to toggle borderless fullscreen / maximized, click to (re)capture the mouse, Escape to
//! release it, Q to toggle between first- and third-person view. There is no character selection:
//! a scene's `player.humans_play_as`, else `--as` / `RE2_CHARACTER`, else the Human. Started without `--connect`, `re2` first shows the
//! connect form (PLAY SOLO skips it). The human
//! holds a bat; left-click swings it, and anything the swing actually touches gets logged, thunks
//! and flashes — a swing through empty air is silent. Hitting things is the seeker's primary
//! action on objects; the crosshair turns gold when something is within bat reach. Cheddar is
//! small and moves at a human's sprint speed all the time. E picks up (and drops) a loose prop —
//! see `red_engine2::physics`. (Right-click is reserved for the hider's "choose an object to
//! replicate", then R — not built yet.)

use clap::Parser;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use red_engine2::audio::{synth_bat_hit, synth_weapon_click, Audio};
use red_engine2::characters::HUMAN_HEIGHT;
use red_engine2::collide::{
    collect_box_colliders_grouped_except, collect_ground_candidates_grouped_except, colliders_on_floor, resolve_collision, Collider2D, GroundCandidates,
};
use red_engine2::easing::Ease;
use red_engine2::hit::{collect_hit_shapes_where, raycast_shapes, HitShape};
use red_engine2::menu::{self, PauseAction};
use red_engine2::net::bot::ClientWorld;
use red_engine2::net::host::{HostOptions, LocalHost};
use red_engine2::net::session::NetSession;
use red_engine2::physics::PropWorld;
use red_engine2::player::{BodySpec, Character, FIXED_DT};
use red_engine2::schema::{Light, LightKind, Object, ObjectKind, Scene};
use red_engine2::sim::clock::TickClock;
use red_engine2::sim::combat::{Cooldown, MeleeSwing, WeaponSwitch};
use red_engine2::sim::player::{step_player_on_tuned, PlayerInput, PlayerState};
use red_engine2::sim::rules::Target;
use red_engine2::sim::rules_run::{RulePlayer, RulesEngine};
use red_engine2::sim::spawns::{parse_spawns, Spawn};
use red_engine2::skeleton::{pose_to_parts, HumanoidRig, PoseSample};
use red_engine2::track::Track;
use red_engine2::viewer::{viewmodel_transform, FpsCamera, FrameOptions, LiveRenderer};
use red_engine2::viewer::{IDLE_PITCH_DEG, IDLE_ROLL_DEG};
use red_engine2::weapons::{
    Ammo, Weapon, DRY_FIRE_COOLDOWN_TICKS, MUZZLE_FLASH_TIME, RECOIL_TIME, SWING_RECOVER_SECS, SWING_STRIKE_SECS, SWING_WINDUP_SECS, SWITCH_SECS,
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

mod ambient;
mod avatar;
mod cards;
mod controller;
mod dump;
mod events;
mod feedback;
mod frame;
mod headless;
mod help;
mod kc;
mod online;
mod project_browser;
mod shots;
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

/// Firearm aim-down-sights FOV and the time used to blend into/out of it.
const ADS_TRANSITION_TIME: f32 = 0.14;
// A game-y widened FOV while sprinting reads as speed even before the eye adjusts to how fast
// the walls are sliding by; it also smoothly signals when sprint actually kicks in vs. Shift
// being held but disallowed (crouching, or not moving forward).
const SPRINT_FOV_BOOST_DEG: f32 = 8.0;
const FOV_TRANSITION_TIME: f32 = 0.15;

// Hitting things with the bat is the seeker's main way to act on objects (a struck object makes a
// sound but does not change colour); `E` picks up / drops loose props; right-click smoothly aims a firearm.

// Bat viewmodel: idle pose and swing animation, both expressed as a pitch (rotation about
// the camera's local right axis, tipping the bar up/down) plus a forward lunge, in the
// camera-local frame `viewmodel_transform` expects (+X right, +Y up, +Z forward). Roll (rotation
// about the bar's own axis) stays fixed — it just leans the bat across the view for a
// less "straight ahead" held pose (the bat leans in across the view).
const VM_RIGHT: f32 = 0.10;
const VM_DOWN: f32 = 0.125;
const VM_FORWARD: f32 = 0.30;
const WINDUP_PITCH_DEG: f32 = red_engine2::avatar::WINDUP_PITCH_DEG;
const STRIKE_PITCH_DEG: f32 = red_engine2::avatar::STRIKE_PITCH_DEG;
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
use red_engine2::scene_pool::HIDDEN_SCALE;
/// Cheddar's gait phase advances this many radians per metre travelled (a quick scurry).
const RAT_GAIT_RAD_PER_M: f32 = red_engine2::avatar::RAT_GAIT_RAD_PER_M;

// Procedural walk cycle for the player body: leg/arm swing amplitude and a cycle rate defined
// relative to WALK_SPEED so sprinting/crouch-walking scale the animation's tempo with actual
// speed instead of playing at a fixed rate regardless of how fast the player is moving.
const WALK_CYCLES_PER_SEC_AT_WALK_SPEED: f32 = red_engine2::avatar::WALK_CYCLES_PER_SEC_AT_WALK_SPEED;
const HIP_SWING_DEG: f32 = red_engine2::avatar::HIP_SWING_DEG;
const KNEE_LIFT_DEG: f32 = red_engine2::avatar::KNEE_LIFT_DEG;
const KNEE_REST_DEG: f32 = red_engine2::avatar::KNEE_REST_DEG;
const SHOULDER_SWING_DEG: f32 = red_engine2::avatar::SHOULDER_SWING_DEG;
const IDLE_SWAY_DEG: f32 = red_engine2::avatar::IDLE_SWAY_DEG;

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
const BAT_FOREARM_PART: usize = red_engine2::avatar::HAND_FOREARM_PART;

// Swing pose for the bat arm in third person (shoulder raise/swing on the local
// right axis, elbow bend), driven by the same windup/strike/recover phases as the first-person
// viewmodel's pitch (see `weapon_transform`) so both views read as the same motion.
const ARM_WINDUP_SHOULDER_X: f32 = red_engine2::avatar::ARM_WINDUP_SHOULDER_X;
const ARM_STRIKE_SHOULDER_X: f32 = red_engine2::avatar::ARM_STRIKE_SHOULDER_X;
const ARM_IDLE_ELBOW_DEG: f32 = red_engine2::avatar::ARM_IDLE_ELBOW_DEG;
const ARM_WINDUP_ELBOW_DEG: f32 = red_engine2::avatar::ARM_WINDUP_ELBOW_DEG;
const ARM_STRIKE_ELBOW_DEG: f32 = red_engine2::avatar::ARM_STRIKE_ELBOW_DEG;
/// Seconds the lower-and-raise animation takes when scrolling between weapons
/// (whole ticks in the simulation, `weapons::SWITCH_TICKS`).
const SWITCH_TIME: f32 = SWITCH_SECS;
/// Scroll lines needed to change weapon (a notch of a wheel is one line; touchpads send fractions).
const SCROLL_LINES_PER_SWITCH: f32 = 1.0;
/// Online: how many ticks an action button (E, click, wheel, R) is held on the input sent to the server (the server acts on
/// the press, and the redundant input packets make three ticks robust against a lost datagram).
const NET_PULSE_TICKS: u8 = 3;
/// Arm pose for aiming a firearm in third person (shoulder raised to level, elbow nearly straight).
const AIM_SHOULDER_X: f32 = red_engine2::avatar::AIM_SHOULDER_X;
const AIM_ELBOW_DEG: f32 = red_engine2::avatar::AIM_ELBOW_DEG;
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
    let mut o = red_engine2::characters::character_object(who, "player_body");
    o.scale = Track::constant(Vec3::splat(HIDDEN_SCALE));
    o.collide = true;
    o
}

struct GpuState {
    /// The window's swap chain (`None` in a headless run, which only draws offscreen).
    surface: Option<wgpu::Surface<'static>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// Draws the map; built when the game starts (its scene includes the player's body).
    live: Option<LiveRenderer>,
    /// Draws the connect form's backdrop until then.
    backdrop: Option<LiveRenderer>,
}

/// Whether the connect form or the game itself is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
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
    collider_groups: Vec<Vec<Collider2D>>,
    ground_groups: Vec<GroundCandidates>,
    collision_object_ids: Vec<String>,
    /// Every solid leaf shape a swing can strike (see `red_engine2::hit`), excluding the player.
    hit_shapes: Vec<HitShape>,
    /// Loose props (pick up with E, drop, knock over); built when the game starts.
    props: Option<PropWorld>,
    /// The loose prop under the crosshair that this character can pick up.
    pickup_target: Option<usize>,
    camera: FpsCamera,
    phase: Phase,
    /// Who the player is.
    character: Character,
    body: BodySpec,
    /// `--as` / `RE2_CHARACTER` / the scene's policy: start at once.
    forced_character: Option<Character>,
    backdrop_scene: Scene,
    keys: HashSet<KeyCode>,
    controller: red_engine2::controller::Controller,
    pad: red_engine2::controller::Sample,
    focused: bool,
    project_maps: Vec<red_engine2::project_browser::MapEntry>,
    map_selection: Option<usize>,
    grabbed: bool,
    /// Take the mouse back after the next resize (a fullscreen change released it).
    regrab: bool,
    /// `--fullscreen`: enter borderless fullscreen as soon as the window exists.
    start_fullscreen: bool,
    /// When the mouse was last captured. Capturing recenters the cursor, which delivers one big
    /// spurious motion delta — without ignoring input briefly the camera spins away from the
    /// spawn heading the moment the window opens.
    grabbed_at: Instant,
    sprint_held: bool,
    jump_queued: bool,
    /// Left click waiting for the next simulation tick (swing or shot).
    attack_queued: bool,
    /// Held trigger for automatic weapons; cleared whenever mouse capture is released.
    attack_held: bool,
    /// Right mouse is held and the current firearm should aim down sights.
    ads_held: bool,
    /// Smoothed 0 (hip) .. 1 (sights) presentation blend.
    ads_blend: f32,
    /// The third-person chase camera used in a kart race (`kart_camera`).
    chase: red_engine2::kart_camera::ChaseCamera,
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
    /// How the server's identity is checked when joining (ADR 0044).
    transport: TransportChoice,
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
    /// The player's ammunition, one supply for every firearm (infinite unless the scene's `weapons.ammo` says otherwise).
    ammo: Ammo,
    /// Shot-to-shot delay, in ticks.
    shot_cd: Cooldown,
    /// The bat swing (simulation state, in ticks). `swing_timer` below is its render-side mirror.
    swing: MeleeSwing,
    /// The weapon switch (simulation state). `switching` below is its render-side mirror.
    switch: WeaponSwitch,
    /// The fixed 60 Hz clock: frames push real time in, ticks come out.
    clock: TickClock,
    /// Offline uses the same pure scene-rule state machine as `MatchSim`; online presents the
    /// server's repeated authoritative rule-state snapshot.
    rules: RulesEngine,
    /// The `persist` variables as last saved, so a save happens only when one changes.
    saved_vars: std::collections::BTreeMap<String, f64>,
    /// The sun's height at the previous tick (for the `sunrise` and `sunset` events).
    last_sun_elev: Option<f32>,
    /// Named targets for the rule `teleport` action.
    spawns: Vec<Spawn>,
    /// Most recent non-terminal rule event, shown briefly by the generic rules HUD.
    rule_event: Option<String>,
    rule_event_until: u64,
    /// Last `(width, height, content)` painted into the offline rules HUD.
    rule_hud_painted: Option<(u32, u32, String)>,
    /// The start or end card of the scene's `ui` block, when one is up (offline only; `cards.rs`).
    card: Option<cards::CardState>,
    /// A restart does not show the start card again.
    skip_start_card: bool,
    /// Seconds since the last shot (drives the recoil kick); starts settled.
    since_shot: f32,
    /// Seconds of muzzle flash left.
    flash_left: f32,
    /// The player's eye this frame (the origin of swings and shots).
    eye: Vec3,
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
    horizontal_velocity: Vec2,
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
    /// Every other synthesized sound of the game: guns, feedback cues, movement (see `feedback.rs`).
    sounds: red_engine2::sfx::SoundBank,
    /// The presentation of a fight: sound cues and screen effects.
    feel: red_engine2::feel::Feel,
    /// Tracers and sparks (`None` until the game starts and the pool is added to the scene).
    streaks: Option<red_engine2::streaks::Streaks>,
    /// Whether the music is audible (`N` toggles it; `RE2_MUSIC=0` starts without).
    music_on: bool,
    /// The countryside and the mood music of a scene with an `audio` block.
    ambient: Option<ambient::Ambient>,
    /// Whether sound effects play (the pause menu's SOUND toggle).
    sfx_on: bool,
    /// Whether the player's carried light is on (`scene.flashlight`; the `T` key toggles it). Meaningless
    /// (never shown, never lit) when the scene did not ask for a flashlight.
    flashlight_on: bool,
    /// The persisted preference the music/sfx toggles read and write (`red_engine2::settings`); `music_on`/
    /// `sfx_on` above are the *live* state, which can momentarily differ (e.g. `RE2_MUSIC=1` forces music on
    /// without changing what is saved).
    settings: red_engine2::settings::Settings,
    /// The key `settings` is saved under for this game (`red_engine2::settings::key_for`).
    settings_key: String,
    /// A one-line result to show on the pause menu (e.g. where the music was saved), overriding the online
    /// status line until the next pause-menu action replaces or clears it.
    pause_message: Option<String>,
    /// When this game hosts its own match: the switch that freezes it (the pause menu and losing focus set it).
    host_pause: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Footsteps played so far (picks the foot).
    step_count: u32,
    /// Online: whether the attack button was down on the previous tick (the server acts on the press).
    pred_prev_attack: bool,
    /// Whether the server had us dead last frame (to notice coming back to life).
    was_dead: bool,
    /// A line to flash on the HUD (a level-up) and the seconds it has left.
    notice: Option<(String, f32)>,
    /// Online: the weapon in hand has been synced from the server once (the first sync must not animate a switch).
    net_weapon_synced: bool,
    /// The smallest launch speed of the map's jump pads, if it has any (to tell a pad from a jump).
    pad_launch: Option<f32>,
    /// Debug: `RE2_FEEL=hit|kill|hurt|dead|low|protected|flash` holds that screen effect (for screenshots).
    debug_feel: Option<String>,
    /// Online with a firearm: an enemy is under the crosshair (it turns red).
    aim_enemy: bool,
    /// Debug: `RE2_LOG_CUES=1` prints every sound cue as it plays (to check what the feedback does without listening).
    log_cues: bool,
    /// Debug: `RE2_AUTOFIRE=1` (with `RE2_AUTOWALK`) pulls the trigger five times a second.
    autofire: bool,
    /// Debug: `RE2_AUTOAIM=1` (with `RE2_AUTOWALK`) turns to the nearest other player and fires (through walls: it does not look).
    autoaim: bool,
    /// Debug: `RE2_VIEW=third` starts in third person (for screenshots).
    debug_third_person: bool,
    /// Debug: `RE2_FREEZE_SHOT=<seconds since the shot>` holds the firearm's recoil/flash there.
    freeze_shot: Option<f32>,
    /// Debug: `RE2_FREEZE_SWING=<seconds>` holds the swing animation at that time (for screenshots).
    freeze_swing: Option<f32>,
    start: Instant,
    last_frame: Instant,
    /// `RE2_STATS=1`: print average frame time / FPS every couple of seconds (for measuring
    /// how a map performs without attaching a profiler).
    stats: Option<FrameStats>,
    /// `--headless`: no window; a script (or nothing) plays, the loop is `headless::run`.
    headless: bool,
    /// The frame size of a headless run (there is no window to ask); the HUD lays out at this size even with no GPU at all.
    virtual_size: Option<(u32, u32)>,
    /// What draws: `none`, `window` or `offscreen`.
    gpu_kind: String,
    /// Seconds of game time since the game started, and frames updated (a script's clock; screenshots are named by it).
    play_secs: f32,
    frame_no: u64,
    /// Screenshots asked for and taken (`shots.rs`).
    shots: shots::Shots,
    /// The HUD's text as of the last layout, `(widget id, text)`: what the player can read.
    hud_lines: Vec<(String, String)>,
    /// The last sound cues played `(game seconds, cue)` and how many of each kind so far.
    cue_log: std::collections::VecDeque<(f32, String)>,
    cue_counts: std::collections::BTreeMap<String, u32>,
    /// The kart race's sounds (which cue the HUD's changes ask for).
    kart_sound: red_engine2::kart_sound::KartSound,
    /// States stored by a script's `snapshot` steps.
    snapshots: Vec<(String, serde_json::Value)>,
    /// Everything that went wrong that a person would only notice by looking: warnings from the session, failed expectations, shots that could not be taken.
    failures: Vec<String>,
    /// `F3`: the debug overlay (frame rate, ping, the remote-player counters), its lines and the smoothed frame rate.
    debug_hud: bool,
    debug_text: Vec<String>,
    fps_avg: f32,
}

struct FrameStats {
    window_start: Instant,
    frames: u32,
    worst_ms: f32,
    /// Longest `update` and `draw` of the window, milliseconds: which half a slow frame was spent in.
    worst_update_ms: f32,
    worst_draw_ms: f32,
    /// Longest gamepad poll of the window, milliseconds.
    worst_pad_ms: f32,
}

impl App {
    /// Who to start playing as right away, if the game should skip the connect form: a scene policy or `--as`, or any `--connect`
    /// (the Human unless one of those says otherwise). `None` shows the form first.
    fn start_character(&self) -> Option<Character> {
        self.forced_character.or_else(|| self.net_server.is_some().then_some(Character::Human))
    }
}

impl App {
    fn new(scene: Scene, scene_path: PathBuf, forced_character: Option<Character>, net_server: Option<SocketAddr>, net_world: Option<ClientWorld>) -> Self {
        // The player's body is added by `start_game` once the character is chosen.
        let player_object_index = scene.objects.len();
        let project_maps = red_engine2::project_browser::maps_for(&scene_path);
        let character = forced_character.unwrap_or(Character::Human);
        let body = character.body();

        let cam = scene.camera.position.sample(0.0);
        let target = scene.camera.target.sample(0.0);
        let cam_yaw = (target.x - cam.x).atan2(-(target.z - cam.z)).to_degrees();
        // A validated scene with authored spawns parses here. The camera fallback below also
        // supports animated/offline scenes whose raw camera is not a constant triple.
        let spawns = std::fs::read_to_string(&scene_path)
            .ok()
            .and_then(|text| parse_spawns(&text).ok())
            .unwrap_or_else(|| vec![Spawn { id: "camera".into(), position: [cam.x, 0.0, cam.z], yaw_deg: cam_yaw, group: String::new() }]);
        // Offline play starts where a match would: at the first spawn, at its height, facing its way (ADR 2026-09-29-verification-honours-the-map).
        let spawn = spawns.first().map(|s| Vec3::from(s.position)).unwrap_or(Vec3::new(cam.x, 0.0, cam.z));
        let yaw = spawns.first().map_or(cam_yaw, |s| s.yaw_deg);
        let mut camera = FpsCamera::new(Vec3::new(spawn.x, spawn.y + body.stand_eye, spawn.z), yaw);
        camera.fov_deg = scene.player.fov_deg;
        let player_fov_deg = scene.player.fov_deg;
        // Debug: `RE2_PITCH=<degrees>` starts looking up (+) / down (-), for screenshots.
        if let Some(deg) = std::env::var("RE2_PITCH").ok().and_then(|v| v.parse::<f32>().ok()) {
            camera.pitch = deg.to_radians();
        }
        let scene_third_person = scene.player.third_person;
        let scene_ammo = scene.weapons.ammo;
        let pad_launch = scene.jump_pads.iter().map(|p| p.launch_speed).reduce(f32::min);
        let starting_weapon = scene.weapons.starting_weapon;
        let mut rules = RulesEngine::new(scene.rules.clone()).with_wrap(scene.player.expanse.wrap);
        let settings_key = red_engine2::settings::key_for(&scene_path);
        // What the game keeps between sessions (`persist`) comes back before the first tick.
        let saved_vars: std::collections::BTreeMap<String, f64> =
            if scene.rules.persist.is_empty() { Default::default() } else { red_engine2::settings::load_vars(&settings_key) };
        for (name, value) in &saved_vars {
            rules.set_var(name, *value);
        }
        let settings = red_engine2::settings::load(&settings_key);
        let mut audio = Audio::new();
        if let Some(a) = audio.as_mut() {
            a.set_sfx_enabled(settings.sfx);
        }
        App {
            window: None,
            gpu: None,
            scene,
            scene_path,
            colliders: Vec::new(),
            ground: GroundCandidates::default(),
            collider_groups: Vec::new(),
            ground_groups: Vec::new(),
            collision_object_ids: Vec::new(),
            hit_shapes: Vec::new(),
            props: None,
            pickup_target: None,
            camera,
            phase: Phase::Connect,
            character,
            body,
            forced_character,
            backdrop_scene: menu::backdrop_scene(),
            keys: HashSet::new(),
            controller: Default::default(),
            pad: Default::default(),
            focused: true,
            project_maps,
            map_selection: None,
            grabbed: false,
            regrab: false,
            start_fullscreen: false,
            grabbed_at: Instant::now(),
            sprint_held: false,
            jump_queued: false,
            attack_queued: false,
            attack_held: false,
            ads_held: false,
            ads_blend: 0.0,
            chase: red_engine2::kart_camera::ChaseCamera::new(),
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
            transport: TransportChoice::default(),
            autowalk: std::env::var("RE2_AUTOWALK").ok().filter(|v| !v.is_empty()),
            net_title_at: Instant::now(),
            switch_queued: None,
            shot_cd: Cooldown::default(),
            swing: MeleeSwing::default(),
            switch: WeaponSwitch::default(),
            clock: TickClock::default(),
            rules,
            saved_vars,
            last_sun_elev: None,
            spawns,
            rule_event: None,
            rule_event_until: 0,
            rule_hud_painted: None,
            card: None,
            skip_start_card: false,
            swing_timer: None,
            target_index: None,
            physics_pos: Vec2::new(spawn.x, spawn.z),
            prev_physics_pos: Vec2::new(spawn.x, spawn.z),
            foot_y: spawn.y,
            prev_foot_y: spawn.y,
            vertical_velocity: 0.0,
            horizontal_velocity: Vec2::ZERO,
            last_move_speed: 0.0,
            eye_height: body.stand_eye,
            fov_deg: player_fov_deg,
            view_mode: if scene_third_person { ViewMode::ThirdPerson } else { ViewMode::FirstPerson },
            player_object_index,
            walk_phase: 0.0,
            hand_prop_transform: Mat4::from_scale(Vec3::splat(HIDDEN_SCALE)),
            audio,
            hit_sound: synth_bat_hit(),
            sounds: red_engine2::sfx::SoundBank::new(),
            feel: red_engine2::feel::Feel::new(),
            streaks: None,
            music_on: false,
            ambient: None,
            sfx_on: settings.sfx,
            flashlight_on: true,
            settings,
            settings_key,
            pause_message: None,
            host_pause: None,
            step_count: 0,
            pred_prev_attack: false,
            was_dead: false,
            notice: None,
            net_weapon_synced: false,
            pad_launch,
            debug_feel: std::env::var("RE2_FEEL").ok().filter(|v| !v.is_empty()),
            autofire: std::env::var_os("RE2_AUTOFIRE").is_some(),
            autoaim: std::env::var_os("RE2_AUTOAIM").is_some(),
            aim_enemy: false,
            log_cues: std::env::var_os("RE2_LOG_CUES").is_some(),
            weapon: starting_weapon,
            switching: None,
            scroll_accum: 0.0,
            ammo: scene_ammo,
            since_shot: RECOIL_TIME,
            flash_left: 0.0,
            eye: Vec3::ZERO,
            click_sound: synth_weapon_click(),
            debug_third_person: std::env::var("RE2_VIEW").is_ok_and(|v| v == "third"),
            freeze_shot: std::env::var("RE2_FREEZE_SHOT").ok().and_then(|v| v.parse().ok()),
            freeze_swing: std::env::var("RE2_FREEZE_SWING").ok().and_then(|v| v.parse().ok()),
            start: Instant::now(),
            last_frame: Instant::now(),
            stats: std::env::var_os("RE2_STATS").map(|_| FrameStats {
                window_start: Instant::now(),
                frames: 0,
                worst_ms: 0.0,
                worst_update_ms: 0.0,
                worst_draw_ms: 0.0,
                worst_pad_ms: 0.0,
            }),
            headless: false,
            virtual_size: None,
            gpu_kind: "none".to_string(),
            play_secs: 0.0,
            frame_no: 0,
            shots: shots::Shots::default(),
            hud_lines: Vec::new(),
            cue_log: Default::default(),
            cue_counts: Default::default(),
            kart_sound: Default::default(),
            snapshots: Vec::new(),
            failures: Vec::new(),
            debug_hud: false,
            debug_text: Vec::new(),
            fps_avg: 60.0,
        }
    }
}

/// The transport options from the command line: a pinned fingerprint or CA (QUIC), or explicit development UDP.
#[derive(Default, Clone)]
struct TransportChoice {
    fingerprint: Option<String>,
    ca: Option<PathBuf>,
    server_name: Option<String>,
    dev_udp: bool,
}

impl TransportChoice {
    /// The client transport for `addr`, or why it will not connect (never a silent plaintext fallback).
    fn for_server(&self, addr: SocketAddr) -> Result<red_engine2::net::client::ClientTransportConfig, String> {
        red_engine2::net::client::ClientTransportConfig::choose(
            addr,
            self.fingerprint.as_deref(),
            self.ca.as_deref(),
            self.server_name.as_deref(),
            self.dev_udp,
        )
    }
}

/// What the resolved command line asked for.
struct Args {
    scene: PathBuf,
    who: Option<Character>,
    connect: Option<SocketAddr>,
    key: Option<String>,
    name: Option<String>,
    host: bool,
    fill: Option<usize>,
    bot_skill: Option<String>,
    headless: headless::Options,
    debug_help: bool,
    transport: TransportChoice,
    fullscreen: bool,
}

/// Red Engine 2 real-time game client.
#[derive(Parser)]
#[command(
    version,
    about,
    after_help = "Controls: WASD/arrow keys move, mouse looks, Shift sprints, Space jumps, Ctrl crouches, Q changes view, F toggles fullscreen. --host plays against the map's bots on this machine."
)]
struct CliArgs {
    /// Scene/map JSON to play.
    #[arg(default_value = "examples/room.json")]
    scene: PathBuf,
    /// Play as this character (there is no picker; a scene's `player.humans_play_as` wins).
    #[arg(long = "as", alias = "character", value_parser = parse_character_arg, value_name = "CHARACTER")]
    who: Option<Character>,
    /// Join a server (`:27015` is added when no port is given).
    #[arg(long, value_name = "HOST:PORT")]
    connect: Option<String>,
    /// Server join key.
    #[arg(long)]
    key: Option<String>,
    /// Multiplayer display name.
    #[arg(long)]
    name: Option<String>,
    /// Host the match on this machine and join it: the map's bots (or `--fill`) are the opponents, and nothing else has to be started.
    #[arg(long)]
    host: bool,
    /// With `--host`: how many players (you included) the match aims for; empty places are filled with bots (`0` = none).
    #[arg(long, value_name = "N")]
    fill: Option<usize>,
    /// With `--host`: the bots' level: rookie, easy, normal, hard, nightmare, or a number from 0 to 1.
    #[arg(long, value_name = "LEVEL")]
    bot_skill: Option<String>,
    /// Run without a window: the real client loop, played by `--script` or `--playtest`, with no renderer unless pictures are asked for.
    #[arg(long)]
    headless: bool,
    /// What the player does, as a JSON script (`red_engine2 describe playtest`); implies `--headless`.
    #[arg(long, value_name = "FILE")]
    script: Option<PathBuf>,
    /// Write the client's final state as JSON: the players drawn, the HUD's text, the sounds played, the crosshair, every failure counter.
    #[arg(long, value_name = "FILE")]
    dump: Option<PathBuf>,
    /// Save pictures at these seconds of game time (`--shot-at 5,10,20`); drawn offscreen, so no visible window or focus is needed.
    #[arg(long, value_name = "SECS", value_delimiter = ',')]
    shot_at: Vec<f32>,
    /// Where pictures go.
    #[arg(long, value_name = "DIR", default_value = "out/shots")]
    shot_dir: PathBuf,
    /// The frame size of a headless run.
    #[arg(long, value_name = "WxH", default_value = "1280x720")]
    size: String,
    /// A scripted session of the map (hosted here unless `--connect`): a spin, a walk, a fight, pictures from four camera positions, a contact sheet and a report.
    #[arg(long)]
    playtest: bool,
    /// With `--playtest`: how long to play, seconds.
    #[arg(long, value_name = "SECS")]
    secs: Option<f32>,
    /// With `--playtest`: how many pictures.
    #[arg(long, value_name = "N")]
    shots: Option<usize>,
    /// With `--playtest`: the folder for pictures, the contact sheet and the report.
    #[arg(long, value_name = "DIR")]
    out: Option<PathBuf>,
    /// Print the RE2_* debug switches and hotkeys, and exit.
    #[arg(long)]
    debug_help: bool,
    /// Open in borderless fullscreen (F or F11 toggles it while playing; the pause menu has a button).
    #[arg(long)]
    fullscreen: bool,
    /// The server's identity fingerprint (`sha256:...`, printed by `red_server`): join over QUIC + TLS 1.3 (or RE2_SERVER_FINGERPRINT).
    #[arg(long, value_name = "SHA256")]
    server_fingerprint: Option<String>,
    /// A CA bundle (PEM) the server's certificate must chain to: join over QUIC + TLS 1.3.
    #[arg(long, value_name = "PEM")]
    server_ca: Option<PathBuf>,
    /// The name in the server's certificate (with --server-ca; default: the address).
    #[arg(long)]
    server_name: Option<String>,
    /// Join a development server over plain UDP (not encrypted; trusted LAN only). Loopback servers use it without the flag.
    #[arg(long)]
    dev_udp: bool,
}

fn parse_character_arg(value: &str) -> Result<Character, String> {
    Character::parse(value).ok_or_else(|| "expected human, rat, wizard, cowboy, alien or robot".to_string())
}

fn resolved_character(requested: Option<Character>, scene_policy: Option<Character>) -> Option<Character> {
    scene_policy.or(requested)
}

/// Command line: `re2 [scene.json] [--as human|rat] [--connect HOST:PORT] [--key JOIN_KEY] [--name NAME]` (the character can also
/// come from `RE2_CHARACTER`, the server from `RE2_CONNECT`, the key from `RE2_KEY`, the name from `RE2_NAME`); without `--connect` the
/// connect form (server, key and name) shows first, and its PLAY SOLO button starts a local game. Nobody chooses a character.
fn parse_args() -> Args {
    let cli = CliArgs::parse();
    let who = cli.who.or_else(|| std::env::var("RE2_CHARACTER").ok().and_then(|value| Character::parse(&value)));
    let connect = cli.connect.or_else(|| std::env::var("RE2_CONNECT").ok().filter(|value| !value.is_empty()));
    let key = cli.key.or_else(|| std::env::var("RE2_KEY").ok().filter(|value| !value.is_empty()));
    let name = cli.name.or_else(|| std::env::var("RE2_NAME").ok().filter(|value| !value.is_empty()));
    let connect = connect.map(|c| {
        let c = if c.contains(':') { c } else { format!("{c}:{}", red_engine2::net::DEFAULT_PORT) };
        c.to_socket_addrs().ok().and_then(|mut i| i.next()).unwrap_or_else(|| fail_online(&format!("'{c}' is not a valid HOST:PORT")))
    });
    let size = cli
        .size
        .split_once(['x', 'X'])
        .and_then(|(w, h)| Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?)))
        .filter(|(w, h)| (64..=7680).contains(w) && (64..=4320).contains(h))
        .unwrap_or_else(|| {
            eprintln!("--size '{}' is not WxH between 64x64 and 7680x4320", cli.size);
            std::process::exit(2)
        });
    let headless = headless::Options {
        enabled: cli.headless || cli.script.is_some() || cli.playtest,
        script: cli.script,
        dump: cli.dump,
        shot_at: cli.shot_at,
        shot_dir: cli.shot_dir,
        size,
        playtest: cli.playtest,
        secs: cli.secs,
        shots: cli.shots,
        out: cli.out,
    };
    // A playtest hosts its own match unless it was told where to connect.
    let host = cli.host || (cli.playtest && connect.is_none());
    let transport = TransportChoice {
        fingerprint: cli.server_fingerprint.or_else(|| std::env::var("RE2_SERVER_FINGERPRINT").ok().filter(|v| !v.is_empty())),
        ca: cli.server_ca,
        server_name: cli.server_name,
        dev_udp: cli.dev_udp,
    };
    Args {
        scene: cli.scene,
        who,
        connect,
        key,
        name,
        host,
        fill: cli.fill,
        bot_skill: cli.bot_skill,
        headless,
        debug_help: cli.debug_help,
        transport,
        fullscreen: cli.fullscreen,
    }
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
    win::install_crash_box(&win::init(win::wants_terminal(&std::env::args().collect::<Vec<_>>())));
    env_logger::init();
    let Args {
        scene: scene_path,
        who: requested_character,
        mut connect,
        key,
        name,
        host,
        fill,
        bot_skill,
        headless: headless_options,
        debug_help,
        transport,
        fullscreen,
    } = parse_args();
    if debug_help {
        print!("{}", help::text());
        return;
    }
    // A map with a `shooter` block is Killchain-style (loadouts, teams, killcam): it has its own client with its own front end.
    if !headless_options.enabled && connect.is_none() && !host {
        let is_loadout = std::fs::read_to_string(&scene_path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .is_some_and(|v| v.get("shooter").is_some());
        if is_loadout {
            kc::run(kc::Options { scene: scene_path, fullscreen, name });
            return;
        }
    }
    // `--host`: serve the map from a thread of this process and join it; the server stops when the game closes (it drops after `app`).
    let mut local_host = None;
    if host {
        match LocalHost::start(&scene_path, &HostOptions { fill, bot_skill, ..Default::default() }) {
            Ok(h) => {
                println!("Hosting a match on {} (the map's bots fill the empty places).", h.addr());
                connect = Some(h.addr());
                local_host = Some(h);
            }
            Err(e) => fail_online(&e),
        }
    }
    // A server elsewhere needs a pinned identity (or an explicit development transport); a hosted or loopback match needs nothing.
    if let Some(addr) = connect {
        if let Err(e) = transport.for_server(addr) {
            fail_online(&e);
        }
    }
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
        std::process::exit(1);
    });
    // A game-authored policy wins over local flags: the authoritative server enforces the same
    // value, so exposing an impossible character choice would only create a correction after join.
    let forced_character = resolved_character(requested_character, scene.player.character);

    println!("Red Engine 2 — {}", scene_path.display());
    if let Some(character) = scene.player.character {
        println!("This game starts as {} (set by player.humans_play_as).", character.name());
    } else {
        println!("You play as the Human (--as CHARACTER or RE2_CHARACTER overrides).");
    }
    println!("WASD / arrow keys to walk, mouse to look, Shift to sprint forward, Space to jump, Ctrl to crouch.");
    match scene.player.character {
        Some(
            Character::Human
            | Character::Wizard
            | Character::Cowboy
            | Character::Alien
            | Character::Robot
            | Character::Ridgeback
            | Character::Nightfall
            | Character::Hollow,
        ) => {
            println!("Left-click / right trigger uses the equipped weapon.")
        }
        Some(Character::Boy) => println!("The boy: just walk, run and look around."),
        Some(Character::Rat) => println!("Cheddar is small and always as fast as a human sprinting."),
        None => println!("Human: left-click swings the bat. Cheddar: small, and always as fast as a human sprinting."),
    }
    println!("Q to toggle first-/third-person view, F to toggle fullscreen / maximized.");
    println!("Click the window to capture the mouse, Escape to release it.");

    if let Some(addr) = connect {
        println!("Online: will join {addr}.");
    }
    let mut app = App::new(scene, scene_path, forced_character, connect, net_world);
    app.host_pause = local_host.as_ref().map(|h| h.pause_flag());
    app.transport = transport;
    app.start_fullscreen = fullscreen;
    if key.is_some() {
        app.join_key = key;
    }
    if let Some(n) = name {
        app.online.form.name = n.clone();
        app.player_name = n;
    }
    if headless_options.enabled {
        // No window and no event loop: the same `App` is stepped by hand (`headless.rs`). The exit code says whether every expectation held.
        let code = headless::run(&mut app, headless_options);
        drop(app);
        drop(local_host);
        std::process::exit(code);
    }
    let event_loop = EventLoop::new().unwrap_or_else(|e| {
        let msg = e.to_string();
        if msg.contains("DISPLAY") || msg.contains("WAYLAND") {
            eprintln!("re2: no display available ({msg})");
            eprintln!("re2: this box can't open a window. Use --headless (with --script FILE or --playtest) for a run with no window; see `red_engine2 describe playtest`.");
            std::process::exit(2);
        }
        panic!("failed to create event loop: {e}")
    });
    event_loop.set_control_flow(ControlFlow::Poll);
    app.shots = shots::Shots::new(headless_options.shot_dir.clone(), headless_options.shot_at.clone());
    event_loop.run_app(&mut app).expect("event loop error");
    // Tearing down the GPU, the audio device and the server should take a moment; if anything wedges, the game still ends.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(4));
        std::process::exit(0);
    });
    drop(app);
    drop(local_host);
}

#[cfg(test)]
mod character_policy_tests {
    use super::*;

    #[test]
    fn a_scene_policy_skips_and_overrides_the_generic_picker() {
        assert_eq!(resolved_character(None, Some(Character::Human)), Some(Character::Human));
        assert_eq!(resolved_character(Some(Character::Rat), Some(Character::Human)), Some(Character::Human));
        assert_eq!(resolved_character(None, None), None);
    }
}
