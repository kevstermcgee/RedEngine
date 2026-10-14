//! Game systems: reusable, game-owned behaviors composed under the match's one authority (ADR
//! 2026-10-14-game-systems-compose-under-one-match-authority).
//!
//! A [`System`] is a game crate's code: one behavior (machines, storage routing, abilities) with its own per-instance state. A game builds a
//! [`Systems`] host from several of them and one [`Ledger`], and runs it from its `MatchGame` (the match's one authority: there is never a
//! second simulation). The host:
//! - **orders** the systems by their declared dependencies ([`System::after`]), ties broken by id, so the order never depends on the order
//!   they were registered; an undeclared dependency, a duplicate id or a cycle is refused when the host is built, with a stable code;
//! - **delivers events** ([`Cx::emit`]) after the systems' ticks, in order, to the systems that hear them, at most [`Systems::budget`] per
//!   tick: a cascade that does not settle halts the host with [`code::OVERFLOW`] (reported every tick after) instead of dropping work;
//!   an event a system emits without declaring it, or hears that nobody emits, is [`code::EVENT`];
//! - **routes commands** (a player's decision) to the system that declares them, checked before anything changes;
//! - **saves** each system's state with its own version, and on load migrates older state step by step ([`System::migrate`]), refuses newer
//!   or unknown state with [`code::STATE_VERSION`], and on any failure puts everything back as it was;
//! - **inspects** an instance: its status, the definition (and the file and pointer) it came from, and the first reason it cannot continue
//!   with the input that would let it ([`Inspection`]).
//!
//! Pure data and deterministic: one seeded random stream ([`Cx::random`]), id-ordered maps, no clocks. Used by Workshop (`games/workshop`)
//! and the ability fixture (`tests/systems_abilities.rs`).

use super::defs::{code, Diagnostic, Id, Provenance};
use super::ledger::{Fnv, Ledger, LedgerState};
use serde::Serialize;
use serde_json::{json, Value};
use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The host's own save format version (`"systems_save"`).
pub const SAVE_FORMAT: u64 = 1;

/// Events one tick may deliver by default.
pub const DEFAULT_BUDGET: usize = 4096;

/// Events kept for inspection.
const RECENT: usize = 32;

/// Something that happened, for the systems that hear it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Event {
    /// What (`ws:crafted`).
    pub name: Id,
    /// To what or whom (an instance), if anything.
    pub subject: Option<Id>,
    /// A number that goes with it.
    pub value: i64,
    /// The system that emitted it.
    pub from: Id,
    /// The tick.
    pub tick: u64,
}

/// A decision from outside the systems (a player's input, a rule): delivered to the system that declares `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// What (`ws:route`).
    pub name: Id,
    /// To which instance, if one.
    pub subject: Option<Id>,
    /// A number that goes with it.
    pub value: i64,
}

/// A reusable behavior a game composes ([`Systems`]). Everything but [`id`](System::id) and [`tick`](System::tick) has a default.
pub trait System: Any + Send {
    /// Its id (`ws:machines`).
    fn id(&self) -> Id;

    /// The systems it runs after (it reads what they leave). Each must be in the host.
    fn after(&self) -> Vec<Id> {
        Vec::new()
    }

    /// The events it may emit.
    fn emits(&self) -> Vec<Id> {
        Vec::new()
    }

    /// The events it hears ([`System::event`]).
    fn hears(&self) -> Vec<Id> {
        Vec::new()
    }

    /// The commands it takes ([`System::command`]).
    fn commands(&self) -> Vec<Id> {
        Vec::new()
    }

    /// The live instances it owns (`ws:furnace_a`), for inspection and discovery.
    fn instances(&self) -> Vec<Id> {
        Vec::new()
    }

    /// One tick of its work.
    fn tick(&mut self, cx: &mut Cx);

    /// An event it hears.
    fn event(&mut self, _event: &Event, _cx: &mut Cx) {}

    /// A command it declares; an error refuses it and changes nothing.
    fn command(&mut self, _command: &Command, _cx: &mut Cx) -> Result<(), Diagnostic> {
        Ok(())
    }

    /// The version of the state [`save`](System::save) writes.
    fn version(&self) -> u32 {
        1
    }

    /// Its state, to save.
    fn save(&self) -> Value {
        Value::Null
    }

    /// Takes saved state of the current [`version`](System::version).
    fn load(&mut self, _state: Value) -> Result<(), String> {
        Ok(())
    }

    /// State saved at version `from`, rewritten as version `from + 1`.
    fn migrate(&self, from: u32, _state: Value) -> Result<Value, String> {
        Err(format!("no migration from version {from}"))
    }

    /// A hash of its state (the match checksum folds it in).
    fn hash(&self) -> u64 {
        0
    }

    /// One of its instances explained, if `target` is one.
    fn inspect(&self, _target: &Id, _look: &Look) -> Option<Inspection> {
        None
    }
}

/// What a system sees and may change while it runs.
pub struct Cx<'a> {
    /// The tick.
    pub tick: u64,
    /// The resources.
    pub ledger: &'a mut Ledger,
    me: &'a Id,
    emits: &'a BTreeSet<Id>,
    queue: &'a mut VecDeque<Event>,
    rng: &'a mut u64,
}

impl Cx<'_> {
    /// Emits an event for the systems that hear it (after every system has ticked). One this system does not declare is refused.
    pub fn emit(&mut self, name: &Id, subject: Option<&Id>, value: i64) -> Result<(), Diagnostic> {
        if !self.emits.contains(name) {
            return Err(Diagnostic::new(code::EVENT, self.me, format!("emits `{name}`, which it does not declare")).fix("add it to the system's `emits`"));
        }
        self.queue.push_back(Event { name: name.clone(), subject: subject.cloned(), value, from: self.me.clone(), tick: self.tick });
        Ok(())
    }

    /// A random number below `n` (`0` for `n == 0`) from the host's one seeded stream (saved with it).
    pub fn random(&mut self, n: u32) -> u32 {
        // splitmix64
        *self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = *self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        if n == 0 {
            0
        } else {
            (z % n as u64) as u32
        }
    }
}

/// What an inspection may read.
pub struct Look<'a> {
    /// The tick.
    pub tick: u64,
    /// The resources.
    pub ledger: &'a Ledger,
}

/// One instance explained: what it is, where it was defined, what it is doing and, when it cannot continue, why and what would let it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Inspection {
    /// The instance.
    pub id: Id,
    /// The system that owns it.
    pub system: Id,
    /// Where the instance itself was written (the entry that put it in the game), when it is content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_at: Option<Provenance>,
    /// The definition it instantiates.
    pub definition: Option<Id>,
    /// Where that definition was written.
    pub defined_at: Option<Provenance>,
    /// `working`, `blocked`, `idle`, ...
    pub status: String,
    /// When it cannot continue: the first reason, with the input that would let it (`fix`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<Diagnostic>,
    /// Its state.
    pub state: Value,
}

/// One system as discovery shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SystemInfo {
    /// Its id.
    pub id: Id,
    /// What it runs after.
    pub after: Vec<Id>,
    /// What it emits.
    pub emits: Vec<Id>,
    /// What it hears.
    pub hears: Vec<Id>,
    /// What commands it takes.
    pub commands: Vec<Id>,
    /// Its live instances.
    pub instances: Vec<Id>,
    /// Its state version.
    pub version: u32,
}

/// What [`Systems::load`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadReport {
    /// `(system, saved version, current version)` for each system whose state was migrated.
    pub migrated: Vec<(Id, u32, u32)>,
    /// Systems the save had nothing for (they start fresh).
    pub fresh: Vec<Id>,
}

struct Meta {
    id: Id,
    emits: BTreeSet<Id>,
    hears: BTreeSet<Id>,
    commands: BTreeSet<Id>,
}

/// Game systems and their ledger, run as one.
pub struct Systems {
    list: Vec<Box<dyn System>>,
    meta: Vec<Meta>,
    ledger: Ledger,
    tick: u64,
    rng: u64,
    budget: usize,
    halted: Option<Diagnostic>,
    queue: VecDeque<Event>,
    recent: VecDeque<Event>,
}

impl std::fmt::Debug for Systems {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Systems").field("order", &self.order()).field("tick", &self.tick).finish_non_exhaustive()
    }
}

impl Systems {
    /// Checks and orders `systems` (see the module docs) around `ledger`, the random stream seeded with `seed`. Every problem is reported.
    pub fn new(systems: Vec<Box<dyn System>>, ledger: Ledger, seed: u64) -> Result<Systems, Vec<Diagnostic>> {
        let mut errs = Vec::new();
        let mut by_id: BTreeMap<Id, Box<dyn System>> = BTreeMap::new();
        for s in systems {
            let id = s.id();
            match by_id.entry(id) {
                std::collections::btree_map::Entry::Occupied(e) => {
                    errs.push(Diagnostic::new(code::DUPLICATE, e.key(), "two systems have this id").fix("register it once"))
                }
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(s);
                }
            }
        }
        let after: BTreeMap<Id, Vec<Id>> = by_id.iter().map(|(id, s)| (id.clone(), s.after())).collect();
        for (id, deps) in &after {
            for d in deps.iter().filter(|d| !by_id.contains_key(*d)) {
                errs.push(
                    Diagnostic::new(code::DEPENDENCY, id, format!("runs after `{d}`, which is not in this game"))
                        .fix(format!("add the system `{d}`, or drop it from `after`")),
                );
            }
        }
        let emitted: BTreeSet<Id> = by_id.values().flat_map(|s| s.emits()).collect();
        for (id, s) in &by_id {
            for h in s.hears().iter().filter(|h| !emitted.contains(*h)) {
                errs.push(
                    Diagnostic::new(code::EVENT, id, format!("hears `{h}`, which no system emits")).fix("add a system that emits it, or drop it from `hears`"),
                );
            }
        }
        // Kahn's order, the ready set taken in id order: the same order whatever order the systems were given in.
        let mut order = Vec::new();
        let mut placed: BTreeSet<Id> = BTreeSet::new();
        while order.len() < by_id.len() {
            let ready = after.iter().find(|(id, deps)| !placed.contains(*id) && deps.iter().all(|d| placed.contains(d) || !by_id.contains_key(d)));
            match ready {
                Some((id, _)) => {
                    placed.insert(id.clone());
                    order.push(id.clone());
                }
                None => {
                    let stuck: Vec<&str> = by_id.keys().filter(|i| !placed.contains(*i)).map(Id::as_str).collect();
                    errs.push(
                        Diagnostic::new(code::CYCLE, stuck[0], format!("these systems each run after another of them: {}", stuck.join(", ")))
                            .fix("drop one `after`"),
                    );
                    break;
                }
            }
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        let list: Vec<Box<dyn System>> = order.iter().filter_map(|id| by_id.remove(id)).collect();
        let meta = list
            .iter()
            .map(|s| Meta {
                id: s.id(),
                emits: s.emits().into_iter().collect(),
                hears: s.hears().into_iter().collect(),
                commands: s.commands().into_iter().collect(),
            })
            .collect();
        Ok(Systems { list, meta, ledger, tick: 0, rng: seed, budget: DEFAULT_BUDGET, halted: None, queue: VecDeque::new(), recent: VecDeque::new() })
    }

    /// The most events one tick may deliver (a cascade beyond it halts the host).
    pub fn with_budget(mut self, events_per_tick: usize) -> Systems {
        self.budget = events_per_tick;
        self
    }

    /// The bound on events per tick.
    pub fn budget(&self) -> usize {
        self.budget
    }

    /// The order the systems run in.
    pub fn order(&self) -> Vec<Id> {
        self.meta.iter().map(|m| m.id.clone()).collect()
    }

    /// Ticks run.
    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    /// The resources.
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Why the host stopped, if it did (it then refuses to tick: authority-critical work is never silently dropped).
    pub fn halted(&self) -> Option<&Diagnostic> {
        self.halted.as_ref()
    }

    /// The last few events delivered, oldest first.
    pub fn recent_events(&self) -> impl Iterator<Item = &Event> {
        self.recent.iter()
    }

    /// The system of type `T`.
    pub fn get<T: System>(&self) -> Option<&T> {
        self.list.iter().find_map(|s| (s.as_ref() as &dyn Any).downcast_ref::<T>())
    }

    /// Every system as discovery shows it, in run order.
    pub fn catalog(&self) -> Vec<SystemInfo> {
        self.list
            .iter()
            .map(|s| SystemInfo {
                id: s.id(),
                after: s.after(),
                emits: s.emits(),
                hears: s.hears(),
                commands: s.commands(),
                instances: s.instances(),
                version: s.version(),
            })
            .collect()
    }

    fn cx<'a>(&'a mut self, i: usize) -> (&'a mut Box<dyn System>, Cx<'a>) {
        let Systems { list, meta, ledger, tick, rng, queue, .. } = self;
        (&mut list[i], Cx { tick: *tick, ledger, me: &meta[i].id, emits: &meta[i].emits, queue, rng })
    }

    /// Runs one tick: every system in order, then the events they emitted (and the ones those emit) to the systems that hear them.
    pub fn tick(&mut self) -> Result<(), Diagnostic> {
        if let Some(h) = &self.halted {
            return Err(h.clone());
        }
        self.tick += 1;
        for i in 0..self.list.len() {
            let (s, mut cx) = self.cx(i);
            s.tick(&mut cx);
        }
        let mut delivered = 0usize;
        while let Some(ev) = self.queue.pop_front() {
            delivered += 1;
            if delivered > self.budget {
                let d = Diagnostic::new(
                    code::OVERFLOW,
                    &ev.from,
                    format!("more than {} events in tick {} (the last: `{}`): a cascade that does not settle", self.budget, self.tick, ev.name),
                )
                .fix("make the event chain end (an event that causes itself), or raise the budget if the work is real");
                self.queue.clear();
                self.halted = Some(d.clone());
                return Err(d);
            }
            for i in 0..self.list.len() {
                if self.meta[i].hears.contains(&ev.name) {
                    let (s, mut cx) = self.cx(i);
                    s.event(&ev, &mut cx);
                }
            }
            if self.recent.len() == RECENT {
                self.recent.pop_front();
            }
            self.recent.push_back(ev);
        }
        Ok(())
    }

    /// Delivers `command` to the system that declares it, now (between ticks, in the order commands arrive). A command no system declares
    /// is [`code::EVENT`]; one the system refuses changes nothing.
    pub fn command(&mut self, command: &Command) -> Result<(), Diagnostic> {
        let Some(i) = self.meta.iter().position(|m| m.commands.contains(&command.name)) else {
            return Err(Diagnostic::new(code::EVENT, &command.name, "no system takes this command")
                .fix(format!("one of: {}", self.meta.iter().flat_map(|m| m.commands.iter().map(Id::as_str)).collect::<Vec<_>>().join(", "))));
        };
        let (s, mut cx) = self.cx(i);
        s.command(command, &mut cx)
    }

    /// Explains `target`: an instance some system owns, or a store.
    pub fn inspect(&self, target: &str) -> Option<Inspection> {
        let id = Id::parse(target).ok()?;
        let look = Look { tick: self.tick, ledger: &self.ledger };
        if let Some(found) = self.list.iter().find_map(|s| s.inspect(&id, &look)) {
            return Some(found);
        }
        let store = self.ledger.store(&id)?;
        let status = if store.free() == 0 {
            "full"
        } else if store.used() == 0 {
            "empty"
        } else {
            "holding"
        };
        Some(Inspection {
            id,
            system: store.owner.clone(),
            instance_at: None,
            definition: None,
            defined_at: None,
            status: status.into(),
            blocked_by: None,
            state: json!({"owner": store.owner, "capacity": store.capacity, "items": store.items}),
        })
    }

    /// A hash of everything: the tick, the random stream, the ledger and each system's state.
    pub fn hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.tick);
        h.u64(self.rng);
        h.u64(self.ledger.hash());
        for s in &self.list {
            h.str(s.id().as_str());
            h.u64(s.hash());
        }
        h.0
    }

    /// Everything to save: `{"systems_save": 1, "tick", "rng", "ledger", "systems": {id: {"version", "state"}}}`.
    pub fn save(&self) -> Value {
        let systems: serde_json::Map<String, Value> =
            self.list.iter().map(|s| (s.id().to_string(), json!({"version": s.version(), "state": s.save()}))).collect();
        json!({"systems_save": SAVE_FORMAT, "tick": self.tick, "rng": self.rng, "ledger": self.ledger.state(), "systems": systems})
    }

    /// Loads [`save`](Systems::save)d state, migrating each system's older state to its current version. Newer or unknown state is
    /// [`code::STATE_VERSION`]; on any failure every system, the ledger and the clock are as they were.
    pub fn load(&mut self, saved: &Value) -> Result<LoadReport, Diagnostic> {
        let bad = |m: String| Diagnostic::new(code::STATE_VERSION, "save", m);
        match saved.get("systems_save").and_then(Value::as_u64) {
            Some(SAVE_FORMAT) => {}
            Some(v) => return Err(bad(format!("save format {v}; this program reads format {SAVE_FORMAT}"))),
            None => return Err(bad("not a game-systems save (no `systems_save`)".into())),
        }
        let tick = saved.get("tick").and_then(Value::as_u64).ok_or_else(|| bad("no `tick`".into()))?;
        let rng = saved.get("rng").and_then(Value::as_u64).ok_or_else(|| bad("no `rng`".into()))?;
        let ledger: LedgerState = serde_json::from_value(saved.get("ledger").cloned().unwrap_or(Value::Null)).map_err(|e| bad(format!("ledger: {e}")))?;
        let empty = serde_json::Map::new();
        let states = saved.get("systems").and_then(Value::as_object).unwrap_or(&empty);
        let mine: BTreeSet<String> = self.list.iter().map(|s| s.id().to_string()).collect();
        if let Some(other) = states.keys().find(|k| !mine.contains(*k)) {
            return Err(bad(format!("the save has state for `{other}`, which this game does not run")));
        }
        // Every system's state at its current version, before anything changes.
        let mut report = LoadReport { migrated: Vec::new(), fresh: Vec::new() };
        let mut staged: Vec<Option<Value>> = Vec::new();
        for s in &self.list {
            let Some(entry) = states.get(s.id().as_str()) else {
                report.fresh.push(s.id());
                staged.push(None);
                continue;
            };
            let at = format!("save#/systems/{}", s.id());
            let version = entry.get("version").and_then(Value::as_u64).ok_or_else(|| Diagnostic::new(code::STATE_VERSION, &at, "no `version`"))? as u32;
            let mut state = entry.get("state").cloned().unwrap_or(Value::Null);
            let current = s.version();
            if version > current {
                return Err(Diagnostic::new(code::STATE_VERSION, &at, format!("saved by a newer version ({version}); this program reads up to {current}"))
                    .fix("open it with the version that saved it, or a newer one"));
            }
            for v in version..current {
                state = s.migrate(v, state).map_err(|e| Diagnostic::new(code::STATE_VERSION, &at, format!("cannot migrate version {v} to {}: {e}", v + 1)))?;
            }
            if version < current {
                report.migrated.push((s.id(), version, current));
            }
            staged.push(Some(state));
        }
        let before = self.save();
        let mut ledger_next = self.ledger.clone();
        ledger_next.restore(ledger)?;
        for (i, state) in staged.into_iter().enumerate() {
            if let Some(state) = state {
                if let Err(e) = self.list[i].load(state) {
                    self.rollback(&before);
                    return Err(Diagnostic::new(code::STATE_VERSION, format!("save#/systems/{}", self.meta[i].id), e));
                }
            }
        }
        self.ledger = ledger_next;
        self.tick = tick;
        self.rng = rng;
        self.halted = None;
        self.queue.clear();
        Ok(report)
    }

    /// Puts every system back to `before` (its own save, which it always reads).
    fn rollback(&mut self, before: &Value) {
        for s in &mut self.list {
            if let Some(state) = before.get("systems").and_then(|m| m.get(s.id().as_str())).and_then(|e| e.get("state")) {
                let _ = s.load(state.clone());
            }
        }
    }
}

/// Writes `text` to `path` so a crash or a full disk never leaves a half-written file: a temporary file beside it, flushed to disk, then
/// renamed over it. The file it replaces is kept as `<path>.bak`. On failure `path` is as it was.
pub fn write_atomic(path: &std::path::Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", tmp.display()));
    }
    if path.exists() {
        let bak = path.with_extension("bak");
        std::fs::copy(path, &bak).map_err(|e| format!("{}: {e}", bak.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::ledger::{Reason, Tx};

    fn id(s: &str) -> Id {
        Id::parse(s).unwrap()
    }

    /// A counter that emits `t:ping` every tick and hears `t:ping` (re-emitting while `echo` is on: an endless cascade).
    struct Pinger {
        me: Id,
        after: Vec<Id>,
        count: u64,
        echo: bool,
        heard: u64,
    }

    impl System for Pinger {
        fn id(&self) -> Id {
            self.me.clone()
        }
        fn after(&self) -> Vec<Id> {
            self.after.clone()
        }
        fn emits(&self) -> Vec<Id> {
            vec![id("t:ping")]
        }
        fn hears(&self) -> Vec<Id> {
            vec![id("t:ping")]
        }
        fn commands(&self) -> Vec<Id> {
            vec![id("t:echo")]
        }
        fn tick(&mut self, cx: &mut Cx) {
            self.count += 1;
            cx.emit(&id("t:ping"), None, 1).unwrap();
        }
        fn event(&mut self, ev: &Event, cx: &mut Cx) {
            self.heard += 1;
            if self.echo && ev.from == self.me {
                cx.emit(&id("t:ping"), None, ev.value + 1).unwrap();
            }
        }
        fn command(&mut self, c: &Command, _cx: &mut Cx) -> Result<(), Diagnostic> {
            self.echo = c.value != 0;
            Ok(())
        }
        fn version(&self) -> u32 {
            2
        }
        fn save(&self) -> Value {
            json!({"count": self.count, "heard": self.heard})
        }
        fn load(&mut self, s: Value) -> Result<(), String> {
            self.count = s["count"].as_u64().ok_or("no count")?;
            self.heard = s["heard"].as_u64().ok_or("no heard")?;
            Ok(())
        }
        fn migrate(&self, from: u32, s: Value) -> Result<Value, String> {
            match from {
                1 => Ok(json!({"count": s["n"], "heard": 0})),
                _ => Err(format!("unknown version {from}")),
            }
        }
        fn hash(&self) -> u64 {
            self.count ^ (self.heard << 32)
        }
    }

    fn pinger(name: &str, after: &[&str]) -> Box<dyn System> {
        Box::new(Pinger { me: id(name), after: after.iter().map(|a| id(a)).collect(), count: 0, echo: false, heard: 0 })
    }

    #[test]
    fn the_order_follows_dependencies_and_never_the_registration_order() {
        let a = Systems::new(vec![pinger("t:c", &["t:a"]), pinger("t:b", &[]), pinger("t:a", &[])], Ledger::new(), 1).unwrap();
        let b = Systems::new(vec![pinger("t:a", &[]), pinger("t:c", &["t:a"]), pinger("t:b", &[])], Ledger::new(), 1).unwrap();
        assert_eq!(a.order(), b.order());
        assert_eq!(a.order().iter().map(Id::as_str).collect::<Vec<_>>(), ["t:a", "t:b", "t:c"]);
    }

    #[test]
    fn invalid_composition_is_refused_with_stable_codes() {
        let codes = |systems: Vec<Box<dyn System>>| Systems::new(systems, Ledger::new(), 1).unwrap_err().iter().map(|d| d.code).collect::<Vec<_>>();
        assert_eq!(codes(vec![pinger("t:a", &[]), pinger("t:a", &[])]), [code::DUPLICATE]);
        assert_eq!(codes(vec![pinger("t:a", &["t:missing"])]), [code::DEPENDENCY]);
        assert_eq!(codes(vec![pinger("t:a", &["t:b"]), pinger("t:b", &["t:a"])]), [code::CYCLE]);
    }

    #[test]
    fn a_cascade_that_does_not_settle_halts_the_host_visibly() {
        let mut s = Systems::new(vec![pinger("t:a", &[])], Ledger::new(), 1).unwrap().with_budget(100);
        s.tick().unwrap();
        s.command(&Command { name: id("t:echo"), subject: None, value: 1 }).unwrap();
        let e = s.tick().unwrap_err();
        assert_eq!(e.code, code::OVERFLOW, "{e}");
        assert_eq!(s.tick().unwrap_err(), e, "it stays halted and says why every tick");
        assert_eq!(s.halted(), Some(&e));
        let e = s.command(&Command { name: id("t:nothing"), subject: None, value: 0 }).unwrap_err();
        assert_eq!(e.code, code::EVENT);
    }

    #[test]
    fn a_save_reloads_exactly_and_older_state_is_migrated() {
        let mut ledger = Ledger::new();
        ledger.resource(id("t:ore"));
        ledger.declare(Reason::Source(id("t:mine")));
        ledger.add_store(id("t:box"), id("t:a"), 5).unwrap();
        let mut s = Systems::new(vec![pinger("t:a", &[])], ledger.clone(), 7).unwrap();
        for _ in 0..3 {
            s.tick().unwrap();
        }
        let saved = s.save();
        let mut again = Systems::new(vec![pinger("t:a", &[])], ledger.clone(), 99).unwrap();
        assert_eq!(again.load(&saved).unwrap(), LoadReport { migrated: vec![], fresh: vec![] });
        assert_eq!(again.hash(), s.hash());
        // Version 1 kept the count as `n` and no `heard`.
        let mut v1 = saved.clone();
        v1["systems"]["t:a"] = json!({"version": 1, "state": {"n": 3}});
        let mut migrated = Systems::new(vec![pinger("t:a", &[])], ledger.clone(), 1).unwrap();
        assert_eq!(migrated.load(&v1).unwrap().migrated, vec![(id("t:a"), 1, 2)]);
        assert_eq!(migrated.get::<Pinger>().unwrap().count, 3);
        // A newer version, an unknown system, a broken migration: refused, nothing changed.
        let before = migrated.hash();
        for (path, value) in [("version", json!(3)), ("state", json!({"count": "x"}))] {
            let mut bad = saved.clone();
            bad["systems"]["t:a"][path] = value;
            assert_eq!(migrated.load(&bad).unwrap_err().code, code::STATE_VERSION);
            assert_eq!(migrated.hash(), before);
        }
        let mut alien = saved;
        alien["systems"]["t:z"] = json!({"version": 1, "state": null});
        assert!(migrated.load(&alien).unwrap_err().message.contains("does not run"));
        let mut v0 = v1;
        v0["systems"]["t:a"]["version"] = json!(0);
        assert!(migrated.load(&v0).unwrap_err().message.contains("cannot migrate version 0"));
        assert_eq!(migrated.hash(), before);
    }

    #[test]
    fn an_atomic_write_keeps_the_previous_file_and_never_leaves_half_a_file() {
        let dir = std::env::temp_dir().join(format!("red-systems-{}", std::process::id()));
        let path = dir.join("save.json");
        write_atomic(&path, "one").unwrap();
        write_atomic(&path, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(std::fs::read_to_string(path.with_extension("bak")).unwrap(), "one");
        assert!(!path.with_extension("tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = Tx::new();
    }
}
