//! Map-authoring and analysis tools, exposed as `red_engine2` subcommands (see `src/main.rs`
//! and `AGENTS.md`). Everything here works on the same [`world::MapWorld`] — a scene flattened
//! into world-space items plus the *engine's own* collision/ground data — so the tools answer
//! questions about what the live game will actually do.

/// The message every render-dependent command returns in a build without the `gfx` feature.
pub const NO_GFX: &str = "this build has no renderer (built with --no-default-features); rebuild with `cargo build --release` (feature `gfx`, on by default) to use frame/tour/render/storyboard, catalog --sheet and golden-view checks";

pub mod blueprint;
pub mod catalog;
pub mod describe;
pub mod diff;
pub mod doctor;
pub mod edit;
pub mod envelope;
pub mod font;
pub mod game;
pub mod gen;
pub mod inspect;
pub mod lint;
pub mod newgame;
pub mod patch;
pub mod pathing;
pub mod plan;
pub mod reach;
pub mod recipes;
pub mod search;
pub mod shots;
pub mod sight;
pub mod simrun;
pub mod status;
pub mod symbols;
pub mod verify;
pub mod walk;
pub mod world;
