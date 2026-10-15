//! Tidewatch as the match's game: the watch (`tw:watch`) and the story (`tidewatch:story`, the engine's `sim::story` runtime on the
//! compiled package) run as one `Systems` host inside the match's one authority (`MatchGame::tick`), on a server, in a local session and
//! in `sim` scenarios alike. There is no second simulation: the story reads the watch through its queries and the scene through
//! `scene:<var>`, and changes the watch only through its commands, applied as batches.
//!
//! The scene and the game meet through the scene's variables: zone rules set `at_shed`, `at_brazier`, `at_bell`, `near_odile`, `near_bram`
//! as the player walks; the game turns being at the shed or the brazier into `tw:take` and `tw:feed`, and the bell into `tw:ring`; the
//! `show` variables carry the game's answers back to the HUD. While a conversation is open the player is held still (`input_locked`): the
//! world does not pause, the watch's clock and the brazier keep burning.

use crate::content::TidewatchSpec;
use crate::watch::{phase, Watch};
use red_engine2::sim::defs::{Diagnostic, Id};
use red_engine2::sim::game::{Intent, MatchGame};
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::story::runtime::{self as story, Story};
use red_engine2::sim::systems::{Command, LoadReport, Systems};
use serde_json::{json, Value};
use std::sync::Arc;

/// The save's own kind (`"game"` in a save file).
pub const SAVE_KIND: &str = "tidewatch";

/// Problems kept for inspection.
const PROBLEMS: usize = 16;

/// Tidewatch, running.
pub struct TidewatchGame {
    spec: Arc<TidewatchSpec>,
    systems: Systems,
    problems: Vec<Diagnostic>,
}

fn id(s: &str) -> Id {
    Id::parse(s).unwrap_or_else(|e| panic!("{e}"))
}

impl TidewatchGame {
    /// A night on the island: the shed stocked, every watch ahead, the story at its start.
    pub fn new(spec: Arc<TidewatchSpec>) -> Result<TidewatchGame, Vec<Diagnostic>> {
        let ledger = Watch::ledger(&spec.rules).map_err(|e| vec![e])?;
        let systems = Systems::new(vec![Box::new(Watch::new(spec.rules.clone())), Box::new(Story::new(spec.story.clone()))], ledger, spec.seed)?;
        Ok(TidewatchGame { spec, systems, problems: Vec::new() })
    }

    /// The systems (read).
    pub fn systems(&self) -> &Systems {
        &self.systems
    }

    /// The watch.
    pub fn watch(&self) -> &Watch {
        self.systems.get::<Watch>().unwrap_or_else(|| panic!("Tidewatch runs a watch"))
    }

    /// The story.
    pub fn story(&self) -> &Story {
        self.systems.get::<Story>().unwrap_or_else(|| panic!("Tidewatch runs a story"))
    }

    /// A command now (a test's, a tool's), with `outside` (the match's rules) answering the scene's variables; refused ones change nothing.
    pub fn command(&mut self, outside: &dyn red_engine2::sim::systems::Outside, c: &Command) -> Result<(), Diagnostic> {
        self.systems.command_with(c, outside)
    }

    /// Problems seen while running, newest last.
    pub fn problems(&self) -> &[Diagnostic] {
        &self.problems
    }

    fn note(&mut self, d: Diagnostic) {
        if self.problems.last() != Some(&d) {
            if self.problems.len() == PROBLEMS {
                self.problems.remove(0);
            }
            self.problems.push(d);
        }
    }

    /// Everything to save: `{"game": "tidewatch", "story": file, "systems": <the host's save>}`.
    pub fn save(&self) -> Value {
        json!({"game": SAVE_KIND, "story": self.spec.story_file, "systems": self.systems.save()})
    }

    /// Loads a [`save`](TidewatchGame::save); another game's save, or one this version cannot read, is refused and nothing changes.
    pub fn load(&mut self, saved: &Value) -> Result<LoadReport, Diagnostic> {
        match saved.get("game").and_then(Value::as_str) {
            Some(SAVE_KIND) => {}
            other => return Err(Diagnostic::new(red_engine2::sim::defs::code::STATE_VERSION, "save", format!("not a Tidewatch save (`game` is {other:?})"))),
        }
        self.systems.load(saved.get("systems").unwrap_or(&Value::Null))
    }
}

impl MatchGame for TidewatchGame {
    fn name(&self) -> &'static str {
        "tidewatch"
    }

    fn tick(&mut self, sim: &mut MatchSim) {
        let var = |n: &str| sim.rules().var(n).unwrap_or(0.0) != 0.0;
        let q = |s: &Systems, n: &str| s.query(&id(n), sim.rules()).unwrap_or(0.0);
        let mut want = Vec::new();
        if !self.story().locks_input() {
            if var("at_shed") && q(&self.systems, "tw:carrying") == 0.0 && q(&self.systems, "tw:shed_fuel") > 0.0 {
                want.push("tw:take");
            }
            if var("at_brazier") && q(&self.systems, "tw:carrying") != 0.0 {
                want.push("tw:feed");
            }
            if var("at_lamp") && q(&self.systems, "tw:carrying") != 0.0 {
                want.push("tw:stock_lamp");
            }
            if var("at_bell") && q(&self.systems, "tw:phase") == phase::ASHORE as f64 {
                want.push("tw:ring");
            }
        }
        for c in want {
            if let Err(e) = self.systems.command_with(&Command { name: id(c), subject: None, value: 0 }, sim.rules()) {
                self.note(e);
            }
        }
        if let Err(e) = self.systems.tick_with(sim.rules()) {
            self.note(e);
        }
        let shown: Vec<(String, f64)> = self.spec.show.iter().map(|(v, q)| (v.clone(), self.systems.query(q, sim.rules()).unwrap_or(0.0))).collect();
        for (v, x) in shown {
            sim.set_var(&v, x);
        }
    }

    fn intend(&mut self, sim: &mut MatchSim, _slot: usize, intent: &Intent) -> Result<(), String> {
        story::intend(&mut self.systems, sim.rules(), intent)
    }

    fn input_locked(&self) -> bool {
        self.story().locks_input()
    }

    fn state_hash(&self) -> Option<u64> {
        Some(self.systems.hash())
    }

    fn inspect(&self, sim: &MatchSim, target: &str) -> Option<Value> {
        if target == "systems" {
            return Some(json!({"systems": self.systems.catalog(), "order": self.systems.order(), "story": self.spec.story_file,
                "halted": self.systems.halted(), "problems": self.problems, "recent_events": self.systems.recent_events().collect::<Vec<_>>()}));
        }
        if let Some(v) = story::inspect(&self.systems, sim.rules(), target) {
            return Some(v);
        }
        self.systems.inspect(target).and_then(|i| serde_json::to_value(i).ok())
    }
}
