//! Procedural generation: an endless world made of pure functions of a seed.
//!
//! See [`world`] for the generator (ground, climate, biomes, plant placement per chunk), [`flora`] for the table of real species and the
//! climate each grows in, and [`noise`] for the deterministic noise under both. `red_engine2 procgen` draws a top-down map of any region
//! so a world can be looked at, and measured, without a renderer.

pub mod flora;
pub mod noise;
pub mod world;

pub use flora::{Climate, Kind, Species, SpeciesId};
pub use world::{Biome, ChunkId, Config, Plant, Trunk, World, CHUNK};
