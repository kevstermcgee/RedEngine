//! The host's own tests: order, composition errors, cascades, saves and migration, queries and batches.
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

/// A purse: `t:coins` (an int query), `t:pay` takes `value` coins (refused when there are too few), `t:gift` adds them (unattended).
struct Purse {
    coins: i64,
}

impl System for Purse {
    fn id(&self) -> Id {
        id("t:purse")
    }
    fn commands(&self) -> Vec<Id> {
        vec![id("t:pay"), id("t:gift")]
    }
    fn unattended(&self) -> Vec<Id> {
        vec![id("t:gift")]
    }
    fn queries(&self) -> Vec<QueryDecl> {
        vec![QueryDecl::new(id("t:coins"), Ty::Int, "coins in the purse")]
    }
    fn query(&self, q: &Id, _look: &Look) -> Option<f64> {
        (q.as_str() == "t:coins").then_some(self.coins as f64)
    }
    fn tick(&mut self, _cx: &mut Cx) {}
    fn command(&mut self, c: &Command, _cx: &mut Cx) -> Result<(), Diagnostic> {
        match c.name.as_str() {
            "t:pay" if c.value > self.coins => Err(Diagnostic::new(code::MISSING, "t:purse", format!("{} coins, {} asked", self.coins, c.value))),
            "t:pay" => {
                self.coins -= c.value;
                Ok(())
            }
            _ => {
                self.coins += c.value;
                Ok(())
            }
        }
    }
    fn save(&self) -> Value {
        json!(self.coins)
    }
    fn load(&mut self, s: Value) -> Result<(), String> {
        self.coins = s.as_i64().ok_or("not a number")?;
        Ok(())
    }
    fn hash(&self) -> u64 {
        self.coins as u64
    }
}

/// Reads the purse and the scene, and on `t:buy` issues one batch of payments; remembers what the host said.
struct Shopper {
    seen: Vec<f64>,
    results: Vec<(u64, bool)>,
    reads: Vec<Id>,
}

impl System for Shopper {
    fn id(&self) -> Id {
        id("t:shopper")
    }
    fn reads(&self) -> Vec<Id> {
        self.reads.clone()
    }
    fn issues(&self) -> Vec<Id> {
        vec![id("t:pay")]
    }
    fn commands(&self) -> Vec<Id> {
        vec![id("t:buy")]
    }
    fn tick(&mut self, cx: &mut Cx) {
        self.seen = self.reads.iter().map(|r| cx.query(r).unwrap_or(-1.0)).collect();
    }
    fn command(&mut self, c: &Command, cx: &mut Cx) -> Result<(), Diagnostic> {
        // Two payments in one batch: `value` then `value` again.
        let pay = Command { name: id("t:pay"), subject: None, value: c.value };
        cx.issue(c.value as u64, vec![pay.clone(), pay])
    }
    fn settled(&mut self, tag: u64, result: Result<(), Diagnostic>, cx: &mut Cx) {
        self.results.push((tag, result.is_ok()));
        assert!(cx.issue(0, vec![]).is_ok(), "an empty batch is nothing");
        assert!(cx.issue(0, vec![Command { name: id("t:pay"), subject: None, value: 1 }]).is_err(), "no issuing while settling");
    }
}

struct Scene(f64);

impl Outside for Scene {
    fn query(&self, q: &Id) -> Option<f64> {
        (q.as_str() == "scene:tide").then_some(self.0)
    }
}

fn shop(coins: i64, reads: &[&str]) -> Result<Systems, Vec<Diagnostic>> {
    let shopper = Shopper { seen: vec![], results: vec![], reads: reads.iter().map(|r| id(r)).collect() };
    Systems::new(vec![Box::new(shopper), Box::new(Purse { coins })], Ledger::new(), 1)
}

#[test]
fn a_system_reads_another_s_declared_queries_and_the_match_s_variables() {
    let mut s = shop(5, &["t:coins", "scene:tide"]).unwrap();
    s.tick_with(&Scene(3.0)).unwrap();
    assert_eq!(s.get::<Shopper>().unwrap().seen, [5.0, 3.0]);
    s.tick().unwrap();
    assert_eq!(s.get::<Shopper>().unwrap().seen, [5.0, -1.0], "no outside: a scene read fails, it never reads as 0");
    assert_eq!(s.query(&id("t:coins"), &()), Some(5.0));
    assert_eq!(s.queries().iter().map(|q| (q.id.as_str(), q.ty)).collect::<Vec<_>>(), [("t:coins", Ty::Int)]);
    let info = s.catalog();
    assert_eq!(info.iter().find(|i| i.id.as_str() == "t:purse").unwrap().unattended, [id("t:gift")]);
    // A read nothing answers, an issued command nobody takes: refused when the host is built.
    let e = shop(5, &["t:wallet"]).unwrap_err();
    assert_eq!(e.iter().map(|d| d.code).collect::<Vec<_>>(), [code::QUERY], "{e:?}");
    let alone = Shopper { seen: vec![], results: vec![], reads: vec![] };
    let e = Systems::new(vec![Box::new(alone)], Ledger::new(), 1).unwrap_err();
    assert_eq!(e.iter().map(|d| d.code).collect::<Vec<_>>(), [code::EVENT], "{e:?}");
}

#[test]
fn a_batch_applies_whole_or_not_at_all_and_its_issuer_hears_which() {
    let mut s = shop(5, &[]).unwrap();
    let buy = |n| Command { name: id("t:buy"), subject: None, value: n };
    // 2 + 2 of 5: both apply.
    s.command(&buy(2)).unwrap();
    assert_eq!(s.query(&id("t:coins"), &()), Some(1.0));
    // 1 + 1 of 1: the second is refused, so the first is undone too.
    let before = s.hash();
    s.command(&buy(1)).unwrap();
    assert_eq!(s.query(&id("t:coins"), &()), Some(1.0), "no partial payment");
    assert_eq!(s.hash(), before, "the purse is exactly as it was");
    assert_eq!(s.get::<Shopper>().unwrap().results, [(2, true), (1, false)]);
}
