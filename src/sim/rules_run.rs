//! Running a [`RuleSet`]: the deterministic state machine behind a scene's `rules`.
//!
//! [`RulesEngine::step`] is called once per simulation tick with where every player is; it reports what the
//! world must do as [`Effect`]s (teleport a player, shove a prop) and keeps everything else — variables, which
//! rules have fired, hidden objects, the event log, whether the match ended — as plain data that
//! [`RulesEngine::checksum`] folds into the match checksum, so a replay that diverges in game logic is detected.
//! Nothing here knows about sockets, windows or physics: it is fed positions and returns effects.

use super::clock::TICK_RATE_HZ;
use super::rules::{Action, Rule, RuleSet, Target, Volume, When, Who};
use crate::player::Character;
use glam::Vec3;
use std::collections::{BTreeMap, BTreeSet};

/// Most events kept in [`RulesEngine::history`] (the oldest are dropped).
pub const MAX_HISTORY: usize = 4096;
/// How many times events may trigger further events within one tick (a rule that emits what it listens for stops here).
pub const MAX_CHAIN: usize = 4;

/// A player as the rules see them.
#[derive(Debug, Clone, Copy)]
pub struct RulePlayer {
    /// The player's slot.
    pub slot: usize,
    /// Feet position.
    pub pos: Vec3,
    /// Body radius, m.
    pub radius: f32,
    /// Body height, m.
    pub height: f32,
    /// Which body.
    pub character: Character,
}

/// Something that happened because a rule ran.
#[derive(Debug, Clone, PartialEq)]
pub struct GameEvent {
    /// The tick it happened on.
    pub tick: u64,
    /// The rule that emitted it.
    pub rule: String,
    /// The event name (`emit`), or `end:<reason>` when the match ended.
    pub name: String,
    /// The player that triggered the rule, if any.
    pub slot: Option<usize>,
}

/// A change the rules ask the world to make (the engine cannot do it itself).
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Move a player.
    Teleport {
        /// Which player.
        slot: usize,
        /// Where.
        target: Target,
    },
    /// Give a prop a shove.
    Impulse {
        /// Object id of the prop.
        object: String,
        /// Direction.
        dir: Vec3,
        /// Speed, m/s.
        speed: f32,
    },
}

/// The live state of a scene's rules. See the module docs.
#[derive(Debug, Clone)]
pub struct RulesEngine {
    set: RuleSet,
    vars: Vec<f64>,
    fired: Vec<bool>,
    next_ok: Vec<u64>,
    inside: BTreeMap<(usize, usize), bool>,
    hidden: BTreeSet<String>,
    collision_disabled: BTreeSet<String>,
    ended: Option<String>,
    started: bool,
    history: Vec<GameEvent>,
    dropped: u64,
    pending: Vec<GameEvent>,
    /// Engine-raised events waiting to trigger `when: {event}` rules on the next step.
    injected: Vec<(String, Option<usize>)>,
}

impl RulesEngine {
    /// Starts a match with `set`.
    pub fn new(set: RuleSet) -> Self {
        RulesEngine {
            vars: set.var_init.clone(),
            fired: vec![false; set.rules.len()],
            next_ok: vec![0; set.rules.len()],
            set,
            inside: BTreeMap::new(),
            hidden: BTreeSet::new(),
            collision_disabled: BTreeSet::new(),
            ended: None,
            started: false,
            history: Vec::new(),
            dropped: 0,
            pending: Vec::new(),
            injected: Vec::new(),
        }
    }

    /// True when the scene declared any rule (a match without rules never ends and has no events).
    pub fn has_rules(&self) -> bool {
        !self.set.rules.is_empty()
    }

    /// The outcome the match ended with (`end` action), if it has.
    pub fn ended(&self) -> Option<&str> {
        self.ended.as_deref()
    }

    /// The value of a variable by name (built-ins included).
    pub fn var(&self, name: &str) -> Option<f64> {
        self.set.var_names.iter().position(|n| n == name).map(|i| self.vars[i])
    }

    /// The scene's own variables (not the built-ins) as `(name, value)`.
    pub fn vars(&self) -> Vec<(&str, f64)> {
        self.set.var_names.iter().zip(&self.vars).skip(super::rules::BUILTIN_VARS.len()).map(|(n, v)| (n.as_str(), *v)).collect()
    }

    /// Ids of objects a rule has hidden.
    pub fn hidden(&self) -> impl Iterator<Item = &str> {
        self.hidden.iter().map(String::as_str)
    }

    /// Top-level objects whose authored static collision is currently disabled.
    pub fn collision_disabled(&self) -> impl Iterator<Item = &str> {
        self.collision_disabled.iter().map(String::as_str)
    }

    /// Every event so far (at most [`MAX_HISTORY`]; the oldest are dropped).
    pub fn history(&self) -> &[GameEvent] {
        &self.history
    }

    /// How many old events were dropped from [`RulesEngine::history`].
    pub fn dropped_events(&self) -> u64 {
        self.dropped
    }

    /// Records an event the *engine* raised (`pickup`, `hit`, `kill`, ...: see [`super::rules::ENGINE_EVENTS`]): it goes
    /// into the history/new-event lists like an `emit`, and any `when: {event}` rule reacts on the next [`step`](Self::step).
    pub fn inject(&mut self, tick: u64, name: &str, slot: Option<usize>) {
        self.record(tick, "engine", name.to_string(), slot);
        if self.injected.len() < 64 {
            self.injected.push((name.to_string(), slot));
        }
    }

    /// Events since the last call (a server logs and clears these each tick).
    pub fn take_new_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.pending)
    }

    fn inside(v: &Volume, p: &RulePlayer) -> bool {
        let cx = p.pos.x.clamp(v.min.x, v.max.x);
        let cz = p.pos.z.clamp(v.min.z, v.max.z);
        let (dx, dz) = (p.pos.x - cx, p.pos.z - cz);
        dx * dx + dz * dz <= p.radius * p.radius && p.pos.y + p.height >= v.min.y && p.pos.y <= v.max.y
    }

    fn who_ok(who: Who, c: Character) -> bool {
        matches!((who, c), (Who::Any, _) | (Who::Human, Character::Human) | (Who::Rat, Character::Rat))
    }

    fn record(&mut self, tick: u64, rule: &str, name: String, slot: Option<usize>) {
        let ev = GameEvent { tick, rule: rule.to_string(), name, slot };
        if self.history.len() >= MAX_HISTORY {
            self.history.remove(0);
            self.dropped += 1;
        }
        self.history.push(ev.clone());
        if self.pending.len() < 256 {
            self.pending.push(ev);
        }
    }

    /// Runs the actions of rule `ri` if its guards allow it. Emitted event names are pushed onto `queue`.
    fn fire(&mut self, ri: usize, tick: u64, slot: Option<usize>, queue: &mut Vec<(String, Option<usize>)>, effects: &mut Vec<Effect>) {
        let rule: &Rule = &self.set.rules[ri];
        if (rule.once && self.fired[ri]) || tick < self.next_ok[ri] || self.ended.is_some() {
            return;
        }
        if let Some(c) = &rule.cond {
            if !c.truthy(&self.vars) {
                return;
            }
        }
        self.fired[ri] = true;
        self.next_ok[ri] = tick + rule.cooldown_ticks;
        let id = rule.id.clone();
        let actions = rule.actions.clone();
        for a in actions {
            match a {
                Action::Set { var, value } => {
                    let v = value.eval(&self.vars);
                    if let Some(slot_v) = self.vars.get_mut(var) {
                        *slot_v = v;
                    }
                }
                Action::Emit(name) => {
                    self.record(tick, &id, name.clone(), slot);
                    queue.push((name, slot));
                }
                Action::Hide(o) => {
                    self.hidden.insert(o);
                }
                Action::Show(o) => {
                    self.hidden.remove(&o);
                }
                Action::Collision { object, enabled } => {
                    if enabled {
                        self.collision_disabled.remove(&object);
                    } else {
                        self.collision_disabled.insert(object);
                    }
                }
                Action::Teleport(target) => {
                    if let Some(slot) = slot {
                        effects.push(Effect::Teleport { slot, target });
                    }
                }
                Action::End(reason) => {
                    self.record(tick, &id, format!("end:{reason}"), slot);
                    self.ended = Some(reason);
                }
                Action::Impulse { object, dir, speed } => effects.push(Effect::Impulse { object, dir, speed }),
            }
        }
    }

    /// Advances the rules one tick (`tick` = ticks completed so far) given where the players are; returns the
    /// effects the world must apply. Deterministic: rules run in declaration order, players in slot order.
    pub fn step(&mut self, tick: u64, players: &[RulePlayer]) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.ended.is_some() || self.set.rules.is_empty() {
            self.injected.clear();
            return effects;
        }
        self.vars[0] = tick as f64 / TICK_RATE_HZ as f64;
        self.vars[1] = tick as f64;
        self.vars[2] = players.len() as f64;
        self.inside.retain(|(_, slot), _| players.iter().any(|p| p.slot == *slot));
        let mut queue: Vec<(String, Option<usize>)> = std::mem::take(&mut self.injected);
        for ri in 0..self.set.rules.len() {
            let (when, who) = (self.set.rules[ri].when.clone(), self.set.rules[ri].who);
            match when {
                When::Start => {
                    if !self.started {
                        self.fire(ri, tick, None, &mut queue, &mut effects);
                    }
                }
                When::Enter(v) | When::Exit(v) => {
                    let entering = matches!(self.set.rules[ri].when, When::Enter(_));
                    for p in players.iter().filter(|p| Self::who_ok(who, p.character)) {
                        let now = Self::inside(&v, p);
                        let was = self.inside.insert((ri, p.slot), now);
                        if let Some(was) = was {
                            if (entering && now && !was) || (!entering && !now && was) {
                                self.fire(ri, tick, Some(p.slot), &mut queue, &mut effects);
                            }
                        }
                    }
                }
                When::Every(n) => {
                    if tick > 0 && tick.is_multiple_of(n) {
                        self.fire(ri, tick, None, &mut queue, &mut effects);
                    }
                }
                When::After(n) => {
                    if tick == n {
                        self.fire(ri, tick, None, &mut queue, &mut effects);
                    }
                }
                When::Event(_) => {}
            }
        }
        self.started = true;
        for _ in 0..MAX_CHAIN {
            if queue.is_empty() {
                break;
            }
            let batch = std::mem::take(&mut queue);
            for (name, slot) in batch {
                for ri in 0..self.set.rules.len() {
                    let matches_event = matches!(&self.set.rules[ri].when, When::Event(n) if *n == name);
                    let who_ok = slot.is_none_or(|s| players.iter().find(|p| p.slot == s).is_none_or(|p| Self::who_ok(self.set.rules[ri].who, p.character)));
                    if matches_event && who_ok {
                        self.fire(ri, tick, slot, &mut queue, &mut effects);
                    }
                }
            }
        }
        effects
    }

    /// A 64-bit fold of all rule state (variables, fired flags, cooldowns, occupancy, hidden objects, outcome, event count).
    pub fn checksum(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for v in &self.vars {
            mix(v.to_bits());
        }
        for (f, n) in self.fired.iter().zip(&self.next_ok) {
            mix(*f as u64);
            mix(*n);
        }
        for ((r, s), v) in &self.inside {
            mix((*r as u64) << 32 | (*s as u64) << 1 | *v as u64);
        }
        for o in &self.hidden {
            o.bytes().for_each(|b| mix(b as u64));
            mix(0xff);
        }
        // Preserve pre-v5 checksums while no dynamic collision state exists, so old traces remain
        // replayable. The marker makes a non-empty collision set distinct from hidden-object data.
        if !self.collision_disabled.is_empty() {
            mix(0xfe);
            for o in &self.collision_disabled {
                o.bytes().for_each(|b| mix(b as u64));
                mix(0xff);
            }
        }
        if let Some(e) = &self.ended {
            e.bytes().for_each(|b| mix(b as u64));
        }
        mix(self.history.len() as u64 + self.dropped);
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::rules::{parse_rules, Refs};
    use serde_json::{json, Value};

    fn engine(v: Value) -> RulesEngine {
        let mut refs = Refs::default();
        refs.object_ids.insert("coin".into());
        refs.top_level_ids.insert("coin".into());
        refs.bounds.insert("coin".into(), (Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.4, 0.4, 0.4)));
        refs.zones.insert("exit".into(), (Vec3::new(8.0, 0.0, -1.0), Vec3::new(10.0, 0.0, 1.0)));
        refs.spawn_ids.insert("start".into());
        RulesEngine::new(parse_rules(v.as_object().unwrap_or_else(|| panic!("object")), &refs).unwrap_or_else(|e| panic!("{e:?}")))
    }

    fn at(slot: usize, x: f32, z: f32, c: Character) -> RulePlayer {
        RulePlayer { slot, pos: Vec3::new(x, 0.0, z), radius: 0.3, height: 1.8, character: c }
    }

    fn coin_game() -> RulesEngine {
        engine(json!({
            "vars": {"score": 0},
            "rules": [
                {"id": "take", "when": {"enter": {"object": "coin", "pad": 0.2}}, "once": true, "do": [{"add": ["score", 1]}, {"hide": "coin"}, {"emit": "coin"}]},
                {"id": "win", "when": {"enter": {"zone": "exit"}}, "if": "score >= 1", "do": [{"emit": "victory"}, {"end": "victory"}]}
            ]
        }))
    }

    #[test]
    fn walking_into_a_volume_fires_once_and_then_the_conditional_win_rule_ends_the_match() {
        let mut e = coin_game();
        let h = Character::Human;
        // Far from everything: nothing happens.
        for t in 1..=5 {
            assert!(e.step(t, &[at(0, -5.0, 0.0, h)]).is_empty());
        }
        assert_eq!(e.var("score"), Some(0.0));
        // Enter the coin: score 1, coin hidden, event logged. Standing in it does not fire again.
        e.step(6, &[at(0, 2.2, 0.2, h)]);
        e.step(7, &[at(0, 2.2, 0.2, h)]);
        assert_eq!(e.var("score"), Some(1.0));
        assert_eq!(e.hidden().collect::<Vec<_>>(), ["coin"]);
        assert_eq!(e.history().iter().filter(|x| x.name == "coin").count(), 1);
        // Leave and re-enter: `once` holds.
        e.step(8, &[at(0, 5.0, 0.0, h)]);
        e.step(9, &[at(0, 2.2, 0.2, h)]);
        assert_eq!(e.var("score"), Some(1.0));
        // The exit: conditional on score >= 1, ends the match.
        e.step(10, &[at(0, 6.0, 0.0, h)]);
        assert_eq!(e.ended(), None);
        e.step(11, &[at(0, 8.5, 0.0, h)]);
        assert_eq!(e.ended(), Some("victory"));
        let names: Vec<&str> = e.history().iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["coin", "victory", "end:victory"]);
        // Once ended, nothing runs any more.
        assert!(e.step(12, &[at(0, 2.2, 0.2, h)]).is_empty());
        assert_eq!(e.var("score"), Some(1.0));
    }

    #[test]
    fn the_exit_does_nothing_without_the_coin() {
        let mut e = coin_game();
        e.step(1, &[at(0, 5.0, 0.0, Character::Human)]);
        e.step(2, &[at(0, 8.5, 0.0, Character::Human)]);
        assert_eq!(e.ended(), None, "score 0: the `if` blocks the win");
    }

    #[test]
    fn who_filters_players_and_events_chain_within_a_tick() {
        let mut e = engine(json!({
            "vars": {"n": 0},
            "rules": [
                {"id": "a", "when": {"enter": {"zone": "exit"}}, "who": "rat", "do": [{"emit": "rat_in"}]},
                {"id": "b", "when": {"event": "rat_in"}, "do": [{"add": ["n", 10]}, {"emit": "second"}]},
                {"id": "c", "when": {"event": "second"}, "do": [{"add": ["n", 1]}, {"teleport": "start"}]}
            ]
        }));
        let far = [at(0, 0.0, 0.0, Character::Human), at(1, 0.0, 3.0, Character::Rat)];
        e.step(1, &far);
        let near_human = [at(0, 9.0, 0.0, Character::Human), at(1, 0.0, 3.0, Character::Rat)];
        e.step(2, &near_human);
        assert_eq!(e.var("n"), Some(0.0), "a human entering does not fire a rat rule");
        let near_rat = [at(0, 9.0, 0.0, Character::Human), at(1, 9.0, 0.0, Character::Rat)];
        let fx = e.step(3, &near_rat);
        assert_eq!(e.var("n"), Some(11.0), "rat_in -> second, both handled in the same tick");
        assert_eq!(fx, vec![Effect::Teleport { slot: 1, target: Target::Spawn("start".into()) }]);
    }

    #[test]
    fn timers_start_rules_cooldowns_and_exits_work() {
        let mut e = engine(json!({
            "vars": {"boot": 0, "tick_n": 0, "left": 0},
            "rules": [
                {"id": "s", "when": {"start": true}, "do": [{"set": ["boot", 1]}]},
                {"id": "t", "when": {"every": 0.5}, "do": [{"add": ["tick_n", 1]}]},
                {"id": "x", "when": {"exit": {"zone": "exit"}}, "do": [{"add": ["left", 1]}]},
                {"id": "cd", "when": {"every": 0.5}, "cooldown": 2.0, "do": [{"emit": "slow"}]}
            ]
        }));
        let h = Character::Human;
        for t in 1..=120 {
            let x = if (30..60).contains(&t) { 9.0 } else { 0.0 };
            e.step(t, &[at(0, x, 0.0, h)]);
        }
        assert_eq!(e.var("boot"), Some(1.0));
        assert_eq!(e.var("tick_n"), Some(4.0), "every 0.5 s over 2 s");
        assert_eq!(e.var("left"), Some(1.0), "one exit at tick 60");
        assert_eq!(e.history().iter().filter(|x| x.name == "slow").count(), 1, "the 2 s cooldown suppressed the rest");
        assert_eq!(e.var("time"), Some(2.0));
    }

    #[test]
    fn an_event_loop_is_bounded_and_state_is_checksummed() {
        let mut e = engine(json!({"vars": {"n": 0}, "rules": [
            {"id": "ping", "when": {"start": true}, "do": [{"emit": "p"}]},
            {"id": "loop", "when": {"event": "p"}, "do": [{"add": ["n", 1]}, {"emit": "p"}]}]}));
        e.step(1, &[]);
        assert_eq!(e.var("n"), Some(MAX_CHAIN as f64), "the chain stops at MAX_CHAIN instead of spinning");
        let (a, mut b) = (e.checksum(), coin_game());
        assert_ne!(a, b.checksum());
        let c1 = b.checksum();
        b.step(1, &[at(0, 2.2, 0.2, Character::Human)]);
        assert_eq!(b.step(2, &[at(0, 2.2, 0.2, Character::Human)]), vec![]);
        assert_ne!(c1, b.checksum(), "firing a rule changes the checksum");
        let (mut x, mut y) = (coin_game(), coin_game());
        for t in 1..50 {
            let p = [at(0, t as f32 * 0.2, 0.0, Character::Human)];
            x.step(t, &p);
            y.step(t, &p);
        }
        assert_eq!(x.checksum(), y.checksum(), "same inputs, same state");
    }
}
