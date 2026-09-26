//! The demo game's weapons as **data**: the baseball bat (melee) and the silver revolver (hitscan).
//!
//! Pure data and rules, no window/GPU types: which weapons exist, their numbers (reach, range, damage, fire rate,
//! knock-back) and the revolver's [`Ammo`]. A scene tunes them with its top-level `weapons` block
//! ([`WeaponConfig`]: damage for each, and the revolver's ammo: `"infinite"` or `{loaded, capacity, reserve}`), so the
//! limited-ammo game is a scene edit, not a code change. The authoritative rules that *use* them live in
//! `sim::interact` (server and scenarios); the models live in `viewer::build_held_parts` (bat) and
//! `revolver::build_revolver_parts`; the synthesized sounds in `audio`; the single-player wiring in `bin/re2.rs`.

/// A weapon the human can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weapon {
    /// The wooden bat: melee, the primary weapon and the one the human starts with.
    Bat,
    /// The silver revolver: hitscan, one shot per click.
    Revolver,
    /// Compact service pistol.
    Pistol,
    /// Fast-handling machine pistol.
    MachinePistol,
    /// Compact submachine gun.
    Smg,
    /// Short assault carbine.
    Carbine,
    /// Full-length assault rifle.
    Rifle,
    /// Bullpup rifle.
    Bullpup,
    /// Semi-automatic marksman rifle.
    Marksman,
    /// Pump-action combat shotgun (single hitscan prototype projectile).
    Shotgun,
    /// Belt-fed light machine gun.
    Lmg,
    /// Lightweight scout rifle.
    Scout,
}

impl Weapon {
    /// Every weapon, in scroll order.
    pub const ALL: [Weapon; 12] = [
        Weapon::Bat,
        Weapon::Revolver,
        Weapon::Pistol,
        Weapon::MachinePistol,
        Weapon::Smg,
        Weapon::Carbine,
        Weapon::Rifle,
        Weapon::Bullpup,
        Weapon::Marksman,
        Weapon::Shotgun,
        Weapon::Lmg,
        Weapon::Scout,
    ];

    /// The ten firearms shipped with the reusable shooter prototype.
    pub const FIREARMS: [Weapon; 11] = [
        Weapon::Pistol,
        Weapon::Revolver,
        Weapon::MachinePistol,
        Weapon::Smg,
        Weapon::Carbine,
        Weapon::Rifle,
        Weapon::Bullpup,
        Weapon::Marksman,
        Weapon::Shotgun,
        Weapon::Lmg,
        Weapon::Scout,
    ];

    /// The next weapon in scroll order (wraps around); `-1` goes the other way.
    pub fn cycle(self, direction: i32) -> Weapon {
        let i = Weapon::ALL.iter().position(|w| *w == self).unwrap_or(0) as i32;
        Weapon::ALL[(i + direction.signum()).rem_euclid(Weapon::ALL.len() as i32) as usize]
    }

    /// The weapon's number on the wire (its index in [`Weapon::ALL`]).
    pub fn wire(self) -> u8 {
        Weapon::ALL.iter().position(|w| *w == self).unwrap_or(0) as u8
    }

    /// The inverse of [`wire`](Self::wire); an unknown number is the bat.
    pub fn from_wire(v: u8) -> Weapon {
        Weapon::ALL.get(v as usize).copied().unwrap_or(Weapon::Bat)
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Weapon::Bat => "baseball bat",
            Weapon::Revolver => "silver revolver",
            Weapon::Pistol => "R9 service pistol",
            Weapon::MachinePistol => "Viper machine pistol",
            Weapon::Smg => "Ember SMG",
            Weapon::Carbine => "Rook carbine",
            Weapon::Rifle => "Redline rifle",
            Weapon::Bullpup => "Kestrel bullpup",
            Weapon::Marksman => "Longbow marksman rifle",
            Weapon::Shotgun => "Breach shotgun",
            Weapon::Lmg => "Atlas LMG",
            Weapon::Scout => "Warden scout rifle",
        }
    }

    /// True for every ranged weapon.
    pub fn is_firearm(self) -> bool {
        self != Weapon::Bat
    }

    /// Shooter-facing tuning shared by offline play and the authoritative server.
    pub fn firearm(self) -> Option<FirearmSpec> {
        let spec = match self {
            Weapon::Bat => return None,
            Weapon::Pistol => FirearmSpec::new(26, 0.28, 70.0, 22.0, 0.55),
            Weapon::Revolver => FirearmSpec::new(25, REVOLVER_COOLDOWN, REVOLVER_RANGE, REVOLVER_IMPULSE, 1.0),
            Weapon::MachinePistol => FirearmSpec::new(18, 0.12, 55.0, 18.0, 0.42),
            Weapon::Smg => FirearmSpec::new(20, 0.10, 62.0, 20.0, 0.38),
            Weapon::Carbine => FirearmSpec::new(28, 0.15, 95.0, 28.0, 0.52),
            Weapon::Rifle => FirearmSpec::new(32, 0.18, 110.0, 32.0, 0.64),
            Weapon::Bullpup => FirearmSpec::new(30, 0.16, 100.0, 30.0, 0.56),
            Weapon::Marksman => FirearmSpec::new(48, 0.36, 150.0, 40.0, 0.82),
            Weapon::Shotgun => FirearmSpec::new(62, 0.72, 32.0, 58.0, 1.15),
            Weapon::Lmg => FirearmSpec::new(27, 0.13, 105.0, 36.0, 0.72),
            Weapon::Scout => FirearmSpec::new(70, 0.85, 180.0, 48.0, 0.95),
        };
        Some(spec)
    }
}

/// Pure gameplay tuning for one hitscan firearm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FirearmSpec {
    /// Damage dealt to a player.
    pub damage: u32,
    /// Minimum time between shots.
    pub cooldown_ticks: u32,
    /// Hitscan range in metres.
    pub range: f32,
    /// Impulse applied to loose props.
    pub impulse: f32,
    /// Relative camera/viewmodel recoil strength.
    pub recoil: f32,
}

impl FirearmSpec {
    const fn new(damage: u32, cooldown: f32, range: f32, impulse: f32, recoil: f32) -> Self {
        Self { damage, cooldown_ticks: secs_to_ticks(cooldown), range, impulse, recoil }
    }
}

/// Target seconds between revolver shots (a deliberate, hammer-back rhythm rather than a machine gun).
/// The simulation uses [`REVOLVER_COOLDOWN_TICKS`] (this rounded to a whole tick).
pub const REVOLVER_COOLDOWN: f32 = 0.42;
/// Furthest a bullet travels, m.
pub const REVOLVER_RANGE: f32 = 80.0;
/// Impulse a bullet gives a loose prop, N·s: enough to spin a crate, launch an apple.
pub const REVOLVER_IMPULSE: f32 = 40.0;
/// How long the muzzle flash is visible, s.
pub const MUZZLE_FLASH_TIME: f32 = 0.06;
/// How long the recoil kick takes to settle, s.
pub const RECOIL_TIME: f32 = 0.30;
/// Chambers in the cylinder (used once ammo is limited).
pub const REVOLVER_CYLINDER: u32 = 6;

/// How far the bat reaches, m.
pub const BAT_REACH: f32 = 2.2;
/// Hit points a player starts (and respawns) with.
pub const PLAYER_MAX_HP: u32 = 100;
/// Damage of one bat hit on a player, by default.
pub const BAT_DAMAGE: u32 = 20;
/// Damage of one revolver hit on a player, by default.
pub const REVOLVER_DAMAGE: u32 = 25;
/// How long a dead player waits before respawning, ticks (3 s).
pub const RESPAWN_TICKS: u64 = crate::sim::clock::secs_to_ticks(3.0) as u64;

/// The per-scene weapon numbers (`weapons` in the scene; defaults are the numbers above).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponConfig {
    /// Damage of a bat hit on a player.
    pub bat_damage: u32,
    /// Damage of a revolver hit on a player.
    pub revolver_damage: u32,
    /// What the revolver starts with.
    pub revolver_ammo: Ammo,
}

impl Default for WeaponConfig {
    fn default() -> Self {
        WeaponConfig { bat_damage: BAT_DAMAGE, revolver_damage: REVOLVER_DAMAGE, revolver_ammo: REVOLVER_AMMO }
    }
}

impl WeaponConfig {
    /// Damage for any weapon, preserving the scene's legacy bat/revolver overrides.
    pub fn damage(&self, weapon: Weapon) -> u32 {
        match weapon {
            Weapon::Bat => self.bat_damage,
            Weapon::Revolver => self.revolver_damage,
            other => other.firearm().map_or(0, |s| s.damage),
        }
    }
}

const WEAPONS_KEYS: &[&str] = &["bat", "revolver"];
const BAT_KEYS: &[&str] = &["damage"];
const REVOLVER_KEYS: &[&str] = &["damage", "ammo"];
const AMMO_KEYS: &[&str] = &["loaded", "capacity", "reserve"];

type JsonMap = serde_json::Map<String, serde_json::Value>;

fn whole(o: &JsonMap, key: &str, path: &str, max: u64, default: u32, errs: &mut Vec<String>) -> u32 {
    match o.get(key) {
        None => default,
        Some(v) => v.as_u64().filter(|d| *d <= max).map(|d| d as u32).unwrap_or_else(|| {
            errs.push(format!("{path}.{key}: must be a whole number from 0 to {max}"));
            default
        }),
    }
}

/// Parses a scene's optional `weapons` block; every problem is `weapons.path: message` with a did-you-mean.
pub fn parse_weapons(root: &JsonMap) -> Result<WeaponConfig, Vec<String>> {
    use crate::strict::check_keys;
    use serde_json::Value;
    let mut cfg = WeaponConfig::default();
    let Some(w) = root.get("weapons") else { return Ok(cfg) };
    let Some(w) = w.as_object() else {
        return Err(vec!["weapons: must be an object like {\"revolver\": {\"ammo\": {\"loaded\": 6, \"reserve\": 24}}}".to_string()]);
    };
    let mut errs = Vec::new();
    check_keys(&mut errs, "weapons", w, WEAPONS_KEYS);
    if let Some(b) = w.get("bat").and_then(Value::as_object) {
        check_keys(&mut errs, "weapons.bat", b, BAT_KEYS);
        cfg.bat_damage = whole(b, "damage", "weapons.bat", 10_000, BAT_DAMAGE, &mut errs);
    }
    if let Some(r) = w.get("revolver").and_then(Value::as_object) {
        check_keys(&mut errs, "weapons.revolver", r, REVOLVER_KEYS);
        cfg.revolver_damage = whole(r, "damage", "weapons.revolver", 10_000, REVOLVER_DAMAGE, &mut errs);
        match r.get("ammo") {
            None => {}
            Some(Value::String(s)) if s == "infinite" => cfg.revolver_ammo = Ammo::Infinite,
            Some(Value::Object(a)) => {
                check_keys(&mut errs, "weapons.revolver.ammo", a, AMMO_KEYS);
                let path = "weapons.revolver.ammo";
                let capacity = whole(a, "capacity", path, 1000, REVOLVER_CYLINDER, &mut errs).max(1);
                let loaded = whole(a, "loaded", path, 1000, capacity, &mut errs).min(capacity);
                cfg.revolver_ammo = Ammo::Limited { loaded, capacity, reserve: whole(a, "reserve", path, 1000, 24, &mut errs) };
            }
            Some(_) => errs.push("weapons.revolver.ammo: must be \"infinite\" or {\"loaded\": 6, \"capacity\": 6, \"reserve\": 24}".to_string()),
        }
    }
    if errs.is_empty() {
        Ok(cfg)
    } else {
        Err(errs)
    }
}

/// What the revolver carries. `Infinite` never runs out; `Limited` counts rounds in the cylinder and
/// a reserve to reload from (a scene switches it on with `weapons.revolver.ammo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ammo {
    /// Never runs out.
    Infinite,
    /// A cylinder of `loaded` rounds (up to `capacity`) and `reserve` loose rounds.
    Limited {
        /// Rounds in the cylinder.
        loaded: u32,
        /// Cylinder size.
        capacity: u32,
        /// Rounds in the pocket.
        reserve: u32,
    },
}

/// The revolver's ammunition when a scene does not say otherwise: infinite. A scene sets
/// `"weapons": {"revolver": {"ammo": {"loaded": 6, "reserve": 24}}}` for the limited game.
pub const REVOLVER_AMMO: Ammo = Ammo::Infinite;

impl Ammo {
    /// Spends one round; `false` (an empty click) if there is none to fire.
    pub fn try_fire(&mut self) -> bool {
        match self {
            Ammo::Infinite => true,
            Ammo::Limited { loaded, .. } if *loaded > 0 => {
                *loaded -= 1;
                true
            }
            Ammo::Limited { .. } => false,
        }
    }

    /// Refills the cylinder from the reserve; returns how many rounds went in.
    pub fn reload(&mut self) -> u32 {
        match self {
            Ammo::Infinite => 0,
            Ammo::Limited { loaded, capacity, reserve } => {
                let n = (*capacity - *loaded).min(*reserve);
                *loaded += n;
                *reserve -= n;
                n
            }
        }
    }

    /// True if the cylinder is empty (never for infinite ammo).
    pub fn is_empty(&self) -> bool {
        matches!(self, Ammo::Limited { loaded: 0, .. })
    }
}

// ---- Timing in simulation ticks -----------------------------------------------------------------
// Ticks are the source of truth (`sim::clock`, 60 Hz): the design seconds below are rounded to the
// nearest whole tick, and the *_SECS constants the animation uses are derived back from the ticks,
// so the swing you see is exactly the swing the simulation runs.

use crate::sim::clock::{secs_to_ticks, ticks_to_secs};

/// Bat swing windup (the hit lands when it ends), ticks. Design: 0.09 s.
pub const SWING_WINDUP_TICKS: u32 = secs_to_ticks(0.09);
/// Bat swing strike phase, ticks. Design: 0.11 s.
pub const SWING_STRIKE_TICKS: u32 = secs_to_ticks(0.11);
/// Bat swing recovery, ticks. Design: 0.16 s.
pub const SWING_RECOVER_TICKS: u32 = secs_to_ticks(0.16);
/// Whole bat swing, ticks.
pub const SWING_TOTAL_TICKS: u32 = SWING_WINDUP_TICKS + SWING_STRIKE_TICKS + SWING_RECOVER_TICKS;
/// Weapon switch (lower + raise), ticks. Design: 0.34 s.
pub const SWITCH_TICKS: u32 = secs_to_ticks(0.34);
/// Revolver shot-to-shot delay, ticks.
pub const REVOLVER_COOLDOWN_TICKS: u32 = secs_to_ticks(REVOLVER_COOLDOWN);
/// Delay after a dry-fire click on an empty cylinder, ticks. Design: 0.3 s.
pub const DRY_FIRE_COOLDOWN_TICKS: u32 = secs_to_ticks(0.3);

/// [`SWING_WINDUP_TICKS`] in seconds (animation).
pub const SWING_WINDUP_SECS: f32 = ticks_to_secs(SWING_WINDUP_TICKS);
/// [`SWING_STRIKE_TICKS`] in seconds (animation).
pub const SWING_STRIKE_SECS: f32 = ticks_to_secs(SWING_STRIKE_TICKS);
/// [`SWING_RECOVER_TICKS`] in seconds (animation).
pub const SWING_RECOVER_SECS: f32 = ticks_to_secs(SWING_RECOVER_TICKS);
/// [`SWITCH_TICKS`] in seconds (animation).
pub const SWITCH_SECS: f32 = ticks_to_secs(SWITCH_TICKS);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_cycles_through_the_weapons_both_ways() {
        assert_eq!(Weapon::Bat.cycle(1), Weapon::Revolver);
        assert_eq!(Weapon::Scout.cycle(1), Weapon::Bat);
        assert_eq!(Weapon::Bat.cycle(-1), Weapon::Scout);
        assert_eq!(Weapon::Revolver.cycle(-3), Weapon::Bat, "only the direction matters");
        assert_eq!(Weapon::FIREARMS.len(), 11);
        assert!(Weapon::FIREARMS.iter().all(|w| w.is_firearm() && w.firearm().is_some()));
    }

    #[test]
    fn a_scene_switches_the_revolver_to_limited_ammo_and_typos_are_caught() {
        let root = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap().as_object().unwrap().clone();
        assert_eq!(parse_weapons(&root("{}")).unwrap(), WeaponConfig::default());
        let cfg = parse_weapons(&root(r#"{"weapons":{"bat":{"damage":50},"revolver":{"damage":10,"ammo":{"loaded":2,"reserve":5}}}}"#)).unwrap();
        assert_eq!((cfg.bat_damage, cfg.revolver_damage), (50, 10));
        assert_eq!(cfg.revolver_ammo, Ammo::Limited { loaded: 2, capacity: 6, reserve: 5 });
        assert_eq!(parse_weapons(&root(r#"{"weapons":{"revolver":{"ammo":"infinite"}}}"#)).unwrap().revolver_ammo, Ammo::Infinite);
        let e = parse_weapons(&root(r#"{"weapons":{"revolvr":{},"bat":{"dmg":1},"revolver":{"ammo":7}}}"#)).unwrap_err().join("\n");
        assert!(e.contains("weapons.revolvr: unknown field") && e.contains("weapons.bat.dmg: unknown field") && e.contains("ammo: must be"), "{e}");
        assert_eq!(Weapon::from_wire(Weapon::Revolver.wire()), Weapon::Revolver);
    }

    #[test]
    fn infinite_ammo_never_runs_out() {
        let mut a = Ammo::Infinite;
        assert!((0..10_000).all(|_| a.try_fire()));
        assert!(!a.is_empty());
        assert_eq!(REVOLVER_AMMO, Ammo::Infinite, "the revolver is infinite for now");
    }

    #[test]
    fn limited_ammo_counts_down_clicks_when_empty_and_reloads_from_the_reserve() {
        let mut a = Ammo::Limited { loaded: 2, capacity: REVOLVER_CYLINDER, reserve: 10 };
        assert!(a.try_fire() && a.try_fire());
        assert!(!a.try_fire() && a.is_empty());
        assert_eq!(a.reload(), 6);
        assert_eq!(a, Ammo::Limited { loaded: 6, capacity: 6, reserve: 4 });
        assert_eq!(a.reload(), 0, "already full");
    }

    /// The chosen tick rate must keep every weapon phase meaningful: at least 3 ticks long and
    /// within half a tick of its design duration (the rounding error the player could ever feel).
    #[test]
    fn timings_survive_the_tick_rate() {
        use crate::sim::clock::{ticks_to_secs, TICK_DT};
        let phases = [
            ("swing windup", SWING_WINDUP_TICKS, 0.09),
            ("swing strike", SWING_STRIKE_TICKS, 0.11),
            ("swing recover", SWING_RECOVER_TICKS, 0.16),
            ("weapon switch", SWITCH_TICKS, 0.34),
            ("revolver cooldown", REVOLVER_COOLDOWN_TICKS, REVOLVER_COOLDOWN),
            ("dry fire", DRY_FIRE_COOLDOWN_TICKS, 0.3),
        ];
        for (name, ticks, design) in phases {
            assert!(ticks >= 3, "{name} is only {ticks} ticks long");
            let err = (ticks_to_secs(ticks) - design).abs();
            assert!(err <= TICK_DT * 0.5 + 1e-6, "{name}: {ticks} ticks = {:.1} ms vs design {:.1} ms", ticks_to_secs(ticks) * 1e3, design * 1e3);
        }
        // Click-to-hit latency: one tick of input quantisation plus the windup, well inside 50 ms x2.
        let click_to_hit_ms = (SWING_WINDUP_TICKS + 1) as f32 * TICK_DT * 1e3;
        assert!(click_to_hit_ms < 110.0, "{click_to_hit_ms}");
    }
}
