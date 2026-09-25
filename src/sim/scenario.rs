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
//! seconds}` presses inputs. The run ends when the match ends (an `end` rule), when every script is done (plus a short
//! settle), or at `max_seconds`. Every mistake in a scenario is reported with a path and a did-you-mean.

use super::match_sim::MatchSim;
use super::player::{PlayerInput, PlayerState};
use super::rules::RuleSet;
use super::rules_run::GameEvent;
use super::spawns::Spawn;
use super::trace::{Header, Trace};
use crate::player::Character;
use crate::strict::check_keys;
use glam::Vec2;
use serde_json::{json, Map, Value};

const SCENARIO_KEYS: &[&str] = &["name", "spawn_group", "players", "script", "max_seconds", "settle_seconds", "expect"];
const PLAYER_KEYS: &[&str] = &["id", "character", "spawn"];
const STEP_KEYS: &[&str] = &["player", "walk", "wait", "hold", "until_event"];
const HOLD_KEYS: &[&str] = &["forward", "strafe", "sprint", "crouch", "jump", "yaw_deg", "pitch_deg", "interact", "attack", "reload", "switch", "seconds"];
const EXPECT_KEYS: &[&str] = &[
    "event",
    "no_event",
    "var",
    "ended",
    "not_ended",
    "hidden",
    "shown",
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
];
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
        if !self.vars.is_empty() {
            out.push_str(&format!("  vars: {}\n", self.vars.iter().map(|(n, v)| format!("{n}={v}")).collect::<Vec<_>>().join(", ")));
        }
        for (id, x, z, y) in &self.finals {
            out.push_str(&format!("  {id} ended at ({x:.2}, {z:.2}) y={y:.2}\n"));
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
                let character = match po.get("character").and_then(Value::as_str).unwrap_or("human") {
                    "human" => Character::Human,
                    "rat" => Character::Rat,
                    other => {
                        errs.push(format!("{pp}.character: `{other}` is not human or rat"));
                        Character::Human
                    }
                };
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
        let actions: Vec<&str> = ["walk", "wait", "hold"].into_iter().filter(|k| so.contains_key(*k)).collect();
        if actions.len() != 1 {
            errs.push(format!(
                "{sp}: give exactly one of walk, wait, hold (got {})",
                if actions.is_empty() { "none".to_string() } else { actions.join(" + ") }
            ));
            continue;
        }
        match actions[0] {
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
                let input = PlayerInput {
                    seq: 0,
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
                };
                script.push((pi, Action::Hold { input, ticks, keep_yaw: !h.contains_key("yaw_deg"), keep_pitch: !h.contains_key("pitch_deg") }, until));
            }
        }
    }
    let max_secs = o.get("max_seconds").and_then(Value::as_f64).unwrap_or(30.0);
    if !(max_secs > 0.0 && max_secs.is_finite()) {
        errs.push(format!("{p}.max_seconds: must be a number of seconds greater than 0"));
    }
    let settle = o.get("settle_seconds").and_then(Value::as_f64).unwrap_or(0.5).max(0.0);
    let mut expect = Vec::new();
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
    let main: Vec<&str> = ["event", "no_event", "var", "ended", "not_ended", "hidden", "shown", "player"].into_iter().filter(|k| eo.contains_key(*k)).collect();
    if main.len() != 1 {
        errs.push(format!(
            "{ep}: give exactly one of event, no_event, var, ended, not_ended, hidden, shown, player (got {})",
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
            let given: Vec<(Cmp, f64)> = cmps.iter().filter_map(|(k, c)| eo.get(*k).and_then(Value::as_f64).map(|v| (*c, v))).collect();
            match given.as_slice() {
                [(cmp, value)] => Some(Expect::Var { name: text, cmp: *cmp, value: *value }),
                _ => {
                    errs.push(format!("{ep}: a `var` check needs exactly one of eq, ne, gt, gte, lt, lte with a number"));
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
    stuck: Option<String>,
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
    let mut cursors: Vec<Cursor> =
        scenario.players.iter().map(|_| Cursor { mark: 0, step: 0, leg: 0, ticks_in_step: 0, best: f32::INFINITY, since_progress: 0, stuck: None }).collect();
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
                        Action::Hold { input: held, ticks, keep_yaw, keep_pitch } => {
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
                        }
                        Action::Walk(wps) => {
                            let Some(target) = wps.get(cur.leg) else {
                                (cur.step, cur.leg, cur.best, cur.since_progress) = (cur.step + 1, 0, f32::INFINITY, 0);
                                continue;
                            };
                            let to = *target - state.pos;
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
                                    cur.stuck = Some(format!(
                                        "{} got stuck on leg {} toward ({:.2}, {:.2}); stopped at ({:.2}, {:.2}) y={:.2}",
                                        scenario.players[pi].id,
                                        cur.leg + 1,
                                        target.x,
                                        target.y,
                                        state.pos.x,
                                        state.pos.y,
                                        state.foot_y
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
        if let Some(msg) = &cur.stuck {
            outcomes.push(Outcome { label: format!("walk {}", scenario.players[pi].id), ok: false, detail: msg.clone() });
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
    let ticks = sim.tick();
    let trace = sim.take_trace();
    Ok(ScenarioResult { name: scenario.name.clone(), passed, ticks, outcomes, events: history, vars, ended, finals, checksum, trace })
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
