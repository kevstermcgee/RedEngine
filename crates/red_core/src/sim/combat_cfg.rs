//! The scene's optional `combat` block: how fights are *paced*, as data. Weapon numbers live in `weapons`; this is what happens around them.
//!
//! ```json
//! "combat": { "respawn_secs": 2, "spawn": "farthest", "spawn_protect_secs": 1.5, "regen_delay_secs": 4, "regen_per_sec": 25 }
//! ```
//!
//! - `respawn_secs`: how long a dead player waits (default 3).
//! - `spawn`: where the dead come back. `round_robin` (default) walks the spawn list; `farthest` picks the spawn farthest from every living
//!   opponent and not in an opponent's line of sight (what arena shooters do, so nobody is killed the moment they appear).
//! - `spawn_protect_secs`: invulnerability after (re)spawning, ended early by the player's first shot (default 0 = none).
//! - `regen_delay_secs` / `regen_per_sec`: health returns at that rate once nobody has hurt the player for that long (default off).
//!
//! Every number is quantised to whole simulation ticks, so the rules cannot depend on the frame rate.

use crate::sim::clock::{secs_to_ticks, TICK_RATE_HZ};
use crate::strict::check_keys;
use crate::weapons::RESPAWN_TICKS;
use serde_json::{Map, Value};

/// Where a dead player reappears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnPolicy {
    /// The next spawn point in the list (the original behaviour).
    RoundRobin,
    /// The spawn farthest from every living opponent, out of their sight where possible.
    Farthest,
}

/// The parsed `combat` block; the default is the engine's classic behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatConfig {
    /// Ticks a dead player waits before respawning.
    pub respawn_ticks: u64,
    /// Where they come back.
    pub spawn: SpawnPolicy,
    /// Ticks of invulnerability after a (re)spawn (`0` = none).
    pub protect_ticks: u64,
    /// Ticks without taking damage before health starts to return (`0` = never).
    pub regen_delay_ticks: u64,
    /// Health points per second once regeneration has started.
    pub regen_per_sec: u32,
}

impl Default for CombatConfig {
    fn default() -> Self {
        CombatConfig { respawn_ticks: RESPAWN_TICKS, spawn: SpawnPolicy::RoundRobin, protect_ticks: 0, regen_delay_ticks: 0, regen_per_sec: 0 }
    }
}

/// Keys of the `combat` block.
pub const COMBAT_KEYS: &[&str] = &["respawn_secs", "spawn", "spawn_protect_secs", "regen_delay_secs", "regen_per_sec"];

/// Regeneration is integer maths: each tick adds `regen_per_sec` to an accumulator and every `REGEN_UNIT` of it pays out one health point.
pub const REGEN_UNIT: u32 = TICK_RATE_HZ;

fn seconds(o: &Map<String, Value>, key: &str, min: f64, max: f64, errs: &mut Vec<String>) -> Option<f32> {
    let v = o.get(key)?;
    match v.as_f64().filter(|n| n.is_finite() && (min..=max).contains(n)) {
        Some(n) => Some(n as f32),
        None => {
            errs.push(format!("combat.{key}: must be a number from {min} to {max}"));
            None
        }
    }
}

/// Parses a scene's optional `combat` block; every problem is `combat.path: message` with a did-you-mean.
pub fn parse_combat(root: &Map<String, Value>) -> Result<CombatConfig, Vec<String>> {
    let mut cfg = CombatConfig::default();
    let Some(value) = root.get("combat") else { return Ok(cfg) };
    let Some(o) = value.as_object() else {
        return Err(vec!["combat: must be an object like {\"respawn_secs\": 2, \"spawn\": \"farthest\"}".to_string()]);
    };
    let mut errs = Vec::new();
    check_keys(&mut errs, "combat", o, COMBAT_KEYS);
    if let Some(s) = seconds(o, "respawn_secs", 0.0, 30.0, &mut errs) {
        cfg.respawn_ticks = secs_to_ticks(s) as u64;
    }
    if let Some(v) = o.get("spawn") {
        match v.as_str() {
            Some("round_robin") => cfg.spawn = SpawnPolicy::RoundRobin,
            Some("farthest") => cfg.spawn = SpawnPolicy::Farthest,
            _ => errs.push("combat.spawn: must be \"round_robin\" or \"farthest\"".to_string()),
        }
    }
    if let Some(s) = seconds(o, "spawn_protect_secs", 0.0, 10.0, &mut errs) {
        cfg.protect_ticks = secs_to_ticks(s) as u64;
    }
    if let Some(s) = seconds(o, "regen_delay_secs", 0.0, 60.0, &mut errs) {
        cfg.regen_delay_ticks = secs_to_ticks(s) as u64;
    }
    if let Some(v) = o.get("regen_per_sec") {
        match v.as_u64().filter(|n| *n <= 1000) {
            Some(n) => cfg.regen_per_sec = n as u32,
            None => errs.push("combat.regen_per_sec: must be a whole number from 0 to 1000".to_string()),
        }
    }
    if cfg.regen_per_sec > 0 && cfg.regen_delay_ticks == 0 && o.get("regen_delay_secs").is_none() {
        errs.push("combat.regen_per_sec: needs regen_delay_secs too (how long a player must go unhurt before health returns)".to_string());
    }
    if errs.is_empty() {
        Ok(cfg)
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(s: &str) -> Map<String, Value> {
        serde_json::from_str::<Value>(s).unwrap().as_object().unwrap().clone()
    }

    #[test]
    fn no_block_means_the_classic_behaviour() {
        let c = parse_combat(&root("{}")).unwrap();
        assert_eq!(c, CombatConfig::default());
        assert_eq!((c.protect_ticks, c.regen_delay_ticks, c.spawn), (0, 0, SpawnPolicy::RoundRobin));
        assert_eq!(c.respawn_ticks, RESPAWN_TICKS);
    }

    #[test]
    fn every_field_is_read_and_quantised_to_ticks() {
        let c = parse_combat(&root(r#"{"combat":{"respawn_secs":1.5,"spawn":"farthest","spawn_protect_secs":2,"regen_delay_secs":4,"regen_per_sec":25}}"#))
            .unwrap();
        assert_eq!((c.respawn_ticks, c.protect_ticks, c.regen_delay_ticks, c.regen_per_sec), (90, 120, 240, 25));
        assert_eq!(c.spawn, SpawnPolicy::Farthest);
    }

    #[test]
    fn mistakes_name_the_field_and_the_fix() {
        let e = parse_combat(&root(r#"{"combat":{"respawn":2,"spawn":"random","spawn_protect_secs":99,"regen_per_sec":5}}"#)).unwrap_err().join("\n");
        assert!(e.contains("combat.respawn: unknown field — did you mean") && e.contains("`respawn_secs`"), "{e}");
        assert!(e.contains("combat.spawn: must be"), "{e}");
        assert!(e.contains("combat.spawn_protect_secs: must be a number from 0 to 10"), "{e}");
        assert!(e.contains("combat.regen_per_sec: needs regen_delay_secs"), "{e}");
        assert!(parse_combat(&root(r#"{"combat":3}"#)).is_err());
    }
}
