//! Workshop's server side: its content, its game systems and its match game, on the engine's public API only (ADR
//! 2026-10-14-game-systems-compose-under-one-match-authority). A headless library and the `workshop_server` program; the player's client is
//! the `workshop` crate beside it.
//!
//! A scene with a `workshop` block (definition files: resources, recipes, machine kinds, stores, sources, sinks, machines, switches) plays
//! Workshop: [`register`] defines the block and the game before anything parses a scene. `workshop_server cli <command>` is the engine's
//! command line with them (`workshop_server cli sim content/workshop.json --inspect ws:furnace_a`).

pub mod content;
pub mod flow;
pub mod game;
pub mod machines;
pub mod wire;

use red_engine2::schema::{BlockDef, BlockValue, Scene};
use red_engine2::sim::game::{GameDef, MatchGame};
use std::sync::Arc;

/// The `workshop` scene block.
pub const BLOCK: BlockDef = BlockDef {
    key: "workshop",
    parse: |root, base| content::parse_block(root, base).map(|s| s.map(|s| Arc::new(s) as BlockValue)),
    fix_launch_seed: |_, _| None,
    art_file: |_| None,
    endless: false,
    describe: "{defs: [file, ...], seed, show: {var: sink}} a workshop (games/workshop): definition files of resources, recipes, machine kinds, stores, sources, sinks, machines and switches, each under a namespace; `show` copies what a sink shipped into a scene variable",
};

/// Workshop's match game: a scene with a `workshop` block plays it.
pub const GAME: GameDef =
    GameDef { name: "workshop", build: |scene| spec_of(scene).and_then(|s| game::WorkshopGame::new(s).ok()).map(|g| Box::new(g) as Box<dyn MatchGame>) };

/// The scene's `workshop` block, if it has one.
pub fn spec_of(scene: &Scene) -> Option<Arc<content::WorkshopSpec>> {
    scene.blocks.get_arc::<content::WorkshopSpec>()
}

/// Defines the engine's blocks and Workshop's block and game (before anything parses a scene; again is harmless).
pub fn register() {
    red_engine2::scene_blocks::register();
    red_engine2::schema::define_blocks(&[BLOCK]);
    red_engine2::sim::game::define_games(&[GAME]);
}

/// The `workshop_server` program: Workshop's block and game, no modes.
pub const SERVER: red_engine2::server_app::GameServer =
    red_engine2::server_app::GameServer { program: "workshop_server", register, modes: &[], overrides: None };
