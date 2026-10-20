//! The 2D simulation: a fixed 60 Hz step over a [`GameDef`], with no rendering, no clock, no files and no platform.
//!
//! The same code runs headless (playthroughs, `verify`, the frame renderer) and in the native window (`play2d`, `re2d`), so what a scripted playthrough proves is
//! what the player plays. Everything that could differ between machines is kept out: time is a tick count, the random numbers are a seeded generator, angles use `libm`, and
//! the only float operations are the exactly-rounded ones (`+ - * /` and square root). [`Sim::state_hash`] summarises the whole state so two runs (on two machines) can be compared.
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
/// Where the built-ins after `music_on` sit in [`BUILTINS`] (a test keeps them in step).
/// How hard a glide catches a fall, px/s^2 (a fall faster than the glide slows to it over a few ticks).
const GLIDE_CATCH: f32 = 2400.0;
const B_LEVEL: usize = 6;
const B_HELD: usize = 7;
const B_SELF: usize = 13;
const B_OTHER: usize = 18;
const B_GAMEPAD: usize = 23;
const B_ACCEL: usize = 24;
/// How far outside the screen an `asleep` thing wakes, px.
const WAKE_MARGIN: f32 = 16.0;

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
/// Something a rule asked of the platform around the game (the window, the save file, a file for the player): the simulation only records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Borderless fullscreen on or off.
    Fullscreen,
    /// Close the game (after saving).
    Close,
    /// Write the saved variables now.
    Save,
    /// Export this music track (index into `GameDef::music`) for the player to keep.
    ExportMusic(usize),
}

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
    /// Last moved to the left (what `flip: "auto"` mirrors).
    pub face_left: bool,
    home: [f32; 2],
    dir: f32,
    timer: f32,
    emit_acc: f32,
    /// Where it was at the start of this tick's movement (which side a `touch` came from).
    prev: [f32; 2],
    /// Its velocity at the start of this tick's movement (`self_ax`, `self_ay`: how fast it changed).
    prev_v: [f32; 2],
    /// Seconds since it last stood on something (coyote time).
    air: f32,
    /// Seconds a pressed jump is still remembered.
    jump_buf: f32,
    /// Rising from its own jump (letting go of jump cuts it).
    jumping: bool,
    /// Seconds of glide used since it last stood on something.
    glided: f32,
    /// An `asleep` thing the camera has come near.
    awake: bool,
    /// Its sideways move was stopped by something solid this tick (a walker turns).
    blocked_x: bool,
    /// The moving platform it stands on (carried along with it).
    carrier: Option<u32>,
}

/// Where the things that never move by themselves are (tiles, walls, blocks), so a collision test looks at a few cells instead of every thing.
/// Built at the start of a tick's movement when something was added, removed, changed kind or was teleported; everything else (`loose`) is tested one by one.
#[derive(Debug, Clone, Default)]
struct SolidGrid {
    built: bool,
    origin: [f32; 2],
    nx: i32,
    ny: i32,
    cells: Vec<Vec<u32>>,
    loose: Vec<u32>,
}

const GRID_CELL: f32 = 32.0;

impl SolidGrid {
    fn cell_range(&self, x: f32, y: f32, w: f32, h: f32) -> (i32, i32, i32, i32) {
        let c = |v: f32, o: f32, n: i32| (((v - o) / GRID_CELL).floor() as i32).clamp(0, n - 1);
        (
            c(x - w * 0.5, self.origin[0], self.nx),
            c(x + w * 0.5, self.origin[0], self.nx),
            c(y - h * 0.5, self.origin[1], self.ny),
            c(y + h * 0.5, self.origin[1], self.ny),
        )
    }
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
    /// What the rules asked of the platform (`window`, `save`, `export_music`), oldest first, until the platform takes them.
    pub requests: Vec<Request>,
    /// A gamepad is connected (the platform says so; the `gamepad` variable): a game shows pad buttons instead of keys.
    pub gamepad: bool,
    /// The world's size now (the level's, in a game with `levels`).
    pub world: (f32, f32),
    /// The clear colour now (the level's, in a game with `levels`).
    pub background: Color,
    /// The level being played (index into `GameDef::levels`; 0 without levels).
    pub level: usize,
    /// The music track that plays (index into `GameDef::music`).
    pub music_track: usize,
    /// Solid contacts of this tick's movement: (mover, solid, side of the mover).
    hits: Vec<(u32, u32, u8)>,
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
    /// Each prefab's tags as indices into `GameDef::tags` (counting and contact tests compare numbers, not names).
    prefab_tags: Vec<Vec<usize>>,
    /// Per rule: the tag indices of a touch / touching / hit trigger.
    rule_tags: Vec<Option<(usize, usize)>>,
    grid: SolidGrid,
    grid_dirty: bool,
    /// Something was added, removed or changed kind since the tags were last counted.
    counts_dirty: bool,
    /// Indices of the things that have a scene id (their `<id>_x`, `<id>_y`), valid while `counts_dirty` is false.
    id_ents: Vec<usize>,
    saved_snapshot: Vec<f64>,
    save_dirty: bool,
}

/// Answers `random(a, b)` in a rule from the sim's own generator (the 2D game has no prop functions), so a draw is part of the seeded, replayable run.
struct Draws<'a>(&'a std::cell::Cell<u64>);

impl crate::rules_expr::World for Draws<'_> {
    fn call(&self, _: crate::rules_expr::Func, _: usize) -> f64 {
        0.0
    }
    fn random(&self, lo: f64, hi: f64) -> f64 {
        let mut s = self.0.get();
        let v = crate::rules_expr::draw_int(&mut s, lo, hi);
        self.0.set(s);
        v
    }
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
        let id_idx = def
            .var_names
            .iter()
            .enumerate()
            .skip(nd + BUILTINS.len() + def.tags.len())
            .filter_map(|(i, n)| n.strip_suffix("_x").map(|id| (id.to_string(), i)))
            .collect();
        let collides = def.prefabs.iter().map(|a| def.prefabs.iter().map(|b| a.collide.iter().any(|t| b.tags.contains(t))).collect()).collect();
        let (world, background) = (def.view.world, def.view.background);
        let tag_ix = |t: &str| def.tags.iter().position(|x| x == t);
        let prefab_tags = def.prefabs.iter().map(|p| p.tags.iter().filter_map(|t| tag_ix(t)).collect()).collect();
        let rule_tags = def
            .rules
            .iter()
            .map(|r| match &r.when {
                When::Touch(a, b) | When::Touching(a, b) | When::Hit(a, b) => Some((tag_ix(a)?, tag_ix(b)?)),
                _ => None,
            })
            .collect();
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
            requests: Vec::new(),
            gamepad: false,
            world,
            background,
            level: 0,
            music_track: 0,
            hits: Vec::new(),
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
            prefab_tags,
            rule_tags,
            grid: SolidGrid::default(),
            grid_dirty: true,
            counts_dirty: true,
            id_ents: Vec::new(),
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
        self.counts_dirty = true;
        self.grid_dirty = true;
        self.particles.clear();
        self.touch_prev.clear();
        self.events.clear();
        self.ended = None;
        self.t_ticks = 0;
        self.restart = false;
        self.shake = 0.0;
        self.shake_off = [0.0; 2];
        self.rule_state.fill(RuleState { fired: 0, last: f64::NEG_INFINITY, next_tick: 0 });
        for (ri, r) in def.rules.iter().enumerate() {
            match r.when {
                When::Every(s) => self.rule_state[ri].next_tick = (s * TPS as f32).ceil().max(1.0) as u64,
                When::After(s) => self.rule_state[ri].next_tick = (s * TPS as f32).ceil().max(1.0) as u64,
                _ => {}
            }
        }
        if def.levels.is_empty() {
            for p in &def.placements {
                self.spawn(&def, p.prefab, p.at[0], p.at[1], [0.0, 0.0], p.id.clone());
            }
        } else {
            self.music_track = 0;
            self.load_level(0);
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

    /// Replaces the world with level `i` (a game with `levels`): its things, world size, background and music. Variables are kept: they are the game's.
    /// Returns false (and changes nothing) when there is no such level.
    pub fn load_level(&mut self, i: usize) -> bool {
        let def = self.def.clone();
        let Some(lv) = def.levels.get(i) else { return false };
        self.entities.clear();
        self.counts_dirty = true;
        self.grid_dirty = true;
        self.particles.clear();
        self.touch_prev.clear();
        self.hits.clear();
        self.level = i;
        self.world = lv.world;
        self.background = lv.background;
        if let Some(m) = lv.music {
            self.music_track = m;
        }
        for p in &lv.placements {
            self.spawn(&def, p.prefab, p.at[0], p.at[1], [0.0, 0.0], p.id.clone());
        }
        self.cam = [0.0, 0.0];
        self.vars_dirty = true;
        self.refresh_vars();
        self.snap_camera(&def);
        true
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
        let shown: Vec<String> = def
            .ui
            .iter()
            .filter(|w| self.widget_shown(w))
            .filter_map(|w| if let WidgetKind::Button { id, .. } = &w.kind { Some(id.clone()) } else { None })
            .collect();
        Err(format!(
            "no visible button `{id}` (visible now: {}; all buttons: {})",
            if shown.is_empty() { "none".into() } else { shown.join(", ") },
            if ids.is_empty() { "none".into() } else { ids.join(", ") }
        ))
    }

    /// The pointer in world px (through the 3D camera in a `world3d` view).
    pub fn pointer_world(&self) -> [f32; 2] {
        self.world_from_screen(self.pointer)
    }

    /// The pointer on the virtual screen, in px.
    pub fn pointer_screen(&self) -> [f32; 2] {
        self.pointer
    }

    /// A point of the virtual screen as a point of the world: the camera offset in a flat view, the ray to the ground in a 3D one.
    pub fn world_from_screen(&self, p: [f32; 2]) -> [f32; 2] {
        if let Some(w) = &self.def.world3d {
            let (vw, vh) = (self.def.view.width as f32, self.def.view.height as f32);
            let center = [self.cam[0] + vw * 0.5, self.cam[1] + vh * 0.5];
            if let Some(q) = crate::game3d::world_point(w, &self.vars, center, self.def.view.width, self.def.view.height, p[0], p[1]) {
                return q;
            }
            return center;
        }
        [p[0] + self.cam[0], p[1] + self.cam[1]]
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
        let shown = w.show.as_ref().is_none_or(|e| e.truthy(&self.vars));
        match &w.kind {
            WidgetKind::Dialogue(d) => shown && self.dialogue_line(d).is_some(),
            _ => shown,
        }
    }

    /// The line a dialogue shows (its `<id>_line`), or None when it is closed (past the last line, or negative).
    pub fn dialogue_line(&self, d: &crate::game::Dialogue) -> Option<usize> {
        let v = self.vars.get(d.line_var).copied().unwrap_or(0.0);
        let i = v.floor();
        (i >= 0.0 && (i as usize) < d.lines.len()).then_some(i as usize)
    }

    /// The platform reports something back (`music_exported`, ...): the event fires for the game's rules at the next tick.
    pub fn notify(&mut self, name: &str) {
        self.emit_event(name);
    }

    fn emit_event(&mut self, name: &str) {
        *self.event_counts.entry(name.to_string()).or_insert(0) += 1;
        self.events.push(name.to_string());
    }

    /// The open dialogue takes its keys: up/down (or 1-9, or a click on a row) choose, the advance action confirms or moves on, a click in a box with no
    /// choices moves on. Returns true when it used the click. A choice's `emit` fires on top of its line's; closing emits `done`.
    fn dialogue_input(&mut self, d: &crate::game::Dialogue, pressed: &mut [bool; 7], codes: &[String], click: Option<[f32; 2]>) -> bool {
        use crate::game::Goto;
        let Some(li) = self.dialogue_line(d) else { return false };
        let line = &d.lines[li];
        let n = line.choices.len();
        let (up, down) = (action_index("up").unwrap_or(2), action_index("down").unwrap_or(3));
        let in_box = |p: [f32; 2], r: [f32; 4]| p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3];
        let took_click = click.is_some_and(|p| in_box(p, [d.at[0], d.at[1], d.size[0], d.size[1]]));
        // None: nothing chosen; Some(None): move on past a line without choices; Some(Some(i)): choice i.
        let mut confirm: Option<Option<usize>> = None;
        if n > 0 {
            let mut c = (self.vars[d.choice_var].max(0.0) as usize).min(n - 1);
            if pressed[up] {
                c = (c + n - 1) % n;
            }
            if pressed[down] {
                c = (c + 1) % n;
            }
            for code in codes {
                if let Some(k) = code.strip_prefix("Digit").and_then(|s| s.parse::<usize>().ok()).filter(|k| (1..=n).contains(k)) {
                    c = k - 1;
                    confirm = Some(Some(c));
                }
            }
            if let Some(p) = click {
                if let Some(i) = (0..n).find(|i| in_box(p, d.choice_rect(*i, n))) {
                    c = i;
                    confirm = Some(Some(i));
                }
            }
            if pressed[d.advance] {
                confirm = Some(Some(c));
            }
            self.vars[d.choice_var] = c as f64;
            pressed[up] = false;
            pressed[down] = false;
        } else if pressed[d.advance] || took_click {
            confirm = Some(None);
        }
        pressed[d.advance] = false;
        if let Some(choice) = confirm {
            if let Some(e) = &line.emit {
                self.emit_event(e);
            }
            let goto = match choice {
                Some(i) => {
                    if let Some(e) = &line.choices[i].emit {
                        self.emit_event(e);
                    }
                    line.choices[i].goto.or(line.goto)
                }
                None => line.goto,
            };
            let next = match goto {
                Some(Goto::Line(i)) => i,
                Some(Goto::End) => d.lines.len(),
                None => li + 1,
            };
            self.vars[d.line_var] = next as f64;
            self.vars[d.choice_var] = 0.0;
            if next >= d.lines.len() {
                if let Some(e) = &d.done {
                    self.emit_event(e);
                }
            }
        }
        took_click
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
            put(u64::from(e.face_left));
        }
        put(self.particles.len() as u64);
        put(self.rng);
        if !self.def.levels.is_empty() {
            put(self.level as u64);
            put(self.music_track as u64);
        }
        h
    }

    /// The hash as 16 hex digits.
    pub fn hash_hex(&self) -> String {
        format!("{:016x}", self.state_hash())
    }

    /// A readable snapshot for tools: tick, outcome, variables, the scene ids' positions and tag counts.
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
        let status = self.read_save(text);
        if matches!(status, SaveStatus::Loaded(n) if n > 0) {
            // `start` rules ran before the save was read, so a world that depends on saved progress (a gate already opened, a level reached) is rebuilt by a rule that reacts to this:
            // the built-in event `loaded`, fired once right now, so the very first frame already shows the restored world.
            let def = self.def.clone();
            self.events.push("loaded".to_string());
            self.drain_events(&def);
            self.cleanup();
            self.refresh_vars();
            self.save_dirty = false;
            self.saved_snapshot = self.persisted_values();
        }
        status
    }

    /// Reads saved text into the persisted variables and the music setting, without the game's `loaded` rules: the save format alone (what `verify`'s
    /// save round trip checks, since a `loaded` rule may fairly correct a value, such as an out-of-range choice).
    pub(crate) fn read_save(&mut self, text: &str) -> SaveStatus {
        let v: serde_json::Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => return SaveStatus::Corrupt(format!("not JSON: {e}")),
        };
        let Some(o) = v.as_object() else { return SaveStatus::Corrupt("not an object".into()) };
        let Some(ver) = o.get("red2d_save").and_then(|x| x.as_u64()) else {
            return SaveStatus::Corrupt("no `red2d_save` version: not a RedEngine 2D save".into());
        };
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
        let mut click = self.click.take();
        let buttons = std::mem::take(&mut self.buttons);

        // An open dialogue owns its keys (the first open one in `ui`); with `pause` the world stands still and no press rule fires until it closes.
        // `pause_if` stands the world still the same way but leaves the keys to the game's press rules (a pause menu made of rules).
        let mut paused = def.pause_if.as_ref().is_some_and(|e| self.expr_value(e) != 0.0);
        if self.ended.is_none() {
            if let Some(w) = def.ui.iter().find(|w| matches!(w.kind, WidgetKind::Dialogue(_)) && self.widget_shown(w)) {
                if let WidgetKind::Dialogue(d) = &w.kind {
                    if self.dialogue_input(d, &mut pressed, &codes, click) {
                        click = None;
                    }
                    if d.pause {
                        paused = true;
                        pressed = [false; 7];
                    }
                }
            }
        }

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
        // press rules: their `self` is the thing the keys move (the player), when there is one
        for (ai, down) in pressed.iter().enumerate() {
            if *down {
                for ri in 0..def.rules.len() {
                    if matches!(&def.rules[ri].when, When::Press(a) if a == ACTIONS[ai]) {
                        let own = self.entities.iter().find(|e| e.alive && matches!(def.prefabs[e.prefab].mv, Move::Keys { .. })).map(|e| e.id);
                        self.fire(&def, ri, Ctx { own, other: None });
                    }
                }
            }
        }
        self.drain_events(&def);

        if self.ended.is_none() && !paused {
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
            // The game's clock (`time`, `every`, `after`, cooldowns) stands still with the world: a long pause does not end in a burst of overdue timers.
            if !paused {
                self.t_ticks += 1;
            }
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
        self.vars[nd + B_LEVEL] = self.level as f64;
        for k in 0..6 {
            self.vars[nd + B_HELD + k] = f64::from(self.held[k]);
        }
        self.vars[nd + B_GAMEPAD] = f64::from(self.gamepad);
        if self.counts_dirty {
            let mut counts = vec![0u32; self.def.tags.len()];
            for e in self.entities.iter().filter(|e| e.alive) {
                for &ti in &self.prefab_tags[e.prefab] {
                    counts[ti] += 1;
                }
            }
            for &(ti, vi) in &self.count_idx {
                self.vars[vi] = f64::from(counts[ti]);
            }
            self.id_ents = (0..self.entities.len()).filter(|&i| self.entities[i].scene_id.is_some()).collect();
            self.counts_dirty = false;
        }
        {
            // The first living thing with each scene id (most things have none: tiles of a map).
            let mut set = vec![false; self.id_idx.len()];
            for &i in &self.id_ents {
                let e = &self.entities[i];
                if !e.alive {
                    continue;
                }
                let Some(sid) = e.scene_id.as_deref() else { continue };
                if let Some(k) = self.id_idx.iter().position(|(id, _)| id == sid) {
                    if !set[k] {
                        set[k] = true;
                        let vi = self.id_idx[k].1;
                        self.vars[vi] = f64::from(e.x);
                        self.vars[vi + 1] = f64::from(e.y);
                    }
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
        self.grid_dirty = true;
        self.counts_dirty = true;
        self.entities.push(Entity {
            id,
            prefab,
            scene_id,
            x,
            y,
            vx: v[0],
            vy: v[1],
            age: 0.0,
            grounded: false,
            alive: true,
            face_left: false,
            home: [x, y],
            dir: if let Move::Walk { dir, .. } = def.prefabs[prefab].mv { dir } else { 1.0 },
            timer: 0.0,
            emit_acc: 0.0,
            prev: [x, y],
            prev_v: v,
            air: 0.0,
            jump_buf: 0.0,
            jumping: false,
            glided: 0.0,
            awake: false,
            blocked_x: false,
            carrier: None,
        });
        self.vars_dirty = true;
        Some(id)
    }

    /// An expression's value in a rule or action: `random(a, b)` draws from this sim's generator (a widget's `show` reads without drawing).
    fn value(&mut self, v: &Val) -> f64 {
        let state = std::cell::Cell::new(self.rng);
        let x = v.eval_in(&self.vars, &Draws(&state));
        self.rng = state.get();
        x
    }

    /// [`value`](Self::value) for a rule's condition (a bare expression).
    fn expr_value(&mut self, e: &crate::rules_expr::Expr) -> f64 {
        let state = std::cell::Cell::new(self.rng);
        let x = e.eval_in(&self.vars, &Draws(&state));
        self.rng = state.get();
        x
    }

    fn eval_coord(&mut self, c: &Coord) -> f32 {
        match c {
            Coord::At(v) => self.value(v) as f32,
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

    /// Rule ids with the number of times each fired this run, in file order (a failed expectation uses it to say which rules never ran).
    pub fn rule_fires(&self) -> Vec<(String, u32)> {
        self.def.rules.iter().zip(&self.rule_state).map(|(r, st)| (r.id.clone(), st.fired)).collect()
    }

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
        self.set_ctx_vars(ctx);
        if let Some(c) = &r.cond {
            if self.expr_value(c) == 0.0 {
                return;
            }
        }
        self.rule_state[ri].fired += 1;
        self.rule_state[ri].last = now;
        self.run_actions(def, &r.actions, ctx);
    }

    /// `self_*` and `other_*`: the rule's two things (0 when it has none).
    fn set_ctx_vars(&mut self, ctx: Ctx) {
        let nd = self.def.vars.len();
        for (base, id) in [(B_SELF, ctx.own), (B_OTHER, ctx.other)] {
            let e = id.and_then(|id| self.ent(id)).map(|i| &self.entities[i]);
            let v = e.map_or([0.0; 5], |e| [e.x, e.y, e.vx, e.vy, if e.face_left { -1.0 } else { 1.0 }]);
            for (k, x) in v.iter().enumerate() {
                self.vars[nd + base + k] = f64::from(*x);
            }
        }
        // `self_ax`, `self_ay`: the change in the rule's own thing's velocity over its last movement, per second ("a sudden move").
        let own = ctx.own.and_then(|id| self.ent(id)).map(|i| &self.entities[i]);
        let a = own.map_or([0.0; 2], |e| [(e.vx - e.prev_v[0]) * TPS as f32, (e.vy - e.prev_v[1]) * TPS as f32]);
        self.vars[nd + B_ACCEL] = f64::from(a[0]);
        self.vars[nd + B_ACCEL + 1] = f64::from(a[1]);
    }

    fn run_actions(&mut self, def: &GameDef, acts: &[Act], ctx: Ctx) {
        for a in acts {
            match a {
                Act::Set(i, v) => {
                    let x = self.value(v);
                    self.vars[*i] = if x.is_finite() { x } else { 0.0 };
                }
                Act::Add(i, v) => {
                    let x = self.value(v);
                    if x.is_finite() {
                        self.vars[*i] += x;
                    }
                }
                Act::Emit(name) => self.emit_event(name),
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
                            self.counts_dirty = true;
                        }
                    }
                }
                Act::Play(s) => {
                    self.audio.push(*s);
                    self.sound_counts[*s] += 1;
                }
                Act::Music(MusicCmd::Track(t)) => self.music_track = *t,
                Act::Music(MusicCmd::Level) => {
                    if let Some(m) = def.levels.get(self.level).and_then(|l| l.music) {
                        self.music_track = m;
                    }
                }
                Act::Music(m) => {
                    self.music_on = match m {
                        MusicCmd::On => true,
                        MusicCmd::Off => false,
                        _ => !self.music_on,
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
                Act::Window(cmd) => self.requests.push(match cmd {
                    crate::game::WindowCmd::Fullscreen => Request::Fullscreen,
                    crate::game::WindowCmd::Close => Request::Close,
                }),
                Act::Save => self.requests.push(Request::Save),
                Act::ExportMusic => self.requests.push(Request::ExportMusic(self.music_track)),
                Act::ResetSave => {
                    for &i in &def.persist {
                        self.vars[i] = def.vars[i].1;
                    }
                    self.music_on = true;
                }
                Act::Velocity(t, v) => {
                    let (vx, vy) = (self.value(&v[0]) as f32, self.value(&v[1]) as f32);
                    for id in self.targets(t, ctx) {
                        if let Some(i) = self.ent(id) {
                            let e = &mut self.entities[i];
                            e.vx = vx;
                            e.vy = vy;
                            // A walker keeps going the way it was pushed; a jumper that is thrown is no longer rising from its own jump.
                            if matches!(def.prefabs[e.prefab].mv, Move::Walk { .. }) && vx != 0.0 {
                                e.dir = vx.signum();
                            }
                            e.jumping = false;
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
                    // later actions of this list (and events drained right after it) read `<id>_x`/`<id>_y`: they must see the new place
                    self.refresh_vars();
                    self.grid_dirty = true;
                }
                Act::Become(t, prefab) => {
                    for id in self.targets(t, ctx) {
                        if let Some(i) = self.ent(id) {
                            let e = &mut self.entities[i];
                            // The feet stay where they are: a thing that grows grows upwards.
                            e.y += (def.prefabs[e.prefab].size[1] - def.prefabs[*prefab].size[1]) * 0.5;
                            e.prefab = *prefab;
                            e.age = 0.0;
                            e.home = [e.x, e.y];
                            e.awake = true;
                            e.timer = 0.0;
                        }
                    }
                    self.vars_dirty = true;
                    self.grid_dirty = true;
                    self.counts_dirty = true;
                }
                Act::Level(v) => {
                    let x = self.value(v);
                    let ok = x.is_finite() && x >= 0.0 && self.load_level(x.floor() as usize);
                    if !ok {
                        let note = format!(
                            "`level` asked for level {x}, and the game has {} (0 to {}): nothing happened",
                            def.levels.len(),
                            def.levels.len().saturating_sub(1)
                        );
                        if !self.notes.contains(&note) {
                            self.notes.push(note);
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
                let note = format!(
                    "events keep triggering events (more than {MAX_EVENT_ROUNDS} rounds in one tick; last: `{}`): a rule loop, the rest were dropped",
                    self.events[0]
                );
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
        let wp = self.world_from_screen(p);
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

    /// Puts every thing that cannot move by itself (no dynamic body, no mover) into the grid; the rest is `loose`.
    fn build_grid(&mut self, def: &GameDef) {
        let pad = 256.0;
        let origin = [-pad, -pad];
        let (nx, ny) = (((self.world.0 + 2.0 * pad) / GRID_CELL).ceil() as i32 + 1, ((self.world.1 + 2.0 * pad) / GRID_CELL).ceil() as i32 + 1);
        let mut g = SolidGrid { built: true, origin, nx, ny, cells: vec![Vec::new(); (nx * ny) as usize], loose: Vec::new() };
        for (i, e) in self.entities.iter().enumerate() {
            let p = &def.prefabs[e.prefab];
            let fixed = matches!(p.mv, Move::None) && !matches!(p.body, Some(Body { kind: BodyKind::Dynamic, .. }));
            if fixed {
                let (x0, x1, y0, y1) = g.cell_range(e.x, e.y, p.size[0], p.size[1]);
                for cy in y0..=y1 {
                    for cx in x0..=x1 {
                        g.cells[(cy * nx + cx) as usize].push(i as u32);
                    }
                }
            } else {
                g.loose.push(i as u32);
            }
        }
        self.grid = g;
        self.grid_dirty = false;
    }

    fn move_all(&mut self, def: &GameDef, pressed: &[bool; 7]) {
        if self.grid_dirty || !self.grid.built {
            self.build_grid(def);
        }
        self.hits.clear();
        for e in &mut self.entities {
            e.prev = [e.x, e.y];
            e.prev_v = [e.vx, e.vy];
        }
        // Things standing on a moving platform move after everything else, so they ride where the platform went this tick.
        let mut riders = Vec::new();
        for i in 0..self.entities.len() {
            if self.entities[i].carrier.is_some() {
                riders.push(i);
            } else {
                self.move_one(def, i, pressed);
            }
        }
        for i in riders {
            self.carry(def, i);
            self.move_one(def, i, pressed);
        }
        self.vars_dirty = true;
    }

    fn move_one(&mut self, def: &GameDef, i: usize, pressed: &[bool; 7]) {
        let world = self.world;
        if !self.entities[i].alive {
            return;
        }
        let pi = self.entities[i].prefab;
        let p = &def.prefabs[pi];
        if p.asleep && !self.entities[i].awake {
            let (vw, vh) = (def.view.width as f32, def.view.height as f32);
            let e = &self.entities[i];
            let (hw, hh) = (p.size[0] * 0.5 + WAKE_MARGIN, p.size[1] * 0.5 + WAKE_MARGIN);
            if e.x + hw < self.cam[0] || e.x - hw > self.cam[0] + vw || e.y + hh < self.cam[1] || e.y - hh > self.cam[1] + vh {
                return;
            }
            self.entities[i].awake = true;
        }
        self.entities[i].age += DT;
        if matches!(p.body, Some(Body { kind: BodyKind::Static, .. })) {
            return;
        }
        let (ex, ey) = (self.entities[i].x, self.entities[i].y);
        match &p.mv {
            Move::None => {}
            Move::Keys { mode, speed, jump, feel } => {
                let (dx, dy) = self.dir_input();
                match mode {
                    KeyMode::TopDown => {
                        let k = if dx != 0.0 && dy != 0.0 { 0.707_106_77 } else { 1.0 };
                        self.entities[i].vx = dx * speed * k;
                        self.entities[i].vy = dy * speed * k;
                    }
                    KeyMode::Platformer => {
                        let running = feel.run > *speed && self.held("secondary");
                        let top = if running { feel.run } else { *speed };
                        let jump_held = self.held("up") || self.held("action");
                        let up = pressed[action_index("up").unwrap_or(2)] || pressed[action_index("action").unwrap_or(4)];
                        let e = &mut self.entities[i];
                        if feel.accel <= 0.0 {
                            e.vx = dx * top;
                        } else {
                            let want = dx * top;
                            // Turning against the motion skids (twice the grip); with no direction held it slows at `decel`.
                            let a = if dx == 0.0 {
                                feel.decel
                            } else if e.vx != 0.0 && e.vx.signum() != dx {
                                feel.accel * 2.0
                            } else if e.vx.abs() > top {
                                feel.decel
                            } else {
                                feel.accel
                            };
                            let step = a * DT;
                            e.vx = if (want - e.vx).abs() <= step { want } else { e.vx + step * (want - e.vx).signum() };
                        }
                        e.air = if e.grounded { 0.0 } else { e.air + DT };
                        if up {
                            e.jump_buf = feel.buffer.max(DT * 0.5);
                        }
                        let can = e.grounded || (feel.coyote > 0.0 && e.air <= feel.coyote && !e.jumping);
                        if e.jump_buf > 0.0 && can {
                            let fast = if feel.run > *speed { (e.vx.abs() / feel.run).min(1.0) } else { 0.0 };
                            e.vy = -(jump + feel.run_jump * fast);
                            e.jump_buf = 0.0;
                            e.jumping = true;
                            e.air = feel.coyote + 1.0;
                        }
                        e.jump_buf = (e.jump_buf - DT).max(0.0);
                        if e.jumping && e.vy < 0.0 && !jump_held && feel.cut < 1.0 {
                            e.vy *= feel.cut;
                            e.jumping = false;
                        }
                        if e.vy >= 0.0 {
                            e.jumping = false;
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
            Move::Walk { speed, .. } => {
                self.entities[i].vx = self.entities[i].dir * speed;
            }
            Move::Orbit { radius, speed, phase } => {
                let e = &mut self.entities[i];
                let a = (phase + speed * e.age).to_radians();
                let (nx, ny) = (e.home[0] + libm::cosf(a) * radius, e.home[1] + libm::sinf(a) * radius);
                // Its velocity is what it moved, so a thing standing on it rides along.
                e.vx = (nx - e.x) / DT;
                e.vy = (ny - e.y) / DT;
                e.x = nx;
                e.y = ny;
                return;
            }
        }
        // gravity, drag, friction
        if let Some(b) = p.body {
            // A platformer's glide: holding jump while falling caps the fall (eased in, so it catches rather than stops), for `glide_time` per time in the air.
            let glide = match &p.mv {
                Move::Keys { mode: KeyMode::Platformer, feel, .. } if feel.glide > 0.0 => Some((feel.glide, feel.glide_time)),
                _ => None,
            };
            let jump_held = glide.is_some() && (self.held("up") || self.held("action"));
            let e = &mut self.entities[i];
            e.vy = (e.vy + b.gravity * DT).min(b.max_fall);
            if let Some((cap, limit)) = glide {
                if e.grounded {
                    e.glided = 0.0;
                } else if jump_held && e.vy > cap && (limit <= 0.0 || e.glided < limit) {
                    e.vy = (e.vy - GLIDE_CATCH * DT).max(cap);
                    e.glided += DT;
                }
            }
            if b.drag > 0.0 {
                e.vx *= 1.0 - b.drag;
                e.vy *= 1.0 - b.drag;
            }
            if b.friction > 0.0 && e.grounded && !matches!(p.mv, Move::Keys { .. } | Move::Walk { .. }) {
                e.vx *= 1.0 - b.friction;
                if e.vx.abs() < 2.0 {
                    e.vx = 0.0;
                }
            }
        }
        self.integrate(def, i);
        if let Move::Walk { turn_at_edges, .. } = p.mv {
            let e = &self.entities[i];
            // Turn at a wall; with `edges: "turn"`, also where the ground ends one body-width ahead.
            let ledge = turn_at_edges && e.grounded && self.solid_hit(i, e.x + e.dir * p.size[0], e.y + 1.0).is_none();
            if e.blocked_x || ledge {
                self.entities[i].dir = -self.entities[i].dir;
            }
        }
        if p.clamp {
            let (hw, hh) = (p.size[0] * 0.5, p.size[1] * 0.5);
            let e = &mut self.entities[i];
            e.x = e.x.clamp(hw, (world.0 - hw).max(hw));
            e.y = e.y.clamp(hh, (world.1 - hh).max(hh));
        }
        if def.view.forward_only && def.view.follow.as_ref().is_some_and(|f| p.tags.contains(f)) {
            let e = &mut self.entities[i];
            let left = self.cam[0] + p.size[0] * 0.5;
            if e.x < left {
                e.x = left;
                e.vx = e.vx.max(0.0);
            }
        }
        if let Some(ttl) = p.ttl {
            if self.entities[i].age >= ttl {
                self.entities[i].alive = false;
                self.counts_dirty = true;
            }
        }
    }

    /// A thing standing on a moving platform moves with it first (the platform has already moved this tick), stopped by solids like any move.
    fn carry(&mut self, def: &GameDef, i: usize) {
        let Some(c) = self.entities[i].carrier.and_then(|id| self.ent(id)).filter(|&c| self.entities[c].alive) else {
            self.entities[i].carrier = None;
            return;
        };
        let (dx, dy) = (self.entities[c].vx * DT, self.entities[c].vy * DT);
        let p = &def.prefabs[self.entities[i].prefab];
        let (mut x, mut y) = (self.entities[i].x, self.entities[i].y);
        for (d, horizontal) in [(dx, true), (dy, false)] {
            if d == 0.0 {
                continue;
            }
            if horizontal {
                x += d;
            } else {
                y += d;
            }
            for _ in 0..4 {
                let Some(j) = self.solid_hit_except(i, x, y, c) else { break };
                let (o, q) = (&self.entities[j], &def.prefabs[self.entities[j].prefab]);
                if horizontal {
                    x = if d > 0.0 { o.x - (q.size[0] + p.size[0]) * 0.5 } else { o.x + (q.size[0] + p.size[0]) * 0.5 };
                } else {
                    y = if d > 0.0 { o.y - (q.size[1] + p.size[1]) * 0.5 } else { o.y + (q.size[1] + p.size[1]) * 0.5 };
                }
            }
        }
        self.entities[i].x = x;
        self.entities[i].y = y;
    }

    /// Map analysis: can the thing with scene id `from` walk to touch `to` (touch = overlap, or within one 1 px cell) (a scene id, or `tag:NAME` for any living thing with that tag), as the world stands *now*?
    ///
    /// It uses the game's own collision (`solid_hit`, the call a real step makes) on a grid of 1 px cells over the world, from where `from` is. Only a top-down walker has a reach:
    /// a platformer's depends on jumps, which a scenario plays instead. Whatever is solid at this moment blocks: ask it of a sim after a rule opened the gate and it says what
    /// the player can do now; ask it of a fresh sim with the gate removed ([`Sim::remove_for_analysis`]) and it says what the gate was keeping out. Both must agree.
    pub fn can_reach(&self, from: &str, to: &str) -> Result<bool, String> {
        let (w, h) = self.world;
        // 1 px cells (a gap barely wider than the walker must not vanish between grid lines), coarser only when the world is huge: at most about 300 000 cells.
        let cell: f32 = (w * h / 300_000.0).sqrt().max(1.0);
        let (nx, ny) = (((w / cell).ceil() as i64).max(1), ((h / cell).ceil() as i64).max(1));
        let i =
            self.entities.iter().position(|e| e.alive && e.scene_id.as_deref() == Some(from)).ok_or_else(|| format!("`{from}` does not exist (destroyed?)"))?;
        let p = &self.def.prefabs[self.entities[i].prefab];
        if !matches!(p.mv, crate::game::Move::Keys { mode: crate::game::KeyMode::TopDown, .. }) {
            return Err(format!("`{from}` is not a top-down walker (`move.keys` with mode topdown): reach analysis cannot tell where a jumper or a drifting thing can get; use a scenario"));
        }
        let at = |cx: i64, cy: i64| ((cx as f32 + 0.5) * cell, (cy as f32 + 0.5) * cell);
        let start = ((self.entities[i].x / cell).floor() as i64, (self.entities[i].y / cell).floor() as i64);
        let (sx, sy) = (start.0.clamp(0, nx - 1), start.1.clamp(0, ny - 1));
        let mut seen = vec![false; (nx * ny) as usize];
        let mut queue = std::collections::VecDeque::from([(sx, sy)]);
        seen[(sy * nx + sx) as usize] = true;
        // The walker starts where it stands (even if the grid centre of that cell is a hair inside something); every other cell must be free.
        let mut cells = vec![(self.entities[i].x, self.entities[i].y)];
        while let Some((cx, cy)) = queue.pop_front() {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (ax, ay) = (cx + dx, cy + dy);
                if ax < 0 || ay < 0 || ax >= nx || ay >= ny || seen[(ay * nx + ax) as usize] {
                    continue;
                }
                let (x, y) = at(ax, ay);
                if self.solid_hit(i, x, y).is_some() {
                    continue;
                }
                seen[(ay * nx + ax) as usize] = true;
                cells.push((x, y));
                queue.push_back((ax, ay));
            }
        }
        let wanted = |e: &Entity| {
            e.alive
                && e.id != self.entities[i].id
                && match to.strip_prefix("tag:") {
                    Some(t) => self.def.prefabs[e.prefab].tags.iter().any(|x| x == t),
                    None => e.scene_id.as_deref() == Some(to),
                }
        };
        let targets: Vec<&Entity> = self.entities.iter().filter(|e| wanted(e)).collect();
        if targets.is_empty() {
            return Err(format!("there is nothing called `{to}` in the world now"));
        }
        Ok(targets.iter().any(|t| {
            let q = &self.def.prefabs[t.prefab];
            // "Touch": overlapping, or standing within one cell of it, so a solid thing (a gate, a wall) can be walked up to.
            cells.iter().any(|&(x, y)| overlaps(x, y, p.size[0], p.size[1], t.x, t.y, q.size[0], q.size[1], -cell))
        }))
    }

    /// Takes things out of the world for an analysis: `selector` is a scene id or `tag:NAME`. Returns how many were removed. (A gate a rule would open, assumed open.)
    pub fn remove_for_analysis(&mut self, selector: &str) -> usize {
        let mut n = 0;
        for k in 0..self.entities.len() {
            let hit = match selector.strip_prefix("tag:") {
                Some(t) => self.def.prefabs[self.entities[k].prefab].tags.iter().any(|x| x == t),
                None => self.entities[k].scene_id.as_deref() == Some(selector),
            };
            if hit && self.entities[k].alive {
                self.entities[k].alive = false;
                self.counts_dirty = true;
                n += 1;
            }
        }
        n
    }

    fn solid_hit(&self, i: usize, x: f32, y: f32) -> Option<usize> {
        self.solid_hit_except(i, x, y, usize::MAX)
    }

    /// [`solid_hit`](Self::solid_hit), ignoring the thing at index `skip`. With a current grid it looks only at the cells the box covers and the
    /// loose things, and answers the lowest index that overlaps: the same answer as testing every thing in order.
    fn solid_hit_except(&self, i: usize, x: f32, y: f32, skip: usize) -> Option<usize> {
        if !self.grid.built || self.grid_dirty {
            return self.solid_hit_scan(i, x, y, skip);
        }
        let e = &self.entities[i];
        let p = &self.def.prefabs[e.prefab];
        let mut best: Option<usize> = None;
        let mut test = |j: usize| {
            if j == i || j == skip || best.is_some_and(|b| b <= j) {
                return;
            }
            let o = &self.entities[j];
            if !o.alive || !self.collides[e.prefab][o.prefab] {
                return;
            }
            let q = &self.def.prefabs[o.prefab];
            if overlaps(x, y, p.size[0], p.size[1], o.x, o.y, q.size[0], q.size[1], EPS) {
                best = Some(j);
            }
        };
        let (x0, x1, y0, y1) = self.grid.cell_range(x, y, p.size[0], p.size[1]);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                for &j in &self.grid.cells[(cy * self.grid.nx + cx) as usize] {
                    test(j as usize);
                }
            }
        }
        for &j in &self.grid.loose {
            test(j as usize);
        }
        best
    }

    /// Every thing in order (no grid): map analysis between ticks, and before the first movement.
    fn solid_hit_scan(&self, i: usize, x: f32, y: f32, skip: usize) -> Option<usize> {
        let e = &self.entities[i];
        let p = &self.def.prefabs[e.prefab];
        self.entities.iter().enumerate().find_map(|(j, o)| {
            if j == i || j == skip || !o.alive || !self.collides[e.prefab][o.prefab] {
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
        let mut blocked_x = false;
        let mut carrier = None;
        let mut hits: Vec<(u32, u32, u8)> = Vec::new();
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
                        blocked_x = true;
                        hits.push((e.id, o.id, if sx > 0.0 { SIDE_RIGHT } else { SIDE_LEFT }));
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
                            // A platform that moves itself carries what stands on it.
                            if q.body.is_some_and(|b| b.kind == BodyKind::Dynamic)
                                && matches!(q.mv, Move::Patrol { .. } | Move::Drift { .. } | Move::Orbit { .. })
                            {
                                carrier = Some(o.id);
                            }
                        } else {
                            y = o.y + (q.size[1] + p.size[1]) * 0.5;
                        }
                        hits.push((e.id, o.id, if sy > 0.0 { SIDE_BOTTOM } else { SIDE_TOP }));
                        vy = rebound(vy);
                    }
                }
                // A solid sat on the entity's starting spot (a mover pushed into it, or it was placed inside): nothing to resolve by axis.
            }
        }
        self.hits.extend(hits);
        let e = &mut self.entities[i];
        e.x = x;
        e.y = y;
        e.vx = vx;
        e.vy = vy;
        e.grounded = grounded;
        e.blocked_x = blocked_x;
        e.carrier = carrier;
        if vx < -1.0 {
            e.face_left = true;
        } else if vx > 1.0 {
            e.face_left = false;
        }
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

    /// The living things of each tag a contact rule asks about, sorted by x, with the widest half-width among them: built once per tick.
    fn contact_index(&self, def: &GameDef) -> Vec<Option<(Vec<usize>, f32)>> {
        let mut by_tag: Vec<Option<(Vec<usize>, f32)>> = vec![None; def.tags.len()];
        for (ri, tags) in self.rule_tags.iter().enumerate() {
            if let (Some((a, b)), When::Touch(..) | When::Touching(..)) = (tags, &def.rules[ri].when) {
                by_tag[*a] = Some((Vec::new(), 0.0));
                by_tag[*b] = Some((Vec::new(), 0.0));
            }
        }
        for (i, e) in self.entities.iter().enumerate().filter(|(_, e)| e.alive) {
            for &t in &self.prefab_tags[e.prefab] {
                if let Some((list, half)) = &mut by_tag[t] {
                    list.push(i);
                    *half = half.max(def.prefabs[e.prefab].size[0] * 0.5);
                }
            }
        }
        for (list, _) in by_tag.iter_mut().flatten() {
            // A map's things come row by row, so a tag on one row is already in order.
            if !list.is_sorted_by(|&i, &j| self.entities[i].x <= self.entities[j].x) {
                list.sort_unstable_by(|&i, &j| self.entities[i].x.total_cmp(&self.entities[j].x).then(i.cmp(&j)));
            }
        }
        by_tag
    }

    /// Overlapping pairs (a with tag `ta`, b with tag `tb`), in id order: the same pairs, in the same order, as testing every thing against every
    /// other, found with a sweep along x.
    fn pairs_for(&self, def: &GameDef, index: &[Option<(Vec<usize>, f32)>], ta: usize, tb: usize) -> Vec<(u32, u32)> {
        let (Some((alist, _)), Some((blist, bhalf))) = (&index[ta], &index[tb]) else { return Vec::new() };
        let mut out = Vec::new();
        for &ia in alist {
            let a = &self.entities[ia];
            let pa = &def.prefabs[a.prefab];
            let reach = pa.size[0] * 0.5 + bhalf;
            let start = blist.partition_point(|&j| self.entities[j].x < a.x - reach);
            for &ib in &blist[start..] {
                let b = &self.entities[ib];
                if b.x > a.x + reach {
                    break;
                }
                if b.id == a.id || (ta == tb && b.id < a.id) {
                    continue;
                }
                let pb = &def.prefabs[b.prefab];
                if overlaps(a.x, a.y, pa.size[0], pa.size[1], b.x, b.y, pb.size[0], pb.size[1], 0.0) {
                    out.push((a.id, b.id));
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// The side of `a` through which it met `b` this tick, judged from where both were before they moved: above it is `bottom` (landed on it),
    /// below it is `top`, else `left`/`right`. Coming in diagonally from above counts as from above (a stomp), from below as from the side.
    fn side_of(&self, a: u32, b: u32) -> u8 {
        let (Some(ia), Some(ib)) = (self.ent(a), self.ent(b)) else { return 0 };
        let (ea, eb) = (&self.entities[ia], &self.entities[ib]);
        let (pa, pb) = (&self.def.prefabs[ea.prefab], &self.def.prefabs[eb.prefab]);
        let (dx, dy) = (eb.prev[0] - ea.prev[0], eb.prev[1] - ea.prev[1]);
        let (hw, hh) = ((pa.size[0] + pb.size[0]) * 0.5, (pa.size[1] + pb.size[1]) * 0.5);
        let apart_y = dy.abs() >= hh - EPS;
        let apart_x = dx.abs() >= hw - EPS;
        if apart_y && (dy > 0.0 || !apart_x) {
            return if dy > 0.0 { SIDE_BOTTOM } else { SIDE_TOP };
        }
        if apart_x {
            return if dx > 0.0 { SIDE_RIGHT } else { SIDE_LEFT };
        }
        // Already overlapping before: the shallower overlap now says which way they met.
        let (nx, ny) = (eb.x - ea.x, eb.y - ea.y);
        if hw - nx.abs() < hh - ny.abs() {
            if nx > 0.0 {
                SIDE_RIGHT
            } else {
                SIDE_LEFT
            }
        } else if ny > 0.0 {
            SIDE_BOTTOM
        } else {
            SIDE_TOP
        }
    }

    fn touch_rules(&mut self, def: &GameDef) {
        let mut now: BTreeSet<(usize, u32, u32)> = BTreeSet::new();
        let mut todo: Vec<(usize, u32, u32)> = Vec::new();
        let tagged = |s: &Sim, id: u32, t: usize| s.ent(id).is_some_and(|i| s.prefab_tags[s.entities[i].prefab].contains(&t));
        let index = self.contact_index(def);
        for (ri, r) in def.rules.iter().enumerate() {
            let Some((a, b)) = self.rule_tags[ri] else { continue };
            match &r.when {
                When::Touch(..) => {
                    for (x, y) in self.pairs_for(def, &index, a, b) {
                        if r.sides != 0 && self.side_of(x, y) & r.sides == 0 {
                            continue;
                        }
                        now.insert((ri, x, y));
                        if !self.touch_prev.contains(&(ri, x, y)) {
                            todo.push((ri, x, y));
                        }
                    }
                }
                When::Touching(..) => {
                    for (x, y) in self.pairs_for(def, &index, a, b) {
                        if r.sides == 0 || self.side_of(x, y) & r.sides != 0 {
                            todo.push((ri, x, y));
                        }
                    }
                }
                When::Hit(..) => {
                    for &(x, y, side) in &self.hits {
                        if (r.sides == 0 || side & r.sides != 0)
                            && tagged(self, x, a)
                            && tagged(self, y, b)
                            && now.insert((ri, x, y))
                            && !self.touch_prev.contains(&(ri, x, y))
                        {
                            todo.push((ri, x, y));
                        }
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
            self.grid_dirty = true;
            self.counts_dirty = true;
        }
    }

    fn camera_target(&self, def: &GameDef) -> [f32; 2] {
        let (vw, vh) = (def.view.width as f32, def.view.height as f32);
        let mut c = [0.0, 0.0];
        if let Some(tag) = &def.view.follow {
            match self.entities.iter().find(|e| e.alive && def.prefabs[e.prefab].tags.contains(tag)) {
                Some(e) => c = [e.x - vw * 0.5, e.y - vh * 0.5],
                // Nothing to follow (the player is gone, or changed into something else for a moment): the camera stays.
                None => return self.cam,
            }
        }
        if def.view.forward_only {
            c[0] = c[0].max(self.cam[0]);
        }
        if let Some(e) = &def.view.min_x {
            c[0] = c[0].max(e.eval(&self.vars) as f32);
        }
        if let Some(e) = &def.view.max_x {
            c[0] = c[0].min(e.eval(&self.vars) as f32);
        }
        if def.world3d.is_some() {
            // The 3D camera looks at the middle of the view and sees a different part of the world than the flat screen would: only its focus has to stay inside the world.
            return [c[0].clamp(-vw * 0.5, self.world.0 - vw * 0.5), c[1].clamp(-vh * 0.5, self.world.1 - vh * 0.5)];
        }
        [c[0].clamp(0.0, (self.world.0 - vw).max(0.0)), c[1].clamp(0.0, (self.world.1 - vh).max(0.0))]
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
            "capabilities":{{"presentation":"2d","platforms":["windows","linux"],"networking":"offline","input":["keyboard","mouse"],"persistence":["progress","settings"]}},
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
    fn a_teleport_is_visible_to_the_actions_and_events_after_it() {
        // `score` copies p_x after each teleport in one action list; the event rule reads p_y after the teleport that emitted it.
        let rules = r##"{"when":{"press":"action"},"do":[
            {"teleport":{"target":"id:p","to":[100,45]}},{"set":["score","p_x"]},
            {"teleport":{"target":"id:p","to":[120,30]}},{"add":["score","p_x"]},
            {"emit":"moved"}]},
            {"when":{"event":"moved"},"do":[{"set":["best","p_y"]}]}"##;
        let mut s = Sim::new(game(rules, WALLS, r#"{"prefab":"coin","at":[5,5],"id":"p"}"#), 1);
        s.key("Space", true);
        s.step();
        assert_eq!(s.var("score"), Some(220.0), "100 then 120, not the old 5");
        assert_eq!(s.var("best"), Some(30.0));
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
        let prefabs =
            r##""card":{"tag":"card","shape":{"rect":[20,20],"color":"#44f"},"layer":1},"back":{"tag":"card","shape":{"rect":[40,40],"color":"#222"}}"##;
        let rules = r##"{"when":{"click":"card"},"do":[{"add":["score",1]},{"destroy":"self"}]},{"when":{"click":"*"},"do":[{"add":["best",1]}]}"##;
        let text = format!(
            r##"{{"game2d":1,"id":"t","title":"T","description":"d","capabilities":{{"presentation":"2d","platforms":["windows","linux"],"networking":"offline","input":["mouse"],"persistence":["progress"]}},
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
    fn reach_analysis_stops_at_a_wall_follows_a_gap_and_sees_a_removed_gate() {
        let def = game(
            "",
            r##""wall":{"tag":"wall","shape":{"rect":[8,8],"color":"#888"},"body":{"type":"static"}},
                "gate":{"tag":["wall","gate"],"shape":{"rect":[8,8],"color":"#a60"},"body":{"type":"static"}},
                "goal":{"tag":"goal","shape":{"circle":3,"color":"#ff0"}},
                "player":{"tag":"player","shape":{"rect":[6,6],"color":"#fc0"},"size":[6,6],"clamp":true,"body":{"type":"dynamic"},"collide":["wall"],"move":{"keys":{"mode":"topdown","speed":80}}}"##,
            // a wall at x = 60 from y = 0..56, a gate in its gap at y = 60, open floor below y = 68? no: a full wall, so only the gate is a way through
            &format!(
                "{{\"prefab\":\"player\",\"at\":[20,40],\"id\":\"p\"}},{{\"prefab\":\"goal\",\"at\":[100,40],\"id\":\"goal\"}},{{\"prefab\":\"gate\",\"at\":[60,44],\"id\":\"gate\"}},{}",
                (0..12).filter(|k| *k != 5).map(|k| format!("{{\"prefab\":\"wall\",\"at\":[60,{}]}}", 4 + k * 8)).collect::<Vec<_>>().join(",")
            ),
        );
        let mut sim = Sim::new(def, 1);
        assert_eq!(sim.can_reach("p", "goal"), Ok(false), "a wall with a shut gate");
        assert_eq!(sim.can_reach("p", "gate"), Ok(true), "the walker can walk up to the gate");
        assert!(sim.can_reach("goal", "p").is_err(), "a goal is not a walker");
        assert!(sim.can_reach("p", "tag:nothing").is_err());
        assert_eq!(sim.remove_for_analysis("gate"), 1);
        assert_eq!(sim.can_reach("p", "goal"), Ok(true), "the gate gone, the way is open");
        assert_eq!(sim.remove_for_analysis("tag:wall"), 11, "a tag removes every thing that has it");
    }

    #[test]
    fn loading_saved_progress_fires_the_loaded_event_once_and_an_empty_save_does_not() {
        let text = r##"{"game2d":1,"id":"t","title":"T","description":"d",
          "capabilities":{"presentation":"2d","platforms":["windows","linux"],"networking":"offline","input":["keyboard"],"persistence":["progress"]},
          "view":{"width":160,"height":90},"vars":{"opened":0},"persist":["opened"],
          "prefabs":{"door":{"tag":"door","shape":{"rect":[8,8],"color":"#a60"},"body":{"type":"static"}}},
          "scene":[{"prefab":"door","at":[20,20],"id":"door"}],
          "rules":[{"id":"rebuild","when":{"event":"loaded"},"if":"opened == 1","do":[{"destroy":"id:door"}]}]}"##;
        let def = Arc::new(crate::game::parse(text).unwrap_or_else(|e| panic!("{e:?}")));
        let mut opened = Sim::new(def.clone(), 1);
        opened.vars[0] = 1.0;
        let save = opened.save_json();
        let mut page = Sim::new(def.clone(), 1);
        assert_eq!(page.count_tag("door"), 1);
        page.load_save(&save);
        assert_eq!(page.count_tag("door"), 0, "the rule that reacts to `loaded` ran inside the load");
        assert!(page.take_save_if_dirty().is_none(), "restoring a save is not a change worth saving again");
        // A save that restores nothing (nothing persisted differs) still fires the event harmlessly; one for another game is refused and fires nothing.
        let mut other = Sim::new(def.clone(), 1);
        assert!(matches!(other.load_save("{\"red2d_save\":1,\"game\":\"someone-else\",\"vars\":{}}"), SaveStatus::Incompatible(_)));
        assert_eq!(other.count_tag("door"), 1);
    }

    #[test]
    fn patrol_chase_wander_and_pointer_movers_move() {
        let prefabs = r##""p":{"tag":"p","shape":{"rect":[6,6],"color":"#fff"},"move":{"pointer":"x"},"clamp":true},
            "g":{"tag":"g","shape":{"rect":[6,6],"color":"#f00"},"move":{"chase":{"target":"p","speed":30}}},
            "w":{"tag":"w","shape":{"rect":[6,6],"color":"#0f0"},"move":{"wander":{"speed":20,"turn":0.5}}},
            "r":{"tag":"r","shape":{"rect":[6,6],"color":"#00f"},"move":{"patrol":{"axis":"x","range":20,"speed":40}}}"##;
        let mut s = Sim::new(
            game(
                "",
                prefabs,
                r#"{"prefab":"p","at":[80,80],"id":"p"},{"prefab":"g","at":[10,10],"id":"g"},{"prefab":"w","at":[80,40],"id":"w"},{"prefab":"r","at":[80,60],"id":"r"}"#,
            ),
            3,
        );
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

    #[test]
    fn the_builtin_offsets_match_the_list() {
        assert_eq!(BUILTINS[B_LEVEL], "level");
        assert_eq!(BUILTINS[B_HELD], "held_left");
        assert_eq!(BUILTINS[B_HELD + 5], "held_secondary");
        assert_eq!(BUILTINS[B_SELF], "self_x");
        assert_eq!(BUILTINS[B_SELF + 4], "self_face");
        assert_eq!(BUILTINS[B_OTHER], "other_x");
        assert_eq!(BUILTINS[B_GAMEPAD], "gamepad");
        assert_eq!(BUILTINS[B_ACCEL], "self_ax");
        assert_eq!(BUILTINS.len(), B_ACCEL + 2);
    }

    /// A small platformer world: a floor, a player, an enemy and a block in the air.
    const PLAT: &str = r##""floor":{"tag":"solid","shape":{"rect":[160,10],"color":"#888"},"body":{"type":"static"}},
        "block":{"tag":["solid","block"],"shape":{"rect":[16,16],"color":"#a60"},"body":{"type":"static"}},
        "used":{"tag":["solid","used"],"shape":{"rect":[16,16],"color":"#640"},"body":{"type":"static"}},
        "wall":{"tag":"solid","shape":{"rect":[10,90],"color":"#888"},"body":{"type":"static"}},
        "hero":{"tag":"player","shape":{"rect":[8,8],"color":"#fc0"},"body":{"type":"dynamic","gravity":900,"max_fall":400},"collide":["solid"],
          "move":{"keys":{"mode":"platformer","speed":60,"jump":300,"run":120,"accel":600,"cut":0.4,"coyote":0.1,"buffer":0.1}}},
        "big":{"tag":"player","shape":{"rect":[8,16],"color":"#fc0"},"body":{"type":"dynamic","gravity":900},"collide":["solid"],"move":{"keys":{"mode":"platformer","speed":60,"jump":300}}},
        "bug":{"tag":"enemy","shape":{"rect":[8,8],"color":"#f00"},"body":{"type":"dynamic","gravity":900},"collide":["solid"],"move":{"walk":{"speed":20}}}"##;

    #[test]
    fn landing_on_an_enemy_is_a_stomp_and_walking_into_it_is_not() {
        let rules = r##"{"when":{"touch":["player","enemy"]},"side":"bottom","do":[{"add":["score",1]},{"destroy":"other"}]},
            {"when":{"touch":["player","enemy"]},"side":"sides","do":[{"add":["best",1]}]}"##;
        // Dropped from above onto the bug: a stomp.
        let mut s = Sim::new(game(rules, PLAT, r#"{"prefab":"floor","at":[80,85]},{"prefab":"bug","at":[60,76]},{"prefab":"hero","at":[60,40],"id":"p"}"#), 1);
        s.run_ticks(60);
        assert_eq!((s.var("score"), s.var("best")), (Some(1.0), Some(0.0)), "stomped, not hurt");
        // Standing on the floor while the bug walks into it: from the side.
        let mut s = Sim::new(game(rules, PLAT, r#"{"prefab":"floor","at":[80,85]},{"prefab":"bug","at":[60,76]},{"prefab":"hero","at":[30,76],"id":"p"}"#), 1);
        s.run_ticks(120);
        assert_eq!((s.var("score"), s.var("best")), (Some(0.0), Some(1.0)), "met from the side");
    }

    #[test]
    fn jumping_into_a_block_from_below_hits_its_top_side_once_and_become_changes_it() {
        let rules = r##"{"when":{"hit":["player","block"]},"side":"top","do":[{"add":["score",1]},{"become":{"target":"other","prefab":"used"}}]},
            {"when":{"hit":["player","used"]},"side":"top","do":[{"add":["best",1]}]}"##;
        let mut s =
            Sim::new(game(rules, PLAT, r#"{"prefab":"floor","at":[80,85]},{"prefab":"block","at":[40,40]},{"prefab":"hero","at":[40,76],"id":"p"}"#), 1);
        s.run_ticks(10);
        s.key("Space", true);
        s.run_ticks(40);
        s.key("Space", false);
        s.run_ticks(30);
        assert_eq!(s.var("score"), Some(1.0), "one bump");
        assert_eq!(s.count_tag("used"), 1, "the block became a used block");
        s.key("Space", true);
        s.run_ticks(40);
        assert_eq!((s.var("score"), s.var("best")), (Some(1.0), Some(1.0)), "the second bump hits the used block");
        // Standing on top of it is a hit from below, not a bump.
        let mut s =
            Sim::new(game(rules, PLAT, r#"{"prefab":"floor","at":[80,85]},{"prefab":"block","at":[40,60]},{"prefab":"hero","at":[40,30],"id":"p"}"#), 1);
        s.run_ticks(60);
        assert_eq!(s.var("score"), Some(0.0));
    }

    #[test]
    fn become_keeps_the_scene_id_and_the_feet() {
        let rules = r##"{"when":{"press":"secondary"},"do":[{"become":{"target":"self","prefab":"big"}}]}"##;
        let mut s = Sim::new(game(rules, PLAT, r#"{"prefab":"floor","at":[80,85]},{"prefab":"hero","at":[40,76],"id":"p"}"#), 1);
        s.run_ticks(20);
        let feet = s.by_id("p").unwrap().y + 4.0;
        s.key("KeyX", true);
        s.run_ticks(20);
        let p = s.by_id("p").unwrap();
        assert_eq!(s.def.prefabs[p.prefab].name, "big");
        assert!((p.y + 8.0 - feet).abs() < 0.01, "feet stayed: {} vs {feet}", p.y + 8.0);
    }

    #[test]
    fn a_walker_turns_at_a_wall_and_at_a_ledge_when_told() {
        let prefabs = format!(
            r##"{PLAT},"careful":{{"tag":"enemy","shape":{{"rect":[8,8],"color":"#f0f"}},"body":{{"type":"dynamic","gravity":900}},"collide":["solid"],"move":{{"walk":{{"speed":40,"edges":"turn","dir":"right"}}}}}}"##
        );
        let mut s = Sim::new(game("", &prefabs, r#"{"prefab":"floor","at":[80,85]},{"prefab":"wall","at":[20,45]},{"prefab":"bug","at":[60,76],"id":"b"}"#), 1);
        s.run_ticks(240);
        assert!(s.by_id("b").unwrap().vx > 0.0, "turned at the wall and walks right");
        // On a short ledge (a block), the careful walker paces without falling off.
        let mut s =
            Sim::new(game("", &prefabs, r#"{"prefab":"block","at":[80,60]},{"prefab":"floor","at":[80,85]},{"prefab":"careful","at":[80,48],"id":"c"}"#), 1);
        let mut xs = (80.0f32, 80.0f32);
        for _ in 0..300 {
            s.step();
            let c = s.by_id("c").unwrap();
            xs = (xs.0.min(c.x), xs.1.max(c.x));
        }
        assert!(s.by_id("c").unwrap().y < 50.0, "still on the block");
        assert!(xs.0 < 76.5 && xs.1 > 83.5, "paces to both ends of the block: {xs:?}");
    }

    #[test]
    fn running_jumps_higher_and_letting_go_cuts_the_jump() {
        let scene = r#"{"prefab":"floor","at":[80,85]},{"prefab":"hero","at":[20,76],"id":"p"}"#;
        let peak = |hold_jump: u64, run: bool| {
            let mut s = Sim::new(game("", PLAT, scene), 1);
            s.run_ticks(10);
            if run {
                s.key("KeyX", true);
                s.key("ArrowRight", true);
                s.run_ticks(30);
            }
            let y0 = s.by_id("p").unwrap().y;
            s.key("Space", true);
            let mut top = y0;
            for t in 0..60 {
                if t == hold_jump {
                    s.key("Space", false);
                }
                s.step();
                top = top.min(s.by_id("p").unwrap().y);
            }
            y0 - top
        };
        let (short, full) = (peak(3, false), peak(60, false));
        assert!(short < full * 0.6, "a tap is a small hop: {short} vs {full}");
        assert!((full - 300.0 * 300.0 / 1800.0).abs() < 6.0, "full jump about v^2/2g: {full}");
        // run_jump is 0 here: the same height running; speed builds up gradually to the run speed.
        let mut s = Sim::new(game("", PLAT, scene), 1);
        s.run_ticks(10);
        s.key("ArrowRight", true);
        s.step();
        assert!(s.by_id("p").unwrap().vx < 20.0, "accelerates, not instant");
        s.key("KeyX", true);
        s.run_ticks(40);
        assert!((s.by_id("p").unwrap().vx - 120.0).abs() < 0.01, "runs at the run speed");
    }

    #[test]
    fn a_jump_pressed_just_before_landing_and_just_after_a_ledge_still_happens() {
        let scene = r#"{"prefab":"floor","at":[80,85]},{"prefab":"hero","at":[20,60],"id":"p"}"#;
        let mut s = Sim::new(game("", PLAT, scene), 1);
        // Falling: press jump a few ticks before touching the floor.
        loop {
            s.step();
            if s.by_id("p").unwrap().y > 74.0 {
                break;
            }
        }
        s.key("Space", true);
        s.run_ticks(15);
        assert!(s.by_id("p").unwrap().y < 70.0, "the buffered jump happened on landing");
    }

    #[test]
    fn levels_replace_the_world_keep_the_variables_and_scenarios_can_start_in_one() {
        let text = r##"{"game2d":1,"id":"t","title":"T","description":"d",
          "capabilities":{"presentation":"2d","platforms":["windows","linux"],"networking":"offline","input":["keyboard"],"persistence":[]},
          "view":{"width":160,"height":90,"background":"#000000"},"vars":{"score":0},
          "music":{"a":{"bpm":100,"bars":1,"instruments":{"i":{"seconds":0.2,"layers":[{"sine":1,"decay":9}]}},"tracks":[{"inst":"i","play":"chords","chords":"1"}]},
                   "b":{"bpm":100,"bars":1,"instruments":{"i":{"seconds":0.2,"layers":[{"sine":1,"decay":9}]}},"tracks":[{"inst":"i","play":"chords","chords":"1"}]}},
          "prefabs":{"coin":{"tag":"coin","shape":{"rect":[4,4],"color":"#ff0"}},"door":{"tag":"door","shape":{"rect":[4,4],"color":"#00f"}}},
          "levels":[{"id":"one","scene":[{"prefab":"coin","at":[10,10],"id":"c"}],"music":"a"},
                    {"id":"two","map":{"tile":10,"rows":["d.........d...................."],"legend":{"d":"door"}},"background":"#ff0000","music":"b"}],
          "rules":[{"when":{"press":"action"},"do":[{"add":["score",1]},{"level":"level + 1"},{"add":["score","count_door"]}]}],
          "checks":{"scenarios":[{"name":"two","level":"two","script":[{"wait":0.1}],"expect":[{"count":"door","eq":2}]}]}}"##;
        let def = Arc::new(parse(text).unwrap_or_else(|e| panic!("{e:?}")));
        let mut s = Sim::new(def.clone(), 1);
        assert_eq!((s.count_tag("coin"), s.count_tag("door"), s.music_track), (1, 0, 0));
        s.key("Space", true);
        s.step();
        assert_eq!((s.count_tag("coin"), s.count_tag("door"), s.level, s.music_track), (0, 2, 1, 1));
        assert_eq!(s.var("score"), Some(3.0), "kept, and the action after `level` saw the new world");
        assert_eq!(s.var("level"), Some(1.0));
        assert_eq!(s.world, (310.0, 90.0), "the map's width: 31 tiles");
        assert_eq!(s.background, [255, 0, 0, 255]);
        let (rep, _) = crate::script::run_scenario(&def, &def.scenarios[0], None);
        assert!(rep.failures.is_empty(), "{:?}", rep.failures);
        // Asking for a level that is not there changes nothing and says so.
        s.key("Space", false);
        s.step();
        s.key("Space", true);
        s.step();
        assert_eq!(s.level, 1);
        assert!(s.notes.iter().any(|n| n.contains("asked for level 2")), "{:?}", s.notes);
        // A top-level scene beside levels is refused, and so is a level action without levels.
        assert!(parse(&text.replace(r#""vars":{"score":0},"#, r#""vars":{"score":0},"scene":[{"prefab":"coin","at":[1,1]}],"#)).is_err());
    }

    #[test]
    fn asleep_things_wait_for_the_camera_orbiters_circle_and_the_camera_only_goes_forward() {
        let prefabs = r##""floor":{"tag":"solid","shape":{"rect":[640,10],"color":"#888"},"body":{"type":"static"}},
            "hero":{"tag":"player","shape":{"rect":[8,8],"color":"#fc0"},"body":{"type":"dynamic","gravity":900},"collide":["solid"],"move":{"keys":{"mode":"platformer","speed":120,"jump":300}}},
            "bug":{"tag":"enemy","shape":{"rect":[8,8],"color":"#f00"},"body":{"type":"dynamic","gravity":900},"collide":["solid"],"move":{"walk":{"speed":20}},"asleep":true},
            "spark":{"tag":"spark","shape":{"rect":[4,4],"color":"#f80"},"move":{"orbit":{"radius":10,"speed":90}}}"##;
        let text = format!(
            r##"{{"game2d":1,"id":"t","title":"T","description":"d","capabilities":{{"presentation":"2d","platforms":["windows","linux"],"networking":"offline","input":["keyboard"],"persistence":[]}},
            "view":{{"width":160,"height":90,"world":[640,90],"camera":{{"follow":"player","lerp":1,"forward_only":true}}}},
            "prefabs":{{{prefabs}}},"scene":[{{"prefab":"floor","at":[320,85]}},{{"prefab":"hero","at":[20,76],"id":"p"}},{{"prefab":"bug","at":[500,76],"id":"b"}},{{"prefab":"spark","at":[40,40],"id":"s"}}]}}"##
        );
        let mut s = Sim::new(Arc::new(parse(&text).unwrap_or_else(|e| panic!("{e:?}"))), 1);
        s.run_ticks(60);
        assert_eq!(s.by_id("b").unwrap().x, 500.0, "asleep while far away");
        let sp = s.by_id("s").unwrap();
        assert!(((sp.x - 40.0).powi(2) + (sp.y - 40.0).powi(2)).sqrt() - 10.0 < 0.01, "on its circle");
        s.key("ArrowRight", true);
        s.run_ticks(220);
        s.key("ArrowRight", false);
        s.run_ticks(10);
        assert!(s.by_id("b").is_none_or(|b| b.x < 500.0), "woke when the camera came near");
        let cam = s.cam[0];
        assert!(cam > 100.0);
        s.key("ArrowLeft", true);
        s.run_ticks(120);
        assert_eq!(s.cam[0], cam, "the camera never goes back");
        assert!((s.by_id("p").unwrap().x - (cam + 4.0)).abs() < 0.01, "the player stops at the screen's left edge");
    }

    #[test]
    fn a_moving_platform_carries_what_stands_on_it() {
        let prefabs = r##""lift":{"tag":"solid","shape":{"rect":[32,8],"color":"#888"},"body":{"type":"dynamic"},"move":{"patrol":{"axis":"x","range":30,"speed":40}}},
            "hero":{"tag":"player","shape":{"rect":[8,8],"color":"#fc0"},"body":{"type":"dynamic","gravity":900},"collide":["solid"],"move":{"keys":{"mode":"platformer","speed":60,"jump":300}}}"##;
        let mut s = Sim::new(game("", prefabs, r#"{"prefab":"lift","at":[80,60],"id":"l"},{"prefab":"hero","at":[80,40],"id":"p"}"#), 1);
        s.run_ticks(40);
        let off = s.by_id("p").unwrap().x - s.by_id("l").unwrap().x;
        let mut drift = 0.0f32;
        for _ in 0..200 {
            s.step();
            drift = drift.max((s.by_id("p").unwrap().x - s.by_id("l").unwrap().x - off).abs());
        }
        assert!(drift < 0.01, "rode along, both ways: drifted {drift} px");
        assert!(s.by_id("p").unwrap().y < 60.0, "still on top");
    }

    #[test]
    fn rule_context_variables_describe_self_and_other() {
        let rules = r##"{"when":{"touch":["player","coin"]},"do":[{"set":["score","other_x - self_x"]}]},
            {"when":{"press":"secondary"},"do":[{"set":["best","self_face * 10 + held_secondary"]}]}"##;
        let mut s =
            Sim::new(game(rules, WALLS, r#"{"prefab":"floor","at":[80,85]},{"prefab":"player","at":[20,76],"id":"p"},{"prefab":"coin","at":[23,76]}"#), 1);
        s.key("ArrowLeft", true);
        s.run_ticks(3);
        s.key("ArrowLeft", false);
        s.key("KeyX", true);
        s.step();
        assert!(s.var("score").unwrap() > 0.0, "the coin was to the right of the player: {:?}", s.var("score"));
        assert_eq!(s.var("best"), Some(-9.0), "facing left, secondary held");
    }

    /// Copper Choir's guest panics at sudden moves: `self_ax` is how fast the keys' thing changed speed (px/s per second), 0 at a steady speed.
    #[test]
    fn self_ax_is_how_sharply_the_rules_own_thing_changed_speed() {
        let rules = r##"{"when":{"press":"secondary"},"do":[{"set":["score","self_ax"]}]},{"when":{"press":"action"},"do":[{"set":["best","self_ax"]}]}"##;
        let mut s = Sim::new(game(rules, WALLS, r#"{"prefab":"floor","at":[80,85]},{"prefab":"player","at":[30,76],"id":"p"}"#), 1);
        s.run_ticks(20);
        s.key("ArrowRight", true);
        s.step();
        let speed = s.by_id("p").unwrap().vx;
        s.key("KeyX", true);
        s.step();
        assert!(
            speed > 0.0 && (s.var("score").unwrap() - f64::from(speed) * TPS as f64).abs() < 1e-3,
            "from rest to {speed} px/s in one tick: {:?}",
            s.var("score")
        );
        s.run_ticks(60);
        s.key("Space", true);
        s.step();
        assert_eq!(s.var("best"), Some(0.0), "a steady run is no jolt");
    }
}
