//! The machine behavior (`ws:machines`): one system for every machine instance, each with its own kind, recipe, timer, counters and
//! stores (`<instance>/in`, `<instance>/out`, owned by the instance).
//!
//! Each tick, per instance in id order: **feed** (move up to two cycles' inputs from its `from` store into its input), **start** a cycle
//! when the input holds one cycle's inputs, **count down**, and at the end **craft** in one transaction (consume the inputs, produce the
//! outputs, for the recipe); then **drain** its output into its `to` store. The inputs stay in the machine's own input until the cycle's
//! transaction, so stopping a cycle (`ws:stop`), an output with no room, or dismantling the machine (`ws:dismantle`, which hands both of its
//! stores back to `from`) never loses or duplicates anything.
//!
//! State version 2 (`left` ticks and `made`); version 1 kept the cycle as a fraction done (`progress`) and no count, and is migrated.

use crate::content::{Catalog, Recipe};
use red_engine2::sim::defs::{code, Diagnostic, Id};
use red_engine2::sim::ledger::{Fnv, Ledger, Reason, Res, Slot, SlotOp, Tx, Why};
use red_engine2::sim::systems::{Command, Cx, Inspection, Look, System};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// One machine's state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unit {
    /// Its kind.
    pub kind: Id,
    /// The recipe it runs.
    pub recipe: Id,
    /// The store it takes from.
    pub from: Id,
    /// The store its outputs go to.
    pub to: Id,
    /// Ticks left in the cycle (`None`: no cycle started).
    pub left: Option<u32>,
    /// Cycles finished.
    pub made: u64,
}

/// The machines.
pub struct Machines {
    cat: Arc<Catalog>,
    units: BTreeMap<Id, Unit>,
    /// What the tick needs of each unit (in id order, beside `units`), resolved to ledger slots once; `None` after a command or a load
    /// changed what a unit points at, until the next tick resolves it again.
    hot: Option<Vec<Hot>>,
    /// Every unit's recipe inputs and outputs, flattened (`Hot::inputs`, `Hot::outputs` index it), each recipe's in resource id order.
    flows: Vec<(Res, u32)>,
    /// Scratch kept between ticks so a tick allocates nothing: a transaction and an output's contents.
    ops: Vec<SlotOp>,
    held: Vec<(Res, u32)>,
    crafted: Id,
}

/// One unit, resolved: its stores' slots, its recipe's flows and reason, and its cycle length.
#[derive(Debug, Clone)]
struct Hot {
    input: Slot,
    output: Slot,
    from: Slot,
    to: Slot,
    inputs: std::ops::Range<usize>,
    outputs: std::ops::Range<usize>,
    why: Option<Why>,
    cycle: u32,
}

fn id(s: &str) -> Id {
    Id::parse(s).unwrap_or_else(|e| panic!("{e}"))
}

/// The system's id.
pub fn system_id() -> Id {
    id("ws:machines")
}

impl Machines {
    /// Every instance of the catalog, idle, with its stores added to the ledger by the caller ([`Machines::stores`]).
    pub fn new(cat: Arc<Catalog>) -> Machines {
        let units = cat
            .instances
            .iter()
            .filter_map(|(iid, inst)| {
                let kind = cat.machines.get(&inst.machine)?;
                Some((*iid, Unit { kind: inst.machine, recipe: *kind.recipes.first()?, from: inst.from, to: inst.to, left: None, made: 0 }))
            })
            .collect();
        Machines { cat, units, hot: None, flows: Vec::new(), ops: Vec::new(), held: Vec::new(), crafted: id("ws:crafted") }
    }

    /// Resolves every unit to slots (once, then after a command or a load). A recipe's resources and reason are declared by the game from
    /// the same content, so each resolves; one that did not would be left out of the flows.
    fn resolve(&mut self, ledger: &mut Ledger) {
        self.flows.clear();
        let mut hot = Vec::with_capacity(self.units.len());
        for (inst, u) in &self.units {
            let rec = self.recipe(u);
            let mut flow = |side: &BTreeMap<Id, u32>| {
                let start = self.flows.len();
                self.flows.extend(side.iter().filter_map(|(r, n)| Some((ledger.res(r)?, *n))));
                start..self.flows.len()
            };
            let (inputs, outputs) = (flow(&rec.inputs), flow(&rec.outputs));
            hot.push(Hot {
                input: ledger.key(&Catalog::input_store(inst)),
                output: ledger.key(&Catalog::output_store(inst)),
                from: ledger.key(&u.from),
                to: ledger.key(&u.to),
                inputs,
                outputs,
                why: ledger.why(&Reason::Recipe(u.recipe)),
                cycle: self.cycle_ticks(u),
            });
        }
        self.hot = Some(hot);
    }

    /// The stores each instance owns: `(store, owner, capacity)`.
    pub fn stores(cat: &Catalog) -> Vec<(Id, Id, u32)> {
        let mut out = Vec::new();
        for (iid, inst) in cat.instances.iter() {
            if let Some(k) = cat.machines.get(&inst.machine) {
                out.push((Catalog::input_store(iid), *iid, k.input));
                out.push((Catalog::output_store(iid), *iid, k.output));
            }
        }
        out
    }

    /// An instance's state.
    pub fn unit(&self, inst: &Id) -> Option<&Unit> {
        self.units.get(inst)
    }

    /// Every instance in id order.
    pub fn units(&self) -> impl Iterator<Item = (&Id, &Unit)> {
        self.units.iter()
    }

    fn recipe(&self, u: &Unit) -> Recipe {
        self.cat.recipes.get(&u.recipe).cloned().unwrap_or(Recipe { inputs: BTreeMap::new(), outputs: BTreeMap::new(), ticks: 1 })
    }

    fn cycle_ticks(&self, u: &Unit) -> u32 {
        let speed = self.cat.machines.get(&u.kind).map_or(1.0, |k| k.speed);
        ((self.recipe(u).ticks as f32 / speed).round() as u32).max(1)
    }

    /// Why `inst` cannot go on right now, with the input that would let it (`None`: it can).
    pub fn blocked(&self, inst: &Id, ledger: &red_engine2::sim::ledger::Ledger) -> Option<Diagnostic> {
        let u = self.units.get(inst)?;
        let rec = self.recipe(u);
        let (input, output) = (Catalog::input_store(inst), Catalog::output_store(inst));
        let why = Reason::Recipe(u.recipe);
        if u.left == Some(0) {
            // The cycle is done but its outputs do not fit: name the store that is really full.
            let mut tx = Tx::new();
            for (r, n) in &rec.inputs {
                tx = tx.consuming(&input, r, *n, &why);
            }
            for (r, n) in &rec.outputs {
                tx = tx.producing(&output, r, *n, &why);
            }
            let e = ledger.check(&tx).err()?;
            let to = ledger.store(&u.to);
            let msg = match to {
                Some(s) if s.free() == 0 => format!("its output is full and so is `{}` ({} of {}), where the output goes", u.to, s.used(), s.capacity()),
                _ => e.message.clone(),
            };
            return Some(Diagnostic::new(code::FULL, inst, msg).fix(format!("make room in `{}`, or switch this machine's output elsewhere", u.to)));
        }
        if u.left.is_none() {
            for (r, n) in &rec.inputs {
                let have = ledger.count(&input, r) + ledger.count(&u.from, r);
                if have < *n {
                    let mut msg = format!(
                        "a cycle of `{}` needs {n} `{r}`; it holds {} and `{}` has {}",
                        u.recipe,
                        ledger.count(&input, r),
                        u.from,
                        ledger.count(&u.from, r)
                    );
                    let mut fix = format!("bring {} more `{r}` to `{}`", n - have, u.from);
                    // The store it takes from is full of something else: nothing more can arrive there until room is made.
                    if let Some(s) = ledger.store(&u.from).filter(|s| s.free() == 0) {
                        let held: Vec<String> = s.items().map(|(r, n)| format!("{n} `{r}`")).collect();
                        msg.push_str(&format!(", and `{}` is full ({}), so no more can arrive", u.from, held.join(", ")));
                        fix = format!("make room in `{}` (move or ship what fills it)", u.from);
                    }
                    return Some(Diagnostic::new(code::MISSING, inst, msg).fix(fix));
                }
            }
        }
        None
    }
}

impl System for Machines {
    fn id(&self) -> Id {
        system_id()
    }

    fn after(&self) -> Vec<Id> {
        vec![id("ws:supply")]
    }

    fn emits(&self) -> Vec<Id> {
        vec![id("ws:crafted")]
    }

    fn commands(&self) -> Vec<Id> {
        vec![id("ws:route"), id("ws:stop"), id("ws:dismantle")]
    }

    fn instances(&self) -> Vec<Id> {
        self.units.keys().cloned().collect()
    }

    fn tick(&mut self, cx: &mut Cx) {
        if self.hot.as_ref().is_none_or(|h| h.len() != self.units.len()) {
            self.resolve(cx.ledger);
        }
        let Machines { units, hot: Some(hot), flows, ops, held, crafted, .. } = self else { return };
        for ((inst, u), h) in units.iter_mut().zip(hot.iter()) {
            let (inputs, outputs) = (&flows[h.inputs.clone()], &flows[h.outputs.clone()]);
            // Feed: up to two cycles' worth, as much as is there and fits.
            for &(r, n) in inputs {
                let want = (2 * n).saturating_sub(cx.ledger.count_at(h.input, r));
                let k = want.min(cx.ledger.count_at(h.from, r)).min(cx.ledger.free_at(h.input));
                if k > 0 {
                    let _ = cx.ledger.apply_ops(&[SlotOp::Move { from: h.from, to: h.input, res: r, n: k }]);
                }
            }
            if u.left.is_none() && inputs.iter().all(|&(r, n)| cx.ledger.count_at(h.input, r) >= n) {
                u.left = Some(h.cycle);
            }
            if let Some(left) = u.left {
                let left = left.saturating_sub(1);
                u.left = Some(left);
                if left == 0 {
                    ops.clear();
                    if let Some(why) = h.why {
                        ops.extend(inputs.iter().map(|&(res, n)| SlotOp::Consume { from: h.input, res, n, why }));
                        ops.extend(outputs.iter().map(|&(res, n)| SlotOp::Produce { to: h.output, res, n, why }));
                    }
                    // A full output keeps the cycle finished but not crafted: it crafts the first tick there is room.
                    if cx.ledger.apply_ops(ops).is_ok() {
                        u.left = None;
                        u.made += 1;
                        let _ = cx.emit(crafted, Some(inst), 1);
                    }
                }
            }
            // Drain: everything the output holds that fits where it goes.
            held.clear();
            held.extend(cx.ledger.items_at(h.output));
            for &(r, n) in held.iter() {
                let k = n.min(cx.ledger.free_at(h.to));
                if k > 0 {
                    let _ = cx.ledger.apply_ops(&[SlotOp::Move { from: h.output, to: h.to, res: r, n: k }]);
                }
            }
        }
    }

    fn command(&mut self, c: &Command, cx: &mut Cx) -> Result<(), Diagnostic> {
        self.hot = None;
        let inst = c.subject.ok_or_else(|| Diagnostic::new("missing-field", c.name, "names the machine (`subject`)"))?;
        let Some(u) = self.units.get(&inst).cloned() else {
            return Err(Diagnostic::new(code::UNKNOWN, inst, "no such machine"));
        };
        match c.name.as_str() {
            // Sends the output to a switch position's store: setting it again changes nothing.
            "ws:route" => {
                let to = self
                    .cat
                    .switches
                    .iter()
                    .find(|(_, s)| s.instance == inst)
                    .and_then(|(_, s)| s.to.get(c.value.max(0) as usize).cloned())
                    .ok_or_else(|| Diagnostic::new("out-of-range", inst, format!("no switch position {} for this machine", c.value)))?;
                self.units.insert(inst, Unit { to, ..u });
                Ok(())
            }
            // Abandons the cycle: its inputs stay in the machine's input.
            "ws:stop" => {
                self.units.insert(inst, Unit { left: None, ..u });
                Ok(())
            }
            // Takes the machine off the floor: both of its stores go back to where it took from, or nothing happens.
            "ws:dismantle" => {
                cx.ledger.remove_stores(&[Catalog::input_store(&inst), Catalog::output_store(&inst)], &u.from)?;
                self.units.remove(&inst);
                Ok(())
            }
            other => Err(Diagnostic::new(code::EVENT, other, "not a machine command")),
        }
    }

    fn version(&self) -> u32 {
        2
    }

    fn save(&self) -> Value {
        json!(self.units)
    }

    fn load(&mut self, state: Value) -> Result<(), String> {
        let units: BTreeMap<Id, Unit> = serde_json::from_value(state).map_err(|e| e.to_string())?;
        if let Some(gone) = units.keys().find(|i| !self.cat.instances.contains(i)) {
            return Err(format!("the save has a machine `{gone}` that the content does not define"));
        }
        self.units = units;
        self.hot = None;
        Ok(())
    }

    /// Version 1 kept `progress` (the fraction of the cycle done) and no `made`: the ticks left follow from the recipe and the speed.
    fn migrate(&self, from: u32, state: Value) -> Result<Value, String> {
        if from != 1 {
            return Err(format!("no migration from version {from}"));
        }
        let Some(units) = state.as_object() else { return Err("version 1 machines are an object".into()) };
        let mut out = serde_json::Map::new();
        for (iid, u) in units {
            let kind = u.get("kind").and_then(Value::as_str).ok_or_else(|| format!("{iid}: no `kind`"))?;
            let recipe = u.get("recipe").and_then(Value::as_str).ok_or_else(|| format!("{iid}: no `recipe`"))?;
            let progress = u.get("progress").and_then(Value::as_f64).ok_or_else(|| format!("{iid}: no `progress`"))?;
            if !(0.0..=1.0).contains(&progress) {
                return Err(format!("{iid}: `progress` {progress} is not between 0 and 1"));
            }
            let probe = Unit {
                kind: Id::parse(kind)?,
                recipe: Id::parse(recipe)?,
                from: Id::parse(u.get("from").and_then(Value::as_str).unwrap_or(""))?,
                to: Id::parse(u.get("to").and_then(Value::as_str).unwrap_or(""))?,
                left: None,
                made: 0,
            };
            let total = self.cycle_ticks(&probe);
            let left = (u.get("working").and_then(Value::as_bool) == Some(true)).then(|| ((1.0 - progress) * total as f64).round().max(1.0) as u32);
            out.insert(iid.clone(), json!(Unit { left, ..probe }));
        }
        Ok(Value::Object(out))
    }

    fn hash(&self) -> u64 {
        let mut h = Fnv::new();
        for (i, u) in &self.units {
            h.str(i.as_str());
            h.str(u.recipe.as_str());
            h.str(u.to.as_str());
            h.u64(u.left.map_or(u64::MAX, u64::from));
            h.u64(u.made);
        }
        h.0
    }

    fn inspect(&self, target: &Id, look: &Look) -> Option<Inspection> {
        let u = self.units.get(target)?;
        let blocked = self.blocked(target, look.ledger);
        let status = match (&blocked, u.left) {
            (Some(_), _) => "blocked",
            (None, Some(_)) => "working",
            (None, None) => "idle",
        };
        let total = self.cycle_ticks(u);
        let switch = self
            .cat
            .switches
            .iter()
            .find(|(_, s)| s.instance == *target)
            .map(|(sid, s)| json!({"switch": sid, "var": s.var, "provenance": self.cat.switches.provenance(sid)}));
        let store = |s: &Id| look.ledger.store(s).map(|s| json!(s.to_store().items)).unwrap_or(Value::Null);
        Some(Inspection {
            id: *target,
            system: system_id(),
            instance_at: self.cat.instances.provenance(target).cloned(),
            definition: Some(u.kind),
            defined_at: self.cat.machines.provenance(&u.kind).cloned(),
            status: status.into(),
            blocked_by: blocked,
            state: json!({
                "recipe": u.recipe,
                "recipe_defined_at": self.cat.recipes.provenance(&u.recipe),
                "instance_defined_at": self.cat.instances.provenance(target),
                "cycle_ticks": total,
                "left": u.left,
                "made": u.made,
                "from": u.from,
                "to": u.to,
                "routed_by": switch,
                "input": store(&Catalog::input_store(target)),
                "output": store(&Catalog::output_store(target)),
            }),
        })
    }
}
