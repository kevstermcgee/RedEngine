//! The weapons as **data**: the baseball bat (melee) and the reusable firearm arsenal (hitscan).
//!
//! Pure data and rules, no window/GPU types: which weapons exist, their numbers (reach, range, damage, fire rate,
//! knock-back) and the players' [`Ammo`]. A scene tunes them with its top-level `weapons` block
//! ([`WeaponConfig`]: the starting weapon, the ladder, the bat's damage, and the ammunition every firearm draws on: `"infinite"` or
//! `{loaded, capacity, reserve}`), so the limited-ammo game is a scene edit, not a code change. The authoritative rules that *use* them live in
//! `sim::interact` (server and scenarios); the models live in `viewer::build_held_parts` (bat) and `firearms::build_firearm_parts`; the
//! synthesized sounds in `sfx`; the single-player wiring in `bin/re2/weapons.rs`.
//!
//! The loadout arsenal of [`crate::arsenal`] adds twenty more variants after the legacy eleven; [`Weapon::ALL`] (the prototype's scroll order) is
//! unchanged, [`Weapon::ROSTER`] is every weapon in wire order.
//!
//! There is no special weapon: every firearm is a [`FirearmSpec`] row and a procedural model, so a new gun is one row and one shape
//! (ADR 2026-09-28-remove-the-revolver removed the one that had its own code path).

/// A weapon the human can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weapon {
    /// The wooden bat: melee, the primary weapon and the one the human starts with.
    Bat,
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
    // ---- the loadout arsenal (`crate::arsenal`); the legacy prototype's scroll order ([`Weapon::ALL`]) never reaches these ----
    /// Combat knife: the default melee weapon of a loadout match.
    Knife,
    /// Camp hatchet.
    Hatchet,
    /// Single-stack .45 pistol.
    Bulldog,
    /// Large-frame .50 pistol.
    HandCannon,
    /// Six-shot .357 revolver.
    Marshal,
    /// .45 submachine gun.
    Stinger,
    /// Personal defence weapon with a 50-round magazine.
    Ranger,
    /// Very fast .45 submachine gun.
    Wasp,
    /// 7.62 battle rifle.
    Ironside,
    /// Semi-automatic sniper rifle.
    Gale,
    /// Bolt-action .338 anti-personnel sniper rifle.
    Sentinel,
    /// Semi-automatic combat shotgun.
    Auto12,
    /// Sawed-off double-barrel shotgun.
    Coach,
    /// Belt-fed machine gun.
    Hammer,
    /// Shoulder-fired rocket launcher.
    Lancer,
    /// Single-shot 40 mm grenade launcher.
    Thumper,
    /// Fragmentation grenade.
    Frag,
    /// Flashbang.
    Flash,
    /// Smoke grenade.
    Smoke,
    /// Incendiary grenade.
    Incendiary,
}

impl Weapon {
    /// Every weapon, in scroll order. A weapon's number on the wire is its index here, so the order is part of the protocol.
    pub const ALL: [Weapon; 11] = [
        Weapon::Bat,
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

    /// Every weapon of the loadout arsenal, in wire order (the first eleven are [`Weapon::ALL`], so legacy numbers never changed).
    pub const ROSTER: [Weapon; 31] = [
        Weapon::Bat,
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
        Weapon::Knife,
        Weapon::Hatchet,
        Weapon::Bulldog,
        Weapon::HandCannon,
        Weapon::Marshal,
        Weapon::Stinger,
        Weapon::Ranger,
        Weapon::Wasp,
        Weapon::Ironside,
        Weapon::Gale,
        Weapon::Sentinel,
        Weapon::Auto12,
        Weapon::Coach,
        Weapon::Hammer,
        Weapon::Lancer,
        Weapon::Thumper,
        Weapon::Frag,
        Weapon::Flash,
        Weapon::Smoke,
        Weapon::Incendiary,
    ];

    /// The ten firearms shipped with the reusable shooter prototype.
    pub const FIREARMS: [Weapon; 10] = [
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

    /// The next weapon in scroll order (wraps around); `-1` goes the other way.
    pub fn cycle(self, direction: i32) -> Weapon {
        let i = Weapon::ALL.iter().position(|w| *w == self).unwrap_or(0) as i32;
        Weapon::ALL[(i + direction.signum()).rem_euclid(Weapon::ALL.len() as i32) as usize]
    }

    /// The weapon's number on the wire (its index in [`Weapon::ROSTER`]).
    pub fn wire(self) -> u8 {
        Weapon::ROSTER.iter().position(|w| *w == self).unwrap_or(0) as u8
    }

    /// The inverse of [`wire`](Self::wire); an unknown number is the bat.
    pub fn from_wire(v: u8) -> Weapon {
        Weapon::ROSTER.get(v as usize).copied().unwrap_or(Weapon::Bat)
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Weapon::Bat => "baseball bat",
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
            Weapon::Knife => "combat knife",
            Weapon::Hatchet => "camp hatchet",
            Weapon::Bulldog => "Bulldog .45",
            Weapon::HandCannon => "Hand Cannon .50",
            Weapon::Marshal => "Marshal .357",
            Weapon::Stinger => "Stinger SMG",
            Weapon::Ranger => "Ranger PDW",
            Weapon::Wasp => "Wasp SMG",
            Weapon::Ironside => "Ironside battle rifle",
            Weapon::Gale => "Gale sniper",
            Weapon::Sentinel => "Sentinel .338",
            Weapon::Auto12 => "Auto-12 shotgun",
            Weapon::Coach => "Coach gun",
            Weapon::Hammer => "Hammer MG",
            Weapon::Lancer => "Lancer rocket launcher",
            Weapon::Thumper => "Thumper grenade launcher",
            Weapon::Frag => "frag grenade",
            Weapon::Flash => "flashbang",
            Weapon::Smoke => "smoke grenade",
            Weapon::Incendiary => "incendiary grenade",
        }
    }

    /// Parses a stable scene-facing weapon name.
    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase().replace([' ', '_'], "-");
        match normalized.as_str() {
            "bat" | "baseball-bat" => Some(Weapon::Bat),
            "pistol" => Some(Weapon::Pistol),
            "machine-pistol" => Some(Weapon::MachinePistol),
            "smg" => Some(Weapon::Smg),
            "carbine" => Some(Weapon::Carbine),
            "rifle" => Some(Weapon::Rifle),
            "bullpup" => Some(Weapon::Bullpup),
            "marksman" | "marksman-rifle" => Some(Weapon::Marksman),
            "shotgun" => Some(Weapon::Shotgun),
            "lmg" => Some(Weapon::Lmg),
            "scout" | "scout-rifle" => Some(Weapon::Scout),
            "knife" | "combat-knife" => Some(Weapon::Knife),
            "hatchet" => Some(Weapon::Hatchet),
            "bulldog" => Some(Weapon::Bulldog),
            "hand-cannon" | "handcannon" => Some(Weapon::HandCannon),
            "marshal" | "revolver-357" => Some(Weapon::Marshal),
            "stinger" => Some(Weapon::Stinger),
            "ranger" | "pdw" => Some(Weapon::Ranger),
            "wasp" => Some(Weapon::Wasp),
            "ironside" => Some(Weapon::Ironside),
            "gale" => Some(Weapon::Gale),
            "sentinel" => Some(Weapon::Sentinel),
            "auto12" | "auto-12" => Some(Weapon::Auto12),
            "coach" | "coach-gun" => Some(Weapon::Coach),
            "hammer" | "hammer-mg" => Some(Weapon::Hammer),
            "lancer" | "rocket-launcher" => Some(Weapon::Lancer),
            "thumper" | "grenade-launcher" => Some(Weapon::Thumper),
            "frag" | "frag-grenade" => Some(Weapon::Frag),
            "flash" | "flashbang" => Some(Weapon::Flash),
            "smoke" | "smoke-grenade" => Some(Weapon::Smoke),
            "incendiary" => Some(Weapon::Incendiary),
            _ => None,
        }
    }

    /// True for every gun (not the melee weapons and not grenades).
    pub fn is_firearm(self) -> bool {
        self.is_gun()
    }

    /// Perspective magnification while aiming; irons stay modest, precision rifles zoom further.
    pub fn aim_magnification(self) -> f32 {
        match self {
            Weapon::Marksman => 2.0,
            Weapon::Scout => 2.5,
            Weapon::Bat => 1.0,
            Weapon::Pistol | Weapon::MachinePistol | Weapon::Smg | Weapon::Carbine | Weapon::Rifle | Weapon::Bullpup | Weapon::Shotgun | Weapon::Lmg => 1.25,
            other => other.kit().zoom,
        }
    }

    /// Whether holding the trigger repeats shots at the authoritative cooldown.
    pub fn automatic(self) -> bool {
        match self {
            Weapon::MachinePistol | Weapon::Smg | Weapon::Carbine | Weapon::Rifle | Weapon::Bullpup | Weapon::Lmg => true,
            Weapon::Bat | Weapon::Pistol | Weapon::Marksman | Weapon::Shotgun | Weapon::Scout => false,
            other => other.kit().auto,
        }
    }

    /// Number of deterministic rays emitted by one trigger action.
    pub fn pellets(self) -> u32 {
        match self {
            Weapon::Shotgun => 9,
            Weapon::Bat
            | Weapon::Pistol
            | Weapon::MachinePistol
            | Weapon::Smg
            | Weapon::Carbine
            | Weapon::Rifle
            | Weapon::Bullpup
            | Weapon::Marksman
            | Weapon::Lmg
            | Weapon::Scout => 1,
            other => other.kit().pellets.max(1) as u32,
        }
    }

    /// Shared pellet cone; the center plus eight ring samples, with no frame-dependent RNG.
    pub fn shot_direction(self, forward: glam::Vec3, pellet: u32) -> glam::Vec3 {
        if self.pellets() == 1 || pellet == 0 {
            return forward;
        }
        let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let angle = (pellet - 1) as f32 * std::f32::consts::TAU / 8.0;
        (forward + (right * angle.cos() + up * angle.sin()) * 0.055).normalize()
    }

    /// Split configured shot damage across pellets without increasing the total.
    pub fn pellet_damage(self, total: u32, pellet: u32) -> u32 {
        total / self.pellets() + u32::from(pellet < total % self.pellets())
    }

    /// Shooter-facing tuning shared by offline play and the authoritative server.
    pub fn firearm(self) -> Option<FirearmSpec> {
        let spec = match self {
            Weapon::Bat => return None,
            Weapon::Pistol => FirearmSpec::new(26, 0.28, 70.0, 22.0, 0.55),
            Weapon::MachinePistol => FirearmSpec::new(18, 0.12, 55.0, 18.0, 0.42),
            Weapon::Smg => FirearmSpec::new(20, 0.10, 62.0, 20.0, 0.38),
            Weapon::Carbine => FirearmSpec::new(28, 0.15, 95.0, 28.0, 0.52),
            Weapon::Rifle => FirearmSpec::new(32, 0.18, 110.0, 32.0, 0.64),
            Weapon::Bullpup => FirearmSpec::new(30, 0.16, 100.0, 30.0, 0.56),
            Weapon::Marksman => FirearmSpec::new(48, 0.36, 150.0, 40.0, 0.82),
            Weapon::Shotgun => FirearmSpec::new(62, 0.72, 32.0, 58.0, 1.15),
            Weapon::Lmg => FirearmSpec::new(27, 0.13, 105.0, 36.0, 0.72),
            Weapon::Scout => FirearmSpec::new(70, 0.85, 180.0, 48.0, 0.95),
            other if other.is_gun() => {
                let k = other.kit();
                FirearmSpec {
                    damage: k.damage as u32,
                    cooldown_ticks: k.cooldown_ticks(),
                    range: k.range,
                    impulse: k.damage as f32 * 0.4,
                    recoil: (k.kick / 4.0).clamp(0.3, 1.2),
                }
            }
            _ => return None,
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

/// How long the muzzle flash is visible, s.
pub const MUZZLE_FLASH_TIME: f32 = 0.06;
/// How long the recoil kick takes to settle, s.
pub const RECOIL_TIME: f32 = 0.30;

/// How far the bat reaches, m.
pub const BAT_REACH: f32 = 2.2;
/// Hit points a player starts (and respawns) with.
pub const PLAYER_MAX_HP: u32 = 100;
/// Damage of one bat hit on a player, by default.
pub const BAT_DAMAGE: u32 = 20;
/// How long a dead player waits before respawning, ticks (3 s).
pub const RESPAWN_TICKS: u64 = crate::sim::clock::secs_to_ticks(3.0) as u64;

/// Most rungs a weapon ladder may have.
pub const MAX_LADDER: usize = 16;

/// The per-scene weapon numbers (`weapons` in the scene; defaults are the numbers above).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponConfig {
    /// Weapon equipped on spawn and respawn.
    pub starting_weapon: Weapon,
    /// Damage of a bat hit on a player.
    pub bat_damage: u32,
    /// What every firearm draws on when a player starts (or is handed the next rung): [`Ammo::Infinite`] unless the scene says otherwise.
    pub ammo: Ammo,
    /// The weapon ladder (Gun Game): the first `ladder_len` entries. A player carries `ladder[kills]`; empty = ordinary play.
    ladder: [Weapon; MAX_LADDER],
    ladder_len: usize,
}

impl Default for WeaponConfig {
    fn default() -> Self {
        WeaponConfig { starting_weapon: Weapon::Bat, bat_damage: BAT_DAMAGE, ammo: DEFAULT_AMMO, ladder: [Weapon::Bat; MAX_LADDER], ladder_len: 0 }
    }
}

impl WeaponConfig {
    /// Damage for any weapon: the bat's is the scene's `weapons.bat.damage`, a firearm's is its row in [`Weapon::firearm`].
    pub fn damage(&self, weapon: Weapon) -> u32 {
        match weapon {
            Weapon::Bat => self.bat_damage,
            other => other.firearm().map_or(0, |s| s.damage),
        }
    }

    /// The ladder, first rung first (empty when the scene has none).
    pub fn ladder(&self) -> &[Weapon] {
        &self.ladder[..self.ladder_len]
    }

    /// Whether a weapon ladder is in force (weapons then follow kills and cannot be switched by hand).
    pub fn has_ladder(&self) -> bool {
        self.ladder_len > 0
    }

    /// The weapon a player with `kills` kills carries: their rung on the ladder (the last one once past the top), else the starting weapon.
    pub fn weapon_for_kills(&self, kills: u32) -> Weapon {
        match self.ladder().get((kills as usize).min(self.ladder_len.saturating_sub(1))) {
            Some(w) => *w,
            None => self.starting_weapon,
        }
    }

    /// A configuration whose ladder is `rungs` (at most [`MAX_LADDER`]; extra entries are ignored): for tests and code-built matches.
    pub fn with_ladder(mut self, rungs: &[Weapon]) -> Self {
        self.ladder_len = rungs.len().min(MAX_LADDER);
        self.ladder[..self.ladder_len].copy_from_slice(&rungs[..self.ladder_len]);
        self
    }
}

const WEAPONS_KEYS: &[&str] = &["starting", "bat", "ammo", "ladder"];
const BAT_KEYS: &[&str] = &["damage"];
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

/// Why a scene that still names the removed weapon is told what to write instead (a bare "expected a weapon name" would leave the author guessing).
fn removed_weapon_hint(name: &str) -> Option<&'static str> {
    (name.trim().eq_ignore_ascii_case("revolver"))
        .then_some("the silver revolver was removed from the engine (ADR 2026-09-28-remove-the-revolver): use `pistol`")
}

/// Parses a scene's optional `weapons` block; every problem is `weapons.path: message` with a did-you-mean.
pub fn parse_weapons(root: &JsonMap) -> Result<WeaponConfig, Vec<String>> {
    use crate::strict::check_keys;
    use serde_json::Value;
    let mut cfg = WeaponConfig::default();
    let Some(w) = root.get("weapons") else { return Ok(cfg) };
    let Some(w) = w.as_object() else {
        return Err(vec![
            "weapons: must be an object like {\"ammo\": {\"loaded\": 30, \"reserve\": 90}, \"ladder\": [\"pistol\", \"smg\", \"bat\"]}".to_string()
        ]);
    };
    let mut errs = Vec::new();
    if w.contains_key("revolver") {
        errs.push("weapons.revolver: the silver revolver was removed from the engine (ADR 2026-09-28-remove-the-revolver); the ammunition setting it carried is now `weapons.ammo`, shared by every firearm".to_string());
    }
    let known: JsonMap = w.iter().filter(|(k, _)| k.as_str() != "revolver").map(|(k, v)| (k.clone(), v.clone())).collect();
    check_keys(&mut errs, "weapons", &known, WEAPONS_KEYS);
    if let Some(value) = w.get("starting") {
        match value.as_str().and_then(Weapon::parse) {
            Some(weapon) => cfg.starting_weapon = weapon,
            None => errs.push(format!(
                "weapons.starting: {}",
                value
                    .as_str()
                    .and_then(removed_weapon_hint)
                    .unwrap_or("expected bat, pistol, machine-pistol, smg, carbine, rifle, bullpup, marksman, shotgun, lmg, or scout")
            )),
        }
    }
    if let Some(value) = w.get("ladder") {
        match value.as_array() {
            Some(rungs) if !rungs.is_empty() && rungs.len() <= MAX_LADDER => {
                let mut parsed = Vec::with_capacity(rungs.len());
                for (i, rung) in rungs.iter().enumerate() {
                    match rung.as_str().and_then(Weapon::parse) {
                        Some(weapon) => parsed.push(weapon),
                        None => errs.push(format!(
                            "weapons.ladder[{i}]: {}",
                            rung.as_str()
                                .and_then(removed_weapon_hint)
                                .unwrap_or("expected a weapon name (bat, pistol, machine-pistol, smg, carbine, rifle, bullpup, marksman, shotgun, lmg, scout)")
                        )),
                    }
                }
                if parsed.len() == rungs.len() {
                    cfg = cfg.with_ladder(&parsed);
                    if !w.contains_key("starting") {
                        cfg.starting_weapon = parsed[0];
                    }
                }
            }
            _ => errs.push(format!("weapons.ladder: must be a list of 1 to {MAX_LADDER} weapon names, first rung first, e.g. [\"pistol\", \"smg\", \"bat\"]")),
        }
    }
    if let Some(b) = w.get("bat").and_then(Value::as_object) {
        check_keys(&mut errs, "weapons.bat", b, BAT_KEYS);
        cfg.bat_damage = whole(b, "damage", "weapons.bat", 10_000, BAT_DAMAGE, &mut errs);
    }
    match w.get("ammo") {
        None => {}
        Some(Value::String(s)) if s == "infinite" => cfg.ammo = Ammo::Infinite,
        Some(Value::Object(a)) => {
            check_keys(&mut errs, "weapons.ammo", a, AMMO_KEYS);
            let path = "weapons.ammo";
            let capacity = whole(a, "capacity", path, 1000, DEFAULT_MAGAZINE, &mut errs).max(1);
            let loaded = whole(a, "loaded", path, 1000, capacity, &mut errs).min(capacity);
            cfg.ammo = Ammo::Limited { loaded, capacity, reserve: whole(a, "reserve", path, 1000, DEFAULT_RESERVE, &mut errs) };
        }
        Some(_) => errs.push(format!(
            "weapons.ammo: must be \"infinite\" or {{\"loaded\": {DEFAULT_MAGAZINE}, \"capacity\": {DEFAULT_MAGAZINE}, \"reserve\": {DEFAULT_RESERVE}}}"
        )),
    }
    if errs.is_empty() {
        Ok(cfg)
    } else {
        Err(errs)
    }
}

/// What a player's firearms draw on. `Infinite` never runs out; `Limited` counts rounds in the magazine and
/// a reserve to reload from (a scene switches it on with `weapons.ammo`). One supply serves every firearm a player carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ammo {
    /// Never runs out.
    Infinite,
    /// A magazine of `loaded` rounds (up to `capacity`) and `reserve` loose rounds.
    Limited {
        /// Rounds in the magazine.
        loaded: u32,
        /// Magazine size.
        capacity: u32,
        /// Rounds in the pocket.
        reserve: u32,
    },
}

/// Rounds in a full magazine when a scene turns on limited ammunition without saying how big it is.
pub const DEFAULT_MAGAZINE: u32 = 30;
/// Loose rounds a player starts with when a scene turns on limited ammunition without saying how many.
pub const DEFAULT_RESERVE: u32 = 90;

/// The ammunition when a scene does not say otherwise: infinite. A scene sets
/// `"weapons": {"ammo": {"loaded": 30, "reserve": 90}}` for the limited game.
pub const DEFAULT_AMMO: Ammo = Ammo::Infinite;

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

    /// Refills the magazine from the reserve; returns how many rounds went in.
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

    /// True if the magazine is empty (never for infinite ammo).
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
/// Delay after a dry-fire click on an empty magazine, ticks. Design: 0.3 s.
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
        assert_eq!(Weapon::Bat.cycle(1), Weapon::Pistol);
        assert_eq!(Weapon::Scout.cycle(1), Weapon::Bat);
        assert_eq!(Weapon::Bat.cycle(-1), Weapon::Scout);
        assert_eq!(Weapon::Pistol.cycle(-3), Weapon::Bat, "only the direction matters");
        assert_eq!(Weapon::FIREARMS.len(), 10);
        assert!(Weapon::FIREARMS.iter().all(|w| w.is_firearm() && w.firearm().is_some()));
        assert_eq!(Weapon::ALL.len(), Weapon::FIREARMS.len() + 1, "the bat and the firearms are every weapon");
    }

    /// The wire numbers are the indices of `Weapon::ALL`: every weapon survives the round trip and no two share a number.
    #[test]
    fn every_weapon_has_its_own_wire_number_and_comes_back_from_it() {
        for (i, w) in Weapon::ALL.iter().enumerate() {
            assert_eq!(w.wire() as usize, i);
            assert_eq!(Weapon::from_wire(w.wire()), *w);
        }
        assert_eq!(Weapon::from_wire(200), Weapon::Bat, "an unknown number is the bat");
        assert_eq!(Weapon::Bat.wire(), 0, "the bat is number 0: a fresh player and a default are the bat");
    }

    #[test]
    fn a_scene_switches_to_limited_ammo_and_typos_are_caught() {
        let root = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap().as_object().unwrap().clone();
        assert_eq!(parse_weapons(&root("{}")).unwrap(), WeaponConfig::default());
        let cfg = parse_weapons(&root(r#"{"weapons":{"bat":{"damage":50},"ammo":{"loaded":2,"reserve":5}}}"#)).unwrap();
        assert_eq!(cfg.bat_damage, 50);
        assert_eq!(cfg.ammo, Ammo::Limited { loaded: 2, capacity: DEFAULT_MAGAZINE, reserve: 5 });
        assert_eq!(parse_weapons(&root(r#"{"weapons":{"ammo":"infinite"}}"#)).unwrap().ammo, Ammo::Infinite);
        assert_eq!(
            parse_weapons(&root(r#"{"weapons":{"ammo":{}}}"#)).unwrap().ammo,
            Ammo::Limited { loaded: DEFAULT_MAGAZINE, capacity: DEFAULT_MAGAZINE, reserve: DEFAULT_RESERVE },
            "no numbers at all: a full default magazine and reserve"
        );
        let e = parse_weapons(&root(r#"{"weapons":{"amo":{},"bat":{"dmg":1},"ammo":7}}"#)).unwrap_err().join("\n");
        assert!(e.contains("weapons.amo: unknown field") && e.contains("weapons.bat.dmg: unknown field") && e.contains("weapons.ammo: must be"), "{e}");
    }

    #[test]
    fn a_scene_that_still_names_the_removed_revolver_is_told_what_to_write() {
        let root = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap().as_object().unwrap().clone();
        let e = parse_weapons(&root(r#"{"weapons":{"revolver":{"damage":50,"ammo":"infinite"},"starting":"revolver","ladder":["pistol","revolver","bat"]}}"#))
            .unwrap_err()
            .join("\n");
        assert!(e.contains("weapons.revolver: the silver revolver was removed") && e.contains("`weapons.ammo`"), "{e}");
        assert!(e.contains("weapons.starting: the silver revolver was removed from the engine") && e.contains("use `pistol`"), "{e}");
        assert!(e.contains("weapons.ladder[1]: the silver revolver was removed"), "{e}");
        assert!(Weapon::parse("revolver").is_none());
    }

    #[test]
    fn a_scene_can_choose_any_starting_weapon() {
        let root = serde_json::from_str::<serde_json::Value>(r#"{"weapons":{"starting":"shotgun"}}"#).unwrap().as_object().unwrap().clone();
        assert_eq!(parse_weapons(&root).unwrap().starting_weapon, Weapon::Shotgun);
        let bad = serde_json::from_str::<serde_json::Value>(r#"{"weapons":{"starting":"rocket-sock"}}"#).unwrap().as_object().unwrap().clone();
        assert!(parse_weapons(&bad).unwrap_err()[0].contains("weapons.starting"));
    }

    #[test]
    fn a_ladder_maps_kills_to_weapons_and_stops_at_the_top() {
        let root = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap().as_object().unwrap().clone();
        assert!(!WeaponConfig::default().has_ladder());
        assert_eq!(WeaponConfig::default().weapon_for_kills(9), Weapon::Bat, "no ladder: the starting weapon");
        let cfg = parse_weapons(&root(r#"{"weapons":{"ladder":["pistol","smg","Scout","bat"]}}"#)).unwrap();
        assert_eq!(cfg.ladder(), &[Weapon::Pistol, Weapon::Smg, Weapon::Scout, Weapon::Bat]);
        assert_eq!(cfg.starting_weapon, Weapon::Pistol, "the first rung is the default starting weapon");
        assert_eq!([0, 1, 2, 3, 4, 40].map(|k| cfg.weapon_for_kills(k)), [Weapon::Pistol, Weapon::Smg, Weapon::Scout, Weapon::Bat, Weapon::Bat, Weapon::Bat]);
        let e = parse_weapons(&root(r#"{"weapons":{"ladder":["pistol","water-gun"]}}"#)).unwrap_err().join("\n");
        assert!(e.contains("weapons.ladder[1]: expected a weapon name"), "{e}");
        assert!(parse_weapons(&root(r#"{"weapons":{"ladder":[]}}"#)).is_err(), "an empty ladder is a typo, not a setting");
        assert!(parse_weapons(&root(r#"{"weapons":{"ladder":"pistol"}}"#)).is_err());
        let seventeen = format!(r#"{{"weapons":{{"ladder":[{}]}}}}"#, vec!["\"pistol\""; MAX_LADDER + 1].join(","));
        assert!(parse_weapons(&root(&seventeen)).is_err(), "at most {MAX_LADDER} rungs");
    }

    #[test]
    fn infinite_ammo_never_runs_out() {
        let mut a = Ammo::Infinite;
        assert!((0..10_000).all(|_| a.try_fire()));
        assert!(!a.is_empty());
        assert_eq!(DEFAULT_AMMO, Ammo::Infinite, "ammunition is unlimited unless a scene says otherwise");
    }

    #[test]
    fn limited_ammo_counts_down_clicks_when_empty_and_reloads_from_the_reserve() {
        let mut a = Ammo::Limited { loaded: 2, capacity: 6, reserve: 10 };
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
            ("dry fire", DRY_FIRE_COOLDOWN_TICKS, 0.3),
        ];
        for (name, ticks, design) in phases {
            assert!(ticks >= 3, "{name} is only {ticks} ticks long");
            let err = (ticks_to_secs(ticks) - design).abs();
            assert!(err <= TICK_DT * 0.5 + 1e-6, "{name}: {ticks} ticks = {:.1} ms vs design {:.1} ms", ticks_to_secs(ticks) * 1e3, design * 1e3);
        }
        // Every firearm's cadence is a whole number of ticks of at least a few (the spec rounds seconds to ticks).
        for w in Weapon::FIREARMS {
            let s = w.firearm().unwrap();
            assert!(s.cooldown_ticks >= 3, "{w:?} fires every {} ticks", s.cooldown_ticks);
        }
        // Click-to-hit latency: one tick of input quantisation plus the windup, well inside 50 ms x2.
        let click_to_hit_ms = (SWING_WINDUP_TICKS + 1) as f32 * TICK_DT * 1e3;
        assert!(click_to_hit_ms < 110.0, "{click_to_hit_ms}");
    }
}
