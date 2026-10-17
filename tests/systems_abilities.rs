//! A reuse test of the game-system primitive (`sim::systems`, `sim::ledger`, `sim::defs`; ADR
//! 2026-10-14-game-systems-compose-under-one-match-authority) on something that is not a workshop: abilities with a mana cost and a cooldown,
//! one instance per caster and ability, and a mana regeneration system. No engine change was needed for it.

use red_engine2::sim::defs::{code, Defs, Diagnostic, Id, Provenance};
use red_engine2::sim::ledger::{Ledger, Reason, Tx};
use red_engine2::sim::systems::{Command, Cx, Event, Inspection, Look, System, Systems};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

fn id(s: &str) -> Id {
    Id::parse(s).unwrap()
}

/// An ability's definition.
#[derive(Clone)]
struct Ability {
    cost: u32,
    cooldown: u32,
}

/// Mana comes back one point every 30 ticks, into each caster's own pool, as far as it fits.
struct Regen {
    casters: Vec<Id>,
}

impl System for Regen {
    fn id(&self) -> Id {
        id("ab:regen")
    }
    fn tick(&mut self, cx: &mut Cx) {
        if cx.tick.is_multiple_of(30) {
            for c in &self.casters {
                let pool = id(&format!("{c}/mana"));
                if cx.ledger.free(&pool) > 0 {
                    let _ = cx.ledger.apply(&Tx::new().producing(&pool, &id("ab:mana"), 1, &Reason::Source(id("ab:regen"))));
                }
            }
        }
    }
}

/// Every caster's abilities: a cooldown each (isolated per caster and ability).
struct Abilities {
    defs: Arc<Defs<Ability>>,
    cooldowns: BTreeMap<Id, u32>,
}

fn caster_of(instance: &Id) -> Id {
    id(instance.as_str().split('/').next().unwrap())
}

fn ability_of(instance: &Id) -> Id {
    id(&format!("ab:{}", instance.as_str().rsplit('/').next().unwrap()))
}

impl Abilities {
    fn why_not(&self, instance: &Id, ledger: &Ledger) -> Option<Diagnostic> {
        let left = *self.cooldowns.get(instance)?;
        if left > 0 {
            return Some(Diagnostic::new("invalid", instance, format!("cooling down: {left} ticks left")).fix(format!("cast again in {left} ticks")));
        }
        let a = self.defs.get(&ability_of(instance))?;
        ledger.check(&Tx::new().consuming(&id(&format!("{}/mana", caster_of(instance))), &id("ab:mana"), a.cost, &Reason::Sink(id("ab:cast")))).err()
    }
}

impl System for Abilities {
    fn id(&self) -> Id {
        id("ab:abilities")
    }
    fn after(&self) -> Vec<Id> {
        vec![id("ab:regen")]
    }
    fn emits(&self) -> Vec<Id> {
        vec![id("ab:cast")]
    }
    fn commands(&self) -> Vec<Id> {
        vec![id("ab:cast")]
    }
    fn instances(&self) -> Vec<Id> {
        self.cooldowns.keys().cloned().collect()
    }
    fn tick(&mut self, _cx: &mut Cx) {
        for left in self.cooldowns.values_mut() {
            *left = left.saturating_sub(1);
        }
    }
    fn command(&mut self, c: &Command, cx: &mut Cx) -> Result<(), Diagnostic> {
        let inst = c.subject.ok_or_else(|| Diagnostic::new("missing-field", c.name, "names the caster's ability"))?;
        if !self.cooldowns.contains_key(&inst) {
            return Err(Diagnostic::new(code::UNKNOWN, inst, "no such ability"));
        }
        if let Some(why) = self.why_not(&inst, cx.ledger) {
            return Err(why);
        }
        let a = self.defs.get(&ability_of(&inst)).cloned().ok_or_else(|| Diagnostic::new(code::UNKNOWN, inst, "no definition"))?;
        cx.ledger.apply(&Tx::new().consuming(&id(&format!("{}/mana", caster_of(&inst))), &id("ab:mana"), a.cost, &Reason::Sink(id("ab:cast"))))?;
        self.cooldowns.insert(inst, a.cooldown);
        cx.emit(&id("ab:cast"), Some(&inst), a.cost as i64)
    }
    fn save(&self) -> Value {
        json!(self.cooldowns)
    }
    fn load(&mut self, state: Value) -> Result<(), String> {
        self.cooldowns = serde_json::from_value(state).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn hash(&self) -> u64 {
        self.cooldowns.iter().fold(0u64, |h, (k, v)| h.wrapping_mul(31).wrapping_add(k.as_str().len() as u64 * 1000 + *v as u64))
    }
    fn inspect(&self, target: &Id, look: &Look) -> Option<Inspection> {
        let left = *self.cooldowns.get(target)?;
        let blocked = self.why_not(target, look.ledger);
        let ability = ability_of(target);
        Some(Inspection {
            id: *target,
            system: self.id(),
            instance_at: None,
            definition: Some(ability),
            defined_at: self.defs.provenance(&ability).cloned(),
            status: if blocked.is_some() { "blocked" } else { "ready" }.into(),
            blocked_by: blocked,
            state: json!({"cooldown_left": left}),
        })
    }
}

/// Counts the casts it hears (an event consumer).
struct Log {
    casts: u64,
}

impl System for Log {
    fn id(&self) -> Id {
        id("ab:log")
    }
    fn hears(&self) -> Vec<Id> {
        vec![id("ab:cast")]
    }
    fn tick(&mut self, _cx: &mut Cx) {}
    fn event(&mut self, _e: &Event, _cx: &mut Cx) {
        self.casts += 1;
    }
    fn save(&self) -> Value {
        json!(self.casts)
    }
    fn load(&mut self, state: Value) -> Result<(), String> {
        self.casts = state.as_u64().ok_or("not a count")?;
        Ok(())
    }
    fn hash(&self) -> u64 {
        self.casts
    }
}

fn host() -> Systems {
    let mut defs = Defs::new();
    defs.define(id("ab:fireball"), Ability { cost: 5, cooldown: 120 }, Provenance::new("abilities.json", "/abilities/0")).unwrap();
    defs.define(id("ab:blink"), Ability { cost: 2, cooldown: 30 }, Provenance::new("abilities.json", "/abilities/1")).unwrap();
    let defs = Arc::new(defs);
    let casters = vec![id("ab:alice"), id("ab:bob")];
    let mut ledger = Ledger::new();
    ledger.resource(id("ab:mana"));
    for why in [Reason::Source(id("ab:regen")), Reason::Sink(id("ab:cast")), Reason::Source(id("ab:start"))] {
        ledger.declare(why);
    }
    let mut cooldowns = BTreeMap::new();
    for c in &casters {
        let pool = id(&format!("{c}/mana"));
        ledger.add_store(pool, *c, 10).unwrap();
        ledger.apply(&Tx::new().producing(&pool, &id("ab:mana"), 6, &Reason::Source(id("ab:start")))).unwrap();
        for (a, _) in defs.iter() {
            cooldowns.insert(id(&format!("{c}/{}", a.name())), 0);
        }
    }
    let systems: Vec<Box<dyn System>> = vec![Box::new(Log { casts: 0 }), Box::new(Abilities { defs, cooldowns }), Box::new(Regen { casters })];
    Systems::new(systems, ledger, 3).unwrap()
}

fn cast(who: &str) -> Command {
    Command { name: id("ab:cast"), subject: Some(id(who)), value: 0 }
}

#[test]
fn a_cast_costs_mana_once_and_then_cools_down_per_caster() {
    let mut s = host();
    s.tick().unwrap();
    s.command(&cast("ab:alice/fireball")).unwrap();
    // The same command again in the same tick: refused (cooling down), so the mana is spent once.
    let e = s.command(&cast("ab:alice/fireball")).unwrap_err();
    assert!(e.message.contains("cooling down: 120 ticks left"), "{e}");
    assert_eq!(s.ledger().count(&id("ab:alice/mana"), &id("ab:mana")), 1);
    // Bob's fireball and Alice's blink have their own state.
    s.command(&cast("ab:bob/fireball")).unwrap();
    let e = s.command(&cast("ab:alice/blink")).unwrap_err();
    assert_eq!((e.code, e.fix.as_deref()), (code::MISSING, Some("bring 1 more `ab:mana`")), "{e}");
    s.tick().unwrap();
    let casts = s.recent_events().filter(|e| e.name == id("ab:cast")).count();
    assert_eq!(casts, 2);
    s.ledger().audit().unwrap();
}

#[test]
fn an_inspection_says_why_an_ability_cannot_be_cast_and_where_it_is_defined() {
    let mut s = host();
    s.command(&cast("ab:alice/fireball")).unwrap();
    let i = s.inspect("ab:alice/fireball").unwrap();
    assert_eq!((i.status.as_str(), i.definition.as_ref().unwrap().as_str()), ("blocked", "ab:fireball"));
    assert_eq!(i.defined_at.unwrap().to_string(), "abilities.json#/abilities/0");
    assert_eq!(i.blocked_by.unwrap().fix.as_deref(), Some("cast again in 120 ticks"));
    assert_eq!(s.inspect("ab:bob/fireball").unwrap().status, "ready");
    // Cooldown over, mana regained: castable again.
    for _ in 0..150 {
        s.tick().unwrap();
    }
    assert_eq!(s.inspect("ab:alice/fireball").unwrap().status, "ready");
    s.command(&cast("ab:alice/fireball")).unwrap();
}

#[test]
fn a_save_mid_cooldown_reloads_into_the_same_future() {
    let mut a = host();
    a.command(&cast("ab:alice/fireball")).unwrap();
    for _ in 0..47 {
        a.tick().unwrap();
    }
    let saved = a.save();
    let mut b = host();
    b.load(&saved).unwrap();
    for s in [&mut a, &mut b] {
        for t in 0..300 {
            s.tick().unwrap();
            let _ = s.command(&cast(if t % 2 == 0 { "ab:alice/blink" } else { "ab:bob/fireball" }));
        }
    }
    assert_eq!(a.hash(), b.hash());
    assert_eq!(a.save(), b.save());
    assert_eq!(a.get::<Log>().unwrap().casts, b.get::<Log>().unwrap().casts);
}
