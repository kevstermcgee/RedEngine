//! A scene (`examples/*.json`, `recipes/*.json`, a game's maps, `tests/fixtures/*` scenes): the root and every section, typed. The checkers are
//! `schema::parse_scene_in` and the section parsers it calls (`strict.rs` key tables, `sim::*`, `ambience`, `atmosphere`, `daycycle`, `procgen`,
//! `style`, `ui_config`, `hud_config`, `weapons`), plus `tools::check_schema` / `simrun` for `checks`.

use super::checks::Checks;
use super::common::{Any, Material, V3Track};
use super::objects::Object;
use super::prefab::PrefabDefs;
use crate::strict_struct;
use std::collections::BTreeMap;

strict_struct! {
    /// A scene file.
    pub struct Scene {
        schema_version: u32,
        recipe: Recipe,
        meta: Meta,
        background: Background,
        ambient: Ambient,
        camera: Camera,
        post: Post,
        lights: Vec<Light>,
        zones: Vec<Zone>,
        spawns: Vec<Spawn>,
        portals: Vec<Portal>,
        interest: Interest,
        checks: Checks,
        prefabs: PrefabDefs,
        /// Game variables: name -> number or true/false.
        vars: BTreeMap<String, Any>,
        /// Per-player variables: name -> number or true/false.
        player_vars: BTreeMap<String, Any>,
        persist: Vec<String>,
        rules: Vec<Rule>,
        /// Named level states: name -> the rule ids assumed fired.
        phases: BTreeMap<String, Vec<String>>,
        fields: Vec<ForceField>,
        player: Player,
        jump_pads: Vec<JumpPad>,
        weapons: Weapons,
        r#match: Match,
        race: Race,
        combat: Combat,
        shooter: Shooter,
        bots: Bots,
        nav: Nav,
        music: bool,
        flashlight: bool,
        death_text: String,
        teams: bool,
        hud: Hud,
        ui: Ui,
        world: World,
        sky: Sky,
        clock: Clock,
        procgen: Procgen,
        audio: Audio,
        ocean: Ocean,
        style: Style,
        fog: Fog,
        objects: Vec<Object>,
    }
}

strict_struct! {
    /// What a known-good example map teaches (`recipe`).
    pub struct Recipe { title: String, summary: String, teaches: Any, tips: Any }
}

strict_struct! {
    /// Video and frame settings.
    pub struct Meta { fps: Any, duration: Any, resolution: Any }
}

strict_struct! {
    /// A screen-space sky: a gradient or a flat colour.
    pub struct Background { sky_top: String, sky_bottom: String, color: String }
}

strict_struct! {
    /// Flat fill light.
    pub struct Ambient { color: String, intensity: Any }
}

strict_struct! {
    /// The camera (and the spawn when there are no `spawns`).
    pub struct Camera { fov: Any, near: Any, far: Any, position: V3Track, target: V3Track, roll: Any }
}

strict_struct! {
    /// The clarity post pass.
    pub struct Post { enabled: bool, ao: Any, outline: Any, ao_radius: Any }
}

/// A light, by its `type`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Light {
    Point(PointLight),
    Directional(DirectionalLight),
}

strict_struct! {
    /// A point light.
    pub struct PointLight { id: String, position: V3Track, color: Any, intensity: Any, range: Any }
}

strict_struct! {
    /// The sun (one may cast shadows).
    pub struct DirectionalLight {
        id: String,
        direction: V3Track,
        color: Any,
        intensity: Any,
        cast_shadows: bool,
        shadow_radius: Any,
        shadow_center: Any,
        shadow_follow: bool,
    }
}

strict_struct! {
    /// A named room.
    pub struct Zone { id: String, rect: Any, y: Any, kind: String }
}

strict_struct! {
    /// A spawn point.
    pub struct Spawn { id: String, position: Any, yaw_deg: Any, group: String }
}

strict_struct! {
    /// A doorway between two zones.
    pub struct Portal { id: String, between: Vec<String>, center: Any, width: Any, height: Any, open: bool }
}

strict_struct! {
    /// Network interest settings.
    pub struct Interest { cell_size: Any, note: String, hops: Any }
}

strict_struct! {
    /// Live-player tuning.
    pub struct Player {
        humans_play_as: Any,
        character: String,
        fov: Any,
        walk_speed: Any,
        sprint_speed: Any,
        crouch_multiplier: Any,
        jump_speed: Any,
        gravity: Any,
        acceleration: Any,
        air_acceleration: Any,
        friction: Any,
        max_speed: Any,
        throw_speed: Any,
        mode: String,
        view: Any,
        fade_in: Any,
        still: Any,
    }
}

strict_struct! {
    /// A vertical launch pad.
    pub struct JumpPad { id: String, position: Any, size: Any, launch_speed: Any }
}

// ---- rules ---------------------------------------------------------------------------------------------------------------------------------

strict_struct! {
    /// One rule: a trigger, an optional condition, actions (`describe rules`).
    pub struct Rule { id: String, when: When, who: String, r#if: String, once: bool, cooldown: Any, r#do: Vec<Action> }
}

strict_struct! {
    /// A rule's trigger (exactly one key; the checker says so).
    pub struct When {
        start: bool,
        enter: Volume,
        exit: Volume,
        event: String,
        every: Any,
        after: Any,
        prop_enter: Volume,
        prop_exit: Volume,
        prop_below: Any,
        /// With `prop_enter` / `prop_exit` / a prop's `event`: only this prop, or these (`self` is the one that fired the rule).
        prop: Any,
    }
}

strict_struct! {
    /// A place a rule watches.
    pub struct Volume { zone: String, object: String, r#box: Any, pad: Any, height: Any }
}

/// A rule action: an object with exactly one key.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Set(Any),
    Add(Any),
    Emit(String),
    Hide(String),
    Show(String),
    Collision(Any),
    Deactivate(String),
    Activate(String),
    Teleport(Any),
    End(String),
    Impulse(Impulse),
    Reset(Reset),
    Place(Any),
    /// `[field, target]`: point an aimed field at a loose prop, an object, `[x, y, z]`, `self` or `null`.
    Aim(Any),
}

strict_struct! {
    /// A shove on a loose prop.
    pub struct Impulse { object: String, dir: Any, speed: Any }
}

/// What `reset` puts back: an id, a list of ids, or the props in a zone.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Reset {
    One(String),
    Many(Vec<String>),
    Zone(ResetZone),
}

strict_struct! {
    /// `{"zone": id}`.
    pub struct ResetZone { zone: String }
}

strict_struct! {
    /// A force volume on loose props.
    pub struct ForceField {
        id: String,
        zone: String,
        object: String,
        r#box: Any,
        pad: Any,
        height: Any,
        velocity: Any,
        lift: Any,
        rate: Any,
        /// An aimed field: a loose prop (followed), an object, `[x, y, z]`, or `null` until a rule `aim`s it.
        toward: Any,
        /// An aimed field's pull, m/s (negative pushes away); a number or an expression.
        speed: Any,
        /// Only these loose props (an id or a list).
        props: Any,
    }
}

// ---- gameplay ------------------------------------------------------------------------------------------------------------------------------

strict_struct! {
    /// The single-weapon arena's weapons.
    pub struct Weapons { starting: String, bat: Bat, ammo: Ammo, ladder: Vec<String> }
}

strict_struct! {
    /// The bat.
    pub struct Bat { damage: Any }
}

/// `"infinite"` or a supply.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Ammo {
    Infinite(String),
    Supply(AmmoSupply),
}

strict_struct! {
    /// One ammunition supply for every firearm.
    pub struct AmmoSupply { loaded: Any, capacity: Any, reserve: Any }
}

strict_struct! {
    /// The lobby -> countdown -> round -> results flow.
    pub struct Match { min_players: Any, countdown_secs: Any, round_secs: Any, results_secs: Any, score_to_win: Any, join_in_progress: bool, ready_check: bool }
}

strict_struct! {
    /// A kart race.
    pub struct Race {
        laps: Any,
        gates: Vec<String>,
        countdown_secs: Any,
        finish_grace_secs: Any,
        line: Any,
        item_boxes: Vec<String>,
        item_respawn_secs: Any,
        surfaces: Vec<RaceSurface>,
    }
}

strict_struct! {
    /// A zone's driving surface.
    pub struct RaceSurface { zone: String, kind: String }
}

strict_struct! {
    /// Fight pacing.
    pub struct Combat { respawn_secs: Any, spawn: String, spawn_protect_secs: Any, regen_delay_secs: Any, regen_per_sec: Any }
}

strict_struct! {
    /// The loadout shooter.
    pub struct Shooter {
        start: Vec<String>,
        friendly_fire: bool,
        pickups: Vec<Pickup>,
        mode: String,
        team_size: Any,
        flags: Vec<Flag>,
        sites: Vec<Site>,
        objective: Objective,
    }
}

strict_struct! {
    /// A weapon or ammo on the map.
    pub struct Pickup { weapon: String, ammo: bool, at: Any, respawn_secs: Any }
}

strict_struct! {
    /// A capture-the-flag flag.
    pub struct Flag { team: Any, at: Any }
}

strict_struct! {
    /// A bomb site.
    pub struct Site { name: String, at: Any, radius: Any }
}

strict_struct! {
    /// Mode timings and limits.
    pub struct Objective {
        capture_limit: Any,
        return_secs: Any,
        win_rounds: Any,
        swap_after: Any,
        round_secs: Any,
        freeze_secs: Any,
        plant_secs: Any,
        defuse_secs: Any,
        fuse_secs: Any,
    }
}

strict_struct! {
    /// AI players.
    pub struct Bots { fill: Any, skill: Any, roster: Vec<Bot> }
}

strict_struct! {
    /// One named bot.
    pub struct Bot { name: String, character: String, skill: Any, style: String, team: Any }
}

strict_struct! {
    /// The bots' waypoint graph.
    pub struct Nav { nodes: Vec<NavNode>, edges: Vec<Any> }
}

strict_struct! {
    /// A waypoint.
    pub struct NavNode { id: String, pos: Any }
}

// ---- presentation --------------------------------------------------------------------------------------------------------------------------

strict_struct! {
    /// Which on-screen display is drawn.
    pub struct Hud {
        enabled: bool,
        show_scoreboard: bool,
        show_round: bool,
        show_ping: bool,
        show_combat: bool,
        show_crosshair: bool,
        show_events: bool,
        show_help: bool,
        show_rules_vars: bool,
        custom_vars: Vec<String>,
    }
}

strict_struct! {
    /// The game's own words (`describe ui`).
    pub struct Ui {
        title: String,
        /// var -> the name the HUD shows.
        labels: BTreeMap<String, String>,
        counters: Vec<Counter>,
        objective: UiObjective,
        start: Card,
        /// A line shown on the pause menu (`{var}` placeholders).
        pause: String,
        /// outcome (or `default`) -> its end card.
        end: BTreeMap<String, Card>,
    }
}

strict_struct! {
    /// A counter on the HUD.
    pub struct Counter { var: String, of: Any, label: String, format: String }
}

/// The objective line: one text, or texts chosen by conditions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum UiObjective {
    Text(String),
    Lines(Vec<ObjectiveLine>),
}

strict_struct! {
    /// One conditional objective line.
    pub struct ObjectiveLine { r#if: String, text: String }
}

strict_struct! {
    /// A start, pause or end card.
    pub struct Card { title: String, text: String, button: String }
}

strict_struct! {
    /// An endless world: a looping axis and the edges of the others.
    pub struct World { wrap: Wrap, bounds: Bounds }
}

strict_struct! {
    /// The looping axis.
    pub struct Wrap { axis: String, min: Any, max: Any }
}

strict_struct! {
    /// The invisible edge.
    pub struct Bounds { x: Any, z: Any }
}

strict_struct! {
    /// A sky dome with a sun.
    pub struct Sky { zenith: String, horizon: String, gradient_power: Any, sun: Sun, shader: String, seed: Any }
}

strict_struct! {
    /// The sun at infinity.
    pub struct Sun { direction: Any, size_deg: Any, color: String, glow: Any }
}

strict_struct! {
    /// A day that turns to night.
    pub struct Clock {
        day_secs: Any,
        start: Any,
        sun_max_deg: Any,
        latitude_deg: Any,
        fog: Any,
        stars: Any,
        moon: bool,
        moon_phase: Any,
        night_light: Any,
        light: String,
    }
}

strict_struct! {
    /// An endless generated world.
    pub struct Procgen { seed: Any, relief: Any, trees: Any, flowers: Any, grass: Any }
}

strict_struct! {
    /// The sounds of the world.
    pub struct Audio {
        ambience: String,
        /// dawn | day | dusk | night -> a score file.
        music: AudioMusic,
        music_volume: Any,
        ambience_volume: Any,
        reverb: AudioReverb,
        duck: AudioDuck,
        layers: Vec<AudioLayer>,
        birds: bool,
        /// Music that never repeats.
        journey: AudioJourney,
        /// The player's own music: true, or {title_secs, corner, shuffle}.
        playlist: AudioPlaylistOr,
    }
}

/// `audio.playlist`: `true`, or the details.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum AudioPlaylistOr {
    On(bool),
    Detailed(AudioPlaylist),
}

strict_struct! {
    /// The player's own music.
    pub struct AudioPlaylist { title_secs: Any, corner: String, shuffle: bool }
}

strict_struct! {
    /// Endless sections re-rendered from scores.
    pub struct AudioJourney { scores: Vec<String>, seed: Any, fade: Any, repeats: Any }
}

strict_struct! {
    /// A score per time of day.
    pub struct AudioMusic { dawn: String, day: String, dusk: String, night: String }
}

strict_struct! {
    /// Room reverb on the music.
    pub struct AudioReverb { decay: Any, mix: Any }
}

strict_struct! {
    /// Music ducking on events.
    pub struct AudioDuck { events: Vec<String>, depth: Any, attack: Any, hold: Any, release: Any }
}

strict_struct! {
    /// A score layered in while a variable is above a level.
    pub struct AudioLayer { score: String, var: String, above: Any, fade: Any, volume: Any }
}

strict_struct! {
    /// An endless water plane.
    pub struct Ocean {
        y: Any,
        color_deep: String,
        color_shallow: String,
        foam_color: String,
        wave_amplitude: Any,
        wave_frequency: Any,
        wave_speed: Any,
        roughness: Any,
        depth_scale: Any,
        haze_distance: Any,
    }
}

strict_struct! {
    /// A game's own look (`describe style`).
    pub struct Style {
        /// role -> `#rrggbb`.
        palette: BTreeMap<String, String>,
        /// preset name -> material fields.
        materials: BTreeMap<String, Material>,
        light: StyleLight,
        sky: StyleSky,
        fog: Fog,
        post: Post,
    }
}

strict_struct! {
    /// The light rig.
    pub struct StyleLight { sun: StyleSun, ambient: Ambient, lamp: StyleLamp }
}

strict_struct! {
    /// The rig's sun.
    pub struct StyleSun { color: String, intensity: Any, direction: Any, cast_shadows: bool }
}

strict_struct! {
    /// The rig's lamp.
    pub struct StyleLamp { color: String, intensity: Any }
}

strict_struct! {
    /// The style's sky.
    pub struct StyleSky { top: String, bottom: String, color: String }
}

strict_struct! {
    /// Distance fog.
    pub struct Fog { color: String, density: Any }
}
