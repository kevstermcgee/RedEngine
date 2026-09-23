//! Red Engine 2 — a first-person, walk-around viewer for a red_engine2 scene, forked from
//! the original Red Engine to be the base for an online prop hunt game.
//!
//! `re2 [scene.json]` opens a window, drops you inside the scene at the camera's
//! default position, and lets you walk around and look at things: WASD or the arrow keys to
//! move, the mouse to look, Shift to sprint forward, Space for a small jump, Ctrl to crouch,
//! F to toggle borderless fullscreen / maximized, click to (re)capture the mouse, Escape to
//! release it, Q to toggle between first- and third-person view. The player holds a crowbar;
//! left-click swings it, and anything within melee reach when the swing connects gets logged
//! plus a brief hit flash. Aim the crosshair at something else (it turns gold when something's
//! in reach) and press E to interact with it — for now that's a console log plus a brief
//! highlight flash, a placeholder for real object interaction (picking things up, opening
//! doors, ...) to build on later.

use red_engine2::color::parse_hex_to_linear;
use red_engine2::easing::Ease;
use red_engine2::schema::{HumanoidDef, Material, Object, ObjectKind, Pose, Scene};
use red_engine2::skeleton::{pose_to_parts, HumanoidRig, PoseSample};
use red_engine2::track::Track;
use red_engine2::viewer::{
    collect_box_colliders, collect_interactables, raycast_nearest, viewmodel_transform, Collider2D, FpsCamera,
    Interactable, LiveRenderer, resolve_collision,
};
use glam::{Mat4, Quat, Vec2, Vec3};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

const WALK_SPEED: f32 = 3.2;
const SPRINT_SPEED: f32 = 6.5;
const CROUCH_SPEED_MULT: f32 = 0.5;
const PLAYER_RADIUS: f32 = 0.35;
const STAND_EYE_HEIGHT: f32 = 1.7;
const CROUCH_EYE_HEIGHT: f32 = 1.05;
const CROUCH_TRANSITION_TIME: f32 = 0.12;
const MOUSE_SENSITIVITY: f32 = 0.0025;

const BASE_FOV_DEG: f32 = 70.0;
// A game-y widened FOV while sprinting reads as speed even before the eye adjusts to how fast
// the walls are sliding by; it also smoothly signals when sprint actually kicks in vs. Shift
// being held but disallowed (crouching, or not moving forward).
const SPRINT_FOV_BOOST_DEG: f32 = 8.0;
const FOV_TRANSITION_TIME: f32 = 0.15;

// Arcade-ish (stronger than real 9.8 m/s^2) gravity for a snappy, "small" hop rather than a
// floaty real-world jump: height = JUMP_SPEED^2 / (2 * GRAVITY) ~= 0.4m.
const GRAVITY: f32 = 18.0;
const JUMP_SPEED: f32 = 3.8;

// How far the player can reach to interact with something, and how the highlight flash on a
// just-interacted object fades back to its original look.
const INTERACT_REACH: f32 = 3.2;
const FLASH_DURATION: f32 = 0.35;
const FLASH_BOOST: Vec3 = Vec3::new(0.9, 0.75, 0.25);
// A hotter, redder flash than the interact one so a crowbar hit reads as an impact rather than
// a UI-style highlight.
const HIT_FLASH_BOOST: Vec3 = Vec3::new(1.0, 0.3, 0.12);

// Crowbar viewmodel: idle pose and swing animation, both expressed as a pitch (rotation about
// the camera's local right axis, tipping the bar up/down) plus a forward lunge, in the
// camera-local frame `viewmodel_transform` expects (+X right, +Y up, +Z forward). Roll (rotation
// about the bar's own axis) stays fixed — it just angles the hooked end across the view for a
// less "straight ahead" held pose.
const VM_RIGHT: f32 = 0.24;
const VM_DOWN: f32 = 0.20;
const VM_FORWARD: f32 = 0.40;
const IDLE_PITCH_DEG: f32 = -28.0;
const IDLE_ROLL_DEG: f32 = 22.0;
const WINDUP_PITCH_DEG: f32 = -70.0;
const STRIKE_PITCH_DEG: f32 = 55.0;
const STRIKE_LUNGE: f32 = 0.16;

// Swing phase durations (seconds) and the melee reach used for the hit-detection raycast fired
// once per swing, at the start of the strike phase.
const SWING_WINDUP: f32 = 0.09;
const SWING_STRIKE: f32 = 0.11;
const SWING_RECOVER: f32 = 0.16;
const SWING_TOTAL: f32 = SWING_WINDUP + SWING_STRIKE + SWING_RECOVER;
const MELEE_REACH: f32 = 2.2;

// Player body model ("skin"): a default humanoid rig standing in for the player, matched to
// STAND_EYE_HEIGHT's implied stature. Its own scale is toggled between this and a
// near-invisible value to fake per-view-mode visibility (see `update_player_body`), since the
// renderer has no per-object visibility flag to hide it in first person instead.
const PLAYER_HEIGHT: f32 = 1.8;
const PLAYER_BUILD: f32 = 1.0;
const PLAYER_SKIN_HEX: &str = "#4a5568";
const HIDDEN_SCALE: f32 = 0.0005;

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
const THIRD_PERSON_DISTANCE: f32 = 3.4;
const THIRD_PERSON_HEIGHT_OFFSET: f32 = 0.55;
const THIRD_PERSON_CAM_RADIUS: f32 = 0.25;

// Third-person hand attachment: the crowbar rigidly follows the right forearm bone (see
// `skeleton::pose_to_parts`, part index 5) rather than a fixed torso-relative offset, so it
// swings with the arm through both the walk cycle and a melee swing instead of floating in a
// fixed pose regardless of what the arm is doing. `HAND_GRIP_FORWARD` nudges the grip point a
// little past the wrist (roughly into the palm); `HAND_GRIP_ROLL_DEG` reuses the first-person
// viewmodel's idle roll so the hooked end reads the same way from either view.
const RIGHT_FOREARM_PART: usize = 5;
const HAND_GRIP_FORWARD: f32 = 0.05;
const HAND_GRIP_ROLL_DEG: f32 = IDLE_ROLL_DEG;

// Swing pose for the *visible* right arm in third person (shoulder raise/swing on the local
// right axis, elbow bend), driven by the same windup/strike/recover phases as the first-person
// viewmodel's pitch (see `weapon_transform`) so both views read as the same motion.
const ARM_WINDUP_SHOULDER_X: f32 = 55.0;
const ARM_STRIKE_SHOULDER_X: f32 = -95.0;
const ARM_IDLE_ELBOW_DEG: f32 = 8.0;
const ARM_WINDUP_ELBOW_DEG: f32 = 60.0;
const ARM_STRIKE_ELBOW_DEG: f32 = 12.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    FirstPerson,
    ThirdPerson,
}

/// Builds the player's own body: a default `humanoid` rig (see `SPEC.md`'s `humanoid` object)
/// added to the scene at runtime rather than authored in the scene JSON, since it represents
/// the player rather than the room. Its transform and pose are rewritten every frame by
/// `update_player_body`; the values here are just the resting/idle starting point.
fn build_player_object() -> Object {
    let pose = Pose {
        spine: Track::constant(Vec3::ZERO),
        head: Track::constant(Vec3::ZERO),
        l_shoulder: Track::constant(Vec3::new(0.0, 0.0, -6.0)),
        r_shoulder: Track::constant(Vec3::new(0.0, 0.0, 6.0)),
        l_elbow: Track::constant(KNEE_REST_DEG),
        r_elbow: Track::constant(KNEE_REST_DEG),
        l_hip: Track::constant(Vec3::ZERO),
        r_hip: Track::constant(Vec3::ZERO),
        l_knee: Track::constant(KNEE_REST_DEG),
        r_knee: Track::constant(KNEE_REST_DEG),
    };
    let skin_color = parse_hex_to_linear(PLAYER_SKIN_HEX).expect("PLAYER_SKIN_HEX is a valid hex color");
    let material = Material { color: Track::constant(skin_color), metallic: 0.05, roughness: 0.55, emissive: Vec3::ZERO };
    Object {
        id: "player_body".to_string(),
        position: Track::constant(Vec3::ZERO),
        rotation: Track::constant(Vec3::ZERO),
        scale: Track::constant(Vec3::splat(HIDDEN_SCALE)),
        material: None,
        kind: ObjectKind::Humanoid(Box::new(HumanoidDef { height: PLAYER_HEIGHT, build: PLAYER_BUILD, material, pose })),
    }
}

/// A brief emissive-color pulse on the object last interacted with, decaying back to whatever
/// it originally was (not necessarily black — an object could have been authored with its own
/// glow) over `FLASH_DURATION`.
struct Flash {
    object_index: usize,
    original_emissive: Vec3,
    timer: f32,
}

/// The single `Vec3` this object's surface color glows by, if it has one to flash — `None` for
/// a `group`, which has no material of its own (only its children do).
fn object_emissive_mut(o: &mut Object) -> Option<&mut Vec3> {
    match &mut o.kind {
        ObjectKind::Prim(_) => o.material.as_mut().map(|m| &mut m.emissive),
        ObjectKind::Humanoid(h) => Some(&mut h.material.emissive),
        ObjectKind::Group(_) => None,
    }
}

struct GpuState {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    live: LiveRenderer,
}

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<GpuState>,
    scene: Scene,
    scene_path: PathBuf,
    colliders: Vec<Collider2D>,
    interactables: Vec<Interactable>,
    camera: FpsCamera,
    keys: HashSet<KeyCode>,
    grabbed: bool,
    sprint_held: bool,
    jump_queued: bool,
    interact_queued: bool,
    /// Seconds elapsed since the current crowbar swing started, or `None` when idle/holding.
    swing_timer: Option<f32>,
    /// Whether this swing's melee raycast has already fired (once per swing, at the start of
    /// the strike phase).
    swing_hit_done: bool,
    target_index: Option<usize>,
    flash: Option<Flash>,
    foot_y: f32,
    vertical_velocity: f32,
    eye_height: f32,
    fov_deg: f32,
    view_mode: ViewMode,
    /// Index into `scene.objects` of the player's own body (see `build_player_object`), so
    /// `update_player_body` can mutate its transform/pose in place each frame.
    player_object_index: usize,
    /// Radians accumulated while walking, driving the procedural walk-cycle pose.
    walk_phase: f32,
    /// The third-person hand-held crowbar's current world transform (see
    /// `update_player_body`), recomputed each frame from the right forearm bone.
    hand_prop_transform: Mat4,
    start: Instant,
    last_frame: Instant,
}

impl App {
    fn new(mut scene: Scene, scene_path: PathBuf) -> Self {
        let player_object_index = scene.objects.len();
        scene.objects.push(build_player_object());

        let colliders = collect_box_colliders(&scene);
        // The player's own body is in `scene.objects` so the renderer can draw it, but it's not
        // something the crosshair or crowbar should ever be able to aim at (e.g. looking down at
        // your own feet), so it's filtered back out of the raycast target list right after.
        let mut interactables = collect_interactables(&scene);
        interactables.retain(|it| it.object_index != player_object_index);

        let spawn = scene.camera.position.sample(0.0);
        let target = scene.camera.target.sample(0.0);
        let yaw = (target.x - spawn.x).atan2(-(target.z - spawn.z)).to_degrees();
        let mut camera = FpsCamera::new(Vec3::new(spawn.x, STAND_EYE_HEIGHT, spawn.z), yaw);
        camera.fov_deg = BASE_FOV_DEG;
        App {
            window: None,
            gpu: None,
            scene,
            scene_path,
            colliders,
            interactables,
            camera,
            keys: HashSet::new(),
            grabbed: false,
            sprint_held: false,
            jump_queued: false,
            interact_queued: false,
            swing_timer: None,
            swing_hit_done: false,
            target_index: None,
            flash: None,
            foot_y: 0.0,
            vertical_velocity: 0.0,
            eye_height: STAND_EYE_HEIGHT,
            fov_deg: BASE_FOV_DEG,
            view_mode: ViewMode::FirstPerson,
            player_object_index,
            walk_phase: 0.0,
            hand_prop_transform: Mat4::from_scale(Vec3::splat(HIDDEN_SCALE)),
            start: Instant::now(),
            last_frame: Instant::now(),
        }
    }

    /// Applies (or re-applies) the current flash state's emissive value to the scene, then
    /// steps its timer down; clears it and restores the original emissive once it's expired.
    fn advance_flash(&mut self, dt: f32) {
        let Some(flash) = &mut self.flash else { return };
        flash.timer -= dt;
        if flash.timer <= 0.0 {
            let (object_index, original) = (flash.object_index, flash.original_emissive);
            if let Some(e) = object_emissive_mut(&mut self.scene.objects[object_index]) {
                *e = original;
            }
            self.flash = None;
            return;
        }
        let frac = flash.timer / FLASH_DURATION;
        let (object_index, original) = (flash.object_index, flash.original_emissive);
        if let Some(e) = object_emissive_mut(&mut self.scene.objects[object_index]) {
            *e = original + FLASH_BOOST * frac;
        }
    }

    /// Boosts `object_index`'s emissive by `boost` and starts it decaying back over
    /// `FLASH_DURATION` — the shared visual feedback behind both E/click interaction and a
    /// crowbar hit, just with a different color.
    ///
    /// The true, un-flashed emissive to flash from and decay back to: if a flash is already
    /// running on this same object, reuse its recorded original rather than the object's
    /// current (still-boosted) value, so rapid re-triggers don't ratchet the glow up. A flash
    /// running on a *different* object is restored first so it doesn't get stuck.
    fn flash_object(&mut self, object_index: usize, boost: Vec3) {
        let original = match self.flash.take() {
            Some(prev) if prev.object_index == object_index => prev.original_emissive,
            Some(prev) => {
                if let Some(e) = object_emissive_mut(&mut self.scene.objects[prev.object_index]) {
                    *e = prev.original_emissive;
                }
                object_emissive_mut(&mut self.scene.objects[object_index]).map_or(Vec3::ZERO, |e| *e)
            }
            None => object_emissive_mut(&mut self.scene.objects[object_index]).map_or(Vec3::ZERO, |e| *e),
        };

        if let Some(e) = object_emissive_mut(&mut self.scene.objects[object_index]) {
            *e = original + boost;
            self.flash = Some(Flash { object_index, original_emissive: original, timer: FLASH_DURATION });
        }
    }

    /// Runs when the player presses E or clicks while aiming at something in reach: logs it and
    /// starts a brief highlight flash. Placeholder for real object interaction later.
    fn interact_with(&mut self, object_index: usize) {
        let id = self.scene.objects[object_index].id.clone();
        println!("Interacted with '{id}'");
        self.flash_object(object_index, FLASH_BOOST);
    }

    /// Runs once per crowbar swing, at the start of the strike phase, if the melee raycast found
    /// something within `MELEE_REACH`: logs it and starts the same highlight-flash feedback as
    /// `interact_with`, just in a hotter color so it reads as an impact.
    fn hit_with(&mut self, object_index: usize) {
        let id = self.scene.objects[object_index].id.clone();
        println!("Hit '{id}' with the crowbar!");
        self.flash_object(object_index, HIT_FLASH_BOOST);
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

    /// The crowbar viewmodel's current world transform: an idle held pose, or mid-swing pose
    /// interpolated by elapsed time through windup/strike/recover. Pitch is rotation about the
    /// camera's local right axis (tipping the bar up/down); roll (about the bar's own axis, so
    /// the shaft itself doesn't visibly change) stays fixed to keep the hooked end angled across
    /// the view.
    fn weapon_transform(&self) -> Mat4 {
        // The crowbar viewmodel is camera-attached, not bound to the body rig's hand bone, so in
        // third person it would just float in front of the (now distant) camera. Shrinking it
        // away reuses the same "scale to near-nothing" hide trick as the player body's own
        // first-person visibility toggle rather than adding a second code path.
        if self.view_mode == ViewMode::ThirdPerson {
            return Mat4::from_scale(Vec3::splat(HIDDEN_SCALE));
        }
        let pitch_deg = self.swing_blend(IDLE_PITCH_DEG, WINDUP_PITCH_DEG, STRIKE_PITCH_DEG);
        let lunge = self.swing_blend(0.0, 0.0, STRIKE_LUNGE);
        let local_rotation = Mat4::from_rotation_z(IDLE_ROLL_DEG.to_radians()) * Mat4::from_rotation_x(pitch_deg.to_radians());
        let local_offset = Vec3::new(VM_RIGHT, -VM_DOWN, VM_FORWARD + lunge);
        viewmodel_transform(&self.camera, local_offset, local_rotation)
    }

    fn set_grab(&mut self, grabbed: bool) {
        let Some(window) = &self.window else { return };
        if grabbed {
            let ok = window.set_cursor_grab(CursorGrabMode::Locked).is_ok()
                || window.set_cursor_grab(CursorGrabMode::Confined).is_ok();
            if ok {
                window.set_cursor_visible(false);
                self.grabbed = true;
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
    /// are additionally blended toward the swing pose (via `swing_blend`) whenever a crowbar
    /// swing is in progress, so a melee attack visibly moves the arm instead of just the weapon.
    /// Also computes `self.hand_prop_transform` — the third-person crowbar rigidly welded to
    /// that same right forearm bone — and flips both the body's and the hand prop's scale
    /// between `HIDDEN_SCALE` and life-size depending on `self.view_mode`, since there's no
    /// per-object render-visibility flag to hide the player's own body in first person instead.
    fn update_player_body(&mut self, planar_pos: Vec2, yaw_deg: f32, speed: f32, dt: f32) {
        if speed > 0.0 {
            self.walk_phase += dt * speed * (WALK_CYCLES_PER_SEC_AT_WALK_SPEED / WALK_SPEED) * std::f32::consts::TAU;
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

        // The right arm additionally blends toward the crowbar's windup/strike pose during a
        // swing — `r_sh_x` (this frame's walk-cycle value) is the blend's idle endpoint, so a
        // swing mid-stride recovers back into whatever the walk cycle is doing by then rather
        // than snapping to a fixed rest angle.
        let r_sh_x_final = self.swing_blend(r_sh_x, ARM_WINDUP_SHOULDER_X, ARM_STRIKE_SHOULDER_X);
        let r_elbow_final = self.swing_blend(ARM_IDLE_ELBOW_DEG, ARM_WINDUP_ELBOW_DEG, ARM_STRIKE_ELBOW_DEG);

        let third_person = self.view_mode == ViewMode::ThirdPerson;
        let scale = if third_person { Vec3::ONE } else { Vec3::splat(HIDDEN_SCALE) };
        let body_pos = Vec3::new(planar_pos.x, self.foot_y, planar_pos.y);
        let body = &mut self.scene.objects[self.player_object_index];
        body.position = Track::constant(body_pos);
        body.rotation = Track::constant(Vec3::new(0.0, yaw_deg, 0.0));
        body.scale = Track::constant(scale);
        let l_shoulder = Vec3::new(l_sh_x, 0.0, -6.0);
        let r_shoulder = Vec3::new(r_sh_x_final, 0.0, 6.0);
        if let ObjectKind::Humanoid(h) = &mut body.kind {
            h.pose.spine = Track::constant(Vec3::new(spine_x, 0.0, 0.0));
            h.pose.l_hip = Track::constant(Vec3::new(l_hip_x, 0.0, 0.0));
            h.pose.r_hip = Track::constant(Vec3::new(r_hip_x, 0.0, 0.0));
            h.pose.l_knee = Track::constant(l_knee);
            h.pose.r_knee = Track::constant(r_knee);
            h.pose.l_shoulder = Track::constant(l_shoulder);
            h.pose.r_shoulder = Track::constant(r_shoulder);
            h.pose.r_elbow = Track::constant(r_elbow_final);
        }

        // Weld the third-person crowbar to the right forearm bone: run the same forward-kinematic
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
                l_elbow: KNEE_REST_DEG,
                r_elbow: r_elbow_final,
                l_hip: Vec3::new(l_hip_x, 0.0, 0.0),
                r_hip: Vec3::new(r_hip_x, 0.0, 0.0),
                l_knee,
                r_knee,
            };
            let forearm = &pose_to_parts(&rig, &pose)[RIGHT_FOREARM_PART];
            let body_world = Mat4::from_scale_rotation_translation(scale, Quat::from_rotation_y(yaw_deg.to_radians()), body_pos);
            let hand_bone_world = body_world
                * Mat4::from_rotation_translation(forearm.rotation, forearm.center)
                * Mat4::from_translation(Vec3::new(0.0, forearm.length * 0.5, 0.0));
            let grip_pose = Mat4::from_translation(Vec3::new(0.0, HAND_GRIP_FORWARD, 0.0))
                * Mat4::from_rotation_x((-90.0_f32).to_radians())
                * Mat4::from_rotation_z(HAND_GRIP_ROLL_DEG.to_radians());
            hand_bone_world * grip_pose
        } else {
            Mat4::from_scale(Vec3::splat(HIDDEN_SCALE))
        };
    }

    fn update(&mut self, dt: f32) {
        if !self.grabbed {
            return;
        }
        let crouching = self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight);

        let fwd = self.camera.forward_flat();
        let right = self.camera.right_flat();
        let mut dir = Vec2::ZERO;
        let forward_held = self.keys.contains(&KeyCode::KeyW) || self.keys.contains(&KeyCode::ArrowUp);
        let back_held = self.keys.contains(&KeyCode::KeyS) || self.keys.contains(&KeyCode::ArrowDown);
        let right_held = self.keys.contains(&KeyCode::KeyD) || self.keys.contains(&KeyCode::ArrowRight);
        let left_held = self.keys.contains(&KeyCode::KeyA) || self.keys.contains(&KeyCode::ArrowLeft);
        if forward_held {
            dir += Vec2::new(fwd.x, fwd.z);
        }
        if back_held {
            dir -= Vec2::new(fwd.x, fwd.z);
        }
        if right_held {
            dir += Vec2::new(right.x, right.z);
        }
        if left_held {
            dir -= Vec2::new(right.x, right.z);
        }
        // Sprinting needs Shift held, a forward component (no sprinting backward, matching most
        // shooters), and not crouching — crouch always wins if both are held.
        let sprinting = self.sprint_held && forward_held && !back_held && !crouching;

        // Tracked outside the block below so the walk-cycle animation can see how fast the
        // player is actually moving this frame (0 when standing still).
        let mut current_speed = 0.0;
        if dir.length_squared() > 1e-8 {
            dir = dir.normalize();
            let speed = if crouching {
                WALK_SPEED * CROUCH_SPEED_MULT
            } else if sprinting {
                SPRINT_SPEED
            } else {
                WALK_SPEED
            };
            current_speed = speed;
            let mut pos2 = Vec2::new(self.camera.position.x, self.camera.position.z);

            // Resolve one movement axis at a time so sliding along a wall works instead of the
            // player sticking when their motion isn't purely into it.
            pos2.x += dir.x * speed * dt;
            pos2 = resolve_collision(pos2, PLAYER_RADIUS, &self.colliders);
            pos2.y += dir.y * speed * dt;
            pos2 = resolve_collision(pos2, PLAYER_RADIUS, &self.colliders);

            self.camera.position.x = pos2.x;
            self.camera.position.z = pos2.y;
        }

        // Vertical: jump + gravity. `foot_y` is the player's height above the floor (y=0);
        // grounded means last frame's physics settled it back to exactly 0 with no velocity.
        let grounded = self.foot_y <= 0.0 && self.vertical_velocity <= 0.0;
        if self.jump_queued && grounded {
            self.vertical_velocity = JUMP_SPEED;
        }
        self.jump_queued = false;
        self.vertical_velocity -= GRAVITY * dt;
        self.foot_y += self.vertical_velocity * dt;
        if self.foot_y <= 0.0 {
            self.foot_y = 0.0;
            self.vertical_velocity = 0.0;
        }

        // Crouch: blend the eye height toward its target instead of snapping, so the camera
        // doesn't jump-cut when Ctrl is pressed/released.
        let target_eye_height = if crouching { CROUCH_EYE_HEIGHT } else { STAND_EYE_HEIGHT };
        let blend = (dt / CROUCH_TRANSITION_TIME).min(1.0);
        self.eye_height += (target_eye_height - self.eye_height) * blend;

        // Update the player's own body (position/facing/pose) from the pre-third-person-pullback
        // planar position, then place the camera: directly at the eye in first person, or pulled
        // back behind/above it in third person. This order matters — the body must be placed
        // before `self.camera.position.x/z` are potentially overwritten by the third-person
        // pullback below.
        let planar_pos = Vec2::new(self.camera.position.x, self.camera.position.z);
        let body_yaw_deg = 180.0 - self.camera.yaw.to_degrees();
        self.update_player_body(planar_pos, body_yaw_deg, current_speed, dt);

        let anchor = Vec3::new(planar_pos.x, self.foot_y + self.eye_height, planar_pos.y);
        self.camera.position = match self.view_mode {
            ViewMode::FirstPerson => anchor,
            ViewMode::ThirdPerson => {
                let desired = anchor - self.camera.forward() * THIRD_PERSON_DISTANCE + Vec3::Y * THIRD_PERSON_HEIGHT_OFFSET;
                let clamped = resolve_collision(Vec2::new(desired.x, desired.z), THIRD_PERSON_CAM_RADIUS, &self.colliders);
                Vec3::new(clamped.x, desired.y, clamped.y)
            }
        };

        // Sprint FOV kick, blended the same way as the crouch height.
        let target_fov = if sprinting { BASE_FOV_DEG + SPRINT_FOV_BOOST_DEG } else { BASE_FOV_DEG };
        let fov_blend = (dt / FOV_TRANSITION_TIME).min(1.0);
        self.fov_deg += (target_fov - self.fov_deg) * fov_blend;
        self.camera.fov_deg = self.fov_deg;

        // What's the crosshair aimed at, and did the player just try to interact with it?
        // `raycast_nearest` returns an index into `self.interactables`, not `scene.objects`
        // directly (they can diverge — e.g. an empty `group` has no bounds and is skipped when
        // the list is built), so map it through `Interactable::object_index`.
        self.target_index = raycast_nearest(self.camera.position, self.camera.forward(), INTERACT_REACH, &self.interactables)
            .map(|i| self.interactables[i].object_index);
        if self.interact_queued {
            if let Some(idx) = self.target_index {
                self.interact_with(idx);
            }
        }
        self.interact_queued = false;

        // Crowbar swing: advance the timer, and fire one melee raycast/hit check at the start
        // of the strike phase — gated by `swing_hit_done` so a single click can't hit twice
        // while its animation plays out.
        if let Some(elapsed) = self.swing_timer {
            let elapsed = elapsed + dt;
            if elapsed >= SWING_WINDUP && !self.swing_hit_done {
                self.swing_hit_done = true;
                let hit = raycast_nearest(self.camera.position, self.camera.forward(), MELEE_REACH, &self.interactables)
                    .map(|i| self.interactables[i].object_index);
                if let Some(idx) = hit {
                    self.hit_with(idx);
                }
            }
            self.swing_timer = if elapsed >= SWING_TOTAL { None } else { Some(elapsed) };
        }

        self.advance_flash(dt);
    }

    fn draw(&mut self) {
        let weapon_transform = self.weapon_transform();
        let Some(gpu) = self.gpu.as_mut() else { return };
        let (surface_tex, reconfigure) = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => (t, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => (t, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gpu.surface.configure(&gpu.device, &gpu.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => return,
        };
        let view = surface_tex.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let t = if self.scene.duration > 0.0 { self.start.elapsed().as_secs_f32() % self.scene.duration } else { 0.0 };
        gpu.live.render(
            &gpu.device,
            &gpu.queue,
            &self.scene,
            t,
            &self.camera,
            &view,
            self.target_index.is_some(),
            weapon_transform,
            self.hand_prop_transform,
        );
        gpu.queue.present(surface_tex);
        if reconfigure {
            gpu.surface.configure(&gpu.device, &gpu.config);
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

        let live = LiveRenderer::new(&device, format, &self.scene, config.width, config.height);

        self.gpu = Some(GpuState { surface, device, queue, config, live });
        self.window = Some(window);
        self.last_frame = Instant::now();
        self.set_grab(true);
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
                    gpu.live.resize(&gpu.device, gpu.config.width, gpu.config.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if code == KeyCode::Escape && event.state == ElementState::Pressed {
                        self.set_grab(false);
                    }
                    if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight) {
                        self.sprint_held = event.state == ElementState::Pressed;
                    }
                    if code == KeyCode::Space && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.jump_queued = true;
                    }
                    if code == KeyCode::KeyE && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.interact_queued = true;
                    }
                    if code == KeyCode::KeyF && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.toggle_fullscreen();
                    }
                    if code == KeyCode::KeyQ && event.state == ElementState::Pressed && !self.keys.contains(&code) {
                        self.toggle_view_mode();
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
                if !self.grabbed {
                    self.set_grab(true);
                } else if self.swing_timer.is_none() {
                    self.swing_timer = Some(0.0);
                    self.swing_hit_done = false;
                }
            }
            WindowEvent::Focused(false) => self.set_grab(false),
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;
                self.update(dt);
                self.draw();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device_id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            if self.grabbed {
                self.camera.look(dx as f32 * MOUSE_SENSITIVITY, -dy as f32 * MOUSE_SENSITIVITY);
            }
        }
    }
}

fn main() {
    env_logger::init();
    let scene_path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("examples/room.json"));
    let scene = red_engine2::load_scene(&scene_path).unwrap_or_else(|errs| {
        eprintln!("failed to load scene {}:", scene_path.display());
        for e in &errs {
            eprintln!("  {e}");
        }
        std::process::exit(1);
    });

    println!("Red Engine 2 — {}", scene_path.display());
    println!("WASD / arrow keys to walk, mouse to look, Shift to sprint forward, Space to jump, Ctrl to crouch.");
    println!("Left-click to swing the crowbar, E to interact with whatever the crosshair is aimed at.");
    println!("Q to toggle first-/third-person view, F to toggle fullscreen / maximized.");
    println!("Click the window to capture the mouse, Escape to release it.");

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(scene, scene_path);
    event_loop.run_app(&mut app).expect("event loop error");
}
