//! Restartable timers for the rules: `{"timer": [name, secs]}` starts (or restarts) one, `{"cancel": name}` stops it, and a rule with
//! `when: {"timer": name}` fires once when it runs out. A name with `self.` is the subject prop's own timer (one per loose prop), so a per-crate
//! fuse is one rule, not one per crate; a `for_each` template gives a static object one each (`crumble_$tile`, `sim::rule_groups`).
//!
//! `every` fires forever and `after` once per match, so "two seconds after the player last stepped here" used to be a polling rule and a
//! timestamp variable per thing (Idea Forge `no-per-entity-timers`, 5 runs). Timers are rule state: the match checksum holds them (only in a scene
//! that has any, so older checksums and traces are unchanged); clients see what the rules do with them, never the timers, so the wire is
//! unchanged (ADR 2026-10-17-rule-timers-and-prop-variables-are-server-state).

use super::clock::TICK_RATE_HZ;
use serde_json::Value;

/// A timer a rule names: one of the scene's (index into [`super::rules::RuleSet::timers`]) or every loose prop's own copy of one
/// (index into [`super::rules::RuleSet::prop_timers`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerRef {
    /// `name`.
    Global(usize),
    /// `self.name`: the subject prop's own.
    Prop(usize),
}

/// The timers a scene's rules name, in the order first named: `(scene timers, per-prop timers without the "self." prefix)`, with an error for a
/// timer only started (nothing waits for it) or only waited for (nothing starts it), each with a did-you-mean.
pub fn names(rules: Option<&Value>) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut started: Vec<String> = Vec::new();
    // (name, where), for the messages.
    let mut cancelled: Vec<(String, String)> = Vec::new();
    let mut waited: Vec<(String, String)> = Vec::new();
    for (i, r) in rules.and_then(Value::as_array).into_iter().flatten().enumerate() {
        if let Some(n) = r.get("when").and_then(|w| w.get("timer")).and_then(Value::as_str) {
            waited.push((n.to_string(), format!("rules[{i}].when.timer")));
        }
        for (k, a) in r.get("do").and_then(Value::as_array).into_iter().flatten().enumerate() {
            if let Some(n) = a.get("timer").and_then(Value::as_array).and_then(|p| p.first()).and_then(Value::as_str) {
                if !started.iter().any(|s| s == n) {
                    started.push(n.to_string());
                }
            }
            if let Some(n) = a.get("cancel").and_then(Value::as_str) {
                cancelled.push((n.to_string(), format!("rules[{i}].do[{k}].cancel")));
            }
        }
    }
    let mut all: Vec<String> = started.clone();
    for (n, _) in waited.iter().chain(&cancelled) {
        if !all.contains(n) {
            all.push(n.clone());
        }
    }
    let mut errs = Vec::new();
    for (n, at) in waited.iter().chain(&cancelled) {
        if !started.contains(n) {
            let hint = crate::suggest::suggest(n, started.iter().map(String::as_str)).first().map(|h| format!(" — did you mean `{h}`?")).unwrap_or_default();
            errs.push(format!("{at}: no rule starts the timer `{n}`{hint} (start it with {{\"timer\": [\"{n}\", seconds]}})"));
        }
    }
    for n in &started {
        if !waited.iter().any(|(w, _)| w == n) {
            errs.push(format!("rules: the timer `{n}` is started but no rule waits for it (`\"when\": {{\"timer\": \"{n}\"}}`)"));
        }
    }
    let (props, global): (Vec<String>, Vec<String>) = all.into_iter().partition(|n| n.starts_with("self."));
    (global, props.into_iter().map(|n| n["self.".len()..].to_string()).collect(), errs)
}

/// Whether `name` (after an optional `self.`) is a timer name: letters, digits and `_`, not starting with a digit.
pub fn valid(name: &str) -> bool {
    let n = name.strip_prefix("self.").unwrap_or(name);
    n.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && n.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Every timer of a match: when each runs out (`None` = not running) and the tick each last ran out (so every rule waiting for it fires that tick,
/// even one after a rule that restarted it). Scene timers first, then each loose prop's own, prop by prop.
#[derive(Debug, Clone, Default)]
pub struct Timers {
    global: usize,
    per_prop: usize,
    deadline: Vec<Option<u64>>,
    rang: Vec<u64>,
}

impl Timers {
    /// `global` scene timers and `per_prop` timers for each of `props` loose props, none running.
    pub fn new(global: usize, per_prop: usize, props: usize) -> Self {
        let n = global + per_prop * props;
        Timers { global, per_prop, deadline: vec![None; n], rang: vec![u64::MAX; n] }
    }

    fn slot(&self, t: TimerRef, prop: Option<usize>) -> Option<usize> {
        let s = match t {
            TimerRef::Global(i) => i,
            TimerRef::Prop(i) => self.global + prop? * self.per_prop + i,
        };
        (s < self.deadline.len()).then_some(s)
    }

    /// Starts (or restarts) `t` (for the prop at set index `prop`, a per-prop timer) to run out `secs` from `tick`: at least one tick later, so a
    /// timer restarted by its own rule fires again on a later tick, never in a loop. A negative or non-finite `secs` counts as 0.
    pub fn start(&mut self, t: TimerRef, prop: Option<usize>, tick: u64, secs: f64) {
        let ticks = if secs.is_finite() { (secs.max(0.0) * TICK_RATE_HZ as f64).round() as u64 } else { 0 };
        if let Some(s) = self.slot(t, prop) {
            self.deadline[s] = Some(tick + ticks.max(1));
        }
    }

    /// Stops `t` before it runs out (nothing happens when it is not running).
    pub fn cancel(&mut self, t: TimerRef, prop: Option<usize>) {
        if let Some(s) = self.slot(t, prop) {
            self.deadline[s] = None;
        }
    }

    /// At the start of `tick`: every running timer due by now runs out (it stops, and [`rang`](Self::rang) is true for it this tick).
    pub fn advance(&mut self, tick: u64) {
        for (d, r) in self.deadline.iter_mut().zip(self.rang.iter_mut()) {
            if d.is_some_and(|d| d <= tick) {
                *d = None;
                *r = tick;
            }
        }
    }

    /// Whether `t` ran out at the start of `tick`.
    pub fn rang(&self, t: TimerRef, prop: Option<usize>, tick: u64) -> bool {
        self.slot(t, prop).is_some_and(|s| self.rang[s] == tick)
    }

    /// The props (set indices) whose own `t` ran out at the start of `tick`, in prop order.
    pub fn rang_props(&self, i: usize, tick: u64) -> impl Iterator<Item = usize> + '_ {
        let props = (self.deadline.len() - self.global).checked_div(self.per_prop).unwrap_or(0);
        (0..props).filter(move |&p| self.rang(TimerRef::Prop(i), Some(p), tick))
    }

    /// Seconds until `t` runs out at `tick`, or `None` when it is not running.
    pub fn left(&self, t: TimerRef, prop: Option<usize>, tick: u64) -> Option<f64> {
        let d = self.deadline[self.slot(t, prop)?]?;
        Some(d.saturating_sub(tick) as f64 / TICK_RATE_HZ as f64)
    }

    /// Folds the running timers into a checksum (nothing for a scene without timers).
    pub fn fold(&self, mut mix: impl FnMut(u64)) {
        for d in &self.deadline {
            mix(d.unwrap_or(u64::MAX));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Character;
    use crate::sim::rules::{parse_rules, Refs};
    use crate::sim::rules_run::{RulePlayer, RuleProp, RulesEngine};
    use glam::Vec3;
    use serde_json::json;

    fn player(x: f32) -> RulePlayer {
        RulePlayer { slot: 0, pos: Vec3::new(x, 0.0, 0.5), radius: 0.3, height: 1.8, character: Character::Human, team: 0 }
    }

    fn prop(x: f32) -> RuleProp {
        RuleProp {
            origin: Vec3::new(x, 0.3, 0.5),
            at: Vec3::new(x, 0.3, 0.5),
            height: 0.5,
            tilt_deg: 0.0,
            held_by: None,
            mass: 5.0,
            moved: 0.0,
            vel: Vec3::ZERO,
        }
    }

    #[test]
    fn a_timer_restarted_by_each_step_runs_out_once_after_the_last_one() {
        let set = parse_rules(
            json!({"vars": {"steps": 0, "shut": 0}, "rules": [
                {"id": "step", "when": {"enter": {"box": [0, 0, 0, 1, 2, 1]}}, "do": [{"add": ["steps", 1]}, {"timer": ["close", 1]}]},
                {"id": "close", "when": {"timer": "close"}, "do": [{"add": ["shut", 1]}]},
                {"id": "also", "when": {"timer": "close"}, "do": [{"timer": ["close", "steps * 0"]}, {"cancel": "close"}]}
            ]})
            .as_object()
            .unwrap(),
            &Refs::default(),
        )
        .unwrap();
        let mut e = RulesEngine::new(set);
        let walk = |e: &mut RulesEngine, from: u64, to: u64, x: f32| (from..to).for_each(|t| drop(e.step(t, &[player(x)])));
        walk(&mut e, 0, 5, 5.0);
        walk(&mut e, 5, 10, 0.5); // on the plate at tick 5: the timer starts
        walk(&mut e, 10, 40, 5.0);
        walk(&mut e, 40, 45, 0.5); // back on at 40, before it ran out at 65: restarted
        walk(&mut e, 45, 99, 5.0);
        assert_eq!((e.var("steps"), e.var("shut")), (Some(2.0), Some(0.0)), "the first start was replaced");
        walk(&mut e, 99, 200, 5.0);
        assert_eq!(e.var("shut"), Some(1.0), "it ran out once, 60 ticks after the last step; `also` restarted then cancelled it in the same firing");
    }

    #[test]
    fn each_prop_keeps_its_own_variable_and_fuse() {
        let mut refs = Refs::default();
        refs.prop_ids.extend(["a".to_string(), "b".to_string()]);
        refs.object_ids.extend(["a".to_string(), "b".to_string()]);
        let set = parse_rules(
            json!({"vars": {"booms": 0}, "prop_vars": {"heat": 0}, "rules": [
                {"id": "warm", "when": {"prop_enter": {"box": [0, 0, 0, 1, 2, 1]}}, "do": [{"add": ["self.heat", 1]}, {"timer": ["self.fuse", "0.5 * self.heat"]}]},
                {"id": "boom", "when": {"timer": "self.fuse"}, "if": "self.heat >= 1", "do": [{"add": ["booms", 1]}, {"hide": "self"}]}
            ]})
            .as_object()
            .unwrap(),
            &refs,
        )
        .unwrap();
        let mut e = RulesEngine::new(set);
        e.bind_props(|id| match id {
            "a" => Some(0),
            "b" => Some(1),
            _ => None,
        });
        e.step_props(0, &[], &[prop(5.0), prop(5.0)]);
        e.step_props(1, &[], &[prop(5.0), prop(0.5)]); // b enters: heat 1, fuse 30 ticks
        for t in 2..31 {
            e.step_props(t, &[], &[prop(5.0), prop(0.5)]);
        }
        assert_eq!((e.prop_var("a", "heat"), e.prop_var("b", "heat"), e.var("booms")), (Some(0.0), Some(1.0), Some(0.0)));
        e.step_props(31, &[], &[prop(5.0), prop(0.5)]);
        assert_eq!(e.var("booms"), Some(1.0));
        assert_eq!(e.hidden().collect::<Vec<_>>(), ["b"], "`self` is the prop whose fuse ran out, not the other one");
    }

    #[test]
    fn a_timer_or_prop_variable_used_without_its_subject_or_partner_is_an_error() {
        let mut refs = Refs::default();
        refs.prop_ids.insert("a".to_string());
        let e = parse_rules(
            json!({"prop_vars": {"heat": 0, "9bad": 1}, "rules": [
                {"id": "s", "when": {"start": true}, "do": [{"timer": ["self.fuse", 1]}, {"timer": ["t", -1]}]},
                {"id": "f", "when": {"timer": "self.fuse"}, "do": [{"set": ["self.hot", 1]}]},
                {"id": "g", "when": {"timer": "t"}, "do": [{"add": ["self.heat", 1]}]}
            ]})
            .as_object()
            .unwrap(),
            &refs,
        )
        .unwrap_err()
        .join("\n");
        for want in [
            "prop_vars.9bad: a variable name is letters",
            "rules[0] (s): uses `self` but a `start` trigger has no prop",
            "rules[0] (s).do[1].timer[1]: must be 0 seconds or more (got -1)",
            "unknown per-prop variable `self.hot` — did you mean `self.heat`?",
            "rules[2] (g): uses `self` but a scene timer's trigger (only a `self.` timer belongs to a prop) has no prop",
        ] {
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
        let none = parse_rules(json!({"prop_vars": {"heat": 0}}).as_object().unwrap(), &Refs::default()).unwrap_err();
        assert!(none[0].contains("no loose props"), "{none:?}");
    }

    #[test]
    fn names_are_collected_in_order_and_a_lonely_timer_is_an_error() {
        let rules = json!([
            {"id": "a", "when": {"start": true}, "do": [{"timer": ["crumble", 2]}, {"timer": ["self.fuse", 1]}]},
            {"id": "b", "when": {"timer": "crumble"}, "do": [{"cancel": "fuse_typo"}]},
            {"id": "c", "when": {"timer": "self.fuse"}, "do": [{"emit": "boom"}]},
            {"id": "d", "when": {"timer": "crumbel"}, "do": [{"emit": "x"}]}
        ]);
        let (global, props, errs) = names(Some(&rules));
        assert_eq!(global, ["crumble", "crumbel", "fuse_typo"]);
        assert_eq!(props, ["fuse"]);
        let e = errs.join("\n");
        assert!(e.contains("rules[3].when.timer: no rule starts the timer `crumbel` — did you mean `crumble`?"), "{e}");
        assert!(e.contains("rules[1].do[0].cancel: no rule starts the timer `fuse_typo`"), "{e}");
        assert_eq!(errs.len(), 2, "{e}");
    }

    #[test]
    fn a_timer_runs_out_once_restarts_and_cancels() {
        let mut t = Timers::new(1, 1, 3);
        let g = TimerRef::Global(0);
        t.start(g, None, 10, 0.5);
        assert_eq!(t.left(g, None, 10), Some(0.5));
        t.advance(39);
        assert!(!t.rang(g, None, 39));
        t.start(g, None, 39, 0.5); // restarted before it ran out: the old deadline is gone
        t.advance(40);
        assert!(!t.rang(g, None, 40), "it was restarted at 39");
        t.advance(69);
        assert!(t.rang(g, None, 69) && t.left(g, None, 69).is_none(), "runs out 30 ticks after the restart, once");
        t.advance(70);
        assert!(!t.rang(g, None, 70));
        t.start(g, None, 70, 0.0);
        t.advance(71);
        assert!(t.rang(g, None, 71), "a zero-second timer runs out on the next tick, never the same one");
        t.start(g, None, 80, 1.0);
        t.cancel(g, None);
        t.advance(200);
        assert!(!t.rang(g, None, 200));
    }

    #[test]
    fn each_prop_has_its_own() {
        let mut t = Timers::new(0, 1, 3);
        let f = TimerRef::Prop(0);
        t.start(f, Some(2), 0, 1.0);
        t.start(f, Some(0), 0, 2.0);
        t.advance(60);
        assert_eq!(t.rang_props(0, 60).collect::<Vec<_>>(), [2]);
        t.advance(120);
        assert_eq!(t.rang_props(0, 120).collect::<Vec<_>>(), [0]);
        t.start(f, None, 0, 1.0); // no subject prop: nothing
        let mut folded = Vec::new();
        t.fold(|v| folded.push(v));
        assert_eq!(folded, [u64::MAX; 3]);
    }
}
