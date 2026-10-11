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
#[cfg(feature = "render")]
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

/// A museum's lamps as scene lights (the client puts the nearest into the scene each frame; the offline renderer does the same for a still).
pub fn lamps_as_lights(lamps: &[build::Lamp]) -> Vec<crate::schema::Light> {
    use crate::schema::{Light, LightKind};
    use crate::track::Track;
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
