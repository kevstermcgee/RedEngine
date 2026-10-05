//! The 2D simulation: a fixed 60 Hz step over a [`GameDef`], with no rendering, no clock, no files and no platform.
//!
//! The same code runs natively (headless playthroughs, `verify`, the native frame renderer) and inside the browser's WebAssembly module, so what a scripted playthrough proves is
//! what the browser plays. Everything that could differ between machines is kept out: time is a tick count, the random numbers are a seeded generator, angles use `libm`, and
//! the only float operations are the exactly-rounded ones (`+ - * /` and square root). [`Sim::state_hash`] summarises the whole state so two runs (native and browser) can be compared.
//!
//! A tick, in order: refresh the built-in variables; clicks and button keys; `press` rules; (unless the game has ended) movement and physics, particles, `touch` rules, timers; events
//! queued by rules (up to 16 rounds); removal of the destroyed; the camera; `end` rules; a requested restart. After the game ends only buttons, `press`, `click` and `event` rules
//! still run, so a game-over screen can offer a restart.

use crate::game::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Seconds per tick.
pub const DT: f32 = 1.0 / 60.0;
/// Ticks per second.
pub const TPS: u64 = 60;
const MAX_PARTICLES: usize = 2000;
const MAX_ENTITIES: usize = 4000;
const MAX_EVENT_ROUNDS: u32 = 16;
const EPS: f32 = 1e-3;

/// The index of an action name in [`ACTIONS`].
pub fn action_index(name: &str) -> Option<usize> {
    ACTIONS.iter().position(|a| *a == name)
}

/// The action a keyboard key code (`KeyboardEvent.code`) drives.
pub fn action_for_key(code: &str) -> Option<&'static str> {
    Some(match code {
        "ArrowLeft" | "KeyA" => "left",
        "ArrowRight" | "KeyD" => "right",
        "ArrowUp" | "KeyW" => "up",
        "ArrowDown" | "KeyS" => "down",
        "Space" | "KeyZ" | "KeyJ" => "action",
        "ShiftLeft" | "ShiftRight" | "KeyX" | "KeyK" => "secondary",
        "Escape" | "KeyP" => "pause",
        _ => return None,
    })
}

/// One living thing.
#[derive(Debug, Clone)]
pub struct Entity {
    /// Unique, increasing.
    pub id: u32,
    /// Index into [`GameDef::prefabs`].
    pub prefab: usize,
    /// The scene id, if it has one.
    pub scene_id: Option<String>,
    /// Centre x.
    pub x: f32,
    /// Centre y.
    pub y: f32,
    /// Velocity x, px/s.
    pub vx: f32,
    /// Velocity y, px/s.
    pub vy: f32,
    /// Seconds alive.
    pub age: f32,
    /// Standing on something solid this tick.
    pub grounded: bool,
    /// False once destroyed (removed at the end of the tick).
    pub alive: bool,
    home: [f32; 2],
    dir: f32,
    timer: f32,
    emit_acc: f32,
}

/// One particle.
#[derive(Debug, Clone, Copy)]
pub struct Particle {
    /// x.
    pub x: f32,
    /// y.
    pub y: f32,
    vx: f32,
    vy: f32,
    /// Seconds left.
    pub life: f32,
    /// Seconds at birth.
    pub max_life: f32,
    /// Colour.
    pub color: Color,
    /// Side, px.
    pub size: f32,
    gravity: f32,
}

#[derive(Debug, Clone, Default)]
struct RuleState {
    fired: u32,
    last: f64,
    next_tick: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Ctx {
    own: Option<u32>,
    other: Option<u32>,
}

/// What loading saved progress found.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveStatus {
    /// Nothing was saved yet.
    Fresh,
    /// This many values restored.
    Loaded(usize),
    /// A save from another game or a newer format: ignored (not erased).
    Incompatible(String),
    /// Not a save at all: ignored.
    Corrupt(String),
}

impl std::fmt::Display for SaveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveStatus::Fresh => write!(f, "no saved progress"),
            SaveStatus::Loaded(n) => write!(f, "restored {n} saved value(s)"),
            SaveStatus::Incompatible(why) => write!(f, "saved progress ignored (incompatible): {why}"),
            SaveStatus::Corrupt(why) => write!(f, "saved progress ignored (unreadable): {why}"),
        }
    }
}

/// The save format version.
pub const SAVE_VERSION: u64 = 1;

/// The 2D world.
#[derive(Debug, Clone)]
pub struct Sim {
    /// The game.
    pub def: Arc<GameDef>,
    /// Living things (and the destroyed until the end of the tick), ascending id.
    pub entities: Vec<Entity>,
    /// Particles.
    pub particles: Vec<Particle>,
    /// Variable values, in [`GameDef::var_names`] order.
    pub vars: Vec<f64>,
    /// Ticks since the sim was created (never reset).
    pub tick: u64,
    /// Ticks since the last (re)start.
    pub t_ticks: u64,
    /// How it ended.
    pub ended: Option<Outcome>,
    /// The camera's top-left in world px.
    pub cam: [f32; 2],
    /// The screen-shake offset this tick.
    pub shake_off: [f32; 2],
    /// Sound is on (the platform plays music only when this holds and audio is active).
    pub music_on: bool,
    /// Times each event was emitted, over the whole run.
    pub event_counts: BTreeMap<String, u32>,
    /// Times each sound was played (index as `GameDef::sounds`), over the whole run.
    pub sound_counts: Vec<u32>,
    /// Things that need the author's attention (an event storm).
    pub notes: Vec<String>,
    shake: f32,
    pointer: [f32; 2],
    held: [bool; 7],
    prev: [bool; 7],
    codes: Vec<String>,
    click: Option<[f32; 2]>,
    buttons: Vec<String>,
    rng: u64,
    next_id: u32,
    events: Vec<String>,
    audio: Vec<usize>,
    rule_state: Vec<RuleState>,
    touch_prev: BTreeSet<(usize, u32, u32)>,
    restart: bool,
    vars_dirty: bool,
    count_idx: Vec<(usize, usize)>,
    id_idx: Vec<(String, usize)>,
    collides: Vec<Vec<bool>>,
    saved_snapshot: Vec<f64>,
    save_dirty: bool,
}

fn rand_next(state: &mut u64) -> u64 {
    // xorshift64*
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

fn overlaps(ax: f32, ay: f32, aw: f32, ah: f32, bx: f32, by: f32, bw: f32, bh: f32, eps: f32) -> bool {
    (ax - bx).abs() < (aw + bw) * 0.5 - eps && (ay - by).abs() < (ah + bh) * 0.5 - eps
}

impl Sim {
    /// A fresh game with the random seed `seed`.
    pub fn new(def: Arc<GameDef>, seed: u64) -> Sim {
        let nd = def.vars.len();
        let count_idx = def.tags.iter().enumerate().filter_map(|(ti, t)| Some((ti, def.var_names.iter().position(|n| *n == format!("count_{t}"))?))).collect();
        let id_idx = def.var_names.iter().enumerate().skip(nd + BUILTINS.len() + def.tags.len()).filter_map(|(i, n)| n.strip_suffix("_x").map(|id| (id.to_string(), i))).collect();
        let collides = def.prefabs.iter().map(|a| def.prefabs.iter().map(|b| a.collide.iter().any(|t| b.tags.contains(t))).collect()).collect();
        let mut sim = Sim {
            vars: vec![0.0; def.var_names.len()],
            sound_counts: vec![0; def.sounds.len()],
            rule_state: vec![RuleState { fired: 0, last: f64::NEG_INFINITY, next_tick: 0 }; def.rules.len()],
            saved_snapshot: Vec::new(),
            def,
            entities: Vec::new(),
            particles: Vec::new(),
            tick: 0,
            t_ticks: 0,
            ended: None,
            cam: [0.0; 2],
            shake_off: [0.0; 2],
            music_on: true,
            event_counts: BTreeMap::new(),
            notes: Vec::new(),
            shake: 0.0,
            pointer: [0.0; 2],
            held: [false; 7],
            prev: [false; 7],
            codes: Vec::new(),
            click: None,
            buttons: Vec::new(),
            rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
            next_id: 0,
            events: Vec::new(),
            audio: Vec::new(),
            touch_prev: BTreeSet::new(),
            restart: false,
            vars_dirty: true,
            count_idx,
            id_idx,
            collides,
            save_dirty: false,
        };
        for (i, (_, v)) in sim.def.vars.clone().iter().enumerate() {
            sim.vars[i] = *v;
        }
        sim.begin();
        sim.saved_snapshot = sim.persisted_values();
        sim
    }

    /// Places the scene and fires the `start` rules.
    fn begin(&mut self) {
        let def = self.def.clone();
        self.entities.clear();
        self.particles.clear();
        self.touch_prev.clear();
        self.events.clear();
        self.ended = None;
        self.t_ticks = 0;
        self.restart = false;
        self.shake = 0.0;
        self.shake_off = [0.0; 2];
        for st in &mut self.rule_state {
            *st = RuleState { fired: 0, last: f64::NEG_INFINITY, next_tick: 0 };
        }
        for (ri, r) in def.rules.iter().enumerate() {
            match r.when {
                When::Every(s) => self.rule_state[ri].next_tick = (s * TPS as f32).ceil().max(1.0) as u64,
                When::After(s) => self.rule_state[ri].next_tick = (s * TPS as f32).ceil().max(1.0) as u64,
                _ => {}
            }
        }
        for p in &def.placements {
            self.spawn(&def, p.prefab, p.at[0], p.at[1], [0.0, 0.0], p.id.clone());
        }
        self.vars_dirty = true;
        self.refresh_vars();
        self.snap_camera(&def);
        for ri in 0..def.rules.len() {
            if def.rules[ri].when == When::Start {
                self.fire(&def, ri, Ctx::default());
            }
        }
        self.drain_events(&def);
        self.refresh_vars();
    }

    // ---- input ---------------------------------------------------------------------------------------------------------------------------------------

    /// Holds or releases an action by name (`left`, `action`, ...). `false` if there is no such action.
    pub fn set_action(&mut self, name: &str, down: bool) -> bool {
        match action_index(name) {
            Some(i) => {
                self.held[i] = down;
                true
            }
            None => false,
        }
    }

    /// A keyboard key (`KeyboardEvent.code`) goes down or up: drives its action and, on the down edge, any button bound to the code.
    pub fn key(&mut self, code: &str, down: bool) {
        if let Some(a) = action_for_key(code) {
            self.set_action(a, down);
        }
        if down {
            self.codes.push(code.to_string());
        }
    }

    /// The pointer is at `(x, y)` in virtual screen pixels.
    pub fn set_pointer(&mut self, x: f32, y: f32) {
        self.pointer = [x, y];
    }

    /// The pointer is pressed at `(x, y)` in virtual screen pixels (handled at the next tick).
    pub fn click(&mut self, x: f32, y: f32) {
        self.pointer = [x, y];
        self.click = Some([x, y]);
    }

    /// Presses the visible HUD button `id` at the next tick.
    pub fn press_button(&mut self, id: &str) -> Result<(), String> {
        let def = self.def.clone();
        let mut ids = Vec::new();
        for w in &def.ui {
            if let WidgetKind::Button { id: bid, .. } = &w.kind {
                if bid == id && self.widget_shown(w) {
                    self.buttons.push(id.to_string());
                    return Ok(());
                }
                ids.push(bid.clone());
            }
        }
        let shown: Vec<String> = def.ui.iter().filter(|w| self.widget_shown(w)).filter_map(|w| if let WidgetKind::Button { id, .. } = &w.kind { Some(id.clone()) } else { None }).collect();
        Err(format!("no visible button `{id}` (visible now: {}; all buttons: {})", if shown.is_empty() { "none".into() } else { shown.join(", ") }, if ids.is_empty() { "none".into() } else { ids.join(", ") }))
    }

    /// The pointer in world px.
    pub fn pointer_world(&self) -> [f32; 2] {
        [self.pointer[0] + self.cam[0], self.pointer[1] + self.cam[1]]
    }

    /// Whether an action is held now.
    pub fn held(&self, name: &str) -> bool {
        action_index(name).is_some_and(|i| self.held[i])
    }

    // ---- reading -------------------------------------------------------------------------------------------------------------------------------------

    /// A variable by name (declared, built-in, `count_<tag>`, `<id>_x`).
    pub fn var(&self, name: &str) -> Option<f64> {
        self.def.var_names.iter().position(|n| n == name).map(|i| self.vars[i])
    }

    /// Living things with a tag.
    pub fn count_tag(&self, tag: &str) -> usize {
        self.entities.iter().filter(|e| e.alive && self.def.prefabs[e.prefab].tags.iter().any(|t| t == tag)).count()
    }

    /// A living thing by scene id.
    pub fn by_id(&self, id: &str) -> Option<&Entity> {
        self.entities.iter().find(|e| e.alive && e.scene_id.as_deref() == Some(id))
    }

    /// The sounds played since the last call (indices into `GameDef::sounds`).
    pub fn take_sounds(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.audio)
    }

    /// Whether a HUD widget is visible now.
    pub fn widget_shown(&self, w: &Widget) -> bool {
        w.show.as_ref().is_none_or(|e| e.truthy(&self.vars))
    }

    /// A hash of the whole state: two runs that agree on it agree on everything the player can see.
    pub fn state_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut put = |v: u64| {
            for b in v.to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
            }
        };
        put(self.tick);
        put(self.t_ticks);
        put(match self.ended {
            None => 0,
            Some(Outcome::Win) => 1,
            Some(Outcome::Lose) => 2,
        });
        for v in &self.vars {
            put((v * 1e6).round() as i64 as u64);
        }
        for e in self.entities.iter().filter(|e| e.alive) {
            put(e.id as u64);
            put(e.prefab as u64);
            put(e.x.to_bits() as u64);
            put(e.y.to_bits() as u64);
            put(e.vx.to_bits() as u64);
            put(e.vy.to_bits() as u64);
        }
        put(self.particles.len() as u64);
        put(self.rng);
        h
    }

    /// The hash as 16 hex digits.
    pub fn hash_hex(&self) -> String {
        format!("{:016x}", self.state_hash())
    }

    /// A readable snapshot for tools and the browser: tick, outcome, variables, the scene ids' positions and tag counts.
    pub fn snapshot(&self) -> serde_json::Value {
        let mut vars = serde_json::Map::new();
        for (n, v) in self.def.var_names.iter().zip(&self.vars) {
            vars.insert(n.clone(), serde_json::json!(*v));
        }
        serde_json::json!({
            "tick": self.tick,
            "seconds": self.t_ticks as f64 / TPS as f64,
            "ended": self.ended.map(Outcome::name),
            "entities": self.entities.iter().filter(|e| e.alive).count(),
            "particles": self.particles.len(),
            "music_on": self.music_on,
            "hash": self.hash_hex(),
            "vars": vars,
        })
    }

    // ---- save ----------------------------------------------------------------------------------------------------------------------------------------

    fn persisted_values(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self.def.persist.iter().map(|&i| self.vars[i]).collect();
        v.push(f64::from(self.music_on));
        v
    }

    /// The save as JSON text: the persisted variables and the music setting.
    pub fn save_json(&self) -> String {
        let mut vars = serde_json::Map::new();
        for &i in &self.def.persist {
            vars.insert(self.def.var_names[i].clone(), serde_json::json!(self.vars[i]));
        }
        let mut obj = serde_json::json!({"red2d_save": SAVE_VERSION, "game": self.def.id, "vars": vars});
        if self.def.caps.persistence.contains(&crate::caps::Persistence::Settings) {
            obj["settings"] = serde_json::json!({"music": self.music_on});
        }
        obj.to_string()
    }

    /// The save, once, if something worth keeping changed since the last call (the platform writes it to storage).
    pub fn take_save_if_dirty(&mut self) -> Option<String> {
        if self.save_dirty {
            self.save_dirty = false;
            Some(self.save_json())
        } else {
            None
        }
    }

    /// Restores progress from saved text. Anything that does not fit this game is ignored and said so; the game always starts.
    pub fn load_save(&mut self, text: &str) -> SaveStatus {
        let v: serde_json::Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => return SaveStatus::Corrupt(format!("not JSON: {e}")),
        };
        let Some(o) = v.as_object() else { return SaveStatus::Corrupt("not an object".into()) };
        let Some(ver) = o.get("red2d_save").and_then(|x| x.as_u64()) else { return SaveStatus::Corrupt("no `red2d_save` version: not a RedEngine 2D save".into()) };
        if ver != SAVE_VERSION {
            return SaveStatus::Incompatible(format!("save format {ver}, this game reads {SAVE_VERSION}"));
        }
        if o.get("game").and_then(|x| x.as_str()) != Some(self.def.id.as_str()) {
            return SaveStatus::Incompatible(format!("saved by game `{}`, this is `{}`", o.get("game").and_then(|x| x.as_str()).unwrap_or("?"), self.def.id));
        }
        let mut n = 0;
        if let Some(vars) = o.get("vars").and_then(|x| x.as_object()) {
            for &i in &self.def.persist {
                if let Some(x) = vars.get(&self.def.var_names[i]).and_then(|x| x.as_f64()).filter(|x| x.is_finite()) {
                    self.vars[i] = x;
                    n += 1;
                }
            }
        }
        if let Some(m) = o.get("settings").and_then(|s| s.get("music")).and_then(|x| x.as_bool()) {
            if self.def.caps.persistence.contains(&crate::caps::Persistence::Settings) {
                self.music_on = m;
                n += 1;
            }
        }
        self.saved_snapshot = self.persisted_values();
        self.vars_dirty = true;
        self.refresh_vars();
        SaveStatus::Loaded(n)
    }

    // ---- the step ------------------------------------------------------------------------------------------------------------------------------------

    /// Advances one tick.
    pub fn step(&mut self) {
        let def = self.def.clone();
        self.refresh_vars();
        let mut pressed = [false; 7];
        for (i, p) in pressed.iter_mut().enumerate() {
            *p = self.held[i] && !self.prev[i];
        }
        self.prev = self.held;
        let codes = std::mem::take(&mut self.codes);
        let click = self.click.take();
        let buttons = std::mem::take(&mut self.buttons);

        // HUD buttons and clicks (these work after the end too)
        for id in &buttons {
            self.run_button(&def, |k| k == id);
        }
        for code in &codes {
            let code = code.as_str();
            self.run_button_key(&def, code);
        }
        if let Some(p) = click {
            self.handle_click(&def, p);
        }
        // press rules
        for (ai, down) in pressed.iter().enumerate() {
            if *down {
                for ri in 0..def.rules.len() {
                    if matches!(&def.rules[ri].when, When::Press(a) if a == ACTIONS[ai]) {
                        self.fire(&def, ri, Ctx::default());
                    }
                }
            }
        }
        self.drain_events(&def);

        if self.ended.is_none() {
            self.move_all(&def, &pressed);
            self.update_particles_and_emitters(&def);
            self.refresh_vars();
            self.touch_rules(&def);
            self.timer_rules(&def);
            self.drain_events(&def);
        }
        self.cleanup();
        self.refresh_vars();
        self.update_camera(&def);

        if self.shake > 0.0 {
            let (a, b) = (self.rand() * 2.0 - 1.0, self.rand() * 2.0 - 1.0);
            self.shake_off = [a * self.shake, b * self.shake];
            self.shake *= 0.9;
            if self.shake < 0.2 {
                self.shake = 0.0;
            }
        } else {
            self.shake_off = [0.0; 2];
        }

        if self.ended.is_some() {
            self.fire_end_rules(&def);
            self.drain_events(&def);
        }
        let restart = self.restart;
        if restart {
            // Progress and settings survive a restart; everything else starts over.
            let keep: Vec<(usize, f64)> = def.persist.iter().map(|&i| (i, self.vars[i])).collect();
            for (i, (_, v)) in def.vars.iter().enumerate() {
                self.vars[i] = *v;
            }
            for (i, v) in keep {
                self.vars[i] = v;
            }
            self.begin();
        } else {
            self.tick += 1;
            self.t_ticks += 1;
        }
        let snap = self.persisted_values();
        if snap != self.saved_snapshot {
            self.saved_snapshot = snap;
            if !def.persist.is_empty() || def.caps.persistence.contains(&crate::caps::Persistence::Settings) {
                self.save_dirty = true;
            }
        }
        if restart {
            self.tick += 1;
        }
    }

    /// Advances `n` ticks.
    pub fn run_ticks(&mut self, n: u64) {
        for _ in 0..n {
            self.step();
        }
    }

    fn rand(&mut self) -> f32 {
        (rand_next(&mut self.rng) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn rand_range(&mut self, lo: f32, hi: f32) -> f32 {
        if lo >= hi {
            lo
        } else {
            lo + self.rand() * (hi - lo)
        }
    }

    fn refresh_vars(&mut self) {
        let nd = self.def.vars.len();
        self.vars[nd] = self.t_ticks as f64 / TPS as f64;
        self.vars[nd + 1] = self.tick as f64;
        self.vars[nd + 2] = match self.ended {
            None => 0.0,
            Some(Outcome::Win) => 1.0,
            Some(Outcome::Lose) => 2.0,
        };
        let w = self.pointer_world();
        self.vars[nd + 3] = f64::from(w[0]);
        self.vars[nd + 4] = f64::from(w[1]);
        self.vars[nd + 5] = f64::from(self.music_on);
        {
            let mut counts = vec![0u32; self.def.tags.len()];
            for e in self.entities.iter().filter(|e| e.alive) {
                for t in &self.def.prefabs[e.prefab].tags {
                    if let Some(ti) = self.def.tags.iter().position(|x| x == t) {
                        counts[ti] += 1;
                    }
                }
            }
            for &(ti, vi) in &self.count_idx {
                self.vars[vi] = f64::from(counts[ti]);
            }
            for (id, vi) in &self.id_idx {
                if let Some(e) = self.entities.iter().find(|e| e.alive && e.scene_id.as_deref() == Some(id.as_str())) {
                    self.vars[*vi] = f64::from(e.x);
                    self.vars[*vi + 1] = f64::from(e.y);
                }
            }
        }
        self.vars_dirty = false;
    }

    fn ent(&self, id: u32) -> Option<usize> {
        self.entities.binary_search_by_key(&id, |e| e.id).ok()
    }

    fn pos(&self, id: Option<u32>) -> Option<(f32, f32)> {
        let i = self.ent(id?)?;
        Some((self.entities[i].x, self.entities[i].y))
    }

    fn spawn(&mut self, def: &GameDef, prefab: usize, x: f32, y: f32, v: [f32; 2], scene_id: Option<String>) -> Option<u32> {
        if self.entities.len() >= MAX_ENTITIES {
            let note = format!("more than {MAX_ENTITIES} things at once: a `spawn` is running away (spawns were dropped)");
            if !self.notes.contains(&note) {
                self.notes.push(note);
            }
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let _ = def;
        self.entities.push(Entity { id, prefab, scene_id, x, y, vx: v[0], vy: v[1], age: 0.0, grounded: false, alive: true, home: [x, y], dir: 1.0, timer: 0.0, emit_acc: 0.0 });
        self.vars_dirty = true;
        Some(id)
    }

    fn eval_coord(&mut self, c: &Coord) -> f32 {
        match c {
            Coord::At(v) => v.eval(&self.vars) as f32,
            Coord::Range(a, b) => self.rand_range(*a, *b),
        }
    }

    fn place(&mut self, p: &Place, ctx: Ctx) -> (f32, f32) {
        match p {
            Place::Xy(x, y) => (self.eval_coord(x), self.eval_coord(y)),
            Place::Own => self.pos(ctx.own).unwrap_or((0.0, 0.0)),
            Place::Other => self.pos(ctx.other).unwrap_or((0.0, 0.0)),
            Place::Pointer => {
                let w = self.pointer_world();
                (w[0], w[1])
            }
        }
    }

    fn targets(&self, t: &Target, ctx: Ctx) -> Vec<u32> {
        match t {
            Target::Own => ctx.own.into_iter().collect(),
            Target::Other => ctx.other.into_iter().collect(),
            Target::Tag(tag) => self.entities.iter().filter(|e| e.alive && self.def.prefabs[e.prefab].tags.contains(tag)).map(|e| e.id).collect(),
            Target::Id(id) => self.entities.iter().filter(|e| e.alive && e.scene_id.as_deref() == Some(id.as_str())).map(|e| e.id).collect(),
        }
    }

    // ---- rules ---------------------------------------------------------------------------------------------------------------------------------------

    fn fire(&mut self, def: &GameDef, ri: usize, ctx: Ctx) {
        let r = &def.rules[ri];
        if r.once && self.rule_state[ri].fired > 0 {
            return;
        }
        let now = self.t_ticks as f64 / TPS as f64;
        if r.cooldown > 0.0 && now - self.rule_state[ri].last < f64::from(r.cooldown) {
            return;
        }
        if self.vars_dirty {
            self.refresh_vars();
        }
        if let Some(c) = &r.cond {
            if !c.truthy(&self.vars) {
                return;
            }
        }
        self.rule_state[ri].fired += 1;
        self.rule_state[ri].last = now;
        self.run_actions(def, &r.actions, ctx);
    }

    fn run_actions(&mut self, def: &GameDef, acts: &[Act], ctx: Ctx) {
        for a in acts {
            match a {
                Act::Set(i, v) => {
                    let x = v.eval(&self.vars);
                    self.vars[*i] = if x.is_finite() { x } else { 0.0 };
                }
                Act::Add(i, v) => {
                    let x = v.eval(&self.vars);
                    if x.is_finite() {
                        self.vars[*i] += x;
                    }
                }
                Act::Emit(name) => {
                    *self.event_counts.entry(name.clone()).or_insert(0) += 1;
                    self.events.push(name.clone());
                }
                Act::Spawn { prefab, at, vel, count } => {
                    for _ in 0..*count {
                        let (x, y) = self.place(at, ctx);
                        let v = match vel {
                            Some([a, b]) => [self.eval_coord(a), self.eval_coord(b)],
                            None => [0.0, 0.0],
                        };
                        self.spawn(def, *prefab, x, y, v, None);
                    }
                }
                Act::Destroy(t) => {
                    for id in self.targets(t, ctx) {
                        if let Some(i) = self.ent(id) {
                            self.entities[i].alive = false;
                            self.vars_dirty = true;
                        }
                    }
                }
                Act::Play(s) => {
                    self.audio.push(*s);
                    self.sound_counts[*s] += 1;
                }
                Act::Music(m) => {
                    self.music_on = match m {
                        MusicCmd::On => true,
                        MusicCmd::Off => false,
                        MusicCmd::Toggle => !self.music_on,
                    };
                }
                Act::Burst { at, n, color, speed, life, size, gravity } => {
                    let (x, y) = self.place(at, ctx);
                    for _ in 0..*n {
                        let ang = self.rand() * 360.0;
                        let sp = self.rand_range(speed[0], speed[1]);
                        let lf = self.rand_range(life[0], life[1]).max(0.05);
                        self.add_particle(x, y, ang, sp, lf, *color, *size, *gravity);
                    }
                }
                Act::Shake(s) => self.shake = self.shake.max(*s),
                Act::End(o) => {
                    if self.ended.is_none() {
                        self.ended = Some(*o);
                    }
                }
                Act::Restart => self.restart = true,
                Act::ResetSave => {
                    for &i in &def.persist {
                        self.vars[i] = def.vars[i].1;
                    }
                    self.music_on = true;
                }
                Act::Velocity(t, v) => {
                    let (vx, vy) = (v[0].eval(&self.vars) as f32, v[1].eval(&self.vars) as f32);
                    for id in self.targets(t, ctx) {
                        if let Some(i) = self.ent(id) {
                            self.entities[i].vx = vx;
                            self.entities[i].vy = vy;
                        }
                    }
                }
                Act::Teleport(t, p) => {
                    let (x, y) = self.place(p, ctx);
                    for id in self.targets(t, ctx) {
                        if let Some(i) = self.ent(id) {
                            self.entities[i].x = x;
                            self.entities[i].y = y;
                            self.entities[i].home = [x, y];
                        }
                    }
                }
            }
        }
    }

    fn add_particle(&mut self, x: f32, y: f32, angle_deg: f32, speed: f32, life: f32, color: Color, size: f32, gravity: f32) {
        if self.particles.len() >= MAX_PARTICLES {
            return;
        }
        let a = angle_deg.to_radians();
        self.particles.push(Particle { x, y, vx: libm::cosf(a) * speed, vy: libm::sinf(a) * speed, life, max_life: life, color, size, gravity });
    }

    fn drain_events(&mut self, def: &GameDef) {
        let mut rounds = 0;
        while !self.events.is_empty() {
            if rounds >= MAX_EVENT_ROUNDS {
                let note = format!("events keep triggering events (more than {MAX_EVENT_ROUNDS} rounds in one tick; last: `{}`): a rule loop, the rest were dropped", self.events[0]);
                if !self.notes.contains(&note) {
                    self.notes.push(note);
                }
                self.events.clear();
                break;
            }
            rounds += 1;
            let evs = std::mem::take(&mut self.events);
            for ev in evs {
                for ri in 0..def.rules.len() {
                    if matches!(&def.rules[ri].when, When::Event(n) if *n == ev) {
                        self.fire(def, ri, Ctx::default());
                    }
                }
            }
        }
    }

    fn fire_end_rules(&mut self, def: &GameDef) {
        let Some(outcome) = self.ended else { return };
        for ri in 0..def.rules.len() {
            if let When::End(which) = &def.rules[ri].when {
                if which.is_none_or(|w| w == outcome) && self.rule_state[ri].fired == 0 {
                    self.fire(def, ri, Ctx::default());
                }
            }
        }
    }

    fn run_button(&mut self, def: &GameDef, matches_id: impl Fn(&str) -> bool) {
        for w in &def.ui {
            if let WidgetKind::Button { id, actions, .. } = &w.kind {
                if matches_id(id) && self.widget_shown(w) {
                    self.run_actions(def, actions, Ctx::default());
                }
            }
        }
    }

    fn run_button_key(&mut self, def: &GameDef, code: &str) {
        for w in &def.ui {
            if let WidgetKind::Button { key: Some(k), actions, .. } = &w.kind {
                if k == code && self.widget_shown(w) {
                    self.run_actions(def, actions, Ctx::default());
                }
            }
        }
    }

    fn handle_click(&mut self, def: &GameDef, p: [f32; 2]) {
        // HUD buttons first (screen coordinates), topmost = last declared.
        for w in def.ui.iter().rev() {
            if let WidgetKind::Button { at, size, actions, .. } = &w.kind {
                if self.widget_shown(w) && p[0] >= at[0] && p[0] < at[0] + size[0] && p[1] >= at[1] && p[1] < at[1] + size[1] {
                    self.run_actions(def, actions, Ctx::default());
                    return;
                }
            }
        }
        let wp = [p[0] + self.cam[0], p[1] + self.cam[1]];
        let clickable: Vec<&str> = def.rules.iter().filter_map(|r| if let When::Click(t) = &r.when { Some(t.as_str()) } else { None }).collect();
        let mut best: Option<(i32, u32)> = None;
        for e in self.entities.iter().filter(|e| e.alive) {
            let pf = &def.prefabs[e.prefab];
            if pf.hidden || !pf.tags.iter().any(|t| clickable.contains(&t.as_str())) {
                continue;
            }
            if (wp[0] - e.x).abs() <= pf.size[0] * 0.5 && (wp[1] - e.y).abs() <= pf.size[1] * 0.5 && best.is_none_or(|(l, id)| (pf.layer, e.id) > (l, id)) {
                best = Some((pf.layer, e.id));
            }
        }
        match best {
            Some((_, id)) => {
                let tags = def.prefabs[self.entities[self.ent(id).unwrap_or(0)].prefab].tags.clone();
                for ri in 0..def.rules.len() {
                    if matches!(&def.rules[ri].when, When::Click(t) if tags.contains(t)) {
                        self.fire(def, ri, Ctx { own: Some(id), other: None });
                    }
                }
            }
            None => {
                for ri in 0..def.rules.len() {
                    if matches!(&def.rules[ri].when, When::Click(t) if t == "*") {
                        self.fire(def, ri, Ctx::default());
                    }
                }
            }
        }
    }

    // ---- movement and physics -------------------------------------------------------------------------------------------------------------------------

    fn dir_input(&self) -> (f32, f32) {
        let h = |n: &str| if self.held(n) { 1.0 } else { 0.0 };
        (h("right") - h("left"), h("down") - h("up"))
    }

    fn move_all(&mut self, def: &GameDef, pressed: &[bool; 7]) {
        let n = self.entities.len();
        let world = def.view.world;
        for i in 0..n {
            if !self.entities[i].alive {
                continue;
            }
            self.entities[i].age += DT;
            let pi = self.entities[i].prefab;
            let p = &def.prefabs[pi];
            if matches!(p.body, Some(Body { kind: BodyKind::Static, .. })) {
                continue;
            }
            let (ex, ey) = (self.entities[i].x, self.entities[i].y);
            match &p.mv {
                Move::None => {}
                Move::Keys { mode, speed, jump } => {
                    let (dx, dy) = self.dir_input();
                    match mode {
                        KeyMode::TopDown => {
                            let k = if dx != 0.0 && dy != 0.0 { 0.707_106_77 } else { 1.0 };
                            self.entities[i].vx = dx * speed * k;
                            self.entities[i].vy = dy * speed * k;
                        }
                        KeyMode::Platformer => {
                            self.entities[i].vx = dx * speed;
                            let up = pressed[action_index("up").unwrap_or(2)] || pressed[action_index("action").unwrap_or(4)];
                            if up && self.entities[i].grounded {
                                self.entities[i].vy = -jump;
                            }
                        }
                    }
                }
                Move::Pointer { x, y } => {
                    let w = self.pointer_world();
                    let (hw, hh) = (p.size[0] * 0.5, p.size[1] * 0.5);
                    if *x {
                        self.entities[i].x = w[0].clamp(hw, (world.0 - hw).max(hw));
                    }
                    if *y {
                        self.entities[i].y = w[1].clamp(hh, (world.1 - hh).max(hh));
                    }
                    self.entities[i].vx = 0.0;
                    self.entities[i].vy = 0.0;
                }
                Move::Chase { target, speed } => {
                    let mut best: Option<(f32, f32, f32)> = None;
                    for e in self.entities.iter().filter(|e| e.alive && e.id != self.entities[i].id && def.prefabs[e.prefab].tags.contains(target)) {
                        let d2 = (e.x - ex) * (e.x - ex) + (e.y - ey) * (e.y - ey);
                        if best.is_none_or(|(b, _, _)| d2 < b) {
                            best = Some((d2, e.x, e.y));
                        }
                    }
                    match best {
                        Some((d2, tx, ty)) if d2 > 1.0 => {
                            let d = d2.sqrt();
                            self.entities[i].vx = (tx - ex) / d * speed;
                            self.entities[i].vy = (ty - ey) / d * speed;
                        }
                        _ => {
                            self.entities[i].vx = 0.0;
                            self.entities[i].vy = 0.0;
                        }
                    }
                }
                Move::Drift { v } => {
                    self.entities[i].vx = v[0];
                    self.entities[i].vy = v[1];
                }
                Move::Wander { speed, turn } => {
                    self.entities[i].timer -= DT;
                    if self.entities[i].timer <= 0.0 {
                        let a = (self.rand() * 360.0).to_radians();
                        self.entities[i].vx = libm::cosf(a) * speed;
                        self.entities[i].vy = libm::sinf(a) * speed;
                        self.entities[i].timer = *turn;
                    }
                }
                Move::Patrol { x, range, speed } => {
                    let home = self.entities[i].home;
                    let off = if *x { ex - home[0] } else { ey - home[1] };
                    if off > *range {
                        self.entities[i].dir = -1.0;
                    } else if off < -*range {
                        self.entities[i].dir = 1.0;
                    }
                    let v = self.entities[i].dir * speed;
                    if *x {
                        self.entities[i].vx = v;
                        self.entities[i].vy = 0.0;
                    } else {
                        self.entities[i].vx = 0.0;
                        self.entities[i].vy = v;
                    }
                }
            }
            // gravity, drag, friction
            if let Some(b) = p.body {
                let e = &mut self.entities[i];
                e.vy = (e.vy + b.gravity * DT).min(b.max_fall);
                if b.drag > 0.0 {
                    e.vx *= 1.0 - b.drag;
                    e.vy *= 1.0 - b.drag;
                }
                if b.friction > 0.0 && e.grounded && !matches!(p.mv, Move::Keys { .. }) {
                    e.vx *= 1.0 - b.friction;
                    if e.vx.abs() < 2.0 {
                        e.vx = 0.0;
                    }
                }
            }
            self.integrate(def, i);
            if p.clamp {
                let (hw, hh) = (p.size[0] * 0.5, p.size[1] * 0.5);
                let e = &mut self.entities[i];
                e.x = e.x.clamp(hw, (world.0 - hw).max(hw));
                e.y = e.y.clamp(hh, (world.1 - hh).max(hh));
            }
            if let Some(ttl) = p.ttl {
                if self.entities[i].age >= ttl {
                    self.entities[i].alive = false;
                }
            }
        }
        self.vars_dirty = true;
    }

    fn solid_hit(&self, i: usize, x: f32, y: f32) -> Option<usize> {
        let e = &self.entities[i];
        let p = &self.def.prefabs[e.prefab];
        self.entities.iter().enumerate().find_map(|(j, o)| {
            if j == i || !o.alive || !self.collides[e.prefab][o.prefab] {
                return None;
            }
            let q = &self.def.prefabs[o.prefab];
            overlaps(x, y, p.size[0], p.size[1], o.x, o.y, q.size[0], q.size[1], EPS).then_some(j)
        })
    }

    fn integrate(&mut self, def: &GameDef, i: usize) {
        let e = &self.entities[i];
        let p = &def.prefabs[e.prefab];
        let (mut x, mut y, mut vx, mut vy) = (e.x, e.y, e.vx, e.vy);
        let (dx, dy) = (vx * DT, vy * DT);
        let bounce = p.body.map_or(0.0, |b| b.bounce);
        let rebound = |v: f32| if bounce > 0.0 && v.abs() > 8.0 { -v * bounce } else { 0.0 };
        let mut grounded = false;
        if p.collide.is_empty() {
            x += dx;
            y += dy;
        } else {
            let steps = (dx.abs().max(dy.abs()) / 4.0).ceil().clamp(1.0, 16.0);
            let (sx, sy) = (dx / steps, dy / steps);
            for _ in 0..steps as i32 {
                if sx != 0.0 {
                    x += sx;
                    for _ in 0..4 {
                        let Some(j) = self.solid_hit(i, x, y) else { break };
                        let (o, q) = (&self.entities[j], &def.prefabs[self.entities[j].prefab]);
                        x = if sx > 0.0 { o.x - (q.size[0] + p.size[0]) * 0.5 } else { o.x + (q.size[0] + p.size[0]) * 0.5 };
                        vx = rebound(vx);
                    }
                }
                if sy != 0.0 {
                    y += sy;
                    for _ in 0..4 {
                        let Some(j) = self.solid_hit(i, x, y) else { break };
                        let (o, q) = (&self.entities[j], &def.prefabs[self.entities[j].prefab]);
                        if sy > 0.0 {
                            y = o.y - (q.size[1] + p.size[1]) * 0.5;
                            grounded = true;
                        } else {
                            y = o.y + (q.size[1] + p.size[1]) * 0.5;
                        }
                        vy = rebound(vy);
                    }
                }
                // A solid sat on the entity's starting spot (a mover pushed into it, or it was placed inside): nothing to resolve by axis.
            }
        }
        let e = &mut self.entities[i];
        e.x = x;
        e.y = y;
        e.vx = vx;
        e.vy = vy;
        e.grounded = grounded;
    }

    fn update_particles_and_emitters(&mut self, def: &GameDef) {
        for i in 0..self.entities.len() {
            if !self.entities[i].alive {
                continue;
            }
            if let Some(em) = def.prefabs[self.entities[i].prefab].emit {
                self.entities[i].emit_acc += em.rate * DT;
                while self.entities[i].emit_acc >= 1.0 {
                    self.entities[i].emit_acc -= 1.0;
                    let (x, y) = (self.entities[i].x, self.entities[i].y);
                    let ang = self.rand_range(em.angle[0], em.angle[1]);
                    let sp = self.rand_range(em.speed[0], em.speed[1]);
                    let lf = self.rand_range(em.life[0], em.life[1]).max(0.05);
                    self.add_particle(x, y, ang, sp, lf, em.color, em.size, em.gravity);
                }
            }
        }
        for p in &mut self.particles {
            p.x += p.vx * DT;
            p.y += p.vy * DT;
            p.vy += p.gravity * DT;
            p.life -= DT;
        }
        self.particles.retain(|p| p.life > 0.0);
    }

    fn pairs_for(&self, def: &GameDef, a_tag: &str, b_tag: &str) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for a in self.entities.iter().filter(|e| e.alive && def.prefabs[e.prefab].tags.iter().any(|t| t == a_tag)) {
            let pa = &def.prefabs[a.prefab];
            for b in self.entities.iter().filter(|e| e.alive && e.id != a.id && def.prefabs[e.prefab].tags.iter().any(|t| t == b_tag)) {
                if a_tag == b_tag && b.id < a.id {
                    continue;
                }
                let pb = &def.prefabs[b.prefab];
                if overlaps(a.x, a.y, pa.size[0], pa.size[1], b.x, b.y, pb.size[0], pb.size[1], 0.0) {
                    out.push((a.id, b.id));
                }
            }
        }
        out
    }

    fn touch_rules(&mut self, def: &GameDef) {
        let mut now: BTreeSet<(usize, u32, u32)> = BTreeSet::new();
        let mut todo: Vec<(usize, u32, u32)> = Vec::new();
        for (ri, r) in def.rules.iter().enumerate() {
            match &r.when {
                When::Touch(a, b) => {
                    for (x, y) in self.pairs_for(def, a, b) {
                        now.insert((ri, x, y));
                        if !self.touch_prev.contains(&(ri, x, y)) {
                            todo.push((ri, x, y));
                        }
                    }
                }
                When::Touching(a, b) => {
                    for (x, y) in self.pairs_for(def, a, b) {
                        todo.push((ri, x, y));
                    }
                }
                _ => {}
            }
        }
        self.touch_prev = now;
        for (ri, a, b) in todo {
            let alive = |s: &Sim, id: u32| s.ent(id).is_some_and(|i| s.entities[i].alive);
            if alive(self, a) && alive(self, b) {
                self.fire(def, ri, Ctx { own: Some(a), other: Some(b) });
            }
        }
    }

    fn timer_rules(&mut self, def: &GameDef) {
        let t = self.t_ticks + 1;
        for ri in 0..def.rules.len() {
            match def.rules[ri].when {
                When::Every(s) if t >= self.rule_state[ri].next_tick => {
                    self.rule_state[ri].next_tick += (s * TPS as f32).ceil().max(1.0) as u64;
                    self.fire(def, ri, Ctx::default());
                }
                When::After(_) if self.rule_state[ri].next_tick != u64::MAX && t >= self.rule_state[ri].next_tick => {
                    self.rule_state[ri].next_tick = u64::MAX;
                    self.fire(def, ri, Ctx::default());
                }
                _ => {}
            }
        }
    }

    fn cleanup(&mut self) {
        let before = self.entities.len();
        self.entities.retain(|e| e.alive);
        if self.entities.len() != before {
            self.vars_dirty = true;
        }
    }

    fn camera_target(&self, def: &GameDef) -> [f32; 2] {
        let (vw, vh) = (def.view.width as f32, def.view.height as f32);
        let mut c = [0.0, 0.0];
        if let Some(tag) = &def.view.follow {
            if let Some(e) = self.entities.iter().find(|e| e.alive && def.prefabs[e.prefab].tags.contains(tag)) {
                c = [e.x - vw * 0.5, e.y - vh * 0.5];
            }
        }
        [c[0].clamp(0.0, (def.view.world.0 - vw).max(0.0)), c[1].clamp(0.0, (def.view.world.1 - vh).max(0.0))]
    }

    fn snap_camera(&mut self, def: &GameDef) {
        self.cam = self.camera_target(def);
    }

    fn update_camera(&mut self, def: &GameDef) {
        let t = self.camera_target(def);
        let k = def.view.lerp;
        self.cam = [self.cam[0] + (t[0] - self.cam[0]) * k, self.cam[1] + (t[1] - self.cam[1]) * k];
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn game(extra_rules: &str, prefabs: &str, scene: &str) -> Arc<GameDef> {
        let text = format!(
            r##"{{"game2d":1,"id":"t","title":"T","description":"d",
            "capabilities":{{"presentation":"2d","platforms":["web"],"networking":"offline","input":["keyboard","mouse"],"persistence":["progress","settings"]}},
            "view":{{"width":160,"height":90}},
            "sounds":{{"beep":{{"seconds":0.1,"layers":[{{"sine":440,"decay":20}}]}}}},
            "vars":{{"score":0,"best":0}},"persist":["best"],
            "prefabs":{{{prefabs}}},"scene":[{scene}],"rules":[{extra_rules}]}}"##
        );
        Arc::new(parse(&text).unwrap_or_else(|e| panic!("{e:#?}\n{text}")))
    }

    const WALLS: &str = r##""wall":{"tag":"wall","shape":{"rect":[10,90],"color":"#888"},"body":{"type":"static"}},
        "floor":{"tag":"wall","shape":{"rect":[160,10],"color":"#888"},"body":{"type":"static"}},
        "ball":{"tag":"ball","shape":{"circle":4,"color":"#fff"},"body":{"type":"dynamic","gravity":300,"bounce":0.8},"collide":["wall"]},
        "player":{"tag":"player","shape":{"rect":[8,8],"color":"#fc0"},"body":{"type":"dynamic","gravity":600},"collide":["wall"],"move":{"keys":{"mode":"platformer","speed":60,"jump":220}}},
        "coin":{"tag":"coin","shape":{"circle":3,"color":"#ff0"}}"##;

    #[test]
    fn a_ball_falls_and_bounces_on_the_floor_and_never_tunnels() {
        let def = game("", WALLS, r#"{"prefab":"floor","at":[80,85]},{"prefab":"ball","at":[80,10],"id":"b"}"#);
        let mut s = Sim::new(def, 1);
        let mut lowest_vy = 0.0f32;
        for _ in 0..300 {
            s.step();
            let b = s.by_id("b").unwrap();
            assert!(b.y < 85.0, "tunnelled through the floor: y={}", b.y);
            lowest_vy = lowest_vy.max(b.vy);
        }
        assert!(lowest_vy > 100.0, "it fell");
        // It bounced: at some point it moved upward again.
        let mut s = Sim::new(game("", WALLS, r#"{"prefab":"floor","at":[80,85]},{"prefab":"ball","at":[80,10],"id":"b"}"#), 1);
        let mut went_up = false;
        let mut prev = 10.0;
        for _ in 0..200 {
            s.step();
            let y = s.by_id("b").unwrap().y;
            if y < prev - 0.5 && s.tick > 20 {
                went_up = true;
            }
            prev = y;
        }
        assert!(went_up);
    }

    #[test]
    fn a_fast_thing_does_not_pass_through_a_thin_wall() {
        let walls = r##""wall":{"tag":"wall","shape":{"rect":[2,90],"color":"#888"},"body":{"type":"static"}},
            "bullet":{"tag":"b","shape":{"rect":[2,2],"color":"#fff"},"body":{"type":"dynamic"},"collide":["wall"],"move":{"drift":[1800,0]}}"##;
        let mut s = Sim::new(game("", walls, r#"{"prefab":"wall","at":[100,45]},{"prefab":"bullet","at":[10,45],"id":"b"}"#), 1);
        s.run_ticks(30);
        assert!(s.by_id("b").unwrap().x < 100.0, "x={}", s.by_id("b").unwrap().x);
    }

    #[test]
    fn a_platformer_stands_runs_and_jumps_only_from_the_ground() {
        let def = game("", WALLS, r#"{"prefab":"floor","at":[80,85]},{"prefab":"player","at":[80,70],"id":"p"}"#);
        let mut s = Sim::new(def, 1);
        s.run_ticks(30);
        let ground = s.by_id("p").unwrap().y;
        assert!(s.by_id("p").unwrap().grounded);
        s.key("ArrowRight", true);
        s.run_ticks(30);
        assert!(s.by_id("p").unwrap().x > 100.0, "ran right");
        s.key("ArrowRight", false);
        s.key("Space", true);
        s.run_ticks(3);
        s.key("Space", false);
        assert!(s.by_id("p").unwrap().y < ground - 3.0, "jumped");
        // Pressing jump in mid-air does nothing extra.
        let vy = s.by_id("p").unwrap().vy;
        s.key("Space", true);
        s.step();
        assert!(s.by_id("p").unwrap().vy >= vy, "no double jump");
    }

    #[test]
    fn touch_fires_once_per_overlap_and_touching_every_tick() {
        let rules = r##"{"id":"pick","when":{"touch":["player","coin"]},"do":[{"add":["score",1]},{"destroy":"other"},{"play":"beep"}]}"##;
        let def = game(rules, WALLS, r#"{"prefab":"player","at":[20,40],"id":"p"},{"prefab":"coin","at":[22,40]},{"prefab":"coin","at":[150,40]}"#);
        let mut s = Sim::new(def, 1);
        s.run_ticks(5);
        assert_eq!(s.var("score"), Some(1.0));
        assert_eq!(s.count_tag("coin"), 1);
        assert_eq!(s.sound_counts[0], 1);
        assert_eq!(s.take_sounds(), vec![0]);
        assert_eq!(s.var("count_coin"), Some(1.0));
    }

    #[test]
    fn rules_cooldown_once_conditions_timers_and_events() {
        let rules = r##"
            {"id":"tick","when":{"every":0.5},"do":[{"add":["score",1]}]},
            {"id":"once","when":{"start":true},"once":true,"do":[{"add":["best",5]}]},
            {"id":"gate","when":{"every":0.5},"if":"score >= 3","do":[{"emit":"three"}]},
            {"id":"on three","when":{"event":"three"},"once":true,"do":[{"end":"win"}]}"##;
        let mut s = Sim::new(game(rules, WALLS, r#"{"prefab":"coin","at":[5,5]}"#), 1);
        assert_eq!(s.var("best"), Some(5.0));
        s.run_ticks(29);
        assert_eq!(s.var("score"), Some(0.0), "not yet at 0.5 s");
        s.run_ticks(2);
        assert_eq!(s.var("score"), Some(1.0));
        s.run_ticks(60 * 2);
        assert_eq!(s.ended, Some(Outcome::Win));
        assert_eq!(s.event_counts.get("three"), Some(&1));
        // After the end nothing advances: the score stays.
        let sc = s.var("score");
        s.run_ticks(120);
        assert_eq!(s.var("score"), sc);
    }

    #[test]
    fn restart_resets_the_game_but_keeps_saved_progress() {
        let rules = r##"{"when":{"press":"action"},"do":[{"add":["score",1]},{"add":["best",1]}]},{"when":{"press":"secondary"},"do":[{"restart":true}]}"##;
        let mut s = Sim::new(game(rules, WALLS, r#"{"prefab":"coin","at":[5,5]}"#), 1);
        s.key("Space", true);
        s.step();
        s.key("Space", false);
        s.step();
        assert_eq!((s.var("score"), s.var("best")), (Some(1.0), Some(1.0)));
        s.key("KeyX", true);
        s.step();
        assert_eq!((s.var("score"), s.var("best")), (Some(0.0), Some(1.0)), "score restarted, best kept");
        assert_eq!(s.var("time").map(|t| t < 0.1), Some(true));
    }

    #[test]
    fn a_save_is_written_only_when_something_worth_keeping_changed_and_restores() {
        let rules = r##"{"when":{"press":"action"},"do":[{"add":["best",3]}]},{"when":{"press":"pause"},"do":[{"music":"toggle"}]}"##;
        let def = game(rules, WALLS, r#"{"prefab":"coin","at":[5,5]}"#);
        let mut s = Sim::new(def.clone(), 1);
        s.run_ticks(10);
        assert!(s.take_save_if_dirty().is_none(), "nothing to keep yet");
        s.key("Space", true);
        s.step();
        let save = s.take_save_if_dirty().expect("best changed");
        assert!(s.take_save_if_dirty().is_none());
        assert!(save.contains("\"best\":3") && save.contains("\"game\":\"t\""), "{save}");
        s.key("Escape", true);
        s.step();
        let save2 = s.take_save_if_dirty().expect("music setting changed");
        assert!(save2.contains("\"music\":false"), "{save2}");
        // A new session restores both.
        let mut s2 = Sim::new(def.clone(), 1);
        assert_eq!(s2.load_save(&save2), SaveStatus::Loaded(2));
        assert_eq!(s2.var("best"), Some(3.0));
        assert!(!s2.music_on);
        // Other games', newer, corrupt and foreign saves are ignored without harm.
        let mut s3 = Sim::new(def, 1);
        assert!(matches!(s3.load_save(&save2.replace("\"game\":\"t\"", "\"game\":\"other\"")), SaveStatus::Incompatible(m) if m.contains("other")));
        assert!(matches!(s3.load_save(&save2.replace("\"red2d_save\":1", "\"red2d_save\":9")), SaveStatus::Incompatible(m) if m.contains("save format 9")));
        assert!(matches!(s3.load_save("not json"), SaveStatus::Corrupt(_)));
        assert!(matches!(s3.load_save("{}"), SaveStatus::Corrupt(m) if m.contains("red2d_save")));
        assert_eq!(s3.var("best"), Some(0.0));
    }

    #[test]
    fn the_same_seed_and_inputs_give_the_same_hash_and_another_seed_does_not() {
        let rules = r##"{"id":"rain","when":{"every":0.1},"do":[{"spawn":{"prefab":"coin","at":{"x":[0,150],"y":[0,80]}}}]}"##;
        let def = game(rules, WALLS, r#"{"prefab":"coin","at":[5,5]}"#);
        let run = |seed| {
            let mut s = Sim::new(def.clone(), seed);
            s.run_ticks(300);
            (s.state_hash(), s.count_tag("coin"))
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7).0, run(8).0);
        assert_eq!(run(7).1, 51);
    }

    #[test]
    fn a_rule_loop_is_cut_off_and_reported() {
        let rules = r##"{"when":{"start":true},"do":[{"emit":"a"}]},{"when":{"event":"a"},"do":[{"emit":"a"}]}"##;
        let s = Sim::new(game(rules, WALLS, r#"{"prefab":"coin","at":[5,5]}"#), 1);
        assert!(s.notes.iter().any(|n| n.contains("keep triggering events")), "{:?}", s.notes);
    }

    #[test]
    fn clicks_hit_the_topmost_tagged_thing_a_button_or_nothing() {
        let prefabs = r##""card":{"tag":"card","shape":{"rect":[20,20],"color":"#44f"},"layer":1},"back":{"tag":"card","shape":{"rect":[40,40],"color":"#222"}}"##;
        let rules = r##"{"when":{"click":"card"},"do":[{"add":["score",1]},{"destroy":"self"}]},{"when":{"click":"*"},"do":[{"add":["best",1]}]}"##;
        let text = format!(
            r##"{{"game2d":1,"id":"t","title":"T","description":"d","capabilities":{{"presentation":"2d","platforms":["web"],"networking":"offline","input":["mouse"],"persistence":["progress"]}},
            "view":{{"width":160,"height":90}},"vars":{{"score":0,"best":0}},"persist":["best"],"prefabs":{{{prefabs}}},
            "scene":[{{"prefab":"back","at":[50,50]}},{{"prefab":"card","at":[50,50]}}],
            "ui":[{{"button":{{"id":"go","label":"GO","at":[120,0],"size":[40,20],"do":[{{"add":["score",100]}}]}}}}],"rules":[{rules}]}}"##
        );
        let mut s = Sim::new(Arc::new(parse(&text).unwrap()), 1);
        s.click(50.0, 50.0);
        s.step();
        assert_eq!(s.count_tag("card"), 1, "only the top card was hit");
        s.click(50.0, 50.0);
        s.step();
        assert_eq!(s.count_tag("card"), 0);
        s.click(150.0, 10.0);
        s.step();
        assert_eq!(s.var("score"), Some(102.0), "the button, not a card");
        s.click(5.0, 85.0);
        s.step();
        assert_eq!(s.var("best"), Some(1.0), "a click on nothing");
        assert!(s.press_button("go").is_ok());
        assert!(s.press_button("gone").unwrap_err().contains("all buttons: go"));
    }

    #[test]
    fn patrol_chase_wander_and_pointer_movers_move() {
        let prefabs = r##""p":{"tag":"p","shape":{"rect":[6,6],"color":"#fff"},"move":{"pointer":"x"},"clamp":true},
            "g":{"tag":"g","shape":{"rect":[6,6],"color":"#f00"},"move":{"chase":{"target":"p","speed":30}}},
            "w":{"tag":"w","shape":{"rect":[6,6],"color":"#0f0"},"move":{"wander":{"speed":20,"turn":0.5}}},
            "r":{"tag":"r","shape":{"rect":[6,6],"color":"#00f"},"move":{"patrol":{"axis":"x","range":20,"speed":40}}}"##;
        let mut s = Sim::new(game("", prefabs, r#"{"prefab":"p","at":[80,80],"id":"p"},{"prefab":"g","at":[10,10],"id":"g"},{"prefab":"w","at":[80,40],"id":"w"},{"prefab":"r","at":[80,60],"id":"r"}"#), 3);
        s.set_pointer(120.0, 0.0);
        let g0 = s.by_id("g").unwrap().x;
        let (mut rmin, mut rmax) = (80.0f32, 80.0f32);
        for _ in 0..240 {
            s.step();
            let r = s.by_id("r").unwrap().x;
            rmin = rmin.min(r);
            rmax = rmax.max(r);
        }
        assert!((s.by_id("p").unwrap().x - 120.0).abs() < 0.01, "pointer mover");
        assert!(s.by_id("g").unwrap().x > g0 + 20.0, "chaser approaches");
        assert!(rmin < 62.0 && rmax > 98.0 && rmin > 55.0 && rmax < 105.0, "patrols +-20 about home: {rmin}..{rmax}");
        assert!((s.by_id("w").unwrap().x - 80.0).abs() + (s.by_id("w").unwrap().y - 40.0).abs() > 1.0, "wanders");
    }
}
