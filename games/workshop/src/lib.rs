//! Workshop's client: a top-down window on the match the server would run (`app::LocalSession`: the authoritative `MatchSim` with Workshop
//! as its game, as `workshop_server` and `sim` run it). It turns keys and clicks into the player's movement (walking onto the lever is the
//! decision; the scene's rules flip the route), draws each machine's status lamp from the game's own inspection, shows the scene's objective
//! and the machines on the HUD, and saves progress: the game's versioned save plus the scene's own variables, written atomically.

use red_engine2::app::{input_toward, place_object, ClientGame, Frame, InputState, KeyCode, LocalSession, MouseButton, ViewCamera};
use red_engine2::glam::{Vec2, Vec3};
use red_engine2::schema::Scene;
use red_engine2::sim::player::PlayerInput;
use red_engine2::ui::Layout;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use workshop_server::game::WorkshopGame;

/// The scene the game plays.
pub fn default_scene() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("content/workshop.json")
}

/// Where the progress of the game that owns `scene` is saved (its save folder, keyed by its `game.json` id or its location).
pub fn save_path(scene: &Path) -> Option<PathBuf> {
    red_engine2::settings::game_state_path(&red_engine2::settings::key_for(scene), "workshop.json")
}

const PLAYER: &str = "player";
const HINT: &str = "WASD / CLICK: MOVE   WALK ONTO THE LEVER: ROUTE THE PRESS   F5: SAVE   ESC: SAVE AND QUIT";
/// Seconds between automatic saves.
const AUTOSAVE_SECS: f32 = 10.0;

/// The client.
pub struct Workshop {
    session: LocalSession,
    save: Option<PathBuf>,
    save_note: Option<String>,
    since_save: f32,
    /// A clicked ground point the player walks to.
    pub target: Option<Vec2>,
    facing: f32,
}

impl Workshop {
    /// Loads the scene (Workshop's block and game defined first) and, when `save` names a file that exists, the progress in it: a save this
    /// version cannot read is reported ([`Workshop::save_note`]) and the game starts fresh, never touching the file.
    pub fn load(path: &Path, save: Option<PathBuf>) -> Result<Workshop, Vec<String>> {
        workshop_server::register();
        let session = LocalSession::load(path)?;
        let mut w = Workshop { session, save, save_note: None, since_save: 0.0, target: None, facing: 180.0 };
        if let Some(text) = w.save.as_deref().and_then(|p| std::fs::read_to_string(p).ok()) {
            w.save_note = Some(match w.restore(&text) {
                Ok(note) => note,
                Err(e) => format!("progress not restored (the file is kept as it is): {e}"),
            });
        }
        w.sync_marker();
        Ok(w)
    }

    fn restore(&mut self, text: &str) -> Result<String, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not a save: {e}"))?;
        let report = self.game_mut().load(&v).map_err(|e| e.to_string())?;
        if let Some(vars) = v.get("scene_vars").and_then(Value::as_object) {
            let vars = vars.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?))).collect();
            self.session.restore_vars(&vars);
        }
        Ok(match report.migrated.as_slice() {
            [] => "progress restored".to_string(),
            m => format!("progress restored, migrated: {}", m.iter().map(|(s, a, b)| format!("{s} v{a} -> v{b}")).collect::<Vec<_>>().join(", ")),
        })
    }

    /// What happened to the saved progress at load, if there was any.
    pub fn save_note(&self) -> Option<&str> {
        self.save_note.as_deref()
    }

    /// The running session.
    pub fn session(&self) -> &LocalSession {
        &self.session
    }

    /// Workshop, as the match runs it.
    pub fn game(&self) -> &WorkshopGame {
        self.session.sim().game::<WorkshopGame>().expect("a Workshop scene")
    }

    fn game_mut(&mut self) -> &mut WorkshopGame {
        self.session.sim_mut().game_mut::<WorkshopGame>().expect("a Workshop scene")
    }

    /// Writes the progress now (the game's save and the scene's variables), atomically.
    pub fn save_now(&mut self) -> Result<(), String> {
        let Some(path) = self.save.clone() else { return Ok(()) };
        let mut v = self.game().save();
        v["scene_vars"] = json!(self.session.rules().vars().into_iter().collect::<std::collections::BTreeMap<_, _>>());
        self.since_save = 0.0;
        red_engine2::sim::systems::write_atomic(&path, &v.to_string())
    }

    /// The camera: top down, following the player.
    pub fn view(&self) -> ViewCamera {
        let feet = self.session.player_feet();
        ViewCamera::top_down(Vec3::new(feet.x, 0.0, feet.z - 1.5), 15.0, 30.0, 0.0)
    }

    /// One frame. Returns `false` to quit (after saving).
    pub fn frame(&mut self, input: &InputState, frame: Frame) -> bool {
        if input.pressed(KeyCode::Escape) {
            let _ = self.save_now();
            return false;
        }
        if input.pressed(KeyCode::F5) {
            let _ = self.save_now();
        }
        if let Some((x, y)) = input.clicked(MouseButton::Left) {
            self.target = self.view().pick_ground(x, y, frame.width, frame.height, 0.0).map(|p| Vec2::new(p.x, p.z));
        }
        let (forward, strafe) = input.wasd();
        if forward != 0 || strafe != 0 {
            self.target = None;
        }
        let target = self.target;
        self.session.advance(frame.dt, |state| match target {
            Some(t) if forward == 0 && strafe == 0 => input_toward(state, t, 0.15),
            _ => PlayerInput { forward, strafe, ..Default::default() },
        });
        if self.target.is_some_and(|t| t.distance(self.session.player().pos) <= 0.15) {
            self.target = None;
        }
        self.since_save += frame.dt;
        if self.since_save >= AUTOSAVE_SECS {
            let _ = self.save_now();
        }
        self.sync_marker();
        true
    }

    fn sync_marker(&mut self) {
        let p = self.session.player();
        if p.velocity.length() > 0.2 {
            self.facing = p.velocity.x.atan2(p.velocity.y).to_degrees();
        }
        let (feet, facing) = (self.session.player_feet(), self.facing);
        place_object(self.session.scene_mut(), PLAYER, feet, Some(facing));
    }

    /// Each machine as the game explains it: `(scene object id, status, what would unblock it)`.
    pub fn machines(&self) -> Vec<(String, String, Option<String>)> {
        let sim = self.session.sim();
        self.game()
            .catalog()
            .instances
            .iter()
            .filter_map(|(id, _)| {
                let i = sim.game_inspect(id.as_str())?;
                let status = i.get("status")?.as_str()?.to_string();
                // The player reads what to do (the fix), with names rather than ids.
                let why = i
                    .get("blocked_by")
                    .and_then(|b| b.get("fix").or_else(|| b.get("message")))
                    .and_then(Value::as_str)
                    .map(|t| t.replace('`', "").replace("ws:", ""));
                Some((id.name().to_string(), status, why))
            })
            .collect()
    }

    /// The rules' hidden objects, and each machine's lamp: green while it works or idles, red while it is blocked.
    pub fn hidden_objects(&self) -> Vec<String> {
        let mut hidden: Vec<String> = self.session.hidden().map(str::to_string).collect();
        for (m, status, _) in self.machines() {
            hidden.push(if status == "blocked" { format!("{m}_ok") } else { format!("{m}_blocked") });
        }
        hidden
    }

    /// The scene's HUD (title, counters, objective) plus a line per machine and the controls.
    pub fn hud_layout(&self, w: u32, h: u32) -> Layout {
        let mut l = self.session.hud().layout_scene(self.session.scene(), w, h);
        let s = (h as i32 / 360).max(1);
        let mut y = h as i32 - 30 * s;
        for (m, status, why) in self.machines().into_iter().rev() {
            let text = format!("{}: {}{}", m.replace('_', " ").to_uppercase(), status.to_uppercase(), why.map(|w| format!(" - {w}")).unwrap_or_default());
            let color = if status == "blocked" { [255, 140, 120, 255] } else { [200, 240, 200, 255] };
            l.label_fit(&format!("machine-{m}"), None, w as i32 / 2, y, &text, s, w as i32 - 16, color);
            y -= 9 * s;
        }
        l.label_fit("controls", None, w as i32 / 2, h as i32 - 12 * s, HINT, s, w as i32 - 16, [220, 228, 240, 255]);
        l
    }
}

impl ClientGame for Workshop {
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
        self.hidden_objects()
    }

    fn hud_key(&self) -> String {
        format!("{}|{:?}", self.session.hud().key(), self.machines())
    }

    fn hud(&self, width: u32, height: u32) -> Option<Layout> {
        Some(self.hud_layout(width, height))
    }

    fn title(&self) -> String {
        match self.session.outcome() {
            Some(o) => format!("Workshop: {o}"),
            None => "Workshop: ship 12 gears".to_string(),
        }
    }
}
