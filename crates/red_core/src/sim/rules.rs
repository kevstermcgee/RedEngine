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
//! `place [id, [x,y,z]]`, `aim [field, target]`.
//!
//! **The rule's subject** (ADR 2026-10-13-aimed-fields-and-the-rule-s-subject-prop): a rule fired by one loose prop — `prop_enter`, `prop_exit`,
//! `prop_below`, or the engine events `pickup`, `drop` and `prop_hit` — knows which one, and names it `self` wherever it would name a loose prop
//! (`held(self)`, `hide self`, `reset self`, `place [self, ..]`, `impulse {object: self}`, `aim [field, self]`). An event a rule `emit`s carries
//! its subject on, like the triggering player. `prop: id | [ids]` beside the trigger fires the rule only for those props.
//!
//! **Fields** (`fields`, [`Field`]) push loose props every tick: along a velocity, or — an *aimed* field, `toward` — at a target that a rule
//! re-aims at run time with `aim`; their numbers may be expressions over the variables.
//!
//! Rules see **loose props** too (ADR 2026-09-29-prop-aware-rules-and-scenarios): triggers `{prop_enter: VOLUME}` /
//! `{prop_exit: VOLUME}` (any loose prop, or one named by `prop: id` next to it) and `{prop_below: [id, y]}`, and the
//! expression built-ins `prop_y(id)`, `tilt(id)`, `held(id)`, `mass(id)`, `moved(id)`, `props_in(zone)`, `in_zone(id, zone)` in `if` and values.

use super::clock::secs_to_ticks;
use super::rule_timers::TimerRef;
use super::rules_expr::{self, Expr, Op, Scope};
use crate::strict::{check_keys, describe_value};
use glam::Vec3;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// Variables every rule can read but not set: seconds elapsed, ticks elapsed, connected players.
pub const BUILTIN_VARS: &[&str] = &["time", "tick", "players"];

/// The most per-player variables a scene may declare (each player has a copy; the count is bounded so a snapshot of them stays small).
pub const MAX_PLAYER_VARS: usize = 8;
/// The most per-prop variables a scene may declare (each loose prop has a copy, `self.name`).
pub const MAX_PROP_VARS: usize = 8;

/// Events the engine itself raises (a rule can react with `when: {event: name}` without any rule emitting them):
/// `pickup` / `drop` (a player took / released a prop), `shot` (a firearm was fired), `hit` (a player was damaged),
/// `kill` (a player was killed; the player is the killer), `respawn` (a dead player came back), `swing` (a bat swing
/// started), `prop_hit` (a bat or a bullet struck a loose prop; the player is the striker).
pub const ENGINE_EVENTS: &[&str] = &["pickup", "drop", "shot", "hit", "kill", "respawn", "swing", "prop_hit", "sunrise", "sunset"];

/// The action names, for error messages and `describe rules`.
pub const ACTIONS: &[(&str, &str)] = &[
    ("set", "[var, value]  set a variable to a number, true/false, or an expression string"),
    ("add", "[var, n]      add a number (or expression) to a variable"),
    ("emit", "name         record a game event; other rules can react with `when: {event: name}`"),
    ("timer", "[name, secs]  start (or restart) a timer; `when: {timer: name}` fires once when it runs out; `self.name` is the subject prop's own"),
    ("cancel", "name         stop a timer before it runs out (nothing happens when it is not running)"),
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
    ("aim", "[field, target]   point an aimed field (`toward`) at a loose prop (followed as it moves), a top-level object, [x,y,z], self, or null (off); from the next tick"),
];

/// Engine events that carry the prop they are about, so a rule they trigger has a subject (`self`): the prop picked up, put down, or struck
/// (a shot that strikes several props names the first one it reached).
pub const SUBJECT_EVENTS: &[&str] = &["pickup", "drop", "prop_hit"];

/// The name a rule gives its subject prop (see [`SUBJECT_EVENTS`] and the module docs).
pub const SELF: &str = "self";

/// The most `fields` a scene may declare (each is evaluated every tick against every loose prop it acts on).
pub const MAX_FIELDS: usize = 32;

/// Within this many metres of its target an aimed field's pull fades linearly to nothing, so a prop settles at the target instead of orbiting it.
pub const AIM_ARRIVE_M: f32 = 0.5;

/// An axis-aligned box in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Volume {
    /// Lowest corner.
    pub min: Vec3,
    /// Highest corner.
    pub max: Vec3,
}

/// A force volume acting on loose props (`fields` in the scene): a river current, a conveyor belt, a wind tunnel, or — with `toward` — a pull
/// at a target a rule can re-aim (gravity toward the last thing touched, a magnet, a whirlpool). Every tick a prop whose origin is inside is
/// pulled toward the field's target velocity, exponentially at `rate` per second, so the push is a *speed target* (a prop cannot be accelerated
/// past it, unlike a repeating `impulse` rule) and gravity, floors and walls still act. Its numbers may be expressions over the variables,
/// evaluated once per tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// Its id (errors, `describe` and the `aim` action name it).
    pub id: String,
    /// The volume props must be in (same forms as a rule volume).
    pub volume: Volume,
    /// Target horizontal velocity `[x, z]`, m/s; `None` leaves horizontal motion alone (and an aimed field has none).
    pub velocity: Option<[Expr; 2]>,
    /// Upward target speed, m/s: props slower than this upward are pulled up toward it (a wind tunnel, a hover pad); `None` = none.
    pub lift: Option<Expr>,
    /// How hard the velocity is pulled toward its target, 1/s (default 10). It is a drag, not a teleport: a prop on a floor settles a bit under the
    /// target because floor friction (about 7 m/s^2) pulls back, so use a higher `rate` for a stronger belt.
    pub rate: f32,
    /// `Some` for an aimed field (`toward`): where it points when the match starts; rules move it with `aim`. `None`: a velocity field.
    pub toward: Option<Aim>,
    /// An aimed field's pull, m/s along the line to its target (negative pushes away); required with `toward`.
    pub speed: Option<Expr>,
    /// Only these loose props (indexes into [`RuleSet::prop_ids`]); `None` = every loose prop in the volume.
    pub props: Option<Vec<usize>>,
}

impl Field {
    /// The velocity change for a prop moving at `vel` in a *velocity* field whose numbers evaluated to `target` (`[x, z]`) and `lift` this tick,
    /// for a step of `dt` seconds (zero once it is at the target).
    pub fn drag(&self, target: Option<[f32; 2]>, lift: Option<f32>, vel: Vec3, dt: f32) -> Vec3 {
        let k = (self.rate * dt).clamp(0.0, 1.0);
        let mut dv = Vec3::ZERO;
        if let Some([tx, tz]) = target {
            dv.x = (tx - vel.x) * k;
            dv.z = (tz - vel.z) * k;
        }
        if let Some(up) = lift {
            dv.y = ((up - vel.y) * k).max(0.0);
        }
        dv
    }

    /// The velocity change for a prop at `from` moving at `vel` in an *aimed* field pointing at `to` with `speed` (m/s, negative = away) this tick.
    /// Only the horizontal component along the line to the target is driven, and only up to `speed`: a prop already moving that way faster is
    /// not braked and its sideways motion is left to friction (gravity pulls, it does not steer). Within [`AIM_ARRIVE_M`] of the target the pull
    /// fades to zero, and a prop exactly on it is not pushed.
    pub fn pull(&self, from: Vec3, to: Vec3, speed: f32, lift: Option<f32>, vel: Vec3, dt: f32) -> Vec3 {
        let k = (self.rate * dt).clamp(0.0, 1.0);
        let mut dv = Vec3::ZERO;
        let (dx, dz) = (to.x - from.x, to.z - from.z);
        let dist = (dx * dx + dz * dz).sqrt();
        if dist > 1e-4 && speed != 0.0 {
            let s = speed * (dist / AIM_ARRIVE_M).min(1.0);
            let (ux, uz, want) = if s > 0.0 { (dx / dist, dz / dist, s) } else { (-dx / dist, -dz / dist, -s) };
            let along = vel.x * ux + vel.z * uz;
            if along < want {
                let a = (want - along) * k;
                dv.x = ux * a;
                dv.z = uz * a;
            }
        }
        if let Some(up) = lift {
            dv.y = ((up - vel.y) * k).max(0.0);
        }
        dv
    }
}

/// Where an aimed field points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Aim {
    /// Nowhere: the field pushes nothing until a rule aims it (`"toward": null`, `aim [field, null]`).
    Off,
    /// A loose prop, followed wherever it is (in a player's hands too): an index into [`RuleSet::prop_ids`]. A prop the physics world does not
    /// have (an unbound id) leaves the field idle.
    Prop(usize),
    /// A fixed point: `[x, y, z]`, or a top-level object's centre.
    Point(Vec3),
}

/// A loose prop a rule action names: one by id, or the rule's subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropSel {
    /// An index into [`RuleSet::prop_ids`].
    Prop(usize),
    /// `self`: the prop that triggered the rule. The action does nothing when the rule ran without one.
    Subject,
}

/// An object `hide` / `show` / `impulse` names: by id, or the rule's subject prop.
#[derive(Debug, Clone, PartialEq)]
pub enum Obj {
    /// An object id.
    Id(String),
    /// `self`: the prop that triggered the rule. The action does nothing when the rule ran without one.
    Subject,
}

/// What an `aim` action points its field at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AimTo {
    /// A fixed aim (a prop, a point, or off).
    At(Aim),
    /// `self`: the rule's subject prop. Without one the field keeps its aim.
    Subject,
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
    /// Team 1 only (needs the scene's own `"teams": true`, or a `shooter` block).
    Team1,
    /// Team 2 only (needs the scene's own `"teams": true`, or a `shooter` block).
    Team2,
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
    /// A loose prop's origin comes to lie inside the volume (each prop that crosses fires the rule once, as its subject; [`Rule::props`] narrows which).
    PropEnter(Volume),
    /// A loose prop leaves the volume.
    PropExit(Volume),
    /// A timer ran out (`sim::rule_timers`); a per-prop timer fires once for each prop whose own ran out, as its subject.
    Timer(TimerRef),
    /// A player presses an input, or lets go of it (`sim::rule_input`); `me` is that player.
    Input(super::rule_input::InputTrigger),
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
    /// `self`: the prop that triggered the rule (nothing when there is none).
    Subject,
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
    /// `me.name = value` for the player that triggered the rule (an `add` is compiled to a `SetMe` of `me.name + n`). Does nothing when no player did.
    SetMe {
        /// Index in [`RuleSet::player_var_names`].
        var: usize,
        /// The new value.
        value: Expr,
    },
    /// `self.name = value` for the rule's subject prop (an `add` is compiled to a `SetIts` of `self.name + n`). Does nothing without a subject.
    SetIts {
        /// Index in [`RuleSet::prop_var_names`].
        var: usize,
        /// The new value.
        value: Expr,
    },
    /// Start (or restart) a timer to run out after `secs` seconds.
    Timer {
        /// Which.
        timer: TimerRef,
        /// Seconds (an expression; negative counts as 0).
        secs: Expr,
    },
    /// Stop a timer before it runs out.
    Cancel(TimerRef),
    /// Record an event.
    Emit(String),
    /// Mark an object hidden.
    Hide(Obj),
    /// `hide` and `collision: false` in one step.
    Deactivate(String),
    /// `show` and `collision: true` in one step.
    Activate(String),
    /// Mark an object shown.
    Show(Obj),
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
        /// The prop (its object id, or the subject).
        object: Obj,
        /// Direction (normalised when applied).
        dir: Vec3,
        /// Speed given, m/s.
        speed: f32,
    },
    /// Put loose props back where the map author placed them, at rest.
    Reset(ResetTarget),
    /// Move a loose prop to a point, upright as authored, at rest.
    Place {
        /// The prop.
        prop: PropSel,
        /// Where its origin goes.
        at: Vec3,
    },
    /// Point an aimed field somewhere else (it takes effect from the next tick).
    Aim {
        /// Index into [`RuleSet::fields`] (always an aimed one).
        field: usize,
        /// Where.
        to: AimTo,
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
    /// `prop: id | [ids]` beside the trigger: fire only when the rule's subject is one of these loose props (indexes into [`RuleSet::prop_ids`]).
    pub props: Option<Vec<usize>>,
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
    /// Per-player variable names (`player_vars`), in declaration order: every player has their own copy, read and written in rules as `me.name`.
    pub player_var_names: Vec<String>,
    /// Initial value of every per-player variable (a player's copy starts here when they join).
    pub player_var_init: Vec<f64>,
    /// Per-prop variable names (`prop_vars`): every loose prop has its own copy, read and written as `self.name` by a rule with a subject prop.
    pub prop_var_names: Vec<String>,
    /// Initial value of every per-prop variable.
    pub prop_var_init: Vec<f64>,
    /// The scene's timers (`sim::rule_timers`), in the order the rules first name them.
    pub timers: Vec<String>,
    /// The per-prop timers (`self.name`; the name without `self.`): every loose prop has its own.
    pub prop_timers: Vec<String>,
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
    /// Whether any rule or field looks at loose props (a trigger, a built-in function, a `reset` or `place`, a field): if not, the
    /// simulation need not describe its props to the rules every tick.
    pub needs_props: bool,
    /// The scene's named level states (`phases`), in declaration order: for the analysis tools only, the simulation ignores them.
    pub phases: Vec<Phase>,
    /// Variables the game keeps between sessions (`persist`): loaded when it starts, saved when they change.
    pub persist: Vec<String>,
}

/// A named state of the level for `lint` / `reach` / `walk` (`"phases": {"gate_open": ["open_gate"]}`): the rules to assume
/// have already fired, so what they open (collision off, `deactivate`) is open to the tools.
#[derive(Debug, Clone, PartialEq)]
pub struct Phase {
    /// The phase name (`checks.reach[].phase`, `--phase`).
    pub name: String,
    /// Indices into [`RuleSet::rules`], applied in this order after the unconditional `start` rules.
    pub rules: Vec<usize>,
}

impl Default for RuleSet {
    fn default() -> Self {
        RuleSet {
            var_names: BUILTIN_VARS.iter().map(|s| s.to_string()).collect(),
            var_init: vec![0.0; BUILTIN_VARS.len()],
            player_var_names: Vec::new(),
            player_var_init: Vec::new(),
            prop_var_names: Vec::new(),
            prop_var_init: Vec::new(),
            timers: Vec::new(),
            prop_timers: Vec::new(),
            rules: Vec::new(),
            prop_ids: Vec::new(),
            zone_ids: Vec::new(),
            zone_volumes: Vec::new(),
            fields: Vec::new(),
            needs_props: false,
            phases: Vec::new(),
            persist: Vec::new(),
        }
    }
}

impl RuleSet {
    /// The scene's own variable names (`vars`, declaration order), the built-ins left out: the order of `RulesEngine::vars` and of a networked
    /// `RuleState`'s variable indices.
    pub fn own_var_names(&self) -> &[String] {
        &self.var_names[BUILTIN_VARS.len().min(self.var_names.len())..]
    }

    /// Top-level objects the player cannot collide with in `phase` (`None` = the initial state): what the unconditional `start`
    /// rules switch off, then what each of the phase's rules switches off or back on, in order. An unknown phase is an error
    /// that names the declared ones.
    pub fn open_objects(&self, phase: Option<&str>) -> Result<Vec<String>, String> {
        let mut open: Vec<String> = Vec::new();
        let mut apply = |actions: &[Action]| {
            for a in actions {
                match a {
                    Action::Collision { object, enabled: false } | Action::Deactivate(object) => {
                        if !open.contains(object) {
                            open.push(object.clone());
                        }
                    }
                    Action::Collision { object, enabled: true } | Action::Activate(object) => open.retain(|o| o != object),
                    _ => {}
                }
            }
        };
        for r in self.rules.iter().filter(|r| matches!(r.when, When::Start) && r.cond.is_none()) {
            apply(&r.actions);
        }
        if let Some(name) = phase {
            let Some(ph) = self.phases.iter().find(|p| p.name == name) else {
                let declared: Vec<String> = self.phases.iter().map(|p| p.name.clone()).collect();
                return Err(format!(
                    "unknown phase `{name}`{} (declared: {})",
                    near(name, declared.iter().cloned()),
                    if declared.is_empty() { "none; add \"phases\": {\"name\": [\"rule_id\"]} to the scene".to_string() } else { declared.join(", ") }
                ));
            };
            for &i in &ph.rules {
                apply(&self.rules[i].actions);
            }
        }
        Ok(open)
    }

    /// The expression scope: the loose props and zones a built-in function may name.
    pub fn scope(&self) -> Scope<'_> {
        Scope { props: &self.prop_ids, zones: &self.zone_ids, player_vars: &self.player_var_names, prop_vars: &self.prop_var_names }
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
const TRIGGER_KEYS: &[&str] = &["start", "enter", "exit", "event", "every", "after", "prop_enter", "prop_exit", "prop_below", "timer", "input"];
/// Everything a `when` may contain: a trigger plus the optional `prop` filter of `prop_enter` / `prop_exit`.
const WHEN_KEYS: &[&str] = &["start", "enter", "exit", "event", "every", "after", "prop_enter", "prop_exit", "prop_below", "timer", "input", "release", "prop"];
const VOLUME_KEYS: &[&str] = &["zone", "object", "box", "pad", "height"];
const FIELD_KEYS: &[&str] = &["id", "zone", "object", "box", "pad", "height", "velocity", "lift", "rate", "toward", "speed", "props"];
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

/// A name or id the rule must give as a string. Anything else is an error that says so, not an empty name that later reports "no object ``".
fn name_of<'a>(v: &'a Value, path: &str, what: &str, errs: &mut Vec<String>) -> Option<&'a str> {
    let s = v.as_str();
    if s.is_none() {
        errs.push(format!("{path}: must be {what} (a string), got {}", describe_value(v)));
    }
    s
}

/// An optional `id` of a rule or a field: `Ok("")` when absent (the caller says it is missing); a non-string is its own error.
fn id_of<'a>(v: &'a Value, path: &str, what: &str, errs: &mut Vec<String>) -> &'a str {
    match v.get("id") {
        None => "",
        Some(Value::String(s)) => s,
        Some(other) => {
            errs.push(format!("{path}.id: must be a string, a unique id for the {what} (got {})", describe_value(other)));
            ""
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
    // `pad` only grows a volume. A negative one used to be treated as 0, so changing -0.55 to -0.8 changed nothing and nothing said why.
    let pad = match o.get("pad") {
        None => 0.0,
        Some(v) => match v.as_f64().filter(|p| p.is_finite() && *p >= 0.0) {
            Some(p) => p as f32,
            None => {
                errs.push(format!(
                    "{path}.pad: must be a number of metres, 0 or more (got {}); `pad` only grows a volume: for a smaller one use a smaller `box`, or a zone of the size you want",
                    describe_value(v)
                ));
                0.0
            }
        },
    };
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
            let id = name_of(&o["zone"], &format!("{path}.zone"), "a zone id", errs)?;
            let Some((lo, hi)) = refs.zones.get(id) else {
                errs.push(format!("{path}.zone: no zone `{id}`{} (zones: {})", near(id, refs.zones.keys().cloned()), sorted(refs.zones.keys())));
                return None;
            };
            let h = match o.get("height") {
                None => ZONE_HEIGHT,
                Some(v) => match v.as_f64().filter(|h| h.is_finite() && *h > 0.0) {
                    Some(h) => h as f32,
                    None => {
                        errs.push(format!(
                            "{path}.height: must be a number of metres greater than 0 (got {}); leave it out for the default {ZONE_HEIGHT}",
                            describe_value(v)
                        ));
                        ZONE_HEIGHT
                    }
                },
            };
            Some(Volume { min: *lo - Vec3::splat(pad), max: Vec3::new(hi.x, lo.y + h, hi.z) + Vec3::splat(pad) })
        }
        "object" => {
            let id = name_of(&o["object"], &format!("{path}.object"), "a top-level object id", errs)?;
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

/// A field's number: a literal or an expression over the variables (and the prop built-ins), evaluated once per tick. A field runs for no player
/// and no prop, so `me.` and `self` are errors here.
fn field_num(v: &Value, set: &RuleSet, path: &str, what: &str, errs: &mut Vec<String>) -> Option<Expr> {
    match v {
        Value::Number(_) | Value::String(_) => {
            let e = parse_value(v, set, path, errs)?;
            if e.uses_me() || e.uses_subject() {
                errs.push(format!("{path}: a field is not run for a player or a prop: `me.` and `self` belong in rules (got {})", describe_value(v)));
                return None;
            }
            Some(e)
        }
        _ => {
            errs.push(format!("{path}: must be {what}, a number or an expression string (got {})", describe_value(v)));
            None
        }
    }
}

/// Where a `toward` or an `aim` points: a loose prop (followed), a top-level object (its centre), `[x, y, z]`, or `null` (off).
fn parse_aim(v: &Value, set: &RuleSet, refs: &Refs, path: &str, errs: &mut Vec<String>) -> Option<Aim> {
    match v {
        Value::Null => Some(Aim::Off),
        Value::String(id) => {
            if let Some(i) = set.prop_ids.iter().position(|p| p == id) {
                return Some(Aim::Prop(i));
            }
            if let Some((lo, hi)) = refs.bounds.get(id.as_str()) {
                return Some(Aim::Point((*lo + *hi) * 0.5));
            }
            let options = set.prop_ids.iter().cloned().chain(refs.bounds.keys().cloned());
            errs.push(format!(
                "{path}: no loose prop or top-level object `{id}`{} (a loose prop is followed as it moves; a fixed object is aimed at its centre)",
                near(id, options)
            ));
            None
        }
        other => match vec3(other) {
            Some(p) => Some(Aim::Point(p)),
            None => {
                errs.push(format!("{path}: must be a loose prop or object id, [x, y, z], or null for nowhere (got {})", describe_value(other)));
                None
            }
        },
    }
}

/// A list of loose props given as one id or an array of ids (a field's `props`, a rule's `prop` filter): never empty, no repeats.
fn prop_list(v: &Value, set: &RuleSet, path: &str, errs: &mut Vec<String>) -> Option<Vec<usize>> {
    let items: Vec<&Value> = match v {
        Value::String(_) => vec![v],
        Value::Array(a) if !a.is_empty() => a.iter().collect(),
        Value::Array(_) => {
            errs.push(format!("{path}: an empty list selects nothing; leave it out for every loose prop"));
            return None;
        }
        other => {
            errs.push(format!("{path}: must be a loose prop id or a list of them (got {})", describe_value(other)));
            return None;
        }
    };
    let mut out: Vec<usize> = Vec::with_capacity(items.len());
    let mut ok = true;
    for (k, item) in items.iter().enumerate() {
        let at = if items.len() == 1 && v.is_string() { path.to_string() } else { format!("{path}[{k}]") };
        match prop_index(item.as_str(), &set.prop_ids, &at, errs) {
            Some(i) if out.contains(&i) => {
                errs.push(format!("{at}: `{}` is listed twice", set.prop_ids[i]));
                ok = false;
            }
            Some(i) => out.push(i),
            None => ok = false,
        }
    }
    ok.then_some(out)
}

/// `fields`: force volumes on loose props (see [`Field`]).
fn parse_fields(root: &Map<String, Value>, refs: &Refs, set: &mut RuleSet, errs: &mut Vec<String>) {
    let Some(list) = root.get("fields") else { return };
    let Some(list) = list.as_array() else {
        errs.push("fields: must be an array like [{\"id\": \"river\", \"zone\": \"channel\", \"velocity\": [0, 1.5]}]".to_string());
        return;
    };
    if list.len() > MAX_FIELDS {
        errs.push(format!("fields: {} fields; at most {MAX_FIELDS} (each is evaluated every tick against every loose prop it acts on)", list.len()));
        return;
    }
    let mut seen = HashSet::new();
    for (i, fv) in list.iter().enumerate() {
        let Some(fo) = fv.as_object() else {
            errs.push(format!("fields[{i}]: must be an object"));
            continue;
        };
        let id = id_of(fv, &format!("fields[{i}]"), "field", errs);
        let path = format!("fields[{i}]{}", if id.is_empty() { String::new() } else { format!(" ({id})") });
        check_keys(errs, &path, fo, FIELD_KEYS);
        if id.is_empty() {
            if fv.get("id").is_none() {
                errs.push(format!("{path}.id: missing (every field needs a unique id string)"));
            }
        } else if !seen.insert(id.to_string()) {
            errs.push(format!("{path}.id: duplicate field id `{id}`"));
        }
        // The volume keys sit beside the field's own; `parse_volume` checks only the ones it owns.
        let vol_only: Map<String, Value> = fo.iter().filter(|(k, _)| VOLUME_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
        let volume = parse_volume(&Value::Object(vol_only), refs, &path, errs);
        let velocity = match fo.get("velocity") {
            None => None,
            Some(Value::Array(a)) if a.len() == 2 => {
                let x = field_num(&a[0], set, &format!("{path}.velocity[0]"), "m/s along +x", errs);
                let z = field_num(&a[1], set, &format!("{path}.velocity[1]"), "m/s along +z", errs);
                x.zip(z).map(|(x, z)| [x, z])
            }
            Some(_) => {
                errs.push(format!("{path}.velocity: must be two numbers [x, z] (m/s along +x and +z; up is `lift`; each may be an expression)"));
                None
            }
        };
        let lift = fo.get("lift").and_then(|v| field_num(v, set, &format!("{path}.lift"), "upward m/s", errs));
        let toward = fo.get("toward").and_then(|v| parse_aim(v, set, refs, &format!("{path}.toward"), errs));
        let speed = fo.get("speed").and_then(|v| field_num(v, set, &format!("{path}.speed"), "m/s toward the target (negative pushes away)", errs));
        match (fo.contains_key("toward"), fo.contains_key("velocity"), fo.contains_key("speed")) {
            (true, true, _) => errs.push(format!(
                "{path}: an aimed field (`toward`) pushes along the line to its target at `speed`; give `speed`, not `velocity` (a fixed direction is a `velocity` field without `toward`)"
            )),
            (true, false, false) => errs.push(format!("{path}.speed: missing (m/s toward the target, negative pushes away; a number or an expression over the variables)")),
            (false, _, true) => errs.push(format!("{path}.speed: only an aimed field takes `speed`: add `toward` (a target, or null until a rule aims it), or use `velocity`")),
            (false, false, false) if !fo.contains_key("lift") => {
                errs.push(format!("{path}: give `velocity` [x, z] and/or `lift`, or `toward` + `speed` (what the field pushes toward)"))
            }
            _ => {}
        }
        let props = fo.get("props").and_then(|v| prop_list(v, set, &format!("{path}.props"), errs));
        let rate = match fo.get("rate") {
            None => 10.0,
            Some(v) => match v.as_f64().filter(|r| *r > 0.0 && *r <= 60.0) {
                Some(r) => r as f32,
                None => {
                    errs.push(format!(
                        "{path}.rate: must be a number greater than 0 and at most 60 per second (got {}); leave it out for the default 10",
                        describe_value(v)
                    ));
                    10.0
                }
            },
        };
        if let Some(volume) = volume {
            set.fields.push(Field { id: id.to_string(), volume, velocity, lift, rate, toward, speed, props });
        }
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

/// A `when`: the trigger, and the `prop` filter beside it (`prop_enter` / `prop_exit` / `event`: only these props as the rule's subject).
fn parse_when(v: &Value, set: &RuleSet, refs: &Refs, path: &str, errs: &mut Vec<String>) -> Option<(When, Option<Vec<usize>>)> {
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
    super::rule_input::release_only_on_input(o, key, path, errs);
    let props = match o.get("prop") {
        None => None,
        Some(p) => {
            let event = o.get("event").and_then(Value::as_str);
            let carries = match key {
                "prop_enter" | "prop_exit" => true,
                "event" => event.is_some_and(|e| SUBJECT_EVENTS.contains(&e) || !ENGINE_EVENTS.contains(&e)),
                _ => false,
            };
            if !carries {
                let why = match event {
                    Some(e) if key == "event" => format!("the engine event `{e}` is about no prop (the ones that are: {})", SUBJECT_EVENTS.join(", ")),
                    _ => format!("`{key}` is not about a prop"),
                };
                errs.push(format!("{path}.prop: only `prop_enter`, `prop_exit` and a prop's `event` take a `prop` filter; {why}"));
                return None;
            }
            Some(prop_list(p, set, &format!("{path}.prop"), errs)?)
        }
    };
    let when = match key {
        "start" => When::Start,
        "enter" => When::Enter(parse_volume(&o[key], refs, &sub, errs)?),
        "exit" => When::Exit(parse_volume(&o[key], refs, &sub, errs)?),
        "event" => match o[key].as_str().filter(|s| !s.is_empty()) {
            Some(name) => When::Event(name.to_string()),
            None => {
                errs.push(format!("{sub}: must be an event name (a string)"));
                return None;
            }
        },
        "timer" => When::Timer(timer_ref(o[key].as_str(), set, &sub, errs)?),
        "input" => When::Input(super::rule_input::trigger(&o[key], o.get("release"), &sub, errs)?),
        "every" => When::Every(secs(&o[key], &sub, errs)),
        "after" => When::After(secs(&o[key], &sub, errs)),
        "prop_enter" => When::PropEnter(parse_volume(&o[key], refs, &sub, errs)?),
        "prop_exit" => When::PropExit(parse_volume(&o[key], refs, &sub, errs)?),
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
            When::PropBelow { prop, y: y as f32 }
        }
    };
    Some((when, props))
}

/// A timer by name (`self.name` for a prop's own), as the rules named it (`rule_timers::names` collected them before the rules were parsed).
fn timer_ref(name: Option<&str>, set: &RuleSet, path: &str, errs: &mut Vec<String>) -> Option<TimerRef> {
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        errs.push(format!("{path}: must be a timer's name, like \"fuse\" (or \"self.fuse\" for the subject prop's own)"));
        return None;
    };
    match name.strip_prefix("self.") {
        Some(n) => set.prop_timers.iter().position(|t| t == n).map(TimerRef::Prop),
        None => set.timers.iter().position(|t| t == name).map(TimerRef::Global),
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
            if let Some(its) = p[0].as_str().and_then(|n| n.strip_prefix("self.")) {
                let Some(var) = set.prop_var_names.iter().position(|n| n == its) else {
                    errs.push(format!("{sub}[0]: {}", rules_expr::unknown_prop_var(its, &set.prop_var_names)));
                    return None;
                };
                let value = parse_value(&p[1], set, &format!("{sub}[1]"), errs)?;
                let value = if key.as_str() == "add" { Expr::Bin(Op::Add, Box::new(Expr::Its(var)), Box::new(value)) } else { value };
                return Some(Action::SetIts { var, value });
            }
            if let Some(me) = p[0].as_str().and_then(|n| n.strip_prefix("me.")) {
                let Some(var) = set.player_var_names.iter().position(|n| n == me) else {
                    errs.push(format!("{sub}[0]: {}", rules_expr::unknown_player_var(me, &set.player_var_names)));
                    return None;
                };
                let value = parse_value(&p[1], set, &format!("{sub}[1]"), errs)?;
                let value = if key.as_str() == "add" { Expr::Bin(Op::Add, Box::new(Expr::Me(var)), Box::new(value)) } else { value };
                return Some(Action::SetMe { var, value });
            }
            let var = var_index(p[0].as_str(), names, &format!("{sub}[0]"), errs)?;
            let value = parse_value(&p[1], set, &format!("{sub}[1]"), errs)?;
            let value = if key.as_str() == "add" { Expr::Bin(Op::Add, Box::new(Expr::Var(var)), Box::new(value)) } else { value };
            Some(Action::Set { var, value })
        }
        "timer" => {
            let Some(p) = pair() else {
                errs.push(format!("{sub}: must be [name, seconds], like [\"fuse\", 2]"));
                return None;
            };
            if p[1].as_f64().is_some_and(|x| x < 0.0) {
                errs.push(format!("{sub}[1]: must be 0 seconds or more (got {})", p[1]));
                return None;
            }
            let timer = timer_ref(p[0].as_str(), set, &format!("{sub}[0]"), errs)?;
            Some(Action::Timer { timer, secs: parse_value(&p[1], set, &format!("{sub}[1]"), errs)? })
        }
        "cancel" => Some(Action::Cancel(timer_ref(val.as_str(), set, &sub, errs)?)),
        "emit" | "end" => match val.as_str().filter(|s| !s.is_empty()) {
            Some(s) if key.as_str() == "emit" => Some(Action::Emit(s.to_string())),
            Some(s) => Some(Action::End(s.to_string())),
            None => {
                errs.push(format!("{sub}: must be a name (a string)"));
                None
            }
        },
        "hide" | "show" => {
            let id = name_of(val, &sub, "an object id (or self)", errs)?;
            let obj = if id == SELF {
                Obj::Subject
            } else if refs.object_ids.contains(id) {
                Obj::Id(id.to_string())
            } else {
                errs.push(format!("{sub}: no object `{id}`{}", near(id, refs.object_ids.iter().cloned())));
                return None;
            };
            Some(if key.as_str() == "hide" { Action::Hide(obj) } else { Action::Show(obj) })
        }
        "deactivate" | "activate" => {
            let id = name_of(val, &sub, "a top-level object id", errs)?;
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
            let id = name_of(&pair[0], &format!("{sub}[0]"), "a top-level object id", errs)?;
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
            let object = io.get("object").map(|o| name_of(o, &format!("{sub}.object"), "a top-level object id", errs)).unwrap_or_else(|| {
                errs.push(format!("{sub}.object: missing (the loose prop to shove)"));
                None
            })?;
            let object = if object == SELF {
                Obj::Subject
            } else if refs.bounds.contains_key(object) {
                Obj::Id(object.to_string())
            } else {
                errs.push(format!("{sub}.object: no top-level object `{object}`{}", near(object, refs.bounds.keys().cloned())));
                return None;
            };
            let Some(dir) = io.get("dir").and_then(vec3) else {
                errs.push(format!("{sub}.dir: must be [x, y, z]"));
                return None;
            };
            let speed = match io.get("speed") {
                None => 4.0,
                Some(v) => match v.as_f64().filter(|s| s.is_finite() && *s > 0.0) {
                    Some(s) => s as f32,
                    None => {
                        errs.push(format!(
                            "{sub}.speed: must be a number of m/s greater than 0 (got {}); leave it out for 4, and props never exceed 14",
                            describe_value(v)
                        ));
                        return None;
                    }
                },
            };
            Some(Action::Impulse { object, dir, speed })
        }
        "reset" => match val {
            Value::String(id) if id == SELF => Some(Action::Reset(ResetTarget::Subject)),
            Value::String(id) => prop_index(Some(id), &set.prop_ids, &sub, errs).map(|p| Action::Reset(ResetTarget::Props(vec![p]))),
            Value::Array(a) if !a.is_empty() => {
                let props: Vec<usize> = a.iter().enumerate().filter_map(|(k, v)| prop_index(v.as_str(), &set.prop_ids, &format!("{sub}[{k}]"), errs)).collect();
                (props.len() == a.len()).then_some(Action::Reset(ResetTarget::Props(props)))
            }
            Value::Object(m) => {
                check_keys(errs, &sub, m, &["zone"]);
                let id = m.get("zone").map(|z| name_of(z, &format!("{sub}.zone"), "a zone id", errs)).unwrap_or_else(|| {
                    errs.push(format!("{sub}.zone: missing (the zone whose loose props are put back)"));
                    None
                })?;
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
            let prop = match p[0].as_str() {
                Some(SELF) => PropSel::Subject,
                id => PropSel::Prop(prop_index(id, &set.prop_ids, &format!("{sub}[0]"), errs)?),
            };
            let Some(at) = vec3(&p[1]) else {
                errs.push(format!("{sub}[1]: must be [x, y, z]"));
                return None;
            };
            Some(Action::Place { prop, at })
        }
        "aim" => {
            let Some(p) = pair() else {
                errs.push(format!("{sub}: must be [field_id, target] (target: a loose prop or object id, [x, y, z], self, or null for nowhere)"));
                return None;
            };
            let name = name_of(&p[0], &format!("{sub}[0]"), "a field id", errs)?;
            let Some(field) = set.fields.iter().position(|f| f.id == name) else {
                let ids: Vec<String> = set.fields.iter().map(|f| f.id.clone()).collect();
                let known = if ids.is_empty() { "none: declare one under `fields` with `toward`".to_string() } else { ids.join(", ") };
                errs.push(format!("{sub}[0]: no field `{name}`{} (fields: {known})", near(name, ids.iter().cloned())));
                return None;
            };
            if set.fields[field].toward.is_none() {
                errs.push(format!(
                    "{sub}[0]: field `{name}` pushes along a fixed `velocity`; only an aimed field can be aimed: give it `toward` (null until a rule aims it) and `speed`"
                ));
                return None;
            }
            let to = if p[1].as_str() == Some(SELF) { AimTo::Subject } else { AimTo::At(parse_aim(&p[1], set, refs, &format!("{sub}[1]"), errs)?) };
            Some(Action::Aim { field, to })
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
    if refs.object_ids.contains(SELF) || refs.prop_ids.contains(SELF) {
        errs.push("objects: an object is called `self`, which rules use for the prop that triggered them: rename the object".to_string());
    }
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
    if let Some(pv) = root.get("player_vars") {
        match pv.as_object() {
            None => errs.push("player_vars: must be an object like {\"laps\": 0, \"lives\": 3}".to_string()),
            Some(o) => {
                for (name, v) in o.iter().filter(|(k, _)| !(k.starts_with("x-") || *k == "$comment" || *k == "notes")) {
                    let ok_name = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') && !name.starts_with(|c: char| c.is_ascii_digit());
                    if !ok_name {
                        errs.push(format!("player_vars.{name}: a variable name is letters, digits and `_`, not starting with a digit"));
                        continue;
                    }
                    let init = match v {
                        Value::Number(n) => n.as_f64(),
                        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                        _ => None,
                    };
                    match init {
                        Some(_) if set.player_var_names.len() >= MAX_PLAYER_VARS => {
                            errs.push(format!("player_vars.{name}: at most {MAX_PLAYER_VARS} per-player variables (each player has a copy)"))
                        }
                        Some(x) => {
                            set.player_var_names.push(name.clone());
                            set.player_var_init.push(x);
                        }
                        None => errs.push(format!("player_vars.{name}: must be a number or true/false")),
                    }
                }
            }
        }
    }
    if let Some(pv) = root.get("prop_vars") {
        match pv.as_object() {
            None => errs.push("prop_vars: must be an object like {\"hp\": 3} (every loose prop gets its own copy, `self.hp`)".to_string()),
            Some(_) if set.prop_ids.is_empty() => errs.push("prop_vars: the scene has no loose props (`\"movable\": true`) to give them to".to_string()),
            Some(o) => {
                // As in `vars`: `_name` is a variable, only `x-*` / `$comment` / `notes` are notes (a `_` name used to be dropped silently).
                for (name, v) in o.iter().filter(|(k, _)| !(k.starts_with("x-") || *k == "$comment" || *k == "notes")) {
                    let init = match v {
                        Value::Number(n) => n.as_f64(),
                        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
                        _ => None,
                    };
                    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') || name.starts_with(|c: char| c.is_ascii_digit()) {
                        errs.push(format!("prop_vars.{name}: a variable name is letters, digits and `_`, not starting with a digit"));
                    } else if set.prop_var_names.len() >= MAX_PROP_VARS {
                        errs.push(format!("prop_vars.{name}: at most {MAX_PROP_VARS} per-prop variables (each loose prop has a copy)"));
                    } else if let Some(x) = init {
                        set.prop_var_names.push(name.clone());
                        set.prop_var_init.push(x);
                    } else {
                        errs.push(format!("prop_vars.{name}: must be a number or true/false"));
                    }
                }
            }
        }
    }
    let (timers, prop_timers, timer_errs) = super::rule_timers::names(root.get("rules"));
    for n in timers.iter().chain(&prop_timers) {
        if !super::rule_timers::valid(n) {
            errs.push(format!("rules: `{n}` is not a timer name (letters, digits and `_`, with `self.` for a prop's own)"));
        }
    }
    (set.timers, set.prop_timers) = (timers, prop_timers);
    errs.extend(timer_errs);
    if let Some(p) = root.get("persist") {
        match p.as_array() {
            None => errs.push("persist: must be a list of variable names like [\"days_lived\"]".to_string()),
            Some(list) => {
                for (i, v) in list.iter().enumerate() {
                    match v.as_str() {
                        Some(name) if set.var_names.iter().skip(BUILTIN_VARS.len()).any(|n| n == name) => set.persist.push(name.to_string()),
                        Some(name) => errs.push(format!("persist[{i}]: `{name}` is not one of the scene's vars (declare it under `vars`)")),
                        None => errs.push(format!("persist[{i}]: must be a variable name")),
                    }
                }
            }
        }
    }
    parse_fields(root, refs, &mut set, &mut errs);
    set.needs_props = !set.fields.is_empty();
    let Some(list) = root.get("rules") else {
        parse_phases(root, &mut set, &mut errs);
        return if errs.is_empty() { Ok(set) } else { Err(errs) };
    };
    let Some(list) = list.as_array() else {
        errs.push("rules: must be an array of rules".to_string());
        return Err(errs);
    };
    let mut seen = HashSet::new();
    for (i, rv) in list.iter().enumerate() {
        let Some(ro) = rv.as_object() else {
            errs.push(format!("{}: must be an object", at(i, "")));
            continue;
        };
        let id = id_of(rv, &at(i, ""), "rule", &mut errs);
        let p = at(i, id);
        check_keys(&mut errs, &p, ro, RULE_KEYS);
        if id.is_empty() {
            if rv.get("id").is_none() {
                errs.push(format!("{p}.id: missing (every rule needs a unique id string)"));
            }
        } else if !seen.insert(id.to_string()) {
            errs.push(format!("{p}.id: duplicate rule id `{id}`"));
        }
        let (when, props) = match ro.get("when") {
            Some(w) => match parse_when(w, &set, refs, &format!("{p}.when"), &mut errs) {
                Some((w, props)) => (Some(w), props),
                None => (None, None),
            },
            None => {
                errs.push(format!("{p}.when: missing (one of {})", TRIGGER_KEYS.join(", ")));
                (None, None)
            }
        };
        let who = match ro.get("who") {
            None => Who::Any,
            Some(w) => match w.as_str() {
                Some("any") => Who::Any,
                Some("human") => Who::Human,
                Some("rat") => Who::Rat,
                Some("team1") => Who::Team1,
                Some("team2") => Who::Team2,
                Some(other) => {
                    errs.push(format!("{p}.who: `{other}` is not one of any, human, rat, team1, team2"));
                    Who::Any
                }
                None => {
                    errs.push(format!("{p}.who: must be a string, one of any, human, rat, team1, team2 (got {})", describe_value(w)));
                    Who::Any
                }
            },
        };
        let cond = ro.get("if").and_then(|c| parse_value(c, &set, &format!("{p}.if"), &mut errs));
        let cooldown_ticks = ro.get("cooldown").map_or(0, |c| secs(c, &format!("{p}.cooldown"), &mut errs));
        let once = match ro.get("once") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(other) => {
                errs.push(format!("{p}.once: must be true or false (got {}); write it without quotes", describe_value(other)));
                false
            }
        };
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
        if let Some(when) = &when {
            // A timer or the match start has no player: `me.*` there would read 0 and write nothing, which is never what the author meant.
            let slotless = match when {
                When::Start => Some("start"),
                When::Every(_) => Some("every"),
                When::After(_) => Some("after"),
                When::PropEnter { .. } => Some("prop_enter"),
                When::PropExit { .. } => Some("prop_exit"),
                When::PropBelow { .. } => Some("prop_below"),
                When::Timer(_) => Some("timer"),
                _ => None,
            };
            let uses_me = cond.as_ref().is_some_and(Expr::uses_me)
                || actions.iter().any(|a| match a {
                    Action::SetMe { .. } => true,
                    Action::Set { value, .. } | Action::SetIts { value, .. } | Action::Timer { secs: value, .. } => value.uses_me(),
                    _ => false,
                });
            if let (Some(kind), true) = (slotless, uses_me) {
                errs.push(format!(
                    "{p}: uses `me.` but a `{kind}` trigger has no player (use enter, exit, input, or an event a player causes, such as `kill` or an `emit` from such a rule)"
                ));
            }
            // `teleport` moves the player that triggered the rule, so with no player it silently moved nobody.
            if let (Some(kind), true) = (slotless, actions.iter().any(|a| matches!(a, Action::Teleport(_)))) {
                errs.push(format!(
                    "{p}: `teleport` moves the player that triggered the rule, but a `{kind}` trigger has no player, so it would move nobody (use enter, exit, or an event a player causes, such as `kill` or an `emit` from such a rule)"
                ));
            }
            // `self` is the prop that triggered the rule: a timer, the start or a player's step has none, and neither do the engine events about no prop.
            let subjectless = match when {
                When::Start => Some("a `start` trigger".to_string()),
                When::Every(_) => Some("an `every` trigger".to_string()),
                When::After(_) => Some("an `after` trigger".to_string()),
                When::Enter(_) => Some("an `enter` trigger (a player, not a prop)".to_string()),
                When::Exit(_) => Some("an `exit` trigger (a player, not a prop)".to_string()),
                When::Input(_) => Some("an `input` trigger (a player, not a prop)".to_string()),
                When::Timer(TimerRef::Global(_)) => Some("a scene timer's trigger (only a `self.` timer belongs to a prop)".to_string()),
                When::Event(e) if ENGINE_EVENTS.contains(&e.as_str()) && !SUBJECT_EVENTS.contains(&e.as_str()) => {
                    Some(format!("the engine event `{e}` (it is about no prop; {} are)", SUBJECT_EVENTS.join(", ")))
                }
                _ => None,
            };
            let uses_self = cond.as_ref().is_some_and(Expr::uses_subject)
                || actions.iter().any(|a| match a {
                    Action::Set { value, .. } | Action::SetMe { value, .. } => value.uses_subject(),
                    Action::SetIts { .. } | Action::Timer { timer: TimerRef::Prop(_), .. } | Action::Cancel(TimerRef::Prop(_)) => true,
                    Action::Timer { secs, .. } => secs.uses_subject(),
                    Action::Hide(Obj::Subject) | Action::Show(Obj::Subject) | Action::Reset(ResetTarget::Subject) => true,
                    Action::Impulse { object: Obj::Subject, .. } | Action::Place { prop: PropSel::Subject, .. } | Action::Aim { to: AimTo::Subject, .. } => {
                        true
                    }
                    _ => false,
                });
            if let (Some(kind), true) = (subjectless, uses_self) {
                errs.push(format!(
                    "{p}: uses `self` but {kind} has no prop (self is the prop that fired the rule: use prop_enter, prop_exit, prop_below, or the event pickup, drop or prop_hit)"
                ));
            }
        }
        if let Some(when) = when {
            set.rules.push(Rule { id: id.to_string(), when, who, cond, once, cooldown_ticks, props, actions });
        }
    }
    parse_phases(root, &mut set, &mut errs);
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
    // A field reads every prop it pushes, every tick (this used to be overwritten here, so a scene whose rules never looked at a prop got
    // no prop views and its fields pushed nothing).
    set.needs_props = !set.fields.is_empty()
        || set.rules.iter().any(|r| {
            matches!(r.when, When::PropEnter(_) | When::PropExit(_) | When::PropBelow { .. })
                || r.cond.as_ref().is_some_and(Expr::reads_world)
                || r.actions.iter().any(|a| match a {
                    Action::Set { value, .. } | Action::SetMe { value, .. } | Action::SetIts { value, .. } | Action::Timer { secs: value, .. } => {
                        value.reads_world()
                    }
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

/// `phases`: `{"gate_open": ["open_gate"], "done": ["open_gate", "win"]}`. Each name lists rule ids the analysis tools assume have fired.
fn parse_phases(root: &Map<String, Value>, set: &mut RuleSet, errs: &mut Vec<String>) {
    let Some(v) = root.get("phases") else { return };
    let Some(o) = v.as_object() else {
        errs.push("phases: must be an object like {\"gate_open\": [\"open_gate\"]} (a phase name, then the rule ids assumed to have fired)".to_string());
        return;
    };
    for (name, list) in o.iter().filter(|(k, _)| !(k.starts_with("x-") || *k == "$comment" || *k == "notes")) {
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') || name == "initial" {
            errs.push(format!("phases.{name}: a phase name is letters, digits, `_` or `-` (and `initial` is the implicit first state)"));
            continue;
        }
        let Some(ids) = list.as_array() else {
            errs.push(format!("phases.{name}: must be an array of rule ids"));
            continue;
        };
        let mut rules = Vec::new();
        for (k, id) in ids.iter().enumerate() {
            let Some(id) = id.as_str() else {
                errs.push(format!("phases.{name}[{k}]: must be a rule id string"));
                continue;
            };
            match set.rules.iter().position(|r| r.id == id) {
                Some(i) => rules.push(i),
                None => errs.push(format!("phases.{name}[{k}]: no rule `{id}`{}", near(id, set.rules.iter().map(|r| r.id.clone())))),
            }
        }
        set.phases.push(Phase { name: name.clone(), rules });
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

    fn gate_rules() -> Value {
        json!({
            "vars": {},
            "rules": [
                {"id": "boot", "when": {"start": true}, "do": [{"collision": ["crate", false]}]},
                {"id": "open_door", "when": {"event": "go"}, "do": [{"deactivate": "door"}]},
                {"id": "shut_door", "when": {"event": "go"}, "do": [{"activate": "door"}, {"emit": "go"}]},
                {"id": "open_crate", "when": {"event": "go"}, "if": "1 == 1", "do": [{"collision": ["crate", true]}]}
            ],
            "phases": {"open": ["open_door"], "reopened": ["open_door", "shut_door"], "crate_back": ["open_crate"]}
        })
    }

    #[test]
    fn phases_name_the_objects_their_rules_open_in_order() {
        let set = parse(gate_rules()).unwrap();
        assert_eq!(set.phases.len(), 3);
        assert_eq!(set.open_objects(None).unwrap(), vec!["crate".to_string()], "initial: only the unconditional start rule counts");
        assert_eq!(set.open_objects(Some("open")).unwrap(), vec!["crate".to_string(), "door".to_string()], "deactivate opens");
        assert_eq!(set.open_objects(Some("reopened")).unwrap(), vec!["crate".to_string()], "a later activate closes it again");
        assert!(set.open_objects(Some("crate_back")).unwrap().is_empty(), "collision true closes what start opened");
    }

    #[test]
    fn an_unknown_phase_names_the_declared_ones_and_a_bad_phase_entry_is_a_validate_error() {
        let set = parse(gate_rules()).unwrap();
        let e = set.open_objects(Some("opn")).unwrap_err();
        assert!(e.contains("did you mean `open`") && e.contains("declared: "), "{e}");
        assert!(RuleSet::default().open_objects(Some("x")).unwrap_err().contains("add \"phases\""));
        let mut bad = gate_rules();
        bad["phases"] = json!({"open": ["open_dor"], "initial": [], "n": "open_door", "ok": []});
        let e = parse(bad).unwrap_err().join("\n");
        assert!(e.contains("phases.open[0]: no rule `open_dor`") && e.contains("did you mean `open_door`"), "{e}");
        assert!(e.contains("phases.initial") && e.contains("phases.n: must be an array"), "{e}");
        // phases without any rules are still validated
        let e = parse(json!({"phases": {"p": ["nope"]}})).unwrap_err().join("\n");
        assert!(e.contains("no rule `nope`"), "{e}");
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
    fn who_accepts_team1_and_team2_and_rejects_anything_else() {
        let set = parse(json!({
            "vars": {},
            "rules": [
                {"id": "a", "when": {"every": 1.0}, "who": "team1", "do": [{"emit": "a"}]},
                {"id": "b", "when": {"every": 1.0}, "who": "team2", "do": [{"emit": "b"}]}
            ]
        }))
        .unwrap();
        assert_eq!((set.rules[0].who, set.rules[1].who), (Who::Team1, Who::Team2));
        let e = parse(json!({"vars": {}, "rules": [{"id": "a", "when": {"every": 1.0}, "who": "team3", "do": [{"emit": "a"}]}]})).unwrap_err();
        assert!(e.iter().any(|m| m.starts_with("rules[0] (a).who:") && m.contains("team1, team2")), "{e:?}");
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
        let When::PropEnter(volume) = &set.rules[0].when else { panic!("{:?}", set.rules[0].when) };
        assert_eq!((volume.min.y, volume.max.y), (-3.0, 0.0));
        assert_eq!(set.rules[0].props, None);
        assert!(matches!(set.rules[1].when, When::PropExit(_)) && set.rules[1].props == Some(vec![1]), "crate is prop_ids[1]");
        assert!(matches!(set.rules[2].when, When::PropBelow { prop: 0, y } if y == 0.5));
        assert_eq!(
            set.rules[3].actions,
            [
                Action::Reset(ResetTarget::Props(vec![0])),
                Action::Reset(ResetTarget::Props(vec![1, 0])),
                Action::Reset(ResetTarget::Zone(1)),
                Action::Place { prop: PropSel::Prop(1), at: Vec3::new(1.0, 2.0, 3.0) }
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
            "rules[1] (b).when.prop: only `prop_enter`, `prop_exit` and a prop's `event` take a `prop` filter; `enter` is not about a prop",
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

    #[test]
    fn player_vars_parse_and_me_resolves_in_conditions_and_actions() {
        let set = parse(json!({
            "vars": {"total": 0},
            "player_vars": {"laps": 0, "lives": 3, "alive": true},
            "rules": [{"id": "lap", "when": {"enter": {"zone": "exit"}}, "if": "me.laps < 3 && me.alive", "do": [{"add": ["me.laps", 1]}, {"set": ["total", "me.laps * 10"]}]}]
        }))
        .unwrap();
        assert_eq!(
            (set.player_var_names.as_slice(), set.player_var_init.as_slice()),
            (&["laps".to_string(), "lives".to_string(), "alive".to_string()][..], &[0.0, 3.0, 1.0][..])
        );
        assert!(matches!(set.rules[0].actions[0], Action::SetMe { var: 0, .. }));
        assert!(set.rules[0].cond.as_ref().is_some_and(Expr::uses_me));
        assert!(!set.needs_props, "me.* does not need the props described to the rules");
    }

    #[test]
    fn player_var_mistakes_name_the_fix() {
        let e = parse(json!({
            "player_vars": {"laps": 0, "bad name": 1, "x": "s"},
            "rules": [
                {"id": "a", "when": {"enter": {"zone": "exit"}}, "if": "me.lapz > 1", "do": [{"add": ["me.lap", 1]}]},
                {"id": "b", "when": {"every": 1.0}, "do": [{"add": ["me.laps", 1]}]},
                {"id": "c", "when": {"start": true}, "if": "me.laps == 0", "do": [{"emit": "x"}]},
                {"id": "d", "when": {"prop_below": ["bell", 0.5]}, "do": [{"add": ["me.laps", 1]}]}
            ]
        }))
        .unwrap_err()
        .join("\n");
        for want in [
            "player_vars.bad name: a variable name is letters",
            "player_vars.x: must be a number or true/false",
            "unknown per-player variable `me.lapz` — did you mean `me.laps`?",
            "unknown per-player variable `me.lap` — did you mean `me.laps`?",
            "rules[1] (b): uses `me.` but a `every` trigger has no player",
            "rules[2] (c): uses `me.` but a `start` trigger has no player",
            "rules[3] (d): uses `me.` but a `prop_below` trigger has no player",
        ] {
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
        let e = parse(json!({"vars": {}, "rules": [
            {"id": "gasp", "when": {"every": 5.0}, "do": [{"teleport": [0, 0, 0]}]},
            {"id": "ok", "when": {"enter": {"zone": "exit"}}, "do": [{"teleport": [0, 0, 0]}]}
        ]}))
        .unwrap_err()
        .join("\n");
        assert!(e.contains("rules[0] (gasp): `teleport` moves the player that triggered the rule, but a `every` trigger has no player"), "{e}");
        assert!(!e.contains("rules[1]"), "{e}");
        let many: serde_json::Map<String, Value> = (0..=MAX_PLAYER_VARS).map(|i| (format!("v{i}"), json!(0))).collect();
        let e = parse(json!({"player_vars": many})).unwrap_err().join("\n");
        assert!(e.contains(&format!("at most {MAX_PLAYER_VARS} per-player variables")), "{e}");
        assert!(parse(json!({"vars": {"laps": 0}, "player_vars": {"laps": 0}})).is_ok(), "a per-player name may match a global one: `me.` tells them apart");
    }

    /// One rule with the given `when` and `do`, parsed against the test scene; `Err` is the list of messages.
    fn one(extra: Value, when: Value, actions: Value) -> Result<RuleSet, Vec<String>> {
        let mut rule = json!({"id": "r", "when": when, "do": actions});
        for (k, v) in extra.as_object().unwrap() {
            rule[k] = v.clone();
        }
        parse(json!({"vars": {"n": 0}, "rules": [rule]}))
    }

    fn messages(r: Result<RuleSet, Vec<String>>) -> String {
        match r {
            Err(e) => e.join(" | "),
            Ok(_) => panic!("must be rejected"),
        }
    }

    #[test]
    fn a_negative_or_non_numeric_pad_is_an_error_with_the_fix_not_a_silent_zero() {
        for (pad, got) in [(json!(-0.55), "-0.55"), (json!("big"), "string \"big\"")] {
            let e = messages(one(json!({}), json!({"enter": {"object": "coin_1", "pad": pad}}), json!([{"emit": "x"}])));
            assert!(e.contains("rules[0] (r).when.enter.pad: must be a number of metres, 0 or more"), "{e}");
            assert!(e.contains(got) && e.contains("only grows a volume") && e.contains("smaller `box`"), "{e}");
        }
        assert!(one(json!({}), json!({"enter": {"object": "coin_1", "pad": 0.4}}), json!([{"emit": "x"}])).is_ok());
        assert!(one(json!({}), json!({"enter": {"object": "coin_1", "pad": 0}}), json!([{"emit": "x"}])).is_ok());
    }

    #[test]
    fn a_zone_height_that_is_not_a_positive_number_is_an_error_with_the_default_named() {
        for height in [json!(-1), json!(0), json!("tall")] {
            let e = messages(one(json!({}), json!({"enter": {"zone": "exit", "height": height}}), json!([{"emit": "x"}])));
            assert!(e.contains(".height: must be a number of metres greater than 0") && e.contains("default 3"), "{e}");
        }
        assert!(one(json!({}), json!({"enter": {"zone": "exit", "height": 2.5}}), json!([{"emit": "x"}])).is_ok());
    }

    #[test]
    fn once_who_and_the_impulse_speed_must_be_what_they_say() {
        let e = messages(one(json!({"once": "yes"}), json!({"start": true}), json!([{"emit": "x"}])));
        assert!(e.contains("rules[0] (r).once: must be true or false (got string \"yes\")"), "{e}");
        let e = messages(one(json!({"who": 3}), json!({"start": true}), json!([{"emit": "x"}])));
        assert!(e.contains(".who: must be a string, one of any, human, rat, team1, team2 (got number 3)"), "{e}");
        for speed in [json!("fast"), json!(-2), json!(0)] {
            let e = messages(one(json!({}), json!({"start": true}), json!([{"impulse": {"object": "crate", "dir": [0, 1, 0], "speed": speed}}])));
            assert!(e.contains(".impulse.speed: must be a number of m/s greater than 0") && e.contains("leave it out for 4"), "{e}");
        }
        assert!(one(json!({"once": true, "who": "human"}), json!({"start": true}), json!([{"impulse": {"object": "crate", "dir": [0, 1, 0]}}])).is_ok());
    }

    #[test]
    fn an_id_that_is_not_a_string_is_said_so_instead_of_reporting_no_object_with_an_empty_name() {
        for action in [json!({"hide": 5}), json!({"deactivate": ["door"]}), json!({"collision": [7, true]})] {
            let e = messages(one(json!({}), json!({"start": true}), json!([action])));
            assert!(e.contains("must be a") && e.contains("(a string), got"), "{e}");
            assert!(!e.contains("``"), "{e}");
        }
        let e = messages(one(json!({}), json!({"start": true}), json!([{"impulse": {"dir": [0, 1, 0]}}])));
        assert!(e.contains(".impulse.object: missing"), "{e}");
        let e = messages(parse(json!({"rules": [{"id": 12, "when": {"start": true}, "do": [{"emit": "x"}]}]})));
        assert!(e.contains("rules[0].id: must be a string, a unique id for the rule (got number 12)"), "{e}");
    }

    #[test]
    fn a_field_rate_that_is_not_a_number_in_range_is_an_error() {
        let field = |rate: Value| parse(json!({"fields": [{"id": "river", "zone": "exit", "velocity": [0, 1], "rate": rate}]}));
        for rate in [json!("fast"), json!(0), json!(61)] {
            let e = messages(field(rate));
            assert!(e.contains("fields[0] (river).rate: must be a number greater than 0 and at most 60 per second") && e.contains("default 10"), "{e}");
        }
        assert!(field(json!(12)).is_ok());
    }

    #[test]
    fn an_aimed_field_parses_its_target_speed_expression_and_props() {
        let set = parse(json!({
            "vars": {"dial": 1},
            "fields": [
                {"id": "gravity", "zone": "pit", "toward": null, "speed": "(dial == 1) * 4 - (dial == 3) * 3", "props": ["crate"], "rate": 6},
                {"id": "magnet", "zone": "pit", "toward": "bell", "speed": 2},
                {"id": "drain", "zone": "pit", "toward": "coin_1", "speed": -1, "lift": 0.5},
                {"id": "point", "zone": "pit", "toward": [1, 0, 2], "speed": 3},
                {"id": "river", "zone": "pit", "velocity": ["dial * 2", 0]}
            ],
            "rules": [{"id": "r", "when": {"event": "pickup", "prop": ["bell", "crate"]},
                       "do": [{"aim": ["gravity", "self"]}, {"aim": ["magnet", null]}, {"aim": ["point", "coin_1"]}, {"aim": ["drain", [0, 0, 9]]}]}]
        }))
        .unwrap();
        let f = &set.fields;
        assert_eq!((f[0].toward, f[0].props.clone(), f[0].rate), (Some(Aim::Off), Some(vec![1]), 6.0));
        assert_eq!(f[0].speed.as_ref().map(|e| e.eval(&[0.0, 0.0, 0.0, 3.0])), Some(-3.0), "the speed is an expression over the vars");
        assert_eq!(f[1].toward, Some(Aim::Prop(0)), "a loose prop is followed: bell is prop_ids[0]");
        assert_eq!(f[2].toward, Some(Aim::Point(Vec3::new(1.2, 0.2, 1.2))), "a fixed object is aimed at its centre");
        assert_eq!(f[3].toward, Some(Aim::Point(Vec3::new(1.0, 0.0, 2.0))));
        assert_eq!(f[4].toward, None, "a velocity field is not aimable");
        assert!(set.needs_props, "fields read the props");
        assert_eq!(set.rules[0].props, Some(vec![0, 1]));
        assert_eq!(
            set.rules[0].actions,
            [
                Action::Aim { field: 0, to: AimTo::Subject },
                Action::Aim { field: 1, to: AimTo::At(Aim::Off) },
                Action::Aim { field: 3, to: AimTo::At(Aim::Point(Vec3::new(1.2, 0.2, 1.2))) },
                Action::Aim { field: 2, to: AimTo::At(Aim::Point(Vec3::new(0.0, 0.0, 9.0))) },
            ]
        );
    }

    #[test]
    fn every_aimed_field_mistake_names_the_fix() {
        let e = messages(parse(json!({
            "vars": {"n": 0},
            "player_vars": {"lap": 0},
            "fields": [
                {"id": "a", "zone": "pit", "toward": "bel", "speed": 1},
                {"id": "b", "zone": "pit", "toward": "bell", "velocity": [1, 0], "speed": 1},
                {"id": "c", "zone": "pit", "toward": "bell"},
                {"id": "d", "zone": "pit", "velocity": [1, 0], "speed": 2},
                {"id": "e", "zone": "pit", "toward": null, "speed": "held(self) * 2", "props": []},
                {"id": "f", "zone": "pit", "toward": null, "speed": "me.lap", "props": ["crate", "crate"]},
                {"id": "g", "zone": "pit", "toward": 5, "speed": {"fast": true}, "props": ["crat"]},
                {"id": "h", "zone": "pit", "velocity": ["nn", 0]}
            ]
        })));
        for want in [
            "fields[0] (a).toward: no loose prop or top-level object `bel` — did you mean `bell`?",
            "fields[1] (b): an aimed field (`toward`) pushes along the line to its target at `speed`; give `speed`, not `velocity`",
            "fields[2] (c).speed: missing (m/s toward the target, negative pushes away",
            "fields[3] (d).speed: only an aimed field takes `speed`: add `toward`",
            "fields[4] (e).speed: a field is not run for a player or a prop: `me.` and `self` belong in rules",
            "fields[4] (e).props: an empty list selects nothing",
            "fields[5] (f).speed: a field is not run for a player or a prop",
            "fields[5] (f).props[1]: `crate` is listed twice",
            "fields[6] (g).toward: must be a loose prop or object id, [x, y, z], or null for nowhere (got number 5)",
            "fields[6] (g).speed: must be m/s toward the target (negative pushes away), a number or an expression string",
            "fields[6] (g).props[0]: no loose prop `crat` — did you mean `crate`?",
            "fields[7] (h).velocity[0]: unknown variable `nn` — did you mean `n`?",
        ] {
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
        let many: Vec<Value> = (0..=MAX_FIELDS).map(|i| json!({"id": format!("f{i}"), "zone": "pit", "velocity": [1, 0]})).collect();
        let e = messages(parse(json!({"fields": many})));
        assert!(e.contains(&format!("at most {MAX_FIELDS}")), "{e}");
    }

    #[test]
    fn aim_names_an_aimed_field_and_a_target() {
        let base = |action: Value| {
            parse(json!({
                "fields": [{"id": "gravity", "zone": "pit", "toward": null, "speed": 2}, {"id": "river", "zone": "pit", "velocity": [1, 0]}],
                "rules": [{"id": "r", "when": {"event": "pickup"}, "do": [action]}]
            }))
        };
        assert!(base(json!({"aim": ["gravity", "crate"]})).is_ok());
        for (action, want) in [
            (json!({"aim": ["gravty", "crate"]}), "do[0].aim[0]: no field `gravty` — did you mean `gravity`? (fields: gravity, river)"),
            (json!({"aim": ["river", "crate"]}), "do[0].aim[0]: field `river` pushes along a fixed `velocity`; only an aimed field can be aimed"),
            (json!({"aim": ["gravity", "crat"]}), "do[0].aim[1]: no loose prop or top-level object `crat` — did you mean `crate`?"),
            (json!({"aim": "gravity"}), "do[0].aim: must be [field_id, target]"),
        ] {
            let e = messages(base(action));
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
        let e = messages(parse(json!({"rules": [{"id": "r", "when": {"start": true}, "do": [{"aim": ["x", null]}]}]})));
        assert!(e.contains("no field `x`") && e.contains("declare one under `fields` with `toward`"), "{e}");
    }

    #[test]
    fn self_is_the_triggering_prop_and_only_a_prop_trigger_has_one() {
        let set = parse(json!({
            "vars": {"n": 0},
            "rules": [
                {"id": "in", "when": {"prop_enter": {"zone": "pit"}, "prop": ["crate", "bell"]}, "if": "!held(self) && in_zone(self, pit)",
                 "do": [{"hide": "self"}, {"reset": "self"}, {"place": ["self", [0, 1, 0]]}, {"impulse": {"object": "self", "dir": [0, 1, 0]}}, {"show": "self"}, {"emit": "landed"}]},
                {"id": "chain", "when": {"event": "landed"}, "do": [{"set": ["n", "prop_y(self)"]}]},
                {"id": "below", "when": {"prop_below": ["bell", 0]}, "do": [{"reset": "self"}]},
                {"id": "took", "when": {"event": "pickup", "prop": "bell"}, "do": [{"add": ["n", "mass(self)"]}]}
            ]
        }))
        .unwrap();
        let r = &set.rules[0];
        assert_eq!(r.props, Some(vec![1, 0]), "the filter keeps the author's order");
        assert!(r.cond.as_ref().is_some_and(Expr::uses_subject));
        assert_eq!(r.actions[0], Action::Hide(Obj::Subject));
        assert_eq!(r.actions[1], Action::Reset(ResetTarget::Subject));
        assert_eq!(r.actions[2], Action::Place { prop: PropSel::Subject, at: Vec3::new(0.0, 1.0, 0.0) });
        assert!(matches!(r.actions[3], Action::Impulse { object: Obj::Subject, .. }));
        assert_eq!(set.rules[3].props, Some(vec![0]), "one id is a list of one");
        let e = messages(parse(json!({
            "vars": {"n": 0},
            "rules": [
                {"id": "a", "when": {"start": true}, "do": [{"hide": "self"}]},
                {"id": "b", "when": {"every": 1}, "if": "held(self)", "do": [{"emit": "x"}]},
                {"id": "c", "when": {"enter": {"zone": "exit"}}, "do": [{"reset": "self"}]},
                {"id": "d", "when": {"event": "shot"}, "do": [{"set": ["n", "tilt(self)"]}]},
                {"id": "e", "when": {"event": "shot", "prop": "crate"}, "do": [{"emit": "x"}]},
                {"id": "f", "when": {"every": 1, "prop": "crate"}, "do": [{"emit": "x"}]},
                {"id": "g", "when": {"prop_enter": {"zone": "pit"}, "prop": []}, "do": [{"emit": "x"}]},
                {"id": "h", "when": {"after": 1}, "do": [{"place": ["self", [0, 0, 0]]}]}
            ]
        })));
        for want in [
            "rules[0] (a): uses `self` but a `start` trigger has no prop (self is the prop that fired the rule",
            "rules[1] (b): uses `self` but an `every` trigger has no prop",
            "rules[2] (c): uses `self` but an `enter` trigger (a player, not a prop) has no prop",
            "rules[3] (d): uses `self` but the engine event `shot` (it is about no prop; pickup, drop, prop_hit are) has no prop",
            "rules[4] (e).when.prop: only `prop_enter`, `prop_exit` and a prop's `event` take a `prop` filter; the engine event `shot` is about no prop",
            "rules[5] (f).when.prop: only `prop_enter`, `prop_exit` and a prop's `event` take a `prop` filter; `every` is not about a prop",
            "rules[6] (g).when.prop: an empty list selects nothing",
            "rules[7] (h): uses `self` but an `after` trigger has no prop",
        ] {
            assert!(e.contains(want), "missing `{want}` in:\n{e}");
        }
        // A scene without loose props has no `self` to read: the prop functions still say so.
        let e = messages(parse_rules(
            json!({"vars": {"n": 0}, "rules": [{"id": "x", "when": {"event": "pickup"}, "do": [{"set": ["n", "held(self)"]}]}]}).as_object().unwrap(),
            &Refs::default(),
        ));
        assert!(e.contains("no loose prop `self`") && e.contains("none in this scene"), "{e}");
        // `self` is reserved: no object (a loose prop or anything a rule can hide) may take the name.
        for prop in [true, false] {
            let mut refs = refs();
            if prop {
                refs.prop_ids.insert("self".into());
            }
            refs.object_ids.insert("self".into());
            let e = messages(parse_rules(json!({}).as_object().unwrap(), &refs));
            assert!(e.contains("an object is called `self`"), "{e}");
        }
    }

    #[test]
    fn a_scene_whose_fields_exist_and_whose_rules_never_look_at_a_prop_still_needs_the_props() {
        let set = parse(json!({
            "vars": {"n": 0},
            "fields": [{"id": "river", "zone": "pit", "velocity": [1, 0]}],
            "rules": [{"id": "t", "when": {"every": 1}, "do": [{"add": ["n", 1]}]}]
        }))
        .unwrap();
        assert!(set.needs_props, "the fields push props every tick: without views they silently pushed nothing");
    }
}
