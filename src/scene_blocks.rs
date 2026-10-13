//! The scene blocks the engine defines above the scene parser (ADR 2026-10-13-a-scene-block-can-be-defined-above-the-core): today the museum's.
//! Every binary calls [`register`] first; the engine's own scene loading (tools, the renderer, tests) goes through [`parse_scene`] and
//! [`parse_scene_in`], which register and then parse. Code headed for red_core calls `schema` directly: in a process, a binary registered first.

use crate::schema::{self, BlockDef, Scene};

/// The blocks the engine defines.
pub static ENGINE: &[BlockDef] = &[crate::museum::BLOCK];

/// Defines the engine's blocks for the scene parser (once per process is enough; again is harmless).
pub fn register() {
    schema::define_blocks(ENGINE);
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
    fn a_museum_scene_parses_with_the_engine_blocks_and_its_block_is_read_back_by_type() {
        let s = parse_scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "museum": {"collection": "c.json"}}"#)
            .expect("parses (no asset directory: only the keys are checked)");
        assert!(crate::museum::spec_of(&s).is_some_and(|m| m.collection_file == "c.json"), "{:?}", s.blocks);
        assert!(schema::root_keys().contains(&"museum"));
    }

    #[test]
    fn a_launch_seed_in_the_museum_is_fixed_with_the_others() {
        let mut s =
            parse_scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "museum": {"collection": "c.json", "seed": "launch"}}"#)
                .expect("parses");
        assert!(s.fix_launch_seeds(0x0123_4567), "the museum's launch seed counts");
        assert_eq!(crate::museum::spec_of(&s).map(|m| m.seed), Some(crate::atmosphere::LaunchSeed::Fixed(0x23_4567)));
        let mut fixed =
            parse_scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "museum": {"collection": "c.json", "seed": 7}}"#)
                .expect("parses");
        assert!(!fixed.fix_launch_seeds(1), "a fixed seed stays");
    }

    #[test]
    fn a_block_nobody_defined_is_an_unknown_field_with_a_suggestion() {
        register();
        let e = schema::parse_scene(r#"{"camera": {"position": [0, 2, 5], "target": [0, 0, 0]}, "objects": [], "musuem": {}}"#).unwrap_err();
        assert!(e.iter().any(|m| m.contains("musuem: unknown field") && m.contains("`museum`")), "{e:?}");
    }
}
