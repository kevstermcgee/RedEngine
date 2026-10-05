//! Map-authoring and analysis tools, exposed as `red_engine2` subcommands (see `src/main.rs`
//! and `AGENTS.md`). Everything here works on the same [`world::MapWorld`] — a scene flattened
//! into world-space items plus the *engine's own* collision/ground data — so the tools answer
//! questions about what the live game will actually do.

/// The message every render-dependent command returns in a build without the `gfx` feature.
pub const NO_GFX: &str = "this build has no renderer (built with --no-default-features); rebuild with `cargo build --release` (feature `gfx`, on by default) to use frame/tour/render/storyboard, catalog --sheet and golden-view checks";

pub mod adr;
pub mod affected;
pub mod analysis;
pub mod audio;
pub mod audio_checks;
pub mod blueprint;
pub mod catalog;
pub mod check_schema;
pub mod context;
pub mod describe;
pub mod diff;
pub mod doctor;
pub mod edit;
pub mod envelope;
pub mod features;
pub mod flora_sheet;
pub mod font;
pub mod game;
pub mod gamepublish;
pub mod gen;
pub mod inspect;
pub mod lint;
pub mod nav;
pub mod nettest;
pub mod newgame;
pub mod package;
pub mod patch;
pub mod pathing;
pub mod pe;
pub mod perf;
pub mod plan;
pub mod portmap;
pub mod preflight;
pub mod procgen_map;
pub mod publish_check;
pub mod racetest;
pub mod racetrack;
pub mod reach;
pub mod recipes;
pub mod search;
pub mod servers;
pub mod sheet;
pub mod shots;
pub mod sight;
pub mod simrun;
pub mod status;
pub mod symbols;
pub mod upgrade;
pub mod verify;
pub mod walk;
pub mod world;
