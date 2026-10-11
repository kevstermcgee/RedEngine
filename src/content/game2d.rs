//! A 2D game (`*.game2d.json`, `describe 2d`), including its hybrid 3D parts. Checker: `red2d::game::parse` (field tables in `game.rs`,
//! `game3d.rs`, `effects.rs`, `caps.rs`).

use super::common::Any;
use super::score::Score;
use super::voice::Voice;
use crate::strict_struct;
use std::collections::BTreeMap;

strict_struct! {
    /// A 2D game file.
    pub struct Game2d {
        game2d: Any,
        id: String,
        title: String,
        description: String,
        capabilities: Capabilities,
        view: View,
        /// name -> sprite.
        sprites: BTreeMap<String, Sprite>,
        /// name -> voice.
        sounds: BTreeMap<String, Voice>,
        /// name -> score.
        music: BTreeMap<String, Score>,
        /// name -> number or true/false.
        vars: BTreeMap<String, Any>,
        /// name -> lines of text (`{"pick": table, "by": expr}`).
        tables: BTreeMap<String, Vec<String>>,
        persist: Vec<String>,
        /// name -> prefab.
        prefabs: BTreeMap<String, Prefab>,
        scene: Vec<Placement>,
        map: TileMap,
        ui: Vec<Widget>,
        /// name -> a named group of actions.
        effects: BTreeMap<String, Effect>,
        rules: Vec<Rule>,
        /// name -> 3D model.
        models: BTreeMap<String, Model>,
        layers3d: Vec<Layer3d>,
        checks: Checks,
    }
}

strict_struct! {
    /// What the game declares it uses (`red_engine2 capabilities`).
    pub struct Capabilities { presentation: String, platforms: Vec<String>, networking: String, input: Vec<String>, persistence: Vec<String>, distribution: Any }
}

strict_struct! {
    /// The virtual screen and the camera.
    pub struct View { width: Any, height: Any, background: String, scale: String, world: Any, camera: ViewCamera, world3d: World3d }
}

strict_struct! {
    /// The camera that follows.
    pub struct ViewCamera { follow: String, lerp: Any }
}

strict_struct! {
    /// A sprite: pixel rows (or frames) and a palette.
    pub struct Sprite { rows: Vec<String>, frames: Vec<Vec<String>>, palette: BTreeMap<String, String>, fps: Any, file: String }
}

strict_struct! {
    /// A kind of thing in the game.
    pub struct Prefab {
        tag: Any,
        shape: Shape,
        size: Any,
        layer: Any,
        body: Body,
        collide: Vec<String>,
        r#move: Move,
        ttl: Any,
        emit: Emitter,
        clamp: bool,
        hidden: bool,
        height3d: Any,
    }
}

strict_struct! {
    /// How a thing looks.
    pub struct Shape {
        sprite: String,
        rect: Any,
        circle: Any,
        text: String,
        model: String,
        color: String,
        scale: Any,
        flip: String,
        fit: Any,
        yaw: Any,
        pitch: Any,
        roll: Any,
        elevation: Any,
        tint: Any,
        light: Any,
    }
}

strict_struct! {
    /// Physics.
    pub struct Body { r#type: String, gravity: Any, bounce: Any, friction: Any, drag: Any, max_fall: Any }
}

strict_struct! {
    /// How a thing moves (exactly one mover; the checker says so).
    pub struct Move { keys: KeysMove, pointer: String, chase: ChaseMove, drift: Any, wander: WanderMove, patrol: PatrolMove }
}

strict_struct! {
    /// Moved by the keys.
    pub struct KeysMove { mode: String, speed: Any, jump: Any }
}

strict_struct! {
    /// Chases a tag.
    pub struct ChaseMove { target: String, speed: Any }
}

strict_struct! {
    /// Wanders.
    pub struct WanderMove { speed: Any, turn: Any }
}

strict_struct! {
    /// Patrols along an axis.
    pub struct PatrolMove { axis: String, range: Any, speed: Any }
}

strict_struct! {
    /// A particle emitter.
    pub struct Emitter { rate: Any, life: Any, speed: Any, angle: Any, color: Any, size: Any, gravity: Any }
}

strict_struct! {
    /// A starting thing.
    pub struct Placement { prefab: String, at: Any, id: String, count: Any }
}

strict_struct! {
    /// A tile map.
    pub struct TileMap {
        tile: Any,
        origin: Any,
        rows: Vec<String>,
        /// character -> prefab.
        legend: BTreeMap<String, String>,
    }
}

/// A HUD widget: flat text, or one kind under its key.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)] // a data definition read by the drift guard and the schema, not a hot path
pub enum Widget {
    Text(TextWidget),
    Bar(BarWidget),
    Panel(PanelWidget),
    Button(ButtonWidget),
    View3d(View3dWidget),
    Dialogue(DialogueWidget),
    Minimap(MinimapWidget),
}

/// A text: a template string, or one entry of a string table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum TextSource {
    Template(String),
    Pick(Pick),
}

strict_struct! {
    /// `{"pick": table, "by": expression}`.
    pub struct Pick { pick: String, by: Any }
}

strict_struct! {
    /// Text on the HUD.
    pub struct TextWidget { text: TextSource, at: Any, width: Any, color: String, scale: Any, align: String, show: String }
}

strict_struct! {
    /// `{"bar": {...}}`.
    pub struct BarWidget { bar: Bar, show: String }
}

strict_struct! {
    /// A bar.
    pub struct Bar { var: String, max: Any, at: Any, size: Any, color: String, back: String }
}

strict_struct! {
    /// `{"panel": {...}}`.
    pub struct PanelWidget { panel: Panel, show: String }
}

strict_struct! {
    /// A filled rectangle.
    pub struct Panel { at: Any, size: Any, color: String }
}

strict_struct! {
    /// `{"button": {...}}`.
    pub struct ButtonWidget { button: Button, show: String }
}

strict_struct! {
    /// A button.
    pub struct Button { id: String, label: String, at: Any, size: Any, color: String, key: String, r#do: Vec<Action> }
}

strict_struct! {
    /// `{"view3d": {...}}`.
    pub struct View3dWidget { view3d: View3d, show: String }
}

strict_struct! {
    /// `{"dialogue": {...}}`.
    pub struct DialogueWidget { dialogue: Dialogue, show: String }
}

strict_struct! {
    /// A conversation.
    pub struct Dialogue {
        id: String,
        lines: Vec<DialogueLine>,
        at: Any,
        size: Any,
        color: String,
        back: String,
        accent: String,
        scale: Any,
        advance: String,
        pause: bool,
        done: String,
    }
}

strict_struct! {
    /// One line of a conversation.
    pub struct DialogueLine { text: TextSource, id: String, who: String, emit: String, goto: String, choices: Vec<Choice> }
}

strict_struct! {
    /// One choice.
    pub struct Choice { label: String, emit: String, goto: String }
}

strict_struct! {
    /// `{"minimap": {...}}`.
    pub struct MinimapWidget { minimap: Minimap, show: String }
}

strict_struct! {
    /// A flat map of the world.
    pub struct Minimap {
        at: Any,
        size: Any,
        /// tag -> dot colour.
        colors: BTreeMap<String, String>,
        background: String,
        border: String,
        dot: Any,
        viewport: bool,
    }
}

strict_struct! {
    /// A named group of actions.
    pub struct Effect { params: Any, r#do: Vec<Action>, about: String }
}

strict_struct! {
    /// A rule.
    pub struct Rule { id: String, when: Trigger, r#if: String, once: bool, cooldown: Any, r#do: Vec<Action> }
}

strict_struct! {
    /// A rule's trigger (exactly one; the checker says so).
    pub struct Trigger { start: bool, every: Any, after: Any, touch: Any, touching: Any, press: String, click: String, event: String, end: String }
}

strict_struct! {
    /// An action (one key, or `apply` with its `with`).
    pub struct Action {
        set: Any,
        add: Any,
        emit: String,
        spawn: SpawnAction,
        destroy: String,
        play: String,
        music: String,
        burst: BurstAction,
        shake: Any,
        end: String,
        restart: bool,
        reset_save: bool,
        velocity: VelocityAction,
        teleport: TeleportAction,
        apply: String,
        /// The effect's arguments: name -> value.
        with: BTreeMap<String, Any>,
    }
}

/// Where: `self`, `other`, `pointer`, `[x, y]`, or a random range `{"x": [min, max], "y": [min, max]}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum At {
    Named(String),
    Point(Vec<Any>),
    Range(AtRange),
}

strict_struct! {
    /// A random place.
    pub struct AtRange { x: Any, y: Any }
}

strict_struct! {
    /// `spawn`.
    pub struct SpawnAction { prefab: String, at: At, vel: Any, count: Any }
}

strict_struct! {
    /// `burst`.
    pub struct BurstAction { at: At, n: Any, color: Any, speed: Any, life: Any, size: Any, gravity: Any }
}

strict_struct! {
    /// `velocity`.
    pub struct VelocityAction { target: String, v: Any }
}

strict_struct! {
    /// `teleport`.
    pub struct TeleportAction { target: String, to: At }
}

strict_struct! {
    /// A 3D model: primitives and sprites pressed into voxels.
    pub struct Model { parts: Vec<ModelPart> }
}

strict_struct! {
    /// One part of a model.
    pub struct ModelPart {
        shape: String,
        size: Any,
        radius: Any,
        height: Any,
        base: Any,
        major: Any,
        minor: Any,
        segments: Any,
        sprite: String,
        depth: Any,
        cell: Any,
        at: Any,
        rot: Any,
        color: String,
    }
}

strict_struct! {
    /// A 3D viewport drawn among the entities.
    pub struct Layer3d { layer: Any, view3d: View3d }
}

strict_struct! {
    /// A small 3D scene in a rectangle.
    pub struct View3d { at: Any, size: Any, camera: View3dCamera, background: String, items: Vec<View3dItem>, light: Light3d }
}

strict_struct! {
    /// Its camera.
    pub struct View3dCamera { eye: Any, target: Any, fov: Any, ortho: Any }
}

strict_struct! {
    /// A model placed in a view.
    pub struct View3dItem { model: String, at: Any, yaw: Any, pitch: Any, roll: Any, scale: Any, tint: Any }
}

strict_struct! {
    /// The light of a 3D part.
    pub struct Light3d { dir: Any, ambient: Any, toon: Any }
}

strict_struct! {
    /// The whole world drawn in 3D.
    pub struct World3d { plane: String, distance: Any, pitch: Any, yaw: Any, fov: Any, ground: Ground3d, light: Light3d, sky: Any }
}

strict_struct! {
    /// The ground under a 3D world.
    pub struct Ground3d { color: String, alt: String, tile: Any }
}

strict_struct! {
    /// Headless proof.
    pub struct Checks { scenarios: Vec<Scenario>, reach: Vec<Reach> }
}

strict_struct! {
    /// A scripted run.
    pub struct Scenario { name: String, seed: Any, max_seconds: Any, script: Vec<Step>, expect: Vec<Expect>, smoke: bool }
}

strict_struct! {
    /// One step (exactly one of the step keys, with `seconds` / `until` / `timeout` beside it).
    pub struct Step {
        wait: Any,
        hold: Any,
        hold_until: Any,
        press: String,
        click: Any,
        button: String,
        point: Any,
        approach: String,
        wait_until: Expect,
        seconds: Any,
        until: Expect,
        timeout: Any,
    }
}

strict_struct! {
    /// What must hold.
    pub struct Expect {
        var: String,
        ended: Any,
        not_ended: bool,
        count: String,
        entity: String,
        event: String,
        sound: String,
        hash: String,
        reach: String,
        from: String,
        reachable: bool,
        eq: Any,
        ne: Any,
        gt: Any,
        gte: Any,
        lt: Any,
        lte: Any,
        near: Any,
        tol: Any,
        min: Any,
        max: Any,
    }
}

strict_struct! {
    /// Map analysis: can a walker reach a place.
    pub struct Reach { from: String, to: String, open: Vec<String>, reachable: bool, why: String }
}
