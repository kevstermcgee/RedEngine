//! Top-down "switch and objective": step on both switches to open the gate, then reach the exit.
//!
//! Everything that decides the game (movement, collision, the switches, the gate, winning) is the scene's `rules`,
//! run by the engine's authoritative simulation through [`LocalSession`]. This crate is only the *client*: it maps keys
//! and clicks to movement, chooses a camera, moves the player marker and composes the HUD.

use red_engine2::app::{input_toward, place_object, ClientGame, Frame, InputState, KeyCode, LocalSession, MouseButton, ViewCamera};
use red_engine2::glam::{Vec2, Vec3};
use red_engine2::schema::Scene;
use red_engine2::sim::player::PlayerInput;
use red_engine2::ui::Layout;
use std::path::{Path, PathBuf};

/// The scene this game plays.
pub fn default_scene() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("topdown_switch.json")
}

/// The player marker's object id in the scene.
const PLAYER: &str = "player";
/// Where the controls hint sits on the HUD.
const HINT: &str = "WASD / CLICK: MOVE   Q E: TURN VIEW   WHEEL: ZOOM   R: RESTART   ESC: QUIT";

/// Client state: the session plus camera and click-to-move policy.
pub struct TopDown {
    path: PathBuf,
    session: LocalSession,
    /// Which way is up on screen, degrees (turned in 90 degree steps with Q / E).
    pub view_yaw: f32,
    /// Camera height above the player, m (mouse wheel).
    pub height: f32,
    /// A clicked ground point the player is walking to.
    pub target: Option<Vec2>,
    /// Facing of the marker, degrees about Y (follows the direction of travel).
    facing: f32,
    viewport: (u32, u32),
}

impl TopDown {
    /// Loads (strictly validates) the scene and starts a match.
    pub fn load(path: &Path) -> Result<Self, Vec<String>> {
        let session = LocalSession::load(path)?;
        let mut game = TopDown { path: path.to_path_buf(), session, view_yaw: 0.0, height: 13.0, target: None, facing: 180.0, viewport: (1280, 720) };
        game.sync_marker();
        Ok(game)
    }

    /// The running session (tests read the rules state from it).
    pub fn session(&self) -> &LocalSession {
        &self.session
    }

    /// The top-down camera following the player.
    pub fn view(&self) -> ViewCamera {
        let feet = self.session.player_feet();
        ViewCamera::top_down(Vec3::new(feet.x, 0.0, feet.z - 1.0), self.height, 28.0, self.view_yaw)
    }

    /// One frame: input to the simulation, then presentation. Returns `false` to quit.
    pub fn frame(&mut self, input: &InputState, frame: Frame) -> bool {
        self.viewport = (frame.width, frame.height);
        if input.pressed(KeyCode::Escape) {
            return false;
        }
        if input.pressed(KeyCode::KeyR) {
            if let Ok(fresh) = TopDown::load(&self.path) {
                *self = TopDown { view_yaw: self.view_yaw, height: self.height, ..fresh };
            }
            return true;
        }
        if input.pressed(KeyCode::KeyQ) {
            self.view_yaw -= 90.0;
        }
        if input.pressed(KeyCode::KeyE) {
            self.view_yaw += 90.0;
        }
        self.height = (self.height - input.wheel() * 1.5).clamp(7.0, 24.0);
        if let Some((x, y)) = input.clicked(MouseButton::Left) {
            self.target = self.view().pick_ground(x, y, frame.width, frame.height, 0.0).map(|p| Vec2::new(p.x, p.z));
        }

        // Keys move relative to the view: `forward` walks up the screen, whatever way the camera is turned.
        let (forward, strafe) = input.wasd();
        if forward != 0 || strafe != 0 {
            self.target = None;
        }
        let yaw = self.view_yaw.to_radians();
        let target = self.target;
        self.session.advance(frame.dt, |state| match target {
            Some(t) if forward == 0 && strafe == 0 => input_toward(state, t, 0.15),
            _ => PlayerInput { forward, strafe, yaw, ..Default::default() },
        });
        if self.target.is_some_and(|t| t.distance(self.session.player().pos) <= 0.15) {
            self.target = None;
        }
        self.sync_marker();
        true
    }

    /// Moves the player marker to where the simulation says the player is, facing the way they are walking.
    fn sync_marker(&mut self) {
        let p = self.session.player();
        if p.velocity.length() > 0.2 {
            // Marker faces local +Z; a travel direction (x, z) is a yaw of atan2(x, z) about Y.
            self.facing = p.velocity.x.atan2(p.velocity.y).to_degrees();
        }
        let feet = self.session.player_feet();
        let facing = self.facing;
        place_object(self.session.scene_mut(), PLAYER, feet, Some(facing));
    }

    /// The standard rules HUD plus this game's controls hint.
    pub fn hud_layout(&self, w: u32, h: u32) -> Layout {
        let mut l = self.session.hud().layout(w, h);
        let s = (h as i32 / 360).max(1);
        l.label_fit("controls", None, w as i32 / 2, h as i32 - 12 * s, HINT, s, w as i32 - 16, [220, 228, 240, 255]);
        l
    }
}

impl ClientGame for TopDown {
    fn scene(&self) -> &Scene {
        self.session.scene()
    }

    fn update(&mut self, input: &InputState, frame: Frame) -> bool {
        self.frame(input, frame)
    }

    fn camera(&self, _width: u32, _height: u32) -> ViewCamera {
        self.view()
    }

    fn hidden(&self) -> Vec<String> {
        self.session.hidden().map(str::to_string).collect()
    }

    fn hud_key(&self) -> String {
        self.session.hud().key()
    }

    fn hud(&self, width: u32, height: u32) -> Option<Layout> {
        Some(self.hud_layout(width, height))
    }

    fn title(&self) -> String {
        match self.session.outcome() {
            Some(o) => format!("Top-down switch: {o} (R to restart)"),
            None => "Top-down switch: press both switches, then reach the exit".to_string(),
        }
    }
}
