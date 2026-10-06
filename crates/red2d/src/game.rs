//! A 2D game as data: the strict parser for `*.game2d.json`.
//!
//! One JSON file holds a whole game: `capabilities`, the virtual screen (`view`), pixel-art `sprites`, `sounds` and `music` as voice and score descriptions, `vars`,
//! `prefabs` (what a thing is: its look, size, tags, body, how it moves), a `scene` or tile `map` (where things start), a `ui` (HUD text, bars, buttons), `rules`
//! (when X, if Y, do Z: the same shape as the 3D rules and the same expression language) and `checks` (scripted headless playthroughs). Parsing never guesses: an unknown
//! key is an error with a likely fix, a value of the wrong type says what was expected, a name that refers to nothing (a sprite, a sound, a prefab, a tag, a variable)
//! says what exists, and a game that asks for something its `capabilities` do not declare (a pointer without `mouse`, saved variables without `persistence`) is refused.
//! Errors are `path: message` lines and all of them are reported at once.

use crate::caps::{self, Capabilities, Input, Persistence};
use crate::fields::{check_fields, check_keys, describe_value, opt, req, Field, Ty};
use crate::rules_expr::{self, Expr};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// The only format version.
pub const GAME_VERSION: u64 = 1;
/// The largest sprite frame, pixels a side.
pub const MAX_SPRITE: usize = 64;
/// The largest virtual screen, pixels a side.
pub const MAX_VIEW: u32 = 1280;

/// RGBA.
pub type Color = [u8; 4];

/// `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn parse_color(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    let v = |i: usize, n: usize| u8::from_str_radix(h.get(i..i + n)?, 16).ok();
    match h.len() {
        3 => Some([v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17, 255]),
        6 => Some([v(0, 2)?, v(2, 2)?, v(4, 2)?, 255]),
        8 => Some([v(0, 2)?, v(2, 2)?, v(4, 2)?, v(6, 2)?]),
        _ => None,
    }
}

pub(crate) fn color_ok(v: &Value) -> Result<(), String> {
    match v.as_str() {
        Some(s) if parse_color(s).is_some() => Ok(()),
        Some(s) => Err(format!("`{s}` is not a color: write #rgb, #rrggbb or #rrggbbaa, like \"#ffcc00\"")),
        None => Err(format!("expected a color string like \"#ffcc00\", got {}", describe_value(v))),
    }
}

/// How the virtual screen is placed in a window of another shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    /// The largest scale that fits, any fraction.
    Fit,
    /// The largest whole-number scale that fits: crisp pixels.
    Integer,
}

/// The virtual screen and its camera.
#[derive(Debug, Clone)]
pub struct View {
    /// Pixels wide.
    pub width: u32,
    /// Pixels high.
    pub height: u32,
    /// The clear colour.
    pub background: Color,
    /// How it is scaled into a window.
    pub scale: Scale,
    /// The world's size (the camera stays inside it); the view's own size by default.
    pub world: (f32, f32),
    /// The tag the camera follows, if any.
    pub follow: Option<String>,
    /// How much of the remaining distance the camera closes per tick (1 = locked on).
    pub lerp: f32,
}

/// A sprite: one or more frames of paletted pixels.
#[derive(Debug, Clone)]
pub struct Sprite {
    /// Its name.
    pub name: String,
    /// Frame width.
    pub w: usize,
    /// Frame height.
    pub h: usize,
    /// RGBA pixels per frame, row-major (alpha 0 = transparent).
    pub frames: Vec<Vec<Color>>,
    /// Frames per second when it has several.
    pub fps: f32,
}

/// Mirroring a sprite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flip {
    /// As drawn.
    None,
    /// Mirrored left-right.
    X,
    /// Mirrored while the thing last moved left (it keeps facing that way when it stops).
    Auto,
}

/// What a prefab looks like.
#[derive(Debug, Clone)]
pub enum Shape {
    /// Nothing drawn.
    None,
    /// A sprite, at an integer scale.
    Sprite {
        /// Index into [`GameDef::sprites`].
        sprite: usize,
        /// Integer scale.
        scale: u32,
        /// Mirroring.
        flip: Flip,
    },
    /// A filled rectangle.
    Rect {
        /// Colour.
        color: Color,
    },
    /// A filled circle (collision is its bounding box).
    Circle {
        /// Colour.
        color: Color,
    },
    /// A 3D model, drawn into the thing's box (a hybrid game).
    Model(crate::game3d::ModelShape),
    /// Text, with `{var}` placeholders.
    Text {
        /// The template.
        text: String,
        /// Colour.
        color: Color,
        /// Integer scale.
        scale: u32,
    },
}

/// How a prefab's body behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// Moves under gravity and is stopped by the tags it `collide`s with.
    Dynamic,
    /// Never moves.
    Static,
}

/// Physics for a prefab.
#[derive(Debug, Clone, Copy)]
pub struct Body {
    /// Dynamic or static.
    pub kind: BodyKind,
    /// Downward acceleration, px/s^2.
    pub gravity: f32,
    /// Speed kept (reversed) when it hits something solid, 0 to 1.
    pub bounce: f32,
    /// Fraction of horizontal speed lost per tick while standing on something, 0 to 1 (1 = stops at once).
    pub friction: f32,
    /// Fraction of speed lost per tick.
    pub drag: f32,
    /// Terminal fall speed, px/s.
    pub max_fall: f32,
}

/// Which keys drive a `keys` mover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    /// Four directions.
    TopDown,
    /// Left and right, jump, gravity from the body.
    Platformer,
}

/// How a prefab moves by itself.
#[derive(Debug, Clone)]
pub enum Move {
    /// Not at all (it may still be moved by physics or a rule).
    None,
    /// The player's keys (or controller).
    Keys {
        /// Mode.
        mode: KeyMode,
        /// px/s.
        speed: f32,
        /// Jump speed, px/s (platformer).
        jump: f32,
    },
    /// Follows the pointer on an axis.
    Pointer {
        /// Follow x.
        x: bool,
        /// Follow y.
        y: bool,
    },
    /// Heads for the nearest thing with a tag.
    Chase {
        /// The tag.
        target: String,
        /// px/s.
        speed: f32,
    },
    /// Constant velocity.
    Drift {
        /// px/s.
        v: [f32; 2],
    },
    /// Changes heading at random every `turn` seconds.
    Wander {
        /// px/s.
        speed: f32,
        /// Seconds between turns.
        turn: f32,
    },
    /// Back and forth along an axis.
    Patrol {
        /// Along x (else y).
        x: bool,
        /// How far each way from where it starts.
        range: f32,
        /// px/s.
        speed: f32,
    },
}

/// A continuous particle emitter on a prefab.
#[derive(Debug, Clone, Copy)]
pub struct Emitter {
    /// Particles per second.
    pub rate: f32,
    /// Lifetime range, s.
    pub life: [f32; 2],
    /// Speed range, px/s.
    pub speed: [f32; 2],
    /// Direction range, degrees (0 = right, 90 = down).
    pub angle: [f32; 2],
    /// Colour.
    pub color: Color,
    /// Size, px.
    pub size: f32,
    /// Downward acceleration.
    pub gravity: f32,
}

/// What a kind of thing is.
#[derive(Debug, Clone)]
pub struct Prefab {
    /// Its name.
    pub name: String,
    /// Tags: counted (`count_<tag>`), matched by rules.
    pub tags: Vec<String>,
    /// Its look.
    pub shape: Shape,
    /// Collision box, px.
    pub size: [f32; 2],
    /// Draw order: higher is on top.
    pub layer: i32,
    /// Physics, if any.
    pub body: Option<Body>,
    /// Tags this thing is stopped by.
    pub collide: Vec<String>,
    /// How it moves.
    pub mv: Move,
    /// Seconds to live.
    pub ttl: Option<f32>,
    /// Particle emitter.
    pub emit: Option<Emitter>,
    /// Kept inside the world.
    pub clamp: bool,
    /// Not drawn.
    pub hidden: bool,
    /// In a `world3d` view: stands this tall (a box with the thing's footprint). None = a flat billboard.
    pub height3d: Option<f32>,
}

/// A number or an expression over the variables.
#[derive(Debug, Clone)]
pub enum Val {
    /// A literal.
    Num(f64),
    /// Evaluated against the variables each time.
    Expr(Expr),
}

impl Val {
    /// The value now.
    pub fn eval(&self, vars: &[f64]) -> f64 {
        match self {
            Val::Num(n) => *n,
            Val::Expr(e) => e.eval(vars),
        }
    }
}

/// A coordinate or a random range of them.
#[derive(Debug, Clone)]
pub enum Coord {
    /// Exactly this.
    At(Val),
    /// Uniform between.
    Range(f32, f32),
}

/// Where something appears.
#[derive(Debug, Clone)]
pub enum Place {
    /// Coordinates.
    Xy(Coord, Coord),
    /// Where the rule's `self` is.
    Own,
    /// Where the rule's `other` is.
    Other,
    /// The pointer (world coordinates).
    Pointer,
}

/// Who an action acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The rule's first thing.
    Own,
    /// The rule's second thing.
    Other,
    /// Everything with a tag.
    Tag(String),
    /// The scene thing with this id.
    Id(String),
}

/// How a game ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Won.
    Win,
    /// Lost.
    Lose,
}

impl Outcome {
    /// `win` or `lose`.
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Win => "win",
            Outcome::Lose => "lose",
        }
    }
}

/// What the `music` action does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicCmd {
    /// Play.
    On,
    /// Silence.
    Off,
    /// Flip.
    Toggle,
}

/// One thing a rule does.
#[derive(Debug, Clone)]
pub enum Act {
    /// `set`.
    Set(usize, Val),
    /// `add`.
    Add(usize, Val),
    /// `emit`.
    Emit(String),
    /// `spawn`.
    Spawn {
        /// Prefab index.
        prefab: usize,
        /// Where.
        at: Place,
        /// Starting velocity.
        vel: Option<[Coord; 2]>,
        /// How many.
        count: u32,
    },
    /// `destroy`.
    Destroy(Target),
    /// `play`.
    Play(usize),
    /// `music`.
    Music(MusicCmd),
    /// `burst`.
    Burst {
        /// Where.
        at: Place,
        /// Particles.
        n: u32,
        /// Colour.
        color: Color,
        /// Speed range.
        speed: [f32; 2],
        /// Lifetime range.
        life: [f32; 2],
        /// Size.
        size: f32,
        /// Gravity.
        gravity: f32,
    },
    /// `shake`.
    Shake(f32),
    /// `end`.
    End(Outcome),
    /// `restart`.
    Restart,
    /// `reset_save`.
    ResetSave,
    /// `velocity`: set a thing's velocity.
    Velocity(Target, [Val; 2]),
    /// `teleport`: put a thing somewhere.
    Teleport(Target, Place),
}

/// What starts a rule.
#[derive(Debug, Clone, PartialEq)]
pub enum When {
    /// The game begins (and after a restart).
    Start,
    /// Every so many seconds.
    Every(f32),
    /// Once, after so many seconds.
    After(f32),
    /// Two tagged things begin to overlap.
    Touch(String, String),
    /// Two tagged things overlap, every tick.
    Touching(String, String),
    /// An action is pressed (down edge).
    Press(String),
    /// A tagged thing is clicked (`*` = a click on nothing else).
    Click(String),
    /// A named event is emitted.
    Event(String),
    /// The game ends (`None` = either way).
    End(Option<Outcome>),
}

/// A rule.
#[derive(Debug, Clone)]
pub struct Rule {
    /// Its id (or `rule N`).
    pub id: String,
    /// What starts it.
    pub when: When,
    /// Only if.
    pub cond: Option<Expr>,
    /// At most once per game.
    pub once: bool,
    /// Minimum seconds between firings.
    pub cooldown: f32,
    /// What it does.
    pub actions: Vec<Act>,
}

/// A HUD widget.
#[derive(Debug, Clone)]
pub struct Widget {
    /// What it is.
    pub kind: WidgetKind,
    /// Shown only while this holds.
    pub show: Option<Expr>,
}

/// The kinds of HUD widget.
#[derive(Debug, Clone)]
pub enum WidgetKind {
    /// Text with `{var}` placeholders.
    Text {
        /// Template.
        text: String,
        /// Top-left (or the anchor for `align`).
        at: [f32; 2],
        /// Colour.
        color: Color,
        /// Integer scale.
        scale: u32,
        /// `left`, `center` or `right`.
        align: Align,
    },
    /// A filled bar showing `var` out of `max`.
    Bar {
        /// The variable.
        var: usize,
        /// The full value.
        max: Val,
        /// Top-left.
        at: [f32; 2],
        /// Size.
        size: [f32; 2],
        /// Fill colour.
        color: Color,
        /// Empty colour.
        back: Color,
    },
    /// A filled rectangle.
    Panel {
        /// Top-left.
        at: [f32; 2],
        /// Size.
        size: [f32; 2],
        /// Colour.
        color: Color,
    },
    /// A small 3D scene in a rectangle (a hybrid game).
    View3d(crate::game3d::View3d),
    /// A flat map of the world: a dot for each thing with a listed tag.
    Minimap {
        /// Top-left.
        at: [f32; 2],
        /// Size.
        size: [f32; 2],
        /// Tag and dot colour; the first listed tag a thing has decides its colour.
        colors: Vec<(String, Color)>,
        /// Background.
        back: Color,
        /// Border.
        border: Color,
        /// Dot side in px.
        dot: f32,
        /// Outline the part of the world the camera shows.
        viewport: bool,
    },
    /// A clickable button.
    Button {
        /// Its id (a scenario can click it by id).
        id: String,
        /// Its label.
        label: String,
        /// Top-left.
        at: [f32; 2],
        /// Size.
        size: [f32; 2],
        /// Colour.
        color: Color,
        /// Key code that also presses it (`KeyR`, `Enter`).
        key: Option<String>,
        /// What it does.
        actions: Vec<Act>,
    },
}

/// Text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Left edge at `at`.
    Left,
    /// Centred on `at`.
    Center,
    /// Right edge at `at`.
    Right,
}

/// One scene placement.
#[derive(Debug, Clone)]
pub struct Placement {
    /// Prefab index.
    pub prefab: usize,
    /// Position.
    pub at: [f32; 2],
    /// Name (for `id:` targets and the `<id>_x`, `<id>_y` variables).
    pub id: Option<String>,
}

/// One step of a scripted playthrough.
#[derive(Debug, Clone)]
pub enum Step {
    /// Idle.
    Wait(f32),
    /// Hold actions for seconds.
    Hold(Vec<String>, f32),
    /// Hold actions until something is true (or the timeout, which fails).
    HoldUntil(Vec<String>, Expect, f32),
    /// Tap an action.
    Press(String),
    /// Move the pointer and click.
    Click([f32; 2]),
    /// Click a HUD button by id.
    Button(String),
    /// Move the pointer.
    Point([f32; 2]),
    /// Steer toward the nearest thing with a tag for seconds (a bot player).
    Approach(String, f32),
    /// Idle until a condition holds (or the timeout).
    WaitUntil(Expect, f32),
}

/// What a scenario asserts.
#[derive(Debug, Clone)]
pub enum Expect {
    /// A variable against a number.
    Var(usize, Cmp, f64),
    /// The game ended so.
    Ended(Outcome),
    /// The game has not ended.
    NotEnded,
    /// How many things have a tag.
    Count(String, Cmp, f64),
    /// A scene thing is near a point.
    Near(String, [f32; 2], f32),
    /// An event was emitted so many times (min, max).
    Event(String, u32, Option<u32>),
    /// A sound was played so many times at least.
    Sound(usize, u32),
    /// The state hash.
    Hash(String),
    /// Whether the thing `from` (a scene id) can walk to touch `to` (a scene id or `tag:NAME`) in the world as it stands: `true` when it must be able to, `false` when it must not.
    Reach(String, String, bool),
}

/// A comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    /// ==
    Eq,
    /// !=
    Ne,
    /// >
    Gt,
    /// >=
    Gte,
    /// <
    Lt,
    /// <=
    Lte,
}

impl Cmp {
    /// Whether `a cmp b`.
    pub fn holds(self, a: f64, b: f64) -> bool {
        match self {
            Cmp::Eq => (a - b).abs() < 1e-9,
            Cmp::Ne => (a - b).abs() >= 1e-9,
            Cmp::Gt => a > b,
            Cmp::Gte => a >= b,
            Cmp::Lt => a < b,
            Cmp::Lte => a <= b,
        }
    }
    /// The JSON key.
    pub fn name(self) -> &'static str {
        match self {
            Cmp::Eq => "eq",
            Cmp::Ne => "ne",
            Cmp::Gt => "gt",
            Cmp::Gte => "gte",
            Cmp::Lt => "lt",
            Cmp::Lte => "lte",
        }
    }
}

/// A scripted headless playthrough.
#[derive(Debug, Clone)]
pub struct Scenario {
    /// Its name.
    pub name: String,
    /// Random seed for this run.
    pub seed: u64,
    /// Longest it may run, game seconds.
    pub max_seconds: f32,
    /// The script.
    pub script: Vec<Step>,
    /// What must hold at the end.
    pub expect: Vec<Expect>,
    /// This is the playthrough the browser repeats: its final state hash must match.
    pub smoke: bool,
}

/// Map analysis of the starting world: whether one thing can walk to another, optionally with some things (a gate) assumed gone.
#[derive(Debug, Clone)]
pub struct ReachCheck {
    /// Its name (the `why`, or from -> to).
    pub name: String,
    /// The walker: a scene id.
    pub from: String,
    /// The destination: a scene id or `tag:NAME`.
    pub to: String,
    /// Scene ids or `tag:NAME`s assumed removed first (what a rule would open).
    pub open: Vec<String>,
    /// Whether the walker must be able to get there.
    pub reachable: bool,
}

/// A browser input check: real key and pointer events, then something must have changed.
#[derive(Debug, Clone)]
pub struct BrowserCheck {
    /// Its name.
    pub name: String,
    /// Key codes to hold (`ArrowRight`).
    pub keys: Vec<String>,
    /// A click in virtual coordinates.
    pub click: Option<[f32; 2]>,
    /// Milliseconds to hold keys.
    pub ms: u32,
    /// Names (variables, `<id>_x`, `count_<tag>`) of which at least one must change.
    pub changes: Vec<String>,
    /// Saved values (a persisted variable, or `music_on`) that must be the same after the page is reloaded.
    pub persists: Vec<String>,
}

/// A sound.
#[derive(Debug, Clone)]
pub struct SoundDef {
    /// Its name.
    pub name: String,
    /// Its voice description (the same JSON `red_engine2 audio export` writes).
    pub voice: Value,
}

/// The whole game.
#[derive(Debug, Clone)]
pub struct GameDef {
    /// Stable id (kebab-case).
    pub id: String,
    /// Display title.
    pub title: String,
    /// One-sentence description.
    pub description: String,
    /// What it declares.
    pub caps: Capabilities,
    /// Screen and camera.
    pub view: View,
    /// Sprites.
    pub sprites: Vec<Sprite>,
    /// Sounds.
    pub sounds: Vec<SoundDef>,
    /// Music scores (name, score JSON).
    pub music: Vec<(String, Value)>,
    /// Declared variables with their starting values.
    pub vars: Vec<(String, f64)>,
    /// Every variable name an expression may use: declared, built-in, `count_<tag>`, `<id>_x`/`<id>_y`.
    pub var_names: Vec<String>,
    /// Declared variables that are saved between sessions.
    pub persist: Vec<usize>,
    /// Prefabs.
    pub prefabs: Vec<Prefab>,
    /// Starting things.
    pub placements: Vec<Placement>,
    /// HUD.
    pub ui: Vec<Widget>,
    /// Rules.
    pub rules: Vec<Rule>,
    /// Every tag used.
    pub tags: Vec<String>,
    /// Headless scenarios.
    pub scenarios: Vec<Scenario>,
    /// Browser input checks.
    pub browser: Vec<BrowserCheck>,
    /// Map analysis of the starting world.
    pub reach: Vec<ReachCheck>,
    /// 3D models (a hybrid game).
    pub models: Vec<crate::game3d::Model>,
    /// 3D viewports drawn among the entities.
    pub layers3d: Vec<crate::game3d::Layer3d>,
    /// The whole world seen in 3D.
    pub world3d: Option<crate::game3d::World3d>,
    /// The touch controller a phone shows below the game (written in `controls`, else chosen from what the game reads).
    pub controls: crate::controls::Controls,
    /// Hash of the game text: its revision.
    pub rev: String,
}

impl GameDef {
    /// The 3D elements the game uses, by name: a hybrid game has at least one.
    pub fn elements_3d(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.prefabs.iter().any(|p| matches!(p.shape, Shape::Model(_))) {
            v.push("a prefab drawn as a 3D model");
        }
        if self.ui.iter().any(|w| matches!(w.kind, WidgetKind::View3d(_))) {
            v.push("a `view3d` widget");
        }
        if !self.layers3d.is_empty() {
            v.push("`layers3d`");
        }
        if self.world3d.is_some() {
            v.push("`view.world3d`");
        }
        v
    }
    /// Whether any 3D element is used.
    pub fn uses_3d(&self) -> bool {
        !self.elements_3d().is_empty()
    }
}

/// The built-in variables after the declared ones.
pub const BUILTINS: &[&str] = &["time", "tick", "ended", "mouse_x", "mouse_y", "music_on"];

/// Actions a game responds to (keys and controllers map onto them).
pub const ACTIONS: &[&str] = &["left", "right", "up", "down", "action", "secondary", "pause"];

/// The triggers a rule's `when` takes.
pub const TRIGGERS: &[&str] = &["start", "every", "after", "touch", "touching", "press", "click", "event", "end"];
/// The movers a prefab's `move` takes.
pub const MOVERS: &[&str] = &["keys", "pointer", "chase", "drift", "wander", "patrol"];
/// The steps of a scripted playthrough.
pub const STEP_KEYS: &[&str] = &["wait", "hold", "hold_until", "press", "click", "button", "point", "approach", "wait_until"];
/// The assertions of a scenario.
pub const EXPECT_KEYS: &[&str] = &["var", "ended", "not_ended", "count", "entity", "event", "sound", "hash"];
/// The keys at the root of a game file.
pub const ROOT: &[&str] = &[
    "game2d",
    "id",
    "title",
    "description",
    "capabilities",
    "view",
    "sprites",
    "sounds",
    "music",
    "vars",
    "persist",
    "prefabs",
    "scene",
    "map",
    "ui",
    "rules",
    "controls",
    "models",
    "layers3d",
    "checks",
];

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// The revision of a game text: 16 hex digits of its content hash.
pub fn revision(text: &str) -> String {
    format!("{:016x}", fnv(text))
}

pub(crate) struct Ctx {
    pub(crate) errs: Vec<String>,
}

impl Ctx {
    pub(crate) fn err(&mut self, path: impl Into<String>, msg: impl Into<String>) {
        self.errs.push(format!("{}: {}", path.into(), msg.into()));
    }
    pub(crate) fn obj<'a>(&mut self, path: &str, v: &'a Value) -> Option<&'a Map<String, Value>> {
        let o = v.as_object();
        if o.is_none() {
            self.err(path, format!("expected an object, got {}", describe_value(v)));
        }
        o
    }
    pub(crate) fn near(name: &str, all: impl Iterator<Item = String>) -> String {
        let all: Vec<String> = all.collect();
        let near = crate::suggest::suggest(name, all.iter().map(String::as_str));
        let did = near.first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default();
        format!("{did} (known: {})", if all.is_empty() { "none".to_string() } else { all.join(", ") })
    }
}

pub(crate) fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}

pub(crate) fn pair(v: &Value) -> Option<[f32; 2]> {
    let a = v.as_array()?;
    (a.len() == 2).then(|| Some([num(&a[0])? as f32, num(&a[1])? as f32]))?
}

fn slug_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

const SPRITE_FIELDS: &[Field] = &[
    opt("rows", Ty::Strs, "one string per pixel row: a letter from `palette`, or . for transparent"),
    opt("frames", Ty::Custom(frames_ok), "a list of animation frames, each a list of row strings"),
    opt("palette", Ty::Custom(palette_ok), "{ \"a\": \"#ff0000\" }: a one-character key per colour"),
    opt("fps", Ty::Num(Some((0.1, 60.0))), "animation speed, default 8"),
    opt("file", Ty::Str, "not supported inline: files are resolved before parsing"),
];

fn frames_ok(v: &Value) -> Result<(), String> {
    let list = v
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| format!("expected a non-empty list of frames (each a list of row strings), got {}", describe_value(v)))?;
    for (i, f) in list.iter().enumerate() {
        if !f.as_array().is_some_and(|rows| !rows.is_empty() && rows.iter().all(Value::is_string)) {
            return Err(format!("frame {i} must be a non-empty list of row strings, got {}", describe_value(f)));
        }
    }
    Ok(())
}

fn palette_ok(v: &Value) -> Result<(), String> {
    let o = v.as_object().ok_or_else(|| format!("expected an object like {{\"a\": \"#ff0000\"}}, got {}", describe_value(v)))?;
    for (k, c) in o {
        if k.chars().count() != 1 {
            return Err(format!("palette key `{k}` must be ONE character"));
        }
        if matches!(k.as_str(), "." | " ") {
            return Err(format!("palette key `{k}` is transparent and cannot be given a colour"));
        }
        color_ok(c).map_err(|e| format!("`{k}`: {e}"))?;
    }
    Ok(())
}

fn parse_sprite(ctx: &mut Ctx, name: &str, v: &Value) -> Option<Sprite> {
    let path = format!("sprites.{name}");
    let o = ctx.obj(&path, v)?;
    let before = ctx.errs.len();
    check_fields(&mut ctx.errs, &path, o, SPRITE_FIELDS);
    if o.contains_key("file") {
        ctx.err(
            &path,
            "an image file cannot be read here: the loader inlines `{\"file\": \"x.json\"}` assets before parsing, and a sprite is written as pixel `rows`",
        );
    }
    if ctx.errs.len() > before {
        return None;
    }
    let palette: HashMap<char, Color> = o
        .get("palette")
        .and_then(Value::as_object)
        .map(|p| p.iter().filter_map(|(k, c)| Some((k.chars().next()?, parse_color(c.as_str()?)?))).collect())
        .unwrap_or_default();
    let frame_rows: Vec<Vec<&str>> = match (o.get("rows"), o.get("frames")) {
        (Some(_), Some(_)) => {
            ctx.err(&path, "give `rows` (one picture) or `frames` (an animation), not both");
            return None;
        }
        (Some(r), None) => vec![r.as_array()?.iter().filter_map(Value::as_str).collect()],
        (None, Some(f)) => f.as_array()?.iter().map(|fr| fr.as_array().map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default()).collect(),
        (None, None) => {
            ctx.err(&path, "needs `rows` (like [\".aa.\", \"aaaa\"]) or `frames`");
            return None;
        }
    };
    let (h, w) = (frame_rows[0].len(), frame_rows[0].first().map_or(0, |r| r.chars().count()));
    if w == 0 || h == 0 || w > MAX_SPRITE || h > MAX_SPRITE {
        ctx.err(&path, format!("a sprite is 1 to {MAX_SPRITE} pixels a side, this is {w}x{h}"));
        return None;
    }
    let mut frames = Vec::new();
    for (fi, rows) in frame_rows.iter().enumerate() {
        if rows.len() != h {
            ctx.err(&path, format!("frame {fi} has {} rows, the first has {h}: every frame is the same size", rows.len()));
            return None;
        }
        let mut px = Vec::with_capacity(w * h);
        for (ri, row) in rows.iter().enumerate() {
            if row.chars().count() != w {
                ctx.err(&path, format!("row {ri} of frame {fi} is {} wide, the first row is {w}: rows must be equally wide", row.chars().count()));
                return None;
            }
            for ch in row.chars() {
                match ch {
                    '.' | ' ' => px.push([0, 0, 0, 0]),
                    c => match palette.get(&c) {
                        Some(col) => px.push(*col),
                        None => {
                            let known: Vec<String> = palette.keys().map(char::to_string).collect();
                            ctx.err(
                                &path,
                                format!("row {ri} of frame {fi} uses `{c}`, which is not in `palette` (palette has: {}; . is transparent)", known.join(" ")),
                            );
                            return None;
                        }
                    },
                }
            }
        }
        frames.push(px);
    }
    let fps = o.get("fps").and_then(num).unwrap_or(8.0) as f32;
    Some(Sprite { name: name.to_string(), w, h, frames, fps })
}

fn tags_of(v: Option<&Value>) -> Result<Vec<String>, String> {
    match v {
        None => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(vec![s.clone()]),
        Some(Value::Array(a)) if a.iter().all(Value::is_string) => Ok(a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()),
        Some(other) => Err(format!("expected a tag name or a list of tag names, got {}", describe_value(other))),
    }
}

const PREFAB_FIELDS: &[Field] = &[
    opt("tag", Ty::Custom(tag_value_ok), "a tag name, or a list of them"),
    opt("shape", Ty::Custom(object_ok), "{ \"sprite\": name } | { \"rect\": [w,h], \"color\": c } | { \"circle\": r, \"color\": c } | { \"text\": \"...\" }"),
    opt("size", Ty::Nums(&[2]), "collision box [w, h], px (default: the shape's size)"),
    opt("layer", Ty::Custom(int_ok), "draw order, higher on top (default 0)"),
    opt("body", Ty::Custom(object_ok), "{ \"type\": \"dynamic\"|\"static\", \"gravity\": 300, \"bounce\": 0.5 ... }"),
    opt("collide", Ty::Strs, "tags this thing is stopped by"),
    opt(
        "move",
        Ty::Custom(object_ok),
        "{ \"keys\": {...} } | { \"pointer\": \"x\" } | { \"chase\": {...} } | { \"drift\": [vx, vy] } | { \"wander\": {...} } | { \"patrol\": {...} }",
    ),
    opt("ttl", Ty::Num(Some((0.01, 100_000.0))), "seconds to live"),
    opt("emit", Ty::Custom(object_ok), "a continuous particle emitter"),
    opt("clamp", Ty::Bool, "keep inside the world"),
    opt("hidden", Ty::Bool, "not drawn"),
    opt("height3d", Ty::Num(Some((0.5, 1000.0))), "in a `world3d` view the thing stands this tall (a box on its footprint); omitted = a flat billboard"),
];

fn object_ok(v: &Value) -> Result<(), String> {
    v.is_object().then_some(()).ok_or_else(|| format!("expected an object, got {}", describe_value(v)))
}
fn int_ok(v: &Value) -> Result<(), String> {
    v.as_i64().map(|_| ()).ok_or_else(|| format!("expected a whole number, got {}", describe_value(v)))
}
fn tag_value_ok(v: &Value) -> Result<(), String> {
    tags_of(Some(v)).map(|_| ())
}

const SHAPE_KEYS: &[&str] =
    &["sprite", "rect", "circle", "text", "model", "color", "scale", "flip", "fit", "yaw", "pitch", "roll", "elevation", "tint", "light"];

fn color_of(ctx: &mut Ctx, path: &str, o: &Map<String, Value>, default: Color) -> Color {
    match o.get("color") {
        None => default,
        Some(v) => match color_ok(v) {
            Ok(()) => parse_color(v.as_str().unwrap_or("")).unwrap_or(default),
            Err(e) => {
                ctx.err(format!("{path}.color"), e);
                default
            }
        },
    }
}

fn scale_of(ctx: &mut Ctx, path: &str, o: &Map<String, Value>) -> u32 {
    match o.get("scale") {
        None => 1,
        Some(v) => match v.as_u64().filter(|s| (1..=16).contains(s)) {
            Some(s) => s as u32,
            None => {
                ctx.err(format!("{path}.scale"), format!("expected a whole number from 1 to 16, got {}", describe_value(v)));
                1
            }
        },
    }
}

fn flip_of(ctx: &mut Ctx, path: &str, o: &Map<String, Value>) -> Flip {
    match o.get("flip") {
        None => Flip::None,
        Some(v) => match v.as_str() {
            Some("x") => Flip::X,
            Some("auto") => Flip::Auto,
            _ => {
                ctx.err(
                    format!("{path}.flip"),
                    format!("expected \"x\" (always mirrored) or \"auto\" (mirrored while it last moved left), got {}", describe_value(v)),
                );
                Flip::None
            }
        },
    }
}

fn text_size(text: &str, scale: u32) -> [f32; 2] {
    let n = text.chars().count().max(1) as f32;
    [n * 6.0 * scale as f32, 7.0 * scale as f32]
}

fn parse_shape(ctx: &mut Ctx, path: &str, v: &Value, sprites: &[Sprite], models: &[crate::game3d::Model], names: &Names) -> (Shape, [f32; 2]) {
    let Some(o) = ctx.obj(path, v) else { return (Shape::None, [8.0, 8.0]) };
    check_keys(&mut ctx.errs, path, o, SHAPE_KEYS);
    let kinds: Vec<&str> = ["sprite", "rect", "circle", "text", "model"].into_iter().filter(|k| o.contains_key(*k)).collect();
    if kinds.len() != 1 {
        ctx.err(
            path,
            format!(
                "give exactly one of sprite, rect, circle, text, model (found {})",
                if kinds.is_empty() { "none".to_string() } else { kinds.join(" and ") }
            ),
        );
        return (Shape::None, [8.0, 8.0]);
    }
    match kinds[0] {
        "model" => match crate::game3d::parse_model_shape(ctx, path, o, models, names) {
            Some((ms, fit)) => (Shape::Model(ms), fit),
            None => (Shape::None, [48.0, 48.0]),
        },
        "sprite" => {
            let name = o["sprite"].as_str().unwrap_or("");
            let scale = scale_of(ctx, path, o);
            match sprites.iter().position(|s| s.name == name) {
                Some(i) => (
                    Shape::Sprite { sprite: i, scale, flip: flip_of(ctx, path, o) },
                    [(sprites[i].w as u32 * scale) as f32, (sprites[i].h as u32 * scale) as f32],
                ),
                None => {
                    ctx.err(format!("{path}.sprite"), format!("no sprite `{name}`{}", Ctx::near(name, sprites.iter().map(|s| s.name.clone()))));
                    (Shape::None, [8.0, 8.0])
                }
            }
        }
        "rect" => match pair(&o["rect"]).filter(|p| p[0] > 0.0 && p[1] > 0.0) {
            Some(p) => (Shape::Rect { color: color_of(ctx, path, o, [255, 255, 255, 255]) }, p),
            None => {
                ctx.err(format!("{path}.rect"), format!("expected [width, height], both above 0, got {}", describe_value(&o["rect"])));
                (Shape::None, [8.0, 8.0])
            }
        },
        "circle" => match num(&o["circle"]).filter(|r| *r > 0.0) {
            Some(r) => (Shape::Circle { color: color_of(ctx, path, o, [255, 255, 255, 255]) }, [r as f32 * 2.0, r as f32 * 2.0]),
            None => {
                ctx.err(format!("{path}.circle"), format!("expected a radius above 0, got {}", describe_value(&o["circle"])));
                (Shape::None, [8.0, 8.0])
            }
        },
        _ => {
            let text = o["text"].as_str().unwrap_or("").to_string();
            let scale = scale_of(ctx, path, o);
            let size = text_size(&text, scale);
            (Shape::Text { text, color: color_of(ctx, path, o, [255, 255, 255, 255]), scale }, size)
        }
    }
}

const BODY_FIELDS: &[Field] = &[
    req("type", Ty::OneOf(&["dynamic", "static"]), "dynamic: gravity and being stopped by `collide` tags; static: never moves"),
    opt("gravity", Ty::Num(Some((-5000.0, 5000.0))), "px/s^2, default 0"),
    opt("bounce", Ty::Num(Some((0.0, 1.0))), "speed kept when it hits something solid, default 0"),
    opt("friction", Ty::Num(Some((0.0, 1.0))), "fraction of horizontal speed lost per tick while on the ground (0.1 slides, 1 stops at once), default 0"),
    opt("drag", Ty::Num(Some((0.0, 1.0))), "fraction of speed lost per tick everywhere, default 0"),
    opt("max_fall", Ty::Num(Some((1.0, 5000.0))), "terminal fall speed, default 600"),
];

fn parse_body(ctx: &mut Ctx, path: &str, v: &Value) -> Option<Body> {
    let o = ctx.obj(path, v)?;
    let before = ctx.errs.len();
    check_fields(&mut ctx.errs, path, o, BODY_FIELDS);
    if ctx.errs.len() > before {
        return None;
    }
    let f = |k: &str, d: f64| o.get(k).and_then(num).unwrap_or(d) as f32;
    Some(Body {
        kind: if o["type"].as_str() == Some("static") { BodyKind::Static } else { BodyKind::Dynamic },
        gravity: f("gravity", 0.0),
        bounce: f("bounce", 0.0),
        friction: f("friction", 0.0),
        drag: f("drag", 0.0),
        max_fall: f("max_fall", 600.0),
    })
}

const KEYS_FIELDS: &[Field] = &[
    req("mode", Ty::OneOf(&["topdown", "platformer"]), "topdown: four directions; platformer: left/right and jump with gravity from the body"),
    req("speed", Ty::Num(Some((1.0, 2000.0))), "px/s"),
    opt("jump", Ty::Num(Some((1.0, 2000.0))), "jump speed, px/s (platformer)"),
];
const CHASE_FIELDS: &[Field] = &[req("target", Ty::Str, "the tag to chase"), req("speed", Ty::Num(Some((1.0, 2000.0))), "px/s")];
const WANDER_FIELDS: &[Field] =
    &[req("speed", Ty::Num(Some((1.0, 2000.0))), "px/s"), opt("turn", Ty::Num(Some((0.05, 60.0))), "seconds between turns, default 1")];
const PATROL_FIELDS: &[Field] = &[
    req("axis", Ty::OneOf(&["x", "y"]), ""),
    req("range", Ty::Num(Some((1.0, 5000.0))), "how far each way from the start, px"),
    req("speed", Ty::Num(Some((1.0, 2000.0))), "px/s"),
];

fn parse_move(ctx: &mut Ctx, path: &str, v: &Value) -> Move {
    let Some(o) = ctx.obj(path, v) else { return Move::None };
    check_keys(&mut ctx.errs, path, o, MOVERS);
    let found: Vec<&String> = o.keys().filter(|k| MOVERS.contains(&k.as_str())).collect();
    if found.len() != 1 {
        ctx.err(path, format!("give exactly one mover ({}), found {}", MOVERS.join(", "), found.len()));
        return Move::None;
    }
    let key = found[0].as_str();
    let sub = format!("{path}.{key}");
    let val = &o[key];
    let before = ctx.errs.len();
    let mover = match key {
        "keys" => ctx.obj(&sub, val).map(|m| {
            check_fields(&mut ctx.errs, &sub, m, KEYS_FIELDS);
            Move::Keys {
                mode: if m.get("mode").and_then(Value::as_str) == Some("platformer") { KeyMode::Platformer } else { KeyMode::TopDown },
                speed: m.get("speed").and_then(num).unwrap_or(0.0) as f32,
                jump: m.get("jump").and_then(num).unwrap_or(0.0) as f32,
            }
        }),
        "pointer" => match val.as_str() {
            Some("x") => Some(Move::Pointer { x: true, y: false }),
            Some("y") => Some(Move::Pointer { x: false, y: true }),
            Some("xy") => Some(Move::Pointer { x: true, y: true }),
            _ => {
                ctx.err(&sub, format!("expected \"x\", \"y\" or \"xy\" (the axis to follow the pointer on), got {}", describe_value(val)));
                None
            }
        },
        "chase" => ctx.obj(&sub, val).map(|m| {
            check_fields(&mut ctx.errs, &sub, m, CHASE_FIELDS);
            Move::Chase { target: m.get("target").and_then(Value::as_str).unwrap_or("").to_string(), speed: m.get("speed").and_then(num).unwrap_or(0.0) as f32 }
        }),
        "drift" => match pair(val) {
            Some(v) => Some(Move::Drift { v }),
            None => {
                ctx.err(&sub, format!("expected [vx, vy] in px/s, got {}", describe_value(val)));
                None
            }
        },
        "wander" => ctx.obj(&sub, val).map(|m| {
            check_fields(&mut ctx.errs, &sub, m, WANDER_FIELDS);
            Move::Wander { speed: m.get("speed").and_then(num).unwrap_or(0.0) as f32, turn: m.get("turn").and_then(num).unwrap_or(1.0) as f32 }
        }),
        _ => ctx.obj(&sub, val).map(|m| {
            check_fields(&mut ctx.errs, &sub, m, PATROL_FIELDS);
            Move::Patrol {
                x: m.get("axis").and_then(Value::as_str) == Some("x"),
                range: m.get("range").and_then(num).unwrap_or(0.0) as f32,
                speed: m.get("speed").and_then(num).unwrap_or(0.0) as f32,
            }
        }),
    };
    if ctx.errs.len() > before {
        return Move::None;
    }
    mover.unwrap_or(Move::None)
}

const EMIT_FIELDS: &[Field] = &[
    req("rate", Ty::Num(Some((0.1, 500.0))), "particles per second"),
    opt("life", Ty::Custom(range_or_num), "seconds, a number or [min, max], default 0.5"),
    opt("speed", Ty::Custom(range_or_num), "px/s, a number or [min, max], default 20"),
    opt("angle", Ty::Range, "degrees [min, max]: 0 is right, 90 is down; default [0, 360]"),
    opt("color", Ty::Custom(color_ok), "default white"),
    opt("size", Ty::Num(Some((1.0, 16.0))), "px, default 1"),
    opt("gravity", Ty::Num(Some((-2000.0, 2000.0))), "px/s^2, default 0"),
];

fn range_or_num(v: &Value) -> Result<(), String> {
    if num(v).is_some() || Ty::Range.check(v, "").is_ok() {
        Ok(())
    } else {
        Err(format!("expected a number or [min, max] with min <= max, got {}", describe_value(v)))
    }
}

fn range_of(o: &Map<String, Value>, key: &str, default: [f32; 2]) -> [f32; 2] {
    match o.get(key) {
        Some(v) if num(v).is_some() => [num(v).unwrap_or(0.0) as f32; 2],
        Some(v) => pair(v).unwrap_or(default),
        None => default,
    }
}

fn parse_emit(ctx: &mut Ctx, path: &str, v: &Value) -> Option<Emitter> {
    let o = ctx.obj(path, v)?;
    let before = ctx.errs.len();
    check_fields(&mut ctx.errs, path, o, EMIT_FIELDS);
    if ctx.errs.len() > before {
        return None;
    }
    Some(Emitter {
        rate: o.get("rate").and_then(num).unwrap_or(1.0) as f32,
        life: range_of(o, "life", [0.5, 0.5]),
        speed: range_of(o, "speed", [20.0, 20.0]),
        angle: range_of(o, "angle", [0.0, 360.0]),
        color: o.get("color").and_then(Value::as_str).and_then(parse_color).unwrap_or([255, 255, 255, 255]),
        size: o.get("size").and_then(num).unwrap_or(1.0) as f32,
        gravity: o.get("gravity").and_then(num).unwrap_or(0.0) as f32,
    })
}

pub(crate) struct Names<'a> {
    pub(crate) vars: &'a [String],
    pub(crate) n_declared: usize,
    pub(crate) sounds: &'a [SoundDef],
    pub(crate) prefabs: &'a [String],
    pub(crate) ids: &'a [String],
    pub(crate) tags: &'a [String],
}

fn expr(ctx: &mut Ctx, path: &str, src: &str, names: &Names) -> Option<Expr> {
    match rules_expr::parse(src, names.vars) {
        Ok(e) => Some(e),
        Err(e) => {
            ctx.err(path, format!("`{src}`: {e}"));
            None
        }
    }
}

pub(crate) fn val(ctx: &mut Ctx, path: &str, v: &Value, names: &Names) -> Option<Val> {
    match v {
        Value::Number(n) => n.as_f64().filter(|x| x.is_finite()).map(Val::Num),
        Value::Bool(b) => Some(Val::Num(f64::from(*b))),
        Value::String(s) => expr(ctx, path, s, names).map(Val::Expr),
        other => {
            ctx.err(path, format!("expected a number, true/false or an expression string like \"score + 1\", got {}", describe_value(other)));
            None
        }
    }
}

fn coord(ctx: &mut Ctx, path: &str, v: &Value, names: &Names) -> Option<Coord> {
    if let Some(a) = v.as_array() {
        if let [lo, hi] = a.as_slice() {
            if let (Some(lo), Some(hi)) = (num(lo), num(hi)) {
                if lo <= hi {
                    return Some(Coord::Range(lo as f32, hi as f32));
                }
            }
        }
        ctx.err(path, format!("a range is [min, max] with min <= max, got {}", describe_value(v)));
        return None;
    }
    val(ctx, path, v, names).map(Coord::At)
}

fn place(ctx: &mut Ctx, path: &str, v: &Value, names: &Names) -> Option<Place> {
    match v {
        Value::String(s) => match s.as_str() {
            "self" => Some(Place::Own),
            "other" => Some(Place::Other),
            "pointer" => Some(Place::Pointer),
            _ => {
                ctx.err(path, format!("`{s}` is not a place: use \"self\", \"other\", \"pointer\", [x, y] or {{\"x\": [min, max], \"y\": [min, max]}}"));
                None
            }
        },
        Value::Array(a) if a.len() == 2 => Some(Place::Xy(coord(ctx, &format!("{path}[0]"), &a[0], names)?, coord(ctx, &format!("{path}[1]"), &a[1], names)?)),
        Value::Object(o) => {
            check_keys(&mut ctx.errs, path, o, &["x", "y"]);
            let (x, y) = (o.get("x")?, o.get("y")?);
            Some(Place::Xy(coord(ctx, &format!("{path}.x"), x, names)?, coord(ctx, &format!("{path}.y"), y, names)?))
        }
        other => {
            ctx.err(
                path,
                format!(
                    "expected a place (\"self\", \"other\", \"pointer\", [x, y] or {{\"x\": [min, max], \"y\": [min, max]}}), got {}",
                    describe_value(other)
                ),
            );
            None
        }
    }
}

fn target(ctx: &mut Ctx, path: &str, v: &Value, names: &Names) -> Option<Target> {
    let Some(s) = v.as_str() else {
        ctx.err(path, format!("expected \"self\", \"other\", \"tag:NAME\" or \"id:NAME\", got {}", describe_value(v)));
        return None;
    };
    match s {
        "self" => Some(Target::Own),
        "other" => Some(Target::Other),
        _ => {
            if let Some(t) = s.strip_prefix("tag:") {
                if names.tags.iter().any(|x| x == t) {
                    return Some(Target::Tag(t.to_string()));
                }
                ctx.err(path, format!("no tag `{t}`{}", Ctx::near(t, names.tags.iter().cloned())));
            } else if let Some(i) = s.strip_prefix("id:") {
                if names.ids.iter().any(|x| x == i) {
                    return Some(Target::Id(i.to_string()));
                }
                ctx.err(path, format!("no scene id `{i}`{}", Ctx::near(i, names.ids.iter().cloned())));
            } else {
                ctx.err(path, format!("`{s}` is not a target: use \"self\", \"other\", \"tag:NAME\" or \"id:NAME\""));
            }
            None
        }
    }
}

/// Every action a rule or button can do.
pub const ACT_KEYS: &[&str] =
    &["set", "add", "emit", "spawn", "destroy", "play", "music", "burst", "shake", "end", "restart", "reset_save", "velocity", "teleport"];

fn declared(ctx: &mut Ctx, path: &str, name: &str, names: &Names) -> Option<usize> {
    match names.vars.iter().position(|v| v == name) {
        Some(i) if i < names.n_declared => Some(i),
        Some(_) => {
            ctx.err(
                path,
                format!("`{name}` is built in and cannot be assigned: assign one of your own variables ({})", names.vars[..names.n_declared].join(", ")),
            );
            None
        }
        None => {
            ctx.err(path, format!("no variable `{name}`{} — declare it in `vars`", Ctx::near(name, names.vars[..names.n_declared].iter().cloned())));
            None
        }
    }
}

const SPAWN_FIELDS: &[Field] = &[
    req("prefab", Ty::Str, "the prefab to create"),
    opt("at", Ty::Custom(any_ok), "\"self\" | \"other\" | \"pointer\" | [x, y] | { \"x\": [min, max], \"y\": [min, max] } (default: self)"),
    opt("vel", Ty::Custom(any_ok), "[vx, vy], each a number, expression or [min, max]"),
    opt("count", Ty::Custom(count_ok), "how many, default 1"),
];
const BURST_FIELDS: &[Field] = &[
    opt("at", Ty::Custom(any_ok), "where (default: self)"),
    opt("n", Ty::Custom(count_ok), "particles, default 12"),
    opt("color", Ty::Custom(color_ok), "default white"),
    opt("speed", Ty::Custom(range_or_num), "px/s, default 40"),
    opt("life", Ty::Custom(range_or_num), "seconds, default 0.5"),
    opt("size", Ty::Num(Some((1.0, 16.0))), "px, default 2"),
    opt("gravity", Ty::Num(Some((-2000.0, 2000.0))), "px/s^2, default 0"),
];

fn any_ok(_: &Value) -> Result<(), String> {
    Ok(())
}
fn count_ok(v: &Value) -> Result<(), String> {
    v.as_u64().filter(|n| (1..=500).contains(n)).map(|_| ()).ok_or_else(|| format!("expected a whole number from 1 to 500, got {}", describe_value(v)))
}

fn parse_actions(ctx: &mut Ctx, path: &str, v: &Value, names: &Names, in_button: bool) -> Vec<Act> {
    let Some(list) = v.as_array() else {
        ctx.err(path, format!("`do` must be a list of actions, got {}", describe_value(v)));
        return Vec::new();
    };
    if list.is_empty() {
        ctx.err(path, "`do` is empty: a rule that does nothing proves nothing (actions: set, add, emit, spawn, destroy, play, music, burst, shake, end, restart, reset_save, velocity, teleport)");
    }
    let mut out = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let p = format!("{path}[{i}]");
        let Some(o) = ctx.obj(&p, item) else { continue };
        if o.len() != 1 {
            ctx.err(&p, format!("an action is an object with exactly one key (one of {}), found {} keys", ACT_KEYS.join(", "), o.len()));
            continue;
        }
        let (key, arg) = o.iter().next().expect("one key");
        let ap = format!("{p}.{key}");
        let act = match key.as_str() {
            "set" | "add" => match arg.as_array().map(Vec::as_slice) {
                Some([Value::String(var), value]) => {
                    let ix = declared(ctx, &ap, var, names);
                    let v = val(ctx, &ap, value, names);
                    match (ix, v) {
                        (Some(ix), Some(v)) => Some(if key == "set" { Act::Set(ix, v) } else { Act::Add(ix, v) }),
                        _ => None,
                    }
                }
                _ => {
                    ctx.err(&ap, format!("expected [\"variable\", value], like [\"score\", 1], got {}", describe_value(arg)));
                    None
                }
            },
            "emit" => arg.as_str().map(|s| Act::Emit(s.to_string())).or_else(|| {
                ctx.err(&ap, format!("expected an event name, got {}", describe_value(arg)));
                None
            }),
            "spawn" => ctx.obj(&ap, arg).and_then(|o| {
                let before = ctx.errs.len();
                check_fields(&mut ctx.errs, &ap, o, SPAWN_FIELDS);
                if ctx.errs.len() > before {
                    return None;
                }
                let name = o["prefab"].as_str().unwrap_or("");
                let Some(prefab) = names.prefabs.iter().position(|p| p == name) else {
                    ctx.err(format!("{ap}.prefab"), format!("no prefab `{name}`{}", Ctx::near(name, names.prefabs.iter().cloned())));
                    return None;
                };
                let at = match o.get("at") {
                    Some(v) => place(ctx, &format!("{ap}.at"), v, names)?,
                    None => Place::Own,
                };
                let vel = match o.get("vel") {
                    Some(Value::Array(a)) if a.len() == 2 => {
                        Some([coord(ctx, &format!("{ap}.vel[0]"), &a[0], names)?, coord(ctx, &format!("{ap}.vel[1]"), &a[1], names)?])
                    }
                    Some(other) => {
                        ctx.err(format!("{ap}.vel"), format!("expected [vx, vy], got {}", describe_value(other)));
                        return None;
                    }
                    None => None,
                };
                Some(Act::Spawn { prefab, at, vel, count: o.get("count").and_then(Value::as_u64).unwrap_or(1) as u32 })
            }),
            "destroy" => target(ctx, &ap, arg, names).map(Act::Destroy),
            "play" => match arg.as_str() {
                Some(s) => match names.sounds.iter().position(|x| x.name == s) {
                    Some(i) => Some(Act::Play(i)),
                    None => {
                        ctx.err(&ap, format!("no sound `{s}`{} — define it in `sounds`", Ctx::near(s, names.sounds.iter().map(|x| x.name.clone()))));
                        None
                    }
                },
                None => {
                    ctx.err(&ap, format!("expected a sound name, got {}", describe_value(arg)));
                    None
                }
            },
            "music" => match arg.as_str() {
                Some("on") => Some(Act::Music(MusicCmd::On)),
                Some("off") => Some(Act::Music(MusicCmd::Off)),
                Some("toggle") => Some(Act::Music(MusicCmd::Toggle)),
                _ => {
                    ctx.err(&ap, format!("expected \"on\", \"off\" or \"toggle\", got {}", describe_value(arg)));
                    None
                }
            },
            "burst" => ctx.obj(&ap, arg).and_then(|o| {
                let before = ctx.errs.len();
                check_fields(&mut ctx.errs, &ap, o, BURST_FIELDS);
                if ctx.errs.len() > before {
                    return None;
                }
                let at = match o.get("at") {
                    Some(v) => place(ctx, &format!("{ap}.at"), v, names)?,
                    None => Place::Own,
                };
                Some(Act::Burst {
                    at,
                    n: o.get("n").and_then(Value::as_u64).unwrap_or(12) as u32,
                    color: o.get("color").and_then(Value::as_str).and_then(parse_color).unwrap_or([255, 255, 255, 255]),
                    speed: range_of(o, "speed", [40.0, 40.0]),
                    life: range_of(o, "life", [0.5, 0.5]),
                    size: o.get("size").and_then(num).unwrap_or(2.0) as f32,
                    gravity: o.get("gravity").and_then(num).unwrap_or(0.0) as f32,
                })
            }),
            "shake" => num(arg).filter(|s| (0.0..=50.0).contains(s)).map(|s| Act::Shake(s as f32)).or_else(|| {
                ctx.err(&ap, format!("expected an amount from 0 to 50 px, got {}", describe_value(arg)));
                None
            }),
            "end" => match arg.as_str() {
                Some("win") => Some(Act::End(Outcome::Win)),
                Some("lose") => Some(Act::End(Outcome::Lose)),
                _ => {
                    ctx.err(&ap, format!("expected \"win\" or \"lose\", got {}", describe_value(arg)));
                    None
                }
            },
            "restart" => (arg == &Value::Bool(true)).then_some(Act::Restart).or_else(|| {
                ctx.err(&ap, "write {\"restart\": true}");
                None
            }),
            "reset_save" => (arg == &Value::Bool(true)).then_some(Act::ResetSave).or_else(|| {
                ctx.err(&ap, "write {\"reset_save\": true}");
                None
            }),
            "velocity" => ctx.obj(&ap, arg).and_then(|o| {
                check_keys(&mut ctx.errs, &ap, o, &["target", "v"]);
                let t = target(ctx, &format!("{ap}.target"), o.get("target")?, names)?;
                let a = o.get("v")?.as_array().filter(|a| a.len() == 2)?;
                Some(Act::Velocity(t, [val(ctx, &format!("{ap}.v[0]"), &a[0], names)?, val(ctx, &format!("{ap}.v[1]"), &a[1], names)?]))
            }),
            "teleport" => ctx.obj(&ap, arg).and_then(|o| {
                check_keys(&mut ctx.errs, &ap, o, &["target", "to"]);
                let t = target(ctx, &format!("{ap}.target"), o.get("target")?, names)?;
                Some(Act::Teleport(t, place(ctx, &format!("{ap}.to"), o.get("to")?, names)?))
            }),
            other => {
                let near = crate::suggest::suggest(other, ACT_KEYS.iter().copied());
                ctx.err(
                    &p,
                    format!(
                        "unknown action `{other}`{} (actions: {})",
                        near.first().map(|n| format!(" — did you mean `{n}`?")).unwrap_or_default(),
                        ACT_KEYS.join(", ")
                    ),
                );
                None
            }
        };
        if let Some(a) = act {
            if in_button && matches!(a, Act::Destroy(Target::Other)) {
                ctx.err(&ap, "a button has no `other`: target \"self\" is the button's rule context");
            }
            out.push(a);
        }
    }
    out
}

fn parse_when(ctx: &mut Ctx, path: &str, v: &Value, names: &Names) -> Option<When> {
    let o = ctx.obj(path, v)?;
    check_keys(&mut ctx.errs, path, o, TRIGGERS);
    let found: Vec<&String> = o.keys().filter(|k| TRIGGERS.contains(&k.as_str())).collect();
    if found.len() != 1 {
        ctx.err(path, format!("`when` takes exactly one trigger ({}), found {}", TRIGGERS.join(", "), found.len()));
        return None;
    }
    let key = found[0].as_str();
    let arg = &o[key];
    let tag_ok = |ctx: &mut Ctx, p: &str, t: &str| -> bool {
        if names.tags.iter().any(|x| x == t) {
            true
        } else {
            ctx.err(p, format!("no tag `{t}`{} — give a prefab `\"tag\": \"{t}\"`", Ctx::near(t, names.tags.iter().cloned())));
            false
        }
    };
    let ap = format!("{path}.{key}");
    match key {
        "start" => Some(When::Start),
        "every" | "after" => num(arg).filter(|s| *s > 0.0).map(|s| if key == "every" { When::Every(s as f32) } else { When::After(s as f32) }).or_else(|| {
            ctx.err(&ap, format!("expected a number of seconds above 0, got {}", describe_value(arg)));
            None
        }),
        "touch" | "touching" => match arg.as_array().map(Vec::as_slice) {
            Some([Value::String(a), Value::String(b)]) => {
                let ok = tag_ok(ctx, &ap, a) & tag_ok(ctx, &ap, b);
                ok.then(|| if key == "touch" { When::Touch(a.clone(), b.clone()) } else { When::Touching(a.clone(), b.clone()) })
            }
            _ => {
                ctx.err(&ap, format!("expected [\"tagA\", \"tagB\"], got {}", describe_value(arg)));
                None
            }
        },
        "press" => match arg.as_str() {
            Some(a) if ACTIONS.contains(&a) => Some(When::Press(a.to_string())),
            _ => {
                ctx.err(
                    &ap,
                    format!(
                        "expected an action ({}), got {}{}",
                        ACTIONS.join(", "),
                        describe_value(arg),
                        arg.as_str()
                            .map(|s| Ctx::near(s, ACTIONS.iter().map(|x| x.to_string())).split(" (known").next().unwrap_or("").to_string())
                            .unwrap_or_default()
                    ),
                );
                None
            }
        },
        "click" => match arg.as_str() {
            Some("*") => Some(When::Click("*".into())),
            Some(t) => tag_ok(ctx, &ap, t).then(|| When::Click(t.to_string())),
            None => {
                ctx.err(&ap, format!("expected a tag name (or \"*\" for a click on nothing), got {}", describe_value(arg)));
                None
            }
        },
        "event" => arg.as_str().map(|s| When::Event(s.to_string())).or_else(|| {
            ctx.err(&ap, format!("expected an event name, got {}", describe_value(arg)));
            None
        }),
        _ => match arg.as_str() {
            Some("win") => Some(When::End(Some(Outcome::Win))),
            Some("lose") => Some(When::End(Some(Outcome::Lose))),
            Some("any") => Some(When::End(None)),
            _ => {
                ctx.err(&ap, format!("expected \"win\", \"lose\" or \"any\", got {}", describe_value(arg)));
                None
            }
        },
    }
}

const RULE_FIELDS: &[Field] = &[
    opt("id", Ty::Str, "a name for messages"),
    req("when", Ty::Custom(object_ok), "exactly one of: start, every, after, touch, touching, press, click, event, end"),
    opt("if", Ty::Str, "an expression over the variables, like \"lives > 0\""),
    opt("once", Ty::Bool, "fire at most once per game"),
    opt("cooldown", Ty::Num(Some((0.0, 100_000.0))), "minimum seconds between firings"),
    req("do", Ty::Custom(any_ok), "a list of actions"),
];

fn parse_rule(ctx: &mut Ctx, i: usize, v: &Value, names: &Names) -> Option<Rule> {
    let mut id = format!("rules[{i}]");
    let o = v.as_object()?;
    if let Some(s) = o.get("id").and_then(Value::as_str) {
        id = format!("rules[{i}] ({s})");
    }
    let before = ctx.errs.len();
    check_fields(&mut ctx.errs, &id, o, RULE_FIELDS);
    if ctx.errs.len() > before {
        return None;
    }
    let when = parse_when(ctx, &format!("{id}.when"), &o["when"], names)?;
    let cond = match o.get("if").and_then(Value::as_str) {
        Some(src) => Some(expr(ctx, &format!("{id}.if"), src, names)?),
        None => None,
    };
    let actions = parse_actions(ctx, &format!("{id}.do"), &o["do"], names, false);
    Some(Rule {
        id: o.get("id").and_then(Value::as_str).map_or_else(|| format!("rule {i}"), str::to_string),
        when,
        cond,
        once: o.get("once").and_then(Value::as_bool).unwrap_or(false),
        cooldown: o.get("cooldown").and_then(num).unwrap_or(0.0) as f32,
        actions,
    })
}

const TEXT_FIELDS: &[Field] = &[
    req("text", Ty::Str, "text; {var} shows a variable"),
    req("at", Ty::Nums(&[2]), "[x, y]"),
    opt("color", Ty::Custom(color_ok), "default white"),
    opt("scale", Ty::Custom(scale_ok), "1 to 16, default 1"),
    opt("align", Ty::OneOf(&["left", "center", "right"]), "default left"),
    opt("show", Ty::Str, "an expression: shown only while it holds"),
];
const BAR_FIELDS: &[Field] = &[
    req("var", Ty::Str, "the variable"),
    req("max", Ty::Custom(any_ok), "the full value: a number or an expression"),
    req("at", Ty::Nums(&[2]), "[x, y]"),
    req("size", Ty::Nums(&[2]), "[w, h]"),
    opt("color", Ty::Custom(color_ok), "fill"),
    opt("back", Ty::Custom(color_ok), "empty part"),
];
const PANEL_FIELDS: &[Field] =
    &[req("at", Ty::Nums(&[2]), "[x, y]"), req("size", Ty::Nums(&[2]), "[w, h]"), opt("color", Ty::Custom(color_ok), "default dark")];
const BUTTON_FIELDS: &[Field] = &[
    req("id", Ty::Str, "a name a scenario can click"),
    req("label", Ty::Str, "the text on it"),
    req("at", Ty::Nums(&[2]), "[x, y] top-left"),
    req("size", Ty::Nums(&[2]), "[w, h]"),
    opt("color", Ty::Custom(color_ok), "default grey"),
    opt("key", Ty::Str, "a key code that also presses it, like KeyR or Enter"),
    req("do", Ty::Custom(any_ok), "a list of actions"),
];

fn scale_ok(v: &Value) -> Result<(), String> {
    v.as_u64().filter(|s| (1..=16).contains(s)).map(|_| ()).ok_or_else(|| format!("expected a whole number from 1 to 16, got {}", describe_value(v)))
}

fn parse_widget(ctx: &mut Ctx, i: usize, v: &Value, names: &Names, models: &[crate::game3d::Model], screen: [f32; 2]) -> Option<Widget> {
    let path = format!("ui[{i}]");
    let o = ctx.obj(&path, v)?;
    let kinds: Vec<&str> = ["text", "bar", "panel", "button"].into_iter().filter(|k| o.contains_key(*k)).collect();
    // `text` is both a widget key and a field of itself: a text widget is a flat object, the others hold one object under their kind.
    let flat_text = o.get("text").is_some_and(Value::is_string);
    let show_src = o.get("show").and_then(Value::as_str);
    let show = match show_src {
        Some(s) => Some(expr(ctx, &format!("{path}.show"), s, names)?),
        None => None,
    };
    let before = ctx.errs.len();
    let col = |o: &Map<String, Value>, k: &str, d: Color| o.get(k).and_then(Value::as_str).and_then(parse_color).unwrap_or(d);
    if let Some(vv) = o.get("view3d") {
        check_keys(&mut ctx.errs, &path, o, &["view3d", "show"]);
        let v3 = crate::game3d::parse_view3d(ctx, &format!("{path}.view3d"), vv, models, names, screen)?;
        return Some(Widget { kind: WidgetKind::View3d(v3), show });
    }
    if let Some(mv) = o.get("minimap") {
        check_keys(&mut ctx.errs, &path, o, &["minimap", "show"]);
        let mp = format!("{path}.minimap");
        let inner = ctx.obj(&mp, mv)?;
        check_fields(
            &mut ctx.errs,
            &mp,
            inner,
            &[
                req("at", Ty::Nums(&[2]), "[x, y] top-left"),
                req("size", Ty::Nums(&[2]), "[w, h]"),
                req("colors", Ty::Custom(object_ok), "{ \"player\": \"#ffffff\", \"enemy\": \"#ff4040\" }: a dot for each thing with that tag"),
                opt("background", Ty::Custom(color_ok), "default dark"),
                opt("border", Ty::Custom(color_ok), "default grey"),
                opt("dot", Ty::Num(Some((1.0, 16.0))), "dot side in px, default 2"),
                opt("viewport", Ty::Bool, "outline what the camera shows"),
            ],
        );
        let mut colors = Vec::new();
        if let Some(c) = inner.get("colors").and_then(Value::as_object) {
            for (tag, cv) in c {
                if !names.tags.iter().any(|t| t == tag) {
                    ctx.err(format!("{mp}.colors.{tag}"), format!("no tag `{tag}`{}", Ctx::near(tag, names.tags.iter().cloned())));
                } else if let Some(color) = cv.as_str().and_then(parse_color) {
                    colors.push((tag.clone(), color));
                } else {
                    ctx.err(format!("{mp}.colors.{tag}"), format!("expected a color string like \"#ffcc00\", got {}", describe_value(cv)));
                }
            }
        }
        if ctx.errs.len() > before {
            return None;
        }
        return Some(Widget {
            kind: WidgetKind::Minimap {
                at: pair(&inner["at"])?,
                size: pair(&inner["size"])?,
                colors,
                back: col(inner, "background", [10, 14, 24, 200]),
                border: col(inner, "border", [90, 104, 140, 255]),
                dot: inner.get("dot").and_then(num).unwrap_or(2.0) as f32,
                viewport: inner.get("viewport").and_then(Value::as_bool).unwrap_or(false),
            },
            show,
        });
    }
    let kind = if flat_text {
        check_fields(&mut ctx.errs, &path, o, TEXT_FIELDS);
        if ctx.errs.len() > before {
            return None;
        }
        WidgetKind::Text {
            text: o["text"].as_str().unwrap_or("").to_string(),
            at: pair(&o["at"])?,
            color: col(o, "color", [255, 255, 255, 255]),
            scale: o.get("scale").and_then(Value::as_u64).unwrap_or(1) as u32,
            align: match o.get("align").and_then(Value::as_str) {
                Some("center") => Align::Center,
                Some("right") => Align::Right,
                _ => Align::Left,
            },
        }
    } else {
        check_keys(&mut ctx.errs, &path, o, &["bar", "panel", "button", "show"]);
        if kinds.len() != 1 {
            ctx.err(
                &path,
                format!(
                    "a widget is text ({{\"text\": \"...\", \"at\": [x, y]}}) or one of bar, panel, button; found {}",
                    if kinds.is_empty() { "none".to_string() } else { kinds.join(" and ") }
                ),
            );
            return None;
        }
        let k = kinds[0];
        let kp = format!("{path}.{k}");
        let inner = ctx.obj(&kp, &o[k])?;
        match k {
            "bar" => {
                check_fields(&mut ctx.errs, &kp, inner, BAR_FIELDS);
                if ctx.errs.len() > before {
                    return None;
                }
                let var = declared_or_builtin(ctx, &format!("{kp}.var"), inner["var"].as_str().unwrap_or(""), names)?;
                WidgetKind::Bar {
                    var,
                    max: val(ctx, &format!("{kp}.max"), &inner["max"], names)?,
                    at: pair(&inner["at"])?,
                    size: pair(&inner["size"])?,
                    color: col(inner, "color", [80, 200, 120, 255]),
                    back: col(inner, "back", [30, 34, 44, 255]),
                }
            }
            "panel" => {
                check_fields(&mut ctx.errs, &kp, inner, PANEL_FIELDS);
                if ctx.errs.len() > before {
                    return None;
                }
                WidgetKind::Panel { at: pair(&inner["at"])?, size: pair(&inner["size"])?, color: col(inner, "color", [20, 24, 34, 220]) }
            }
            _ => {
                check_fields(&mut ctx.errs, &kp, inner, BUTTON_FIELDS);
                if ctx.errs.len() > before {
                    return None;
                }
                let actions = parse_actions(ctx, &format!("{kp}.do"), &inner["do"], names, true);
                WidgetKind::Button {
                    id: inner["id"].as_str().unwrap_or("").to_string(),
                    label: inner["label"].as_str().unwrap_or("").to_string(),
                    at: pair(&inner["at"])?,
                    size: pair(&inner["size"])?,
                    color: col(inner, "color", [70, 84, 120, 255]),
                    key: inner.get("key").and_then(Value::as_str).map(str::to_string),
                    actions,
                }
            }
        }
    };
    if ctx.errs.len() > before {
        return None;
    }
    Some(Widget { kind, show })
}

fn declared_or_builtin(ctx: &mut Ctx, path: &str, name: &str, names: &Names) -> Option<usize> {
    match names.vars.iter().position(|v| v == name) {
        Some(i) => Some(i),
        None => {
            ctx.err(path, format!("no variable `{name}`{}", Ctx::near(name, names.vars.iter().cloned())));
            None
        }
    }
}

/// Parses a `{var}` placeholder template and reports unknown variables.
fn check_template(ctx: &mut Ctx, path: &str, text: &str, names: &Names) {
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            ctx.err(path, "an unclosed `{`: write {score} to show a variable");
            return;
        };
        let spec = &after[..close];
        let name = spec.split(':').next().unwrap_or("");
        if !names.vars.iter().any(|v| v == name) {
            ctx.err(path, format!("`{{{spec}}}` names no variable{}", Ctx::near(name, names.vars.iter().cloned())));
        }
        rest = &after[close + 1..];
    }
}

fn step(ctx: &mut Ctx, path: &str, v: &Value, names: &Names) -> Option<Step> {
    let o = ctx.obj(path, v)?;
    let found: Vec<&String> = o.keys().filter(|k| STEP_KEYS.contains(&k.as_str())).collect();
    let mut allowed: Vec<&str> = STEP_KEYS.to_vec();
    allowed.extend(["seconds", "until", "timeout"]);
    check_keys(&mut ctx.errs, path, o, &allowed);
    if found.len() != 1 {
        ctx.err(path, format!("a step has exactly one of {}, found {}", STEP_KEYS.join(", "), found.len()));
        return None;
    }
    let key = found[0].as_str();
    let arg = &o[key];
    let secs = |ctx: &mut Ctx, v: &Value, p: &str| -> Option<f32> {
        num(v).filter(|s| (0.0..=3600.0).contains(s)).map(|s| s as f32).or_else(|| {
            ctx.err(p, format!("expected seconds from 0 to 3600, got {}", describe_value(v)));
            None
        })
    };
    let action = |ctx: &mut Ctx, v: &Value, p: &str| -> Option<String> {
        match v.as_str().filter(|a| ACTIONS.contains(a)) {
            Some(a) => Some(a.to_string()),
            None => {
                ctx.err(p, format!("expected an action ({}), got {}", ACTIONS.join(", "), describe_value(v)));
                None
            }
        }
    };
    match key {
        "wait" => secs(ctx, arg, &format!("{path}.wait")).map(Step::Wait),
        "hold" => {
            let list: Vec<String> = arg.as_array()?.iter().filter_map(|a| action(ctx, a, &format!("{path}.hold"))).collect();
            let s = secs(ctx, o.get("seconds")?, &format!("{path}.seconds"))?;
            Some(Step::Hold(list, s))
        }
        "hold_until" => {
            let list: Vec<String> = arg.as_array()?.iter().filter_map(|a| action(ctx, a, &format!("{path}.hold_until"))).collect();
            let until = expect_one(ctx, &format!("{path}.until"), o.get("until")?, names)?;
            Some(Step::HoldUntil(list, until, o.get("timeout").and_then(num).unwrap_or(10.0) as f32))
        }
        "press" => action(ctx, arg, &format!("{path}.press")).map(Step::Press),
        "click" | "point" => match pair(arg) {
            Some(p) => Some(if key == "click" { Step::Click(p) } else { Step::Point(p) }),
            None => {
                ctx.err(format!("{path}.{key}"), format!("expected [x, y] in screen pixels, got {}", describe_value(arg)));
                None
            }
        },
        "button" => arg.as_str().map(|s| Step::Button(s.to_string())),
        "approach" => {
            let t = arg.as_str()?;
            if !names.tags.iter().any(|x| x == t) {
                ctx.err(format!("{path}.approach"), format!("no tag `{t}`{}", Ctx::near(t, names.tags.iter().cloned())));
                return None;
            }
            Some(Step::Approach(t.to_string(), secs(ctx, o.get("seconds")?, &format!("{path}.seconds"))?))
        }
        _ => {
            let e = expect_one(ctx, &format!("{path}.wait_until"), arg, names)?;
            Some(Step::WaitUntil(e, o.get("timeout").and_then(num).unwrap_or(30.0) as f32))
        }
    }
}

/// A scene id (or, for a destination, `tag:NAME`) that exists.
fn reach_name_ok(ctx: &mut Ctx, path: &str, name: &str, names: &Names, allow_tag: bool) -> bool {
    if let Some(t) = name.strip_prefix("tag:") {
        if !allow_tag {
            ctx.err(path, "the walker is one thing: give its scene id, not a tag");
            return false;
        }
        if !names.tags.iter().any(|x| x == t) {
            ctx.err(path, format!("no tag `{t}`{}", Ctx::near(t, names.tags.iter().cloned())));
            return false;
        }
        return true;
    }
    if !names.ids.iter().any(|x| x == name) {
        ctx.err(path, format!("no scene id `{name}`{}", Ctx::near(name, names.ids.iter().cloned())));
        return false;
    }
    true
}

fn expect_one(ctx: &mut Ctx, path: &str, v: &Value, names: &Names) -> Option<Expect> {
    let o = ctx.obj(path, v)?;
    const KEYS: &[&str] = &[
        "var",
        "ended",
        "not_ended",
        "count",
        "entity",
        "event",
        "sound",
        "hash",
        "reach",
        "from",
        "reachable",
        "eq",
        "ne",
        "gt",
        "gte",
        "lt",
        "lte",
        "near",
        "tol",
        "min",
        "max",
    ];
    check_keys(&mut ctx.errs, path, o, KEYS);
    let cmp = || -> Option<(Cmp, f64)> {
        [("eq", Cmp::Eq), ("ne", Cmp::Ne), ("gt", Cmp::Gt), ("gte", Cmp::Gte), ("lt", Cmp::Lt), ("lte", Cmp::Lte)]
            .iter()
            .find_map(|(k, c)| Some((*c, num(o.get(*k)?)?)))
    };
    let no_cmp = |ctx: &mut Ctx, what: &str| {
        ctx.err(path, format!("{what} needs a comparison: one of eq, ne, gt, gte, lt, lte with a number, like {{\"{what}\": ..., \"gte\": 1}}"));
    };
    if let Some(var) = o.get("var").and_then(Value::as_str) {
        let ix = declared_or_builtin(ctx, &format!("{path}.var"), var, names)?;
        let Some((c, n)) = cmp() else {
            no_cmp(ctx, "var");
            return None;
        };
        return Some(Expect::Var(ix, c, n));
    }
    if let Some(t) = o.get("ended").and_then(Value::as_str) {
        return match t {
            "win" => Some(Expect::Ended(Outcome::Win)),
            "lose" => Some(Expect::Ended(Outcome::Lose)),
            _ => {
                ctx.err(format!("{path}.ended"), format!("expected \"win\" or \"lose\", got `{t}`"));
                None
            }
        };
    }
    if o.get("not_ended") == Some(&Value::Bool(true)) {
        return Some(Expect::NotEnded);
    }
    if let Some(t) = o.get("count").and_then(Value::as_str) {
        if !names.tags.iter().any(|x| x == t) {
            ctx.err(format!("{path}.count"), format!("no tag `{t}`{}", Ctx::near(t, names.tags.iter().cloned())));
            return None;
        }
        let Some((c, n)) = cmp() else {
            no_cmp(ctx, "count");
            return None;
        };
        return Some(Expect::Count(t.to_string(), c, n));
    }
    if let Some(id) = o.get("entity").and_then(Value::as_str) {
        if !names.ids.iter().any(|x| x == id) {
            ctx.err(format!("{path}.entity"), format!("no scene id `{id}`{}", Ctx::near(id, names.ids.iter().cloned())));
            return None;
        }
        let near = pair(o.get("near")?)?;
        return Some(Expect::Near(id.to_string(), near, o.get("tol").and_then(num).unwrap_or(4.0) as f32));
    }
    if let Some(e) = o.get("event").and_then(Value::as_str) {
        let min = o.get("min").and_then(Value::as_u64).unwrap_or(1) as u32;
        return Some(Expect::Event(e.to_string(), min, o.get("max").and_then(Value::as_u64).map(|m| m as u32)));
    }
    if let Some(s) = o.get("sound").and_then(Value::as_str) {
        let Some(i) = names.sounds.iter().position(|x| x.name == s) else {
            ctx.err(format!("{path}.sound"), format!("no sound `{s}`{}", Ctx::near(s, names.sounds.iter().map(|x| x.name.clone()))));
            return None;
        };
        return Some(Expect::Sound(i, o.get("min").and_then(Value::as_u64).unwrap_or(1) as u32));
    }
    if let Some(h) = o.get("hash").and_then(Value::as_str) {
        return Some(Expect::Hash(h.to_string()));
    }
    if let Some(to) = o.get("reach").and_then(Value::as_str) {
        let from = o.get("from").and_then(Value::as_str);
        let (from_ok, to_ok) =
            (from.is_some_and(|f| reach_name_ok(ctx, &format!("{path}.from"), f, names, false)), reach_name_ok(ctx, &format!("{path}.reach"), to, names, true));
        if from.is_none() {
            ctx.err(path, "`reach` needs `from`: the scene id of the walker, like {\"reach\": \"tag:goal\", \"from\": \"p\"}");
        }
        return (from_ok && to_ok)
            .then(|| Expect::Reach(from.unwrap_or("").to_string(), to.to_string(), o.get("reachable").and_then(Value::as_bool).unwrap_or(true)));
    }
    ctx.err(path, "an expectation has one of var, ended, not_ended, count, entity, event, sound, hash, reach (for example {\"var\": \"score\", \"gte\": 10} or {\"ended\": \"win\"})");
    None
}

fn scenario(ctx: &mut Ctx, i: usize, v: &Value, names: &Names) -> Option<Scenario> {
    let path = format!("checks.scenarios[{i}]");
    let o = ctx.obj(&path, v)?;
    check_keys(&mut ctx.errs, &path, o, &["name", "seed", "max_seconds", "script", "expect", "smoke"]);
    let name = o.get("name").and_then(Value::as_str).map_or_else(|| format!("scenario {i}"), str::to_string);
    let mut script = Vec::new();
    match o.get("script") {
        Some(Value::Array(steps)) if !steps.is_empty() => {
            for (j, s) in steps.iter().enumerate() {
                if let Some(st) = step(ctx, &format!("{path}.script[{j}]"), s, names) {
                    script.push(st);
                }
            }
        }
        Some(Value::Array(_)) | None => {
            ctx.err(&path, "needs a non-empty `script`: the steps a player takes (wait, hold, press, click, button, point, approach, wait_until)")
        }
        Some(other) => ctx.err(format!("{path}.script"), format!("expected a list of steps, got {}", describe_value(other))),
    }
    let mut expect = Vec::new();
    match o.get("expect") {
        Some(Value::Array(list)) if !list.is_empty() => {
            for (j, e) in list.iter().enumerate() {
                if let Some(x) = expect_one(ctx, &format!("{path}.expect[{j}]"), e, names) {
                    expect.push(x);
                }
            }
        }
        Some(Value::Array(_)) | None => ctx.err(
            &path,
            "needs a non-empty `expect`: a scenario that asserts nothing proves nothing (for example [{\"var\": \"score\", \"gte\": 10}] or [{\"ended\": \"win\"}])",
        ),
        Some(other) => ctx.err(format!("{path}.expect"), format!("expected a list, got {}", describe_value(other))),
    }
    let max = o.get("max_seconds").map_or(Some(30.0), num);
    let max = match max.filter(|m| (0.1..=3600.0).contains(m)) {
        Some(m) => m as f32,
        None => {
            ctx.err(format!("{path}.max_seconds"), format!("expected seconds from 0.1 to 3600, got {}", describe_value(&o["max_seconds"])));
            30.0
        }
    };
    Some(Scenario {
        name,
        seed: o.get("seed").and_then(Value::as_u64).unwrap_or(1),
        max_seconds: max,
        script,
        expect,
        smoke: o.get("smoke").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// Parses and fully validates a game text.
pub fn parse(text: &str) -> Result<GameDef, Vec<String>> {
    let v: Value = serde_json::from_str(text).map_err(|e| vec![format!("json: not valid JSON: {e} (line {}, column {})", e.line(), e.column())])?;
    let root = v.as_object().ok_or_else(|| vec!["json: a game is an object".to_string()])?;
    let mut ctx = Ctx { errs: Vec::new() };
    check_keys(&mut ctx.errs, "", root, ROOT);
    if root.get("game2d").and_then(Value::as_u64) != Some(GAME_VERSION) {
        ctx.err("game2d", format!("missing or unsupported version: write \"game2d\": {GAME_VERSION}"));
    }
    // identity
    let id = root.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    if !slug_ok(&id) {
        ctx.err(
            "id",
            format!(
                "a game id is 1 to 40 lowercase letters and digits with single hyphens (like \"tiny-station\"), got {}",
                describe_value(root.get("id").unwrap_or(&Value::Null))
            ),
        );
    }
    let title = root.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    if title.is_empty() {
        ctx.err("title", "needs a display title");
    }
    let description = root.get("description").and_then(Value::as_str).unwrap_or("").to_string();
    if description.is_empty() {
        ctx.err("description", "needs a one-sentence description (the catalog shows it)");
    }

    // capabilities
    let (caps, problems) = match root.get("capabilities") {
        Some(c) => caps::parse(c),
        None => (
            None,
            vec![caps::Problem {
                path: "capabilities".into(),
                message:
                    "needs `capabilities`, like {\"presentation\": \"2d\", \"platforms\": [\"web\"], \"networking\": \"offline\", \"input\": [\"keyboard\"]}"
                        .into(),
            }],
        ),
    };
    for p in &problems {
        ctx.errs.push(p.to_string());
    }
    if let Some(c) = &caps {
        if !matches!(c.presentation, caps::Presentation::TwoD | caps::Presentation::Hybrid) {
            ctx.err("capabilities.presentation", "a `game2d` file is a 2D or hybrid game: write \"2d\", or \"hybrid\" if it uses 3D elements (full 3D games are scenes, `red_engine2 describe scene`)");
        }
        for p in caps::check(c) {
            ctx.errs.push(p.to_string());
        }
    }

    // view
    let mut view = View { width: 320, height: 180, background: [16, 20, 28, 255], scale: Scale::Fit, world: (320.0, 180.0), follow: None, lerp: 0.15 };
    match root.get("view") {
        None => ctx.err("view", "needs `view`, like {\"width\": 320, \"height\": 180}: the virtual screen your game is drawn on, scaled to any window"),
        Some(vv) => {
            if let Some(o) = ctx.obj("view", vv) {
                check_fields(
                    &mut ctx.errs,
                    "view",
                    o,
                    &[
                        req("width", Ty::Custom(dim_ok), "virtual pixels, 64 to 1280"),
                        req("height", Ty::Custom(dim_ok), "virtual pixels, 64 to 1280"),
                        opt("background", Ty::Custom(color_ok), ""),
                        opt("scale", Ty::OneOf(&["fit", "integer"]), "fit: any size; integer: whole-number scale for crisp pixels"),
                        opt("world", Ty::Nums(&[2]), "[w, h] of the world when it is bigger than the screen"),
                        opt("camera", Ty::Custom(object_ok), "{ \"follow\": tag, \"lerp\": 0.15 }"),
                        opt("world3d", Ty::Custom(object_ok), "draw the whole world in 3D: { \"pitch\": 55, \"distance\": 150, \"ground\": {...} }"),
                    ],
                );
                view.width = o.get("width").and_then(Value::as_u64).unwrap_or(320) as u32;
                view.height = o.get("height").and_then(Value::as_u64).unwrap_or(180) as u32;
                view.world = (view.width as f32, view.height as f32);
                if let Some(c) = o.get("background").and_then(Value::as_str).and_then(parse_color) {
                    view.background = c;
                }
                if o.get("scale").and_then(Value::as_str) == Some("integer") {
                    view.scale = Scale::Integer;
                }
                if let Some(w) = o.get("world").and_then(pair) {
                    view.world = (w[0].max(view.width as f32), w[1].max(view.height as f32));
                }
                if let Some(c) = o.get("camera").and_then(Value::as_object) {
                    check_keys(&mut ctx.errs, "view.camera", c, &["follow", "lerp"]);
                    view.follow = c.get("follow").and_then(Value::as_str).map(str::to_string);
                    view.lerp = c.get("lerp").and_then(num).unwrap_or(0.15).clamp(0.01, 1.0) as f32;
                }
            }
        }
    }

    // sprites
    let mut sprites = Vec::new();
    if let Some(sv) = root.get("sprites") {
        if let Some(o) = ctx.obj("sprites", sv) {
            for (name, def) in o {
                if crate::fields::is_extension_key(name) {
                    continue;
                }
                if let Some(s) = parse_sprite(&mut ctx, name, def) {
                    sprites.push(s);
                }
            }
        }
    }

    // 3D models (a hybrid game)
    let models = crate::game3d::parse_models(&mut ctx, root, &sprites);

    // sounds and music
    let mut sounds = Vec::new();
    if let Some(sv) = root.get("sounds") {
        if let Some(o) = ctx.obj("sounds", sv) {
            for (name, def) in o {
                if crate::fields::is_extension_key(name) {
                    continue;
                }
                match crate::voice_spec::parse_voice_up_to(def, 6.0) {
                    Ok(_) => sounds.push(SoundDef { name: name.clone(), voice: def.clone() }),
                    Err(es) => {
                        for e in es {
                            ctx.err(format!("sounds.{name}"), e);
                        }
                    }
                }
            }
        }
    }
    let mut music = Vec::new();
    if let Some(mv) = root.get("music") {
        if let Some(o) = ctx.obj("music", mv) {
            for (name, def) in o {
                if crate::fields::is_extension_key(name) {
                    continue;
                }
                match crate::score::parse_score(def) {
                    Ok(_) => music.push((name.clone(), def.clone())),
                    Err(es) => {
                        for e in es {
                            ctx.err(format!("music.{name}"), e);
                        }
                    }
                }
            }
        }
    }

    // variables
    let mut vars: Vec<(String, f64)> = Vec::new();
    if let Some(vv) = root.get("vars") {
        if let Some(o) = ctx.obj("vars", vv) {
            for (name, init) in o {
                if crate::fields::is_extension_key(name) {
                    continue;
                }
                let ident_ok =
                    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !name.starts_with(|c: char| c.is_ascii_digit());
                if !ident_ok {
                    ctx.err(format!("vars.{name}"), "a variable name is letters, digits and _ (not starting with a digit)");
                    continue;
                }
                if BUILTINS.contains(&name.as_str()) {
                    ctx.err(format!("vars.{name}"), format!("`{name}` is built in ({}): choose another name", BUILTINS.join(", ")));
                    continue;
                }
                match init {
                    Value::Number(n) if n.as_f64().is_some_and(f64::is_finite) => vars.push((name.clone(), n.as_f64().unwrap_or(0.0))),
                    Value::Bool(b) => vars.push((name.clone(), f64::from(*b))),
                    other => ctx.err(format!("vars.{name}"), format!("must be a number or true/false, got {}", describe_value(other))),
                }
            }
        }
    }
    let declared_names: Vec<String> = vars.iter().map(|(n, _)| n.clone()).collect();

    // prefabs: pass 1 (names and tags), so rules and spawns can refer to any of them
    let mut prefab_names: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    if let Some(pv) = root.get("prefabs") {
        if let Some(o) = ctx.obj("prefabs", pv) {
            for (name, def) in o {
                if crate::fields::is_extension_key(name) {
                    continue;
                }
                prefab_names.push(name.clone());
                if let Some(d) = def.as_object() {
                    for t in tags_of(d.get("tag")).unwrap_or_default() {
                        if !tags.contains(&t) {
                            tags.push(t);
                        }
                    }
                }
            }
        }
    } else {
        ctx.err("prefabs", "needs `prefabs`: the kinds of thing in your game, like {\"player\": {\"shape\": {\"rect\": [8, 8], \"color\": \"#ffcc00\"}}}");
    }

    // placements (scene and map) before names for ids
    let mut raw_placements: Vec<(String, [f32; 2], Option<String>)> = Vec::new();
    if let Some(sv) = root.get("scene") {
        match sv.as_array() {
            Some(list) => {
                for (i, item) in list.iter().enumerate() {
                    let path = format!("scene[{i}]");
                    let Some(o) = ctx.obj(&path, item) else { continue };
                    check_fields(
                        &mut ctx.errs,
                        &path,
                        o,
                        &[
                            req("prefab", Ty::Str, "the prefab"),
                            req("at", Ty::Nums(&[2]), "[x, y]"),
                            opt("id", Ty::Str, "a unique name for this one"),
                            opt("count", Ty::Custom(count_ok), "how many copies at this spot"),
                        ],
                    );
                    if let (Some(p), Some(at)) = (o.get("prefab").and_then(Value::as_str), o.get("at").and_then(pair)) {
                        let count = o.get("count").and_then(Value::as_u64).unwrap_or(1);
                        for _ in 0..count {
                            raw_placements.push((p.to_string(), at, o.get("id").and_then(Value::as_str).filter(|_| count == 1).map(str::to_string)));
                        }
                    }
                }
            }
            None => ctx.err("scene", format!("expected a list of {{\"prefab\": name, \"at\": [x, y]}}, got {}", describe_value(sv))),
        }
    }
    if let Some(mv) = root.get("map") {
        if let Some(o) = ctx.obj("map", mv) {
            check_fields(
                &mut ctx.errs,
                "map",
                o,
                &[
                    req("tile", Ty::Num(Some((2.0, 256.0))), "tile size, px"),
                    opt("origin", Ty::Nums(&[2]), "[x, y] of the top-left corner, default [0, 0]"),
                    req("rows", Ty::Strs, "one string per row; each character is a legend entry, . is empty"),
                    req("legend", Ty::Custom(object_ok), "{ \"#\": \"wall\" }: a character to a prefab name"),
                ],
            );
            if let (Some(rows), Some(legend)) = (o.get("rows").and_then(Value::as_array), o.get("legend").and_then(Value::as_object)) {
                let tile = o.get("tile").and_then(num).unwrap_or(16.0) as f32;
                let origin = o.get("origin").and_then(pair).unwrap_or([0.0, 0.0]);
                let width = rows.first().and_then(Value::as_str).map_or(0, |r| r.chars().count());
                for (ry, row) in rows.iter().enumerate() {
                    let Some(row) = row.as_str() else { continue };
                    if row.chars().count() != width {
                        ctx.err(format!("map.rows[{ry}]"), format!("is {} wide, the first row is {width}: map rows must be equally wide", row.chars().count()));
                        continue;
                    }
                    for (rx, ch) in row.chars().enumerate() {
                        if ch == '.' || ch == ' ' {
                            continue;
                        }
                        match legend.get(&ch.to_string()).and_then(Value::as_str) {
                            Some(p) => raw_placements.push((p.to_string(), [origin[0] + (rx as f32 + 0.5) * tile, origin[1] + (ry as f32 + 0.5) * tile], None)),
                            None => ctx.err(
                                format!("map.rows[{ry}]"),
                                format!("`{ch}` (column {rx}) is not in `legend`{}", Ctx::near(&ch.to_string(), legend.keys().cloned())),
                            ),
                        }
                    }
                }
            }
        }
    }
    let mut ids: Vec<String> = Vec::new();
    for (_, _, id) in &raw_placements {
        if let Some(id) = id {
            if ids.contains(id) {
                ctx.err("scene", format!("the id `{id}` is used twice: ids are unique"));
            } else {
                ids.push(id.clone());
            }
        }
    }

    // variable names: declared, built-in, count_<tag>, <id>_x / <id>_y
    let n_declared = declared_names.len();
    let mut var_names = declared_names.clone();
    var_names.extend(BUILTINS.iter().map(|s| s.to_string()));
    var_names.extend(tags.iter().map(|t| format!("count_{t}")));
    for id in &ids {
        var_names.push(format!("{id}_x"));
        var_names.push(format!("{id}_y"));
    }
    {
        let mut seen: Vec<&String> = Vec::new();
        for n in &var_names {
            if seen.contains(&n) {
                ctx.err(
                    "vars",
                    format!("the name `{n}` is used twice (declared variables, built-ins, `count_<tag>` and `<id>_x`/`<id>_y` share one namespace)"),
                );
            }
            seen.push(n);
        }
    }

    // prefabs: pass 2
    let pf_names = Names { vars: &var_names, n_declared, sounds: &sounds, prefabs: &prefab_names, ids: &ids, tags: &tags };
    let mut prefabs = Vec::new();
    if let Some(o) = root.get("prefabs").and_then(Value::as_object) {
        for (name, def) in o {
            if crate::fields::is_extension_key(name) {
                continue;
            }
            let path = format!("prefabs.{name}");
            let Some(d) = ctx.obj(&path, def) else { continue };
            check_fields(&mut ctx.errs, &path, d, PREFAB_FIELDS);
            let (shape, shape_size) = match d.get("shape") {
                Some(s) => parse_shape(&mut ctx, &format!("{path}.shape"), s, &sprites, &models, &pf_names),
                None => (Shape::None, [8.0, 8.0]),
            };
            if let Shape::Text { text, .. } = &shape {
                let names = Names { vars: &var_names, n_declared, sounds: &sounds, prefabs: &prefab_names, ids: &ids, tags: &tags };
                check_template(&mut ctx, &format!("{path}.shape.text"), text, &names);
            }
            let size = d.get("size").and_then(pair).unwrap_or(shape_size);
            let body = d.get("body").and_then(|b| parse_body(&mut ctx, &format!("{path}.body"), b));
            let collide: Vec<String> =
                d.get("collide").and_then(Value::as_array).map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect()).unwrap_or_default();
            for (i, t) in collide.iter().enumerate() {
                if !tags.contains(t) {
                    ctx.err(format!("{path}.collide[{i}]"), format!("no tag `{t}`{}", Ctx::near(t, tags.iter().cloned())));
                }
            }
            if !collide.is_empty() && body.is_none() {
                ctx.err(format!("{path}.collide"), "a thing only collides if it has a `body` (`{\"type\": \"dynamic\"}`): add one, or remove `collide`");
            }
            let mv = d.get("move").map_or(Move::None, |m| parse_move(&mut ctx, &format!("{path}.move"), m));
            if let Move::Chase { target, .. } = &mv {
                if !tags.contains(target) {
                    ctx.err(format!("{path}.move.chase.target"), format!("no tag `{target}`{}", Ctx::near(target, tags.iter().cloned())));
                }
            }
            if matches!(mv, Move::Keys { mode: KeyMode::Platformer, .. }) && body.is_none_or(|b| b.gravity <= 0.0) {
                ctx.err(format!("{path}.move.keys"), "a platformer mover needs gravity: give the prefab `\"body\": {\"type\": \"dynamic\", \"gravity\": 600}` and `\"collide\"` the tags it stands on");
            }
            let tag_list = tags_of(d.get("tag")).unwrap_or_default();
            prefabs.push(Prefab {
                name: name.clone(),
                tags: tag_list,
                shape,
                size,
                layer: d.get("layer").and_then(Value::as_i64).unwrap_or(0) as i32,
                body,
                collide,
                mv,
                ttl: d.get("ttl").and_then(num).map(|t| t as f32),
                emit: d.get("emit").and_then(|e| parse_emit(&mut ctx, &format!("{path}.emit"), e)),
                clamp: d.get("clamp").and_then(Value::as_bool).unwrap_or(false),
                hidden: d.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                height3d: d.get("height3d").and_then(num).map(|h| h as f32),
            });
        }
    }

    // placements resolved
    let mut placements = Vec::new();
    for (p, at, id) in &raw_placements {
        match prefab_names.iter().position(|n| n == p) {
            Some(i) => placements.push(Placement { prefab: i, at: *at, id: id.clone() }),
            None => ctx.err("scene", format!("no prefab `{p}`{}", Ctx::near(p, prefab_names.iter().cloned()))),
        }
    }
    if placements.is_empty() {
        ctx.err("scene", "the game starts empty: add a `scene` (a list of placements) or a tile `map`");
    }
    if let Some(f) = &view.follow {
        if !tags.contains(f) {
            ctx.err("view.camera.follow", format!("no tag `{f}`{}", Ctx::near(f, tags.iter().cloned())));
        }
    }

    let names = Names { vars: &var_names, n_declared, sounds: &sounds, prefabs: &prefab_names, ids: &ids, tags: &tags };

    // 3D world view and viewports among the entities
    let world3d = root.get("view").and_then(|v| v.get("world3d")).and_then(|w| crate::game3d::parse_world3d(&mut ctx, w, &names));
    let mut layers3d = Vec::new();
    if let Some(lv) = root.get("layers3d") {
        match lv.as_array() {
            Some(list) => {
                for (i, item) in list.iter().enumerate() {
                    let path = format!("layers3d[{i}]");
                    let Some(o) = ctx.obj(&path, item) else { continue };
                    check_keys(&mut ctx.errs, &path, o, &["layer", "view3d"]);
                    let layer = o.get("layer").and_then(Value::as_i64).unwrap_or(-1) as i32;
                    match o.get("view3d") {
                        Some(vv) => {
                            if let Some(v3) =
                                crate::game3d::parse_view3d(&mut ctx, &format!("{path}.view3d"), vv, &models, &names, [view.width as f32, view.height as f32])
                            {
                                layers3d.push(crate::game3d::Layer3d { layer, view: v3 });
                            }
                        }
                        None => ctx.err(&path, "needs `view3d`: {\"layer\": -5, \"view3d\": {\"camera\": {...}, \"items\": [...]}}"),
                    }
                }
            }
            None => ctx.err("layers3d", format!("expected a list, got {}", describe_value(lv))),
        }
    }

    // persist
    let mut persist = Vec::new();
    if let Some(pv) = root.get("persist") {
        match pv.as_array() {
            Some(list) => {
                for (i, n) in list.iter().enumerate() {
                    match n.as_str().and_then(|s| declared_names.iter().position(|d| d == s)) {
                        Some(ix) => persist.push(ix),
                        None => ctx.err(
                            format!("persist[{i}]"),
                            format!("{} is not a declared variable{}", describe_value(n), Ctx::near(n.as_str().unwrap_or(""), declared_names.iter().cloned())),
                        ),
                    }
                }
            }
            None => ctx.err("persist", format!("expected a list of variable names, got {}", describe_value(pv))),
        }
    }

    // ui
    let mut ui = Vec::new();
    if let Some(uv) = root.get("ui") {
        match uv.as_array() {
            Some(list) => {
                let mut button_ids: Vec<String> = Vec::new();
                for (i, w) in list.iter().enumerate() {
                    if let Some(w) = parse_widget(&mut ctx, i, w, &names, &models, [view.width as f32, view.height as f32]) {
                        if let WidgetKind::Text { text, .. } = &w.kind {
                            check_template(&mut ctx, &format!("ui[{i}].text"), text, &names);
                        }
                        if let WidgetKind::Button { id, .. } = &w.kind {
                            if button_ids.contains(id) {
                                ctx.err(format!("ui[{i}].button.id"), format!("the button id `{id}` is used twice"));
                            }
                            button_ids.push(id.clone());
                        }
                        ui.push(w);
                    }
                }
            }
            None => ctx.err("ui", format!("expected a list of widgets, got {}", describe_value(uv))),
        }
    }

    // rules
    let mut rules = Vec::new();
    if let Some(rv) = root.get("rules") {
        match rv.as_array() {
            Some(list) => {
                for (i, r) in list.iter().enumerate() {
                    if let Some(rule) = parse_rule(&mut ctx, i, r, &names) {
                        rules.push(rule);
                    }
                }
            }
            None => ctx.err("rules", format!("expected a list of rules, got {}", describe_value(rv))),
        }
    }

    // controls: the pad a phone shows (declared, else inferred from what the game reads)
    let controls = match root.get("controls") {
        Some(cv) => {
            let mut errs = Vec::new();
            let c = crate::controls::parse(cv, &prefabs, &rules, &mut errs);
            for e in errs {
                ctx.errs.push(if e.starts_with("controls") { e } else { format!("controls: {e}") });
            }
            c
        }
        None => Some(crate::controls::infer(&prefabs, &rules)),
    };

    // checks
    let mut scenarios = Vec::new();
    let mut browser = Vec::new();
    let mut reach = Vec::new();
    if let Some(cv) = root.get("checks") {
        if let Some(o) = ctx.obj("checks", cv) {
            check_keys(&mut ctx.errs, "checks", o, &["scenarios", "browser", "reach"]);
            if let Some(list) = o.get("reach").and_then(Value::as_array) {
                for (i, r) in list.iter().enumerate() {
                    let path = format!("checks.reach[{i}]");
                    let Some(ro) = ctx.obj(&path, r) else { continue };
                    check_fields(
                        &mut ctx.errs,
                        &path,
                        ro,
                        &[
                            req("from", Ty::Str, "the walker's scene id"),
                            req("to", Ty::Str, "a scene id, or tag:NAME for any thing with that tag"),
                            opt(
                                "open",
                                Ty::Strs,
                                "scene ids or tag:NAMEs assumed gone first (what a rule opens): the analysis asks what the world allows with them removed",
                            ),
                            opt("reachable", Ty::Bool, "false when the walker must NOT be able to get there (default true)"),
                            opt("why", Ty::Str, "what this proves, shown in the report"),
                        ],
                    );
                    let (Some(from), Some(to)) = (ro.get("from").and_then(Value::as_str), ro.get("to").and_then(Value::as_str)) else { continue };
                    let open: Vec<String> =
                        ro.get("open").and_then(Value::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default();
                    let ok = reach_name_ok(&mut ctx, &format!("{path}.from"), from, &names, false)
                        & reach_name_ok(&mut ctx, &format!("{path}.to"), to, &names, true)
                        & open.iter().enumerate().fold(true, |acc, (j, o)| reach_name_ok(&mut ctx, &format!("{path}.open[{j}]"), o, &names, true) & acc);
                    if ok {
                        reach.push(ReachCheck {
                            name: ro.get("why").and_then(Value::as_str).map_or_else(|| format!("{from} -> {to}"), str::to_string),
                            from: from.to_string(),
                            to: to.to_string(),
                            open,
                            reachable: ro.get("reachable").and_then(Value::as_bool).unwrap_or(true),
                        });
                    }
                }
            } else if o.contains_key("reach") {
                ctx.err("checks.reach", "expected a list of reach checks");
            }
            if let Some(list) = o.get("scenarios").and_then(Value::as_array) {
                for (i, s) in list.iter().enumerate() {
                    if let Some(sc) = scenario(&mut ctx, i, s, &names) {
                        scenarios.push(sc);
                    }
                }
                let smokes = scenarios.iter().filter(|s| s.smoke).count();
                if smokes > 1 {
                    ctx.err("checks.scenarios", "only one scenario may be `smoke: true`: it is the playthrough the browser repeats");
                }
            } else if o.contains_key("scenarios") {
                ctx.err("checks.scenarios", "expected a list of scenarios");
            }
            if let Some(list) = o.get("browser").and_then(Value::as_array) {
                for (i, b) in list.iter().enumerate() {
                    let path = format!("checks.browser[{i}]");
                    let Some(bo) = ctx.obj(&path, b) else { continue };
                    check_fields(
                        &mut ctx.errs,
                        &path,
                        bo,
                        &[
                            opt("name", Ty::Str, ""),
                            opt("keys", Ty::Strs, "key codes to hold, like ArrowRight"),
                            opt("click", Ty::Nums(&[2]), "[x, y] in virtual screen pixels"),
                            opt("ms", Ty::Custom(count_ok_ms), "milliseconds to hold the keys, default 400"),
                            req("changes", Ty::Strs, "names (variables, <id>_x, count_<tag>) of which at least one must change"),
                            opt("persists", Ty::Strs, "saved values (a `persist`ed variable, or music_on) that must survive reloading the page"),
                        ],
                    );
                    if !bo.contains_key("keys") && !bo.contains_key("click") {
                        ctx.err(&path, "needs `keys` or `click`: the real input the browser check performs");
                    }
                    let changes: Vec<String> = bo
                        .get("changes")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    for (j, c) in changes.iter().enumerate() {
                        if !var_names.contains(c) {
                            ctx.err(format!("{path}.changes[{j}]"), format!("no variable `{c}`{}", Ctx::near(c, var_names.iter().cloned())));
                        }
                    }
                    let persists: Vec<String> = bo
                        .get("persists")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    for (j, c) in persists.iter().enumerate() {
                        let saved = c == "music_on" || declared_names.iter().position(|d| d == c).is_some_and(|ix| persist.contains(&ix));
                        if !saved {
                            ctx.err(
                                format!("{path}.persists[{j}]"),
                                format!(
                                    "`{c}` is not saved: `persists` names a variable listed in `persist`, or `music_on`{}",
                                    Ctx::near(c, persist.iter().map(|&i| declared_names[i].clone()).chain(["music_on".to_string()]))
                                ),
                            );
                        }
                    }
                    browser.push(BrowserCheck {
                        persists,
                        name: bo.get("name").and_then(Value::as_str).map_or_else(|| format!("browser check {i}"), str::to_string),
                        keys: bo
                            .get("keys")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                            .unwrap_or_default(),
                        click: bo.get("click").and_then(pair),
                        ms: bo.get("ms").and_then(Value::as_u64).unwrap_or(400) as u32,
                        changes,
                    });
                }
            }
        }
    }

    // capabilities versus what the game actually uses
    if let Some(c) = &caps {
        let uses_keys = prefabs.iter().any(|p| matches!(p.mv, Move::Keys { .. }))
            || rules.iter().any(|r| matches!(&r.when, When::Press(_)))
            || ui.iter().any(|w| matches!(&w.kind, WidgetKind::Button { key: Some(_), .. }));
        let uses_pointer = prefabs.iter().any(|p| matches!(p.mv, Move::Pointer { .. }))
            || rules.iter().any(|r| matches!(&r.when, When::Click(_)))
            || ui.iter().any(|w| matches!(&w.kind, WidgetKind::Button { .. }));
        let mut elements: Vec<&str> = Vec::new();
        if prefabs.iter().any(|p| matches!(p.shape, Shape::Model(_))) {
            elements.push("a prefab drawn as a 3D model (`shape.model`)");
        }
        if ui.iter().any(|w| matches!(w.kind, WidgetKind::View3d(_))) {
            elements.push("a `view3d` widget");
        }
        if !layers3d.is_empty() {
            elements.push("`layers3d`");
        }
        if world3d.is_some() {
            elements.push("`view.world3d`");
        }
        if !elements.is_empty() && c.presentation == caps::Presentation::TwoD {
            ctx.err(
                "capabilities.presentation",
                format!(
                    "the game uses 3D elements ({}) but declares \"2d\": declare \"hybrid\" so tools and players know it mixes 2D and 3D",
                    elements.join(", ")
                ),
            );
        }
        if elements.is_empty() && c.presentation == caps::Presentation::Hybrid {
            ctx.err("capabilities.presentation", "declares \"hybrid\" but uses no 3D element (a prefab `shape.model`, a `view3d` widget, `layers3d` or `view.world3d`): declare \"2d\", or add the 3D element that makes it hybrid");
        }
        if uses_keys && !c.input.contains(&Input::Keyboard) {
            ctx.err(
                "capabilities.input",
                "the game reads keys (a `keys` mover, a `press` rule or a button `key`) but `keyboard` is not declared: add it to `input`",
            );
        }
        if uses_pointer && !c.input.contains(&Input::Mouse) && !c.input.contains(&Input::Touch) {
            ctx.err(
                "capabilities.input",
                "the game reads the pointer (a `pointer` mover, a `click` rule or a button) but neither `mouse` nor `touch` is declared: add one to `input`",
            );
        }
        if root.contains_key("controls") && !c.input.contains(&Input::Touch) {
            ctx.err(
                "capabilities.input",
                "`controls` describes the on-screen pad of a phone, but `touch` is not declared: add \"touch\" to `input` (or remove `controls`)",
            );
        }
        if !persist.is_empty() && !c.persistence.contains(&Persistence::Progress) {
            ctx.err("capabilities.persistence", "`persist` saves variables but `progress` is not declared: add \"progress\" to `persistence`");
        }
        let toggles_music = |acts: &[Act]| acts.iter().any(|a| matches!(a, Act::Music(_)));
        if (rules.iter().any(|r| toggles_music(&r.actions))
            || ui.iter().any(|w| matches!(&w.kind, WidgetKind::Button { actions, .. } if toggles_music(actions))))
            && !c.persistence.contains(&Persistence::Settings)
        {
            ctx.err("capabilities.persistence", "the game has a music setting (`music` action) but `settings` is not declared: add \"settings\" to `persistence`, or the setting is lost on reload");
        }
    }

    if !ctx.errs.is_empty() {
        return Err(ctx.errs);
    }
    let caps = caps.expect("capabilities parsed when there are no errors");
    Ok(GameDef {
        id,
        title,
        description,
        caps,
        view,
        sprites,
        sounds,
        music,
        vars,
        var_names,
        persist,
        prefabs,
        placements,
        ui,
        rules,
        tags,
        scenarios,
        browser,
        reach,
        models,
        layers3d,
        world3d,
        controls: controls.expect("controls parsed when there are no errors"),
        rev: revision(text),
    })
}

fn dim_ok(v: &Value) -> Result<(), String> {
    v.as_u64()
        .filter(|n| (64..=MAX_VIEW as u64).contains(n))
        .map(|_| ())
        .ok_or_else(|| format!("expected a whole number of pixels from 64 to {MAX_VIEW}, got {}", describe_value(v)))
}

fn count_ok_ms(v: &Value) -> Result<(), String> {
    v.as_u64().filter(|n| (50..=10_000).contains(n)).map(|_| ()).ok_or_else(|| format!("expected milliseconds from 50 to 10000, got {}", describe_value(v)))
}
