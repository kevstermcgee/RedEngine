//! The seam a game plugs into the client (ADR 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library): a [`GameMode`] is told what
//! happens (the scene started, a frame passed, a key or a click came, a footstep fell, a script names a target) and answers through a [`ModeContext`],
//! the only door to the client's state a mode gets. Every method has a default that does nothing, so a mode implements only what it changes.
//!
//! The client asks a [`ModeFactory`] for its mode when a scene starts; plain `re2` runs the engine's own ([`super::engine_mode`]), a game binary its
//! own (`client::run`).

use super::*;
use crate::feel::FxParams;
use winit::event::WindowEvent;

/// What a mode did with a window event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Not the mode's: the client handles it as usual.
    Pass,
    /// The mode used it.
    Used,
    /// The mode used it and the program should end (its own "leave" button).
    Quit,
}

/// A game's part of the live client. Every method has a default that does nothing.
pub trait GameMode {
    /// The name its part of the state dump is filed under (`museum`).
    fn name(&self) -> &'static str;

    /// The scene has started and the player stands in it: set up, read a save, place the player.
    fn start(&mut self, _cx: &mut ModeContext) {}

    /// One frame, after the player moved.
    fn frame(&mut self, _cx: &mut ModeContext, _dt: f32) {}

    /// Whether the player's own movement is frozen this frame (the mode owns the camera, as in a flight into a painting).
    fn input_frozen(&self) -> bool {
        false
    }

    /// The player pressed interact (E): true when the mode took it.
    fn interact(&mut self) -> bool {
        false
    }

    /// Whether the mode draws the screen's overlay during play (the client's rules HUD then yields to [`GameMode::hud`]).
    fn owns_hud(&self) -> bool {
        false
    }

    /// Paints the mode's overlay for this frame (only when [`GameMode::owns_hud`]; not while paused).
    fn hud(&mut self, _cx: &mut ModeContext) {}

    /// Whether the mode draws its own pause menu (then [`GameMode::repaint_pause`] instead of the client's).
    fn owns_pause(&self) -> bool {
        false
    }

    /// Paints the mode's pause menu.
    fn repaint_pause(&mut self, _cx: &mut ModeContext) {}

    /// The pause menu has closed.
    fn left_pause(&mut self) {}

    /// A window event, before the client's own handling of it.
    fn window_event(&mut self, _cx: &mut ModeContext, _event: &WindowEvent) -> Answer {
        Answer::Pass
    }

    /// A script presses `key:NAME`: true when the mode used it.
    fn script_key(&mut self, _cx: &mut ModeContext, _name: &str) -> bool {
        false
    }

    /// A footstep: true when the mode played it (or chose silence); false lets the client play its own.
    fn footstep(&mut self, _cx: &mut ModeContext) -> bool {
        false
    }

    /// This frame's screen effects, given the client's.
    fn fx(&self, fx: FxParams) -> FxParams {
        fx
    }

    /// Hands this frame to the renderer (its world, its shader numbers), just before drawing.
    fn sync_gpu(&mut self, _cx: &mut ModeContext) {}

    /// Its part of the state dump (`re2 --dump`).
    fn dump(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// The box of a script target the mode names (`museum:3`), for `approach` and `look_at`.
    fn target_bounds(&self, _id: &str) -> Option<(Vec3, Vec3)> {
        None
    }
}

/// Chooses the mode for a scene when it starts (`None`: the client plays it as it is).
pub type ModeFactory = fn(&Scene) -> Option<Box<dyn GameMode>>;

/// The start card as it reads now: its title, its text and its button's label.
pub struct StartCard {
    pub title: String,
    pub text: String,
    pub button: Option<String>,
    /// The button's id (`start`).
    pub button_id: &'static str,
}

/// What a mode may see and change of the client.
pub struct ModeContext<'a> {
    pub(super) app: &'a mut App,
}

impl ModeContext<'_> {
    /// The scene being played.
    pub fn scene(&self) -> &Scene {
        &self.app.scene
    }

    /// The scene's lights (a mode that lights its world replaces them).
    pub fn set_lights(&mut self, lights: Vec<Light>) {
        self.app.scene.lights = lights;
    }

    /// The key a game's settings and saves are filed under (`settings::game_state_path`).
    pub fn settings_key(&self) -> &str {
        &self.app.settings_key
    }

    /// Whether there is a sound device.
    pub fn has_audio(&self) -> bool {
        self.app.audio.is_some()
    }

    /// The window's size in pixels, when there is a window or an offscreen frame.
    pub fn window_size(&self) -> Option<(u32, u32)> {
        self.app.window_size()
    }

    /// Where the player's eyes are.
    pub fn eye(&self) -> Vec3 {
        self.app.eye
    }

    /// Where the player's feet are.
    pub fn foot(&self) -> Vec3 {
        Vec3::new(self.app.physics_pos.x, self.app.foot_y, self.app.physics_pos.y)
    }

    /// The camera's position.
    pub fn camera_position(&self) -> Vec3 {
        self.app.camera.position
    }

    /// Where the camera looks.
    pub fn camera_forward(&self) -> Vec3 {
        self.app.camera.forward()
    }

    /// The camera's yaw and pitch (radians).
    pub fn camera_angles(&self) -> (f32, f32) {
        (self.app.camera.yaw, self.app.camera.pitch)
    }

    /// Puts the player at `at` (feet), facing `yaw`, standing still, the camera at their eyes.
    pub fn teleport(&mut self, at: Vec3, yaw: f32) {
        let a = &mut *self.app;
        a.physics_pos = Vec2::new(at.x, at.z);
        a.prev_physics_pos = a.physics_pos;
        a.foot_y = at.y;
        a.prev_foot_y = at.y;
        a.vertical_velocity = 0.0;
        a.horizontal_velocity = Vec2::ZERO;
        a.camera.yaw = yaw;
        a.camera.pitch = 0.0;
        a.eye = Vec3::new(at.x, at.y + a.eye_height, at.z);
        a.camera.position = a.eye;
    }

    /// Places the camera this frame (a mode that owns it).
    pub fn set_camera(&mut self, position: Vec3, yaw: f32, pitch: f32) {
        self.app.camera.position = position;
        self.app.camera.yaw = yaw;
        self.app.camera.pitch = pitch;
    }

    /// The field of view, in degrees.
    pub fn set_fov(&mut self, degrees: f32) {
        self.app.fov_deg = degrees;
        self.app.camera.fov_deg = degrees;
    }

    /// Replaces the walls the player collides with (a world the mode builds as the player walks).
    pub fn set_colliders(&mut self, colliders: Vec<Collider2D>) {
        self.app.colliders = colliders;
    }

    /// Steers the scene's music journey toward these scores (indices into its `audio.journey`; nothing without a journey).
    pub fn steer_music(&mut self, scores: &[usize]) {
        if let Some(a) = self.app.ambient.as_mut() {
            a.steer_journey(scores);
        }
    }

    /// Plays a sound (mono samples at the engine's rate), when sound effects are on.
    pub fn play(&self, clip: &[f32], gain: f32, pan: f32) {
        if let Some(audio) = &self.app.audio {
            if self.app.sfx_on {
                audio.play_at(clip, gain, pan);
            }
        }
    }

    /// Ducks the music for a moment.
    pub fn duck_music(&self, duck: crate::mixer::Duck, secs: f32) {
        if let Some(audio) = &self.app.audio {
            audio.duck_music(duck, secs);
        }
    }

    /// Raises an event the client's soundscape and logs see (`museum:KeyFound`).
    pub fn raise_event(&mut self, name: String) {
        self.app.fresh_events.push(name);
    }

    /// Whether the pause menu is up.
    pub fn paused(&self) -> bool {
        self.app.paused
    }

    /// Whether a card (start, end) is up.
    pub fn card_shown(&self) -> bool {
        self.app.card.is_some()
    }

    /// Whether the game is being played (not the connect form or the lobby).
    pub fn playing(&self) -> bool {
        self.app.phase == Phase::Playing
    }

    /// Whether the mouse is captured.
    pub fn grabbed(&self) -> bool {
        self.app.grabbed
    }

    /// The pointer's position in window pixels.
    pub fn cursor(&self) -> (f32, f32) {
        self.app.cursor
    }

    /// Records the pointer's position (a mode that handles a pointer move).
    pub fn set_cursor(&mut self, at: (f32, f32)) {
        self.app.cursor = at;
    }

    /// Opens the pause menu. The client paints its own: a mode that owns the pause paints its menu over it straight after (the client cannot call
    /// back into a mode while one of its methods runs).
    pub fn enter_pause(&mut self) {
        self.app.enter_pause();
    }

    /// Closes the pause menu. A mode that closes it calls its own [`GameMode::left_pause`] too (for the same reason).
    pub fn leave_pause(&mut self) {
        self.app.leave_pause();
    }

    /// Whether the music plays.
    pub fn music_on(&self) -> bool {
        self.app.music_on
    }

    /// Whether sound effects play.
    pub fn sfx_on(&self) -> bool {
        self.app.sfx_on
    }

    /// Turns the music on or off (and saves the choice).
    pub fn toggle_music(&mut self) {
        self.app.toggle_music();
    }

    /// Turns sound effects on or off (and saves the choice).
    pub fn toggle_sfx(&mut self) {
        self.app.toggle_sfx();
    }

    /// Full screen, or back.
    pub fn toggle_fullscreen(&mut self) {
        self.app.toggle_fullscreen();
    }

    /// Whether footsteps are heard (a setting).
    pub fn footsteps(&self) -> bool {
        self.app.settings.steps
    }

    /// Turns footsteps on or off and saves the setting.
    pub fn set_footsteps(&mut self, on: bool) {
        self.app.settings.steps = on;
        let _ = crate::settings::save(&self.app.settings_key, &self.app.settings);
    }

    /// Seconds played.
    pub fn play_secs(&self) -> f32 {
        self.app.play_secs
    }

    /// The scene's clock (seconds).
    pub fn scene_time(&self) -> f32 {
        self.app.scene_time()
    }

    /// The start card as it reads now, when it is up.
    pub fn start_card(&self) -> Option<StartCard> {
        self.app.card_content().map(|(c, id)| StartCard { title: c.title.clone(), text: c.text.clone(), button: c.button.clone(), button_id: id })
    }

    /// The card's layout at this window size (its button's place), when one is up.
    pub fn card_layout(&self, w: u32, h: u32) -> Option<crate::ui::Layout> {
        self.app.card_layout_now(w, h)
    }

    /// Shows `rgba` (w x h) over the frame during play.
    pub fn set_hud_overlay(&mut self, w: u32, h: u32, rgba: &[u8]) {
        if let Some(gpu) = self.app.gpu.as_mut() {
            if let Some(live) = gpu.live.as_mut() {
                live.overlay.set(&gpu.device, &gpu.queue, w, h, rgba);
            }
        }
    }

    /// Shows `rgba` over the whole window (a pause menu), or hides it.
    pub fn set_window_overlay(&mut self, w: u32, h: u32, rgba: Option<&[u8]>) {
        self.app.set_window_overlay(w, h, rgba);
    }

    /// The live renderer and its device, to hand the mode's world to (`None` with no window and no offscreen frame).
    pub fn renderer(&mut self) -> Option<(&wgpu::Device, &mut crate::viewer::LiveRenderer)> {
        let gpu = self.app.gpu.as_mut()?;
        let live = gpu.live.as_mut()?;
        Some((&gpu.device, live))
    }
}

impl App {
    /// Runs `f` on the mode with a context to the rest of the client; `None` when there is no mode. The mode is out of the client while `f` runs,
    /// so a context call that comes back to the client (opening the pause menu) does not reach it.
    pub(crate) fn with_mode<R>(&mut self, f: impl FnOnce(&mut dyn GameMode, &mut ModeContext) -> R) -> Option<R> {
        let mut mode = self.mode.take()?;
        let r = f(mode.as_mut(), &mut ModeContext { app: self });
        self.mode = Some(mode);
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(text: &str) -> Scene {
        crate::scene_blocks::parse_scene(text).expect("parses")
    }

    #[test]
    fn plain_re2_gives_a_museum_scene_the_museum_mode_and_any_other_scene_none() {
        let plain = scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": []}"#);
        assert!(engine_mode(&plain).is_none());
        let museum = scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "museum": {"collection": "c.json"}}"#);
        assert_eq!(engine_mode(&museum).map(|m| m.name()), Some("museum"));
    }

    struct Quiet;
    impl GameMode for Quiet {
        fn name(&self) -> &'static str {
            "quiet"
        }
    }

    #[test]
    fn a_mode_that_implements_only_its_name_changes_nothing() {
        let m = Quiet;
        assert!(!m.input_frozen() && !m.owns_hud() && !m.owns_pause());
        assert!(m.dump().is_null() && m.target_bounds("quiet:1").is_none());
        let fx = crate::feel::FxParams { flash: [1.0, 0.5, 0.25, 0.5], ..Default::default() };
        assert_eq!(m.fx(fx).flash, fx.flash);
    }
}
