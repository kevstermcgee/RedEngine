//! A playtest script (`playtest --script`, `describe playtest`): one action per step. Checker: `playscript::Script::parse`.

use super::common::Any;
use crate::strict_struct;

strict_struct! {
    /// A script.
    pub struct Script { policy: String, steps: Vec<ScriptStep>, schema: Any }
}

strict_struct! {
    /// One step: one action, plus the modifiers it takes.
    pub struct ScriptStep {
        wait: Any,
        look: Look,
        turn: Any,
        hold: Any,
        jump: Any,
        fire: Any,
        aim_at: Any,
        switch: Any,
        interact: Any,
        press: Any,
        approach: Any,
        look_at: Any,
        view: Any,
        policy: String,
        shot: String,
        snapshot: Any,
        player: Any,
        expect: ScriptExpect,
        wait_for: ScriptExpect,
        say: String,
        over: Any,
        secs: Any,
        camera: Any,
        track: bool,
        every: Any,
        clicks: Any,
        msg: String,
        within: Any,
        timeout: Any,
    }
}

strict_struct! {
    /// A direction to face.
    pub struct Look { yaw: Any, pitch: Any }
}

strict_struct! {
    /// An assertion on the state dump (a JSON pointer).
    pub struct ScriptExpect { at: String, eq: Any, ne: Any, min: Any, max: Any, contains: Any, exists: bool, within: Any, msg: String }
}
