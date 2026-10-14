//! The Frameless download ships everything its scene needs. The engine's own check (`tools::publish_check`) cannot load a scene with the museum
//! block, which is Frameless's, so it leaves this playable to this crate: here the check runs with the block defined.

use std::path::Path;

#[test]
fn the_frameless_download_ships_everything_its_scene_needs() {
    frameless::register();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("games-publish.json")).unwrap()).unwrap();
    let frameless = manifest["playables"].as_array().unwrap().iter().find(|p| p["slug"] == "frameless").expect("Frameless is a playable");
    assert_eq!(frameless["binary"], "frameless", "the download runs Frameless's own client");
    let scene = root.join("games/frameless/content/frameless.json");
    red_engine2::load_scene(&scene).unwrap_or_else(|e| panic!("with the museum block defined, the scene loads: {}", e.join("; ")));
    assert_eq!(red_engine2::tools::publish_check::playable_resource_gaps(&root, &manifest).unwrap(), Vec::<String>::new());
}
