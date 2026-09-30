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
//! `teleport [x,y,z] | spawn_id`, `end reason`, `impulse {object, dir, speed}`, `reset id | [ids] | {zone}`,
//! `place [id, [x,y,z]]`.
//!
//! Rules see **loose props** too (ADR 2026-09-29-prop-aware-rules-and-scenarios): triggers `{prop_enter: VOLUME}` /
//! `{prop_exit: VOLUME}` (any loose prop, or one named by `prop: id` next to it) and `{prop_below: [id, y]}`, and the
//! expression built-ins `prop_y(id)`, `tilt(id)`, `held(id)`, `mass(id)`, `moved(id)`, `props_in(zone)`, `in_zone(id, zone)` in `if` and values.

use super::clock::secs_to_ticks;
use super::rules_expr::{self, Expr, Op, Scope};
use crate::strict::check_keys;
use glam::Vec3;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// Variables every rule can read but not set: seconds elapsed, ticks elapsed, connected players.
pub const BUILTIN_VARS: &[&str] = &["time", "tick", "players"];

/// Events the engine itself raises (a rule can react with `when: {event: name}` without any rule emitting them):
/// `pickup` / `drop` (a player took / released a prop), `shot` (a firearm was fired), `hit` (a player was damaged),
/// `kill` (a player was killed; the player is the killer), `respawn` (a dead player came back), `swing` (a bat swing
/// started), `prop_hit` (a bat or a bullet struck a loose prop; the player is the striker).
pub const ENGINE_EVENTS: &[&str] = &["pickup", "drop", "shot", "hit", "kill", "respawn", "swing", "prop_hit"];

/// The action names, for error messages and `describe rules`.
pub const ACTIONS: &[(&str, &str)] = &[
    ("set", "[var, value]  set a variable to a number, true/false, or an expression string"),
    ("add", "[var, n]      add a number (or expression) to a variable"),
    ("emit", "name         record a game event; other rules can react with `when: {event: name}`"),
    ("hide", "object_id    mark an object hidden (state only: renderers/clients decide what that means)"),
    ("show", "object_id    the opposite of hide"),
    ("collision", "[object_id, bool]   enable/disable a top-level object's collision for PLAYERS (movement and standing); loose props still collide with it, and props resting on it are not woken"),
    ("deactivate", "object_id    hide a top-level object AND turn its collision off for players (an opened door, a dropped forcefield); loose props still collide with it and stay on it"),
    ("activate", "object_id    the opposite of deactivate: show it and turn its collision back on"),
    ("teleport", "[x,y,z] | spawn_id   move the player that triggered the rule"),
    ("end", "reason        end the match with this outcome; rules stop firing"),
    ("impulse", "{object, dir:[x,y,z], speed}   shove a loose prop (a physics prop), speed in m/s (props never exceed 14 m/s, so a larger speed saturates)"),
    ("reset", "id | [ids] | {zone: id}   put loose props back where the map placed them, at rest (a carried one is taken from its holder)"),
    ("place", "[id, [x,y,z]]   move a loose prop's origin to a point, upright as authored, at rest"),
];

/// An axis-aligned box in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Volume {
    /// Lowest corner.
    pub min: Vec3,
    /// Highest corner.
    pub max: Vec3,
}

/// A force volume acting on loose props (`fields` in the scene): a river current, a conveyor belt, a wind tunnel. Every tick a prop whose
/// origin is inside is pulled toward the field's target velocity, exponentially at `rate` per second, so the push is a *speed target*
/// (a prop cannot be accelerated past it, unlike a repeating `impulse` rule) and gravity, floors and walls still act.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// Its id (errors and `describe` name it).
    pub id: String,
    /// The volume props must be in (same forms as a rule volume).
    pub volume: Volume,
    /// Target horizontal velocity `[x, z]`, m/s; `None` leaves horizontal motion alone.
    pub velocity: Option<[f32; 2]>,
    /// Upward target speed, m/s: props slower than this upward are pulled up toward it (a wind tunnel, a hover pad); `None` = none.
    pub lift: Option<f32>,
    /// How hard the velocity is pulled toward its target, 1/s (default 10). It is a drag, not a teleport: a prop on a floor settles a bit under the
    /// target because floor friction (about 7 m/s^2) pulls back, so use a higher `rate` for a stronger belt.
    pub rate: f32,
}

impl Field {
    /// The velocity change to apply this tick to a prop moving at `vel`, for a step of `dt` seconds (zero once it is at the target).
    pub fn delta_v(&self, vel: Vec3, dt: f32) -> Vec3 {
        let k = (self.rate * dt).clamp(0.0, 1.0);
        let mut dv = Vec3::ZERO;
        if let Some([tx, tz]) = self.velocity {
            dv.x = (tx - vel.x) * k;
            dv.z = (tz - vel.z) * k;
        }
        if let Some(up) = self.lift {
            dv.y = ((up - vel.y) * k).max(0.0);
        }
        dv
    }
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
    /// A loose prop's origin comes to lie inside the volume (`prop`: only that one, an index into [`RuleSet::prop_ids`]).
    PropEnter {
        /// The volume.
        volume: Volume,
        /// Only this prop, or any loose prop.
        prop: Option<usize>,
    },
    /// A loose prop leaves the volume.
    PropExit {
        /// The volume.
        volume: Volume,
        /// Only this prop, or any loose prop.
        prop: Option<usize>,
    },
    /// A loose prop's origin drops below a height (index into [`RuleSet::prop_ids`]).
    PropBelow {
        /// The prop.
        prop: usize,
        /// The height, m.
        y: f32,
    },
}

/// What a `reset` puts back.
#[derive(Debug, Clone, PartialEq)]
pub enum ResetTarget {
    /// These props (indexes into [`RuleSet::prop_ids`]).
    Props(Vec<usize>),
    /// Every loose prop inside this zone (index into [`RuleSet::zone_ids`]) when the rule fires.
    Zone(usize),
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
    /// `hide` and `collision: false` in one step.
    Deactivate(String),
    /// `show` and `collision: true` in one step.
    Activate(String),
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
    /// Put loose props back where the map author placed them, at rest.
    Reset(ResetTarget),
    /// Move a loose prop to a point, upright as authored, at rest.
    Place {
        /// Index into [`RuleSet::prop_ids`].
        prop: usize,
        /// Where its origin goes.
        at: Vec3,
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
    /// Every loose prop of the scene, by object id, sorted: prop triggers, `reset`/`place` and the expression
    /// built-ins refer to a prop by its index here (the runtime binds each to a physics prop once).
    pub prop_ids: Vec<String>,
    /// Every zone id, sorted (`props_in(zone)` and `reset {zone}` index this).
    pub zone_ids: Vec<String>,
    /// The zones' volumes, parallel to [`zone_ids`](Self::zone_ids) (floor `y` up 3 m, like an `enter {zone}` volume).
    pub zone_volumes: Vec<Volume>,
    /// The scene's force fields (`fields`), in declaration order.
    pub fields: Vec<Field>,
    /// Whether any rule looks at loose props (a trigger, a built-in function, a `reset` or `place`): if not, the
    /// simulation need not describe its props to the rules every tick.
    pub needs_props: bool,
}

impl Default for RuleSet {
    fn default() -> Self {
        RuleSet {
            var_names: BUILTIN_VARS.iter().map(|s| s.to_string()).collect(),
            var_init: vec![0.0; BUILTIN_VARS.len()],
            rules: Vec::new(),
            prop_ids: Vec::new(),
            zone_ids: Vec::new(),
            zone_volumes: Vec::new(),
            fields: Vec::new(),
            needs_props: false,
        }
    }
}

impl RuleSet {
    /// The expression scope: the loose props and zones a built-in function may name.
    pub fn scope(&self) -> Scope<'_> {
        Scope { props: &self.prop_ids, zones: &self.zone_ids }
    }
}

/// What the parser may refer to, gathered from the rest of the scene.
#[derive(Default)]
pub struct Refs {
    /// Every object id (any depth), for `hide`/`show`.
    pub object_ids: HashSet<String>,
    /// Top-level objects that are loose props (rigid bodies a player can move), for prop triggers, `reset`/`place`
    /// and the expression built-ins.
    pub prop_ids: HashSet<String>,
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
/// The trigger keys (exactly one per `when`).
const TRIGGER_KEYS: &[&str] = &["start", "enter", "exit", "event", "every", "after", "prop_enter", "prop_exit", "prop_below"];
/// Everything a `when` may contain: a trigger plus the optional `prop` filter of `prop_enter` / `prop_exit`.
const WHEN_KEYS: &[&str] = &["start", "enter", "exit", "event", "every", "after", "prop_enter", "prop_exit", "prop_below", "prop"];
const VOLUME_KEYS: &[&str] = &["zone", "object", "box", "pad", "height"];
const FIELD_KEYS: &[&str] = &["id", "zone", "object", "box", "pad", "height", "velocity", "lift", "rate"];
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

/// `fields`: force volumes on loose props (see [`Field`]).
fn parse_fields(root: &Map<String, Value>, refs: &Refs, set: &mut RuleSet, errs: &mut Vec<String>) {
    let Some(list) = root.get("fields") else { return };
    let Some(list) = list.as_array() else {
        errs.push("fields: must be an array like [{\"id\": \"river\", \"zone\": \"channel\", \"velocity\": [0, 1.5]}]".to_string());
        return;
    };
    let mut seen = HashSet::new();
    for (i, fv) in list.iter().enumerate() {
        let id = fv.get("id").and_then(Value::as_str).unwrap_or("");
        let path = format!("fields[{i}]{}", if id.is_empty() { String::new() } else { format!(" ({id})") });
        let Some(fo) = fv.as_object() else {
            errs.push(format!("{path}: must be an object"));
            continue;
        };
        check_keys(errs, &path, fo, FIELD_KEYS);
        if id.is_empty() {
            errs.push(format!("{path}.id: missing (every field needs a unique id string)"));
        } else if !seen.insert(id.to_string()) {
            errs.push(format!("{path}.id: duplicate field id `{id}`"));
        }
        // The volume keys sit beside the field's own; `parse_volume` checks only the ones it owns.
        let vol_only: Map<String, Value> = fo.iter().filter(|(k, _)| VOLUME_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        let volume = parse_volume(&Value::Object(vol_only), refs, &path, errs);
        let velocity = match fo.get("velocity") {
            None => None,
            Some(v) => match v.as_array().filter(|a| a.len() == 2).and_then(|a| Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32])) {
                Some(xz) => Some(xz),
                None => {
                    errs.push(format!("{path}.velocity: must be two numbers [x, z] (m/s along +x and +z; up is `lift`)"));
                    None
                }
            },
        };
        let lift = match fo.get("lift") {
            None => None,
            Some(v) => match v.as_f64() {
                Some(x) => Some(x as f32),
                None => {
                    errs.push(format!("{path}.lift: must be a number (upward m/s)"));
                    None
                }
            },
        };
        if velocity.is_none() && lift.is_none() && fo.get("velocity").is_none() && fo.get("lift").is_none() {
            errs.push(format!("{path}: give `velocity` [x, z] and/or `lift` (what the field pushes toward)"));
        }
        let rate = fo.get("rate").and_then(Value::as_f64).map_or(10.0, |r| r as f32);
        if !(rate > 0.0 && rate <= 60.0) {
            errs.push(format!("{path}.rate: must be greater than 0 and at most 60 (per second)"));
        }
        if let Some(volume) = volume {
            set.fields.push(Field { id: id.to_string(), volume, velocity, lift, rate });
        }
    }
    if !set.fields.is_empty() {
        set.needs_props = true;
    }
}

/// The index of loose prop `name` in `prop_ids`, or an error naming the fix.
fn prop_index(name: Option<&str>, prop_ids: &[String], path: &str, errs: &mut Vec<String>) -> Option<usize> {
    let name = name.unwrap_or("");
    match prop_ids.iter().position(|p| p == name) {
        Some(i) => Some(i),
        None => {
            let known = if prop_ids.is_empty() {
                "none in this scene: a `prop` object or a floor prefab that is not a fixture".to_string()
            } else {
                sorted(prop_ids.iter())
            };
            errs.push(format!("{path}: no loose prop `{name}`{} (loose props: {known})", near(name, prop_ids.iter().cloned())));
            None
        }
    }
}

fn parse_when(v: &Value, set: &RuleSet, refs: &Refs, path: &str, errs: &mut Vec<String>) -> Option<When> {
    let Some(o) = v.as_object() else {
        errs.push(format!("{path}: must be an object with one of {}", TRIGGER_KEYS.join(", ")));
        return None;
    };
    check_keys(errs, path, o, WHEN_KEYS);
    let present: Vec<&str> = TRIGGER_KEYS.iter().copied().filter(|k| o.contains_key(*k)).collect();
    if present.len() != 1 {
        errs.push(format!(
            "{path}: give exactly one of {} (got {})",
            TRIGGER_KEYS.join(", "),
            if present.is_empty() { "none".to_string() } else { present.join(" + ") }
        ));
        return None;
    }
    let key = present[0];
    let sub = format!("{path}.{key}");
    if o.contains_key("prop") && !matches!(key, "prop_enter" | "prop_exit") {
        errs.push(format!("{path}.prop: only `prop_enter` / `prop_exit` take a `prop` filter"));
        return None;
    }
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
        "after" => Some(When::After(secs(&o[key], &sub, errs))),
        "prop_enter" | "prop_exit" => {
            let volume = parse_volume(&o[key], refs, &sub, errs)?;
            let prop = match o.get("prop") {
                None => None,
                Some(p) => Some(prop_index(p.as_str(), &set.prop_ids, &format!("{path}.prop"), errs)?),
            };
            Some(if key == "prop_enter" { When::PropEnter { volume, prop } } else { When::PropExit { volume, prop } })
        }
        _ => {
            let (id, y) = match &o[key] {
                Value::Array(a) if a.len() == 2 => (a[0].as_str(), a[1].as_f64()),
                Value::Object(m) => (m.get("prop").or_else(|| m.get("object")).and_then(Value::as_str), m.get("y").and_then(Value::as_f64)),
                _ => (None, None),
            };
            let (Some(id), Some(y)) = (id, y) else {
                errs.push(format!("{sub}: must be [prop_id, y] (fires once the prop's origin drops below y metres)"));
                return None;
            };
            let prop = prop_index(Some(id), &set.prop_ids, &format!("{sub}[0]"), errs)?;
            Some(When::PropBelow { prop, y: y as f32 })
        }
    }
}

/// A number, a bool, or an expression string, compiled against the set's variables, loose props and zones.
fn parse_value(v: &Value, set: &RuleSet, path: &str, errs: &mut Vec<String>) -> Option<Expr> {
    match v {
        Value::Number(n) => n.as_f64().map(Expr::Num),
        Value::Bool(x) => Some(Expr::Num(if *x { 1.0 } else { 0.0 })),
        Value::String(s) => match rules_expr::parse_in(s, &set.var_names, set.scope()) {
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

fn parse_action(v: &Value, set: &RuleSet, refs: &Refs, path: &str, errs: &mut Vec<String>) -> Option<Action> {
    let names = &set.var_names;
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
            let value = parse_value(&p[1], set, &format!("{sub}[1]"), errs)?;
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
        "deactivate" | "activate" => {
            let id = val.as_str().unwrap_or("");
            if !refs.top_level_ids.contains(id) {
                errs.push(format!("{sub}: no top-level object `{id}`{}", near(id, refs.top_level_ids.iter().cloned())));
                return None;
            }
            Some(if key.as_str() == "deactivate" { Action::Deactivate(id.to_string()) } else { Action::Activate(id.to_string()) })
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
        "reset" => match val {
            Value::String(id) => prop_index(Some(id), &set.prop_ids, &sub, errs).map(|p| Action::Reset(ResetTarget::Props(vec![p]))),
            Value::Array(a) if !a.is_empty() => {
                let props: Vec<usize> = a.iter().enumerate().filter_map(|(k, v)| prop_index(v.as_str(), &set.prop_ids, &format!("{sub}[{k}]"), errs)).collect();
                (props.len() == a.len()).then_some(Action::Reset(ResetTarget::Props(props)))
            }
            Value::Object(m) => {
                check_keys(errs, &sub, m, &["zone"]);
                let id = m.get("zone").and_then(Value::as_str).unwrap_or("");
                match set.zone_ids.iter().position(|z| z == id) {
                    Some(z) => Some(Action::Reset(ResetTarget::Zone(z))),
                    None => {
                        errs.push(format!("{sub}.zone: no zone `{id}`{} (zones: {})", near(id, set.zone_ids.iter().cloned()), sorted(set.zone_ids.iter())));
                        None
                    }
                }
            }
            _ => {
                errs.push(format!("{sub}: must be a loose prop id, [ids] or {{\"zone\": id}}"));
                None
            }
        },
        "place" => {
            let Some(p) = pair() else {
                errs.push(format!("{sub}: must be [prop_id, [x, y, z]]"));
                return None;
            };
            let prop = prop_index(p[0].as_str(), &set.prop_ids, &format!("{sub}[0]"), errs)?;
            let Some(at) = vec3(&p[1]) else {
                errs.push(format!("{sub}[1]: must be [x, y, z]"));
                return None;
            };
            Some(Action::Place { prop, at })
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
    set.prop_ids = refs.prop_ids.iter().cloned().collect();
    set.prop_ids.sort();
    let mut zones: Vec<(&String, &(Vec3, Vec3))> = refs.zones.iter().collect();
    zones.sort_by(|a, b| a.0.cmp(b.0));
    for (id, (lo, hi)) in zones {
        set.zone_ids.push(id.clone());
        set.zone_volumes.push(Volume { min: *lo, max: Vec3::new(hi.x, lo.y + ZONE_HEIGHT, hi.z) });
    }
    if let Some(vars) = root.get("vars") {
        match vars.as_object() {
            None => errs.push("vars: must be an object like {\"score\": 0, \"has_key\": false}".to_string()),
            Some(o) => {
                // `_name` is an internal variable (hidden from the HUD), so only `x-*` / `$comment` / `notes` are notes here: a `_` var used to be
                // dropped silently and then rejected as unknown wherever a rule read it.
                for (name, v) in o.iter().filter(|(k, _)| !(k.starts_with("x-") || *k == "$comment" || *k == "notes")) {
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
    parse_fields(root, refs, &mut set, &mut errs);
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
            Some(w) => parse_when(w, &set, refs, &format!("{p}.when"), &mut errs),
            None => {
                errs.push(format!("{p}.when: missing (one of {})", TRIGGER_KEYS.join(", ")));
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
        let cond = ro.get("if").and_then(|c| parse_value(c, &set, &format!("{p}.if"), &mut errs));
        let cooldown_ticks = ro.get("cooldown").map_or(0, |c| secs(c, &format!("{p}.cooldown"), &mut errs));
        let once = ro.get("once").and_then(Value::as_bool).unwrap_or(false);
        let mut actions = Vec::new();
        match ro.get("do").and_then(Value::as_array) {
            Some(arr) if !arr.is_empty() => {
                for (k, av) in arr.iter().enumerate() {
                    if let Some(a) = parse_action(av, &set, refs, &format!("{p}.do[{k}]"), &mut errs) {
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
    set.needs_props = set.rules.iter().any(|r| {
        matches!(r.when, When::PropEnter { .. } | When::PropExit { .. } | When::PropBelow { .. })
            || r.cond.as_ref().is_some_and(Expr::reads_world)
            || r.actions.iter().any(|a| match a {
                Action::Set { value, .. } => value.reads_world(),
                Action::Reset(_) | Action::Place { .. } => true,
                _ => false,
            })
    });
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
        r.object_ids.extend(["coin_1".to_string(), "door".to_string(), "crate".to_string(), "bell".to_string()]);
        r.top_level_ids.extend(["coin_1".to_string(), "door".to_string(), "crate".to_string(), "bell".to_string()]);
        r.bounds.insert("coin_1".into(), (Vec3::new(1.0, 0.0, 1.0), Vec3::new(1.4, 0.4, 1.4)));
        r.bounds.insert("crate".into(), (Vec3::new(3.0, 0.0, 3.0), Vec3::new(3.6, 0.6, 3.6)));
        r.bounds.insert("bell".into(), (Vec3::new(5.0, 1.0, 5.0), Vec3::new(5.3, 1.4, 5.3)));
        r.prop_ids.extend(["crate".to_string(), "bell".to_string()]);
        r.zones.insert("exit".into(), (Vec3::new(8.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 2.0)));
        r.zones.insert("pit".into(), (Vec3::new(0.0, -3.0, 4.0), Vec3::new(6.0, -3.0, 8.0)));
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
        assert!(!set.needs_props, "nothing here looks at a prop");
        assert_eq!(set.prop_ids, ["bell", "crate"]);
        assert_eq!(set.zone_ids, ["exit", "pit"]);
    }

    #[test]
    fn prop_triggers_built_ins_and_reset_place_parse_against_the_loose_props_and_zones() {
        let set = parse(json!({
            "vars": {"score": 0, "fallen": 0},
            "rules": [
                {"id": "score", "when": {"prop_enter": {"zone": "pit"}}, "do": [{"add": ["score", 1]}]},
                {"id": "only_crate", "when": {"prop_exit": {"box": [0,0,0,1,1,1]}, "prop": "crate"}, "do": [{"emit": "left"}]},
                {"id": "bell_down", "when": {"prop_below": ["bell", 0.5]}, "if": "tilt(bell) > 60 || !held(bell)", "do": [{"set": ["fallen", "props_in(pit) + moved(crate)"]}]},
                {"id": "again", "when": {"event": "left"}, "do": [{"reset": "bell"}, {"reset": ["crate", "bell"]}, {"reset": {"zone": "pit"}}, {"place": ["crate", [1, 2, 3]]}]}
            ]
        }))
        .unwrap();
        assert!(set.needs_props);
        let When::PropEnter { volume, prop: None } = &set.rules[0].when else { panic!("{:?}", set.rules[0].when) };
        assert_eq!((volume.min.y, volume.max.y), (-3.0, 0.0));
        assert!(matches!(set.rules[1].when, When::PropExit { prop: Some(1), .. }), "crate is prop_ids[1]");
        assert!(matches!(set.rules[2].when, When::PropBelow { prop: 0, y } if y == 0.5));
        assert_eq!(
            set.rules[3].actions,
            [
                Action::Reset(ResetTarget::Props(vec![0])),
                Action::Reset(ResetTarget::Props(vec![1, 0])),
                Action::Reset(ResetTarget::Zone(1)),
                Action::Place { prop: 1, at: Vec3::new(1.0, 2.0, 3.0) }
            ]
        );
        let e = parse(json!({"vars": {}, "rules": [
            {"id": "a", "when": {"prop_enter": {"zone": "pit"}, "prop": "crat"}, "do": [{"emit": "x"}]},
            {"id": "b", "when": {"enter": {"zone": "pit"}, "prop": "crate"}, "do": [{"emit": "x"}]},
            {"id": "c", "when": {"prop_below": ["coin_1", 1]}, "if": "prop_y(coin_1) < 1", "do": [{"reset": {"zone": "pitt"}}, {"place": ["crate", 3]}]}
        ]}))
        .unwrap_err()
        .join("\n");
        for needle in [
            "rules[0] (a).when.prop: no loose prop `crat` — did you mean `crate`?",
            "rules[1] (b).when.prop: only `prop_enter` / `prop_exit` take a `prop` filter",
            "rules[2] (c).when.prop_below[0]: no loose prop `coin_1`",
            "rules[2] (c).if: `prop_y(coin_1)`: no loose prop `coin_1`",
            "rules[2] (c).do[0].reset.zone: no zone `pitt` — did you mean `pit`?",
            "rules[2] (c).do[1].place[1]: must be [x, y, z]",
        ] {
            assert!(e.contains(needle), "missing `{needle}` in:\n{e}");
        }
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
        let set = parse(json!({})).unwrap();
        let default = RuleSet::default();
        assert_eq!((&set.var_names, &set.var_init, &set.rules, set.needs_props), (&default.var_names, &default.var_init, &default.rules, false));
        assert_eq!(set.prop_ids, ["bell", "crate"], "the scene's loose props and zones are always in scope");
        assert!(parse_rules(json!({}).as_object().unwrap(), &Refs::default()).unwrap() == default, "no scene, no tables");
    }
}
