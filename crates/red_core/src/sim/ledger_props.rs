//! Property tests of the ledger: seeded random sequences of moves, consumptions, productions and store changes run against a plain model of
//! what the ledger promises (ADR 2026-10-14-game-systems-compose-under-one-match-authority). Every step must agree with the model (the same
//! result, the same code, the same stores); after every step the ledger audits; a failed `apply` changes neither `state()` nor `hash()`; and
//! `apply` after `check` said Ok always succeeds. A failure names its seed and step: `SEEDS` narrowed to it reproduces it alone.
//!
//! The generator is the host's splitmix64 (no crate), so the sequences are the same on every platform and every run.

use super::defs::{code, Id};
use super::ledger::{Ledger, Op, Reason, Tx};
use std::collections::BTreeMap;
use std::ops::Range;

/// Sequences run, and steps in each.
const SEEDS: Range<u64> = 0..300;
const STEPS: usize = 200;

/// Store names: the first three exist at the start, the rest may be added (and every one may be removed, or named while unknown).
const STORES: [&str; 6] = ["p:s0", "p:s1", "p:s2", "p:s3", "p:s4", "p:s5"];
/// Resources: the last is never declared.
const RESOURCES: [&str; 3] = ["p:ore", "p:ingot", "p:gold"];

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        z % n.max(1)
    }

    fn pick<'a, T>(&mut self, of: &'a [T]) -> &'a T {
        &of[self.below(of.len() as u64) as usize]
    }
}

fn id(s: &str) -> Id {
    Id::parse(s).unwrap()
}

/// The reasons the ledger is given; [`undeclared`] are not given (an undeclared name, or a declared name of the wrong kind).
fn declared() -> Vec<Reason> {
    vec![Reason::Recipe(id("p:smelt")), Reason::Source(id("p:mine")), Reason::Sink(id("p:ship"))]
}

fn undeclared() -> Vec<Reason> {
    vec![Reason::Recipe(id("p:forge")), Reason::Source(id("p:ship")), Reason::Sink(id("p:mine")), Reason::Source(id("p:printer"))]
}

/// What the ledger promises, written as plainly as possible: per store a capacity and counts; per resource made minus destroyed.
#[derive(Clone, PartialEq, Debug)]
struct Model {
    stores: BTreeMap<String, (u32, BTreeMap<String, u32>)>,
    totals: BTreeMap<String, i64>,
}

impl Model {
    fn used(&self, s: &str) -> u32 {
        self.stores.get(s).map_or(0, |(_, items)| items.values().sum())
    }

    /// `tx` applied to a copy, or the code of the first op that fails.
    fn run(&self, tx: &Tx) -> Result<Model, &'static str> {
        let mut m = self.clone();
        let reasons = declared();
        for op in &tx.ops {
            let (res, consume, produce) = match op {
                Op::Move { res, .. } => (res, None, None),
                Op::Consume { res, why, .. } => (res, Some(why), None),
                Op::Produce { res, why, .. } => (res, None, Some(why)),
            };
            if res.as_str() == RESOURCES[2] {
                return Err(code::UNKNOWN);
            }
            for (why, wants_consume) in [(consume, true), (produce, false)] {
                if let Some(why) = why {
                    let kind_ok = matches!((why, wants_consume), (Reason::Recipe(_), _) | (Reason::Source(_), false) | (Reason::Sink(_), true));
                    if !kind_ok || !reasons.contains(why) {
                        return Err(code::DEPENDENCY);
                    }
                }
            }
            let (take, put, n) = match op {
                Op::Move { from, to, n, .. } => (Some(from), Some(to), *n),
                Op::Consume { from, n, .. } => (Some(from), None, *n),
                Op::Produce { to, n, .. } => (None, Some(to), *n),
            };
            for s in take.iter().chain(put.iter()) {
                if !m.stores.contains_key(s.as_str()) {
                    return Err(code::UNKNOWN);
                }
            }
            if let Some(from) = take {
                let items = &mut m.stores.get_mut(from.as_str()).unwrap().1;
                let have = items.get(res.as_str()).copied().unwrap_or(0);
                if have < n {
                    return Err(code::MISSING);
                }
                items.insert(res.to_string(), have - n);
                items.retain(|_, v| *v > 0);
            }
            if let Some(to) = put {
                let (cap, _) = m.stores[to.as_str()];
                if m.used(to.as_str()) + n > cap {
                    return Err(code::FULL);
                }
                let items = &mut m.stores.get_mut(to.as_str()).unwrap().1;
                *items.entry(res.to_string()).or_insert(0) += n;
                items.retain(|_, v| *v > 0);
            }
            let delta = match op {
                Op::Move { .. } => 0,
                Op::Consume { .. } => -(n as i64),
                Op::Produce { .. } => n as i64,
            };
            *m.totals.entry(res.to_string()).or_insert(0) += delta;
            m.totals.retain(|_, v| *v != 0);
        }
        Ok(m)
    }

    fn add_store(&self, s: &str, cap: u32) -> Result<Model, &'static str> {
        if self.stores.contains_key(s) {
            return Err(code::DUPLICATE);
        }
        let mut m = self.clone();
        m.stores.insert(s.to_string(), (cap, BTreeMap::new()));
        Ok(m)
    }

    fn remove_stores(&self, ids: &[&str], into: &str) -> Result<Model, &'static str> {
        if !self.stores.contains_key(into) {
            return Err(code::UNKNOWN);
        }
        let mut moving = 0;
        for (i, s) in ids.iter().enumerate() {
            if ids[..i].contains(s) || ids[i + 1..].contains(s) {
                return Err(code::DUPLICATE);
            }
            if *s == into {
                return Err(code::DEPENDENCY);
            }
            if !self.stores.contains_key(*s) {
                return Err(code::UNKNOWN);
            }
            moving += self.used(s);
        }
        let (cap, _) = self.stores[into];
        if self.used(into) + moving > cap {
            return Err(code::FULL);
        }
        let mut m = self.clone();
        for s in ids {
            let (_, items) = m.stores.remove(*s).unwrap();
            for (res, n) in items {
                *m.stores.get_mut(into).unwrap().1.entry(res).or_insert(0) += n;
            }
        }
        Ok(m)
    }

    /// The ledger seen through the model's eyes.
    fn of(l: &Ledger) -> Model {
        let st = l.state();
        Model {
            stores: st.stores.iter().map(|(k, s)| (k.to_string(), (s.capacity, s.items.iter().map(|(r, n)| (r.to_string(), *n)).collect()))).collect(),
            totals: st.totals.iter().map(|(r, n)| (r.to_string(), *n)).collect(),
        }
    }
}

/// One random op over the names above: mostly stores that exist, declared resources and reasons and amounts that fit (so about half the
/// transactions apply), the rest unknown stores and resources, undeclared reasons and amounts up to past capacity.
fn op(r: &mut Rng, m: &Model) -> Op {
    let live: Vec<&str> = m.stores.keys().map(String::as_str).collect();
    let store = |r: &mut Rng| if live.is_empty() || r.below(8) == 0 { id(r.pick(&STORES)) } else { id(r.pick(&live)) };
    let res = id(if r.below(16) == 0 { RESOURCES[2] } else { r.pick(&RESOURCES[..2]) });
    let n = if r.below(6) == 0 { r.below(14) } else { r.below(4) } as u32;
    let why = |r: &mut Rng| if r.below(8) == 0 { r.pick(&undeclared()).clone() } else { r.pick(&declared()).clone() };
    match r.below(3) {
        0 => Op::Move { from: store(r), to: store(r), res, n },
        1 => Op::Consume { from: store(r), res, n, why: why(r) },
        _ => Op::Produce { to: store(r), res, n, why: why(r) },
    }
}

fn start() -> (Ledger, Model) {
    let mut l = Ledger::new();
    for r in &RESOURCES[..2] {
        l.resource(id(r));
    }
    for why in declared() {
        l.declare(why);
    }
    let mut m = Model { stores: BTreeMap::new(), totals: BTreeMap::new() };
    for (s, cap) in STORES[..3].iter().zip([12, 6, 0]) {
        l.add_store(id(s), id("p:owner"), cap).unwrap();
        m = m.add_store(s, cap).unwrap();
    }
    (l, m)
}

#[test]
fn random_sequences_agree_with_the_model_audit_and_fail_without_change() {
    let (mut ok, mut refused, mut applied_after_check) = (0u32, 0u32, 0u32);
    for seed in SEEDS {
        let mut r = Rng(seed);
        let (mut l, mut m) = start();
        for step in 0..STEPS {
            let at = format!("seed {seed} step {step}");
            let (state, hash) = (l.state(), l.hash());
            let (got, want) = match r.below(10) {
                // A transaction (one op, or two to four a quarter of the time): checked, then applied.
                0..=6 => {
                    let len = if r.below(4) == 0 { 2 + r.below(3) } else { 1 };
                    let tx = Tx { ops: (0..len).map(|_| op(&mut r, &m)).collect() };
                    let want = m.run(&tx);
                    let check = l.check(&tx).map_err(|e| e.code);
                    assert_eq!(check, want.as_ref().map(|_| ()).map_err(|c| *c), "{at}: check disagrees with the model for {tx:?}");
                    assert_eq!((l.state(), l.hash()), (state.clone(), hash), "{at}: check changed the ledger");
                    let got = l.apply(&tx).map_err(|e| e.code);
                    if check.is_ok() {
                        assert_eq!(got, Ok(()), "{at}: checked Ok, then apply refused {tx:?}");
                        applied_after_check += 1;
                    }
                    (got, want)
                }
                7 => {
                    let (s, cap) = (*r.pick(&STORES), r.below(10) as u32);
                    (l.add_store(id(s), id("p:owner"), cap).map_err(|e| e.code), m.add_store(s, cap))
                }
                _ => {
                    let ids: Vec<&str> = (0..1 + r.below(3)).map(|_| *r.pick(&STORES)).collect();
                    let into = *r.pick(&STORES);
                    let names: Vec<Id> = ids.iter().map(|s| id(s)).collect();
                    (l.remove_stores(&names, &id(into)).map_err(|e| e.code), m.remove_stores(&ids, into))
                }
            };
            match (got, want) {
                (Ok(()), Ok(next)) => {
                    ok += 1;
                    m = next;
                }
                (Err(got), Err(want)) => {
                    refused += 1;
                    assert_eq!(got, want, "{at}: refused for another reason than the model's");
                    assert_eq!((l.state(), l.hash()), (state, hash), "{at}: a refused change changed the ledger");
                }
                (got, want) => panic!("{at}: the ledger says {got:?}, the model {:?}", want.map(|_| ())),
            }
            assert_eq!(Model::of(&l), m, "{at}: the ledger and the model hold different things");
            l.audit().unwrap_or_else(|e| panic!("{at}: {e}"));
        }
        // What it saved restores exactly into a fresh ledger with the same declarations.
        let (mut fresh, _) = start();
        fresh.restore(l.state()).unwrap();
        assert_eq!((fresh.state(), fresh.hash()), (l.state(), l.hash()), "seed {seed}: restore");
    }
    eprintln!("ledger props: {ok} accepted, {refused} refused, {applied_after_check} applied after a successful check");
    // The generator must exercise both outcomes in earnest, or the property is vacuous.
    let total = (SEEDS.end - SEEDS.start) as u32 * STEPS as u32;
    assert!(ok > total / 10 && refused > total / 10, "accepted {ok}, refused {refused} of {total}");
    assert!(applied_after_check > total / 10, "apply after a successful check: {applied_after_check}");
}
