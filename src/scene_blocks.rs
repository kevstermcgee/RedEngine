//! The scene blocks the engine defines above the scene parser (ADR 2026-10-13-a-scene-block-can-be-defined-above-the-core), and the games
//! the match simulation plays for them (`sim::game`): today the loadout shooter's `shooter`, until it moves to Killchain (ADR
//! 2026-10-14-a-game-plugs-into-the-server-s-match-simulation-the). The museum's was the first block; it is Frameless's since Frameless became a
//! crate of its own (`games/frameless` registers it, ADR 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library). Every binary calls [`register`] first; the engine's own scene loading
//! (tools, the renderer, tests) goes through [`parse_scene`] and [`parse_scene_in`], which register and then parse. Code headed for red_core calls
//! `schema` directly: in a process, a binary registered first.

use crate::schema::{self, BlockDef, Scene};

/// The blocks the engine defines.
pub static ENGINE: &[BlockDef] = &[crate::sim::shooter::BLOCK];

/// The games the engine defines for its blocks (the match simulation plays them; `sim::game`).
pub static ENGINE_GAMES: &[crate::sim::game::GameDef] = &[crate::sim::shooter::GAME];

/// The world layers the engine defines for its blocks (drawn by both renderers; `world_layer`).
#[cfg(feature = "render")]
pub static ENGINE_LAYERS: &[crate::world_layer::LayerDef] = &[];

/// Defines the engine's blocks for the scene parser, their games for the match simulation and their world layers (once per process is enough;
/// again is harmless).
pub fn register() {
    schema::define_blocks(ENGINE);
    crate::sim::game::define_games(ENGINE_GAMES);
    #[cfg(feature = "render")]
    crate::world_layer::define_layers(ENGINE_LAYERS);
}

/// [`schema::parse_scene`] with the engine's blocks registered.
pub fn parse_scene(text: &str) -> Result<Scene, Vec<String>> {
    register();
    schema::parse_scene(text)
}

/// [`schema::parse_scene_in`] with the engine's blocks registered.
pub fn parse_scene_in(text: &str, base_dir: Option<&std::path::Path>) -> Result<Scene, Vec<String>> {
    register();
    schema::parse_scene_in(text, base_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_nobody_defined_is_an_unknown_field() {
        register();
        let e = schema::parse_scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "musuem": {}}"#).unwrap_err();
        assert!(e.iter().any(|m| m.contains("musuem: unknown field")), "{e:?}");
    }
}
