//! Frameless: an endless museum built around the player as they walk (rooms, doors and wings from a seed; art with plaques written from a
//! collection), played in the engine's first-person client with a game mode of its own (ADR
//! 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library).
//!
//! The pieces: the `museum` scene block ([`museum::BLOCK`]: `museum::spec`), the visit ([`museum::Museum`]), its world layer for both renderers
//! ([`museum_gpu::LAYER`]), its game mode in the live client ([`mode::MuseumMode`]), the `frameless museum` tool ([`tool`]) and the collection's
//! typed definition ([`content`]). [`register`] defines the block and the layer; the binary does it before anything parses a scene.

pub mod content;
pub mod mode;
pub mod museum;
pub mod museum_gpu;
pub mod tool;

/// The scene blocks Frameless defines (the museum's).
pub static BLOCKS: &[red_engine2::schema::BlockDef] = &[museum::BLOCK];

/// The world layers Frameless defines (the museum's).
pub static LAYERS: &[red_engine2::world_layer::LayerDef] = &[museum_gpu::LAYER];

/// Defines the museum block for the scene parser and its world layer for the renderers (and the engine's own blocks); once per process is
/// enough, again is harmless.
pub fn register() {
    red_engine2::scene_blocks::register();
    red_engine2::schema::define_blocks(BLOCKS);
    red_engine2::world_layer::define_layers(LAYERS);
}

/// Frameless's modes: a museum scene's visit.
pub fn mode_for(scene: &red_engine2::schema::Scene) -> Option<Box<dyn red_engine2::client::GameMode>> {
    museum::spec_of(scene).map(|_| Box::new(mode::MuseumMode::new()) as Box<dyn red_engine2::client::GameMode>)
}

#[cfg(test)]
mod tests {
    use red_engine2::schema::{self, Scene};

    fn scene(text: &str) -> Scene {
        super::register();
        schema::parse_scene(text).expect("parses (no asset directory: only the keys are checked)")
    }

    #[test]
    fn a_museum_scene_parses_once_frameless_registers_and_its_block_is_read_back_by_type() {
        let s = scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "museum": {"collection": "c.json"}}"#);
        assert!(crate::museum::spec_of(&s).is_some_and(|m| m.collection_file == "c.json"), "{:?}", s.blocks);
        assert!(schema::root_keys().contains(&"museum"));
        assert!(red_engine2::tools::describe::scene_keys().iter().any(|(k, d)| *k == "museum" && d.contains("collection")), "the block describes itself");
    }

    #[test]
    fn a_launch_seed_in_the_museum_is_fixed_with_the_others() {
        let mut s = scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "museum": {"collection": "c.json", "seed": "launch"}}"#);
        assert!(s.fix_launch_seeds(0x0123_4567), "the museum's launch seed counts");
        assert_eq!(crate::museum::spec_of(&s).map(|m| m.seed), Some(red_engine2::atmosphere::LaunchSeed::Fixed(0x23_4567)));
        let mut fixed = scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "museum": {"collection": "c.json", "seed": 7}}"#);
        assert!(!fixed.fix_launch_seeds(1), "a fixed seed stays");
    }

    #[test]
    fn a_museum_scene_gets_the_museum_mode_and_any_other_scene_none() {
        let plain = scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": []}"#);
        assert!(super::mode_for(&plain).is_none());
        let museum = scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "museum": {"collection": "c.json"}}"#);
        assert_eq!(super::mode_for(&museum).map(|m| m.name()), Some("museum"));
    }

    #[test]
    fn registering_twice_defines_the_block_and_the_layer_once() {
        super::register();
        super::register();
        assert_eq!(schema::defined_blocks().iter().filter(|b| b.key == "museum").count(), 1);
        let museum = scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "museum": {"collection": "c.json"}}"#);
        assert_eq!(red_engine2::world_layer::art_of(&museum), None, "a museum without `art` paints with the engine's surfaces");
    }

    #[test]
    fn the_museums_painted_surfaces_and_its_sky_compile_everywhere_the_engine_runs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("content/shaders");
        let art = std::fs::read_to_string(dir.join("art.wgsl")).unwrap();
        let sky = std::fs::read_to_string(dir.join("sky.wgsl")).unwrap();
        red_engine2::atmosphere::check_art_shader("museum.art", "shaders/art.wgsl", &art).unwrap();
        red_engine2::atmosphere::check_sky_shader_with("shaders/sky.wgsl", &sky, Some(&art)).unwrap();
    }

    #[test]
    fn the_museum_validates_with_the_block_frameless_defines() {
        super::register();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("content/frameless.json");
        let text = std::fs::read_to_string(&path).unwrap();
        let s = schema::parse_scene_in(&text, path.parent()).unwrap_or_else(|e| panic!("{}", e.join("\n")));
        assert!(crate::museum::spec_of(&s).is_some_and(|m| m.art.is_some()), "the scene's museum names its painted surfaces");
    }
}
