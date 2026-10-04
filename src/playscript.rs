//! Scripted play: what a person would do at the keyboard, as data (ADR 2026-09-28-seeing-what-the-player-sees).
//!
//! `re2 --headless --script play.json` drives the real client loop with these steps instead of a keyboard, and `red_engine2 playtest` generates a script of its own.
//! A script is a JSON object `{"policy": "idle", "steps": [ ... ]}`; a step is an object with one action key, run in order, each taking the time it says (a
//! `wait`, a `hold`, a `turn ... over`, a `fire` with a duration) or none (a `look`, a `shot`, an `expect`). The runner is pure: it talks to the game through the
//! [`Driver`] trait, so it is tested with a fake one and the windowed client implements the trait for its `App`.
//!
//! ```json
//! {"policy": "sentry", "steps": [
//!   {"wait_for": {"at": "/online/in_round", "eq": true, "within": 20}},
//!   {"turn": 360, "over": 6},
//!   {"shot": "spin-end"},
//!   {"hold": ["forward", "sprint"], "secs": 2},
//!   {"aim_at": "nearest"}, {"fire": {"clicks": 3, "every": 0.25}},
//!   {"shot": "overview", "camera": "overview"},
//!   {"expect": {"at": "/remote/drawn", "eq": 7, "msg": "8 fighters means 7 drawn"}}]}
//! ```
//!
//! | action | meaning |
//! |---|---|
//! | `wait: secs` | do nothing |
//! | `look: {yaw, pitch}` | face a direction (degrees; yaw 0 looks along -Z, clockwise from above) |
//! | `turn: deg, over: secs` | turn by an angle, spread over a time (a spin: `360`) |
//! | `hold: [keys], secs` | hold `forward` `back` `left` `right` `sprint` `crouch` |
//! | `jump: true`, `interact: true`, `switch: n` | tap Space, tap E, scroll the mouse wheel |
//! | `press: id` | use the on-screen button with that id (`start` on the start card, `restart` on the end card) |
//! | `approach: id`, `look_at: id`, `interact: id` | by object instead of by keys and angles (`within`, `timeout` beside): walk up to it, face it, press E and check it is carried; the same three steps `checks.sim` has (`sim::approach`) |
//! | `fire: n` or `{clicks, every}` or `{secs}` | click n times / hold the trigger for a time; `track: true` keeps aiming at the nearest visible enemy |
//! | `aim_at: "nearest"` | turn to the nearest remote player in line of sight |
//! | `view: "first"\|"third"`, `policy: name` | camera mode; who plays between steps (`idle`, `sentry`, `walker`) |
//! | `shot: name`, `camera: ...` | save a screenshot (`first`, `third`, `overview`, `follow`, `follow:ID`, or `{eye, at, fov}`) |
//! | `snapshot: name` | store the client's state under that name in the dump |
//! | `expect: {at, eq\|ne\|min\|max\|contains\|exists, within?, msg?}` | assert on the client state (a JSON pointer into the dump); `within` waits up to that many seconds; `wait_for` is the same with a 20 s default |
//! | `say: text` | print a line |

use crate::sim::approach::{self, Approach, Progress, Target};
use glam::{Vec2, Vec3};
use serde_json::Value;

/// Keys a script can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// W.
    Forward,
    /// S.
    Back,
    /// A.
    Left,
    /// D.
    Right,
    /// Shift.
    Sprint,
    /// Ctrl.
    Crouch,
}

impl Key {
    /// Every key with its script name.
    pub const ALL: [(&'static str, Key); 6] =
        [("forward", Key::Forward), ("back", Key::Back), ("left", Key::Left), ("right", Key::Right), ("sprint", Key::Sprint), ("crouch", Key::Crouch)];

    fn parse(s: &str) -> Option<Key> {
        Self::ALL.iter().find(|(n, _)| *n == s).map(|(_, k)| *k)
    }
}

/// Who plays between the steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Policy {
    /// Stands still; only the script's steps move the player.
    #[default]
    Idle,
    /// Stands still, turns to the nearest enemy it can see and fires (the client's `RE2_AUTOAIM`).
    Sentry,
    /// Walks in a circle, turning as it goes, and fires now and then.
    Walker,
}

impl Policy {
    /// Every policy with its script name.
    pub const ALL: [(&'static str, Policy); 3] = [("idle", Policy::Idle), ("sentry", Policy::Sentry), ("walker", Policy::Walker)];
}

/// Where a screenshot is taken from.
#[derive(Debug, Clone, PartialEq)]
pub enum CameraSpec {
    /// The player's own view.
    First,
    /// Third person behind the player.
    Third,
    /// High over the map looking at its middle.
    Overview,
    /// Behind and above a remote player (`None`: the nearest).
    Follow(Option<u8>),
    /// Every local player's view at once, as the shared screen shows them.
    Split,
    /// Anywhere.
    Free {
        /// Camera position.
        eye: [f32; 3],
        /// Point looked at.
        at: [f32; 3],
        /// Vertical field of view, degrees (`None`: the player's).
        fov: Option<f32>,
    },
}

/// A check on the client's state.
#[derive(Debug, Clone, PartialEq)]
pub enum Test {
    /// Equal (numbers compare as numbers).
    Eq(Value),
    /// Not equal.
    Ne(Value),
    /// At least (a number).
    Min(f64),
    /// At most (a number).
    Max(f64),
    /// At least the first and at most the second (a range: `min` and `max` together).
    Between(f64, f64),
    /// A string contains it, or an array holds it.
    Contains(Value),
    /// The path exists (`true`) or does not (`false`).
    Exists(bool),
}

/// An assertion: `test` on the value at the JSON pointer `at`, retried for up to `within` seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Expect {
    /// JSON pointer into the state dump (`/remote/drawn`).
    pub at: String,
    /// What must hold.
    pub test: Test,
    /// How long it may take to become true, seconds.
    pub within: f32,
    /// What a failure says.
    pub msg: Option<String>,
}

/// One step of a script.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Hands the controls to local player N (1 is the first; split-screen co-op) until the next `player` step.
    Player(usize),
    /// Do nothing for that long.
    Wait(f32),
    /// Face a direction, degrees.
    Look {
        /// New yaw.
        yaw: Option<f32>,
        /// New pitch.
        pitch: Option<f32>,
    },
    /// Turn by `deg` over `over` seconds.
    Turn {
        /// Angle, degrees (positive = clockwise from above).
        deg: f32,
        /// Seconds.
        over: f32,
    },
    /// Hold keys.
    Hold {
        /// The keys.
        keys: Vec<Key>,
        /// For how long.
        secs: f32,
    },
    /// Tap jump.
    Jump,
    /// Click `clicks` times `every` seconds apart, or (`clicks` 0) hold the trigger for `hold` seconds; `track` keeps aiming at the nearest visible enemy.
    Fire {
        /// Clicks.
        clicks: u32,
        /// Seconds between clicks.
        every: f32,
        /// Seconds to hold the trigger (when `clicks` is 0).
        hold: f32,
        /// Aim at the nearest visible enemy throughout.
        track: bool,
    },
    /// Turn to the nearest remote player in line of sight.
    AimAt,
    /// Scroll the mouse wheel.
    Switch(f32),
    /// Tap E.
    Interact,
    /// Use the on-screen button with this id (`start`, `restart`).
    Press(String),
    /// Walk straight at an object until within `within` metres of it (`None`: from the body's pickup reach).
    Approach {
        /// The object's id.
        object: String,
        /// Done at this horizontal gap.
        within: Option<f32>,
        /// Seconds before it fails.
        timeout: f32,
    },
    /// Face the middle of an object.
    LookAt(String),
    /// Approach an object, face it, tap E and (where the client can tell) check it is now carried.
    InteractWith {
        /// The object's id.
        object: String,
        /// Approach distance, as for [`Step::Approach`].
        within: Option<f32>,
        /// Seconds the approach may take.
        timeout: f32,
    },
    /// First or third person.
    View {
        /// Third person.
        third: bool,
    },
    /// Change who plays between steps.
    Policy(Policy),
    /// Save a screenshot.
    Shot {
        /// Its name (the file is `<index>-<name>.png`).
        name: String,
        /// Where from.
        camera: CameraSpec,
    },
    /// Store the state under a name.
    Snapshot(String),
    /// Assert.
    Expect(Expect),
    /// Print a line.
    Say(String),
}

/// A parsed script.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    /// Who plays between the steps at the start.
    pub policy: Policy,
    /// The steps.
    pub steps: Vec<Step>,
}

/// Actions a step can be, for the "did you mean" of a typo.
pub const ACTIONS: &[&str] = &[
    "wait", "look", "turn", "hold", "jump", "fire", "aim_at", "switch", "interact", "press", "approach", "look_at", "view", "policy", "shot", "snapshot",
    "player", "expect", "wait_for", "say",
];

/// Keys a step may carry besides its action (checked so a typo in one is an error, not silently ignored).
const STEP_KEYS: &[&str] = &["over", "secs", "camera", "track", "every", "clicks", "msg", "within", "timeout"];

fn num(v: &Value, path: &str, errs: &mut Vec<String>) -> Option<f32> {
    let n = v.as_f64().map(|n| n as f32).filter(|n| n.is_finite());
    if n.is_none() {
        errs.push(format!("{path}: must be a number"));
    }
    n
}

fn secs(v: &Value, path: &str, errs: &mut Vec<String>) -> Option<f32> {
    num(v, path, errs).filter(|s| {
        let ok = (0.0..=3600.0).contains(s);
        if !ok {
            errs.push(format!("{path}: seconds must be between 0 and 3600"));
        }
        ok
    })
}

fn parse_camera(v: &Value, path: &str, errs: &mut Vec<String>) -> CameraSpec {
    match v {
        Value::String(s) => match s.as_str() {
            "first" => CameraSpec::First,
            "third" => CameraSpec::Third,
            "overview" => CameraSpec::Overview,
            "follow" | "follow:nearest" => CameraSpec::Follow(None),
            "split" => CameraSpec::Split,
            other => match other.strip_prefix("follow:").and_then(|id| id.parse::<u8>().ok()) {
                Some(id) => CameraSpec::Follow(Some(id)),
                None => {
                    errs.push(format!("{path}: unknown camera '{other}' (first, third, overview, split, follow, follow:ID, or {{eye, at, fov}})"));
                    CameraSpec::First
                }
            },
        },
        Value::Object(o) => {
            let triple = |key: &str, errs: &mut Vec<String>| -> [f32; 3] {
                let a: Vec<f32> =
                    o.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_f64().map(|n| n as f32)).collect()).unwrap_or_default();
                if a.len() != 3 {
                    errs.push(format!("{path}.{key}: must be [x, y, z]"));
                    return [0.0; 3];
                }
                [a[0], a[1], a[2]]
            };
            let (eye, at) = (triple("eye", errs), triple("at", errs));
            CameraSpec::Free { eye, at, fov: o.get("fov").and_then(Value::as_f64).map(|f| f as f32) }
        }
        _ => {
            errs.push(format!("{path}: a camera is a name or {{eye, at, fov}}"));
            CameraSpec::First
        }
    }
}

fn parse_expect(v: &Value, path: &str, default_within: f32, errs: &mut Vec<String>) -> Option<Expect> {
    let Some(o) = v.as_object() else {
        errs.push(format!("{path}: an expectation is {{at, eq|ne|min|max|contains|exists, within?, msg?}}"));
        return None;
    };
    for k in o.keys() {
        if !["at", "eq", "ne", "min", "max", "contains", "exists", "within", "msg"].contains(&k.as_str()) {
            errs.push(format!("{path}.{k}: unknown key (at, eq, ne, min, max, contains, exists, within, msg)"));
        }
    }
    let at = o.get("at").and_then(Value::as_str).filter(|a| a.starts_with('/')).map(str::to_string);
    if at.is_none() {
        errs.push(format!("{path}.at: a JSON pointer into the state dump, like \"/remote/drawn\""));
    }
    let bounds = match (o.get("min").and_then(Value::as_f64), o.get("max").and_then(Value::as_f64)) {
        (Some(lo), Some(hi)) => Some(Test::Between(lo, hi)),
        (Some(lo), None) => Some(Test::Min(lo)),
        (None, Some(hi)) => Some(Test::Max(hi)),
        (None, None) => None,
    };
    let tests: Vec<Test> = [
        o.get("eq").map(|v| Test::Eq(v.clone())),
        o.get("ne").map(|v| Test::Ne(v.clone())),
        bounds,
        o.get("contains").map(|v| Test::Contains(v.clone())),
        o.get("exists").and_then(Value::as_bool).map(Test::Exists),
    ]
    .into_iter()
    .flatten()
    .collect();
    if tests.len() != 1 {
        errs.push(format!("{path}: give exactly one of eq, ne, contains, exists, or min and/or max"));
    }
    let within = match o.get("within") {
        Some(w) => secs(w, &format!("{path}.within"), errs).unwrap_or(default_within),
        None => default_within,
    };
    Some(Expect { at: at?, test: tests.into_iter().next()?, within, msg: o.get("msg").and_then(Value::as_str).map(str::to_string) })
}

/// An object id for `approach` / `look_at` / `interact`.
fn object_id(id: &str, path: &str, action: &str, errs: &mut Vec<String>) -> Option<String> {
    if id.is_empty() {
        errs.push(format!("{path}.{action}: an object id like \"parcel_1\""));
        return None;
    }
    Some(id.to_string())
}

/// `within` (metres) and `timeout` (seconds) beside an `approach` / `interact`.
fn reach_options(o: &serde_json::Map<String, Value>, path: &str, errs: &mut Vec<String>) -> (Option<f32>, f32) {
    let positive = |k: &str, errs: &mut Vec<String>| {
        o.get(k).and_then(|v| num(v, &format!("{path}.{k}"), errs)).filter(|n| {
            let ok = *n > 0.0;
            if !ok {
                errs.push(format!("{path}.{k}: must be greater than 0"));
            }
            ok
        })
    };
    (positive("within", errs), positive("timeout", errs).unwrap_or(approach::DEFAULT_TIMEOUT_SECS))
}

fn parse_step(v: &Value, path: &str, errs: &mut Vec<String>) -> Option<Step> {
    let Some(o) = v.as_object() else {
        errs.push(format!("{path}: a step is an object with one action, like {{\"wait\": 2}}"));
        return None;
    };
    let actions: Vec<&String> = o.keys().filter(|k| !STEP_KEYS.contains(&k.as_str()) || ACTIONS.contains(&k.as_str())).collect();
    let action = match actions.as_slice() {
        [a] => a.as_str(),
        [] => {
            errs.push(format!("{path}: no action (one of {})", ACTIONS.join(", ")));
            return None;
        }
        many => {
            errs.push(format!("{path}: one action per step, found {}", many.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")));
            return None;
        }
    };
    if !ACTIONS.contains(&action) {
        let hint = crate::prefabs::suggest(action, ACTIONS.iter().copied());
        errs.push(format!(
            "{path}: unknown action '{action}'{} (actions: {})",
            hint.first().map(|h| format!(", did you mean '{h}'?")).unwrap_or_default(),
            ACTIONS.join(", ")
        ));
        return None;
    }
    let val = &o[action];
    let key = |k: &str| o.get(k);
    let step = match action {
        "wait" => Step::Wait(secs(val, &format!("{path}.wait"), errs)?),
        "look" => {
            let l = val.as_object();
            let deg = |k: &str, errs: &mut Vec<String>| l.and_then(|l| l.get(k)).and_then(|v| num(v, &format!("{path}.look.{k}"), errs));
            if l.is_none_or(|l| l.is_empty()) {
                errs.push(format!("{path}.look: give {{yaw, pitch}} in degrees"));
            }
            Step::Look { yaw: deg("yaw", errs), pitch: deg("pitch", errs) }
        }
        "turn" => {
            Step::Turn { deg: num(val, &format!("{path}.turn"), errs)?, over: key("over").map_or(Some(0.0), |v| secs(v, &format!("{path}.over"), errs))? }
        }
        "hold" => {
            let names: Vec<&str> = match val {
                Value::String(s) => vec![s.as_str()],
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            let mut keys = Vec::new();
            for n in &names {
                match Key::parse(n) {
                    Some(k) => keys.push(k),
                    None => errs.push(format!("{path}.hold: unknown key '{n}' (keys: {})", Key::ALL.map(|(n, _)| n).join(", "))),
                }
            }
            if names.is_empty() {
                errs.push(format!("{path}.hold: give a key or a list of keys"));
            }
            Step::Hold { keys, secs: key("secs").map_or(Some(1.0), |v| secs(v, &format!("{path}.secs"), errs))? }
        }
        "jump" => Step::Jump,
        "interact" => match val.as_str() {
            Some(id) => {
                let (within, timeout) = reach_options(o, path, errs);
                Step::InteractWith { object: object_id(id, path, "interact", errs)?, within, timeout }
            }
            None => Step::Interact,
        },
        "approach" => {
            let (within, timeout) = reach_options(o, path, errs);
            Step::Approach { object: object_id(val.as_str().unwrap_or(""), path, "approach", errs)?, within, timeout }
        }
        "look_at" => Step::LookAt(object_id(val.as_str().unwrap_or(""), path, "look_at", errs)?),
        "fire" => {
            let (mut clicks, mut every, mut hold, mut track) = (1u32, 0.25f32, 0.0f32, key("track").and_then(Value::as_bool).unwrap_or(false));
            match val {
                Value::Number(n) => clicks = n.as_u64().unwrap_or(1).min(1000) as u32,
                Value::Object(f) => {
                    for k in f.keys() {
                        if !["clicks", "every", "secs", "track"].contains(&k.as_str()) {
                            errs.push(format!("{path}.fire.{k}: unknown key (clicks, every, secs, track)"));
                        }
                    }
                    if let Some(s) = f.get("secs") {
                        hold = secs(s, &format!("{path}.fire.secs"), errs).unwrap_or(0.0);
                        clicks = 0;
                    }
                    if let Some(c) = f.get("clicks").and_then(Value::as_u64) {
                        clicks = c.min(1000) as u32;
                    }
                    if let Some(e) = f.get("every") {
                        every = secs(e, &format!("{path}.fire.every"), errs).unwrap_or(every).max(0.05);
                    }
                    track |= f.get("track").and_then(Value::as_bool).unwrap_or(false);
                }
                _ => errs.push(format!("{path}.fire: a number of clicks, or {{clicks, every}}, or {{secs}}")),
            }
            Step::Fire { clicks, every, hold, track }
        }
        "aim_at" => {
            if val.as_str() != Some("nearest") {
                errs.push(format!("{path}.aim_at: only \"nearest\" for now"));
            }
            Step::AimAt
        }
        "switch" => Step::Switch(num(val, &format!("{path}.switch"), errs)?),
        "player" => match val.as_u64().filter(|n| (1..=4).contains(n)) {
            Some(n) => Step::Player(n as usize),
            None => {
                errs.push(format!("{path}.player: a local player number from 1 to 4"));
                return None;
            }
        },
        "press" => match val.as_str().filter(|id| !id.is_empty()) {
            Some(id) => Step::Press(id.to_string()),
            None => {
                errs.push(format!("{path}.press: a button id like \"start\" or \"restart\""));
                return None;
            }
        },
        "view" => Step::View {
            third: match val.as_str() {
                Some("first") => false,
                Some("third") => true,
                _ => {
                    errs.push(format!("{path}.view: \"first\" or \"third\""));
                    false
                }
            },
        },
        "policy" => match val.as_str().and_then(|n| Policy::ALL.iter().find(|(p, _)| *p == n)) {
            Some((_, p)) => Step::Policy(*p),
            None => {
                errs.push(format!("{path}.policy: one of {}", Policy::ALL.map(|(n, _)| n).join(", ")));
                return None;
            }
        },
        "shot" => {
            let name = val.as_str().filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
            if name.is_none() {
                errs.push(format!("{path}.shot: a name of letters, digits, - and _ (it becomes part of the file name)"));
            }
            Step::Shot { name: name?.to_string(), camera: key("camera").map_or(CameraSpec::First, |c| parse_camera(c, &format!("{path}.camera"), errs)) }
        }
        "snapshot" => Step::Snapshot(val.as_str().filter(|n| !n.is_empty()).map(str::to_string).or_else(|| {
            errs.push(format!("{path}.snapshot: a name"));
            None
        })?),
        "expect" => Step::Expect(parse_expect(val, &format!("{path}.expect"), 0.0, errs)?),
        "wait_for" => Step::Expect(parse_expect(val, &format!("{path}.wait_for"), 20.0, errs)?),
        "say" => Step::Say(val.as_str().unwrap_or_default().to_string()),
        _ => unreachable!("ACTIONS lists every action"),
    };
    // A key that is neither the action nor one this action reads is a typo.
    let used: &[&str] = match action {
        "turn" => &["over"],
        "hold" => &["secs"],
        "fire" => &["track"],
        "shot" => &["camera"],
        "approach" => &["within", "timeout"],
        "interact" if val.is_string() => &["within", "timeout"],
        _ => &[],
    };
    for k in o.keys() {
        if k != action && !used.contains(&k.as_str()) {
            errs.push(format!("{path}.{k}: '{action}' does not take '{k}'"));
        }
    }
    Some(step)
}

impl Script {
    /// Parses a script; every problem is `steps[3].fire: message`.
    pub fn parse(text: &str) -> Result<Script, Vec<String>> {
        let v: Value = serde_json::from_str(text).map_err(|e| vec![format!("script: not valid JSON: {e}")])?;
        let Some(root) = v.as_object() else { return Err(vec!["script: must be an object {\"policy\": ..., \"steps\": [...]}".to_string()]) };
        let mut errs = Vec::new();
        for k in root.keys() {
            if !["policy", "steps", "schema", "notes"].contains(&k.as_str()) {
                errs.push(format!("script.{k}: unknown key (policy, steps)"));
            }
        }
        let policy = match root.get("policy").and_then(Value::as_str) {
            None => Policy::Idle,
            Some(n) => match Policy::ALL.iter().find(|(p, _)| *p == n) {
                Some((_, p)) => *p,
                None => {
                    errs.push(format!("script.policy: one of {}", Policy::ALL.map(|(n, _)| n).join(", ")));
                    Policy::Idle
                }
            },
        };
        let mut steps = Vec::new();
        match root.get("steps").and_then(Value::as_array) {
            Some(list) => {
                for (i, s) in list.iter().enumerate() {
                    steps.extend(parse_step(s, &format!("steps[{i}]"), &mut errs));
                }
            }
            None => errs.push("script.steps: a list of steps".to_string()),
        }
        if errs.is_empty() {
            Ok(Script { policy, steps })
        } else {
            Err(errs)
        }
    }

    /// The seconds the script takes if nothing waits longer than it says (steps with `within` may take longer).
    pub fn nominal_secs(&self) -> f32 {
        self.steps
            .iter()
            .map(|s| match s {
                Step::Wait(t) => *t,
                Step::Turn { over, .. } => *over,
                Step::Hold { secs, .. } => *secs,
                Step::Fire { clicks, every, hold, .. } => *clicks as f32 * every + hold,
                Step::Expect(e) => e.within,
                Step::Approach { timeout, .. } | Step::InteractWith { timeout, .. } => *timeout + 1.0,
                _ => 0.0,
            })
            .sum()
    }
}

/// Seconds an `interact` aims at its object before it presses.
const AIM_SECS: f32 = 0.1;
/// Seconds an `interact` waits for the object to be carried before it fails.
const PICKUP_SECS: f32 = 1.5;

/// What the runner asks of the game. The windowed client implements it for its `App`; tests use a fake.
pub trait Driver {
    /// Presses or releases keys.
    fn hold_keys(&mut self, keys: &[Key], down: bool);
    /// Sets the look direction (degrees; `None` keeps that axis).
    fn set_look(&mut self, yaw_deg: Option<f32>, pitch_deg: Option<f32>);
    /// Turns by an angle, degrees.
    fn add_yaw(&mut self, deg: f32);
    /// Taps the jump key.
    fn jump(&mut self);
    /// Presses (`true`) or releases the fire button.
    fn fire(&mut self, down: bool);
    /// Scrolls the wheel.
    fn scroll(&mut self, lines: f32);
    /// Taps E.
    fn interact(&mut self);
    /// Turns to the nearest visible remote player; whether there was one.
    fn aim_at_nearest(&mut self) -> bool;
    /// First or third person.
    fn set_view(&mut self, third: bool);
    /// Who plays between steps.
    fn set_policy(&mut self, policy: Policy);
    /// Saves a screenshot; `Err` says why it could not.
    fn shot(&mut self, name: &str, camera: &CameraSpec) -> Result<(), String>;
    /// Stores the current state under `name`.
    fn snapshot(&mut self, name: &str);
    /// The current state, for `expect`.
    fn state(&self) -> Value;
    /// Prints a line.
    fn say(&mut self, text: &str);
    /// Hands the controls to local player `player` (0 is the first); `Err` says why that is not possible (a client with one player).
    fn set_player(&mut self, _player: usize) -> Result<(), String> {
        Err("this client has a single player".to_string())
    }
    /// Uses the on-screen button with this id; `Err` says why there is none.
    fn press(&mut self, _id: &str) -> Result<(), String> {
        Err("this client has no buttons to press".to_string())
    }
    /// Where the player is, for the object steps (`None`: this driver cannot say, and those steps fail).
    fn pose(&self) -> Option<Pose> {
        None
    }
    /// Where the object with this id is right now (a loose prop where it has been moved to); `None` when there is no such top-level object.
    fn locate(&self, _id: &str) -> Option<Target> {
        None
    }
    /// Whether the player carries the object `id`; `None` when the client cannot tell (a prop's holder is the server's to know online).
    fn carrying(&self, _id: &str) -> Option<bool> {
        None
    }
}

/// The player, as the object steps need to know it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Feet, (x, z).
    pub pos: Vec2,
    /// The eye.
    pub eye: Vec3,
    /// How far the body reaches to pick something up, m.
    pub pickup_reach: f32,
}

/// Why an expectation failed, or `None` when `e` holds in `state`.
pub fn check(e: &Expect, state: &Value) -> Option<String> {
    let got = state.pointer(&e.at);
    let describe = |want: &str| {
        format!("{}: {} is {}, expected {want}", e.msg.as_deref().unwrap_or("expectation failed"), e.at, got.map_or("missing".to_string(), Value::to_string))
    };
    let number = |v: &Value| v.as_f64();
    match (&e.test, got) {
        (Test::Exists(want), g) => (g.is_some() != *want).then(|| describe(if *want { "it to exist" } else { "it to be absent" })),
        (_, None) => Some(describe("a value")),
        (Test::Eq(w), Some(g)) => {
            let same = match (number(w), number(g)) {
                (Some(a), Some(b)) => (a - b).abs() < 1e-9,
                _ => w == g,
            };
            (!same).then(|| describe(&format!("{w}")))
        }
        (Test::Ne(w), Some(g)) => (w == g).then(|| describe(&format!("anything but {w}"))),
        (Test::Min(m), Some(g)) => (!number(g).is_some_and(|n| n >= *m)).then(|| describe(&format!("at least {m}"))),
        (Test::Max(m), Some(g)) => (!number(g).is_some_and(|n| n <= *m)).then(|| describe(&format!("at most {m}"))),
        (Test::Between(lo, hi), Some(g)) => (!number(g).is_some_and(|n| n >= *lo && n <= *hi)).then(|| describe(&format!("between {lo} and {hi}"))),
        (Test::Contains(w), Some(g)) => {
            let holds = match (g, w) {
                (Value::String(s), Value::String(t)) => s.contains(t.as_str()),
                (Value::Array(a), w) => a.contains(w),
                _ => false,
            };
            (!holds).then(|| describe(&format!("to contain {w}")))
        }
    }
}

/// Runs a [`Script`] against a [`Driver`], a frame at a time.
pub struct Runner {
    policy: Policy,
    steps: Vec<Step>,
    index: usize,
    /// Seconds into the current step.
    elapsed: f32,
    entered: bool,
    /// Failed expectations, in order.
    pub failures: Vec<String>,
    /// Total script time so far.
    pub secs: f32,
    /// Clicks of the current `fire` step already made, and whether the button is down.
    clicked: u32,
    down_until: Option<f32>,
    /// The running `approach` (or the approach part of an `interact`), and which part of an `interact` it is: 0 approach, 1 aim, 2 settle after the press.
    approach: Option<Approach>,
    phase: u8,
}

/// How a frame of an object step went.
enum Drive {
    /// Not done: come back next frame.
    Going,
    /// Done.
    Done,
    /// Failed, and why.
    Failed(String),
}

impl Runner {
    /// Starts `script`. Its [`policy`](Runner::policy) is for the caller to put in force before the first frame.
    pub fn new(script: Script) -> Runner {
        Runner {
            policy: script.policy,
            steps: script.steps,
            index: 0,
            elapsed: 0.0,
            entered: false,
            failures: Vec::new(),
            secs: 0.0,
            clicked: 0,
            down_until: None,
            approach: None,
            phase: 0,
        }
    }

    /// Who plays between the steps at the start.
    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// Whether every step has run.
    pub fn finished(&self) -> bool {
        self.index >= self.steps.len()
    }

    /// The step being run, for progress messages.
    pub fn current(&self) -> Option<&Step> {
        self.steps.get(self.index)
    }

    fn next(&mut self) {
        self.index += 1;
        self.elapsed = 0.0;
        self.entered = false;
        self.clicked = 0;
        self.down_until = None;
        self.approach = None;
        self.phase = 0;
    }

    /// One frame of walking at `object`: aim the body at it, hold forward, and ask [`Approach`] whether it is there yet.
    fn drive_approach(&mut self, d: &mut dyn Driver, object: &str, within: Option<f32>, timeout: f32, dt: f32, what: &str) -> Drive {
        let (Some(pose), Some(target)) = (d.pose(), d.locate(object)) else {
            d.hold_keys(&[Key::Forward], false);
            return Drive::Failed(format!(
                "{what} `{object}`: {}",
                if d.pose().is_none() {
                    "this client cannot say where the player is"
                } else {
                    "no such top-level object in the scene (nested children have no id of their own)"
                }
            ));
        };
        let goal = within.unwrap_or_else(|| approach::default_within(pose.pickup_reach));
        let gap = target.gap(pose.pos);
        match self.approach.get_or_insert_with(|| Approach::new(goal, timeout)).step(dt, gap) {
            Progress::Arrived => {
                d.hold_keys(&[Key::Forward], false);
                Drive::Done
            }
            Progress::Moving => {
                d.set_look(Some(target.heading(pose.pos).to_degrees()), None);
                d.hold_keys(&[Key::Forward], true);
                Drive::Going
            }
            Progress::Failed(why) => {
                d.hold_keys(&[Key::Forward], false);
                let secs = self.approach.as_ref().map_or(0.0, Approach::elapsed);
                Drive::Failed(approach::failure_message(what, object, why, gap, goal, pose.pos, secs))
            }
        }
    }

    /// Faces the middle of `object` from the eye; false when it cannot be found.
    fn face(d: &mut dyn Driver, object: &str) -> bool {
        let (Some(pose), Some(target)) = (d.pose(), d.locate(object)) else { return false };
        let (yaw, pitch) = approach::aim(pose.eye, target.aim_point());
        d.set_look(Some(yaw.to_degrees()), Some(pitch.to_degrees()));
        true
    }

    /// Runs the script for `dt` seconds of game time: instant steps run at once, timed steps take their time and hand what is left of the frame to the next.
    pub fn advance(&mut self, dt: f32, d: &mut dyn Driver) {
        let mut budget = dt.max(0.0);
        self.secs += budget;
        let mut guard = 0;
        while !self.finished() && guard < 10_000 {
            guard += 1;
            let step = self.steps[self.index].clone();
            let first = !self.entered;
            self.entered = true;
            match step {
                Step::Wait(t) => {
                    let take = (t - self.elapsed).min(budget);
                    self.elapsed += take;
                    budget -= take;
                    if self.elapsed + 1e-6 < t {
                        return;
                    }
                }
                Step::Look { yaw, pitch } => d.set_look(yaw, pitch),
                Step::Turn { deg, over } => {
                    if over <= 0.0 {
                        d.add_yaw(deg);
                    } else {
                        let take = (over - self.elapsed).min(budget);
                        d.add_yaw(deg * take / over);
                        self.elapsed += take;
                        budget -= take;
                        if self.elapsed + 1e-6 < over {
                            return;
                        }
                    }
                }
                Step::Hold { keys, secs } => {
                    if first {
                        d.hold_keys(&keys, true);
                    }
                    let take = (secs - self.elapsed).min(budget);
                    self.elapsed += take;
                    budget -= take;
                    if self.elapsed + 1e-6 < secs {
                        return;
                    }
                    d.hold_keys(&keys, false);
                }
                Step::Jump => d.jump(),
                Step::Interact => d.interact(),
                Step::Approach { object, within, timeout } => match self.drive_approach(d, &object, within, timeout, budget, "approach") {
                    Drive::Going => return,
                    Drive::Done => {}
                    Drive::Failed(why) => self.failures.push(why),
                },
                Step::LookAt(object) => {
                    if !Self::face(d, &object) {
                        self.failures.push(format!("look_at `{object}`: no such top-level object in the scene, or this client cannot say where the player is"));
                    }
                }
                Step::InteractWith { object, within, timeout } => {
                    // 0: walk up to it. 1: face it for a moment, then press. 2: give the pick-up a moment to show, then check.
                    if self.phase == 0 {
                        if d.carrying(&object) == Some(true) {
                            self.next();
                            continue;
                        }
                        match self.drive_approach(d, &object, within, timeout, budget, "interact") {
                            Drive::Going => return,
                            Drive::Done => (self.phase, self.elapsed) = (1, 0.0),
                            Drive::Failed(why) => {
                                self.failures.push(why);
                                self.next();
                                continue;
                            }
                        }
                    }
                    if self.phase == 1 {
                        Self::face(d, &object);
                        let take = (AIM_SECS - self.elapsed).min(budget);
                        self.elapsed += take;
                        budget -= take;
                        if self.elapsed + 1e-6 < AIM_SECS {
                            return;
                        }
                        d.interact();
                        (self.phase, self.elapsed) = (2, 0.0);
                    }
                    // Online the server answers a press a round trip later: wait for the pick-up, up to PICKUP_SECS.
                    let carried = d.carrying(&object);
                    if carried == Some(false) && self.elapsed + 1e-6 < PICKUP_SECS {
                        self.elapsed += budget;
                        return;
                    }
                    if carried == Some(false) {
                        let at = d.pose().map(|p| format!(" from ({:.2}, {:.2})", p.pos.x, p.pos.y)).unwrap_or_default();
                        let crosshair = d.state().pointer("/crosshair").map(Value::to_string).unwrap_or_default();
                        self.failures.push(format!("interact `{object}`: pressed interact{at} but did not pick it up (crosshair {crosshair})"));
                    }
                }
                Step::Press(id) => {
                    if let Err(why) = d.press(&id) {
                        self.failures.push(format!("press `{id}`: {why}"));
                    }
                }
                Step::Switch(n) => d.scroll(n),
                Step::Player(n) => {
                    if let Err(e) = d.set_player(n - 1) {
                        self.failures.push(format!("player {n}: {e}"));
                    }
                }
                Step::View { third } => d.set_view(third),
                Step::Policy(p) => d.set_policy(p),
                Step::AimAt => {
                    d.aim_at_nearest();
                }
                Step::Fire { clicks, every, hold, track } => {
                    let total = if clicks == 0 { hold } else { clicks as f32 * every };
                    if track {
                        d.aim_at_nearest();
                    }
                    if clicks == 0 {
                        if first {
                            d.fire(true);
                        }
                    } else {
                        // Click n at n * every; each press lasts a moment so the game sees an edge.
                        if self.down_until.is_some_and(|t| self.elapsed >= t) {
                            d.fire(false);
                            self.down_until = None;
                        }
                        while self.clicked < clicks && self.elapsed + 1e-6 >= self.clicked as f32 * every {
                            if self.down_until.is_some() {
                                d.fire(false);
                            }
                            d.fire(true);
                            self.down_until = Some(self.clicked as f32 * every + 0.06);
                            self.clicked += 1;
                        }
                    }
                    let take = (total - self.elapsed).min(budget);
                    self.elapsed += take;
                    budget -= take;
                    if self.elapsed + 1e-6 < total {
                        return;
                    }
                    d.fire(false);
                }
                Step::Shot { name, camera } => {
                    if let Err(e) = d.shot(&name, &camera) {
                        self.failures.push(format!("shot '{name}': {e}"));
                    }
                }
                Step::Snapshot(name) => d.snapshot(&name),
                Step::Say(text) => d.say(&text),
                Step::Expect(e) => {
                    let mut failed = check(&e, &d.state());
                    if failed.is_some() && self.elapsed + 1e-6 < e.within {
                        // It may still come true: wait, and ask again next frame (or, when the time is up, one last look).
                        let take = (e.within - self.elapsed).min(budget);
                        self.elapsed += take;
                        budget -= take;
                        if self.elapsed + 1e-6 < e.within {
                            return;
                        }
                        failed = check(&e, &d.state());
                    }
                    if let Some(why) = failed {
                        self.failures.push(if e.within > 0.0 { format!("{why} (after waiting {} s)", e.within) } else { why });
                    }
                }
            }
            self.next();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A game that only records what it was asked.
    #[derive(Default)]
    struct Fake {
        log: Vec<String>,
        state: Value,
        yaw: f32,
        shots_fail: bool,
    }

    impl Driver for Fake {
        fn hold_keys(&mut self, keys: &[Key], down: bool) {
            self.log.push(format!("keys {keys:?} {down}"));
        }
        fn set_look(&mut self, yaw: Option<f32>, pitch: Option<f32>) {
            self.log.push(format!("look {yaw:?} {pitch:?}"));
        }
        fn add_yaw(&mut self, deg: f32) {
            self.yaw += deg;
        }
        fn jump(&mut self) {
            self.log.push("jump".into());
        }
        fn fire(&mut self, down: bool) {
            self.log.push(format!("fire {down}"));
        }
        fn scroll(&mut self, lines: f32) {
            self.log.push(format!("scroll {lines}"));
        }
        fn interact(&mut self) {
            self.log.push("interact".into());
        }
        fn aim_at_nearest(&mut self) -> bool {
            self.log.push("aim".into());
            true
        }
        fn set_view(&mut self, third: bool) {
            self.log.push(format!("view {third}"));
        }
        fn set_policy(&mut self, p: Policy) {
            self.log.push(format!("policy {p:?}"));
        }
        fn shot(&mut self, name: &str, camera: &CameraSpec) -> Result<(), String> {
            self.log.push(format!("shot {name} {camera:?}"));
            if self.shots_fail {
                Err("no GPU".into())
            } else {
                Ok(())
            }
        }
        fn snapshot(&mut self, name: &str) {
            self.log.push(format!("snapshot {name}"));
        }
        fn state(&self) -> Value {
            self.state.clone()
        }
        fn say(&mut self, text: &str) {
            self.log.push(format!("say {text}"));
        }
    }

    fn run(script: &str, frames: usize, dt: f32, fake: &mut Fake) -> Runner {
        let mut r = Runner::new(Script::parse(script).unwrap_or_else(|e| panic!("{e:?}")));
        for _ in 0..frames {
            r.advance(dt, fake);
        }
        r
    }

    /// A player on a flat floor who walks where it looks while forward is held (4 m/s), with one crate and an optional wall that stops it at z = 3.
    struct Walker {
        pos: Vec2,
        yaw_deg: f32,
        forward: bool,
        crate_at: Vec3,
        wall: bool,
        carrying: Option<bool>,
        pickups: bool,
        presses: u32,
    }

    impl Walker {
        fn new(crate_at: Vec3) -> Walker {
            Walker { pos: Vec2::ZERO, yaw_deg: 0.0, forward: false, crate_at, wall: false, carrying: Some(false), pickups: true, presses: 0 }
        }
        /// One frame of the world: walk along the yaw while forward is held.
        fn tick(&mut self, dt: f32) {
            if self.forward {
                let yaw = self.yaw_deg.to_radians();
                let next = self.pos + Vec2::new(yaw.sin(), -yaw.cos()) * 4.0 * dt;
                if !(self.wall && next.y > 3.0) {
                    self.pos = next;
                }
            }
        }
    }

    impl Driver for Walker {
        fn hold_keys(&mut self, keys: &[Key], down: bool) {
            if keys.contains(&Key::Forward) {
                self.forward = down;
            }
        }
        fn set_look(&mut self, yaw: Option<f32>, _pitch: Option<f32>) {
            self.yaw_deg = yaw.unwrap_or(self.yaw_deg);
        }
        fn add_yaw(&mut self, _: f32) {}
        fn jump(&mut self) {}
        fn fire(&mut self, _: bool) {}
        fn scroll(&mut self, _: f32) {}
        fn interact(&mut self) {
            self.presses += 1;
            let close = Vec2::new(self.crate_at.x, self.crate_at.z).distance(self.pos) < 2.3;
            if self.pickups && self.carrying.is_some() && close {
                self.carrying = Some(true);
            }
        }
        fn aim_at_nearest(&mut self) -> bool {
            false
        }
        fn set_view(&mut self, _: bool) {}
        fn set_policy(&mut self, _: Policy) {}
        fn shot(&mut self, _: &str, _: &CameraSpec) -> Result<(), String> {
            Ok(())
        }
        fn snapshot(&mut self, _: &str) {}
        fn state(&self) -> Value {
            json!({"crosshair": {"pickup": false}})
        }
        fn say(&mut self, _: &str) {}
        fn pose(&self) -> Option<Pose> {
            Some(Pose { pos: self.pos, eye: Vec3::new(self.pos.x, 1.6, self.pos.y), pickup_reach: 2.3 })
        }
        fn locate(&self, id: &str) -> Option<Target> {
            (id == "crate").then(|| Target::from_prop(self.crate_at, Vec3::splat(0.6)))
        }
        fn carrying(&self, id: &str) -> Option<bool> {
            (id == "crate").then_some(self.carrying).flatten()
        }
    }

    /// Plays `script` against `w` at 60 frames a second for up to `secs`, moving the world between frames.
    fn play(script: &str, w: &mut Walker, secs: f32) -> Runner {
        let mut r = Runner::new(Script::parse(script).unwrap_or_else(|e| panic!("{e:?}")));
        let dt = 1.0 / 60.0;
        for _ in 0..(secs / dt) as usize {
            if r.finished() {
                break;
            }
            r.advance(dt, w);
            w.tick(dt);
        }
        r
    }

    #[test]
    fn approach_walks_at_the_object_and_stops_within_reach() {
        let mut w = Walker::new(Vec3::new(5.0, 0.0, -8.0));
        let r = play(r#"{"steps":[{"approach":"crate"}]}"#, &mut w, 10.0);
        assert!(r.finished() && r.failures.is_empty(), "{:?}", r.failures);
        let gap = Target::from_prop(w.crate_at, Vec3::splat(0.6)).gap(w.pos);
        assert!(gap <= 1.38 && gap > 0.9, "stopped at the default 60% of reach, not on top of it: {gap}");
        assert!(!w.forward, "the key is released");
        // An explicit `within` is honoured.
        let mut w = Walker::new(Vec3::new(5.0, 0.0, -8.0));
        let r = play(r#"{"steps":[{"approach":"crate","within":0.5}]}"#, &mut w, 10.0);
        assert!(r.failures.is_empty() && Target::from_prop(w.crate_at, Vec3::splat(0.6)).gap(w.pos) <= 0.5);
    }

    #[test]
    fn interact_by_id_approaches_faces_presses_once_and_checks_the_pickup() {
        let mut w = Walker::new(Vec3::new(-4.0, 0.0, -6.0));
        let r = play(r#"{"steps":[{"interact":"crate"},{"say":"after"}]}"#, &mut w, 15.0);
        assert!(r.finished() && r.failures.is_empty(), "{:?}", r.failures);
        assert_eq!((w.presses, w.carrying), (1, Some(true)));
        // Already carrying it: nothing to do, and no press that would drop it.
        let mut w = Walker::new(Vec3::new(-4.0, 0.0, -6.0));
        w.carrying = Some(true);
        let r = play(r#"{"steps":[{"interact":"crate"}]}"#, &mut w, 5.0);
        assert!(r.finished() && r.failures.is_empty() && w.presses == 0 && w.pos == Vec2::ZERO);
    }

    #[test]
    fn an_object_step_that_cannot_finish_fails_by_name_and_the_script_goes_on() {
        // Something in the way (a wall stops the player at z = 3 on the way to z = 9).
        let mut w = Walker::new(Vec3::new(0.0, 0.0, 9.0));
        w.wall = true;
        let r = play(r#"{"steps":[{"approach":"crate"},{"say":"still running"}]}"#, &mut w, 20.0);
        assert!(r.finished(), "a failed step does not hang the script");
        assert!(r.failures.len() == 1 && r.failures[0].contains("approach `crate`") && r.failures[0].contains("in the way"), "{:?}", r.failures);
        // Too slow for its own timeout.
        let mut w = Walker::new(Vec3::new(0.0, 0.0, -30.0));
        let r = play(r#"{"steps":[{"approach":"crate","timeout":1}]}"#, &mut w, 20.0);
        assert!(r.failures[0].contains("raise `timeout`"), "{:?}", r.failures);
        // The press took nothing.
        let mut w = Walker::new(Vec3::new(0.0, 0.0, -5.0));
        w.pickups = false;
        let r = play(r#"{"steps":[{"interact":"crate"}]}"#, &mut w, 20.0);
        assert!(r.failures.len() == 1 && r.failures[0].contains("did not pick it up"), "{:?}", r.failures);
        // No such object.
        let mut w = Walker::new(Vec3::new(0.0, 0.0, -5.0));
        let r = play(r#"{"steps":[{"look_at":"bench"},{"approach":"bench"}]}"#, &mut w, 5.0);
        assert_eq!(r.failures.len(), 2, "{:?}", r.failures);
        assert!(r.failures.iter().all(|f| f.contains("bench") && f.contains("no such top-level object")), "{:?}", r.failures);
    }

    #[test]
    fn a_client_that_cannot_tell_who_carries_a_prop_is_not_failed_for_it() {
        let mut w = Walker::new(Vec3::new(2.0, 0.0, -3.0));
        w.carrying = None;
        let r = play(r#"{"steps":[{"interact":"crate"}]}"#, &mut w, 10.0);
        assert!(r.finished() && r.failures.is_empty() && w.presses == 1, "{:?}", r.failures);
    }

    #[test]
    fn look_at_faces_the_middle_of_the_object() {
        let mut w = Walker::new(Vec3::new(5.0, 0.0, 0.0));
        let r = play(r#"{"steps":[{"look_at":"crate"}]}"#, &mut w, 1.0);
        assert!(r.finished() && r.failures.is_empty());
        assert!((w.yaw_deg - 90.0).abs() < 0.1, "a crate to the east is yaw 90, got {}", w.yaw_deg);
    }

    #[test]
    fn object_steps_parse_with_their_options_and_reject_what_they_do_not_take() {
        let s =
            Script::parse(r#"{"steps":[{"approach":"p","within":1.2,"timeout":5},{"look_at":"p"},{"interact":"p","timeout":3},{"interact":true}]}"#).unwrap();
        assert_eq!(s.steps[0], Step::Approach { object: "p".into(), within: Some(1.2), timeout: 5.0 });
        assert_eq!(s.steps[1], Step::LookAt("p".into()));
        assert_eq!(s.steps[2], Step::InteractWith { object: "p".into(), within: None, timeout: 3.0 });
        assert_eq!(s.steps[3], Step::Interact, "interact with no object is still a tap of E");
        let e = Script::parse(r#"{"steps":[{"look_at":"p","within":1},{"approach":"","timeout":-1},{"interact":true,"within":1},{"aprroach":"p"}]}"#)
            .unwrap_err()
            .join("\n");
        assert!(e.contains("'look_at' does not take 'within'") && e.contains("an object id") && e.contains("must be greater than 0"), "{e}");
        assert!(e.contains("'interact' does not take 'within'") && e.contains("did you mean 'approach'"), "{e}");
    }

    #[test]
    fn press_takes_a_button_id_asks_the_driver_and_a_refusal_is_a_failure() {
        let s = Script::parse(r#"{"steps":[{"press":"start"}]}"#).unwrap();
        assert_eq!(s.steps, vec![Step::Press("start".into())]);
        let e = Script::parse(r#"{"steps":[{"press":""},{"press":3}]}"#).unwrap_err().join("\n");
        assert!(e.contains("a button id"), "{e}");
        // A driver with no buttons refuses, and the script says which button and why.
        let mut fake = Fake::default();
        let r = run(r#"{"steps":[{"press":"start"},{"say":"after"}]}"#, 3, 0.1, &mut fake);
        assert!(r.finished(), "a failed press does not stop the script");
        assert_eq!(r.failures.len(), 1);
        assert!(r.failures[0].contains("press `start`") && r.failures[0].contains("no buttons"), "{:?}", r.failures);
    }

    #[test]
    fn a_script_parses_every_action_and_names_what_is_wrong_with_a_bad_one() {
        let s = Script::parse(
            r#"{"policy":"sentry","steps":[{"wait":1},{"look":{"yaw":90,"pitch":-5}},{"turn":360,"over":4},{"hold":["forward","sprint"],"secs":2},{"jump":true},
            {"fire":3},{"fire":{"clicks":2,"every":0.5}},{"fire":{"secs":1.5,"track":true}},{"aim_at":"nearest"},{"switch":1},{"interact":true},{"view":"third"},
            {"policy":"walker"},{"shot":"a-1","camera":"overview"},{"shot":"b","camera":{"eye":[0,10,0],"at":[0,0,0],"fov":70}},{"snapshot":"s"},
            {"expect":{"at":"/remote/drawn","eq":7,"msg":"seven"}},{"wait_for":{"at":"/online/in_round","eq":true}},{"say":"hi"}]}"#,
        )
        .unwrap();
        assert_eq!(s.policy, Policy::Sentry);
        assert_eq!(s.steps.len(), 19);
        assert!(matches!(&s.steps[7], Step::Fire { clicks: 0, hold, track: true, .. } if (*hold - 1.5).abs() < 1e-6));
        assert!(matches!(&s.steps[17], Step::Expect(e) if e.within == 20.0), "wait_for waits by default");
        let bad = Script::parse(r#"{"steps":[{"fier":3},{"hold":["up"],"secs":1},{"wait":-2},{"shot":"has space"},{"wait":1,"secs":2},{"expect":{"at":"remote","eq":1}},{"camera":"x"}]}"#)
            .unwrap_err()
            .join("\n");
        assert!(bad.contains("unknown action 'fier', did you mean 'fire'?"), "{bad}");
        assert!(bad.contains("unknown key 'up'") && bad.contains("seconds must be between"), "{bad}");
        assert!(bad.contains("steps[3].shot") && bad.contains("'wait' does not take 'secs'") && bad.contains("JSON pointer"), "{bad}");
        assert!(bad.contains("steps[6]"), "a step with a camera and no action is named: {bad}");
        assert!(Script::parse("[1]").unwrap_err()[0].contains("must be an object"));
        assert!(Script::parse(r#"{"steps":[],"stepz":1}"#).unwrap_err()[0].contains("unknown key"));
    }

    #[test]
    fn timed_steps_take_their_time_and_hand_the_rest_of_a_frame_on() {
        let mut d = Fake::default();
        // 60 Hz frames: wait 0.5 s, then hold forward 1 s, then jump.
        let r = run(r#"{"steps":[{"wait":0.5},{"hold":["forward"],"secs":1.0},{"jump":true}]}"#, 89, 1.0 / 60.0, &mut d);
        assert!(!r.finished(), "1.483 s in: the hold still has time left");
        assert_eq!(d.log, ["keys [Forward] true"]);
        let mut r = r;
        r.advance(1.0 / 60.0, &mut d);
        r.advance(1.0 / 60.0, &mut d);
        assert!(r.finished(), "the hold ended and jump ran in the same frame the time ran out");
        assert_eq!(d.log, ["keys [Forward] true", "keys [Forward] false", "jump"]);
    }

    #[test]
    fn a_turn_spreads_its_angle_over_its_time_and_a_spin_is_a_full_circle() {
        let mut d = Fake::default();
        let r = run(r#"{"steps":[{"turn":360,"over":2}]}"#, 130, 1.0 / 60.0, &mut d);
        assert!(r.finished());
        assert!((d.yaw - 360.0).abs() < 0.01, "{}", d.yaw);
        let mut d = Fake::default();
        run(r#"{"steps":[{"turn":90}]}"#, 1, 0.016, &mut d);
        assert_eq!(d.yaw, 90.0, "without a time it is instant");
    }

    #[test]
    fn firing_clicks_at_its_pace_and_a_held_trigger_stays_down_for_its_time() {
        let mut d = Fake::default();
        run(r#"{"steps":[{"fire":{"clicks":3,"every":0.25}}]}"#, 60, 1.0 / 60.0, &mut d);
        let presses = d.log.iter().filter(|l| *l == "fire true").count();
        assert_eq!(presses, 3, "{:?}", d.log);
        assert!(d.log.iter().filter(|l| *l == "fire false").count() >= 3, "each press is released: {:?}", d.log);
        let mut d = Fake::default();
        run(r#"{"steps":[{"fire":{"secs":0.5,"track":true}}]}"#, 60, 1.0 / 60.0, &mut d);
        assert_eq!(d.log.first().map(String::as_str), Some("aim"), "tracking aims first");
        assert_eq!(d.log.iter().filter(|l| *l == "fire true").count(), 1);
        assert_eq!(d.log.last().map(String::as_str), Some("fire false"));
        assert!(d.log.iter().filter(|l| *l == "aim").count() > 20, "and again every frame");
    }

    #[test]
    fn an_expectation_passes_fails_or_waits_for_the_state_to_come_true() {
        let mut d = Fake { state: json!({"remote": {"drawn": 7, "names": ["a", "b"]}, "online": {"in_round": false}}), ..Fake::default() };
        let r = run(
            r#"{"steps":[{"expect":{"at":"/remote/drawn","eq":7}},{"expect":{"at":"/remote/drawn","min":5,"max":8}},{"expect":{"at":"/remote/names","contains":"b"}},
                {"expect":{"at":"/remote/nothing","exists":false}},{"expect":{"at":"/remote/drawn","eq":8,"msg":"eight fighters"}}]}"#,
            1,
            0.016,
            &mut d,
        );
        assert!(r.finished());
        assert_eq!(r.failures.len(), 1, "{:?}", r.failures);
        assert!(r.failures[0].starts_with("eight fighters: /remote/drawn is 7, expected 8"), "{}", r.failures[0]);
        // wait_for: the state changes during the wait.
        let mut d = Fake { state: json!({"online": {"in_round": false}}), ..Fake::default() };
        let mut r = Runner::new(Script::parse(r#"{"steps":[{"wait_for":{"at":"/online/in_round","eq":true,"within":2}},{"say":"started"}]}"#).unwrap());
        for frame in 0..200 {
            if frame == 60 {
                d.state = json!({"online": {"in_round": true}});
            }
            r.advance(1.0 / 60.0, &mut d);
        }
        assert!(r.finished() && r.failures.is_empty(), "{:?}", r.failures);
        assert_eq!(d.log, ["say started"], "the next step ran once the state came true");
        // ... and never does: a failure that says how long it waited.
        let mut d = Fake { state: json!({"online": {"in_round": false}}), ..Fake::default() };
        let r = run(r#"{"steps":[{"wait_for":{"at":"/online/in_round","eq":true,"within":1}}]}"#, 120, 1.0 / 60.0, &mut d);
        assert!(r.finished());
        assert!(r.failures[0].contains("after waiting 1 s"), "{:?}", r.failures);
    }

    #[test]
    fn a_shot_that_cannot_be_taken_is_a_failure_not_a_silence() {
        let mut d = Fake { shots_fail: true, ..Fake::default() };
        let r = run(r#"{"steps":[{"shot":"x","camera":"overview"}]}"#, 1, 0.016, &mut d);
        assert_eq!(r.failures, ["shot 'x': no GPU"]);
        assert_eq!(d.log, ["shot x Overview"]);
    }

    #[test]
    fn nominal_time_adds_up_the_timed_steps() {
        let s = Script::parse(r#"{"steps":[{"wait":2},{"turn":90,"over":1.5},{"hold":"forward","secs":3},{"fire":4},{"jump":true}]}"#).unwrap();
        assert!((s.nominal_secs() - (2.0 + 1.5 + 3.0 + 1.0)).abs() < 1e-5, "{}", s.nominal_secs());
    }
}
