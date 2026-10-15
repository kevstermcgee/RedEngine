//! Tidewatch's server side: its content, its gameplay system, its match game, and the bindings its story is written against, on the
//! engine's public API only. The reference game for narrative content (ADR 2026-10-15-narrative-is-compiled-content-run-by-one-story-system):
//! two characters, a decision that moves real peat, a callback a watch later, knowledge that only some of them have, a director that
//! sometimes does nothing on purpose. The player's client is the `tidewatch` crate beside it.

pub mod content;
pub mod game;
pub mod watch;

use red_engine2::schema::{BlockDef, BlockValue, Scene};
use red_engine2::sim::game::{GameDef, MatchGame};
use std::sync::Arc;

/// The `tidewatch` scene block.
pub const BLOCK: BlockDef = BlockDef {
    key: "tidewatch",
    parse: |root, base| content::parse_block(root, base).map(|s| s.map(|s| Arc::new(s) as BlockValue)),
    fix_launch_seed: |_, _| None,
    art_file: |_| None,
    endless: false,
    describe: "{story: compiled .story.json, seed, watch_secs, burn_secs, late_secs, show: {var: query}} Tidewatch (games/tidewatch): night watches on a tidal island, with a story bound to the watch",
};

/// Tidewatch's match game: a scene with a `tidewatch` block plays it.
pub const GAME: GameDef =
    GameDef { name: "tidewatch", build: |scene| spec_of(scene).and_then(|s| game::TidewatchGame::new(s).ok()).map(|g| Box::new(g) as Box<dyn MatchGame>) };

/// The scene's `tidewatch` block, if it has one.
pub fn spec_of(scene: &Scene) -> Option<Arc<content::TidewatchSpec>> {
    scene.blocks.get_arc::<content::TidewatchSpec>()
}

/// Defines the engine's blocks and Tidewatch's block and game (before anything parses a scene; again is harmless).
pub fn register() {
    red_engine2::scene_blocks::register();
    red_engine2::schema::define_blocks(&[BLOCK]);
    red_engine2::sim::game::define_games(&[GAME]);
}

/// The `tidewatch_server` program: Tidewatch's block and game, no modes.
pub const SERVER: red_engine2::server_app::GameServer =
    red_engine2::server_app::GameServer { program: "tidewatch_server", register, modes: &[], overrides: None };

/// What each command and event means, for the story authors' bindings file.
pub const NOTES: &[(&str, &str)] = &[
    ("tw:take", "the player takes a brick from the shed (the game issues it at the shed)"),
    ("tw:feed", "the player puts the carried brick in the brazier (the game issues it at the brazier)"),
    ("tw:ring", "the bell: the next watch begins (ashore only)"),
    ("tw:lend", "value n: move n bricks from the shed to the dock lamp (refused, changing nothing, if the shed has fewer)"),
    ("tw:stock_lamp", "the player puts the carried brick in the dock lamp (the game issues it at the lamp)"),
    ("tw:odile_helps", "Odile carries one brick from her own reserve to the brazier (unattended: a director may run it; the reserve holds 2; between watches it is spent for little: a watch begins with the brazier topped up to 2)"),
    ("tw:watch_started", "a watch begins; value: its number (1, 2)"),
    ("tw:fed", "a brick went in; value: bricks in the brazier now"),
    ("tw:low", "the brazier is down to its last brick"),
    ("tw:out", "the brazier burnt its last brick: the light is out"),
    ("tw:relit", "a brick went into an empty brazier during a watch"),
    ("tw:watch_ended", "a watch is over; value 1: the boat came in, 0: it was late"),
    ("tw:lent", "bricks went to the dock lamp by `tw:lend`; value: how many"),
    ("tw:lamp_stocked", "the player put a brick in the dock lamp themselves"),
    ("tw:odile_helped", "Odile brought a brick from her reserve"),
];

/// The bindings file Tidewatch's story is compiled against (`tidewatch_server story-bindings > content/story/bindings.json`; a test keeps it
/// fresh): the watch's queries, commands and events. The scene's variables come from the scene itself.
pub fn bindings() -> serde_json::Value {
    let mut c = content::catalog(&watch::Rules::default(), &serde_json::Map::new());
    c.scene_vars.clear();
    red_engine2::tools::story::bindings_json("tidewatch", &c, NOTES)
}
