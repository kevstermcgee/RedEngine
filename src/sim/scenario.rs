//! Scripted, headless play-throughs: a **scenario** drives simulated players through a map at 60 Hz and asserts what
//! the game did, with no window, GPU or socket. This is how an AI proves a game rule works ("walk to the coin, walk to
//! the exit: victory, score 3") in one command, and how a scene's `checks.sim` guards its gameplay in `verify`.
//!
//! ```json
//! { "name": "collect the coin, then win",
//!   "players": [ { "id": "p1", "character": "human", "spawn": "spawn_a" } ],
//!   "script": [ { "player": "p1", "walk": "0,-6; 4,0" }, { "player": "p1", "wait": 0.5 } ],
//!   "max_seconds": 30,
//!   "expect": [ { "event": "coin", "count": 1 }, { "var": "score", "gte": 1 }, { "ended": "victory" },
//!               { "player": "p1", "near": [4, 0], "tol": 0.8 } ] }
//! ```
//!
//! Each player runs its own steps in order (players run in parallel): `walk "x,z; x,z"` steers through waypoints with
//! the real per-tick movement, `wait secs` stands still, `hold {forward, strafe, sprint, crouch, jump, yaw_deg,
//! seconds}` presses inputs. `approach "id"`, `look_at "id"` and `interact "id"` do the same by object instead of by coordinates
//! (see [`super::approach`]; the graphical client's script has the same three). The run ends when the match ends (an `end` rule), when every script is done (plus a short
//! settle), or at `max_seconds`. Every mistake in a scenario is reported with a path and a did-you-mean.

use super::approach::{self, default_within, failure_message, Approach, Progress, Target};
use super::match_sim::MatchSim;
use super::player::{PlayerInput, PlayerState};
use super::rules::RuleSet;
use super::rules_run::{prop_inside, GameEvent};
use super::spawns::Spawn;
use super::trace::{Header, Trace};
use crate::player::Character;
use crate::strict::check_keys;
use glam::{Vec2, Vec3};
use serde_json::{json, Map, Value};

const SCENARIO_KEYS: &[&str] = &["name", "spawn_group", "players", "script", "max_seconds", "settle_seconds", "expect"];
const PLAYER_KEYS: &[&str] = &["id", "character", "spawn"];
const STEP_KEYS: &[&str] = &["player", "walk", "wait", "hold", "approach", "look_at", "interact", "within", "timeout", "until_event"];
const HOLD_KEYS: &[&str] =
    &["forward", "strafe", "sprint", "crouch", "jump", "yaw_deg", "pitch_deg", "look_at", "interact", "attack", "reload", "switch", "seconds"];
const EXPECT_KEYS: &[&str] = &[
    "event",
    "no_event",
    "var",
    "ended",
    "not_ended",
    "hidden",
    "shown",
    "collision_disabled",
    "collision_enabled",
    "player",
    "count",
    "min",
    "max",
    "eq",
    "ne",
    "gt",
    "gte",
    "lt",
    "lte",
    "near",
    "tol",
    "y",
    "prop",
    "in_zone",
    "not_in_zone",
    "below_y",
    "y_lt",
    "y_gt",
    "tilt_gt",
    "tilt_lt",
    "moved",
    "held_by",
];
/// The checks a `{prop: id, ...}` expectation may make (exactly one).
const PROP_CHECK_KEYS: &[&str] = &["in_zone", "not_in_zone", "below_y", "y_lt", "y_gt", "tilt_gt", "tilt_lt", "moved", "near", "held_by"];
/// A prop counts as moved once its origin is this far from where the map put it.
const MOVED_M: f32 = 0.05;
/// A walk leg is abandoned (and the scenario fails) after this many ticks without getting 0.2 m closer.
const STUCK_TICKS: u32 = 120;
const REACHED: f32 = 0.25;

/// A comparison of a variable against a number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmp {
    /// `eq`
    Eq,
    /// `ne`
    Ne,
    /// `gt`
    Gt,
    /// `gte`
    Ge,
    /// `lt`
    Lt,
    /// `lte`
    Le,
}

impl Cmp {
    fn holds(self, a: f64, b: f64) -> bool {
        match self {
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
        }
    }
    fn word(self) -> &'static str {
        match self {
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
        }
    }
}

/// One scripted action of one player.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Steer through these `(x, z)` waypoints.
    Walk(Vec<Vec2>),
    /// Stand still this many ticks.
    Wait(u32),
    /// Walk straight at a top-level object until within `within` metres of it (default: from the body's pickup reach).
    Approach {
        /// The object's id.
        object: String,
        /// Done at this horizontal gap.
        within: Option<f32>,
        /// Seconds before it fails.
        timeout: f32,
    },
    /// Face the middle of a top-level object (yaw and pitch from the eye), then go on.
    LookAt {
        /// The object's id.
        object: String,
    },
    /// Approach, look at the object, press interact, and (for a loose prop) check it is now carried.
    Interact {
        /// The object's id.
        object: String,
        /// Approach distance, as for [`Action::Approach`].
        within: Option<f32>,
        /// Seconds the approach may take.
        timeout: f32,
    },
    /// Hold an input for `ticks`.
    Hold {
        /// The input (its `seq` is set by the runner).
        input: PlayerInput,
        /// How long.
        ticks: u32,
        /// No `yaw_deg` was given: keep looking where the player already looks.
        keep_yaw: bool,
        /// No `pitch_deg` was given: keep the current pitch.
        keep_pitch: bool,
        /// Aim at this world point every tick (yaw and pitch from the player's eye), instead of `yaw_deg` / `pitch_deg`.
        look_at: Option<Vec3>,
    },
}

/// A simulated player.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSpec {
    /// Name used by steps and expectations.
    pub id: String,
    /// Human or rat.
    pub character: Character,
    /// A spawn point id to start at (`None` = the next spawn in order).
    pub spawn: Option<String>,
}

/// Something a finished run must satisfy.
#[derive(Debug, Clone, PartialEq)]
pub enum Expect {
    /// A named event happened between `min` and `max` times.
    Event {
        /// Event name.
        name: String,
        /// Fewest times.
        min: usize,
        /// Most times.
        max: Option<usize>,
    },
    /// A named event never happened.
    NoEvent(String),
    /// A variable compares as given.
    Var {
        /// Variable name.
        name: String,
        /// Comparison.
        cmp: Cmp,
        /// Value.
        value: f64,
    },
    /// The match ended with this outcome (`None`: it must still be running).
    Ended(Option<String>),
    /// An object is (`true`) or is not (`false`) hidden.
    Hidden(String, bool),
    /// A top-level object's collision is disabled (`true`) or enabled (`false`).
    CollisionDisabled(String, bool),
    /// A player ended within `tol` metres (in x/z) of a point, and optionally at a floor height.
    PlayerNear {
        /// Player id.
        player: String,
        /// Point.
        at: Vec2,
        /// Distance allowed.
        tol: f32,
        /// Floor height, if it matters.
        y: Option<f32>,
    },
    /// A loose prop ended in a given state (ADR 2026-09-29-prop-aware-rules-and-scenarios).
    Prop {
        /// The prop's object id.
        id: String,
        /// What about it.
        check: PropCheck,
    },
}

/// What a `{prop: id, ...}` expectation checks about the prop's rest state.
#[derive(Debug, Clone, PartialEq)]
pub enum PropCheck {
    /// Its origin is inside (`true`) / not inside the zone (see [`prop_inside`]).
    InZone(String, bool),
    /// Its origin height compares as given (`below_y` / `y_lt`: less than; `y_gt`: greater than).
    Y(Cmp, f32),
    /// Its tilt from the authored orientation, degrees, compares as given.
    Tilt(Cmp, f32),
    /// It moved (`true`: origin more than 5 cm from where the map put it) or did not.
    Moved(bool),
    /// Its origin is within `tol` m (x/z) of a point, optionally at a height (within 0.2 m).
    Near {
        /// Point.
        at: Vec2,
        /// Distance allowed.
        tol: f32,
        /// Height, if it matters.
        y: Option<f32>,
    },
    /// A player carries it (`None`: nobody does).
    HeldBy(Option<String>),
}

/// Where a loose prop ended (`sim` reports every loose prop, like `finals` for players).
#[derive(Debug, Clone, PartialEq)]
pub struct PropFinal {
    /// Object id.
    pub id: String,
    /// Its origin, world space.
    pub pos: Vec3,
    /// Degrees from its authored orientation.
    pub tilt_deg: f32,
    /// Metres from where the map put it.
    pub moved: f32,
    /// At rest (or never disturbed).
    pub asleep: bool,
    /// The scenario player carrying it, if any.
    pub held_by: Option<String>,
}

/// A parsed scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    /// Its name.
    pub name: String,
    /// Restrict spawn points to this group (`""` = all).
    pub spawn_group: String,
    /// The players.
    pub players: Vec<PlayerSpec>,
    /// `(player index, action, until_event)` in script order: a step also ends as soon as its `until_event` is emitted.
    pub script: Vec<(usize, Action, Option<String>)>,
    /// Hard time limit, in ticks.
    pub max_ticks: u64,
    /// Ticks to keep simulating after every script has finished.
    pub settle_ticks: u64,
    /// What must be true afterwards.
    pub expect: Vec<Expect>,
}

/// One checked expectation.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// What was checked.
    pub label: String,
    /// Whether it held.
    pub ok: bool,
    /// The evidence.
    pub detail: String,
}

/// The result of running a scenario.
#[derive(Debug, Clone)]
pub struct ScenarioResult {
    /// The scenario's name.
    pub name: String,
    /// Every expectation held and nothing got stuck.
    pub passed: bool,
    /// Ticks simulated.
    pub ticks: u64,
    /// One entry per expectation (plus a `walk` entry per stuck player).
    pub outcomes: Vec<Outcome>,
    /// Every game event, in order.
    pub events: Vec<GameEvent>,
    /// The scene's variables at the end.
    pub vars: Vec<(String, f64)>,
    /// The match outcome, if it ended.
    pub ended: Option<String>,
    /// Where each player ended `(id, x, z, foot_y)`.
    pub finals: Vec<(String, f32, f32, f32)>,
    /// Where every loose prop ended.
    pub props: Vec<PropFinal>,
    /// Why the first few empty-handed `interact`s picked nothing up (`"tick 105: p1: ..."`); empty when every one worked.
    pub pickup_misses: Vec<String>,
    /// The exact checksum at the end (compare two runs).
    pub checksum: u64,
    /// The recording, when one was requested.
    pub trace: Option<Trace>,
}

impl ScenarioResult {
    /// The result as JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "passed": self.passed,
            "ticks": self.ticks,
            "seconds": self.ticks as f64 / super::clock::TICK_RATE_HZ as f64,
            "ended": self.ended,
            "checksum": format!("{:016x}", self.checksum),
            "vars": self.vars.iter().map(|(n, v)| json!({"name": n, "value": v})).collect::<Vec<_>>(),
            "players": self.finals.iter().map(|(id, x, z, y)| json!({"id": id, "pos": [x, z], "foot_y": y})).collect::<Vec<_>>(),
            "props": self.props.iter().map(|p| json!({"id": p.id, "pos": [p.pos.x, p.pos.y, p.pos.z], "tilt_deg": p.tilt_deg, "moved": p.moved, "asleep": p.asleep, "held_by": p.held_by})).collect::<Vec<_>>(),
            "pickup_misses": self.pickup_misses,
            "events": self.events.iter().map(|e| json!({"tick": e.tick, "rule": e.rule, "name": e.name, "player": e.slot})).collect::<Vec<_>>(),
            "checks": self.outcomes.iter().map(|o| json!({"label": o.label, "ok": o.ok, "detail": o.detail})).collect::<Vec<_>>(),
        })
    }

    /// The result as readable text: a PASS/FAIL line, one line per check, the events, the final state.
    pub fn render(&self) -> String {
        let mut out = format!(
            "{} {} ({} ticks = {:.1} s{})\n",
            if self.passed { "PASS" } else { "FAIL" },
            self.name,
            self.ticks,
            self.ticks as f64 / 60.0,
            self.ended.as_ref().map(|e| format!(", ended: {e}")).unwrap_or_default()
        );
        for o in &self.outcomes {
            out.push_str(&format!("  [{}] {}: {}\n", if o.ok { "ok" } else { "FAIL" }, o.label, o.detail));
        }
        if !self.events.is_empty() {
            let shown: Vec<String> = self.events.iter().take(12).map(|e| format!("{}@{}", e.name, e.tick)).collect();
            out.push_str(&format!("  events: {}{}\n", shown.join(", "), if self.events.len() > 12 { ", ..." } else { "" }));
        }
        for m in &self.pickup_misses {
            out.push_str(&format!("  note: {m}\n"));
        }
        if !self.vars.is_empty() {
            out.push_str(&format!("  vars: {}\n", self.vars.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join(", ")));
        }
        for (id, x, z, y) in &self.finals {
            out.push_str(&format!("  {id} ended at ({x:.2}, {z:.2}) y={y:.2}\n"));
        }
        // Props that did something: the rest stayed where the map put them (all of them are in `to_json`).
        let moved: Vec<&PropFinal> = self.props.iter().filter(|p| p.moved > MOVED_M || p.held_by.is_some() || p.tilt_deg > 5.0).collect();
        for p in moved.iter().take(12) {
            out.push_str(&format!(
                "  prop {} at ({:.2}, {:.2}, {:.2}), tilt {:.0} deg, moved {:.2} m{}{}\n",
                p.id,
                p.pos.x,
                p.pos.y,
                p.pos.z,
                p.tilt_deg,
                p.moved,
                match (&p.held_by, p.asleep) {
                    (Some(_), _) => ", carried",
                    (None, true) => ", at rest",
                    (None, false) => ", still moving",
                },
                p.held_by.as_ref().map(|h| format!(" by {h}")).unwrap_or_default()
            ));
        }
        if moved.len() > 12 {
            out.push_str(&format!("  ... and {} more props moved (`--json` lists every prop)\n", moved.len() - 12));
        }
        out
    }
}

fn near_names(name: &str, names: impl Iterator<Item = String>) -> String {
    let all: Vec<String> = names.collect();
    match crate::prefabs::suggest(name, all.iter().map(String::as_str)).first() {
        Some(h) => format!(" — did you mean `{h}`?"),
        None => String::new(),
    }
}

fn parse_walk(s: &str, path: &str, errs: &mut Vec<String>) -> Vec<Vec2> {
    let mut out = Vec::new();
    for (i, leg) in s.split(';').map(str::trim).filter(|l| !l.is_empty()).enumerate() {
        let parts: Vec<&str> = leg.split(',').map(str::trim).collect();
        match (parts.len() == 2).then(|| (parts[0].parse::<f32>(), parts[1].parse::<f32>())) {
            Some((Ok(x), Ok(z))) => out.push(Vec2::new(x, z)),
            _ => errs.push(format!("{path}: waypoint {} `{leg}` must be `x,z` (numbers)", i + 1)),
        }
    }
    if out.is_empty() && errs.is_empty() {
        errs.push(format!("{path}: give at least one `x,z` waypoint"));
    }
    out
}

fn ticks_of(v: &Value, path: &str, errs: &mut Vec<String>) -> u32 {
    match v.as_f64() {
        Some(s) if s >= 0.0 && s.is_finite() => (s * super::clock::TICK_RATE_HZ as f64).round() as u32,
        _ => {
            errs.push(format!("{path}: must be a number of seconds (0 or more)"));
            0
        }
    }
}

/// Parses a scenario. `rules` is the scene's rule set (variable names in `expect` are checked against it); `object_ids`
/// are the scene's object ids (for `hidden`/`shown`).
pub fn parse(v: &Value, rules: &RuleSet, object_ids: &[String]) -> Result<Scenario, Vec<String>> {
    let mut errs = Vec::new();
    let Some(o) = v.as_object() else { return Err(vec!["scenario: must be an object".to_string()]) };
    let name = o.get("name").and_then(Value::as_str).unwrap_or("scenario").to_string();
    let p = format!("scenario `{name}`");
    check_keys(&mut errs, &p, o, SCENARIO_KEYS);
    let mut players = Vec::new();
    match o.get("players").and_then(Value::as_array) {
        Some(arr) if !arr.is_empty() => {
            for (i, pv) in arr.iter().enumerate() {
                let pp = format!("{p}.players[{i}]");
                let Some(po) = pv.as_object() else {
                    errs.push(format!("{pp}: must be an object like {{\"id\": \"p1\", \"character\": \"human\"}}"));
                    continue;
                };
                check_keys(&mut errs, &pp, po, PLAYER_KEYS);
                let id = po.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                if id.is_empty() {
                    errs.push(format!("{pp}.id: missing"));
                } else if players.iter().any(|q: &PlayerSpec| q.id == id) {
                    errs.push(format!("{pp}.id: duplicate player id `{id}`"));
                }
                let name = po.get("character").and_then(Value::as_str).unwrap_or("human");
                let character = Character::parse(name).unwrap_or_else(|| {
                    errs.push(format!("{pp}.character: unknown character '{name}'"));
                    Character::Human
                });
                players.push(PlayerSpec { id, character, spawn: po.get("spawn").and_then(Value::as_str).map(str::to_string) });
            }
        }
        _ => errs.push(format!("{p}.players: missing (give at least one: [{{\"id\": \"p1\", \"character\": \"human\"}}])")),
    }
    let find = |id: &str| players.iter().position(|q| q.id == id);
    let mut script = Vec::new();
    for (i, sv) in o.get("script").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let sp = format!("{p}.script[{i}]");
        let Some(so) = sv.as_object() else {
            errs.push(format!("{sp}: must be an object like {{\"player\": \"p1\", \"walk\": \"0,0; 3,0\"}}"));
            continue;
        };
        check_keys(&mut errs, &sp, so, STEP_KEYS);
        let who = so.get("player").and_then(Value::as_str).unwrap_or("");
        let Some(pi) = find(who) else {
            errs.push(format!(
                "{sp}.player: no player `{who}`{} (players: {})",
                near_names(who, players.iter().map(|q| q.id.clone())),
                players.iter().map(|q| q.id.as_str()).collect::<Vec<_>>().join(", ")
            ));
            continue;
        };
        let until = so.get("until_event").and_then(Value::as_str).map(str::to_string);
        const ACTIONS: [&str; 6] = ["walk", "wait", "hold", "approach", "look_at", "interact"];
        let actions: Vec<&str> = ACTIONS.into_iter().filter(|k| so.contains_key(*k)).collect();
        if actions.len() != 1 {
            errs.push(format!(
                "{sp}: give exactly one of {} (got {})",
                ACTIONS.join(", "),
                if actions.is_empty() { "none".to_string() } else { actions.join(" + ") }
            ));
            continue;
        }
        if !matches!(actions[0], "approach" | "interact") {
            for k in ["within", "timeout"] {
                if so.contains_key(k) {
                    errs.push(format!("{sp}.{k}: only `approach` and `interact` take `{k}`"));
                }
            }
        }
        match actions[0] {
            "approach" | "look_at" | "interact" => {
                let kind = actions[0];
                let id = so[kind].as_str().unwrap_or("");
                if id.is_empty() {
                    errs.push(format!("{sp}.{kind}: must be an object id like \"parcel_1\""));
                    continue;
                }
                if !object_ids.iter().any(|o| o == id) {
                    errs.push(format!("{sp}.{kind}: no object `{id}`{}", near_names(id, object_ids.iter().cloned())));
                    continue;
                }
                let within = match so.get("within") {
                    None => None,
                    Some(w) => match w.as_f64().filter(|w| *w > 0.0 && w.is_finite()) {
                        Some(w) => Some(w as f32),
                        None => {
                            errs.push(format!("{sp}.within: metres, a number greater than 0"));
                            None
                        }
                    },
                };
                let timeout = match so.get("timeout") {
                    None => super::approach::DEFAULT_TIMEOUT_SECS,
                    Some(t) => match t.as_f64().filter(|t| *t > 0.0 && t.is_finite()) {
                        Some(t) => t as f32,
                        None => {
                            errs.push(format!("{sp}.timeout: seconds, a number greater than 0"));
                            super::approach::DEFAULT_TIMEOUT_SECS
                        }
                    },
                };
                let object = id.to_string();
                let action = match kind {
                    "approach" => Action::Approach { object, within, timeout },
                    "look_at" => Action::LookAt { object },
                    _ => Action::Interact { object, within, timeout },
                };
                script.push((pi, action, until));
            }
            "walk" => {
                let wps = match so["walk"].as_str() {
                    Some(s) => parse_walk(s, &format!("{sp}.walk"), &mut errs),
                    None => {
                        errs.push(format!("{sp}.walk: must be a string like \"0,0; 3,0\""));
                        Vec::new()
                    }
                };
                script.push((pi, Action::Walk(wps), until));
            }
            "wait" => script.push((pi, Action::Wait(ticks_of(&so["wait"], &format!("{sp}.wait"), &mut errs)), until)),
            _ => {
                let hp = format!("{sp}.hold");
                let Some(h) = so["hold"].as_object() else {
                    errs.push(format!("{hp}: must be an object like {{\"forward\": 1, \"seconds\": 1.0}}"));
                    continue;
                };
                check_keys(&mut errs, &hp, h, HOLD_KEYS);
                let int = |k: &str| h.get(k).and_then(Value::as_i64).unwrap_or(0).clamp(-1, 1) as i8;
                let flag = |k: &str| h.get(k).and_then(Value::as_bool).unwrap_or(false);
                let yaw = h.get("yaw_deg").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let ticks = h.get("seconds").map_or(1, |s| ticks_of(s, &format!("{hp}.seconds"), &mut errs));
                let pitch = h.get("pitch_deg").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                let look_at = match h.get("look_at") {
                    None => None,
                    Some(v) => {
                        let p = v
                            .as_array()
                            .filter(|a| a.len() == 3)
                            .and_then(|a| Some(Vec3::new(a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32)));
                        if p.is_none() {
                            errs.push(format!("{hp}.look_at: must be [x, y, z], a world point to aim at (a prop's origin plus about half its height)"));
                        }
                        p
                    }
                };
                let input = PlayerInput {
                    seq: 0,
                    analog: false,
                    forward: int("forward"),
                    strafe: int("strafe"),
                    jump: flag("jump"),
                    sprint: flag("sprint"),
                    crouch: flag("crouch"),
                    yaw: yaw.to_radians(),
                    pitch: pitch.to_radians(),
                    interact: flag("interact"),
                    attack: flag("attack"),
                    reload: flag("reload"),
                    switch_weapon: flag("switch"),
                    aim: flag("aim"),
                    drop: flag("drop"),
                    select: int("select").clamp(0, 7) as u8,
                };
                script.push((
                    pi,
                    Action::Hold { input, ticks, keep_yaw: !h.contains_key("yaw_deg"), keep_pitch: !h.contains_key("pitch_deg"), look_at },
                    until,
                ));
            }
        }
    }
    // A limit written as text (`"60"`) is an error, never the default: a scenario that silently ran for 30 s proves less than its author meant.
    let seconds = |key: &str, default: f64, floor_zero: bool, errs: &mut Vec<String>| -> f64 {
        let Some(v) = o.get(key) else { return default };
        match v.as_f64().filter(|n| n.is_finite() && if floor_zero { *n >= 0.0 } else { *n > 0.0 }) {
            Some(n) => n,
            None => {
                let rule = if floor_zero { "0 or more" } else { "greater than 0" };
                errs.push(format!("{p}.{key}: expected a number of seconds {rule}, got {}", crate::strict::describe_value(v)));
                default
            }
        }
    };
    let max_secs = seconds("max_seconds", 30.0, false, &mut errs);
    let settle = seconds("settle_seconds", 0.5, true, &mut errs);
    let mut expect = Vec::new();
    match o.get("expect") {
        None => errs.push(format!(
            "{p}.expect: needs at least one expectation: a scenario with none proves only that nobody got stuck (like {{\"event\": \"coin\", \"count\": 1}}, {{\"var\": \"score\", \"gte\": 1}}, {{\"ended\": \"victory\"}})"
        )),
        Some(Value::Array(list)) if list.is_empty() => {
            errs.push(format!("{p}.expect: is empty, so the scenario proves only that nobody got stuck; add an expectation like {{\"ended\": \"victory\"}}"))
        }
        Some(Value::Array(_)) => {}
        Some(other) => errs.push(format!("{p}.expect: expected a list of expectations, got {}", crate::strict::describe_value(other))),
    }
    for (i, ev) in o.get("expect").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let ep = format!("{p}.expect[{i}]");
        let Some(eo) = ev.as_object() else {
            errs.push(format!("{ep}: must be an object like {{\"event\": \"coin\", \"count\": 3}}"));
            continue;
        };
        check_keys(&mut errs, &ep, eo, EXPECT_KEYS);
        if let Some(e) = parse_expect(eo, &ep, rules, object_ids, &players, &mut errs) {
            expect.push(e);
        }
    }
    if errs.is_empty() {
        Ok(Scenario {
            name,
            spawn_group: o.get("spawn_group").and_then(Value::as_str).unwrap_or("").to_string(),
            players,
            script,
            max_ticks: (max_secs * super::clock::TICK_RATE_HZ as f64).round() as u64,
            settle_ticks: (settle * super::clock::TICK_RATE_HZ as f64).round() as u64,
            expect,
        })
    } else {
        Err(errs)
    }
}

fn parse_expect(eo: &Map<String, Value>, ep: &str, rules: &RuleSet, object_ids: &[String], players: &[PlayerSpec], errs: &mut Vec<String>) -> Option<Expect> {
    let main: Vec<&str> = ["event", "no_event", "var", "ended", "not_ended", "hidden", "shown", "collision_disabled", "collision_enabled", "player", "prop"]
        .into_iter()
        .filter(|k| eo.contains_key(*k))
        .collect();
    if main.len() != 1 {
        errs.push(format!(
            "{ep}: give exactly one of event, no_event, var, ended, not_ended, hidden, shown, collision_disabled, collision_enabled, player, prop (got {})",
            if main.is_empty() { "none".to_string() } else { main.join(" + ") }
        ));
        return None;
    }
    let key = main[0];
    let text = eo[key].as_str().unwrap_or("").to_string();
    let count = |k: &str| eo.get(k).and_then(Value::as_u64).map(|n| n as usize);
    match key {
        "event" => {
            let (min, max) = match count("count") {
                Some(n) => (n, Some(n)),
                None => (count("min").unwrap_or(1), count("max")),
            };
            Some(Expect::Event { name: text, min, max })
        }
        "no_event" => Some(Expect::NoEvent(text)),
        "var" => {
            if !rules.var_names.contains(&text) {
                errs.push(format!(
                    "{ep}.var: no variable `{text}`{} (declare it in the scene's `vars`; known: {})",
                    near_names(&text, rules.var_names.iter().cloned()),
                    rules.var_names.join(", ")
                ));
                return None;
            }
            let cmps = [("eq", Cmp::Eq), ("ne", Cmp::Ne), ("gt", Cmp::Gt), ("gte", Cmp::Ge), ("lt", Cmp::Lt), ("lte", Cmp::Le)];
            let mut given = Vec::new();
            for (key, cmp) in cmps {
                let Some(value) = eo.get(key) else { continue };
                match value {
                    Value::Number(n) => {
                        if let Some(value) = n.as_f64() {
                            given.push((cmp, value));
                        }
                    }
                    Value::Bool(value) if matches!(cmp, Cmp::Eq | Cmp::Ne) => given.push((cmp, if *value { 1.0 } else { 0.0 })),
                    Value::Bool(_) => errs.push(format!("{ep}.{key}: boolean values are only valid with eq or ne")),
                    _ => errs.push(format!("{ep}.{key}: expected a number, or true/false with eq or ne")),
                }
            }
            match given.as_slice() {
                [(cmp, value)] => Some(Expect::Var { name: text, cmp: *cmp, value: *value }),
                _ => {
                    if given.len() != 1 && !errs.iter().any(|e| e.starts_with(ep)) {
                        errs.push(format!("{ep}: a `var` check needs exactly one of eq, ne, gt, gte, lt, lte; eq/ne also accept true/false"));
                    }
                    None
                }
            }
        }
        "ended" => Some(Expect::Ended(Some(text))),
        "not_ended" => Some(Expect::Ended(None)),
        "hidden" | "shown" => {
            if !object_ids.contains(&text) {
                errs.push(format!("{ep}.{key}: no object `{text}`{}", near_names(&text, object_ids.iter().cloned())));
                return None;
            }
            Some(Expect::Hidden(text, key == "hidden"))
        }
        "collision_disabled" | "collision_enabled" => {
            if !object_ids.contains(&text) {
                errs.push(format!("{ep}.{key}: no object `{text}`{}", near_names(&text, object_ids.iter().cloned())));
                return None;
            }
            Some(Expect::CollisionDisabled(text, key == "collision_disabled"))
        }
        "prop" => {
            if !rules.prop_ids.contains(&text) {
                let known = if rules.prop_ids.is_empty() { "none in this scene".to_string() } else { rules.prop_ids.join(", ") };
                errs.push(format!("{ep}.prop: no loose prop `{text}`{} (loose props: {known})", near_names(&text, rules.prop_ids.iter().cloned())));
                return None;
            }
            let kinds: Vec<&str> = PROP_CHECK_KEYS.iter().copied().filter(|k| eo.contains_key(*k)).collect();
            let [kind] = kinds.as_slice() else {
                errs.push(format!(
                    "{ep}: a `prop` check needs exactly one of {} (got {})",
                    PROP_CHECK_KEYS.join(", "),
                    if kinds.is_empty() { "none".to_string() } else { kinds.join(" + ") }
                ));
                return None;
            };
            let check = match *kind {
                k @ ("in_zone" | "not_in_zone") => {
                    let zone = eo[k].as_str().unwrap_or("").to_string();
                    if !rules.zone_ids.contains(&zone) {
                        errs.push(format!(
                            "{ep}.{k}: no zone `{zone}`{} (zones: {})",
                            near_names(&zone, rules.zone_ids.iter().cloned()),
                            rules.zone_ids.join(", ")
                        ));
                        return None;
                    }
                    PropCheck::InZone(zone, k == "in_zone")
                }
                k @ ("below_y" | "y_lt" | "y_gt" | "tilt_gt" | "tilt_lt") => {
                    let Some(v) = eo[k].as_f64() else {
                        errs.push(format!("{ep}.{k}: must be a number ({})", if k.starts_with("tilt") { "degrees" } else { "metres" }));
                        return None;
                    };
                    match k {
                        "y_gt" => PropCheck::Y(Cmp::Gt, v as f32),
                        "tilt_gt" => PropCheck::Tilt(Cmp::Gt, v as f32),
                        "tilt_lt" => PropCheck::Tilt(Cmp::Lt, v as f32),
                        _ => PropCheck::Y(Cmp::Lt, v as f32),
                    }
                }
                "moved" => {
                    let Some(b) = eo["moved"].as_bool() else {
                        errs.push(format!("{ep}.moved: must be true or false"));
                        return None;
                    };
                    PropCheck::Moved(b)
                }
                "near" => {
                    let at = eo
                        .get("near")
                        .and_then(Value::as_array)
                        .filter(|a| a.len() == 2)
                        .and_then(|a| Some(Vec2::new(a[0].as_f64()? as f32, a[1].as_f64()? as f32)));
                    let Some(at) = at else {
                        errs.push(format!("{ep}.near: must be [x, z]"));
                        return None;
                    };
                    PropCheck::Near {
                        at,
                        tol: eo.get("tol").and_then(Value::as_f64).unwrap_or(0.5) as f32,
                        y: eo.get("y").and_then(Value::as_f64).map(|y| y as f32),
                    }
                }
                _ => {
                    let who = eo["held_by"].as_str().unwrap_or("").to_string();
                    if who == "none" {
                        PropCheck::HeldBy(None)
                    } else if players.iter().any(|q| q.id == who) {
                        PropCheck::HeldBy(Some(who))
                    } else {
                        errs.push(format!(
                            "{ep}.held_by: no player `{who}`{} (a player id, or \"none\")",
                            near_names(&who, players.iter().map(|q| q.id.clone()))
                        ));
                        return None;
                    }
                }
            };
            Some(Expect::Prop { id: text, check })
        }
        _ => {
            if !players.iter().any(|q| q.id == text) {
                errs.push(format!("{ep}.player: no player `{text}`{}", near_names(&text, players.iter().map(|q| q.id.clone()))));
                return None;
            }
            let at =
                eo.get("near").and_then(Value::as_array).filter(|a| a.len() == 2).and_then(|a| Some(Vec2::new(a[0].as_f64()? as f32, a[1].as_f64()? as f32)));
            let Some(at) = at else {
                errs.push(format!("{ep}.near: a player check needs \"near\": [x, z]"));
                return None;
            };
            Some(Expect::PlayerNear {
                player: text,
                at,
                tol: eo.get("tol").and_then(Value::as_f64).unwrap_or(0.5) as f32,
                y: eo.get("y").and_then(Value::as_f64).map(|y| y as f32),
            })
        }
    }
}

/// What to record while running (see [`super::trace`]).
#[derive(Debug, Clone, Copy)]
pub struct RecordOptions {
    /// Hash of the map file text.
    pub map_hash: u32,
    /// Checkpoint interval in ticks.
    pub checkpoint_every: u32,
    /// Dump interval in ticks (0 = end only).
    pub dump_every: u32,
}

struct Cursor {
    /// How many game events existed when the current step began (for `until_event`).
    mark: usize,
    step: usize,
    leg: usize,
    ticks_in_step: u32,
    best: f32,
    since_progress: u32,
    /// A failed step: what it was (the outcome's label) and why.
    stuck: Option<(&'static str, String)>,
    /// The step the fields below belong to (they restart when it changes).
    seen_step: usize,
    /// The running `approach`.
    approach: Option<super::approach::Approach>,
    /// Which part of an `interact` is running: 0 approach, 1 aim, 2 press, 3 settle.
    phase: u8,
}

/// Runs `scenario` on `scene`, returning the checked result (and a [`Trace`] when `record` is given).
pub fn run(scenario: &Scenario, scene: &crate::schema::Scene, spawns: &[Spawn], record: Option<RecordOptions>) -> Result<ScenarioResult, String> {
    let mut group_spawns: Vec<Spawn> = spawns.to_vec();
    if !scenario.spawn_group.is_empty() {
        group_spawns.retain(|s| s.group == scenario.spawn_group);
    }
    let mut sim = MatchSim::try_new(scene, group_spawns)?;
    if let Some(r) = record {
        sim.start_recording(Header::new(r.map_hash, 0, &scenario.spawn_group, r.checkpoint_every, r.dump_every))?;
    }
    let mut slots = Vec::new();
    for p in &scenario.players {
        let slot = match &p.spawn {
            Some(id) => {
                let s = spawns
                    .iter()
                    .find(|s| &s.id == id)
                    .ok_or_else(|| format!("scenario `{}`: player `{}` spawns at `{id}`, which is not a spawn point of this scene", scenario.name, p.id))?;
                sim.add_player_with(PlayerState::spawn(s.position[0], s.position[2], s.position[1], s.yaw_deg, p.character))
            }
            None => sim.add_player(p.character),
        };
        slots.push(slot.ok_or_else(|| format!("scenario `{}`: the match is full (at most {} players)", scenario.name, super::match_sim::MAX_PLAYERS))?);
    }
    // Where a top-level object is right now: loose props from the physics world, everything else from the scene's bounds.
    let statics: std::collections::HashMap<String, (Vec3, Vec3)> =
        crate::collide::interactables_of(&scene.objects).into_iter().map(|i| (i.id, (i.min, i.max))).collect();
    let prop_of: std::collections::HashMap<String, usize> =
        (0..sim.props().props().len()).map(|k| (scene.objects[sim.props().props()[k].object_index].id.clone(), k)).collect();
    let locate = |sim: &MatchSim, id: &str| -> Option<(Target, Option<usize>)> {
        if let Some(&k) = prop_of.get(id) {
            return Some((Target::from_prop(sim.prop_view(k).origin, sim.props().props()[k].shape.extents), Some(k)));
        }
        statics.get(id).map(|(lo, hi)| (Target::from_bounds(*lo, *hi), None))
    };
    for (_, action, _) in &scenario.script {
        if let Action::Approach { object, .. } | Action::LookAt { object } | Action::Interact { object, .. } = action {
            if locate(&sim, object).is_none() {
                return Err(format!(
                    "scenario `{}`: `{object}` is not a top-level object of the scene (approach / look_at / interact take a top-level id; a nested child has none of its own)",
                    scenario.name
                ));
            }
        }
    }
    let tick_secs = 1.0 / super::clock::TICK_RATE_HZ as f32;
    let mut cursors: Vec<Cursor> = scenario
        .players
        .iter()
        .map(|_| Cursor {
            mark: 0,
            step: 0,
            leg: 0,
            ticks_in_step: 0,
            best: f32::INFINITY,
            since_progress: 0,
            stuck: None,
            seen_step: usize::MAX,
            approach: None,
            phase: 0,
        })
        .collect();
    let steps_of = |pi: usize| -> Vec<(&Action, &Option<String>)> { scenario.script.iter().filter(|(p, _, _)| *p == pi).map(|(_, a, u)| (a, u)).collect() };
    let mut seq = vec![0u32; slots.len()];
    let mut settle_left = scenario.settle_ticks;
    while sim.tick() < scenario.max_ticks && sim.rules().ended().is_none() {
        let mut all_done = true;
        for pi in 0..slots.len() {
            let steps = steps_of(pi);
            let cur = &mut cursors[pi];
            let state = sim.player(slots[pi]).map(|p| p.state);
            let mut input = PlayerInput::default();
            if let (Some(state), true) = (state, cur.stuck.is_none()) {
                input.yaw = state.yaw;
                // Finish steps that are already complete, then produce this tick's input from the current one.
                while let Some((action, until)) = steps.get(cur.step).copied() {
                    if cur.seen_step != cur.step {
                        (cur.seen_step, cur.approach, cur.phase) = (cur.step, None, 0);
                    }
                    // `until_event`: the step is over once that event has happened since it began.
                    if let Some(name) = until {
                        if sim.rules().history().iter().skip(cur.mark).any(|h| &h.name == name) {
                            (cur.step, cur.leg, cur.ticks_in_step, cur.best, cur.since_progress, cur.mark) =
                                (cur.step + 1, 0, 0, f32::INFINITY, 0, sim.rules().history().len());
                            continue;
                        }
                    }
                    match action {
                        Action::Wait(n) => {
                            if cur.ticks_in_step >= *n {
                                (cur.step, cur.ticks_in_step) = (cur.step + 1, 0);
                                continue;
                            }
                            cur.ticks_in_step += 1;
                        }
                        Action::Hold { input: held, ticks, keep_yaw, keep_pitch, look_at } => {
                            if cur.ticks_in_step >= *ticks {
                                (cur.step, cur.ticks_in_step) = (cur.step + 1, 0);
                                continue;
                            }
                            cur.ticks_in_step += 1;
                            let (yaw, pitch) = (state.yaw, state.pitch);
                            input = *held;
                            if *keep_yaw {
                                input.yaw = yaw;
                            }
                            input.pitch = if *keep_pitch { pitch } else { input.pitch };
                            if let Some(at) = look_at {
                                // Aim from the standing eye: what a player looking at that point would send this tick.
                                let eye = Vec3::new(state.pos.x, state.foot_y + state.character.body().stand_eye, state.pos.y);
                                (input.yaw, input.pitch) = super::approach::aim(eye, *at);
                            }
                        }
                        Action::Approach { object, within, timeout } => {
                            let Some((target, _)) = locate(&sim, object) else { break };
                            let goal = within.unwrap_or_else(|| default_within(state.character.body().pickup_reach));
                            let gap = target.gap(state.pos);
                            match cur.approach.get_or_insert_with(|| Approach::new(goal, *timeout)).step(tick_secs, gap) {
                                Progress::Arrived => {
                                    cur.step += 1;
                                    continue;
                                }
                                Progress::Moving => {
                                    input.forward = 1;
                                    input.yaw = target.heading(state.pos);
                                }
                                Progress::Failed(why) => {
                                    let secs = cur.approach.as_ref().map_or(0.0, Approach::elapsed);
                                    cur.stuck = Some(("approach", failure_message("approach", object, why, gap, goal, state.pos, secs)));
                                    break;
                                }
                            }
                        }
                        Action::LookAt { object } => {
                            let Some((target, _)) = locate(&sim, object) else { break };
                            let eye = Vec3::new(state.pos.x, state.foot_y + state.character.body().stand_eye, state.pos.y);
                            (input.yaw, input.pitch) = approach::aim(eye, target.aim_point());
                            cur.step += 1;
                        }
                        Action::Interact { object, within, timeout } => {
                            let Some((target, prop)) = locate(&sim, object) else { break };
                            let slot = slots[pi];
                            let eye = Vec3::new(state.pos.x, state.foot_y + state.character.body().stand_eye, state.pos.y);
                            let mine = prop.is_some() && sim.props().held_by(slot) == prop;
                            if mine && cur.phase < 2 {
                                cur.step += 1; // already carrying it
                                continue;
                            }
                            if let (Some(held), true) = (sim.props().held_by(slot), cur.phase == 0) {
                                let held_id = scene.objects[sim.props().props()[held].object_index].id.clone();
                                cur.stuck = Some((
                                    "interact",
                                    format!(
                                        "interact `{object}`: {} is already carrying `{held_id}`, and interact would drop that instead",
                                        scenario.players[pi].id
                                    ),
                                ));
                                break;
                            }
                            if cur.phase == 0 {
                                let goal = within.unwrap_or_else(|| approach::pickup_within(state.character.body().pickup_reach, eye, &target));
                                let gap = target.gap(state.pos);
                                match cur.approach.get_or_insert_with(|| Approach::new(goal, *timeout)).step(tick_secs, gap) {
                                    Progress::Arrived => (cur.phase, cur.ticks_in_step) = (1, 0),
                                    Progress::Moving => {
                                        input.forward = 1;
                                        input.yaw = target.heading(state.pos);
                                        break;
                                    }
                                    Progress::Failed(why) => {
                                        let secs = cur.approach.as_ref().map_or(0.0, Approach::elapsed);
                                        cur.stuck = Some(("interact", failure_message("interact", object, why, gap, goal, state.pos, secs)));
                                        break;
                                    }
                                }
                            }
                            // Face it for two ticks (the view settles), press for one, then give the pick-up two ticks to show.
                            (input.yaw, input.pitch) = approach::aim(eye, target.aim_point());
                            match cur.phase {
                                1 => {
                                    cur.ticks_in_step += 1;
                                    if cur.ticks_in_step >= 2 {
                                        (cur.phase, cur.ticks_in_step) = (2, 0);
                                    }
                                }
                                2 => {
                                    input.interact = true;
                                    (cur.phase, cur.ticks_in_step) = (3, 0);
                                }
                                _ => {
                                    cur.ticks_in_step += 1;
                                    if cur.ticks_in_step >= 2 {
                                        if prop.is_some() && !mine && sim.props().held_by(slot) != prop {
                                            let why = sim
                                                .pickup_misses()
                                                .iter()
                                                .rev()
                                                .find(|m| m.1 == slot)
                                                .map_or("the press found nothing to take", |m| m.2.as_str());
                                            cur.stuck = Some((
                                                "interact",
                                                format!(
                                                    "interact `{object}`: pressed interact from ({:.2}, {:.2}) but did not pick it up: {why}",
                                                    state.pos.x, state.pos.y
                                                ),
                                            ));
                                            break;
                                        }
                                        (cur.step, cur.ticks_in_step) = (cur.step + 1, 0);
                                    }
                                }
                            }
                        }
                        Action::Walk(wps) => {
                            let Some(target) = wps.get(cur.leg) else {
                                (cur.step, cur.leg, cur.best, cur.since_progress) = (cur.step + 1, 0, f32::INFINITY, 0);
                                continue;
                            };
                            // The short way round: on a looping world a waypoint past the seam is a few metres ahead, not a lap behind.
                            let to = sim.movement().0.expanse.delta(state.pos, *target);
                            let dist = to.length();
                            if dist <= REACHED {
                                (cur.leg, cur.best, cur.since_progress) = (cur.leg + 1, f32::INFINITY, 0);
                                continue;
                            }
                            if dist < cur.best - 0.2 || dist > cur.best + 2.0 {
                                // Progress, or the player was moved (a teleport): start measuring again.
                                (cur.best, cur.since_progress) = (dist, 0);
                            } else {
                                cur.since_progress += 1;
                                if cur.since_progress >= STUCK_TICKS {
                                    cur.stuck = Some((
                                        "walk",
                                        format!(
                                            "{} got stuck on leg {} toward ({:.2}, {:.2}); stopped at ({:.2}, {:.2}) y={:.2}",
                                            scenario.players[pi].id,
                                            cur.leg + 1,
                                            target.x,
                                            target.y,
                                            state.pos.x,
                                            state.pos.y,
                                            state.foot_y
                                        ),
                                    ));
                                    break;
                                }
                            }
                            input.forward = 1;
                            input.yaw = libm::atan2f(to.x, -to.y);
                        }
                    }
                    break;
                }
                if steps.get(cur.step).is_some() {
                    all_done = false;
                }
            }
            seq[pi] += 1;
            input.seq = seq[pi];
            sim.push_input(slots[pi], input);
        }
        sim.tick_once();
        if all_done {
            if settle_left == 0 {
                break;
            }
            settle_left -= 1;
        }
    }
    // Evaluate.
    let mut outcomes: Vec<Outcome> = Vec::new();
    for (pi, cur) in cursors.iter().enumerate() {
        if let Some((what, msg)) = &cur.stuck {
            outcomes.push(Outcome { label: format!("{what} {}", scenario.players[pi].id), ok: false, detail: msg.clone() });
        }
    }
    let history = sim.rules().history().to_vec();
    for e in &scenario.expect {
        outcomes.push(check(e, scenario, &sim, &slots, &history));
    }
    let passed = outcomes.iter().all(|o| o.ok);
    let checksum = sim.checksum();
    let ended = sim.rules().ended().map(str::to_string);
    let vars = sim.rules().vars().into_iter().map(|(n, v)| (n.to_string(), v)).collect();
    let finals = scenario
        .players
        .iter()
        .zip(&slots)
        .filter_map(|(p, s)| sim.player(*s).map(|pl| (p.id.clone(), pl.state.pos.x, pl.state.pos.y, pl.state.foot_y)))
        .collect();
    let player_id = |slot: usize| slots.iter().position(|s| *s == slot).map(|pi| scenario.players[pi].id.clone());
    let props: Vec<PropFinal> = (0..sim.props().props().len())
        .map(|k| {
            let v = sim.prop_view(k);
            PropFinal {
                id: scene.objects[sim.props().props()[k].object_index].id.clone(),
                pos: v.origin,
                tilt_deg: v.tilt_deg,
                moved: v.moved,
                asleep: sim.props().is_asleep(k),
                held_by: v.held_by.and_then(player_id),
            }
        })
        .collect();
    let pickup_misses = sim
        .pickup_misses()
        .iter()
        .map(|(tick, slot, why)| format!("tick {tick}: {} could not pick anything up: {why}", player_id(*slot).unwrap_or_else(|| format!("slot {slot}"))))
        .collect();
    let ticks = sim.tick();
    let trace = sim.take_trace();
    Ok(ScenarioResult { name: scenario.name.clone(), passed, ticks, outcomes, events: history, vars, ended, finals, props, pickup_misses, checksum, trace })
}

fn check(e: &Expect, scenario: &Scenario, sim: &MatchSim, slots: &[usize], history: &[GameEvent]) -> Outcome {
    let done = |label: String, ok: bool, detail: String| Outcome { label, ok, detail };
    match e {
        Expect::Event { name, min, max } => {
            let n = history.iter().filter(|h| &h.name == name).count();
            let ok = n >= *min && max.is_none_or(|m| n <= m);
            let want = match max {
                Some(m) if m == min => format!("exactly {min}"),
                Some(m) => format!("{min}..={m}"),
                None => format!("at least {min}"),
            };
            done(
                format!("event `{name}`"),
                ok,
                format!("happened {n} time(s), wanted {want}{}", if ok { String::new() } else { format!(" (events seen: {})", seen(history)) }),
            )
        }
        Expect::NoEvent(name) => {
            let n = history.iter().filter(|h| &h.name == name).count();
            done(format!("no event `{name}`"), n == 0, format!("happened {n} time(s)"))
        }
        Expect::Var { name, cmp, value } => {
            let got = sim.rules().var(name).unwrap_or(f64::NAN);
            done(format!("var {name} {} {value}", cmp.word()), cmp.holds(got, *value), format!("{name} = {got}"))
        }
        Expect::Ended(want) => {
            let got = sim.rules().ended();
            let ok = got == want.as_deref();
            done(
                match want {
                    Some(w) => format!("ended `{w}`"),
                    None => "still running".to_string(),
                },
                ok,
                match got {
                    Some(g) => format!("the match ended: {g}"),
                    None => format!("the match did not end (after {} ticks)", sim.tick()),
                },
            )
        }
        Expect::Hidden(id, want) => {
            let hidden = sim.rules().hidden().any(|h| h == id);
            done(format!("{id} {}", if *want { "hidden" } else { "shown" }), hidden == *want, format!("{id} is {}", if hidden { "hidden" } else { "shown" }))
        }
        Expect::CollisionDisabled(id, want) => {
            let disabled = sim.rules().collision_disabled().any(|object| object == id);
            done(
                format!("{id} collision {}", if *want { "disabled" } else { "enabled" }),
                disabled == *want,
                format!("{id} collision is {}", if disabled { "disabled" } else { "enabled" }),
            )
        }
        Expect::PlayerNear { player, at, tol, y } => {
            let Some(pi) = scenario.players.iter().position(|p| &p.id == player) else { return done(format!("{player} near"), false, "unknown player".into()) };
            let Some(pl) = sim.player(slots[pi]) else { return done(format!("{player} near"), false, "the player left".into()) };
            let d = (pl.state.pos - *at).length();
            let y_ok = y.is_none_or(|y| (pl.state.foot_y - y).abs() <= 0.2);
            done(
                format!("{player} near ({:.1}, {:.1})", at.x, at.y),
                d <= *tol && y_ok,
                format!("at ({:.2}, {:.2}) y={:.2}, {:.2} m away (tol {tol})", pl.state.pos.x, pl.state.pos.y, pl.state.foot_y, d),
            )
        }
        Expect::Prop { id, check } => {
            let Some(prop) = sim.prop_named(id) else { return done(format!("prop {id}"), false, "no such loose prop".into()) };
            let v = sim.prop_view(prop);
            let holder = v.held_by.and_then(|s| slots.iter().position(|x| *x == s)).map(|pi| scenario.players[pi].id.clone());
            let state = format!(
                "{id} at ({:.2}, {:.2}, {:.2}), tilt {:.0} deg, moved {:.2} m{}",
                v.origin.x,
                v.origin.y,
                v.origin.z,
                v.tilt_deg,
                v.moved,
                holder.as_ref().map(|h| format!(", held by {h}")).unwrap_or_default()
            );
            match check {
                PropCheck::InZone(zone, want) => {
                    let set = sim.rules().set();
                    let inside = set.zone_ids.iter().position(|z| z == zone).and_then(|i| set.zone_volumes.get(i)).is_some_and(|vol| prop_inside(vol, &v));
                    done(format!("{id} {} zone {zone}", if *want { "in" } else { "not in" }), inside == *want, state)
                }
                PropCheck::Y(cmp, y) => done(format!("{id} y {} {y}", cmp.word()), cmp.holds(v.origin.y as f64, *y as f64), state),
                PropCheck::Tilt(cmp, t) => done(format!("{id} tilt {} {t} deg", cmp.word()), cmp.holds(v.tilt_deg as f64, *t as f64), state),
                PropCheck::Moved(want) => done(format!("{id} {}", if *want { "moved" } else { "unmoved" }), (v.moved > MOVED_M) == *want, state),
                PropCheck::Near { at, tol, y } => {
                    let d = (Vec2::new(v.origin.x, v.origin.z) - *at).length();
                    let y_ok = y.is_none_or(|y| (v.origin.y - y).abs() <= 0.2);
                    done(format!("{id} near ({:.1}, {:.1})", at.x, at.y), d <= *tol && y_ok, format!("{state}, {d:.2} m away (tol {tol})"))
                }
                PropCheck::HeldBy(want) => done(format!("{id} held by {}", want.as_deref().unwrap_or("nobody")), holder == *want, state),
            }
        }
    }
}

fn seen(history: &[GameEvent]) -> String {
    if history.is_empty() {
        return "none".to_string();
    }
    let mut names: Vec<String> = Vec::new();
    for h in history {
        if !names.contains(&h.name) {
            names.push(h.name.clone());
        }
    }
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rules() -> RuleSet {
        let mut rules = RuleSet::default();
        rules.var_names.push("flag".into());
        rules.var_init.push(0.0);
        rules
    }

    /// A scenario that cannot fail for the reason its author meant is an error: no expectations, a limit written as text, or an `expect` that is not a list.
    #[test]
    fn a_scenario_that_asserts_nothing_or_reads_a_limit_as_text_is_refused() {
        let base = |extra: Value| {
            let mut s = json!({"name": "s", "players": [{"id": "p"}], "script": [{"player": "p", "wait": 0}]});
            s.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            s
        };
        let cases = [
            ("no expect", base(json!({})), "expect: needs at least one expectation"),
            ("an empty expect", base(json!({"expect": []})), "expect: is empty"),
            ("an expect that is one object", base(json!({"expect": {"ended": "victory"}})), "expect: expected a list of expectations, got an object"),
            ("a misspelled expect", base(json!({"expects": [{"ended": "victory"}]})), "expects: unknown field"),
            (
                "a duration as text",
                base(json!({"expect": [{"ended": "victory"}], "max_seconds": "60"})),
                "max_seconds: expected a number of seconds greater than 0, got string \"60\"",
            ),
            ("a zero duration", base(json!({"expect": [{"ended": "victory"}], "max_seconds": 0})), "max_seconds: expected a number of seconds greater than 0"),
            (
                "a settle time as text",
                base(json!({"expect": [{"ended": "victory"}], "settle_seconds": "1"})),
                "settle_seconds: expected a number of seconds 0 or more, got string \"1\"",
            ),
        ];
        for (what, scenario, want) in cases {
            let err = parse(&scenario, &rules(), &[]).err().unwrap_or_else(|| panic!("{what}: parsed")).join(" | ");
            assert!(err.contains(want), "{what}: wanted `{want}` in `{err}`");
        }
    }

    #[test]
    fn boolean_variable_expectations_normalize_for_equality() {
        let scenario = parse(
            &json!({
                "name": "bools",
                "players": [{"id": "p"}],
                "script": [{"player": "p", "wait": 0}],
                "expect": [{"var": "flag", "eq": true}, {"var": "flag", "ne": false}]
            }),
            &rules(),
            &[],
        )
        .unwrap();
        assert!(matches!(scenario.expect[0], Expect::Var { cmp: Cmp::Eq, value: 1.0, .. }));
        assert!(matches!(scenario.expect[1], Expect::Var { cmp: Cmp::Ne, value: 0.0, .. }));
    }

    #[test]
    fn boolean_variable_expectations_reject_ordering() {
        let errors = parse(
            &json!({
                "name": "bools",
                "players": [{"id": "p"}],
                "script": [{"player": "p", "wait": 0}],
                "expect": [{"var": "flag", "gt": true}]
            }),
            &rules(),
            &[],
        )
        .unwrap_err();
        assert!(errors.iter().any(|e| e.contains("only valid with eq or ne")), "{errors:?}");
    }
}
