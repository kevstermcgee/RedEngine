//! Workshop as the match's game: its systems (`ws:supply`, `ws:machines`, `ws:shipping`) and their ledger run inside the match's one
//! authority (`MatchGame::tick`), on the server, in a local session and in `sim` scenarios alike.
//!
//! The scene and the game meet through the scene's own variables: a switch's variable (set by the scene's rules when the player steps on the
//! lever) routes a machine, and the `show` variables (what a sink has shipped) feed the objective, the HUD and `checks.sim`. The game's own
//! state is in the match checksum (`state_hash`), explained by `inspect` (`sim --inspect ws:furnace_a`), saved and loaded with versions
//! ([`WorkshopGame::save`], [`WorkshopGame::load`]) and sent to clients as a snapshot part ([`crate::wire`]).

use crate::content::{Catalog, WorkshopSpec};
use crate::flow::{Shipping, Supply};
use crate::machines::Machines;
use red_engine2::net::protocol::GameMsg;
use red_engine2::sim::defs::{Diagnostic, Id};
use red_engine2::sim::game::MatchGame;
use red_engine2::sim::ledger::{Ledger, Reason, Tx};
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::systems::{Command, LoadReport, Systems};
use serde_json::{json, Value};
use std::sync::Arc;

/// The save's own kind (`"game"` in a save file).
pub const SAVE_KIND: &str = "workshop";

/// Problems kept for inspection (a refused command, a halted host).
const PROBLEMS: usize = 16;

/// The shared stores' owner: the workshop floor itself.
pub fn floor() -> Id {
    Id::parse("workshop:floor").unwrap_or_else(|e| panic!("{e}"))
}

/// Workshop, running.
pub struct WorkshopGame {
    spec: Arc<WorkshopSpec>,
    systems: Systems,
    problems: Vec<Diagnostic>,
}

impl WorkshopGame {
    /// The workshop of `spec`, every store filled with its `start`, every machine idle.
    pub fn new(spec: Arc<WorkshopSpec>) -> Result<WorkshopGame, Vec<Diagnostic>> {
        let cat = spec.catalog.clone();
        let mut ledger = Ledger::new();
        for (r, _) in cat.resources.iter() {
            ledger.resource(*r);
        }
        for (r, _) in cat.recipes.iter() {
            ledger.declare(Reason::Recipe(*r));
        }
        for (s, _) in cat.sources.iter() {
            ledger.declare(Reason::Source(*s));
        }
        for (s, _) in cat.sinks.iter() {
            ledger.declare(Reason::Sink(*s));
        }
        let start = Reason::Source(Catalog::start_source());
        ledger.declare(start);
        let mut errs = Vec::new();
        for (sid, store) in cat.stores.iter() {
            if let Err(e) = ledger.add_store(*sid, floor(), store.capacity) {
                errs.push(e);
            }
            for (r, n) in &store.start {
                if let Err(e) = ledger.apply(&Tx::new().producing(sid, r, *n, &start)) {
                    errs.push(e);
                }
            }
        }
        for (store, owner, cap) in Machines::stores(&cat) {
            if let Err(e) = ledger.add_store(store, owner, cap) {
                errs.push(e);
            }
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        let systems =
            Systems::new(vec![Box::new(Supply::new(cat.clone())), Box::new(Machines::new(cat.clone())), Box::new(Shipping::new(cat))], ledger, spec.seed)?;
        Ok(WorkshopGame { spec, systems, problems: Vec::new() })
    }

    /// The systems (read).
    pub fn systems(&self) -> &Systems {
        &self.systems
    }

    /// The content.
    pub fn catalog(&self) -> &Catalog {
        &self.spec.catalog
    }

    /// Units sink `sink` has shipped.
    pub fn shipped(&self, sink: &Id) -> u64 {
        self.systems.get::<Shipping>().and_then(|s| s.shipped.get(sink).copied()).unwrap_or(0)
    }

    /// A player's or a tool's command (`ws:route`, `ws:stop`, `ws:dismantle`), applied now; refused ones change nothing.
    pub fn command(&mut self, c: &Command) -> Result<(), Diagnostic> {
        self.systems.command(c)
    }

    /// Problems seen while running (a refused switch position, a halted host), newest last.
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

    /// Everything to save: `{"game": "workshop", "content": [files], "systems": <the systems' save>}`.
    pub fn save(&self) -> Value {
        json!({"game": SAVE_KIND, "content": self.spec.catalog.files, "systems": self.systems.save()})
    }

    /// Loads a [`save`](WorkshopGame::save), migrating older state; a save of another game, or one this version cannot read, is refused and
    /// nothing changes.
    pub fn load(&mut self, saved: &Value) -> Result<LoadReport, Diagnostic> {
        match saved.get("game").and_then(Value::as_str) {
            Some(SAVE_KIND) => {}
            other => return Err(Diagnostic::new(red_engine2::sim::defs::code::STATE_VERSION, "save", format!("not a Workshop save (`game` is {other:?})"))),
        }
        self.systems.load(saved.get("systems").unwrap_or(&Value::Null))
    }
}

impl WorkshopGame {
    /// A definition explained: where it is written, what it uses, and what uses it (the instances, recipes and machines an edit to it affects).
    pub fn definition(&self, target: &str) -> Option<Value> {
        let id = Id::find(target)?;
        let cat = &self.spec.catalog;
        let instances_of = |kind: &Id| cat.instances.iter().filter(|(_, i)| i.machine == *kind).map(|(i, _)| *i).collect::<Vec<_>>();
        if let Some(m) = cat.machines.get(&id) {
            return Some(json!({"definition": id, "kind": "machine", "defined_at": cat.machines.provenance(&id), "behavior": crate::machines::system_id(),
                "parameters": {"speed": m.speed, "input": m.input, "output": m.output}, "uses": m.recipes, "used_by": instances_of(&id)}));
        }
        if let Some(r) = cat.recipes.get(&id) {
            let machines: Vec<&Id> = cat.machines.iter().filter(|(_, m)| m.recipes.contains(&id)).map(|(m, _)| m).collect();
            let instances: Vec<Id> = machines.iter().flat_map(|m| instances_of(m)).collect();
            return Some(
                json!({"definition": id, "kind": "recipe", "defined_at": cat.recipes.provenance(&id), "in": r.inputs, "out": r.outputs, "ticks": r.ticks,
                "uses": r.inputs.keys().chain(r.outputs.keys()).collect::<Vec<_>>(), "used_by": {"machines": machines, "instances": instances}}),
            );
        }
        if cat.resources.contains(&id) {
            let recipes: Vec<&Id> = cat.recipes.iter().filter(|(_, r)| r.inputs.contains_key(&id) || r.outputs.contains_key(&id)).map(|(r, _)| r).collect();
            let sinks: Vec<&Id> = cat.sinks.iter().filter(|(_, s)| s.takes.contains_key(&id)).map(|(s, _)| s).collect();
            let sources: Vec<&Id> = cat.sources.iter().filter(|(_, s)| s.gives.contains_key(&id)).map(|(s, _)| s).collect();
            return Some(
                json!({"definition": id, "kind": "resource", "defined_at": cat.resources.provenance(&id), "used_by": {"recipes": recipes, "sources": sources, "sinks": sinks}}),
            );
        }
        None
    }
}

impl MatchGame for WorkshopGame {
    fn name(&self) -> &'static str {
        "workshop"
    }

    fn tick(&mut self, sim: &mut MatchSim) {
        // The player's switches: each one's variable picks where its machine's output goes (setting the same place again changes nothing).
        let routes: Vec<(Id, i64, Id)> = self
            .spec
            .catalog
            .switches
            .iter()
            .filter_map(|(_, s)| {
                let pos = sim.rules().var(&s.var).unwrap_or(0.0).round() as i64;
                let unit = self.systems.get::<Machines>()?.unit(&s.instance)?;
                // A position the switch does not have still goes out, so the refusal is recorded (`problems`).
                match usize::try_from(pos).ok().and_then(|p| s.to.get(p)) {
                    Some(want) if unit.to == *want => None,
                    want => Some((s.instance, pos, want.cloned().unwrap_or(unit.to))),
                }
            })
            .collect();
        for (inst, pos, _) in routes {
            if let Err(e) = self.systems.command(&Command { name: Id::parse("ws:route").unwrap_or_else(|e| panic!("{e}")), subject: Some(inst), value: pos }) {
                self.note(e);
            }
        }
        if let Err(e) = self.systems.tick() {
            self.note(e);
        }
        for (var, sink) in self.spec.show.iter() {
            sim.set_var(var, self.shipped(sink) as f64);
        }
    }

    fn state_hash(&self) -> Option<u64> {
        Some(self.systems.hash())
    }

    fn inspect(&self, _sim: &MatchSim, target: &str) -> Option<Value> {
        if target == "systems" {
            let cat = &self.spec.catalog;
            return Some(json!({
                "systems": self.systems.catalog(),
                "order": self.systems.order(),
                "content": cat.files,
                "definitions": {
                    "resources": cat.resources.iter().map(|(i, _)| i).collect::<Vec<_>>(),
                    "recipes": cat.recipes.iter().map(|(i, _)| json!({"id": i, "at": cat.recipes.provenance(i)})).collect::<Vec<_>>(),
                    "machines": cat.machines.iter().map(|(i, _)| json!({"id": i, "at": cat.machines.provenance(i)})).collect::<Vec<_>>(),
                    "stores": cat.stores.iter().map(|(i, _)| i).collect::<Vec<_>>(),
                },
                "halted": self.systems.halted(),
                "problems": self.problems,
                "recent_events": self.systems.recent_events().collect::<Vec<_>>(),
            }));
        }
        if let Some(found) = self.systems.inspect(target).and_then(|i| serde_json::to_value(i).ok()) {
            return Some(found);
        }
        self.definition(target)
    }

    fn snapshot_part(&self, _sim: &MatchSim, _viewer: usize) -> Option<GameMsg> {
        Some(crate::wire::encode(self))
    }
}
