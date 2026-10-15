//! Tidewatch's client: a third-person window on the match the server would run (`app::LocalSession`: the authoritative `MatchSim` with
//! Tidewatch as its game). It turns keys and a gamepad into the player's movement and intentions, draws the scene's HUD, and presents the
//! story with the engine's optional presenter (`app::dialogue`): subtitles that type out (the first press shows the whole line, the next
//! moves on), choices with predictable focus (up/down, 1-9, a click, the D-pad), barks as subtitles.
//!
//! While a conversation is open the client sends no movement or action (the presenter has the input) and the authority holds the player
//! still too; the world does **not** pause: the watch's clock and the brazier keep burning, so talking during a watch costs light. Barks do
//! not stop play. Progress (the game's versioned save, story included, and the scene's variables) is saved atomically on F5, every 10 s
//! and on Esc.

use red_engine2::app::dialogue::{Controls, Presenter};
use red_engine2::app::{place_object, ClientGame, Frame, InputState, KeyCode, LocalSession, ViewCamera};
use red_engine2::controller::{Controller, Sample};
use red_engine2::glam::Vec3;
use red_engine2::schema::Scene;
use red_engine2::sim::game::Intent;
use red_engine2::sim::player::PlayerInput;
use red_engine2::sim::story::runtime::{HostAsk, View};
use red_engine2::ui::Layout;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tidewatch_server::game::TidewatchGame;

/// The scene the game plays.
pub fn default_scene() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("content/tidewatch.json")
}

/// Where the progress of the game that owns `scene` is saved.
pub fn save_path(scene: &Path) -> Option<PathBuf> {
    red_engine2::settings::game_state_path(&red_engine2::settings::key_for(scene), "tidewatch.json")
}

const PLAYER: &str = "player";
const AUTOSAVE_SECS: f32 = 10.0;
/// Who can be talked to, and the scene variable that says the player is near them.
const SPEAKERS: &[(&str, &str, &str)] = &[("odile", "near_odile", "ODILE"), ("bram", "near_bram", "BRAM")];

/// The client.
pub struct Tidewatch {
    session: LocalSession,
    presenter: Presenter,
    pad: Option<Controller>,
    save: Option<PathBuf>,
    save_note: Option<String>,
    since_save: f32,
    facing: f32,
    /// The last refusal the authority gave an intention (shown briefly).
    pub refused: Option<String>,
    last_layout: Option<Layout>,
}

impl Tidewatch {
    /// Loads the scene (Tidewatch's block and game defined first) and, when `save` names a file that exists, the progress in it: a save this
    /// version cannot read is reported ([`Tidewatch::save_note`]) and the game starts fresh, never touching the file. `pad`: poll a gamepad.
    pub fn load(path: &Path, save: Option<PathBuf>, pad: bool) -> Result<Tidewatch, Vec<String>> {
        tidewatch_server::register();
        // A save resumes where the player stood: the scene's presence variables (near Bram, at the shed) are only true there.
        let saved: Option<Value> = save.as_deref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok());
        let text = std::fs::read_to_string(path).map_err(|e| vec![format!("{}: {e}", path.display())])?;
        let scene = red_engine2::scene_blocks::parse_scene_in(&text, path.parent())?;
        let mut spawns = red_engine2::sim::spawns::parse_spawns(&text).map_err(|e| vec![e])?;
        let at = saved
            .as_ref()
            .and_then(|v| v.get("player"))
            .and_then(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?, p.get(2)?.as_f64()?, p.get(3)?.as_f64()?]));
        if let Some([x, y, z, yaw]) = at {
            spawns = vec![red_engine2::sim::spawns::Spawn {
                id: "saved".into(),
                position: [x as f32, y as f32, z as f32],
                yaw_deg: yaw as f32,
                group: String::new(),
            }];
        }
        let session = LocalSession::new(scene, spawns, None).map_err(|e| vec![e])?;
        let mut t = Tidewatch {
            session,
            presenter: Presenter::new(),
            pad: pad.then(Controller::default),
            save,
            save_note: None,
            since_save: 0.0,
            facing: 180.0,
            refused: None,
            last_layout: None,
        };
        if let Some(text) = t.save.as_deref().and_then(|p| std::fs::read_to_string(p).ok()) {
            t.save_note = Some(match t.restore(&text) {
                Ok(note) => note,
                Err(e) => format!("progress not restored (the file is kept as it is): {e}"),
            });
        }
        t.sync_marker();
        Ok(t)
    }

    fn restore(&mut self, text: &str) -> Result<String, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not a save: {e}"))?;
        let report = self.game_mut().load(&v).map_err(|e| e.to_string())?;
        if let Some(vars) = v.get("scene_vars").and_then(Value::as_object) {
            let vars = vars.iter().filter_map(|(k, x)| Some((k.clone(), x.as_f64()?))).collect();
            self.session.restore_vars(&vars);
        }
        Ok(if report.migrated.is_empty() { "progress restored".into() } else { format!("progress restored, migrated: {:?}", report.migrated) })
    }

    /// What happened to the saved progress at load, if there was any.
    pub fn save_note(&self) -> Option<&str> {
        self.save_note.as_deref()
    }

    /// The running session.
    pub fn session(&self) -> &LocalSession {
        &self.session
    }

    /// Tidewatch, as the match runs it.
    pub fn game(&self) -> &TidewatchGame {
        self.session.sim().game::<TidewatchGame>().expect("a Tidewatch scene")
    }

    fn game_mut(&mut self) -> &mut TidewatchGame {
        self.session.sim_mut().game_mut::<TidewatchGame>().expect("a Tidewatch scene")
    }

    /// What the story shows now.
    pub fn view(&self) -> View {
        let g = self.game();
        g.story().view(&HostAsk { systems: g.systems(), outside: self.session.rules() }).unwrap_or(View { revision: 0, talk: None, bark: None })
    }

    /// The presenter.
    pub fn presenter(&self) -> &Presenter {
        &self.presenter
    }

    /// Writes the progress now (the game's save and the scene's variables), atomically.
    pub fn save_now(&mut self) -> Result<(), String> {
        let Some(path) = self.save.clone() else { return Ok(()) };
        let mut v = self.game().save();
        v["scene_vars"] = json!(self.session.rules().vars().into_iter().collect::<std::collections::BTreeMap<_, _>>());
        let (feet, yaw) = (self.session.player_feet(), self.session.player().yaw.to_degrees());
        v["player"] = json!([feet.x, feet.y, feet.z, yaw]);
        self.since_save = 0.0;
        red_engine2::sim::systems::write_atomic(&path, &v.to_string())
    }

    /// The speaker the player stands near, if any (`(id, name)`).
    pub fn near(&self) -> Option<(&'static str, &'static str)> {
        SPEAKERS.iter().find(|(_, var, _)| self.session.rules().var(var).unwrap_or(0.0) != 0.0).map(|(id, _, name)| (*id, *name))
    }

    /// Sends an intention to the authority; a refusal is kept to show.
    pub fn intend(&mut self, intent: &Intent) {
        let slot = self.session.slot();
        self.refused = self.session.sim_mut().intend(slot, intent).err();
    }

    /// The camera: behind and above the player, looking ahead (north).
    pub fn camera_view(&self) -> ViewCamera {
        let feet = self.session.player_feet();
        ViewCamera::look_at(feet + Vec3::new(0.0, 5.5, 7.5), feet + Vec3::new(0.0, 1.0, -2.0))
    }

    /// One frame with `controls` (from the devices) and the movement axes. Returns `false` to quit (after saving).
    pub fn frame_with(&mut self, controls: &Controls, (forward, strafe): (i8, i8), escape: bool, save: bool, frame: Frame) -> bool {
        if escape {
            let _ = self.save_now();
            return false;
        }
        if save {
            let _ = self.save_now();
        }
        let view = self.view();
        let mut move_input = PlayerInput { forward, strafe, ..Default::default() };
        if Presenter::captures_input(&view) || view.talk.is_some() {
            // The conversation has the input: nothing reaches play.
            move_input = PlayerInput::default();
            if let Some(i) = self.presenter.update(&view, controls, frame.dt, self.last_layout.as_ref()) {
                self.intend(&i);
            }
        } else if controls.confirm {
            match self.near() {
                Some((who, _)) => self.intend(&Intent { name: "story:talk".into(), subject: Some(who.into()), value: view.revision as i64 }),
                None => {
                    let _ = self.presenter.update(&view, controls, frame.dt, None);
                }
            }
        } else if let Some(i) = self.presenter.update(&view, controls, frame.dt, None) {
            self.intend(&i);
        }
        self.session.advance(frame.dt, |_| move_input);
        self.since_save += frame.dt;
        if self.since_save >= AUTOSAVE_SECS {
            let _ = self.save_now();
        }
        self.sync_marker();
        true
    }

    fn pad_sample(&mut self, focused: bool) -> Option<Sample> {
        self.pad.as_mut().map(|c| c.poll(focused))
    }

    fn sync_marker(&mut self) {
        let p = self.session.player();
        if p.velocity.length() > 0.2 {
            self.facing = p.velocity.x.atan2(p.velocity.y).to_degrees();
        }
        let (feet, facing) = (self.session.player_feet(), self.facing);
        place_object(self.session.scene_mut(), PLAYER, feet, Some(facing));
    }

    /// The scene's HUD (title, counters, objective), the talk prompt, and the story's subtitles and choices.
    pub fn hud_layout(&self, w: u32, h: u32) -> Layout {
        let mut l = self.session.hud().layout_scene(self.session.scene(), w, h);
        let view = self.view();
        let s = (h as i32 / 360).max(1);
        if view.talk.is_none() {
            if let Some((_, name)) = self.near() {
                l.label_fit(
                    "talk-prompt",
                    None,
                    w as i32 / 2,
                    h as i32 / 2 + 40 * s,
                    &format!("E / A: TALK TO {name}"),
                    s,
                    w as i32 - 16,
                    [255, 230, 160, 255],
                );
            }
        }
        self.presenter.draw(&mut l, &view, w, h);
        l
    }
}

impl ClientGame for Tidewatch {
    fn scene(&self) -> &Scene {
        self.session.scene()
    }

    fn update(&mut self, input: &InputState, frame: Frame) -> bool {
        let pad = self.pad_sample(input.focused());
        let controls = Controls::from_input(input, pad.as_ref());
        let (mut forward, mut strafe) = input.wasd();
        if let Some(p) = pad {
            let m = red_engine2::controller::stick(p.movement, 0.25);
            if forward == 0 && strafe == 0 {
                (forward, strafe) = ((m.y.signum() * (m.y.abs() > 0.3) as i32 as f32) as i8, (m.x.signum() * (m.x.abs() > 0.3) as i32 as f32) as i8);
            }
        }
        let ok = self.frame_with(&controls, (forward, strafe), input.pressed(KeyCode::Escape), input.pressed(KeyCode::F5), frame);
        self.last_layout = Some(self.hud_layout(frame.width, frame.height));
        ok
    }

    fn camera(&self, _width: u32, _height: u32) -> ViewCamera {
        self.camera_view()
    }

    fn hidden(&self) -> Vec<String> {
        self.session.hidden().map(str::to_string).collect()
    }

    fn hud_key(&self) -> String {
        let v = self.view();
        format!(
            "{}|{}|{}|{}|{}|{:?}",
            self.session.hud().key(),
            v.revision,
            self.presenter.shown(),
            self.presenter.focus(),
            self.near().map_or("", |n| n.0),
            self.refused
        )
    }

    fn hud(&self, width: u32, height: u32) -> Option<Layout> {
        Some(self.hud_layout(width, height))
    }

    fn title(&self) -> String {
        "Tidewatch".into()
    }
}
