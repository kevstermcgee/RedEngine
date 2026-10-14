//! Countable resources in stores, changed only by transactions that apply whole or not at all (ADR
//! 2026-10-14-game-systems-compose-under-one-match-authority).
//!
//! A [`Store`] has an explicit owner (the instance or system responsible for it) and a capacity in units. A [`Tx`] is a list of [`Op`]s: a
//! move between stores conserves what it moves; resources come into being or leave only for a declared [`Reason`] (a recipe, a source or a
//! sink the content defines). [`Ledger::apply`] checks every op in order against a working copy and commits all or none: a missing input is
//! [`code::MISSING`], a full destination [`code::FULL`], and nothing changes. [`Ledger::check`] is the same without committing (what an
//! inspection asks: "would this run?"). The ledger keeps, per resource, how much the declared reasons made and destroyed, so
//! [`Ledger::audit`] proves the stores hold exactly that (after a load, or in a test that tries to break it). Removing a store (a despawned
//! machine) hands its contents to another store or fails.
//!
//! Used by the game-system host (`sim::systems`) and its games (Workshop's machines and storage, the ability fixture's mana).

use super::defs::{code, Diagnostic, Id};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Why resources may appear or disappear: a definition the content declares ([`Ledger::declare`]).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Reason {
    /// A recipe: consumes its inputs and produces its outputs.
    Recipe(Id),
    /// A source: produces from nothing in the game (a mine, a delivery, a starting stock).
    Source(Id),
    /// A sink: consumes into nothing in the game (shipping, an ability's cost).
    Sink(Id),
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
        self.ops.push(Op::Move { from: from.clone(), to: to.clone(), res: res.clone(), n });
        self
    }

    /// Adds a consumption for `why`.
    pub fn consuming(mut self, from: &Id, res: &Id, n: u32, why: &Reason) -> Tx {
        self.ops.push(Op::Consume { from: from.clone(), res: res.clone(), n, why: why.clone() });
        self
    }

    /// Adds a production for `why`.
    pub fn producing(mut self, to: &Id, res: &Id, n: u32, why: &Reason) -> Tx {
        self.ops.push(Op::Produce { to: to.clone(), res: res.clone(), n, why: why.clone() });
        self
    }
}

/// A store of resources.
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

    fn take(&mut self, res: &Id, n: u32) -> bool {
        let have = self.count(res);
        if have < n {
            return false;
        }
        if have == n {
            self.items.remove(res);
        } else {
            self.items.insert(res.clone(), have - n);
        }
        true
    }

    fn put(&mut self, res: &Id, n: u32) -> bool {
        if self.free() < n {
            return false;
        }
        if n > 0 {
            *self.items.entry(res.clone()).or_insert(0) += n;
        }
        true
    }
}

/// The stores, the declared reasons, and what each resource's reasons made and destroyed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ledger {
    stores: BTreeMap<Id, Store>,
    resources: BTreeSet<Id>,
    declared: BTreeSet<Reason>,
    /// Per resource: units the declared reasons made minus units they destroyed (what all stores together must hold).
    totals: BTreeMap<Id, i64>,
}

/// What a ledger saves: the stores and the totals (declarations come from the content, not the save).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerState {
    /// Every store.
    pub stores: BTreeMap<Id, Store>,
    /// Per resource: made minus destroyed.
    pub totals: BTreeMap<Id, i64>,
}

impl Ledger {
    /// An empty ledger.
    pub fn new() -> Ledger {
        Ledger::default()
    }

    /// Declares a resource.
    pub fn resource(&mut self, res: Id) {
        self.resources.insert(res);
    }

    /// Declares a reason resources may appear or disappear for.
    pub fn declare(&mut self, why: Reason) {
        self.declared.insert(why);
    }

    /// Adds an empty store; an id already used is a [`code::DUPLICATE`].
    pub fn add_store(&mut self, id: Id, owner: Id, capacity: u32) -> Result<(), Diagnostic> {
        if self.stores.contains_key(&id) {
            return Err(Diagnostic::new(code::DUPLICATE, &id, "a store with this id already exists"));
        }
        self.stores.insert(id, Store { owner, capacity, items: BTreeMap::new() });
        Ok(())
    }

    /// Removes store `id` (its owner is gone), handing everything it holds to `into` in one transaction; if `into` has no room, nothing changes.
    pub fn remove_store(&mut self, id: &Id, into: &Id) -> Result<(), Diagnostic> {
        let store = self.stores.get(id).ok_or_else(|| Diagnostic::new(code::UNKNOWN, id, "no such store"))?;
        let mut tx = Tx::new();
        for (res, n) in &store.items {
            tx = tx.moving(id, into, res, *n);
        }
        self.apply(&tx)?;
        self.stores.remove(id);
        Ok(())
    }

    /// The store `id`.
    pub fn store(&self, id: &Id) -> Option<&Store> {
        self.stores.get(id)
    }

    /// Every store in id order.
    pub fn stores(&self) -> impl Iterator<Item = (&Id, &Store)> {
        self.stores.iter()
    }

    /// How many of `res` store `id` holds (`0` for an unknown store).
    pub fn count(&self, id: &Id, res: &Id) -> u32 {
        self.stores.get(id).map_or(0, |s| s.count(res))
    }

    /// Room left in store `id`.
    pub fn free(&self, id: &Id) -> u32 {
        self.stores.get(id).map_or(0, Store::free)
    }

    /// Whether `tx` would apply now, and the first reason it would not.
    pub fn check(&self, tx: &Tx) -> Result<(), Diagnostic> {
        self.run(tx).map(|_| ())
    }

    /// Applies `tx` whole, or changes nothing and says why.
    pub fn apply(&mut self, tx: &Tx) -> Result<(), Diagnostic> {
        let (stores, totals) = self.run(tx)?;
        for (id, s) in stores {
            self.stores.insert(id, s);
        }
        self.totals = totals;
        Ok(())
    }

    /// The stores `tx` touches as they would be after it, and the totals, or the first problem.
    #[allow(clippy::type_complexity)]
    fn run(&self, tx: &Tx) -> Result<(BTreeMap<Id, Store>, BTreeMap<Id, i64>), Diagnostic> {
        let mut work: BTreeMap<Id, Store> = BTreeMap::new();
        let mut totals = self.totals.clone();
        let stores = &self.stores;
        let fetch = |work: &mut BTreeMap<Id, Store>, id: &Id| -> Result<(), Diagnostic> {
            if !work.contains_key(id) {
                let s = stores.get(id).ok_or_else(|| Diagnostic::new(code::UNKNOWN, id, "no such store"))?;
                work.insert(id.clone(), s.clone());
            }
            Ok(())
        };
        fn store<'w>(work: &'w mut BTreeMap<Id, Store>, id: &Id) -> Result<&'w mut Store, Diagnostic> {
            work.get_mut(id).ok_or_else(|| Diagnostic::new(code::UNKNOWN, id, "no such store"))
        }
        let known = |res: &Id| -> Result<(), Diagnostic> {
            if self.resources.contains(res) {
                Ok(())
            } else {
                Err(Diagnostic::new(code::UNKNOWN, res, "not a declared resource"))
            }
        };
        let declared = |why: &Reason, consume: bool| -> Result<(), Diagnostic> {
            let allowed = matches!((why, consume), (Reason::Recipe(_), _) | (Reason::Source(_), false) | (Reason::Sink(_), true));
            if !allowed || !self.declared.contains(why) {
                let verb = if consume { "consume" } else { "produce" };
                return Err(Diagnostic::new(code::DEPENDENCY, why, format!("cannot {verb}: {why} is not declared for it"))
                    .fix("resources appear only through a declared recipe or source and leave only through a declared recipe or sink"));
            }
            Ok(())
        };
        let missing = |store: &Id, res: &Id, have: u32, need: u32| {
            Diagnostic::new(code::MISSING, store, format!("needs {need} `{res}`, holds {have}")).fix(format!("bring {} more `{res}`", need - have))
        };
        let full = |store: &Id, free: u32, need: u32| {
            Diagnostic::new(code::FULL, store, format!("no room for {need} (room for {free})")).fix(format!("free {} units of it", need - free))
        };
        for op in &tx.ops {
            match op {
                Op::Move { from, to, res, n } => {
                    known(res)?;
                    fetch(&mut work, from)?;
                    fetch(&mut work, to)?;
                    let src = store(&mut work, from)?;
                    let have = src.count(res);
                    if !src.take(res, *n) {
                        return Err(missing(from, res, have, *n));
                    }
                    let dst = store(&mut work, to)?;
                    let free = dst.free();
                    if !dst.put(res, *n) {
                        return Err(full(to, free, *n));
                    }
                }
                Op::Consume { from, res, n, why } => {
                    known(res)?;
                    declared(why, true)?;
                    fetch(&mut work, from)?;
                    let src = store(&mut work, from)?;
                    let have = src.count(res);
                    if !src.take(res, *n) {
                        return Err(missing(from, res, have, *n));
                    }
                    *totals.entry(res.clone()).or_insert(0) -= *n as i64;
                }
                Op::Produce { to, res, n, why } => {
                    known(res)?;
                    declared(why, false)?;
                    fetch(&mut work, to)?;
                    let dst = store(&mut work, to)?;
                    let free = dst.free();
                    if !dst.put(res, *n) {
                        return Err(full(to, free, *n));
                    }
                    *totals.entry(res.clone()).or_insert(0) += *n as i64;
                }
            }
        }
        totals.retain(|_, v| *v != 0);
        Ok((work, totals))
    }

    /// Proves the stores hold exactly what the declared reasons made minus what they destroyed, per resource, and that no store is over its
    /// capacity: a resource duplicated or lost outside a transaction (or by a corrupt save) fails here.
    pub fn audit(&self) -> Result<(), Diagnostic> {
        let mut held: BTreeMap<&Id, i64> = BTreeMap::new();
        for (id, s) in &self.stores {
            if s.used() > s.capacity {
                return Err(Diagnostic::new(code::FULL, id, format!("holds {} but its capacity is {}", s.used(), s.capacity)));
            }
            for (res, n) in &s.items {
                *held.entry(res).or_insert(0) += *n as i64;
            }
        }
        let resources: BTreeSet<&Id> = held.keys().copied().chain(self.totals.keys()).collect();
        for res in resources {
            let (have, made) = (held.get(res).copied().unwrap_or(0), self.totals.get(res).copied().unwrap_or(0));
            if have != made {
                return Err(Diagnostic::new("invalid", res, format!("the stores hold {have} but the declared reasons account for {made}")));
            }
        }
        Ok(())
    }

    /// What to save.
    pub fn state(&self) -> LedgerState {
        LedgerState { stores: self.stores.clone(), totals: self.totals.clone() }
    }

    /// Replaces the stores and totals with saved ones (the declarations stay the content's), after checking they audit and name only
    /// declared resources; on any problem nothing changes.
    pub fn restore(&mut self, state: LedgerState) -> Result<(), Diagnostic> {
        let mut next = self.clone();
        next.stores = state.stores;
        next.totals = state.totals;
        for (id, s) in &next.stores {
            if let Some(res) = s.items.keys().find(|r| !next.resources.contains(*r)) {
                return Err(Diagnostic::new(code::UNKNOWN, id, format!("holds `{res}`, which is not a declared resource")));
            }
        }
        next.audit()?;
        *self = next;
        Ok(())
    }

    /// A hash of every store's contents and the totals (the match checksum folds it in).
    pub fn hash(&self) -> u64 {
        let mut h = Fnv::new();
        for (id, s) in &self.stores {
            h.str(id.as_str());
            h.u64(s.capacity as u64);
            for (res, n) in &s.items {
                h.str(res.as_str());
                h.u64(*n as u64);
            }
        }
        for (res, n) in &self.totals {
            h.str(res.as_str());
            h.u64(*n as u64);
        }
        h.0
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
