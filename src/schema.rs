//! Scene schema: JSON -> `Scene` with `path: message` errors; runs prefab + wall/fence macro expansion first.

use crate::color::parse_hex_to_linear;
use crate::easing::Ease;
use crate::props::PropKind;
use crate::strict::{self, check_keys};
use crate::track::{Keyframe, Lerp, Track};
use glam::Vec3;
use serde_json::{Map, Value};

/// The scene-format version this engine writes and reads. A scene may say `"schema_version": 1` (omitting it means 1);
/// a newer number is refused with a message instead of half-working. Migration rules: `SPEC.md`, "Versioning".
pub const SCHEMA_VERSION: u32 = 1;

/// Maximum lights evaluated by the shader for one camera. The WGSL `Globals` block must agree.
pub const MAX_LIGHTS: usize = 16;
/// Maximum authored lights in a scene. The renderer selects the nearest point lights for each view.
pub const MAX_SCENE_LIGHTS: usize = 256;

// ---------------------------------------------------------------------------------------------
// Compiled scene (what the renderer actually walks)
// ---------------------------------------------------------------------------------------------

/// Scene-level tuning for the "clarity" post pass (contact ambient occlusion + silhouette
/// outlines, see `shaders/postfx.wgsl`). JSON: `"post": {"ao": 0.8, "outline": 0.5,
/// "ao_radius": 0.55, "enabled": true}` — all optional.
#[derive(Debug, Clone, Copy)]
pub struct PostSettings {
    pub enabled: bool,
    /// Contact-AO strength, 0 (off) .. ~1.5.
    pub ao: f32,
    /// Silhouette outline darkness, 0 (off) .. 1.
    pub outline: f32,
    /// World-space reach of the AO in meters.
    pub ao_radius: f32,
}

impl Default for PostSettings {
    fn default() -> Self {
        PostSettings { enabled: true, ao: 0.9, outline: 0.65, ao_radius: 0.6 }
    }
}

/// A parsed scene: meta, camera, lights, background, post settings and the (macro-expanded) object tree.
#[derive(Debug)]
pub struct Scene {
    pub fps: u32,
    pub duration: f32,
    pub width: u32,
    pub height: u32,
    pub background: Background,
    pub ambient_color: Vec3,
    pub ambient_intensity: f32,
    pub camera: Camera,
    /// Live-player movement/view tuning. Defaults preserve the classic engine feel and 90° FOV.
    pub player: crate::player::PlayerTuning,
    /// Authoritative vertical launch surfaces.
    pub jump_pads: Vec<crate::player::JumpPad>,
    pub post: PostSettings,
    pub lights: Vec<Light>,
    pub objects: Vec<Object>,
    /// The scene's game rules (`vars` + `rules`), validated at parse time; empty when the scene declares none.
    pub rules: crate::sim::rules::RuleSet,
    /// The weapons' numbers (`weapons` block): the starting weapon, the ladder, the bat's damage and the players' ammunition.
    pub weapons: crate::weapons::WeaponConfig,
    /// How fights are paced (`combat` block): respawn delay, spawn policy, spawn protection, health regeneration.
    pub combat: crate::sim::combat_cfg::CombatConfig,
    /// The bots the match wants (`bots` block).
    pub bots: crate::sim::ai::BotsConfig,
    /// The waypoint graph bots route along (`nav` block), if the scene has one.
    pub nav: Option<crate::sim::ai::nav::Nav>,
    /// Which on-screen display the standard client draws (`hud` block; the defaults follow `player.mode`).
    pub hud: crate::hud_config::HudConfig,
    /// What the game says to its player (`ui` block): friendly labels, counters, an objective, a start card and end cards. `None` keeps the generic rules panel.
    pub ui: Option<crate::ui_config::GameUi>,
    /// Whether the standard client starts the built-in music loop (root `"music"`, default true for older maps; new games say `false`;
    /// the `N` key and the pause menu's MUSIC button still toggle it). Not every game needs music: a game whose own audio cues,
    /// tone or pacing it would fight is better off silent (`red_engine2 describe scene` explains this to the next author).
    pub music: bool,
    /// Whether the player carries a toggleable point light (root `"flashlight"`, default false; the `T` key toggles it on/off
    /// once playing). A `Point` light, not a cone: the engine has no spotlight kind (ADR "a player-carried flashlight").
    pub flashlight: bool,
    /// The big line the standard client's death screen shows while waiting to respawn (root `"death_text"`,
    /// default `None` = "ELIMINATED", the arena-shooter wording every map used before this key existed).
    pub death_text: Option<String>,
    /// Whether players (human or bot) can be assigned to team 1 or 2 outside a loadout `shooter` match (root
    /// `"teams"`, default false). A `shooter` scene already has teams regardless of this flag; this is what a
    /// non-shooter game (hide-and-seek roles, capture-the-flag, anything asymmetric) opts into to use `who: team1`/
    /// `who: team2` in its rules and to have the lobby/bot-fill assign players to a side.
    pub teams: bool,
    /// The sky dome and its sun (`sky` block), if the scene has one; without it `background` is a screen-space gradient.
    pub sky: Option<crate::atmosphere::Sky>,
    /// The time-of-day clock (`clock` block), if the scene has one: it drives the sun, moon, stars, sky colours, sun light and ambient from the scene time.
    pub clock: Option<crate::daycycle::Clock>,
    /// An endless generated world (`procgen` block): ground, forests and flowers made from a seed, built around the player as they walk.
    pub procgen: Option<crate::procgen::Config>,
    /// The sounds of the world (`audio` block): nature ambience and a score for each time of day.
    pub audio: Option<crate::ambience::AudioSpec>,
    /// The endless water plane (`ocean` block), if the scene has one.
    pub ocean: Option<crate::atmosphere::Ocean>,
    /// The kart race (`race` block), if the scene has one: its presence turns every player into a kart driver (`sim::kart`, `sim::race`).
    pub race: Option<std::sync::Arc<crate::sim::race::RaceCourse>>,
    /// The loadout shooter (`shooter` block), if the scene has one: a kit of weapons per player, pickups on the map, two teams.
    pub shooter: Option<crate::sim::shooter::ShooterConfig>,
}

/// Sky: a flat color or a vertical gradient.
#[derive(Debug)]
pub enum Background {
    Flat(Vec3),
    Gradient { top: Vec3, bottom: Vec3 },
}

/// The scene camera (also the player's spawn in `re2`): position, target, fov as tracks.
#[derive(Debug)]
pub struct Camera {
    pub fov: Track<f32>,
    pub near: f32,
    pub far: f32,
    pub position: Track<Vec3>,
    pub target: Track<Vec3>,
    pub roll: Track<f32>,
}

/// Point light or the single directional sun.
#[derive(Debug)]
pub enum LightKind {
    Directional { direction: Track<Vec3> },
    Point { position: Track<Vec3>, range: f32 },
}

/// A light in the scene (see `LightKind`).
#[derive(Debug)]
pub struct Light {
    pub id: String,
    pub kind: LightKind,
    pub color: Track<Vec3>,
    pub intensity: Track<f32>,
    pub cast_shadows: bool,
    pub shadow_radius: f32,
    /// World point the shadow map is centered on (default: the origin). Move it onto the middle
    /// of a map that isn't centered at 0,0,0 so the whole thing falls inside the shadow frustum.
    pub shadow_center: Vec3,
    /// `"shadow_follow": true`: the shadow map is centred under the camera instead of on `shadow_center` (snapped to texels so shadows do
    /// not crawl), for open worlds far larger than one shadow map can cover.
    pub shadow_follow: bool,
}

/// Surface material: base color, metallic, roughness, emissive, opacity.
#[derive(Clone, Debug)]
pub struct Material {
    pub color: Track<Vec3>,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
    /// 1 = solid (the default); below 1 the surface is drawn blended over what is behind it (glass, water, a flame's glow) after all
    /// solid objects, back to front, and casts no shadow.
    pub opacity: f32,
}

impl Scene {
    /// The files this scene refers to, as paths resolved against its folder (when it was loaded from one): the score files of its `audio` block. A game that is shipped
    /// without one of these loses it silently, so `tools::publish_check` holds every published playable to this list. Add a field that names a file here, and it is held too.
    pub fn referenced_files(&self) -> Vec<std::path::PathBuf> {
        self.audio.as_ref().map(|a| a.files()).unwrap_or_default()
    }
}

impl Material {
    fn default_gray() -> Self {
        Material { color: Track::constant(Vec3::splat(0.7)), metallic: 0.0, roughness: 0.6, emissive: Vec3::ZERO, opacity: 1.0 }
    }
}

/// The six primitive shapes; only `Box` collides.
#[derive(Debug, Clone, Copy)]
pub enum PrimKind {
    Box { size: Vec3 },
    Sphere { radius: f32 },
    Cylinder { radius: f32, height: f32 },
    Cone { radius: f32, height: f32 },
    Capsule { radius: f32, height: f32 },
    Plane { size: (f32, f32) },
}

impl PrimKind {
    /// Conservative local-space half-extent (the mesh always fits inside it) — used for
    /// bounding boxes, not for rendering.
    pub fn half_extent(&self) -> Vec3 {
        match self {
            PrimKind::Box { size } => *size * 0.5,
            PrimKind::Sphere { radius } => Vec3::splat(*radius),
            PrimKind::Cylinder { radius, height } | PrimKind::Cone { radius, height } | PrimKind::Capsule { radius, height } => {
                Vec3::new(*radius, height * 0.5, *radius)
            }
            PrimKind::Plane { size } => Vec3::new(size.0 * 0.5, 0.02, size.1 * 0.5),
        }
    }
}

/// Joint rotations (as tracks) that pose a `humanoid`.
#[derive(Debug)]
pub struct Pose {
    pub spine: Track<Vec3>,
    pub head: Track<Vec3>,
    pub l_shoulder: Track<Vec3>,
    pub r_shoulder: Track<Vec3>,
    pub l_elbow: Track<f32>,
    pub r_elbow: Track<f32>,
    pub l_hip: Track<Vec3>,
    pub r_hip: Track<Vec3>,
    pub l_knee: Track<f32>,
    pub r_knee: Track<f32>,
}

/// A humanoid figure: height, build, material and pose.
#[derive(Debug)]
pub struct HumanoidDef {
    pub height: f32,
    pub build: f32,
    /// The shirt colour (and finish); skin, hair, trousers and shoes are in `look`.
    pub material: Material,
    /// Non-shirt colours (JSON: optional `skin`, `hair`, `pants`, `shoes` hex strings).
    pub look: crate::characters::HumanLook,
    pub pose: Pose,
}

/// Cheddar-style lab rat (see `crate::characters::rat_parts`): fur `material`, and a gait
/// animated by tracks. Origin at the paws, nose toward local +Z.
#[derive(Debug)]
pub struct RatDef {
    /// Fur colour and finish.
    pub material: Material,
    /// Gait phase in radians.
    pub gait: Track<f32>,
    /// Gait amplitude, 0 (standing) .. 1 (full scurry).
    pub stride: Track<f32>,
    /// Tail/head idle phase in radians.
    pub sway: Track<f32>,
}

/// A prop-hunt prop (see `crate::props`): a schema-level object kind that expands into a
/// handful of primitive parts, the same way `Humanoid` expands into a posed capsule rig,
/// rather than something a map author hand-nests as a `group` of boxes every time.
#[derive(Debug)]
pub struct PropDef {
    pub kind: PropKind,
    pub material: Material,
}

/// A staircase connecting two floor heights. `position.y` is the height of its *bottom* (the
/// floor it starts from); local `+Z` is the run axis, bottom at `-run/2`, top at `+run/2` (only
/// yaw rotation is meaningful, matching every other upright object in the engine). See
/// `geometry::build_stairs_parts` for the stepped visual mesh and `collide::ground_height_at` for
/// how it contributes a smooth walkable ramp despite the visually stepped treads.
#[derive(Debug)]
pub struct StairsDef {
    pub width: f32,
    pub run: f32,
    pub rise: f32,
    pub steps: u32,
    pub material: Material,
}

/// A heightfield terrain (see `crate::terrain`): the shared ground data plus its material (the vertex colours multiply the material colour).
#[derive(Debug)]
pub struct TerrainDef {
    /// The heights, shared with the collision and ground queries built from the scene.
    pub terrain: std::sync::Arc<crate::terrain::Terrain>,
    pub material: Material,
}

/// What an object is: a primitive, `Group`, `Humanoid`, `Rat`, `Prop`, `Stairs`, `Terrain` (macros and prefabs are already expanded away).
#[derive(Debug)]
pub enum ObjectKind {
    Prim(PrimKind),
    Group(Vec<Object>),
    Humanoid(Box<HumanoidDef>),
    Rat(Box<RatDef>),
    Prop(Box<PropDef>),
    Stairs(Box<StairsDef>),
    Terrain(Box<TerrainDef>),
}

/// One scene object: unique id, position/rotation/scale tracks, and its `ObjectKind`.
#[derive(Debug)]
pub struct Object {
    pub id: String,
    pub position: Track<Vec3>,
    pub rotation: Track<Vec3>,
    pub scale: Track<Vec3>,
    pub material: Option<Material>,
    /// `"collide": false` makes the object (and, for a group, everything inside it) walk-through:
    /// no player collider, not standable, no solid volume for the lint tools. Default true.
    pub collide: bool,
    /// Set on the group a prefab instance expands into: which prefab it came from (see [`PrefabTag`]).
    pub prefab: Option<PrefabTag>,
    /// `"movable": true|false` — force this object to be (or not be) a loose, pick-up-able,
    /// physics-driven prop. `None` = decide from what it is (see `crate::physics::classify`).
    pub movable: Option<bool>,
    pub kind: ObjectKind,
}

/// Where a prefab-expanded group came from.
#[derive(Debug, Clone)]
pub struct PrefabTag {
    /// The prefab's catalogue name.
    pub name: String,
    /// `floor`, `wall` or `ceiling` (what it is mounted on); only `floor` things can be loose props.
    pub mount: String,
}

// ---------------------------------------------------------------------------------------------
// Parsing (JSON -> Scene) with precise "path: message" error collection
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Ctx {
    errors: Vec<String>,
    /// The scene's looping axis (`world.wrap`), which a terrain has to tile across.
    wrap: Option<crate::expanse::Wrap>,
}

thread_local! {
    /// The folder a scene's relative asset paths (a terrain `heightmap`) resolve against, set for the duration of [`parse_scene_in`].
    static ASSET_DIR: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) };
}

impl Ctx {
    fn err(&mut self, path: &str, msg: impl std::fmt::Display) {
        self.errors.push(format!("{path}: {msg}"));
    }
}

fn as_f32(v: &Value) -> Option<f32> {
    v.as_f64().map(|x| x as f32)
}

fn as_vec3(v: &Value) -> Result<Vec3, String> {
    let arr = v.as_array().ok_or("must be a [x,y,z] array")?;
    if arr.len() != 3 {
        return Err("must have exactly 3 numbers".to_string());
    }
    let x = as_f32(&arr[0]).ok_or("component must be a number")?;
    let y = as_f32(&arr[1]).ok_or("component must be a number")?;
    let z = as_f32(&arr[2]).ok_or("component must be a number")?;
    Ok(Vec3::new(x, y, z))
}

fn as_scale_vec3(v: &Value) -> Result<Vec3, String> {
    if let Some(n) = as_f32(v) {
        return Ok(Vec3::splat(n));
    }
    as_vec3(v)
}

fn as_color_vec3(v: &Value) -> Result<Vec3, String> {
    let s = v.as_str().ok_or("must be a '#rrggbb' hex string")?;
    parse_hex_to_linear(s)
}

/// Shared engine for every track field: accepts either a bare leaf value or
/// `{"keyframes": [{"t":..,"value":..,"ease":..}, ...]}`.
fn build_track<T: Lerp + Copy>(ctx: &mut Ctx, raw: &Value, path: &str, default: T, parse_leaf: impl Fn(&Value) -> Result<T, String>) -> Track<T> {
    if let Some(obj) = raw.as_object() {
        if let Some(kfs_raw) = obj.get("keyframes") {
            let Some(arr) = kfs_raw.as_array() else {
                ctx.err(&format!("{path}.keyframes"), "must be an array");
                return Track::constant(default);
            };
            if arr.is_empty() {
                ctx.err(&format!("{path}.keyframes"), "must not be empty");
                return Track::constant(default);
            }
            let mut kfs = Vec::new();
            let mut last_t: Option<f32> = None;
            for (i, item) in arr.iter().enumerate() {
                let kpath = format!("{path}.keyframes[{i}]");
                let Some(kobj) = item.as_object() else {
                    ctx.err(&kpath, "must be an object with 't' and 'value'");
                    continue;
                };
                let t = match kobj.get("t").and_then(as_f32) {
                    Some(t) => t,
                    None => {
                        ctx.err(&format!("{kpath}.t"), "missing or not a number");
                        continue;
                    }
                };
                if let Some(prev) = last_t {
                    if t < prev {
                        ctx.err(&format!("{kpath}.t"), "keyframe t values must be sorted ascending");
                    }
                }
                last_t = Some(t);
                let value = match kobj.get("value") {
                    None => {
                        ctx.err(&format!("{kpath}.value"), "missing");
                        continue;
                    }
                    Some(vraw) => match parse_leaf(vraw) {
                        Ok(v) => v,
                        Err(e) => {
                            ctx.err(&format!("{kpath}.value"), e);
                            continue;
                        }
                    },
                };
                let ease = match kobj.get("ease") {
                    None => Ease::Linear,
                    Some(Value::String(s)) => match Ease::parse(s) {
                        Ok(e) => e,
                        Err(msg) => {
                            ctx.err(&format!("{kpath}.ease"), msg);
                            Ease::Linear
                        }
                    },
                    Some(_) => {
                        ctx.err(&format!("{kpath}.ease"), "must be a string");
                        Ease::Linear
                    }
                };
                kfs.push(Keyframe { t, value, ease });
            }
            return if kfs.is_empty() { Track::constant(default) } else { Track::Keyframed(kfs) };
        }
    }
    match parse_leaf(raw) {
        Ok(v) => Track::constant(v),
        Err(e) => {
            ctx.err(path, e);
            Track::constant(default)
        }
    }
}

fn float_field(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: f32) -> Track<f32> {
    match obj.get(key) {
        None => Track::constant(default),
        Some(v) => build_track(ctx, v, &format!("{path}.{key}"), default, |leaf| as_f32(leaf).ok_or_else(|| "must be a number".to_string())),
    }
}

fn vec3_field(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: Vec3) -> Track<Vec3> {
    match obj.get(key) {
        None => Track::constant(default),
        Some(v) => build_track(ctx, v, &format!("{path}.{key}"), default, as_vec3),
    }
}

fn scale_field(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: Vec3) -> Track<Vec3> {
    match obj.get(key) {
        None => Track::constant(default),
        Some(v) => build_track(ctx, v, &format!("{path}.{key}"), default, as_scale_vec3),
    }
}

fn color_field(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: Vec3) -> Track<Vec3> {
    match obj.get(key) {
        None => Track::constant(default),
        Some(v) => build_track(ctx, v, &format!("{path}.{key}"), default, as_color_vec3),
    }
}

/// A plain (non-animated) number field: absent = `default`; present but not a number is an error, not a silent default.
fn plain_f32(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: f32) -> f32 {
    match obj.get(key) {
        None => default,
        Some(v) => as_f32(v).unwrap_or_else(|| {
            ctx.err(&format!("{path}.{key}"), format!("must be a number (got {})", short_json(v)));
            default
        }),
    }
}

/// A value rendered for an error message, cut to a readable length.
fn short_json(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 40 {
        format!("{}…", s.chars().take(40).collect::<String>())
    } else {
        s
    }
}

fn plain_hex(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: Vec3) -> Vec3 {
    match obj.get(key) {
        None => default,
        Some(v) => match as_color_vec3(v) {
            Ok(c) => c,
            Err(e) => {
                ctx.err(&format!("{path}.{key}"), e);
                default
            }
        },
    }
}

fn parse_material(ctx: &mut Ctx, obj: &Map<String, Value>, path: &str) -> Material {
    match obj.get("material").and_then(Value::as_object) {
        None => Material::default_gray(),
        Some(m) => {
            let mpath = format!("{path}.material");
            check_keys(&mut ctx.errors, &mpath, m, strict::MATERIAL_KEYS);
            Material {
                color: color_field(ctx, m, "color", &mpath, Vec3::splat(0.7)),
                metallic: plain_f32(ctx, m, "metallic", &mpath, 0.0).clamp(0.0, 1.0),
                roughness: plain_f32(ctx, m, "roughness", &mpath, 0.6).clamp(0.04, 1.0),
                emissive: plain_hex(ctx, m, "emissive", &mpath, Vec3::ZERO),
                opacity: plain_f32(ctx, m, "opacity", &mpath, 1.0).clamp(0.0, 1.0),
            }
        }
    }
}

fn default_camera() -> Camera {
    Camera {
        fov: Track::constant(90.0),
        near: 0.1,
        far: 200.0,
        position: Track::constant(Vec3::new(0.0, 2.0, 8.0)),
        target: Track::constant(Vec3::new(0.0, 1.0, 0.0)),
        roll: Track::constant(0.0),
    }
}

fn parse_camera(ctx: &mut Ctx, obj: &Map<String, Value>) -> Camera {
    check_keys(&mut ctx.errors, "camera", obj, strict::CAMERA_KEYS);
    Camera {
        fov: float_field(ctx, obj, "fov", "camera", 90.0),
        near: plain_f32(ctx, obj, "near", "camera", 0.1).max(0.001),
        far: plain_f32(ctx, obj, "far", "camera", 200.0),
        position: vec3_field(ctx, obj, "position", "camera", Vec3::new(0.0, 2.0, 8.0)),
        target: vec3_field(ctx, obj, "target", "camera", Vec3::new(0.0, 1.0, 0.0)),
        roll: float_field(ctx, obj, "roll", "camera", 0.0),
    }
}

fn ranged(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str, default: f32, min: f32, max: f32) -> f32 {
    let value = plain_f32(ctx, obj, key, path, default);
    if !(min..=max).contains(&value) {
        ctx.err(&format!("{path}.{key}"), format!("must be between {min} and {max}"));
        default
    } else {
        value
    }
}

fn parse_player(ctx: &mut Ctx, root: &Map<String, Value>) -> crate::player::PlayerTuning {
    let Some(obj) = root.get("player") else { return Default::default() };
    let Some(obj) = obj.as_object() else {
        ctx.err("player", "must be an object");
        return Default::default();
    };
    check_keys(&mut ctx.errors, "player", obj, strict::PLAYER_KEYS);
    let d = crate::player::PlayerTuning::default();
    let walk_speed = ranged(ctx, obj, "walk_speed", "player", d.walk_speed, 0.5, 20.0);
    let sprint_speed = ranged(ctx, obj, "sprint_speed", "player", d.sprint_speed, walk_speed, 30.0);
    // `humans_play_as` says what the setting does (the body every *human* wears; bots wear their own, `bots.roster[].character`).
    // `character` is the older spelling and stays accepted.
    let (key, raw) = match (obj.get("humans_play_as"), obj.get("character")) {
        (Some(new), Some(_)) => {
            ctx.err("player.character", "is the older name of player.humans_play_as: give only one of them");
            ("humans_play_as", Some(new))
        }
        (Some(new), None) => ("humans_play_as", Some(new)),
        (None, Some(old)) => ("character", Some(old)),
        (None, None) => ("humans_play_as", None),
    };
    let character = match raw {
        None => None,
        Some(Value::String(value)) => match crate::player::Character::parse(value) {
            Some(character) => Some(character),
            None => {
                ctx.err(&format!("player.{key}"), "must be human, rat, wizard, cowboy, alien, robot or boy");
                None
            }
        },
        Some(_) => {
            ctx.err(&format!("player.{key}"), "must be a string: human, rat, wizard, cowboy, alien, robot or boy");
            None
        }
    };
    let mode = match obj.get("mode") {
        None => d.mode,
        Some(Value::String(name)) => crate::player::PlayerMode::parse(name).unwrap_or_else(|| {
            ctx.err("player.mode", "must be \"arena\" (weapons and combat, the default) or \"peaceful\" (empty hands, a click interacts)");
            d.mode
        }),
        Some(_) => {
            ctx.err("player.mode", "must be a string: \"arena\" or \"peaceful\"");
            d.mode
        }
    };
    let third_person = match obj.get("view") {
        None => d.third_person,
        Some(Value::String(v)) if v == "third" => true,
        Some(Value::String(v)) if v == "first" => false,
        Some(_) => {
            ctx.err("player.view", "must be \"first\" (out of the character's eyes, the default) or \"third\" (behind the character)");
            d.third_person
        }
    };
    crate::player::PlayerTuning {
        character,
        fov_deg: ranged(ctx, obj, "fov", "player", d.fov_deg, 60.0, 120.0),
        walk_speed,
        sprint_speed,
        crouch_multiplier: ranged(ctx, obj, "crouch_multiplier", "player", d.crouch_multiplier, 0.1, 1.0),
        jump_speed: ranged(ctx, obj, "jump_speed", "player", d.jump_speed, 1.0, 15.0),
        gravity: ranged(ctx, obj, "gravity", "player", d.gravity, 1.0, 40.0),
        acceleration: ranged(ctx, obj, "acceleration", "player", d.acceleration, 0.0, 100.0),
        air_acceleration: ranged(ctx, obj, "air_acceleration", "player", d.air_acceleration, 0.0, 30.0),
        friction: ranged(ctx, obj, "friction", "player", d.friction, 0.0, 30.0),
        max_speed: ranged(ctx, obj, "max_speed", "player", d.max_speed.max(sprint_speed), sprint_speed, 50.0),
        throw_speed: ranged(ctx, obj, "throw_speed", "player", d.throw_speed, 0.0, 30.0),
        mode,
        third_person,
        fade_in: ranged(ctx, obj, "fade_in", "player", d.fade_in, 0.0, 30.0),
        expanse: crate::expanse::Expanse::default(),
    }
}

fn parse_jump_pads(ctx: &mut Ctx, root: &Map<String, Value>) -> Vec<crate::player::JumpPad> {
    let Some(value) = root.get("jump_pads") else { return Vec::new() };
    let Some(items) = value.as_array() else {
        ctx.err("jump_pads", "must be an array");
        return Vec::new();
    };
    let mut out = Vec::new();
    for (index, value) in items.iter().enumerate() {
        let path = format!("jump_pads[{index}]");
        let Some(obj) = value.as_object() else {
            ctx.err(&path, "must be an object");
            continue;
        };
        check_keys(&mut ctx.errors, &path, obj, strict::JUMP_PAD_KEYS);
        let id = obj.get("id").and_then(Value::as_str).unwrap_or("jump_pad").to_string();
        let position = match obj.get("position").map(as_vec3) {
            Some(Ok(v)) => v,
            _ => {
                ctx.err(&format!("{path}.position"), "must be [x, y, z]");
                Vec3::ZERO
            }
        };
        let size = match obj.get("size").and_then(Value::as_array).filter(|v| v.len() == 2) {
            Some(v) => Vec3::new(as_f32(&v[0]).unwrap_or(1.0), 0.0, as_f32(&v[1]).unwrap_or(1.0)),
            None => {
                ctx.err(&format!("{path}.size"), "must be [width, depth]");
                Vec3::new(1.0, 0.0, 1.0)
            }
        };
        let launch_speed = ranged(ctx, obj, "launch_speed", &path, 9.0, 2.0, 30.0);
        out.push(crate::player::JumpPad {
            id,
            center: glam::Vec2::new(position.x, position.z),
            size: glam::Vec2::new(size.x.max(0.2), size.z.max(0.2)),
            foot_y: position.y,
            launch_speed,
        });
    }
    out
}

fn parse_light(ctx: &mut Ctx, obj: &Map<String, Value>, path: &str) -> Light {
    let id = obj.get("id").and_then(Value::as_str).unwrap_or("light").to_string();
    let cast_shadows = obj.get("cast_shadows").and_then(Value::as_bool).unwrap_or(false);
    let shadow_radius = plain_f32(ctx, obj, "shadow_radius", path, 15.0);
    let shadow_center = match obj.get("shadow_center") {
        None => Vec3::ZERO,
        Some(v) => as_vec3(v).unwrap_or_else(|e| {
            ctx.err(&format!("{path}.shadow_center"), e);
            Vec3::ZERO
        }),
    };
    let color = color_field(ctx, obj, "color", path, Vec3::ONE);
    let kind_name = obj.get("type").and_then(Value::as_str);
    match kind_name {
        Some("directional") => check_keys(&mut ctx.errors, path, obj, strict::DIRECTIONAL_LIGHT_KEYS),
        Some("point") => check_keys(&mut ctx.errors, path, obj, strict::POINT_LIGHT_KEYS),
        _ => {}
    }
    let kind = match kind_name {
        Some("directional") => LightKind::Directional { direction: vec3_field(ctx, obj, "direction", path, Vec3::new(-0.4, -1.0, -0.3)) },
        Some("point") => LightKind::Point {
            position: vec3_field(ctx, obj, "position", path, Vec3::new(0.0, 3.0, 0.0)),
            range: plain_f32(ctx, obj, "range", path, 20.0).max(0.01),
        },
        Some(other) => {
            ctx.err(&format!("{path}.type"), format!("unknown light type '{other}' (expected 'directional' or 'point')"));
            LightKind::Directional { direction: Track::constant(Vec3::new(-0.4, -1.0, -0.3)) }
        }
        None => {
            ctx.err(&format!("{path}.type"), "missing (expected 'directional' or 'point')");
            LightKind::Directional { direction: Track::constant(Vec3::new(-0.4, -1.0, -0.3)) }
        }
    };
    if cast_shadows && !matches!(kind, LightKind::Directional { .. }) {
        ctx.err(&format!("{path}.cast_shadows"), "only a 'directional' light may cast shadows");
    }
    let intensity_default = match kind {
        LightKind::Directional { .. } => 2.0,
        LightKind::Point { .. } => 12.0,
    };
    let shadow_follow = obj.get("shadow_follow").and_then(Value::as_bool).unwrap_or(false);
    Light { id, kind, color, intensity: float_field(ctx, obj, "intensity", path, intensity_default), cast_shadows, shadow_radius, shadow_center, shadow_follow }
}

fn parse_prim(ctx: &mut Ctx, ty: &str, obj: &Map<String, Value>, path: &str) -> PrimKind {
    match ty {
        "box" => {
            let size = match obj.get("size") {
                Some(v) => as_vec3(v).unwrap_or_else(|e| {
                    ctx.err(&format!("{path}.size"), e);
                    Vec3::ONE
                }),
                None => Vec3::ONE,
            };
            PrimKind::Box { size }
        }
        "sphere" => PrimKind::Sphere { radius: plain_f32(ctx, obj, "radius", path, 0.5) },
        "cylinder" => PrimKind::Cylinder { radius: plain_f32(ctx, obj, "radius", path, 0.5), height: plain_f32(ctx, obj, "height", path, 1.0) },
        "cone" => PrimKind::Cone { radius: plain_f32(ctx, obj, "radius", path, 0.5), height: plain_f32(ctx, obj, "height", path, 1.0) },
        "capsule" => PrimKind::Capsule { radius: plain_f32(ctx, obj, "radius", path, 0.3), height: plain_f32(ctx, obj, "height", path, 1.0) },
        "plane" => {
            let (w, d) = match obj.get("size").and_then(Value::as_array) {
                Some(a) if a.len() == 2 => (as_f32(&a[0]).unwrap_or(10.0), as_f32(&a[1]).unwrap_or(10.0)),
                _ => (10.0, 10.0),
            };
            PrimKind::Plane { size: (w, d) }
        }
        _ => unreachable!("caller already validated type"),
    }
}

fn parse_pose_track_vec3(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str) -> Track<Vec3> {
    vec3_field(ctx, obj, key, path, Vec3::ZERO)
}

fn parse_pose_track_f32(ctx: &mut Ctx, obj: &Map<String, Value>, key: &str, path: &str) -> Track<f32> {
    float_field(ctx, obj, key, path, 0.0)
}

fn parse_humanoid(ctx: &mut Ctx, obj: &Map<String, Value>, path: &str) -> HumanoidDef {
    let height = plain_f32(ctx, obj, "height", path, 1.8).max(0.1);
    let build = plain_f32(ctx, obj, "build", path, 1.0).max(0.05);
    let material = parse_material(ctx, obj, path);
    let pose_obj = obj.get("pose").and_then(Value::as_object).cloned().unwrap_or_default();
    let ppath = format!("{path}.pose");
    check_keys(&mut ctx.errors, &ppath, &pose_obj, strict::HUMANOID_POSE_KEYS);
    let pose = Pose {
        spine: parse_pose_track_vec3(ctx, &pose_obj, "spine", &ppath),
        head: parse_pose_track_vec3(ctx, &pose_obj, "head", &ppath),
        l_shoulder: parse_pose_track_vec3(ctx, &pose_obj, "l_shoulder", &ppath),
        r_shoulder: parse_pose_track_vec3(ctx, &pose_obj, "r_shoulder", &ppath),
        l_elbow: parse_pose_track_f32(ctx, &pose_obj, "l_elbow", &ppath),
        r_elbow: parse_pose_track_f32(ctx, &pose_obj, "r_elbow", &ppath),
        l_hip: parse_pose_track_vec3(ctx, &pose_obj, "l_hip", &ppath),
        r_hip: parse_pose_track_vec3(ctx, &pose_obj, "r_hip", &ppath),
        l_knee: parse_pose_track_f32(ctx, &pose_obj, "l_knee", &ppath),
        r_knee: parse_pose_track_f32(ctx, &pose_obj, "r_knee", &ppath),
    };
    let style = obj.get("style").and_then(Value::as_str).unwrap_or("human");
    let style = match crate::player::Character::parse(style) {
        Some(who) if who != crate::player::Character::Rat => who,
        _ => {
            ctx.errors.push(format!("{path}.style: expected human, wizard, cowboy, alien, robot or boy"));
            crate::player::Character::Human
        }
    };
    let mut look = crate::characters::HumanLook::styled(style);
    for (key, slot) in [("skin", &mut look.skin), ("hair", &mut look.hair), ("pants", &mut look.pants), ("shoes", &mut look.shoes)] {
        *slot = plain_hex(ctx, obj, key, path, *slot);
    }
    HumanoidDef { height, build, material, look, pose }
}

fn parse_rat(ctx: &mut Ctx, obj: &Map<String, Value>, path: &str) -> RatDef {
    let mut material = parse_material(ctx, obj, path);
    if obj.get("material").and_then(|m| m.get("color")).is_none() {
        material.color = Track::constant(crate::color::parse_hex_to_linear(crate::characters::RAT_FUR_HEX).unwrap_or(Vec3::splat(0.3)));
    }
    let pose_obj = obj.get("pose").and_then(Value::as_object).cloned().unwrap_or_default();
    let ppath = format!("{path}.pose");
    check_keys(&mut ctx.errors, &ppath, &pose_obj, strict::RAT_POSE_KEYS);
    RatDef {
        material,
        gait: parse_pose_track_f32(ctx, &pose_obj, "gait", &ppath),
        stride: parse_pose_track_f32(ctx, &pose_obj, "stride", &ppath),
        sway: parse_pose_track_f32(ctx, &pose_obj, "sway", &ppath),
    }
}

fn parse_prop(ctx: &mut Ctx, obj: &Map<String, Value>, path: &str) -> PropDef {
    let kind = match obj.get("prop").and_then(Value::as_str) {
        Some(name) => match PropKind::from_name(name) {
            Some(k) => k,
            None => {
                let names: Vec<&str> = PropKind::ALL.iter().map(|k| k.name()).collect();
                ctx.err(&format!("{path}.prop"), format!("unknown prop '{name}' (expected one of: {})", names.join(", ")));
                PropKind::Crate
            }
        },
        None => {
            ctx.err(&format!("{path}.prop"), "missing (a prop object needs a 'prop' kind string)");
            PropKind::Crate
        }
    };
    PropDef { kind, material: parse_material(ctx, obj, path) }
}

fn parse_stairs(ctx: &mut Ctx, obj: &Map<String, Value>, path: &str) -> StairsDef {
    let width = plain_f32(ctx, obj, "width", path, 1.2).max(0.1);
    let run = plain_f32(ctx, obj, "run", path, 4.0).max(0.1);
    let rise = plain_f32(ctx, obj, "rise", path, 3.0).max(0.05);
    let steps = obj.get("steps").and_then(Value::as_u64).unwrap_or(16).clamp(1, 64) as u32;
    StairsDef { width, run, rise, steps, material: parse_material(ctx, obj, path) }
}

const PRIM_TYPES: &[&str] = &["box", "sphere", "cylinder", "cone", "capsule", "plane"];

fn parse_object(ctx: &mut Ctx, raw: &Value, path: &str) -> Object {
    let Some(obj) = raw.as_object() else {
        ctx.err(path, "must be an object");
        return Object {
            id: path.to_string(),
            position: Track::constant(Vec3::ZERO),
            rotation: Track::constant(Vec3::ZERO),
            scale: Track::constant(Vec3::ONE),
            material: Some(Material::default_gray()),
            collide: true,
            prefab: None,
            movable: None,
            kind: ObjectKind::Prim(PrimKind::Sphere { radius: 0.5 }),
        };
    };
    let id = match obj.get("id").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => {
            ctx.err(&format!("{path}.id"), "missing (every object needs a unique 'id' string)");
            path.to_string()
        }
    };
    let ty = obj.get("type").and_then(Value::as_str);
    if let Some(allowed) = ty.and_then(strict::object_keys) {
        check_keys(&mut ctx.errors, &id, obj, &allowed);
    }

    // Macro types (`wall`, `fence`) expand into an ordinary group of boxes before parsing, so
    // everything downstream only ever sees primitives. See `crate::macros`.
    if let Some(t) = ty.filter(|t| crate::macros::MACRO_TYPES.contains(t)) {
        return match crate::macros::expand(t, obj, &id) {
            Ok(group) => parse_object(ctx, &group, path),
            Err(errs) => {
                ctx.errors.extend(errs);
                Object {
                    id,
                    position: Track::constant(Vec3::ZERO),
                    rotation: Track::constant(Vec3::ZERO),
                    scale: Track::constant(Vec3::ONE),
                    material: None,
                    collide: true,
                    prefab: None,
                    movable: None,
                    kind: ObjectKind::Group(Vec::new()),
                }
            }
        };
    }

    let position = vec3_field(ctx, obj, "position", &id, Vec3::ZERO);
    let rotation = vec3_field(ctx, obj, "rotation", &id, Vec3::ZERO);
    let scale = scale_field(ctx, obj, "scale", &id, Vec3::ONE);

    let (kind, material) = match ty {
        Some(t) if PRIM_TYPES.contains(&t) => (ObjectKind::Prim(parse_prim(ctx, t, obj, &id)), Some(parse_material(ctx, obj, &id))),
        Some("group") => {
            let children = match obj.get("children").and_then(Value::as_array) {
                Some(arr) => arr.iter().enumerate().map(|(i, c)| parse_object(ctx, c, &format!("{id}.children[{i}]"))).collect(),
                None => {
                    ctx.err(&format!("{id}.children"), "missing (a group needs a 'children' array)");
                    Vec::new()
                }
            };
            (ObjectKind::Group(children), None)
        }
        Some("humanoid") => (ObjectKind::Humanoid(Box::new(parse_humanoid(ctx, obj, &id))), None),
        Some("rat") => (ObjectKind::Rat(Box::new(parse_rat(ctx, obj, &id))), None),
        Some("prop") => (ObjectKind::Prop(Box::new(parse_prop(ctx, obj, &id))), None),
        Some("stairs") => (ObjectKind::Stairs(Box::new(parse_stairs(ctx, obj, &id))), None),
        Some("terrain") => {
            let centre = position.sample(0.0);
            let base = ASSET_DIR.with(|d| d.borrow().clone());
            let tctx = crate::terrain::TerrainContext { wrap: ctx.wrap, base_dir: base.as_deref() };
            match crate::terrain::parse_terrain(obj, &id, centre, &tctx) {
                Ok(t) => (ObjectKind::Terrain(Box::new(TerrainDef { terrain: std::sync::Arc::new(t), material: parse_material(ctx, obj, &id) })), None),
                Err(errs) => {
                    ctx.errors.extend(errs);
                    (ObjectKind::Group(Vec::new()), None)
                }
            }
        }
        Some(other) => {
            ctx.err(
                &format!("{id}.type"),
                format!(
                    "unknown type '{other}' (expected box, sphere, cylinder, cone, capsule, plane, group, humanoid, rat, prop, stairs, terrain, wall, fence, or prefab)"
                ),
            );
            (ObjectKind::Prim(PrimKind::Sphere { radius: 0.5 }), Some(Material::default_gray()))
        }
        None => {
            ctx.err(&format!("{id}.type"), "missing");
            (ObjectKind::Prim(PrimKind::Sphere { radius: 0.5 }), Some(Material::default_gray()))
        }
    };

    let collide = obj.get("collide").and_then(Value::as_bool).unwrap_or(true);
    let prefab = obj
        .get("prefab_name")
        .and_then(Value::as_str)
        .map(|name| PrefabTag { name: name.to_string(), mount: obj.get("mount").and_then(Value::as_str).unwrap_or("floor").to_string() });
    let movable = obj.get("movable").and_then(Value::as_bool);
    Object { id, position, rotation, scale, material, collide, prefab, movable, kind }
}

/// Parses scene JSON text into a `Scene`, expanding prefabs and macros first; `Err` lists every `path: message` problem found.
pub fn parse_scene(text: &str) -> Result<Scene, Vec<String>> {
    parse_scene_in(text, None)
}

/// [`parse_scene`] for a scene that lives in `base_dir` (what a terrain `heightmap` path is relative to).
pub fn parse_scene_in(text: &str, base_dir: Option<&std::path::Path>) -> Result<Scene, Vec<String>> {
    ASSET_DIR.with(|d| *d.borrow_mut() = base_dir.map(std::path::Path::to_path_buf));
    let result = parse_scene_text(text);
    ASSET_DIR.with(|d| *d.borrow_mut() = None);
    result
}

fn parse_scene_text(text: &str) -> Result<Scene, Vec<String>> {
    let mut value: Value = serde_json::from_str(text).map_err(|e| vec![format!("json: {e}")])?;
    // Prefab instances (`"type": "prefab"`) expand into plain groups before anything else sees them.
    crate::prefabs::expand_scene(&mut value)?;
    let mut ctx = Ctx::default();
    let Some(root) = value.as_object() else {
        return Err(vec!["root: scene must be a JSON object".to_string()]);
    };

    check_keys(&mut ctx.errors, "", root, strict::ROOT_KEYS);
    strict::check_sections(&mut ctx.errors, root);
    ctx.errors.extend(crate::sim::interest::validate_sections(root));
    if let Some(v) = root.get("schema_version") {
        match v.as_u64() {
            Some(n) if n as u32 <= SCHEMA_VERSION => {}
            Some(n) => ctx.err("schema_version", format!("{n} is newer than this engine understands (it supports up to {SCHEMA_VERSION}); update red_engine2")),
            None => ctx.err("schema_version", format!("must be a whole number (current: {SCHEMA_VERSION}); omit it to mean {SCHEMA_VERSION}")),
        }
    }
    let meta = root.get("meta").and_then(Value::as_object);
    if let Some(m) = meta {
        check_keys(&mut ctx.errors, "meta", m, strict::META_KEYS);
    }
    let fps = meta.and_then(|m| m.get("fps")).and_then(Value::as_u64).unwrap_or(30).max(1) as u32;
    let duration = meta.and_then(|m| m.get("duration")).and_then(as_f32).unwrap_or(4.0);
    if duration <= 0.0 {
        ctx.err("meta.duration", "must be > 0");
    }
    let (width, height) = meta
        .and_then(|m| m.get("resolution"))
        .and_then(Value::as_array)
        .filter(|a| a.len() == 2)
        .and_then(|a| Some((a[0].as_u64()? as u32, a[1].as_u64()? as u32)))
        .unwrap_or((1280, 720));

    let default_sky_top = Vec3::new(0.42, 0.62, 0.88);
    let default_sky_bottom = Vec3::new(0.90, 0.94, 0.99);
    let background = match root.get("background").and_then(Value::as_object) {
        None => Background::Gradient { top: default_sky_top, bottom: default_sky_bottom },
        Some(bg) => {
            check_keys(&mut ctx.errors, "background", bg, strict::BACKGROUND_KEYS);
            if bg.contains_key("color") {
                Background::Flat(plain_hex(&mut ctx, bg, "color", "background", Vec3::splat(0.05)))
            } else {
                Background::Gradient {
                    top: plain_hex(&mut ctx, bg, "sky_top", "background", default_sky_top),
                    bottom: plain_hex(&mut ctx, bg, "sky_bottom", "background", default_sky_bottom),
                }
            }
        }
    };

    let (bg_top, bg_bottom) = match &background {
        Background::Flat(c) => (*c, *c),
        Background::Gradient { top, bottom } => (*top, *bottom),
    };
    let sky = match crate::atmosphere::parse_sky(root, bg_top, bg_bottom) {
        Ok(s) => s,
        Err(errs) => {
            ctx.errors.extend(errs);
            None
        }
    };
    let clock = match crate::daycycle::parse_clock(root) {
        Ok(c) => c,
        Err(errs) => {
            ctx.errors.extend(errs);
            None
        }
    };
    let procgen = match crate::procgen::world::parse_procgen(root) {
        Ok(p) => p,
        Err(errs) => {
            ctx.errors.extend(errs);
            None
        }
    };
    let audio = match crate::ambience::parse_audio(root, ASSET_DIR.with(|d| d.borrow().clone()).as_deref()) {
        Ok(a) => a,
        Err(errs) => {
            ctx.errors.extend(errs);
            None
        }
    };
    let ocean = match crate::atmosphere::parse_ocean(root) {
        Ok(o) => o,
        Err(errs) => {
            ctx.errors.extend(errs);
            None
        }
    };

    let ambient = root.get("ambient").and_then(Value::as_object);
    if let Some(a) = ambient {
        check_keys(&mut ctx.errors, "ambient", a, strict::AMBIENT_KEYS);
    }
    let ambient_color = match ambient {
        Some(a) => plain_hex(&mut ctx, a, "color", "ambient", Vec3::ONE),
        None => Vec3::ONE,
    };
    let ambient_intensity = match ambient {
        Some(a) => plain_f32(&mut ctx, a, "intensity", "ambient", 0.25),
        None => 0.25,
    };

    let post = match root.get("post") {
        None => PostSettings::default(),
        Some(v) => match v.as_object() {
            None => {
                ctx.err("post", "must be an object like {\"ao\": 0.9, \"outline\": 0.55}");
                PostSettings::default()
            }
            Some(p) => {
                check_keys(&mut ctx.errors, "post", p, strict::POST_KEYS);
                let d = PostSettings::default();
                PostSettings {
                    enabled: p.get("enabled").and_then(Value::as_bool).unwrap_or(d.enabled),
                    ao: plain_f32(&mut ctx, p, "ao", "post", d.ao).clamp(0.0, 3.0),
                    outline: plain_f32(&mut ctx, p, "outline", "post", d.outline).clamp(0.0, 1.0),
                    ao_radius: plain_f32(&mut ctx, p, "ao_radius", "post", d.ao_radius).clamp(0.05, 3.0),
                }
            }
        },
    };

    let camera = match root.get("camera").and_then(Value::as_object) {
        None => {
            ctx.err("camera", "missing");
            default_camera()
        }
        Some(c) => parse_camera(&mut ctx, c),
    };
    let music = match root.get("music") {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            ctx.err("music", "must be true or false (false starts the game silent)");
            true
        }
    };
    let flashlight = match root.get("flashlight") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            ctx.err("flashlight", "must be true or false (true gives the player a toggleable point light, the T key)");
            false
        }
    };
    let death_text = match root.get("death_text") {
        None => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            ctx.err("death_text", "must be a string (the death screen's title; omit it to keep \"ELIMINATED\")");
            None
        }
    };
    let teams = match root.get("teams") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            ctx.err("teams", "must be true or false (true lets players be assigned to team 1 or 2 outside a loadout shooter match)");
            false
        }
    };
    let mut player = parse_player(&mut ctx, root);
    match crate::expanse::parse_world(root) {
        Ok(expanse) => player.expanse = expanse,
        Err(errs) => ctx.errors.extend(errs),
    }
    ctx.wrap = player.expanse.wrap;
    let hud = match crate::hud_config::parse_hud(root, player.mode.is_peaceful()) {
        Ok(h) => h,
        Err(errs) => {
            ctx.errors.extend(errs);
            Default::default()
        }
    };
    let declared: Vec<&str> = root.get("vars").and_then(Value::as_object).map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
    for name in crate::hud_config::unknown_custom_vars(&hud, &declared) {
        ctx.err("hud.custom_vars", format!("`{name}` is not one of the scene's `vars` (a typo would draw nothing)"));
    }
    let jump_pads = parse_jump_pads(&mut ctx, root);

    let mut lights = Vec::new();
    if let Some(arr) = root.get("lights").and_then(Value::as_array) {
        if arr.len() > MAX_SCENE_LIGHTS {
            ctx.err("lights", format!("at most {MAX_SCENE_LIGHTS} authored lights are allowed"));
        }
        let mut shadow_casters = 0;
        for (i, lv) in arr.iter().enumerate() {
            let path = format!("lights[{i}]");
            match lv.as_object() {
                Some(lobj) => {
                    let light = parse_light(&mut ctx, lobj, &path);
                    if light.cast_shadows {
                        shadow_casters += 1;
                    }
                    lights.push(light);
                }
                None => ctx.err(&path, "must be an object"),
            }
        }
        if shadow_casters > 1 {
            ctx.err("lights", "at most one light may set cast_shadows: true");
        }
    }

    let mut objects = Vec::new();
    match root.get("objects").and_then(Value::as_array) {
        Some(arr) => {
            for (i, ov) in arr.iter().enumerate() {
                objects.push(parse_object(&mut ctx, ov, &format!("objects[{i}]")));
            }
        }
        None => ctx.err("objects", "missing (must be an array, may be empty)"),
    }

    // `"on_terrain": true` puts a top-level object on the ground: its `y` becomes a height above the terrain under its x/z (so a shell, a
    // rock or a hut sits on a dune without anyone computing the dune's height).
    if let Some(arr) = root.get("objects").and_then(Value::as_array) {
        let terrains: Vec<std::sync::Arc<crate::terrain::Terrain>> = objects
            .iter()
            .filter_map(|o| match &o.kind {
                ObjectKind::Terrain(t) => Some(t.terrain.clone()),
                _ => None,
            })
            .collect();
        for (i, raw) in arr.iter().enumerate() {
            if raw.get("on_terrain").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let Some(o) = objects.get_mut(i) else { continue };
            if raw.get("position").is_some_and(|p| p.is_object()) {
                ctx.err(&format!("{}.on_terrain", o.id), "needs a plain position [x, y, z] (not animated keyframes)");
                continue;
            }
            let p = o.position.sample(0.0);
            let ground = terrains.iter().filter_map(|t| t.height_at(p.x, p.z)).fold(None, |a: Option<f32>, h| Some(a.map_or(h, |m| m.max(h))));
            match ground {
                Some(h) => o.position = Track::constant(Vec3::new(p.x, p.y + h, p.z)),
                None => ctx.err(&format!("{}.on_terrain", o.id), format!("there is no terrain under ({:.1}, {:.1})", p.x, p.z)),
            }
        }
    }

    // Game rules: validated against the objects, zones and spawn points just parsed.
    let rules = match crate::sim::rules::parse_rules(root, &rule_refs(root, &objects)) {
        Ok(r) => r,
        Err(errs) => {
            ctx.errors.extend(errs);
            crate::sim::rules::RuleSet::default()
        }
    };
    // An audio layer follows a rule variable: it has to be one.
    if let Some(a) = &audio {
        for (i, l) in a.layers.iter().enumerate() {
            if !rules.var_names.iter().skip(crate::sim::rules::BUILTIN_VARS.len()).any(|n| *n == l.var) {
                ctx.err(&format!("audio.layers[{i}].var"), format!("`{}` is not one of the scene's vars (declare it under `vars`)", l.var));
            }
        }
    }
    // The game's own words and screens (`ui` block), checked against the variables and outcomes the rules just declared.
    let ui = match crate::ui_config::parse_ui(root, &rules) {
        Ok(u) => u,
        Err(errs) => {
            ctx.errors.extend(errs);
            None
        }
    };

    let weapons = match crate::weapons::parse_weapons(root) {
        Ok(w) => w,
        Err(errs) => {
            ctx.errors.extend(errs);
            Default::default()
        }
    };

    let combat = match crate::sim::combat_cfg::parse_combat(root) {
        Ok(c) => c,
        Err(errs) => {
            ctx.errors.extend(errs);
            Default::default()
        }
    };

    let bots = match crate::sim::ai::parse_bots(root) {
        Ok(b) => b,
        Err(errs) => {
            ctx.errors.extend(errs);
            Default::default()
        }
    };
    let shooter = match crate::sim::shooter::parse_shooter(root) {
        Ok(s) => s,
        Err(errs) => {
            ctx.errors.extend(errs);
            None
        }
    };
    let nav = match crate::sim::ai::nav::parse_nav(root) {
        Ok(n) => n,
        Err(errs) => {
            ctx.errors.extend(errs);
            None
        }
    };

    // A bad `race` block is reported by the strict section check (`strict::check_sections`) with the same message; here it only has to be read.
    let race = crate::sim::race::RaceCourse::from_scene_json(root).ok().flatten().map(std::sync::Arc::new);

    if !ctx.errors.is_empty() {
        return Err(ctx.errors);
    }

    Ok(Scene {
        fps,
        duration,
        width: width.max(2) + width % 2,
        height: height.max(2) + height % 2,
        background,
        ambient_color,
        ambient_intensity,
        camera,
        player,
        jump_pads,
        post,
        lights,
        objects,
        rules,
        weapons,
        combat,
        bots,
        nav,
        hud,
        ui,
        music,
        flashlight,
        death_text,
        teams,
        sky,
        clock,
        procgen,
        audio,
        ocean,
        race,
        shooter,
    })
}

/// Object ids at every depth in deterministic scene order. Network rule presentation uses this
/// shared dictionary so a compact index can name a nested child as well as a top-level object.
pub fn object_ids(objects: &[Object]) -> Vec<String> {
    fn ids(o: &Object, out: &mut Vec<String>) {
        out.push(o.id.clone());
        if let ObjectKind::Group(kids) = &o.kind {
            kids.iter().for_each(|k| ids(k, out));
        }
    }
    let mut out = Vec::new();
    objects.iter().for_each(|o| ids(o, &mut out));
    out
}

/// Everything a rule may refer to: object ids (any depth), top-level object bounds, zones and spawn ids.
fn rule_refs(root: &Map<String, Value>, objects: &[Object]) -> crate::sim::rules::Refs {
    let mut refs = crate::sim::rules::Refs::default();
    refs.object_ids.extend(object_ids(objects));
    refs.top_level_ids.extend(objects.iter().map(|object| object.id.clone()));
    refs.prop_ids.extend(objects.iter().filter(|o| crate::physics::classify(o).is_some()).map(|o| o.id.clone()));
    for it in crate::collide::interactables_of(objects) {
        refs.bounds.insert(it.id, (it.min, it.max));
    }
    for z in root.get("zones").and_then(Value::as_array).into_iter().flatten() {
        let (Some(id), Some(r)) = (z.get("id").and_then(Value::as_str), z.get("rect").and_then(Value::as_array).filter(|r| r.len() == 4)) else { continue };
        let n: Vec<f32> = r.iter().filter_map(|v| v.as_f64()).map(|v| v as f32).collect();
        if n.len() == 4 {
            let y = z.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            refs.zones.insert(id.to_string(), (Vec3::new(n[0].min(n[2]), y, n[1].min(n[3])), Vec3::new(n[0].max(n[2]), y, n[1].max(n[3]))));
        }
    }
    for s in root.get("spawns").and_then(Value::as_array).into_iter().flatten() {
        if let Some(id) = s.get("id").and_then(Value::as_str) {
            refs.spawn_ids.insert(id.to_string());
        }
    }
    refs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors_of(json: &str) -> Vec<String> {
        parse_scene(json).err().unwrap_or_default()
    }

    #[test]
    fn a_misspelled_object_field_is_an_error_with_the_fix_not_a_silent_default() {
        let e = errors_of(r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[{"id":"b","type":"box","size":[1,1,1],"pos":[0,3,0]}]}"#);
        assert_eq!(e, vec!["b.pos: unknown field — did you mean `position`?"]);
        let e = errors_of(r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[{"id":"s","type":"sphere","color":"red"}]}"#);
        assert!(e[0].starts_with("s.color: unknown field") && e[0].contains("material"), "{e:?}");
    }

    #[test]
    fn root_section_and_light_typos_are_caught_too() {
        let e = errors_of(r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"light":[],"objects":[]}"#);
        assert!(e.iter().any(|m| m.starts_with("light: unknown field") && m.contains("`lights`")), "{e:?}");
        let e = errors_of(r#"{"camera":{"position":[0,2,8],"target":[0,0,0],"fovv":60},"lights":[{"id":"l","type":"point","rnge":9}],"objects":[]}"#);
        assert!(e.iter().any(|m| m.starts_with("camera.fovv:") && m.contains("`fov`")), "{e:?}");
        assert!(e.iter().any(|m| m.starts_with("lights[0].rnge:") && m.contains("`range`")), "{e:?}");
        let e = errors_of(r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"zones":[{"id":"z","rectt":[0,0,1,1]}],"objects":[]}"#);
        assert!(e.iter().any(|m| m.contains("zones[0] (z).rectt") && m.contains("`rect`")), "{e:?}");
    }

    #[test]
    fn flashlight_defaults_off_and_rejects_a_non_bool() {
        let base = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[]}"#;
        assert!(!parse_scene(base).unwrap().flashlight, "absent means no flashlight, like older maps with no `music` key");
        let on = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"flashlight":true,"objects":[]}"#;
        assert!(parse_scene(on).unwrap().flashlight);
        let bad = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"flashlight":"yes","objects":[]}"#;
        assert!(errors_of(bad).iter().any(|m| m.starts_with("flashlight:")), "{:?}", errors_of(bad));
    }

    #[test]
    fn death_text_defaults_absent_and_rejects_a_non_string() {
        let base = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[]}"#;
        assert_eq!(parse_scene(base).unwrap().death_text, None, "absent means the standard client shows its own \"ELIMINATED\"");
        let set = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"death_text":"YOU WERE CAUGHT","objects":[]}"#;
        assert_eq!(parse_scene(set).unwrap().death_text.as_deref(), Some("YOU WERE CAUGHT"));
        let bad = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"death_text":5,"objects":[]}"#;
        assert!(errors_of(bad).iter().any(|m| m.starts_with("death_text:")), "{:?}", errors_of(bad));
    }

    #[test]
    fn teams_defaults_off_and_rejects_a_non_bool() {
        let base = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[]}"#;
        assert!(!parse_scene(base).unwrap().teams, "absent means no team assignment outside a loadout shooter match");
        let on = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"teams":true,"objects":[]}"#;
        assert!(parse_scene(on).unwrap().teams);
        let bad = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"teams":"yes","objects":[]}"#;
        assert!(errors_of(bad).iter().any(|m| m.starts_with("teams:")), "{:?}", errors_of(bad));
    }

    #[test]
    fn the_extension_namespace_lets_notes_through() {
        let json = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"x-tool":{"any":"thing"},"_todo":"later","objects":[{"id":"b","type":"box","notes":"hi","x-owner":"ai"}]}"#;
        assert!(parse_scene(json).is_ok(), "{:?}", errors_of(json));
    }

    #[test]
    fn a_number_field_holding_a_string_is_an_error() {
        let e = errors_of(r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[{"id":"s","type":"sphere","radius":"big"}]}"#);
        assert_eq!(e, vec!["s.radius: must be a number (got \"big\")"]);
    }

    #[test]
    fn a_scene_from_the_future_is_refused_with_a_message() {
        let e = errors_of(r#"{"schema_version":99,"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[]}"#);
        assert!(e[0].starts_with("schema_version: 99 is newer"), "{e:?}");
        assert!(parse_scene(r#"{"schema_version":1,"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[]}"#).is_ok());
    }

    #[test]
    fn prefab_instance_typos_are_caught() {
        let e = errors_of(
            r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[{"id":"a","type":"prefab","prefab":"apple_red","position":[0,0,0],"color":"red"}]}"#,
        );
        assert!(e.iter().any(|m| m.starts_with("a.color: unknown field")), "{e:?}");
    }

    #[test]
    fn minimal_scene_parses() {
        let json = r#"{"camera":{"position":[0,2,8],"target":[0,0,0]},"objects":[]}"#;
        let scene = parse_scene(json).expect("should parse");
        assert_eq!(scene.fps, 30);
        assert_eq!(scene.objects.len(), 0);
    }

    #[test]
    fn player_tuning_and_jump_pads_parse_strictly() {
        let scene = parse_scene(
            r#"{
                "camera":{"position":[0,2,8],"target":[0,1,0]},
                "player":{"character":"human","fov":90,"walk_speed":9,"sprint_speed":13,"crouch_multiplier":0.5,"jump_speed":6,"gravity":18},
                "jump_pads":[{"id":"lift","position":[1,0,2],"size":[2,3],"launch_speed":11}],
                "objects":[]
            }"#,
        )
        .unwrap();
        assert_eq!(scene.player.fov_deg, 90.0);
        assert_eq!(scene.player.sprint_speed, 13.0);
        assert_eq!(scene.player.character, Some(crate::player::Character::Human));
        assert_eq!(scene.jump_pads.len(), 1);
        assert_eq!(scene.jump_pads[0].id, "lift");

        let errors =
            parse_scene(r#"{"camera":{},"player":{"field_of_view":90},"jump_pads":[{"id":"x","position":[0,0,0],"size":[1,1],"boost":9}],"objects":[]}"#)
                .unwrap_err();
        assert!(errors.iter().any(|e| e.contains("field_of_view") && e.contains("fov")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("boost") && e.contains("launch_speed")), "{errors:?}");

        let errors = parse_scene(r#"{"camera":{},"player":{"character":"unknown-creature"},"objects":[]}"#).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("player.character") && e.contains("human")), "{errors:?}");
    }

    #[test]
    fn humans_play_as_is_the_clear_name_for_player_character() {
        let who = |player: &str| parse_scene(&format!(r#"{{"camera":{{}},"player":{player},"objects":[]}}"#)).map(|s| s.player.character);
        assert_eq!(who(r#"{"humans_play_as":"robot"}"#), Ok(Some(crate::player::Character::Robot)));
        assert_eq!(who(r#"{"character":"robot"}"#), Ok(Some(crate::player::Character::Robot)), "the old spelling still works");
        assert_eq!(who(r#"{}"#), Ok(None));
        let errors = who(r#"{"humans_play_as":"robot","character":"robot"}"#).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("player.character") && e.contains("humans_play_as") && e.contains("only one")), "{errors:?}");
        let errors = who(r#"{"humans_play_as":"unknown-creature"}"#).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("player.humans_play_as") && e.contains("human")), "{errors:?}");
    }

    #[test]
    fn missing_object_type_is_reported_with_precise_path() {
        let json = r#"{"camera":{},"objects":[{"id":"thing"}]}"#;
        let errs = parse_scene(json).unwrap_err();
        assert!(errs.iter().any(|e| e == "thing.type: missing"), "{errs:?}");
    }

    #[test]
    fn unsorted_keyframes_are_rejected() {
        let json = r#"{"camera":{},"objects":[
            {"id":"a","type":"sphere","position":{"keyframes":[{"t":1,"value":[0,0,0]},{"t":0,"value":[1,1,1]}]}}
        ]}"#;
        let errs = parse_scene(json).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("sorted ascending")), "{errs:?}");
    }

    #[test]
    fn multiple_shadow_casters_rejected() {
        let json = r#"{"camera":{},"objects":[],"lights":[
            {"id":"a","type":"directional","cast_shadows":true},
            {"id":"b","type":"directional","cast_shadows":true}
        ]}"#;
        let errs = parse_scene(json).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("at most one light")), "{errs:?}");
    }

    #[test]
    fn group_and_humanoid_parse() {
        let json = r##"{"camera":{},"objects":[
            {"id":"g","type":"group","children":[
                {"id":"g.post","type":"cylinder","material":{"color":"#8a6240"}}
            ]},
            {"id":"h","type":"humanoid","pose":{"l_elbow":15}}
        ]}"##;
        let scene = parse_scene(json).expect("should parse");
        assert_eq!(scene.objects.len(), 2);
        assert!(matches!(scene.objects[0].kind, ObjectKind::Group(_)));
        assert!(matches!(scene.objects[1].kind, ObjectKind::Humanoid(_)));
    }
}
