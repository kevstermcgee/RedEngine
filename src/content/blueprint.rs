//! A blueprint (`build`, `new-game`): rooms, doors, spawns and prop fill that compile to a scene. Checker: `tools::blueprint::compile`.

use super::common::Any;
use super::objects::Object;
use super::scene::Scene;
use crate::strict_struct;

strict_struct! {
    /// A blueprint file.
    pub struct Blueprint {
        blueprint: Any,
        name: String,
        height: Any,
        ceiling: bool,
        rooms: Vec<Room>,
        doors: Vec<Door>,
        spawns: Vec<BlueprintSpawn>,
        fill: Vec<Fill>,
        keep_clear: Any,
        extra: Vec<Object>,
        prefab_files: Vec<String>,
        /// Merged into the built scene (rules, vars, ui, style, checks...).
        scene: Scene,
    }
}

strict_struct! {
    /// A room.
    pub struct Room { id: String, rect: Any, floor: String, lamp: bool }
}

strict_struct! {
    /// A door between two rooms.
    pub struct Door { between: Vec<String>, width: Any, at: Any, kind: String }
}

strict_struct! {
    /// Spawn points in a room.
    pub struct BlueprintSpawn { room: String, group: String, count: Any, id: String }
}

strict_struct! {
    /// Props scattered in a room.
    pub struct Fill { room: String, kind: Any, count: Any, seed: Any, scale: Any, colors: Any, min_gap: Any, clearance: Any, id: String }
}
