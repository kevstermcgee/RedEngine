//! RedCraft: an endless world of blocks to mine, craft and build with, played in the engine's first-person client with a game mode of its own
//! (ADR 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library).
//!
//! The pieces: the blocks and items ([`block`], [`item`]), the world and how it is made ([`gen`], [`chunk`], [`light`], [`world`]), streamed and
//! meshed around the player by worker threads ([`stream`], [`mesh`]) and drawn as a world layer ([`gpu`]) painted by the game's own shader
//! (`content/shaders/blocks.wgsl`); the game itself ([`mode`]): the body, mining and building, the inventory and crafting, the screens, the
//! sounds and the save. [`register`] defines the `redcraft` scene block and the layer; the binary does it before anything parses a scene.

pub mod block;
pub mod chunk;
pub mod clouds;
pub mod craft;
pub mod entities;
pub mod gen;
pub mod gpu;
pub mod hand;
pub mod icons;
pub mod inventory;
pub mod item;
pub mod light;
pub mod mesh;
pub mod mode;
pub mod noise;
pub mod physics;
pub mod play;
pub mod save;
pub mod screens;
pub mod sound;
pub mod spec;
pub mod stream;
pub mod tool;
pub mod ui;
pub mod world;

/// The scene blocks RedCraft defines.
pub static BLOCKS: &[red_engine2::schema::BlockDef] = &[spec::BLOCK];

/// The world layers RedCraft defines.
pub static LAYERS: &[red_engine2::world_layer::LayerDef] = &[gpu::LAYER];

/// Defines the `redcraft` block for the scene parser and its world layer for the renderers (and the engine's own blocks); once per process is
/// enough, again is harmless.
pub fn register() {
    red_engine2::scene_blocks::register();
    red_engine2::schema::define_blocks(BLOCKS);
    red_engine2::world_layer::define_layers(LAYERS);
}

/// RedCraft's modes: a scene with a `redcraft` block is played as RedCraft.
pub fn mode_for(scene: &red_engine2::schema::Scene) -> Option<Box<dyn red_engine2::client::GameMode>> {
    spec::spec_of(scene).map(|_| Box::new(mode::CraftMode::new()) as Box<dyn red_engine2::client::GameMode>)
}

#[cfg(test)]
mod tests {
    use red_engine2::schema;

    #[test]
    fn the_game_scene_validates_with_the_block_redcraft_defines_and_its_shader_compiles() {
        super::register();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("content/redcraft.json");
        let text = std::fs::read_to_string(&path).unwrap();
        let s = schema::parse_scene_in(&text, path.parent()).unwrap_or_else(|e| panic!("{}", e.join("\n")));
        let spec = crate::spec::spec_of(&s).expect("the scene has a redcraft block");
        assert!(spec.art.is_some(), "the block shader was read and checked");
        assert!(schema::root_keys().contains(&"redcraft"));
    }

    #[test]
    fn a_bad_block_says_what_is_wrong() {
        super::register();
        let e = schema::parse_scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "redcraft": {"view": 40}}"#).unwrap_err();
        let all = e.join("\n");
        assert!(all.contains("redcraft.view") && all.contains("redcraft.art"), "{all}");
    }
}
