//! An endless museum (`museum` scene block): rooms behind rooms for ever, each hung with art that has a name, a maker and a story, each keeping a key
//! that opens its doors, and paintings one can step into, with paintings inside them.
//!
//! * [`spec`]: the scene block and the collection file (themes, words, styles, masterpieces): what makes a museum a particular one.
//! * [`layout`]: the endless floor plan (rooms on a grid, doors, wings), a pure function of the seed.
//! * [`exhibit`]: what hangs in each room, its plaque, where the key is hidden, and the worlds inside paintings.
//! * [`sculpt`]: eighteen families of sculpture and installation, and how each moves.
//! * [`build`]: rooms, doorways and worlds as meshes, materials and colliders ([`build::Model`]).
//! * [`state`]: a visit: where the player is, what they found, the flight into a painting ([`state::Museum`]).
//! * [`text`]: the seeded word templates.
//!
//! The look of every painting and sculpture surface is the game's own WGSL (`museum.art`, `fn surface(s: SurfaceIn) -> vec4<f32>`), which the engine
//! composes into its object shader and in front of the game's sky, so the sky can draw the world inside the painting the player stepped into
//! (`game_param(0)` says which).

pub mod build;
pub mod exhibit;
pub mod layout;
pub mod map;
pub mod sculpt;
pub mod shape;
pub mod sound;
pub mod spec;
pub mod state;
pub mod text;

pub use layout::{EdgeId, Layout, RoomId, Side};
pub use spec::{parse_museum, Collection, MuseumSpec};
pub use state::{Event, Museum, Place, Progress, Target};

/// The `museum` scene block, defined above the scene parser (ADR 2026-10-13-a-scene-block-can-be-defined-above-the-core; `scene_blocks` registers it).
pub const BLOCK: red_engine2::schema::BlockDef = red_engine2::schema::BlockDef {
    key: "museum",
    parse: parse_block,
    fix_launch_seed: fix_seed,
    art_file: |v| v.get("art").and_then(serde_json::Value::as_str),
    endless: true,
    describe: "{collection, art, seed, start_theme} an endless museum built around the player: rooms, doors and wings from the seed, art with plaques written from the collection file (themes, words, styles, masterpieces), a key hidden in each room (a sculpture, or a painting up to three worlds deep), the worlds inside paintings. art: a .wgsl file defining `fn surface(s: SurfaceIn) -> vec4<f32>` (colour, glow) that paints every canvas and sculpture, composed in front of the sky too; `game_param(i)` says which painting's world surrounds the player. `red_engine2 museum SCENE` prints a room, `--inside N --shot`, `--map`, `--styles`; examples/frameless",
};

fn parse_block(
    root: &serde_json::Map<String, serde_json::Value>,
    base: Option<&std::path::Path>,
) -> Result<Option<red_engine2::schema::BlockValue>, Vec<String>> {
    Ok(parse_museum(root, base)?.map(|m| std::sync::Arc::new(m) as red_engine2::schema::BlockValue))
}

fn fix_seed(v: &red_engine2::schema::BlockValue, fixed: red_engine2::atmosphere::LaunchSeed) -> Option<red_engine2::schema::BlockValue> {
    let m = v.downcast_ref::<MuseumSpec>().filter(|m| m.seed == red_engine2::atmosphere::LaunchSeed::Launch)?;
    Some(std::sync::Arc::new(MuseumSpec { seed: fixed, ..m.clone() }))
}

/// The scene's museum, if it has one.
pub fn spec_of(scene: &red_engine2::schema::Scene) -> Option<std::sync::Arc<MuseumSpec>> {
    scene.blocks.get_arc::<MuseumSpec>()
}

/// Gives the scene this museum, or takes its museum away.
pub fn set_spec(scene: &mut red_engine2::schema::Scene, spec: Option<std::sync::Arc<MuseumSpec>>) {
    scene.blocks.set(BLOCK.key, spec.map(|m| m as red_engine2::schema::BlockValue));
}

/// A museum's lamps as scene lights (the client puts the nearest into the scene each frame; the offline renderer does the same for a still).
pub fn lamps_as_lights(lamps: &[build::Lamp]) -> Vec<red_engine2::schema::Light> {
    use red_engine2::schema::{Light, LightKind};
    use red_engine2::track::Track;
    lamps
        .iter()
        .enumerate()
        .map(|(i, l)| Light {
            id: format!("museum_lamp_{i}"),
            kind: LightKind::Point { position: Track::constant(l.at), range: l.range },
            color: Track::constant(glam::Vec3::from(l.color)),
            intensity: Track::constant(l.intensity),
            cast_shadows: false,
            shadow_radius: 10.0,
            shadow_center: glam::Vec3::ZERO,
            shadow_follow: false,
        })
        .collect()
}
