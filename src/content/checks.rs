//! A scene's `checks` block (`verify`; `validate` holds it to the same shape): lint budget, reachability, walks, object assertions, views,
//! scripted playthroughs, performance budgets, the nav graph and audio. Checkers: `tools::check_schema`, `tools::audio_checks`, `tools::perf`,
//! `sim::scenario` (through `tools::simrun`).

use super::common::Any;
use crate::strict_struct;
use std::collections::BTreeMap;

strict_struct! {
    /// Everything `verify` checks.
    pub struct Checks {
        lint: LintCheck,
        reach: Vec<ReachCheck>,
        walk: Vec<WalkCheck>,
        objects: ObjectsCheck,
        views: Vec<ViewCheck>,
        sim: Vec<SimScenario>,
        perf: PerfCheck,
        nav: NavCheck,
        audio: AudioCheck,
    }
}

strict_struct! {
    /// The lint budget.
    pub struct LintCheck { max_errors: Any, max_warnings: Any, forbid: Vec<String>, ignore: Vec<String> }
}

strict_struct! {
    /// A place the player must (or must not) reach.
    pub struct ReachCheck { to: Any, from: Any, from_y: Any, why: String, phase: String, reachable: bool }
}

strict_struct! {
    /// A real-physics walk.
    pub struct WalkCheck {
        name: String,
        path: Any,
        from: Any,
        from_y: Any,
        to: Any,
        to_y: Any,
        auto: bool,
        ends_near: Any,
        tol: Any,
        floor_y: Any,
        phase: String,
    }
}

strict_struct! {
    /// Object assertions.
    pub struct ObjectsCheck { exist: Vec<String>, absent: Vec<String>, min_count: Any, max_count: Any, count: Vec<CountItem> }
}

strict_struct! {
    /// How many of a kind.
    pub struct CountItem { kind: String, min: Any, max: Any }
}

strict_struct! {
    /// A golden view.
    pub struct ViewCheck { name: String, eye: Any, at: Any, fov: Any, max_diff: Any }
}

strict_struct! {
    /// Performance budgets.
    pub struct PerfCheck {
        players: Any,
        secs: Any,
        windows: Any,
        sim_tick_p95_us: Any,
        server_tick_p95_us: Any,
        server_tick_p99_us: Any,
        bytes_per_client_sec: Any,
        largest_datagram: Any,
        promoted_props_max: Any,
    }
}

strict_struct! {
    /// Nav graph replay budget.
    pub struct NavCheck { max_failures: Any }
}

strict_struct! {
    /// Sound checks.
    pub struct AudioCheck { scores: ScoresCheck, sounds: Vec<SoundCheck>, soundscape: Vec<SoundscapeCheck> }
}

strict_struct! {
    /// Loudness of every score the scene uses.
    pub struct ScoresCheck { lufs: Any, peak_max: Any }
}

strict_struct! {
    /// One sound held to numbers.
    pub struct SoundCheck { name: String, sound: String, lufs: Any, peak_max: Any, seam_max: Any, max_secs: Any }
}

strict_struct! {
    /// A soundscape run: what is heard where.
    pub struct SoundscapeCheck {
        name: String,
        sun: Any,
        rising: bool,
        biome: String,
        secs: Any,
        seed: Any,
        birds: bool,
        speed: Any,
        /// call -> `[min, max]` heard.
        calls: BTreeMap<String, [f64; 2]>,
        /// bed -> `[min, max]` gain.
        beds: BTreeMap<String, [f64; 2]>,
    }
}

strict_struct! {
    /// A scripted headless playthrough (`describe sim`).
    pub struct SimScenario {
        name: String,
        spawn_group: String,
        players: Vec<SimPlayer>,
        script: Vec<SimStep>,
        max_seconds: Any,
        settle_seconds: Any,
        expect: Vec<SimExpect>,
    }
}

strict_struct! {
    /// A scripted player.
    pub struct SimPlayer { id: String, character: String, spawn: Any }
}

strict_struct! {
    /// One step of a script.
    pub struct SimStep {
        player: String,
        walk: Any,
        wait: Any,
        hold: SimHold,
        approach: Any,
        look_at: Any,
        interact: Any,
        within: Any,
        timeout: Any,
        until_event: String,
    }
}

strict_struct! {
    /// Inputs held for a while.
    pub struct SimHold {
        forward: Any,
        strafe: Any,
        sprint: bool,
        crouch: bool,
        jump: bool,
        yaw_deg: Any,
        pitch_deg: Any,
        look_at: Any,
        interact: bool,
        attack: bool,
        reload: bool,
        switch: Any,
        seconds: Any,
    }
}

strict_struct! {
    /// What must hold at the end (or while it runs).
    pub struct SimExpect {
        event: String,
        no_event: String,
        var: String,
        player_var: String,
        of: String,
        ended: Any,
        not_ended: bool,
        hidden: String,
        shown: String,
        collision_disabled: String,
        collision_enabled: String,
        player: String,
        count: Any,
        min: Any,
        max: Any,
        eq: Any,
        ne: Any,
        gt: Any,
        gte: Any,
        lt: Any,
        lte: Any,
        near: Any,
        tol: Any,
        y: Any,
        prop: String,
        in_zone: String,
        not_in_zone: String,
        below_y: Any,
        y_lt: Any,
        y_gt: Any,
        tilt_gt: Any,
        tilt_lt: Any,
        moved: Any,
        held_by: Any,
        field: String,
        aimed_at: Any,
    }
}
