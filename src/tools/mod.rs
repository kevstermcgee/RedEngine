//! Map-authoring and analysis tools, exposed as `red_engine2` subcommands (see `src/main.rs`
//! and `AGENTS.md`). Everything here works on the same [`world::MapWorld`] — a scene flattened
//! into world-space items plus the *engine's own* collision/ground data — so the tools answer
//! questions about what the live game will actually do.

pub mod catalog;
pub mod describe;
pub mod diff;
pub mod edit;
pub mod font;
pub mod gen;
pub mod inspect;
pub mod lint;
pub mod plan;
pub mod reach;
pub mod recipes;
pub mod search;
pub mod shots;
pub mod symbols;
pub mod verify;
pub mod walk;
pub mod world;
