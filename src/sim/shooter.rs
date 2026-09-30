//! The loadout shooter's scene block and the match-wide state it adds to a [`MatchSim`](super::match_sim::MatchSim) (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! A scene opts in with a top-level `shooter` block; without one the world is the classic single-weapon arena and nothing here runs:
//!
//! ```json
//! "shooter": {
//!   "start": ["pistol", "knife"],
//!   "friendly_fire": false,
//!   "pickups": [ {"weapon": "rifle", "at": [4, 0.3, -8], "respawn_secs": 30}, {"ammo": true, "at": [0, 0.3, 2]} ]
//! }
//! ```
//!
//! Every player then carries a [`Kit`](super::kit::Kit): up to two guns each with its own magazine and reserve, one melee weapon and up to
//! two grenades. Weapons lie about the map ([`MapPickup`]) and drop from the dead ([`Dropped`]); rockets and grenades fly as
//! [`Projectile`](super::ordnance::Projectile)s; two teams keep score. This module is data and bookkeeping; the rules that act on it are in
//! `sim::kit` (what a player does) and `sim::ordnance` (what flies and explodes).

use super::kit::Kit;
use super::ordnance::{FxEvent, Projectile, Zone};
use crate::strict::check_keys;
use crate::weapons::Weapon;
use glam::Vec3;
use serde_json::{Map, Value};

/// Keys of the `shooter` block.
pub const SHOOTER_KEYS: &[&str] = &["start", "friendly_fire", "pickups"];
const PICKUP_KEYS: &[&str] = &["weapon", "ammo", "at", "respawn_secs"];

/// Most pickup spots a map may place.
pub const MAX_MAP_PICKUPS: usize = 96;
/// Most dropped weapons lying around at once (older ones vanish first).
pub const MAX_DROPPED: usize = 16;
/// Seconds a dropped weapon stays before it vanishes.
pub const DROP_LIFETIME_SECS: f32 = 40.0;
/// Most people on one team.
pub const MAX_TEAM: usize = 6;

/// One place the map puts a weapon (or an ammunition crate) for the taking.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupSpawn {
    /// The weapon lying there, or `None` for a crate of ammunition.
    pub weapon: Option<Weapon>,
    /// Where it lies (centre of the item).
    pub at: Vec3,
    /// Seconds until it is back after somebody takes it.
    pub respawn_secs: f32,
}

/// The parsed `shooter` block.
#[derive(Debug, Clone, PartialEq)]
pub struct ShooterConfig {
    /// What every player starts (and respawns) with, both teams alike: up to two guns, a melee weapon, up to two grenades.
    pub start: Vec<Weapon>,
    /// Whether bullets and blasts hurt teammates.
    pub friendly_fire: bool,
    /// Where the map's weapons and ammunition crates lie.
    pub pickups: Vec<PickupSpawn>,
}

impl Default for ShooterConfig {
    fn default() -> Self {
        ShooterConfig { start: vec![Weapon::Pistol, Weapon::Knife], friendly_fire: false, pickups: Vec::new() }
    }
}

impl ShooterConfig {
    /// The kit a player starts with.
    pub fn start_kit(&self) -> Kit {
        Kit::from_weapons(&self.start)
    }
}

/// Parses a scene's optional `shooter` block: `Ok(None)` when there is none.
pub fn parse_shooter(root: &Map<String, Value>) -> Result<Option<ShooterConfig>, Vec<String>> {
    let Some(value) = root.get("shooter") else { return Ok(None) };
    let Some(o) = value.as_object() else {
        return Err(vec!["shooter: must be an object like {\"start\": [\"pistol\", \"knife\"], \"pickups\": [...]}".to_string()]);
    };
    let mut errs = Vec::new();
    check_keys(&mut errs, "shooter", o, SHOOTER_KEYS);
    let mut cfg = ShooterConfig::default();
    if let Some(v) = o.get("start") {
        match v.as_array() {
            Some(list) if list.len() <= 5 => {
                let mut start = Vec::new();
                for (i, w) in list.iter().enumerate() {
                    match w.as_str().and_then(Weapon::parse) {
                        Some(w) => start.push(w),
                        None => errs.push(format!("shooter.start[{i}]: expected a weapon name such as \"pistol\", \"knife\", \"frag\"")),
                    }
                }
                let guns = start.iter().filter(|w| w.is_gun()).count();
                let melee = start.iter().filter(|w| w.is_melee()).count();
                let nades = start.iter().filter(|w| w.is_grenade()).count();
                if guns > 2 || melee > 1 || nades > 2 {
                    errs.push("shooter.start: at most 2 guns, 1 melee weapon and 2 grenades".to_string());
                } else {
                    cfg.start = start;
                }
            }
            _ => errs.push("shooter.start: must be a list of up to 5 weapon names".to_string()),
        }
    }
    if let Some(v) = o.get("friendly_fire") {
        match v.as_bool() {
            Some(b) => cfg.friendly_fire = b,
            None => errs.push("shooter.friendly_fire: must be true or false".to_string()),
        }
    }
    if let Some(v) = o.get("pickups") {
        match v.as_array() {
            Some(list) if list.len() <= MAX_MAP_PICKUPS => {
                for (i, p) in list.iter().enumerate() {
                    let path = format!("shooter.pickups[{i}]");
                    let Some(po) = p.as_object() else {
                        errs.push(format!("{path}: must be an object like {{\"weapon\": \"rifle\", \"at\": [x, y, z]}}"));
                        continue;
                    };
                    check_keys(&mut errs, &path, po, PICKUP_KEYS);
                    let ammo = po.get("ammo").and_then(Value::as_bool).unwrap_or(false);
                    let weapon = if ammo {
                        None
                    } else {
                        match po.get("weapon").and_then(Value::as_str).and_then(Weapon::parse) {
                            Some(w) => Some(w),
                            None => {
                                errs.push(format!("{path}.weapon: expected a weapon name (or \"ammo\": true for an ammunition crate)"));
                                continue;
                            }
                        }
                    };
                    let at = po.get("at").and_then(Value::as_array).filter(|a| a.len() == 3).and_then(|a| {
                        let n: Vec<f32> = a.iter().filter_map(|v| v.as_f64()).map(|v| v as f32).collect();
                        (n.len() == 3 && n.iter().all(|v| v.is_finite())).then(|| Vec3::new(n[0], n[1], n[2]))
                    });
                    let Some(at) = at else {
                        errs.push(format!("{path}.at: must be [x, y, z]"));
                        continue;
                    };
                    let respawn_secs = po.get("respawn_secs").and_then(Value::as_f64).unwrap_or(30.0) as f32;
                    if !(1.0..=600.0).contains(&respawn_secs) {
                        errs.push(format!("{path}.respawn_secs: must be 1 to 600"));
                    }
                    cfg.pickups.push(PickupSpawn { weapon, at, respawn_secs });
                }
            }
            _ => errs.push(format!("shooter.pickups: must be a list of at most {MAX_MAP_PICKUPS} pickup spots")),
        }
    }
    if errs.is_empty() {
        Ok(Some(cfg))
    } else {
        Err(errs)
    }
}

/// A map pickup spot and whether it is currently available.
#[derive(Debug, Clone)]
pub struct MapPickup {
    /// The spot as authored.
    pub spawn: PickupSpawn,
    /// Rounds in the magazine of the gun lying there.
    pub loaded: u16,
    /// Rounds in reserve with it.
    pub reserve: u16,
    /// The tick it is back (`None` = lying there now).
    pub taken_until: Option<u64>,
}

/// A weapon somebody dropped, or a dead player's gear.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dropped {
    /// Unique, growing id (how clients tell one from another).
    pub id: u16,
    /// The weapon.
    pub weapon: Weapon,
    /// Rounds in its magazine.
    pub loaded: u16,
    /// Rounds in reserve with it.
    pub reserve: u16,
    /// Where it lies.
    pub pos: Vec3,
    /// The tick it vanishes.
    pub expires: u64,
    /// Whoever dropped it cannot pick it straight back up until `ignore_until`.
    pub ignore_slot: u8,
    /// See `ignore_slot`.
    pub ignore_until: u64,
}

/// A kill, for the scoreboard and the victim's killcam.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KillRecord {
    /// The tick it happened.
    pub tick: u64,
    /// Who scored (equal to `victim` for a suicide).
    pub killer: u8,
    /// Who died.
    pub victim: u8,
    /// With what.
    pub weapon: Weapon,
    /// Whether it was a headshot.
    pub headshot: bool,
}

/// Everything a loadout match keeps beyond its players. Part of the match checksum.
#[derive(Debug, Clone)]
pub struct ArenaState {
    /// The scene's block.
    pub cfg: ShooterConfig,
    /// The map's pickup spots.
    pub pickups: Vec<MapPickup>,
    /// Weapons lying where they were dropped.
    pub dropped: Vec<Dropped>,
    /// Rockets and grenades in flight or at rest.
    pub projectiles: Vec<Projectile>,
    /// Smoke clouds and fires.
    pub zones: Vec<Zone>,
    /// Recent explosions and pops (so a client that missed one snapshot still hears it).
    pub fx: Vec<FxEvent>,
    /// The last kills, oldest first.
    pub kills: Vec<KillRecord>,
    /// Kills per team (index = team - 1).
    pub team_kills: [u32; 2],
    /// Next id for a projectile, a dropped weapon or an effect.
    pub next_id: u16,
}

/// How many kills the record keeps.
pub const KILL_LOG: usize = 16;

impl ArenaState {
    /// Fresh state for a scene's block.
    pub fn new(cfg: ShooterConfig) -> Self {
        let pickups = cfg
            .pickups
            .iter()
            .map(|p| {
                let (loaded, reserve) = p.weapon.map_or((0, 0), |w| {
                    let k = w.kit();
                    (k.mag, k.reserve / 2)
                });
                MapPickup { spawn: p.clone(), loaded, reserve, taken_until: None }
            })
            .collect();
        ArenaState {
            cfg,
            pickups,
            dropped: Vec::new(),
            projectiles: Vec::new(),
            zones: Vec::new(),
            fx: Vec::new(),
            kills: Vec::new(),
            team_kills: [0; 2],
            next_id: 1,
        }
    }

    /// A fresh id (skips 0, wraps).
    pub fn next_id(&mut self) -> u16 {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        id
    }

    /// Records a kill, keeping the last [`KILL_LOG`].
    pub fn record_kill(&mut self, k: KillRecord) {
        if self.kills.len() >= KILL_LOG {
            self.kills.remove(0);
        }
        self.kills.push(k);
    }

    /// A fold of the state that decides play (goes into the match checksum).
    pub fn state_hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for p in &self.pickups {
            mix(p.taken_until.map_or(0, |t| t + 1));
        }
        for d in &self.dropped {
            mix(d.weapon.wire() as u64 ^ (d.loaded as u64) << 8 ^ (d.pos.x.to_bits() as u64) << 16);
        }
        for p in &self.projectiles {
            mix(p.weapon.wire() as u64 ^ (p.pos.x.to_bits() as u64) << 8 ^ (p.pos.y.to_bits() as u64) << 24);
        }
        for z in &self.zones {
            mix(z.until ^ (z.pos.x.to_bits() as u64) << 8);
        }
        mix(self.team_kills[0] as u64 | (self.team_kills[1] as u64) << 32);
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(s: &str) -> Map<String, Value> {
        serde_json::from_str::<Value>(s).unwrap().as_object().unwrap().clone()
    }

    #[test]
    fn no_block_means_the_classic_arena() {
        assert_eq!(parse_shooter(&root("{}")).unwrap(), None);
    }

    #[test]
    fn a_block_sets_the_start_kit_and_the_pickups() {
        let c = parse_shooter(&root(
            r#"{"shooter":{"start":["pistol","knife"],"friendly_fire":true,"pickups":[{"weapon":"rifle","at":[1,0.3,2],"respawn_secs":20},{"ammo":true,"at":[0,0.3,0]}]}}"#,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(c.start, vec![Weapon::Pistol, Weapon::Knife]);
        assert!(c.friendly_fire);
        assert_eq!(c.pickups.len(), 2);
        assert_eq!(c.pickups[0].weapon, Some(Weapon::Rifle));
        assert_eq!(c.pickups[1].weapon, None);
        assert_eq!(c.pickups[1].respawn_secs, 30.0);
    }

    #[test]
    fn mistakes_are_named_with_their_path() {
        let e = parse_shooter(&root(
            r#"{"shooter":{"strat":[],"start":["pistol","rifle","smg","knife"],"pickups":[{"weapon":"zapper","at":[0,0,0]},{"weapon":"rifle","at":[0,0]}]}}"#,
        ))
        .unwrap_err()
        .join("\n");
        assert!(e.contains("shooter.strat: unknown field"), "{e}");
        assert!(e.contains("at most 2 guns"), "{e}");
        assert!(e.contains("shooter.pickups[0].weapon"), "{e}");
        assert!(e.contains("shooter.pickups[1].at"), "{e}");
    }
}
