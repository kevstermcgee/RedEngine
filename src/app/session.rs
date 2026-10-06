//! A local game session for any client: one player in the authoritative [`MatchSim`] (the exact simulation
//! `red_server` runs: movement, props, combat, scene rules), advanced by real time on the fixed 60 Hz clock and observed
//! through plain data. A custom client feeds it [`PlayerInput`]s and draws what it reports; it never re-implements
//! movement or rules.
//!
//! The session keeps two scenes apart on purpose: the validated scene the simulation was built from (its collision,
//! rules and props are fixed at load), and the *presentation* copy returned by [`LocalSession::scene_mut`], which a
//! client may decorate (move a player marker, add effects) without changing gameplay.

use crate::app::hud::{HudState, RecentEvent};
use crate::player::Character;
use crate::schema::Scene;
use crate::sim::clock::TickClock;
use crate::sim::match_sim::MatchSim;
use crate::sim::player::{PlayerInput, PlayerState};
use crate::sim::rules_run::RulesEngine;
use crate::sim::spawns::{parse_spawns, Spawn};
use glam::{Vec2, Vec3};
use std::path::Path;

/// One local player in a running match. See the module docs.
pub struct LocalSession {
    scene: Scene,
    sim: MatchSim,
    slot: usize,
    clock: TickClock,
    prev: PlayerState,
    seq: u32,
    recent: RecentEvent,
}

impl LocalSession {
    /// Loads and strictly validates `path` (the same checks as `red_engine2 validate`) and starts a match with one
    /// player at the first spawn (a scene with no `spawns` array starts at the camera's position). Errors are `object.field: message`
    /// lines.
    pub fn load(path: &Path) -> Result<Self, Vec<String>> {
        let text = std::fs::read_to_string(path).map_err(|e| vec![format!("{}: {e}", path.display())])?;
        Self::from_json(&text)
    }

    /// [`LocalSession::load`] from scene JSON text.
    pub fn from_json(text: &str) -> Result<Self, Vec<String>> {
        let scene = crate::schema::parse_scene(text)?;
        let spawns = parse_spawns(text).map_err(|e| vec![e])?;
        Self::new(scene, spawns, None).map_err(|e| vec![e])
    }

    /// A match on an already-validated `scene`. `character` defaults to the scene's `player.character` policy, else human.
    pub fn new(scene: Scene, spawns: Vec<Spawn>, character: Option<Character>) -> Result<Self, String> {
        let mut sim = MatchSim::try_new(&scene, spawns)?;
        let who = scene.player.character.or(character).unwrap_or(Character::Human);
        let slot = sim.add_player(who).ok_or("the match has no free player slot")?;
        let prev = sim.player(slot).map(|p| p.state).ok_or("the player did not join")?;
        Ok(LocalSession { scene, sim, slot, clock: TickClock::default(), prev, seq: 0, recent: RecentEvent::default() })
    }

    /// Runs exactly one simulation tick with `input` (its `seq` is assigned here). Deterministic: the same inputs give
    /// the same match, which is what tests and scripted demos use.
    pub fn step(&mut self, mut input: PlayerInput) {
        self.prev = self.player();
        self.seq = self.seq.wrapping_add(1);
        input.seq = self.seq;
        self.sim.push_input(self.slot, input);
        self.sim.tick_once();
        let events = self.sim.take_events();
        self.recent.observe(&events, self.sim.tick());
    }

    /// Adds `dt` seconds of real time and runs the whole ticks it covers, asking `input` for each one (it sees the
    /// player's state before that tick). Returns how many ticks ran. Use [`Self::alpha`] to draw between ticks.
    pub fn advance(&mut self, dt: f32, mut input: impl FnMut(&PlayerState) -> PlayerInput) -> u32 {
        self.clock.push_time(dt);
        let mut n = 0;
        while self.clock.next_tick().is_some() {
            let i = input(&self.player());
            self.step(i);
            n += 1;
        }
        n
    }

    /// How far real time is into the next tick (0..1).
    pub fn alpha(&self) -> f32 {
        self.clock.alpha()
    }

    /// Brings back the variables the scene keeps between sessions (`persist`), before the first tick. Unknown names are ignored.
    pub fn restore_vars(&mut self, saved: &std::collections::BTreeMap<String, f64>) {
        self.sim.restore_vars(saved);
    }

    /// The variables the scene keeps between sessions (`persist`) with their values now: what to save.
    pub fn persisted(&self) -> std::collections::BTreeMap<String, f64> {
        self.sim.rules().persisted().into_iter().collect()
    }

    /// The player's state after the latest tick.
    pub fn player(&self) -> PlayerState {
        self.sim.player(self.slot).map(|p| p.state).unwrap_or(self.prev)
    }

    /// The player's feet, interpolated between the last two ticks by [`Self::alpha`] (what to draw).
    pub fn player_feet(&self) -> Vec3 {
        let (a, b) = (self.prev, self.player());
        let t = self.alpha();
        let p: Vec2 = a.pos.lerp(b.pos, t);
        Vec3::new(p.x, a.foot_y + (b.foot_y - a.foot_y) * t, p.y)
    }

    /// Ticks run so far.
    pub fn tick(&self) -> u64 {
        self.sim.tick()
    }

    /// The scene's rules state (variables, hidden objects, outcome, history).
    pub fn rules(&self) -> &RulesEngine {
        self.sim.rules()
    }

    /// The match outcome once a rule has ended it.
    pub fn outcome(&self) -> Option<&str> {
        self.sim.rules().ended()
    }

    /// Object ids the rules have hidden (pass to `LiveRenderer::set_hidden_objects`).
    pub fn hidden(&self) -> impl Iterator<Item = &str> {
        self.sim.rules().hidden()
    }

    /// What the standard rules HUD shows now.
    pub fn hud(&self) -> HudState {
        HudState::from_rules(self.sim.rules(), self.recent.current(self.sim.tick()))
    }

    /// The presentation scene (validated at load).
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The presentation scene, to move markers or decorations. Changing it does not change gameplay: the simulation's
    /// collision, props and rules were built at load.
    pub fn scene_mut(&mut self) -> &mut Scene {
        &mut self.scene
    }

    /// The underlying simulation (props, spawns, recording a trace).
    pub fn sim(&self) -> &MatchSim {
        &self.sim
    }

    /// The underlying simulation, mutable (`start_recording`, `apply_impulse`).
    pub fn sim_mut(&mut self) -> &mut MatchSim {
        &mut self.sim
    }

    /// The local player's slot in the match.
    pub fn slot(&self) -> usize {
        self.slot
    }
}

/// An input that walks from `state` toward the ground point `target` (x, z) at walking pace, or stands still within
/// `arrive` metres: click-to-move and scripted routes without touching movement code.
pub fn input_toward(state: &PlayerState, target: Vec2, arrive: f32) -> PlayerInput {
    let d = target - state.pos;
    if d.length() <= arrive {
        return PlayerInput { yaw: state.yaw, pitch: state.pitch, ..Default::default() };
    }
    PlayerInput { forward: 1, yaw: d.x.atan2(-d.y), pitch: state.pitch, ..Default::default() }
}

/// Moves a presentation object: sets `id`'s `position` (and, if given, its yaw in degrees) to constants. Returns
/// `false` when no top-level object has that id. Purely visual (see [`LocalSession::scene_mut`]).
pub fn place_object(scene: &mut Scene, id: &str, position: Vec3, yaw_deg: Option<f32>) -> bool {
    let Some(o) = scene.objects.iter_mut().find(|o| o.id == id) else { return false };
    o.position = crate::track::Track::constant(position);
    if let Some(y) = yaw_deg {
        o.rotation = crate::track::Track::constant(Vec3::new(0.0, y, 0.0));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r##"{
      "camera": {"position": [0, 10, 6], "target": [0, 0, 0]},
      "spawns": [{"id": "start", "position": [0, 0, 4], "yaw_deg": 0}],
      "zones": [{"id": "goal", "rect": [-1, -5, 1, -3]}],
      "vars": {"reached": 0},
      "rules": [{"id": "win", "when": {"enter": {"zone": "goal"}}, "do": [{"set": ["reached", 1]}, {"emit": "made_it"}, {"end": "victory"}]}],
      "objects": [{"id": "floor", "type": "box", "size": [12, 0.2, 12], "position": [0, -0.1, 0], "material": {"color": "#808080"}},
                  {"id": "marker", "type": "cylinder", "radius": 0.3, "height": 1, "collide": false, "material": {"color": "#ff0000"}}]
    }"##;

    #[test]
    fn a_session_walks_through_the_real_simulation_and_the_rules_end_it() {
        let mut s = LocalSession::from_json(SCENE).unwrap();
        assert_eq!(s.player().pos, Vec2::new(0.0, 4.0));
        let mut ticks = 0;
        while s.outcome().is_none() && ticks < 600 {
            s.step(input_toward(&s.player(), Vec2::new(0.0, -4.0), 0.1));
            ticks += 1;
        }
        assert_eq!(s.outcome(), Some("victory"), "after {ticks} ticks at {:?}", s.player().pos);
        let hud = s.hud();
        assert_eq!(hud.vars, vec![("reached".to_string(), 1.0)]);
        assert_eq!(hud.event.as_deref(), Some("made_it"));
        assert_eq!(hud.outcome.as_deref(), Some("victory"));
    }

    #[test]
    fn advance_runs_whole_ticks_and_the_presentation_scene_is_separate() {
        let mut s = LocalSession::from_json(SCENE).unwrap();
        assert_eq!(s.advance(0.105, |_| PlayerInput::default()), 6);
        assert!(s.alpha() < 1.0);
        let feet = s.player_feet();
        assert!(place_object(s.scene_mut(), "marker", feet, Some(90.0)));
        assert!(!place_object(s.scene_mut(), "nope", feet, None));
        assert_eq!(s.tick(), 6);
    }

    #[test]
    fn without_spawns_the_player_starts_under_the_camera_and_bad_spawns_are_errors() {
        let s = LocalSession::from_json(r#"{"camera": {"position": [2, 2, 5], "target": [0, 0, 0]}, "objects": []}"#).unwrap();
        assert_eq!(s.player().pos, Vec2::new(2.0, 5.0));
        let err = LocalSession::from_json(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "spawns": [{"id": "a"}], "objects": []}"#).err().unwrap();
        assert!(err.join(" ").contains("spawn"), "{err:?}");
    }
}
