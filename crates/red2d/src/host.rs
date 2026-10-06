//! The platform boundary, in one place: everything a *platform* (the browser's JavaScript today, a native window later) needs from a 2D game, as plain method calls with no
//! JavaScript, files, clocks or threads in them.
//!
//! The platform feeds in time (a number of ticks to run), input (keys, actions, a pointer, clicks) and saved text, and reads out a picture (RGBA bytes), sound requests, the text to save and a JSON
//! snapshot. [`crate::web`] wraps this struct in the WebAssembly exports; the unit tests below drive it natively, so most of "the browser build works" is checked without a browser.
//! What stays outside, in `runtime.js`: the animation-frame loop, the canvas, `localStorage`, Web Audio and the gamepad. Those are checked only by a real browser run.

use crate::game::{self, GameDef};
use crate::render::{self, Frame, Layout};
use crate::script;
use crate::sim::{SaveStatus, Sim};
use std::sync::Arc;

/// A loaded game and its output buffers.
pub struct Host {
    sim: Option<Sim>,
    frame: Frame,
    layout: Layout,
    window: (u32, u32),
}

/// The codes `load_save` returns.
pub const SAVE_FRESH: i32 = 0;
/// Saved values restored.
pub const SAVE_LOADED: i32 = 1;
/// A save from another game or format: ignored.
pub const SAVE_INCOMPATIBLE: i32 = 2;
/// Not a save: ignored.
pub const SAVE_CORRUPT: i32 = 3;

impl Default for Host {
    fn default() -> Self {
        Host { sim: None, frame: Frame::new(1, 1, [0; 4]), layout: Layout { x: 0, y: 0, w: 1, h: 1 }, window: (1, 1) }
    }
}

impl Host {
    /// Parses the game text and starts it. The error lists every problem.
    pub fn init(&mut self, game_text: &str, seed: u64) -> Result<(), String> {
        let def = game::parse(game_text).map_err(|e| e.join("\n"))?;
        let sim = Sim::new(Arc::new(def), seed);
        self.frame = Frame::new(sim.def.view.width, sim.def.view.height, sim.def.view.background);
        self.sim = Some(sim);
        self.set_window(self.window.0, self.window.1);
        self.render();
        Ok(())
    }

    /// Hot reload: replaces the running game with `game_text` and carries over the named entities' places (see [`Sim::carry_over`]); returns how many. On an error nothing
    /// changes and the old game keeps running.
    pub fn reload(&mut self, game_text: &str, seed: u64) -> Result<usize, String> {
        let def = game::parse(game_text).map_err(|e| e.join("\n"))?;
        let mut sim = Sim::new(Arc::new(def), seed);
        let carried = self.sim.as_ref().map_or(0, |old| sim.carry_over(old));
        self.frame = Frame::new(sim.def.view.width, sim.def.view.height, sim.def.view.background);
        self.sim = Some(sim);
        self.set_window(self.window.0, self.window.1);
        self.render();
        Ok(carried)
    }

    /// Test helper: the x of the scene entity `p`.
    #[cfg(test)]
    fn player_x(&self) -> f32 {
        self.sim.as_ref().and_then(|s| s.entities.iter().find(|e| e.scene_id.as_deref() == Some("p"))).map(|e| e.x).unwrap()
    }

    fn sim(&mut self) -> &mut Sim {
        self.sim.as_mut().expect("init first")
    }

    /// The loaded game.
    pub fn def(&self) -> Option<&Arc<GameDef>> {
        self.sim.as_ref().map(|s| &s.def)
    }

    /// Runs `n` ticks (the platform decides how many from its clock).
    pub fn step(&mut self, n: u32) {
        if let Some(s) = self.sim.as_mut() {
            s.run_ticks(u64::from(n));
        }
    }

    /// Draws the current state into the frame buffer.
    pub fn render(&mut self) {
        if let Some(s) = &self.sim {
            self.frame = render::render(s);
        }
    }

    /// The picture, `view_w * view_h * 4` RGBA bytes.
    pub fn frame(&self) -> &[u8] {
        &self.frame.rgba
    }

    /// The virtual screen size.
    pub fn view_size(&self) -> (u32, u32) {
        (self.frame.w, self.frame.h)
    }

    /// A key (`KeyboardEvent.code`) goes down or up.
    pub fn key(&mut self, code: &str, down: bool) {
        self.sim().key(code, down);
    }

    /// An action (`left`, `action`, ...) is held or released (a gamepad, a touch control).
    pub fn action(&mut self, name: &str, down: bool) -> bool {
        self.sim().set_action(name, down)
    }

    /// The pointer moved, in virtual screen pixels.
    pub fn pointer(&mut self, x: f32, y: f32) {
        self.sim().set_pointer(x, y);
    }

    /// The pointer was pressed, in virtual screen pixels.
    pub fn click(&mut self, x: f32, y: f32) {
        self.sim().click(x, y);
    }

    /// Tells the host the window's size; returns where the virtual screen lands in it.
    pub fn set_window(&mut self, ww: u32, wh: u32) -> Layout {
        self.window = (ww, wh);
        if let Some(s) = &self.sim {
            self.layout = render::layout(s.def.view.width, s.def.view.height, ww, wh, s.def.view.scale);
        }
        self.layout
    }

    /// A window position to a virtual-screen position, if it is on the screen and not in the bars.
    pub fn window_to_view(&self, wx: f32, wy: f32) -> Option<[f32; 2]> {
        let (w, h) = self.view_size();
        render::window_to_view(self.layout, w, h, wx, wy)
    }

    /// A JSON snapshot of the state (tick, outcome, variables, hash).
    pub fn snapshot_json(&self) -> String {
        self.sim.as_ref().map_or_else(|| "null".to_string(), |s| s.snapshot().to_string())
    }

    /// The text to save, once, if there is something new to keep.
    pub fn take_save(&mut self) -> Option<String> {
        self.sim.as_mut().and_then(Sim::take_save_if_dirty)
    }

    /// Restores saved text; returns a code and a sentence.
    pub fn load_save(&mut self, text: &str) -> (i32, String) {
        let st = self.sim().load_save(text);
        let code = match &st {
            SaveStatus::Fresh => SAVE_FRESH,
            SaveStatus::Loaded(_) => SAVE_LOADED,
            SaveStatus::Incompatible(_) => SAVE_INCOMPATIBLE,
            SaveStatus::Corrupt(_) => SAVE_CORRUPT,
        };
        (code, st.to_string())
    }

    /// Sounds to play now (indices into the game's sounds).
    pub fn take_sounds(&mut self) -> Vec<u32> {
        self.sim().take_sounds().into_iter().map(|i| i as u32).collect()
    }

    /// Whether the game wants music on.
    pub fn music_on(&self) -> bool {
        self.sim.as_ref().is_some_and(|s| s.music_on)
    }

    /// Mono samples of sound `i`.
    pub fn sound_pcm(&self, i: usize) -> Result<Vec<f32>, String> {
        let d = self.sim.as_ref().ok_or("init first")?.def.clone();
        if i >= d.sounds.len() {
            return Err(format!("no sound {i}"));
        }
        crate::sound::voice_pcm(&d, i)
    }

    /// Interleaved stereo samples of the first music track, if the game has one.
    pub fn music_pcm(&self) -> Result<Option<Vec<f32>>, String> {
        let d = self.sim.as_ref().ok_or("init first")?.def.clone();
        if d.music.is_empty() {
            return Ok(None);
        }
        crate::sound::music_pcm(&d, 0).map(|(p, _)| Some(p))
    }

    /// Runs every scenario of the game in this host (a separate simulation; the live one is untouched) and reports `[{name, ok, hash, ticks, failures}]` as JSON.
    pub fn scenarios_json(&self) -> String {
        let Some(s) = &self.sim else { return "[]".into() };
        let def = s.def.clone();
        let rows: Vec<serde_json::Value> = def
            .scenarios
            .iter()
            .map(|sc| {
                let (r, _) = script::run_scenario(&def, sc, None);
                serde_json::json!({"name": r.name, "ok": r.ok, "hash": r.hash, "ticks": r.ticks, "failures": r.failures, "smoke": sc.smoke})
            })
            .collect();
        serde_json::Value::Array(rows).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: &str = r##"{"game2d":1,"id":"hosted","title":"Hosted","description":"d",
      "capabilities":{"presentation":"2d","platforms":["web"],"networking":"offline","input":["keyboard","mouse"],"persistence":["progress","settings"]},
      "view":{"width":160,"height":90},
      "sounds":{"pick":{"seconds":0.1,"layers":[{"sine":660,"decay":20}]}},
      "vars":{"score":0,"best":0},"persist":["best"],
      "prefabs":{"player":{"tag":"player","shape":{"rect":[8,8],"color":"#fc0"},"move":{"keys":{"mode":"topdown","speed":60}}}},
      "scene":[{"prefab":"player","at":[20,45],"id":"p"}],
      "rules":[{"when":{"press":"action"},"do":[{"add":["best",1]},{"play":"pick"}]}],
      "checks":{"scenarios":[{"name":"jump about","script":[{"hold":["right"],"seconds":1}],"expect":[{"entity":"p","near":[80,45],"tol":2}],"smoke":true}]}}"##;

    #[test]
    fn a_host_takes_input_runs_renders_saves_and_reports_without_a_browser() {
        let mut h = Host::default();
        assert!(h.def().is_none() && h.snapshot_json() == "null");
        h.init(GAME, 1).unwrap();
        assert_eq!(h.view_size(), (160, 90));
        assert_eq!(h.frame().len(), 160 * 90 * 4);
        // The window and the pointer.
        let l = h.set_window(800, 600);
        assert_eq!((l.w, l.h, l.y), (800, 450, 75));
        assert_eq!(h.window_to_view(400.0, 300.0), Some([80.0, 45.0]));
        assert_eq!(h.window_to_view(400.0, 10.0), None);
        // Time, input and output.
        let before = h.frame().to_vec();
        h.key("ArrowRight", true);
        h.step(30);
        h.render();
        assert_ne!(h.frame(), &before[..], "the player moved on screen");
        h.key("Space", true);
        h.step(1);
        assert_eq!(h.take_sounds(), vec![0]);
        assert!(h.take_sounds().is_empty());
        let save = h.take_save().expect("best changed");
        assert!(h.take_save().is_none());
        // Audio.
        assert!(h.sound_pcm(0).unwrap().len() > 4000);
        assert!(h.sound_pcm(5).is_err());
        assert_eq!(h.music_pcm().unwrap(), None);
        // A fresh host restores the save; a bad one is reported, not fatal.
        let mut h2 = Host::default();
        h2.init(GAME, 1).unwrap();
        assert_eq!(h2.load_save(&save).0, SAVE_LOADED);
        assert_eq!(h2.load_save("{").0, SAVE_CORRUPT);
        assert_eq!(h2.load_save(&save.replace("hosted", "x")).0, SAVE_INCOMPATIBLE);
        let snap: serde_json::Value = serde_json::from_str(&h2.snapshot_json()).unwrap();
        assert_eq!(snap["vars"]["best"], 1.0);
        // Scenarios run on their own simulation.
        let rows: serde_json::Value = serde_json::from_str(&h.scenarios_json()).unwrap();
        assert_eq!(rows[0]["ok"], true, "{rows}");
        assert_eq!(rows[0]["smoke"], true);
    }

    #[test]
    fn a_reload_keeps_named_entities_where_they_were_and_a_refused_text_changes_nothing() {
        let mut h = Host::default();
        h.init(GAME, 1).unwrap();
        h.key("ArrowRight", true);
        h.step(30);
        let moved = h.player_x();
        assert!(moved > 21.0, "the player walked ({moved})");
        // An edit that changes the game (a wider view, a new prefab and entity) keeps the player where they are.
        let edited =
            GAME.replace("\"width\":160", "\"width\":200").replace("\"scene\":[", "\"scene\":[{\"prefab\":\"player\",\"at\":[100,10],\"id\":\"other\"},");
        assert_eq!(h.reload(&edited, 1), Ok(1), "one named entity existed in both: the player");
        assert_eq!(h.view_size(), (200, 90), "the new game's view");
        assert_eq!(h.player_x(), moved, "the player stays where they were");
        // A refused text: the error names the problem and the running game is untouched.
        let e = h.reload(&GAME.replace("\"persist\":[\"best\"]", "\"persist\":[\"nope\"]"), 1).unwrap_err();
        assert!(e.contains("persist[0]"), "{e}");
        assert_eq!((h.view_size(), h.player_x()), ((200, 90), moved));
    }

    #[test]
    fn a_bad_game_fails_init_with_every_problem_listed() {
        let mut h = Host::default();
        let e = h.init(&GAME.replace("\"persist\":[\"best\"]", "\"persist\":[\"nope\"]").replace("\"best\":0", "\"best\":\"x\""), 1).unwrap_err();
        assert!(e.contains("vars.best") && e.contains("persist[0]"), "{e}");
    }
}
