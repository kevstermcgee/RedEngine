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
//! - **answers queries**: a system declares typed questions about its own state ([`System::queries`]) and another reads them while it runs
//!   ([`Cx::query`], each read declared in [`System::reads`]); a read is pure, so a condition never changes what it looks at. `scene:<var>`
//!   reads are the match's scene variables, answered by the [`Outside`] the game passes ([`Systems::tick_with`]);
//! - **applies batches**: commands one system issues to others ([`Cx::issue`]) apply after its call, in order, **whole or not at all**
//!   (every system a batch touches, the ledger, the random stream and the events it emitted are put back on the first refusal), and the
//!   issuer is told which ([`System::settled`]), so a decision with several effects never half-happens;
//! - **saves** each system's state with its own version, and on load migrates older state step by step ([`System::migrate`]), refuses newer
//!   or unknown state with [`code::STATE_VERSION`], and on any failure puts everything back as it was;
//! - **inspects** an instance: its status, the definition (and the file and pointer) it came from, and the first reason it cannot continue
//!   with the input that would let it ([`Inspection`]).
//!
//! Pure data and deterministic: one seeded random stream ([`Cx::random`]), id-ordered maps, no clocks. Used by Workshop (`games/workshop`)
//! and the ability fixture (`tests/systems_abilities.rs`).

use super::defs::{code, Diagnostic, Id, Provenance};
use super::ledger::{Fnv, Ledger, LedgerState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The host's own save format version (`"systems_save"`).
pub const SAVE_FORMAT: u64 = 1;

/// Events one tick may deliver by default.
pub const DEFAULT_BUDGET: usize = 4096;

/// Events kept for inspection.
const RECENT: usize = 32;

/// The namespace of the match's own answers (`scene:<var>`, the scene's declared variables), read through an [`Outside`].
pub const OUTSIDE_NS: &str = "scene";

/// How a query's answer reads (every answer is an `f64`, as the rules' expressions read numbers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ty {
    /// `1` or `0`.
    Bool,
    /// A whole number.
    Int,
    /// Any number.
    Num,
}

/// A typed question a system answers about its own state (`tw:shed_fuel`, an int): what another system's condition may read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryDecl {
    /// Its id (in the answering system's namespace).
    pub id: Id,
    /// How its answer reads.
    pub ty: Ty,
    /// What it means, for discovery and the story toolchain's bindings.
    pub doc: String,
}

impl QueryDecl {
    /// A query declaration.
    pub fn new(id: Id, ty: Ty, doc: &str) -> QueryDecl {
        QueryDecl { id, ty, doc: doc.to_string() }
    }
}

/// What answers queries from outside the systems: the match (`scene:<var>`; `RulesEngine` answers its declared variables).
pub trait Outside {
    /// The answer to `query`, or `None` when it has none by that name.
    fn query(&self, query: &Id) -> Option<f64>;
}

impl Outside for () {
    fn query(&self, _query: &Id) -> Option<f64> {
        None
    }
}

impl Outside for super::rules_run::RulesEngine {
    fn query(&self, query: &Id) -> Option<f64> {
        (query.namespace() == OUTSIDE_NS).then(|| self.var(query.name())).flatten()
    }
}

/// Commands one system issued at once ([`Cx::issue`]), applied whole or not at all.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Batch {
    /// The issuer's index in run order.
    from: usize,
    /// The issuer's own tag, handed back with the result.
    tag: u64,
    commands: Vec<Command>,
}

/// No commands may be issued (while a batch applies, or while its issuer is told the result).
static NO_ISSUES: BTreeSet<Id> = BTreeSet::new();

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

    /// The typed questions it answers about its own state ([`System::query`]), for other systems' conditions.
    fn queries(&self) -> Vec<QueryDecl> {
        Vec::new()
    }

    /// The answer to one of its [`queries`](System::queries) (`None` for one it does not answer). Pure: it changes nothing.
    fn query(&self, _query: &Id, _look: &Look) -> Option<f64> {
        None
    }

    /// The queries it reads ([`Cx::query`]): each another system's, or the match's (`scene:<var>`).
    fn reads(&self) -> Vec<Id> {
        Vec::new()
    }

    /// The commands of other systems it issues ([`Cx::issue`]).
    fn issues(&self) -> Vec<Id> {
        Vec::new()
    }

    /// The commands among its own that another system may issue unattended (a story director's pacing), not only on a player's decision.
    fn unattended(&self) -> Vec<Id> {
        Vec::new()
    }

    /// A batch it issued ([`Cx::issue`]) applied whole (`Ok`) or not at all (`Err`: the first refusal): commit, or drop, what depended on it.
    fn settled(&mut self, _tag: u64, _result: Result<(), Diagnostic>, _cx: &mut Cx) {}

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
    reads: &'a BTreeSet<Id>,
    issues: &'a BTreeSet<Id>,
    queue: &'a mut VecDeque<Event>,
    rng: &'a mut u64,
    /// The other systems (before and after this one in run order), read-only.
    peers: [&'a [Box<dyn System>]; 2],
    at: usize,
    answers: &'a BTreeMap<Id, (usize, Ty)>,
    outside: &'a dyn Outside,
    batches: &'a mut Vec<Batch>,
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

    /// The answer to `query` now: another system's ([`System::queries`]) or the match's (`scene:<var>`). It must be one of this system's
    /// [`reads`](System::reads); reading changes nothing.
    pub fn query(&self, query: &Id) -> Result<f64, Diagnostic> {
        let undeclared = |why: String| Diagnostic::new(code::QUERY, self.me, why);
        if !self.reads.contains(query) {
            return Err(undeclared(format!("reads `{query}`, which it does not declare")).fix("add it to the system's `reads`"));
        }
        if query.namespace() == OUTSIDE_NS {
            return self
                .outside
                .query(query)
                .ok_or_else(|| undeclared(format!("reads `{query}`, which the match does not answer here")).fix("declare the variable in the scene's `vars`"));
        }
        let Some(&(j, _)) = self.answers.get(query) else { return Err(undeclared(format!("reads `{query}`, which no system answers"))) };
        let peer = match j.cmp(&self.at) {
            std::cmp::Ordering::Less => &self.peers[0][j],
            std::cmp::Ordering::Greater => &self.peers[1][j - self.at - 1],
            std::cmp::Ordering::Equal => return Err(undeclared(format!("reads its own `{query}`: read its own state instead"))),
        };
        peer.query(query, &Look { tick: self.tick, ledger: self.ledger })
            .ok_or_else(|| undeclared(format!("`{query}` declared but not answered by `{}`", peer.id())))
    }

    /// Issues `commands` to the systems that take them, as one batch: after this system's call returns they apply in order, whole or not at
    /// all, and [`System::settled`] hears which with `tag`. Each must be one of this system's [`issues`](System::issues); none may be
    /// issued while a batch applies or while its result is told.
    pub fn issue(&mut self, tag: u64, commands: Vec<Command>) -> Result<(), Diagnostic> {
        if let Some(c) = commands.iter().find(|c| !self.issues.contains(&c.name)) {
            let why = if self.issues.is_empty() { "now (inside a batch, or while one settles)".to_string() } else { "and does not declare it".to_string() };
            return Err(Diagnostic::new(code::EVENT, self.me, format!("issues `{}` {why}", c.name)).fix("add it to the system's `issues`"));
        }
        self.batches.push(Batch { from: self.at, tag, commands });
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
    /// The queries it answers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<QueryDecl>,
    /// The queries it reads.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<Id>,
    /// The other systems' commands it issues.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<Id>,
    /// Its commands another system may issue unattended.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unattended: Vec<Id>,
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
    reads: BTreeSet<Id>,
    issues: BTreeSet<Id>,
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
    /// Each query's answering system (index in run order) and type.
    answers: BTreeMap<Id, (usize, Ty)>,
    batches: Vec<Batch>,
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
        // Queries: one answering system each; every read answered (or the match's); every issued command another system's.
        let mut answers: BTreeMap<Id, (Id, Ty)> = BTreeMap::new();
        for (id, s) in &by_id {
            for q in s.queries() {
                if let Some((other, _)) = answers.get(&q.id) {
                    errs.push(Diagnostic::new(code::DUPLICATE, &q.id, format!("answered by both `{other}` and `{id}`")).fix("declare the query once"));
                } else {
                    answers.insert(q.id, (id.clone(), q.ty));
                }
            }
        }
        let taken: BTreeMap<Id, Id> = by_id.iter().flat_map(|(id, s)| s.commands().into_iter().map(move |c| (c, id.clone()))).collect();
        for (id, s) in &by_id {
            for r in s.reads().iter().filter(|r| r.namespace() != OUTSIDE_NS && !answers.contains_key(*r)) {
                errs.push(
                    Diagnostic::new(code::QUERY, id, format!("reads `{r}`, which no system answers")).fix("add the system that declares it in `queries`"),
                );
            }
            for c in s.issues() {
                match taken.get(&c) {
                    Some(owner) if owner != id => {}
                    _ => errs.push(Diagnostic::new(code::EVENT, id, format!("issues `{c}`, which no other system takes")).fix("add the system that takes it")),
                }
            }
            let own: BTreeSet<Id> = s.commands().into_iter().collect();
            for u in s.unattended().iter().filter(|u| !own.contains(*u)) {
                errs.push(Diagnostic::new(code::EVENT, id, format!("lets `{u}` be issued unattended, but does not take it")).fix("add it to `commands`"));
            }
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        let list: Vec<Box<dyn System>> = order.iter().filter_map(|id| by_id.remove(id)).collect();
        let meta: Vec<Meta> = list
            .iter()
            .map(|s| Meta {
                id: s.id(),
                emits: s.emits().into_iter().collect(),
                hears: s.hears().into_iter().collect(),
                commands: s.commands().into_iter().collect(),
                reads: s.reads().into_iter().collect(),
                issues: s.issues().into_iter().collect(),
            })
            .collect();
        let answers = answers.into_iter().filter_map(|(q, (owner, ty))| Some((q, (meta.iter().position(|m| m.id == owner)?, ty)))).collect();
        Ok(Systems {
            list,
            meta,
            ledger,
            tick: 0,
            rng: seed,
            budget: DEFAULT_BUDGET,
            halted: None,
            queue: VecDeque::new(),
            recent: VecDeque::new(),
            answers,
            batches: Vec::new(),
        })
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
                queries: s.queries(),
                reads: s.reads(),
                issues: s.issues(),
                unattended: s.unattended(),
            })
            .collect()
    }

    /// Every query the systems answer, with its type, in id order.
    pub fn queries(&self) -> Vec<QueryDecl> {
        let mut all: Vec<QueryDecl> = self.list.iter().flat_map(|s| s.queries()).collect();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        all
    }

    /// The answer to `query` now (a system's, or `outside`'s for `scene:<var>`), as a tool or a test asks it: `None` when nothing answers.
    pub fn query(&self, query: &Id, outside: &dyn Outside) -> Option<f64> {
        if query.namespace() == OUTSIDE_NS {
            return outside.query(query);
        }
        let &(j, _) = self.answers.get(query)?;
        self.list[j].query(query, &Look { tick: self.tick, ledger: &self.ledger })
    }

    /// System `i` and what it sees; `settling`: it may issue nothing (a batch is applying, or its result is being told).
    fn cx<'a>(&'a mut self, i: usize, outside: &'a dyn Outside, settling: bool) -> (&'a mut Box<dyn System>, Cx<'a>) {
        let Systems { list, meta, ledger, tick, rng, queue, answers, batches, .. } = self;
        let (before, rest) = list.split_at_mut(i);
        let (me, after) = rest.split_at_mut(1);
        let m = &meta[i];
        let cx = Cx {
            tick: *tick,
            ledger,
            me: &m.id,
            emits: &m.emits,
            reads: &m.reads,
            issues: if settling { &NO_ISSUES } else { &m.issues },
            queue,
            rng,
            peers: [before, after],
            at: i,
            answers,
            outside,
            batches,
        };
        (&mut me[0], cx)
    }

    /// Applies the batches issued so far, each whole or not at all, and tells each issuer which.
    fn settle(&mut self, outside: &dyn Outside) {
        for b in std::mem::take(&mut self.batches) {
            let result = self.apply(&b, outside);
            let (s, mut cx) = self.cx(b.from, outside, true);
            s.settled(b.tag, result, &mut cx);
        }
        self.batches.clear();
    }

    /// One batch: every command in order; on the first refusal every system it touched, the ledger, the random stream and the events it
    /// emitted are put back.
    fn apply(&mut self, b: &Batch, outside: &dyn Outside) -> Result<(), Diagnostic> {
        let mut targets = Vec::new();
        for c in &b.commands {
            let t = self.meta.iter().position(|m| m.commands.contains(&c.name));
            targets.push(t.ok_or_else(|| Diagnostic::new(code::EVENT, &c.name, "no system takes this command"))?);
        }
        let saved: BTreeMap<usize, Value> = targets.iter().map(|&t| (t, self.list[t].save())).collect();
        let (ledger, rng, queued) = (self.ledger.clone(), self.rng, self.queue.len());
        for (k, (c, &t)) in b.commands.iter().zip(&targets).enumerate() {
            let (s, mut cx) = self.cx(t, outside, true);
            if let Err(e) = s.command(c, &mut cx) {
                for (t, state) in saved {
                    let _ = self.list[t].load(state);
                }
                (self.ledger, self.rng) = (ledger, rng);
                self.queue.truncate(queued);
                let message = format!("command {} of {} (`{}`) refused, so none applied: {}", k + 1, b.commands.len(), c.name, e.message);
                return Err(Diagnostic { message, ..e });
            }
        }
        Ok(())
    }

    /// Runs one tick: every system in order, then the events they emitted (and the ones those emit) to the systems that hear them.
    pub fn tick(&mut self) -> Result<(), Diagnostic> {
        self.tick_with(&())
    }

    /// [`tick`](Systems::tick) with the match answering `scene:<var>` reads.
    pub fn tick_with(&mut self, outside: &dyn Outside) -> Result<(), Diagnostic> {
        if let Some(h) = &self.halted {
            return Err(h.clone());
        }
        self.tick += 1;
        for i in 0..self.list.len() {
            let (s, mut cx) = self.cx(i, outside, false);
            s.tick(&mut cx);
            self.settle(outside);
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
                    let (s, mut cx) = self.cx(i, outside, false);
                    s.event(&ev, &mut cx);
                    self.settle(outside);
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
        self.command_with(command, &())
    }

    /// [`command`](Systems::command) with the match answering `scene:<var>` reads. The batches the command issues apply after it (a
    /// refused command issues none).
    pub fn command_with(&mut self, command: &Command, outside: &dyn Outside) -> Result<(), Diagnostic> {
        let Some(i) = self.meta.iter().position(|m| m.commands.contains(&command.name)) else {
            return Err(Diagnostic::new(code::EVENT, &command.name, "no system takes this command")
                .fix(format!("one of: {}", self.meta.iter().flat_map(|m| m.commands.iter().map(Id::as_str)).collect::<Vec<_>>().join(", "))));
        };
        let (s, mut cx) = self.cx(i, outside, false);
        let r = s.command(command, &mut cx);
        if r.is_err() {
            self.batches.clear();
            return r;
        }
        self.settle(outside);
        r
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
mod tests;
