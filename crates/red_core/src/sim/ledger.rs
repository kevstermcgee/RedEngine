//! Countable resources in stores, changed only by transactions that apply whole or not at all (ADR
//! 2026-10-14-game-systems-compose-under-one-match-authority).
//!
//! A store has an explicit owner (the instance or system responsible for it) and a capacity in units. A [`Tx`] is a list of [`Op`]s: a
//! move between stores conserves what it moves; resources come into being or leave only for a declared [`Reason`] (a recipe, a source or a
//! sink the content defines). [`Ledger::apply`] checks every op in order and commits all or none: a missing input is [`code::MISSING`], a
//! full destination [`code::FULL`], and nothing changes. [`Ledger::check`] is the same without committing (what an inspection asks: "would
//! this run?"). The ledger keeps, per resource, how much the declared reasons made and destroyed, so [`Ledger::audit`] proves the stores
//! hold exactly that (after a load, or in a test that tries to break it). Removing a store (a despawned machine) hands its contents to
//! another store or fails.
//!
//! **Slots, for the per-tick path.** Inside, stores and resources are dense slots, and a transaction applies in place with an undo log it
//! reuses: a system that resolves its ids once ([`Ledger::key`], [`Ledger::res`], [`Ledger::why`]) and runs [`SlotOp`]s
//! ([`Ledger::apply_ops`]) allocates nothing per tick, and a refusal is a small [`Fail`] that becomes a [`Diagnostic`] only when someone
//! reads it ([`Ledger::explain`]). A store's slot is stable for the ledger's life, through removal, re-adding and [`Ledger::restore`].
//! The id-keyed API ([`Tx`], [`Ledger::apply`]) is the same transaction, resolved first.
//!
//! Used by the game-system host (`sim::systems`) and its games (Workshop's machines and storage, the ability fixture's mana).

use super::defs::{code, Diagnostic, Id};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Why resources may appear or disappear: a definition the content declares ([`Ledger::declare`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Reason {
    /// A recipe: consumes its inputs and produces its outputs.
    Recipe(Id),
    /// A source: produces from nothing in the game (a mine, a delivery, a starting stock).
    Source(Id),
    /// A sink: consumes into nothing in the game (shipping, an ability's cost).
    Sink(Id),
}

impl Reason {
    /// Whether this kind of reason may consume (`true`) or produce (`false`).
    fn allows(&self, consume: bool) -> bool {
        matches!((self, consume), (Reason::Recipe(_), _) | (Reason::Source(_), false) | (Reason::Sink(_), true))
    }
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::Recipe(id) => write!(f, "recipe `{id}`"),
            Reason::Source(id) => write!(f, "source `{id}`"),
            Reason::Sink(id) => write!(f, "sink `{id}`"),
        }
    }
}

/// One step of a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// `n` of `res` from one store to another.
    Move {
        /// The store it leaves.
        from: Id,
        /// The store it enters.
        to: Id,
        /// The resource.
        res: Id,
        /// How many.
        n: u32,
    },
    /// `n` of `res` leaves the game from `from`, for a declared recipe or sink.
    Consume {
        /// The store.
        from: Id,
        /// The resource.
        res: Id,
        /// How many.
        n: u32,
        /// The declared reason.
        why: Reason,
    },
    /// `n` of `res` enters the game into `to`, for a declared recipe or source.
    Produce {
        /// The store.
        to: Id,
        /// The resource.
        res: Id,
        /// How many.
        n: u32,
        /// The declared reason.
        why: Reason,
    },
}

/// Ops applied in order, all or none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tx {
    /// The ops.
    pub ops: Vec<Op>,
}

impl Tx {
    /// An empty transaction.
    pub fn new() -> Tx {
        Tx::default()
    }

    /// Adds a move.
    pub fn moving(mut self, from: &Id, to: &Id, res: &Id, n: u32) -> Tx {
        self.ops.push(Op::Move { from: *from, to: *to, res: *res, n });
        self
    }

    /// Adds a consumption for `why`.
    pub fn consuming(mut self, from: &Id, res: &Id, n: u32, why: &Reason) -> Tx {
        self.ops.push(Op::Consume { from: *from, res: *res, n, why: *why });
        self
    }

    /// Adds a production for `why`.
    pub fn producing(mut self, to: &Id, res: &Id, n: u32, why: &Reason) -> Tx {
        self.ops.push(Op::Produce { to: *to, res: *res, n, why: *why });
        self
    }
}

/// A store's slot ([`Ledger::key`]): stable for the ledger's life, whether or not the store exists right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot(u32);

/// A declared resource's slot ([`Ledger::res`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Res(u32);

/// A declared reason's slot ([`Ledger::why`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Why(u32);

/// Not a slot: what an id that names nothing resolves to (the op that uses it is refused when it is reached).
const NONE: u32 = u32::MAX;

/// One step of a transaction over slots ([`Ledger::apply_ops`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotOp {
    /// `n` of `res` from one store to another.
    Move {
        /// The store it leaves.
        from: Slot,
        /// The store it enters.
        to: Slot,
        /// The resource.
        res: Res,
        /// How many.
        n: u32,
    },
    /// `n` of `res` leaves the game, for a declared recipe or sink.
    Consume {
        /// The store.
        from: Slot,
        /// The resource.
        res: Res,
        /// How many.
        n: u32,
        /// The declared reason.
        why: Why,
    },
    /// `n` of `res` enters the game, for a declared recipe or source.
    Produce {
        /// The store.
        to: Slot,
        /// The resource.
        res: Res,
        /// How many.
        n: u32,
        /// The declared reason.
        why: Why,
    },
}

/// Why a transaction was refused, without words yet ([`Ledger::explain`] writes them): which op, and what about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fail {
    /// The op (its index in the transaction).
    pub op: usize,
    /// What about it.
    pub kind: FailKind,
}

/// What a refused op ran into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// The resource is not declared.
    UnknownResource,
    /// The reason is not declared, or is of a kind that cannot do this (a sink cannot produce).
    Undeclared,
    /// The store it takes from does not exist.
    UnknownFrom,
    /// The store it puts into does not exist.
    UnknownTo,
    /// The store it takes from holds `have`, less than `need`.
    Missing {
        /// Held.
        have: u32,
        /// Needed.
        need: u32,
    },
    /// The store it puts into has room for `free`, less than `need`.
    Full {
        /// Room.
        free: u32,
        /// Needed.
        need: u32,
    },
}

impl Fail {
    /// The stable code ([`code`]).
    pub fn code(&self) -> &'static str {
        match self.kind {
            FailKind::UnknownResource | FailKind::UnknownFrom | FailKind::UnknownTo => code::UNKNOWN,
            FailKind::Undeclared => code::DEPENDENCY,
            FailKind::Missing { .. } => code::MISSING,
            FailKind::Full { .. } => code::FULL,
        }
    }
}

/// A store as saved: its owner, capacity and contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    /// Who is responsible for it (a machine instance, a shared storage's system).
    pub owner: Id,
    /// Most units it holds, all resources together.
    pub capacity: u32,
    /// What it holds (no zero entries).
    pub items: BTreeMap<Id, u32>,
}

impl Store {
    /// Units held.
    pub fn used(&self) -> u32 {
        self.items.values().sum()
    }

    /// Room left.
    pub fn free(&self) -> u32 {
        self.capacity.saturating_sub(self.used())
    }

    /// How many of `res` it holds.
    pub fn count(&self, res: &Id) -> u32 {
        self.items.get(res).copied().unwrap_or(0)
    }
}

/// A live store, read through the ledger ([`Ledger::store`]).
#[derive(Clone, Copy)]
pub struct StoreRef<'a> {
    ledger: &'a Ledger,
    cell: &'a Cell,
}

impl<'a> StoreRef<'a> {
    /// Who is responsible for it.
    pub fn owner(&self) -> Id {
        self.cell.owner
    }

    /// Most units it holds.
    pub fn capacity(&self) -> u32 {
        self.cell.capacity
    }

    /// Units held.
    pub fn used(&self) -> u32 {
        self.cell.used
    }

    /// Room left.
    pub fn free(&self) -> u32 {
        self.cell.capacity.saturating_sub(self.cell.used)
    }

    /// How many of `res` it holds.
    pub fn count(&self, res: &Id) -> u32 {
        self.ledger.res(res).map_or(0, |r| self.cell.count(r.0))
    }

    /// What it holds, in resource id order, no zero entries.
    pub fn items(&self) -> impl Iterator<Item = (Id, u32)> + 'a {
        let (ledger, cell) = (self.ledger, self.cell);
        ledger.res_order.iter().filter_map(move |&r| Some((ledger.res_ids[r as usize], cell.count(r))).filter(|(_, n)| *n > 0))
    }

    /// The store as saved.
    pub fn to_store(&self) -> Store {
        Store { owner: self.cell.owner, capacity: self.cell.capacity, items: self.items().collect() }
    }
}

impl std::fmt::Debug for StoreRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_store().fmt(f)
    }
}

/// A live store's contents: per resource slot (shorter than the resources when the last ones were never held).
#[derive(Debug, Clone)]
struct Cell {
    owner: Id,
    capacity: u32,
    used: u32,
    items: Vec<u32>,
}

impl Cell {
    fn count(&self, r: u32) -> u32 {
        self.items.get(r as usize).copied().unwrap_or(0)
    }
}

/// One change an in-place transaction made, to put back if a later op is refused.
#[derive(Debug, Clone, Copy)]
enum Undo {
    Count { slot: u32, res: u32, count: u32, used: u32 },
    Total { res: u32, total: i64 },
}

/// The stores, the declared resources and reasons, and what each resource's reasons made and destroyed.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    /// Store id to slot: an entry is never removed, so a slot names one id for the ledger's life.
    slots: BTreeMap<Id, u32>,
    /// Per slot: its id, and the store when it exists.
    cells: Vec<(Id, Option<Cell>)>,
    /// Declared resources: slot to id, id to slot, and the slots in id order.
    res_ids: Vec<Id>,
    res_slots: BTreeMap<Id, u32>,
    res_order: Vec<u32>,
    /// Declared reasons: slot to reason, reason to slot.
    whys: Vec<Reason>,
    why_slots: BTreeMap<Reason, u32>,
    /// Per resource slot: units the declared reasons made minus units they destroyed (what all stores together must hold).
    totals: Vec<i64>,
    /// Scratch, kept between transactions so they allocate nothing: the undo log and an id-keyed transaction resolved to slots.
    undo: Vec<Undo>,
    resolved: Vec<SlotOp>,
}

/// What a ledger saves: the stores and the totals (declarations come from the content, not the save).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerState {
    /// Every store.
    pub stores: BTreeMap<Id, Store>,
    /// Per resource: made minus destroyed.
    pub totals: BTreeMap<Id, i64>,
}

impl PartialEq for Ledger {
    /// The same declarations, stores and totals (slots and scratch are how they are kept, not what they are).
    fn eq(&self, other: &Ledger) -> bool {
        self.res_slots.keys().eq(other.res_slots.keys()) && self.why_slots.keys().eq(other.why_slots.keys()) && self.state() == other.state()
    }
}

impl Eq for Ledger {}

impl Ledger {
    /// An empty ledger.
    pub fn new() -> Ledger {
        Ledger::default()
    }

    /// Declares a resource.
    pub fn resource(&mut self, res: Id) {
        if self.res_slots.contains_key(&res) {
            return;
        }
        let slot = self.res_ids.len() as u32;
        self.res_ids.push(res);
        self.res_slots.insert(res, slot);
        self.totals.push(0);
        self.res_order = self.res_slots.values().copied().collect();
    }

    /// Declares a reason resources may appear or disappear for.
    pub fn declare(&mut self, why: Reason) {
        if !self.why_slots.contains_key(&why) {
            self.why_slots.insert(why, self.whys.len() as u32);
            self.whys.push(why);
        }
    }

    /// Store `id`'s slot, made now if `id` never had one (the store need not exist: an op on it is refused until it does).
    pub fn key(&mut self, id: &Id) -> Slot {
        if let Some(&s) = self.slots.get(id) {
            return Slot(s);
        }
        let s = self.cells.len() as u32;
        self.cells.push((*id, None));
        self.slots.insert(*id, s);
        Slot(s)
    }

    /// Store `id`'s slot, if it ever had one.
    pub fn slot(&self, id: &Id) -> Option<Slot> {
        self.slots.get(id).map(|&s| Slot(s))
    }

    /// Resource `res`'s slot, if it is declared.
    pub fn res(&self, res: &Id) -> Option<Res> {
        self.res_slots.get(res).map(|&r| Res(r))
    }

    /// Reason `why`'s slot, if it is declared.
    pub fn why(&self, why: &Reason) -> Option<Why> {
        self.why_slots.get(why).map(|&w| Why(w))
    }

    fn cell(&self, slot: u32) -> Option<&Cell> {
        self.cells.get(slot as usize).and_then(|(_, c)| c.as_ref())
    }

    /// Adds an empty store; an id already used is a [`code::DUPLICATE`].
    pub fn add_store(&mut self, id: Id, owner: Id, capacity: u32) -> Result<(), Diagnostic> {
        let Slot(s) = self.key(&id);
        let (_, cell) = &mut self.cells[s as usize];
        if cell.is_some() {
            return Err(Diagnostic::new(code::DUPLICATE, id, "a store with this id already exists"));
        }
        *cell = Some(Cell { owner, capacity, used: 0, items: Vec::new() });
        Ok(())
    }

    /// Removes store `id` (its owner is gone), handing everything it holds to `into` in one transaction; if `into` has no room, nothing changes.
    pub fn remove_store(&mut self, id: &Id, into: &Id) -> Result<(), Diagnostic> {
        self.remove_stores(std::slice::from_ref(id), into)
    }

    /// Removes every store in `ids` (one owner's, gone together), handing all they hold to `into` in one transaction: if `into` has no room
    /// for all of it, or a store is unknown, nothing changes.
    pub fn remove_stores(&mut self, ids: &[Id], into: &Id) -> Result<(), Diagnostic> {
        if self.store(into).is_none() {
            return Err(Diagnostic::new(code::UNKNOWN, into, "no such store"));
        }
        let mut tx = Tx::new();
        for id in ids {
            if ids.iter().filter(|i| *i == id).count() > 1 {
                return Err(Diagnostic::new(code::DUPLICATE, id, "named twice"));
            }
            if id == into {
                return Err(Diagnostic::new(code::DEPENDENCY, id, "cannot hand its contents to itself").fix("hand them to a store that stays"));
            }
            let store = self.store(id).ok_or_else(|| Diagnostic::new(code::UNKNOWN, id, "no such store"))?;
            for (res, n) in store.items() {
                tx = tx.moving(id, into, &res, n);
            }
        }
        self.apply(&tx)?;
        for id in ids {
            if let Some(&s) = self.slots.get(id) {
                self.cells[s as usize].1 = None;
            }
        }
        Ok(())
    }

    /// The store `id`, if it exists.
    pub fn store(&self, id: &Id) -> Option<StoreRef<'_>> {
        let cell = self.cell(*self.slots.get(id)?)?;
        Some(StoreRef { ledger: self, cell })
    }

    /// Every store in id order.
    pub fn stores(&self) -> impl Iterator<Item = (Id, StoreRef<'_>)> {
        self.slots.iter().filter_map(move |(id, &s)| Some((*id, StoreRef { ledger: self, cell: self.cell(s)? })))
    }

    /// How many of `res` store `id` holds (`0` for an unknown store).
    pub fn count(&self, id: &Id, res: &Id) -> u32 {
        self.store(id).map_or(0, |s| s.count(res))
    }

    /// Room left in store `id`.
    pub fn free(&self, id: &Id) -> u32 {
        self.store(id).map_or(0, |s| s.free())
    }

    /// How many of `res` the store in `slot` holds (`0` when it does not exist).
    pub fn count_at(&self, slot: Slot, res: Res) -> u32 {
        self.cell(slot.0).map_or(0, |c| c.count(res.0))
    }

    /// Room left in the store in `slot` (`0` when it does not exist).
    pub fn free_at(&self, slot: Slot) -> u32 {
        self.cell(slot.0).map_or(0, |c| c.capacity.saturating_sub(c.used))
    }

    /// What the store in `slot` holds, in resource id order, no zero entries (nothing when it does not exist).
    pub fn items_at(&self, slot: Slot) -> impl Iterator<Item = (Res, u32)> + '_ {
        let cell = self.cell(slot.0);
        self.res_order.iter().filter_map(move |&r| Some((Res(r), cell?.count(r))).filter(|(_, n)| *n > 0))
    }

    /// Whether `tx` would apply now, and the first reason it would not.
    pub fn check(&self, tx: &Tx) -> Result<(), Diagnostic> {
        let ops: Vec<SlotOp> = tx.ops.iter().map(|op| self.resolve(op)).collect();
        self.check_ops(&ops).map_err(|f| self.explain_tx(&f, tx))
    }

    /// Applies `tx` whole, or changes nothing and says why.
    pub fn apply(&mut self, tx: &Tx) -> Result<(), Diagnostic> {
        self.try_apply(tx).map_err(|f| self.explain_tx(&f, tx))
    }

    /// [`apply`](Ledger::apply) with the refusal unexplained (nothing allocated for it): for a system that tries every tick and only counts
    /// what went through.
    pub fn try_apply(&mut self, tx: &Tx) -> Result<(), Fail> {
        let mut ops = std::mem::take(&mut self.resolved);
        ops.clear();
        ops.extend(tx.ops.iter().map(|op| self.resolve(op)));
        let result = self.apply_ops(&ops);
        self.resolved = ops;
        result
    }

    fn resolve(&self, op: &Op) -> SlotOp {
        let store = |id: &Id| Slot(self.slots.get(id).copied().unwrap_or(NONE));
        let res = |id: &Id| Res(self.res_slots.get(id).copied().unwrap_or(NONE));
        let why = |w: &Reason| Why(self.why_slots.get(w).copied().unwrap_or(NONE));
        match op {
            Op::Move { from, to, res: r, n } => SlotOp::Move { from: store(from), to: store(to), res: res(r), n: *n },
            Op::Consume { from, res: r, n, why: w } => SlotOp::Consume { from: store(from), res: res(r), n: *n, why: why(w) },
            Op::Produce { to, res: r, n, why: w } => SlotOp::Produce { to: store(to), res: res(r), n: *n, why: why(w) },
        }
    }

    /// Whether `ops` would apply now.
    pub fn check_ops(&self, ops: &[SlotOp]) -> Result<(), Fail> {
        // A copy of just the counts the ops touch.
        let mut work = Overlay { ledger: self, counts: Vec::new(), used: Vec::new() };
        run(&mut work, &self.whys, ops)
    }

    /// Applies `ops` whole, or changes nothing; allocates nothing once its scratch has grown to the largest transaction.
    pub fn apply_ops(&mut self, ops: &[SlotOp]) -> Result<(), Fail> {
        let mut undo = std::mem::take(&mut self.undo);
        undo.clear();
        let whys = std::mem::take(&mut self.whys);
        let result = run(&mut InPlace { ledger: self, undo: &mut undo }, &whys, ops);
        self.whys = whys;
        if result.is_err() {
            for u in undo.iter().rev() {
                match *u {
                    Undo::Count { slot, res, count, used } => {
                        if let Some(c) = self.cells[slot as usize].1.as_mut() {
                            c.items[res as usize] = count;
                            c.used = used;
                        }
                    }
                    Undo::Total { res, total } => self.totals[res as usize] = total,
                }
            }
        }
        self.undo = undo;
        result
    }

    /// `fail` (from [`apply_ops`](Ledger::apply_ops) or [`check_ops`](Ledger::check_ops) on `ops`, nothing changed since) in words.
    pub fn explain(&self, fail: &Fail, ops: &[SlotOp]) -> Diagnostic {
        let store = |s: Slot| self.cells.get(s.0 as usize).map_or_else(|| "?".to_string(), |(id, _)| id.to_string());
        let res = |r: Res| self.res_ids.get(r.0 as usize).map_or_else(|| "?".to_string(), |id| id.to_string());
        let why = |w: Why| self.whys.get(w.0 as usize).map_or_else(|| "an undeclared reason".to_string(), |w| w.to_string());
        let names = match ops.get(fail.op) {
            Some(SlotOp::Move { from, to, res: r, .. }) => (store(*from), store(*to), res(*r), String::new()),
            Some(SlotOp::Consume { from, res: r, why: w, .. }) => (store(*from), String::new(), res(*r), why(*w)),
            Some(SlotOp::Produce { to, res: r, why: w, .. }) => (String::new(), store(*to), res(*r), why(*w)),
            None => Default::default(),
        };
        let consume = matches!(ops.get(fail.op), Some(SlotOp::Consume { .. }));
        describe(fail, consume, &names.0, &names.1, &names.2, &names.3)
    }

    fn explain_tx(&self, fail: &Fail, tx: &Tx) -> Diagnostic {
        let s = |id: &Id| id.to_string();
        let (from, to, res, why, consume) = match tx.ops.get(fail.op) {
            Some(Op::Move { from, to, res, .. }) => (s(from), s(to), s(res), String::new(), false),
            Some(Op::Consume { from, res, why, .. }) => (s(from), String::new(), s(res), why.to_string(), true),
            Some(Op::Produce { to, res, why, .. }) => (String::new(), s(to), s(res), why.to_string(), false),
            None => Default::default(),
        };
        describe(fail, consume, &from, &to, &res, &why)
    }

    /// Proves the stores hold exactly what the declared reasons made minus what they destroyed, per resource, and that no store is over its
    /// capacity: a resource duplicated or lost outside a transaction (or by a corrupt save) fails here.
    pub fn audit(&self) -> Result<(), Diagnostic> {
        let mut held = vec![0i64; self.res_ids.len()];
        for (id, store) in self.stores() {
            let c = store.cell;
            let sum: u64 = c.items.iter().map(|n| *n as u64).sum();
            if sum > c.capacity as u64 {
                return Err(Diagnostic::new(code::FULL, id, format!("holds {sum} but its capacity is {}", c.capacity)));
            }
            if c.used as u64 != sum {
                return Err(Diagnostic::new("invalid", id, format!("counts {} units but holds {sum}", c.used)));
            }
            for (r, n) in c.items.iter().enumerate() {
                held[r] += *n as i64;
            }
        }
        for &r in &self.res_order {
            let (have, made) = (held[r as usize], self.totals[r as usize]);
            if have != made {
                return Err(Diagnostic::new(
                    "invalid",
                    self.res_ids[r as usize],
                    format!("the stores hold {have} but the declared reasons account for {made}"),
                ));
            }
        }
        Ok(())
    }

    /// What to save.
    pub fn state(&self) -> LedgerState {
        let totals = self.res_order.iter().filter(|&&r| self.totals[r as usize] != 0).map(|&r| (self.res_ids[r as usize], self.totals[r as usize])).collect();
        LedgerState { stores: self.stores().map(|(id, s)| (id, s.to_store())).collect(), totals }
    }

    /// Replaces the stores and totals with saved ones (the declarations stay the content's), after checking they audit and name only
    /// declared resources; on any problem nothing changes. Slots stay the same for the same store ids.
    pub fn restore(&mut self, state: LedgerState) -> Result<(), Diagnostic> {
        let mut next = self.clone();
        for (_, cell) in &mut next.cells {
            *cell = None;
        }
        next.totals.iter_mut().for_each(|t| *t = 0);
        for (res, n) in &state.totals {
            let r = next.res(res).ok_or_else(|| Diagnostic::new(code::UNKNOWN, res, "the save counts a resource that is not declared"))?;
            next.totals[r.0 as usize] = *n;
        }
        for (id, s) in &state.stores {
            let mut items = vec![0u32; next.res_ids.len()];
            for (res, n) in &s.items {
                let r = next.res(res).ok_or_else(|| Diagnostic::new(code::UNKNOWN, id, format!("holds `{res}`, which is not a declared resource")))?;
                items[r.0 as usize] = *n;
            }
            let used = items.iter().fold(0u32, |a, n| a.saturating_add(*n));
            let Slot(slot) = next.key(id);
            next.cells[slot as usize].1 = Some(Cell { owner: s.owner, capacity: s.capacity, used, items });
        }
        next.audit()?;
        *self = next;
        Ok(())
    }

    /// A hash of every store's contents and the totals (the match checksum folds it in).
    pub fn hash(&self) -> u64 {
        let mut h = Fnv::new();
        for (id, s) in self.stores() {
            h.str(id.as_str());
            h.u64(s.capacity() as u64);
            for (res, n) in s.items() {
                h.str(res.as_str());
                h.u64(n as u64);
            }
        }
        for &r in &self.res_order {
            let n = self.totals[r as usize];
            if n != 0 {
                h.str(self.res_ids[r as usize].as_str());
                h.u64(n as u64);
            }
        }
        h.0
    }
}

/// `fail` in words, with the names of what its op touched.
fn describe(fail: &Fail, consume: bool, from: &str, to: &str, res: &str, why: &str) -> Diagnostic {
    match fail.kind {
        FailKind::UnknownResource => Diagnostic::new(code::UNKNOWN, res, "not a declared resource"),
        FailKind::Undeclared => {
            let verb = if consume { "consume" } else { "produce" };
            Diagnostic::new(code::DEPENDENCY, why, format!("cannot {verb}: {why} is not declared for it"))
                .fix("resources appear only through a declared recipe or source and leave only through a declared recipe or sink")
        }
        FailKind::UnknownFrom => Diagnostic::new(code::UNKNOWN, from, "no such store"),
        FailKind::UnknownTo => Diagnostic::new(code::UNKNOWN, to, "no such store"),
        FailKind::Missing { have, need } => {
            Diagnostic::new(code::MISSING, from, format!("needs {need} `{res}`, holds {have}")).fix(format!("bring {} more `{res}`", need - have))
        }
        FailKind::Full { free, need } => {
            Diagnostic::new(code::FULL, to, format!("no room for {need} (room for {free})")).fix(format!("free {} units of it", need - free))
        }
    }
}

/// The counts a transaction reads and writes: the ledger itself (with an undo log), or a copy of the touched counts (for `check`).
trait Work {
    fn exists(&self, slot: u32) -> bool;
    fn count(&self, slot: u32, res: u32) -> u32;
    fn room(&self, slot: u32) -> u32;
    fn set(&mut self, slot: u32, res: u32, count: u32);
    fn total(&mut self, res: u32, delta: i64);
    fn declared(&self, res: u32) -> bool;
}

/// Runs `ops` in order on `w`, stopping at the first one refused (the caller puts back what the earlier ones did).
fn run(w: &mut impl Work, whys: &[Reason], ops: &[SlotOp]) -> Result<(), Fail> {
    for (i, op) in ops.iter().enumerate() {
        let fail = |kind| Err(Fail { op: i, kind });
        let (from, to, res, n, why, consume) = match *op {
            SlotOp::Move { from, to, res, n } => (Some(from), Some(to), res, n, None, false),
            SlotOp::Consume { from, res, n, why } => (Some(from), None, res, n, Some(why), true),
            SlotOp::Produce { to, res, n, why } => (None, Some(to), res, n, Some(why), false),
        };
        if !w.declared(res.0) {
            return fail(FailKind::UnknownResource);
        }
        if let Some(why) = why {
            if !whys.get(why.0 as usize).is_some_and(|w| w.allows(consume)) {
                return fail(FailKind::Undeclared);
            }
        }
        if from.is_some_and(|s| !w.exists(s.0)) {
            return fail(FailKind::UnknownFrom);
        }
        if to.is_some_and(|s| !w.exists(s.0)) {
            return fail(FailKind::UnknownTo);
        }
        if let Some(from) = from {
            let have = w.count(from.0, res.0);
            if have < n {
                return fail(FailKind::Missing { have, need: n });
            }
            w.set(from.0, res.0, have - n);
        }
        if let Some(to) = to {
            let free = w.room(to.0);
            if free < n {
                return fail(FailKind::Full { free, need: n });
            }
            let have = w.count(to.0, res.0);
            w.set(to.0, res.0, have + n);
        }
        match op {
            SlotOp::Consume { .. } => w.total(res.0, -(n as i64)),
            SlotOp::Produce { .. } => w.total(res.0, n as i64),
            SlotOp::Move { .. } => {}
        }
    }
    Ok(())
}

struct InPlace<'a> {
    ledger: &'a mut Ledger,
    undo: &'a mut Vec<Undo>,
}

impl Work for InPlace<'_> {
    fn exists(&self, slot: u32) -> bool {
        self.ledger.cell(slot).is_some()
    }
    fn count(&self, slot: u32, res: u32) -> u32 {
        self.ledger.cell(slot).map_or(0, |c| c.count(res))
    }
    fn room(&self, slot: u32) -> u32 {
        self.ledger.cell(slot).map_or(0, |c| c.capacity.saturating_sub(c.used))
    }
    fn set(&mut self, slot: u32, res: u32, count: u32) {
        let resources = self.ledger.res_ids.len();
        if let Some(c) = self.ledger.cells.get_mut(slot as usize).and_then(|(_, c)| c.as_mut()) {
            if c.items.len() < resources {
                c.items.resize(resources, 0);
            }
            let old = c.items[res as usize];
            self.undo.push(Undo::Count { slot, res, count: old, used: c.used });
            c.used = c.used - old + count;
            c.items[res as usize] = count;
        }
    }
    fn total(&mut self, res: u32, delta: i64) {
        let t = &mut self.ledger.totals[res as usize];
        self.undo.push(Undo::Total { res, total: *t });
        *t += delta;
    }
    fn declared(&self, res: u32) -> bool {
        (res as usize) < self.ledger.res_ids.len()
    }
}

struct Overlay<'a> {
    ledger: &'a Ledger,
    counts: Vec<(u32, u32, u32)>,
    used: Vec<(u32, u32)>,
}

impl Work for Overlay<'_> {
    fn exists(&self, slot: u32) -> bool {
        self.ledger.cell(slot).is_some()
    }
    fn count(&self, slot: u32, res: u32) -> u32 {
        self.counts.iter().rev().find(|(s, r, _)| *s == slot && *r == res).map_or_else(|| self.ledger.cell(slot).map_or(0, |c| c.count(res)), |c| c.2)
    }
    fn room(&self, slot: u32) -> u32 {
        let cap = self.ledger.cell(slot).map_or(0, |c| c.capacity);
        let used = self.used.iter().rev().find(|(s, _)| *s == slot).map_or_else(|| self.ledger.cell(slot).map_or(0, |c| c.used), |u| u.1);
        cap.saturating_sub(used)
    }
    fn set(&mut self, slot: u32, res: u32, count: u32) {
        let old = self.count(slot, res);
        let used = self.used.iter().rev().find(|(s, _)| *s == slot).map_or_else(|| self.ledger.cell(slot).map_or(0, |c| c.used), |u| u.1);
        self.counts.push((slot, res, count));
        self.used.push((slot, used - old + count));
    }
    fn total(&mut self, _res: u32, _delta: i64) {}
    fn declared(&self, res: u32) -> bool {
        (res as usize) < self.ledger.res_ids.len()
    }
}

/// FNV-1a, for state hashes that are the same on every platform.
pub struct Fnv(pub u64);

impl Fnv {
    /// The FNV offset basis.
    pub fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    /// Mixes 8 bytes.
    pub fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    /// Mixes a string and its length.
    pub fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        for b in s.bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

impl Default for Fnv {
    fn default() -> Self {
        Fnv::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> Id {
        Id::parse(s).unwrap()
    }

    /// Ore comes from a mine; smelting makes 1 ingot of 2 ore; shipping takes ingots; a recycler turns an ingot back into 2 ore (a cycle on
    /// purpose).
    fn ledger() -> Ledger {
        let mut l = Ledger::new();
        for r in ["t:ore", "t:ingot"] {
            l.resource(id(r));
        }
        for why in [Reason::Source(id("t:mine")), Reason::Recipe(id("t:smelt")), Reason::Recipe(id("t:recycle")), Reason::Sink(id("t:ship"))] {
            l.declare(why);
        }
        l.add_store(id("t:storage"), id("t:workshop"), 10).unwrap();
        l.add_store(id("t:furnace/in"), id("t:furnace"), 4).unwrap();
        l
    }

    fn mine(n: u32) -> Tx {
        Tx::new().producing(&id("t:storage"), &id("t:ore"), n, &Reason::Source(id("t:mine")))
    }

    fn smelt(store: &str) -> Tx {
        let why = Reason::Recipe(id("t:smelt"));
        Tx::new().consuming(&id(store), &id("t:ore"), 2, &why).producing(&id(store), &id("t:ingot"), 1, &why)
    }

    #[test]
    fn a_transaction_applies_whole_or_changes_nothing() {
        let mut l = ledger();
        l.apply(&mine(3)).unwrap();
        let before = l.clone();
        // Moves 3 ore into a 4-unit input that already holds 3: the first move fits, the second does not, so neither happens.
        l.apply(&Tx::new().moving(&id("t:storage"), &id("t:furnace/in"), &id("t:ore"), 3)).unwrap();
        let after_first = l.clone();
        let e = l.apply(&mine(7).moving(&id("t:storage"), &id("t:furnace/in"), &id("t:ore"), 2)).unwrap_err();
        assert_eq!(e.code, code::FULL, "{e}");
        assert_eq!(l, after_first, "a failed transaction changes nothing, the production before the failing move included");
        assert_ne!(before, after_first);
        let e = l.apply(&smelt("t:storage")).unwrap_err();
        assert_eq!((e.code, e.fix.as_deref()), (code::MISSING, Some("bring 2 more `t:ore`")), "{e}");
        l.audit().unwrap();
    }

    #[test]
    fn resources_appear_and_leave_only_for_a_declared_reason() {
        let mut l = ledger();
        let e = l.apply(&Tx::new().producing(&id("t:storage"), &id("t:ore"), 1, &Reason::Source(id("t:printer")))).unwrap_err();
        assert_eq!(e.code, code::DEPENDENCY);
        let e = l.apply(&Tx::new().producing(&id("t:storage"), &id("t:ore"), 1, &Reason::Sink(id("t:ship")))).unwrap_err();
        assert_eq!(e.code, code::DEPENDENCY, "a sink cannot produce");
        let e = l.apply(&Tx::new().producing(&id("t:storage"), &id("t:gold"), 1, &Reason::Source(id("t:mine")))).unwrap_err();
        assert_eq!(e.code, code::UNKNOWN, "an undeclared resource");
        assert_eq!(l, ledger());
    }

    #[test]
    fn an_intentional_cycle_runs_and_still_audits() {
        let mut l = ledger();
        l.apply(&mine(4)).unwrap();
        let recycle = Reason::Recipe(id("t:recycle"));
        for _ in 0..50 {
            l.apply(&smelt("t:storage")).unwrap();
            l.apply(&Tx::new().consuming(&id("t:storage"), &id("t:ingot"), 1, &recycle).producing(&id("t:storage"), &id("t:ore"), 2, &recycle)).unwrap();
            l.audit().unwrap();
        }
        assert_eq!((l.count(&id("t:storage"), &id("t:ore")), l.count(&id("t:storage"), &id("t:ingot"))), (4, 0));
        l.apply(&smelt("t:storage")).unwrap();
        l.apply(&Tx::new().consuming(&id("t:storage"), &id("t:ingot"), 1, &Reason::Sink(id("t:ship")))).unwrap();
        l.audit().unwrap();
    }

    #[test]
    fn removing_a_store_hands_its_contents_on_or_fails_whole() {
        let mut l = ledger();
        l.apply(&mine(10)).unwrap();
        l.apply(&Tx::new().moving(&id("t:storage"), &id("t:furnace/in"), &id("t:ore"), 4)).unwrap();
        l.apply(&mine(4)).unwrap();
        // Storage is full (10 of 10): the furnace's 4 ore have nowhere to go, so the furnace's store stays, untouched.
        let e = l.remove_store(&id("t:furnace/in"), &id("t:storage")).unwrap_err();
        assert_eq!(e.code, code::FULL, "{e}");
        assert_eq!(l.count(&id("t:furnace/in"), &id("t:ore")), 4);
        l.apply(&Tx::new().consuming(&id("t:storage"), &id("t:ore"), 4, &Reason::Sink(id("t:ship")))).unwrap();
        l.remove_store(&id("t:furnace/in"), &id("t:storage")).unwrap();
        assert!(l.store(&id("t:furnace/in")).is_none());
        assert_eq!(l.count(&id("t:storage"), &id("t:ore")), 10);
        l.audit().unwrap();
    }

    #[test]
    fn removing_several_stores_is_one_transaction() {
        let mut l = ledger();
        l.add_store(id("t:furnace/out"), id("t:furnace"), 4).unwrap();
        l.apply(&mine(4)).unwrap();
        l.apply(&Tx::new().moving(&id("t:storage"), &id("t:furnace/in"), &id("t:ore"), 2)).unwrap();
        l.apply(&smelt("t:furnace/in")).unwrap();
        l.apply(&Tx::new().moving(&id("t:furnace/in"), &id("t:furnace/out"), &id("t:ingot"), 1)).unwrap();
        l.apply(&Tx::new().moving(&id("t:storage"), &id("t:furnace/in"), &id("t:ore"), 2)).unwrap();
        l.apply(&mine(9)).unwrap();
        // Storage has room for 1: the input's 2 ore alone do not fit, nor the output's 1 ingot with them. Both stores stay, untouched.
        let both = [id("t:furnace/in"), id("t:furnace/out")];
        let before = l.clone();
        assert_eq!(l.remove_stores(&both, &id("t:storage")).unwrap_err().code, code::FULL);
        assert_eq!(l, before);
        assert_eq!(l.remove_stores(&[id("t:furnace/out"), id("t:nothing")], &id("t:storage")).unwrap_err().code, code::UNKNOWN);
        assert_eq!(l.remove_stores(&both, &id("t:furnace/in")).unwrap_err().code, code::DEPENDENCY, "into one of the stores it removes");
        assert_eq!(l.remove_stores(&both, &id("t:nowhere")).unwrap_err().code, code::UNKNOWN);
        assert_eq!(l, before);
        l.apply(&Tx::new().consuming(&id("t:storage"), &id("t:ore"), 2, &Reason::Sink(id("t:ship")))).unwrap();
        l.remove_stores(&both, &id("t:storage")).unwrap();
        assert!(l.store(&both[0]).is_none() && l.store(&both[1]).is_none());
        l.audit().unwrap();
    }

    #[test]
    fn a_save_restores_exactly_and_a_tampered_one_is_refused_without_change() {
        let mut l = ledger();
        l.apply(&mine(5)).unwrap();
        l.apply(&smelt("t:storage")).unwrap();
        let saved = serde_json::to_value(l.state()).unwrap();
        let mut fresh = ledger();
        fresh.restore(serde_json::from_value(saved.clone()).unwrap()).unwrap();
        assert_eq!((fresh.hash(), fresh.state()), (l.hash(), l.state()));
        let mut forged = saved;
        forged["stores"]["t:storage"]["items"]["t:ingot"] = serde_json::json!(2); // one ingot more than was ever smelted
        let e = fresh.restore(serde_json::from_value(forged).unwrap()).unwrap_err();
        assert!(e.message.contains("account for"), "{e}");
        assert_eq!(fresh.state(), l.state(), "a refused save changes nothing");
    }
}
