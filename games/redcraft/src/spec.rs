//! The `redcraft` scene block: a world of blocks built around the player.
//!
//! ```json
//! "redcraft": {"art": "shaders/blocks.wgsl", "view": 10, "seed": "launch"}
//! ```
//!
//! `art` is the game's `surface` shader that paints every block face from its texture number (required); `view` how many chunks the world reaches
//! (the player can change it); `seed` the seed of a new world (`"launch"`: a new one each new world; a number repeats a world, for tests and
//! pictures).

use red_engine2::atmosphere::LaunchSeed;
use red_engine2::fields::check_keys;
use serde_json::{Map, Value};

/// `redcraft` keys.
pub const KEYS: &[&str] = &["art", "view", "seed"];

/// The scene's block world.
#[derive(Debug, Clone)]
pub struct CraftSpec {
    pub art_file: String,
    /// The shader's WGSL (read when the scene was loaded from a file).
    pub art: Option<String>,
    /// Chunks the world reaches, by default.
    pub view: i32,
    pub seed: LaunchSeed,
}

/// The block, defined above the scene parser (`register`).
pub const BLOCK: red_engine2::schema::BlockDef = red_engine2::schema::BlockDef {
    key: "redcraft",
    parse: parse_block,
    fix_launch_seed: fix_seed,
    art_file: |v| v.get("art").and_then(Value::as_str),
    endless: true,
    describe: "{art, view, seed} RedCraft's endless world of blocks, generated around the player in 16 x 128 x 16 chunks (biomes, caves, ores, trees). art: the .wgsl file defining `fn surface(s: SurfaceIn) -> vec4<f32>` that paints every block face from the texture number in its vertex colour; view: chunks it reaches (2..16, default 10); seed: a new world's seed, or \"launch\". `redcraft shot SCENE OUT.png --at X,Y,Z`, `redcraft map SCENE OUT.png`; games/redcraft",
};

fn parse_block(root: &Map<String, Value>, base: Option<&std::path::Path>) -> Result<Option<red_engine2::schema::BlockValue>, Vec<String>> {
    Ok(parse(root, base)?.map(|m| std::sync::Arc::new(m) as red_engine2::schema::BlockValue))
}

fn fix_seed(v: &red_engine2::schema::BlockValue, fixed: LaunchSeed) -> Option<red_engine2::schema::BlockValue> {
    let m = v.downcast_ref::<CraftSpec>().filter(|m| m.seed == LaunchSeed::Launch)?;
    Some(std::sync::Arc::new(CraftSpec { seed: fixed, ..m.clone() }))
}

/// The `redcraft` block of a scene root, if it has one.
pub fn parse(root: &Map<String, Value>, base: Option<&std::path::Path>) -> Result<Option<CraftSpec>, Vec<String>> {
    let Some(v) = root.get("redcraft") else { return Ok(None) };
    let Some(o) = v.as_object() else {
        return Err(vec!["redcraft: must be an object like {\"art\": \"shaders/blocks.wgsl\"}".to_string()]);
    };
    let mut errs = Vec::new();
    check_keys(&mut errs, "redcraft", o, KEYS);
    let seed = LaunchSeed::parse(o.get("seed"), "redcraft.seed").unwrap_or_else(|e| {
        errs.push(e);
        LaunchSeed::Fixed(0)
    });
    let view = match o.get("view") {
        None => 10,
        Some(v) => match v.as_i64() {
            Some(n) if (2..=16).contains(&n) => n as i32,
            _ => {
                errs.push("redcraft.view: chunks the world reaches, a whole number from 2 to 16".to_string());
                10
            }
        },
    };
    let art_file = match o.get("art") {
        Some(Value::String(f)) if f.ends_with(".wgsl") => f.clone(),
        _ => {
            errs.push("redcraft.art: the path of the block shader next to the scene, like \"shaders/blocks.wgsl\"".to_string());
            String::new()
        }
    };
    let mut art = None;
    if let (Some(base), false) = (base, art_file.is_empty()) {
        let path = base.join(&art_file);
        match std::fs::read_to_string(&path) {
            Ok(code) => match red_engine2::atmosphere::check_art_shader("redcraft.art", &art_file, &code) {
                Ok(()) => art = Some(code),
                Err(e) => errs.push(e),
            },
            Err(e) => errs.push(format!("redcraft.art: `{art_file}` cannot be read (looked for {}): {e}", path.display())),
        }
    }
    if errs.is_empty() {
        Ok(Some(CraftSpec { art_file, art, view, seed }))
    } else {
        Err(errs)
    }
}

/// The scene's block world, if it has one.
pub fn spec_of(scene: &red_engine2::schema::Scene) -> Option<std::sync::Arc<CraftSpec>> {
    scene.blocks.get_arc::<CraftSpec>()
}
