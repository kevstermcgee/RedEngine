//! Game rules as **data**: the `vars` and `rules` a scene declares, parsed and validated into a [`RuleSet`].
//!
//! ```json
//! "vars": { "score": 0, "has_key": false },
//! "rules": [
//!   { "id": "take_coin_1", "when": { "enter": { "object": "coin_1", "pad": 0.5 } }, "once": true,
//!     "do": [ { "add": ["score", 1] }, { "hide": "coin_1" }, { "emit": "coin" } ] },
//!   { "id": "exit_opens", "when": { "enter": { "zone": "exit" } }, "if": "score >= 3",
//!     "do": [ { "emit": "victory" }, { "end": "victory" } ] }
//! ]
//! ```
//!
//! A rule fires **when** something happens (a player enters/exits a volume, an event is emitted, a timer
//! elapses, the match starts), optionally for one kind of player (`who`), if a condition over the variables holds
//! (`if`, see [`super::rules_expr`]), at most once (`once`) or with a `cooldown`, and then runs its `do` actions
//! in order. Everything a rule refers to — variables, objects, zones, spawn points — is checked when the scene is
//! parsed, so `red_engine2 validate` reports `rules[1] (exit_opens).if: unknown variable ...` instead of a rule
//! that silently never fires. Running the rules is [`super::rules_run`]; the whole layer is headless and
//! deterministic (it is part of [`super::match_sim::MatchSim`] and of its checksum).
//!
//! Actions: `set [var, value]`, `add [var, n]`, `emit name`, `hide id`, `show id`, `collision [id, bool]`,
//! `teleport [x,y,z] | spawn_id`, `end reason`, `impulse {object, dir, speed}`.

use super::clock::secs_to_ticks;
use super::rules_expr::{self, Expr, Op};
use crate::strict::check_keys;
use glam::Vec3;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// Variables every rule can read but not set: seconds elapsed, ticks elapsed, connected players.
pub const BUILTIN_VARS: &[&str] = &["time", "tick", "players"];

/// Events the engine itself raises (a rule can react with `when: {event: name}` without any rule emitting them):
/// `pickup` / `drop` (a player took / released a prop), `shot` (a revolver was fired), `hit` (a player was damaged),
/// `kill` (a player was killed; the player is the killer), `respawn` (a dead player came back).
pub const ENGINE_EVENTS: &[&str] = &["pickup", "drop", "shot", "hit", "kill", "respawn"];

/// The action names, for error messages and `describe rules`.
pub const ACTIONS: &[(&str, &str)] = &[
    ("set", "[var, value]  set a variable to a number, true/false, or an expression string"),
    ("add", "[var, n]      add a number (or expression) to a variable"),
    ("emit", "name         record a game event; other rules can react with `when: {event: name}`"),
    ("hide", "object_id    mark an object hidden (state only: renderers/clients decide what that means)"),
    ("show", "object_id    the opposite of hide"),
    ("collision", "[object_id, bool]   enable/disable a top-level object's collision"),
    ("teleport", "[x,y,z] | spawn_id   move the player that triggered the rule"),
    ("end", "reason        end the match with this outcome; rules stop firing"),
    ("impulse", "{object, dir:[x,y,z], speed}   shove a loose prop (a physics prop), speed in m/s"),
];

/// An axis-aligned box in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Volume {
    /// Lowest corner.
    pub min: Vec3,
    /// Highest corner.
    pub max: Vec3,
}

/// Which players a rule applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// Everyone.
    Any,
    /// Human players only.
    Human,
    /// Rat players only.
    Rat,
}

/// What makes a rule fire.
#[derive(Debug, Clone, PartialEq)]
pub enum When {
    /// Once, on the first tick of the match.
    Start,
    /// A player steps into the volume.
    Enter(Volume),
    /// A player leaves the volume.
    Exit(Volume),
    /// An `emit` action produced this event name.
    Event(String),
    /// Every this many ticks.
    Every(u64),
    /// Once, this many ticks after the start.
    After(u64),
}

/// Where a `teleport` sends the player.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A world position (feet).
    Point(Vec3),
    /// A spawn point of the scene, by id.
    Spawn(String),
}

/// One thing a rule does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// `var = value` (an `add` is compiled to a `Set` of `var + n`).
    Set {
        /// Variable index in [`RuleSet::var_names`].
        var: usize,
        /// The new value.
        value: Expr,
    },
    /// Record an event.
    Emit(String),
    /// Mark an object hidden.
    Hide(String),
    /// Mark an object shown.
    Show(String),
    /// Enable or disable a top-level object's collision.
    Collision {
        /// Top-level object id.
        object: String,
        /// Whether it should collide.
        enabled: bool,
    },
    /// Move the triggering player.
    Teleport(Target),
    /// End the match with an outcome.
    End(String),
    /// Shove a loose prop.
    Impulse {
        /// Object id of the prop.
        object: String,
        /// Direction (normalised when applied).
        dir: Vec3,
        /// Speed given, m/s.
        speed: f32,
    },
}

/// One rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// Unique id (events and logs name it).
    pub id: String,
    /// The trigger.
    pub when: When,
    /// Which players.
    pub who: Who,
    /// Optional condition over the variables.
    pub cond: Option<Expr>,
    /// Fire at most once per match.
    pub once: bool,
    /// Minimum ticks between firings.
    pub cooldown_ticks: u64,
    /// What it does, in order.
    pub actions: Vec<Action>,
}

/// All of a scene's variables and rules. `Default` is "no rules".
#[derive(Debug, Clone, PartialEq)]
pub struct RuleSet {
    /// Variable names: the [`BUILTIN_VARS`] first, then the scene's `vars` in declaration order.
    pub var_names: Vec<String>,
    /// Initial value of every variable (built-ins start at 0).
    pub var_init: Vec<f64>,
    /// The rules, in evaluation order.
    pub rules: Vec<Rule>,
}

impl Default for RuleSet {
    fn default() -> Self {
        RuleSet { var_names: BUILTIN_VARS.iter().map(|s| s.to_string()).collect(), var_init: vec![0.0; BUILTIN_VARS.len()], rules: Vec::new() }
    }
}

/// What the parser may refer to, gathered from the rest of the scene.
#[derive(Default)]
pub struct Refs {
    /// Every object id (any depth), for `hide`/`show`.
    pub object_ids: HashSet<String>,
    /// Top-level object ids, for collision state changes.
    pub top_level_ids: HashSet<String>,
    /// Top-level objects and their world bounds, for `{object}` volumes and `impulse`.
    pub bounds: HashMap<String, (Vec3, Vec3)>,
    /// Zones: id to `(min, max)` where `min.y` is the floor height.
    pub zones: HashMap<String, (Vec3, Vec3)>,
    /// Spawn point ids, for `teleport`.
    pub spawn_ids: HashSet<String>,
}

const RULE_KEYS: &[&str] = &["id", "when", "who", "if", "once", "cooldown", "do"];
const WHEN_KEYS: &[&str] = &["start", "enter", "exit", "event", "every", "after"];
const VOLUME_KEYS: &[&str] = &["zone", "object", "box", "pad", "height"];
const ZONE_HEIGHT: f32 = 3.0;

fn at(i: usize, id: &str) -> String {
    if id.is_empty() {
        format!("rules[{i}]")
    } else {
        format!("rules[{i}] ({id})")
    }
}

fn near(name: &str, options: impl Iterator<Item = String>) -> String {
    let all: Vec<String> = options.collect();
    let hint = crate::prefabs::suggest(name, all.iter().map(String::as_str));
    match hint.first() {
        Some(h) => format!(" — did you mean `{h}`?"),
        None => String::new(),
    }
}

fn vec3(v: &Value) -> Option<Vec3> {
    let a = v.as_array().filter(|a| a.len() == 3)?;
    Some(Vec3::new(a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32))
}

fn secs(v: &Value, path: &str, errs: &mut Vec<String>) -> u64 {
    match v.as_f64() {
        Some(s) if s > 0.0 && s.is_finite() => secs_to_ticks(s as f32) as u64,
        _ => {
            errs.push(format!("{path}: must be a number of seconds greater than 0"));
            1
        }
    }
}

fn parse_volume(v: &Value, refs: &Refs, path: &str, errs: &mut Vec<String>) -> Option<Volume> {
    let Some(o) = v.as_object() else {
        errs.push(format!(
            "{path}: must be an object like {{\"zone\": \"kitchen\"}}, {{\"object\": \"coin_1\", \"pad\": 0.5}} or {{\"box\": [x0,y0,z0,x1,y1,z1]}}"
        ));
        return None;
    };
    check_keys(errs, path, o, VOLUME_KEYS);
    let pad = o.get("pad").and_then(Value::as_f64).unwrap_or(0.0).max(0.0) as f32;
    let kinds: Vec<&str> = ["zone", "object", "box"].into_iter().filter(|k| o.contains_key(*k)).collect();
    if kinds.len() != 1 {
        errs.push(format!(
            "{path}: give exactly one of `zone`, `object`, `box` (got {})",
            if kinds.is_empty() { "none".to_string() } else { kinds.join(" + ") }
        ));
        return None;
    }
    match kinds[0] {
        "zone" => {
            let id = o["zone"].as_str().unwrap_or("");
            let Some((lo, hi)) = refs.zones.get(id) else {
                errs.push(format!("{path}.zone: no zone `{id}`{} (zones: {})", near(id, refs.zones.keys().cloned()), sorted(refs.zones.keys())));
                return None;
            };
            let h = o.get("height").and_then(Value::as_f64).map_or(ZONE_HEIGHT, |h| h as f32);
            Some(Volume { min: *lo - Vec3::splat(pad), max: Vec3::new(hi.x, lo.y + h, hi.z) + Vec3::splat(pad) })
        }
        "object" => {
            let id = o["object"].as_str().unwrap_or("");
            let Some((lo, hi)) = refs.bounds.get(id) else {
                errs.push(format!("{path}.object: no top-level object `{id}`{} (a volume needs a top-level object id)", near(id, refs.bounds.keys().cloned())));
                return None;
            };
            Some(Volume { min: *lo - Vec3::splat(pad), max: *hi + Vec3::splat(pad) })
        }
        _ => {
            let b = o["box"].as_array().filter(|a| a.len() == 6).and_then(|a| a.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>());
            let Some(b) = b else {
                errs.push(format!("{path}.box: must be six numbers [x0, y0, z0, x1, y1, z1]"));
                return None;
            };
            let (p, q) = (Vec3::new(b[0] as f32, b[1] as f32, b[2] as f32), Vec3::new(b[3] as f32, b[4] as f32, b[5] as f32));
            Some(Volume { min: p.min(q) - Vec3::splat(pad), max: p.max(q) + Vec3::splat(pad) })
        }
    }
}

fn parse_when(v: &Value, refs: &Refs, path: &str, errs: &mut Vec<String>) -> Option<When> {
    let Some(o) = v.as_object() else {
        errs.push(format!("{path}: must be an object with one of {}", WHEN_KEYS.join(", ")));
        return None;
    };
    check_keys(errs, path, o, WHEN_KEYS);
    let present: Vec<&str> = WHEN_KEYS.iter().copied().filter(|k| o.contains_key(*k)).collect();
    if present.len() != 1 {
        errs.push(format!(
            "{path}: give exactly one of {} (got {})",
            WHEN_KEYS.join(", "),
            if present.is_empty() { "none".to_string() } else { present.join(" + ") }
        ));
        return None;
    }
    let key = present[0];
    let sub = format!("{path}.{key}");
    match key {
        "start" => Some(When::Start),
        "enter" => parse_volume(&o[key], refs, &sub, errs).map(When::Enter),
        "exit" => parse_volume(&o[key], refs, &sub, errs).map(When::Exit),
        "event" => match o[key].as_str().filter(|s| !s.is_empty()) {
            Some(name) => Some(When::Event(name.to_string())),
            None => {
                errs.push(format!("{sub}: must be an event name (a string)"));
                None
            }
        },
        "every" => Some(When::Every(secs(&o[key], &sub, errs))),
        _ => Some(When::After(secs(&o[key], &sub, errs))),
    }
}

/// A number, a bool, or an expression string, compiled against `names`.
fn parse_value(v: &Value, names: &[String], path: &str, errs: &mut Vec<String>) -> Option<Expr> {
    match v {
        Value::Number(n) => n.as_f64().map(Expr::Num),
        Value::Bool(x) => Some(Expr::Num(if *x { 1.0 } else { 0.0 })),
        Value::String(s) => match rules_expr::parse(s, names) {
            Ok(e) => Some(e),
            Err(e) => {
                errs.push(format!("{path}: {e}"));
                None
            }
        },
        _ => {
            errs.push(format!("{path}: must be a number, true/false or an expression string"));
            None
        }
    }
}

fn var_index(name: Option<&str>, names: &[String], path: &str, errs: &mut Vec<String>) -> Option<usize> {
    let name = name.unwrap_or("");
    match names.iter().position(|n| n == name) {
        Some(i) if i >= BUILTIN_VARS.len() => Some(i),
        Some(_) => {
            errs.push(format!("{path}: `{name}` is built in and read-only"));
            None
        }
        None => {
            errs.push(format!("{path}: {}", rules_expr::unknown_var(name, names)));
            None
        }
    }
}

fn parse_action(v: &Value, names: &[String], refs: &Refs, path: &str, errs: &mut Vec<String>) -> Option<Action> {
    let Some(o) = v.as_object() else {
        errs.push(format!("{path}: must be an object with one action, e.g. {{\"emit\": \"coin\"}}"));
        return None;
    };
    let keys: Vec<&String> = o.keys().filter(|k| !crate::strict::is_extension_key(k)).collect();
    let [key] = keys.as_slice() else {
        errs.push(format!("{path}: an action has exactly one key (one of {}); got {}", ACTIONS.iter().map(|a| a.0).collect::<Vec<_>>().join(", "), keys.len()));
        return None;
    };
    let sub = format!("{path}.{key}");
    let val = &o[key.as_str()];
    let pair = || val.as_array().filter(|a| a.len() == 2);
    match key.as_str() {
        "set" | "add" => {
            let Some(p) = pair() else {
                errs.push(format!("{sub}: must be [variable, value]"));
                return None;
            };
            let var = var_index(p[0].as_str(), names, &format!("{sub}[0]"), errs)?;
            let value = parse_value(&p[1], names, &format!("{sub}[1]"), errs)?;
            let value = if key.as_str() == "add" { Expr::Bin(Op::Add, Box::new(Expr::Var(var)), Box::new(value)) } else { value };
            Some(Action::Set { var, value })
        }
        "emit" | "end" => match val.as_str().filter(|s| !s.is_empty()) {
            Some(s) if key.as_str() == "emit" => Some(Action::Emit(s.to_string())),
            Some(s) => Some(Action::End(s.to_string())),
            None => {
                errs.push(format!("{sub}: must be a name (a string)"));
                None
            }
        },
        "hide" | "show" => {
            let id = val.as_str().unwrap_or("");
            if !refs.object_ids.contains(id) {
                errs.push(format!("{sub}: no object `{id}`{}", near(id, refs.object_ids.iter().cloned())));
                return None;
            }
            Some(if key.as_str() == "hide" { Action::Hide(id.to_string()) } else { Action::Show(id.to_string()) })
        }
        "collision" => {
            let Some(pair) = pair() else {
                errs.push(format!("{sub}: must be [object_id, true|false]"));
                return None;
            };
            let id = pair[0].as_str().unwrap_or("");
            if !refs.top_level_ids.contains(id) {
                errs.push(format!("{sub}[0]: no top-level object `{id}`{}", near(id, refs.top_level_ids.iter().cloned())));
                return None;
            }
            let Some(enabled) = pair[1].as_bool() else {
                errs.push(format!("{sub}[1]: must be true or false"));
                return None;
            };
            Some(Action::Collision { object: id.to_string(), enabled })
        }
        "teleport" => match (vec3(val), val.as_str()) {
            (Some(p), _) => Some(Action::Teleport(Target::Point(p))),
            (None, Some(s)) if refs.spawn_ids.contains(s) => Some(Action::Teleport(Target::Spawn(s.to_string()))),
            (None, Some(s)) => {
                errs.push(format!("{sub}: no spawn point `{s}`{} (spawns: {})", near(s, refs.spawn_ids.iter().cloned()), sorted(refs.spawn_ids.iter())));
                None
            }
            _ => {
                errs.push(format!("{sub}: must be [x, y, z] or a spawn point id"));
                None
            }
        },
        "impulse" => {
            let Some(io) = val.as_object() else {
                errs.push(format!("{sub}: must be {{\"object\": id, \"dir\": [x,y,z], \"speed\": n}}"));
                return None;
            };
            check_keys(errs, &sub, io, &["object", "dir", "speed"]);
            let object = io.get("object").and_then(Value::as_str).unwrap_or("");
            if !refs.bounds.contains_key(object) {
                errs.push(format!("{sub}.object: no top-level object `{object}`{}", near(object, refs.bounds.keys().cloned())));
                return None;
            }
            let Some(dir) = io.get("dir").and_then(vec3) else {
                errs.push(format!("{sub}.dir: must be [x, y, z]"));
                return None;
            };
            let speed = io.get("speed").and_then(Value::as_f64).unwrap_or(4.0) as f32;
            Some(Action::Impulse { object: object.to_string(), dir, speed })
        }
        other => {
            let names: Vec<String> = ACTIONS.iter().map(|a| a.0.to_string()).collect();
            errs.push(format!("{sub}: unknown action `{other}`{} (actions: {})", near(other, names.iter().cloned()), names.join(", ")));
            None
        }
    }
}

fn sorted<'a>(it: impl Iterator<Item = &'a String>) -> String {
    let mut v: Vec<&String> = it.collect();
    v.sort();
    v.iter().take(12).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
}

/// Parses the scene's `vars` and `rules` (both optional) against what the rest of the scene provides. Every
/// problem is reported, each prefixed `rules[i] (id).field`, so one `validate` shows them all.
pub fn parse_rules(root: &Map<String, Value>, refs: &Refs) -> Result<RuleSet, Vec<String>> {
    let mut errs = Vec::new();
    let mut set = RuleSet::default();
    if let Some(vars) = root.get("vars") {
        match vars.as_object() {
            None => errs.push("vars: must be an object like {\"score\": 0, \"has_key\": false}".to_string()),
            Some(o) => {
                for (name, v) in o.iter().filter(|(k, _)| !crate::strict::is_extension_key(k)) {
                    let ok_name = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.starts_with(|c: char| c.is_ascii_digit());
                    if !ok_name {
                        errs.push(format!("vars.{name}: a variable name is letters, digits and `_`, not starting with a digit"));
                    } else if BUILTIN_VARS.contains(&name.as_str()) || matches!(name.as_str(), "true" | "false") {
                        errs.push(format!("vars.{name}: `{name}` is reserved (built-ins: {})", BUILTIN_VARS.join(", ")));
                    }
                    let init = match v {
                        Value::Number(n) => n.as_f64(),
                        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                        _ => None,
                    };
                    match init {
                        Some(x) => {
                            set.var_names.push(name.clone());
                            set.var_init.push(x);
                        }
                        None => errs.push(format!("vars.{name}: must be a number or true/false")),
                    }
                }
            }
        }
    }
    let Some(list) = root.get("rules") else {
        return if errs.is_empty() { Ok(set) } else { Err(errs) };
    };
    let Some(list) = list.as_array() else {
        errs.push("rules: must be an array of rules".to_string());
        return Err(errs);
    };
    let mut seen = HashSet::new();
    for (i, rv) in list.iter().enumerate() {
        let id = rv.get("id").and_then(Value::as_str).unwrap_or("");
        let p = at(i, id);
        let Some(ro) = rv.as_object() else {
            errs.push(format!("{p}: must be an object"));
            continue;
        };
        check_keys(&mut errs, &p, ro, RULE_KEYS);
        if id.is_empty() {
            errs.push(format!("{p}.id: missing (every rule needs a unique id string)"));
        } else if !seen.insert(id.to_string()) {
            errs.push(format!("{p}.id: duplicate rule id `{id}`"));
        }
        let when = match ro.get("when") {
            Some(w) => parse_when(w, refs, &format!("{p}.when"), &mut errs),
            None => {
                errs.push(format!("{p}.when: missing (one of {})", WHEN_KEYS.join(", ")));
                None
            }
        };
        let who = match ro.get("who").map(|w| w.as_str().unwrap_or("")) {
            None | Some("any") => Who::Any,
            Some("human") => Who::Human,
            Some("rat") => Who::Rat,
            Some(other) => {
                errs.push(format!("{p}.who: `{other}` is not one of any, human, rat"));
                Who::Any
            }
        };
        let cond = ro.get("if").and_then(|c| parse_value(c, &set.var_names, &format!("{p}.if"), &mut errs));
        let cooldown_ticks = ro.get("cooldown").map_or(0, |c| secs(c, &format!("{p}.cooldown"), &mut errs));
        let once = ro.get("once").and_then(Value::as_bool).unwrap_or(false);
        let mut actions = Vec::new();
        match ro.get("do").and_then(Value::as_array) {
            Some(arr) if !arr.is_empty() => {
                for (k, av) in arr.iter().enumerate() {
                    if let Some(a) = parse_action(av, &set.var_names, refs, &format!("{p}.do[{k}]"), &mut errs) {
                        actions.push(a);
                    }
                }
            }
            _ => errs
                .push(format!("{p}.do: missing or empty (a rule needs at least one action: {})", ACTIONS.iter().map(|a| a.0).collect::<Vec<_>>().join(", "))),
        }
        if let Some(when) = when {
            set.rules.push(Rule { id: id.to_string(), when, who, cond, once, cooldown_ticks, actions });
        }
    }
    // A rule that reacts to an event nothing emits can never fire: say so (a typo in either place).
    let mut emitted: HashSet<&str> =
        set.rules.iter().flat_map(|r| r.actions.iter()).filter_map(|a| if let Action::Emit(n) = a { Some(n.as_str()) } else { None }).collect();
    emitted.extend(ENGINE_EVENTS.iter().copied());
    for (i, r) in set.rules.iter().enumerate() {
        if let When::Event(name) = &r.when {
            if !emitted.contains(name.as_str()) {
                errs.push(format!(
                    "{} .when.event: nothing emits `{name}`{} (events emitted: {})",
                    at(i, &r.id),
                    near(name, emitted.iter().map(|s| s.to_string())),
                    emitted.iter().copied().collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }
    if errs.is_empty() {
        Ok(set)
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn refs() -> Refs {
        let mut r = Refs::default();
        r.object_ids.extend(["coin_1".to_string(), "door".to_string()]);
        r.top_level_ids.extend(["coin_1".to_string(), "door".to_string()]);
        r.bounds.insert("coin_1".into(), (Vec3::new(1.0, 0.0, 1.0), Vec3::new(1.4, 0.4, 1.4)));
        r.zones.insert("exit".into(), (Vec3::new(8.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 2.0)));
        r.spawn_ids.insert("spawn_a".into());
        r
    }

    fn parse(v: Value) -> Result<RuleSet, Vec<String>> {
        parse_rules(v.as_object().unwrap(), &refs())
    }

    #[test]
    fn a_complete_rule_set_parses_into_resolved_volumes_and_indexed_variables() {
        let set = parse(json!({
            "vars": {"score": 0, "has_key": false},
            "rules": [
                {"id": "take", "when": {"enter": {"object": "coin_1", "pad": 0.5}}, "once": true,
                 "do": [{"add": ["score", 1]}, {"hide": "coin_1"}, {"emit": "coin"}]},
                {"id": "win", "when": {"enter": {"zone": "exit"}}, "if": "score >= 1 && !has_key", "do": [{"emit": "victory"}, {"end": "victory"}]},
                {"id": "react", "when": {"event": "coin"}, "who": "human", "cooldown": 0.5, "do": [{"set": ["has_key", true]}, {"teleport": "spawn_a"}]},
                {"id": "tick", "when": {"every": 2.0}, "do": [{"impulse": {"object": "coin_1", "dir": [0, 1, 0], "speed": 3}}]}
            ]
        }))
        .unwrap();
        assert_eq!(set.var_names, ["time", "tick", "players", "score", "has_key"]);
        assert_eq!(set.rules.len(), 4);
        let When::Enter(v) = &set.rules[0].when else { panic!() };
        assert_eq!(v.min, Vec3::new(0.5, -0.5, 0.5));
        assert_eq!(v.max, Vec3::new(1.9, 0.9, 1.9));
        let When::Enter(z) = &set.rules[1].when else { panic!() };
        assert_eq!((z.min.y, z.max.y), (0.0, ZONE_HEIGHT));
        assert_eq!(set.rules[2].cooldown_ticks, secs_to_ticks(0.5) as u64);
        assert_eq!(set.rules[2].who, Who::Human);
    }

    #[test]
    fn every_mistake_is_reported_with_its_path_and_a_fix() {
        let e = parse(json!({
            "vars": {"score": 0},
            "rules": [
                {"id": "a", "when": {"enter": {"zone": "exitt"}}, "if": "scor > 1", "do": [{"add": ["score", 1]}]},
                {"id": "b", "when": {"event": "nothing"}, "do": [{"hide": "coinn"}]},
                {"id": "c", "when": {"every": 0}, "do": [{"adds": ["score", 1]}]},
                {"id": "d", "wen": {}, "do": []},
                {"id": "e", "when": {"enter": {"zone": "exit"}, "exit": {"zone": "exit"}}, "do": [{"set": ["time", 1]}, {"teleport": "spawn_z"}]}
            ]
        }))
        .unwrap_err()
        .join("\n");
        for needle in [
            "rules[0] (a).when.enter.zone: no zone `exitt` — did you mean `exit`?",
            "rules[0] (a).if: unknown variable `scor` — did you mean `score`?",
            "rules[1] (b).do[0].hide: no object `coinn` — did you mean `coin_1`?",
            "nothing emits `nothing`",
            "rules[2] (c).when.every: must be a number of seconds greater than 0",
            "rules[2] (c).do[0].adds: unknown action `adds` — did you mean `add`?",
            "rules[3] (d).wen: unknown field — did you mean `when`",
            "rules[3] (d).when: missing",
            "rules[3] (d).do: missing or empty",
            "rules[4] (e).when: give exactly one of",
            "rules[4] (e).do[0].set[0]: `time` is built in and read-only",
            "rules[4] (e).do[1].teleport: no spawn point `spawn_z` — did you mean `spawn_a`?",
        ] {
            assert!(e.contains(needle), "missing `{needle}` in:\n{e}");
        }
    }

    #[test]
    fn vars_are_validated() {
        let e = parse(json!({"vars": {"time": 1, "bad name": 2, "x": "s", "9a": 1}})).unwrap_err().join("\n");
        assert!(
            e.contains("vars.time: `time` is reserved") && e.contains("vars.bad name") && e.contains("vars.x: must be a number") && e.contains("vars.9a"),
            "{e}"
        );
    }

    #[test]
    fn no_rules_is_the_default_and_valid() {
        assert_eq!(parse(json!({})).unwrap(), RuleSet::default());
    }
}
