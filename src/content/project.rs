//! A game project's `game.json` (`new-game`, `game check`). Checker: `tools::game`.

use super::common::Any;
use super::game2d::Capabilities;
use crate::strict_struct;

strict_struct! {
    /// `game.json`.
    pub struct Project {
        game: Any,
        id: String,
        name: String,
        engine: Engine,
        blueprints: Vec<String>,
        maps: Vec<String>,
        server: Server,
        capabilities: Capabilities,
        binary: String,
        server_binary: String,
    }
}

strict_struct! {
    /// The engine the game pins.
    pub struct Engine { git: String, r#ref: String, path: String }
}

strict_struct! {
    /// How `serve` runs the game.
    pub struct Server { map: String, port: Any, spawn_group: String, args: Vec<String> }
}
