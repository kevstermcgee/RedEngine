//! The kit arsenal: every weapon of a loadout shooter as one row of data (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! A [`KitSpec`] is what the authoritative sim (`sim::kit`), the bots and the client all read: class, damage, cadence, per-weapon magazine
//! and reserve, reload time, spread, movement penalty, scope, and for launchers and grenades the projectile numbers. The rows are tuned to
//! their real-world counterpart (magazine size, rate of fire, calibre class, reload time) and then scaled so a hundred hit points make a
//! fight last about as long as they do in a tactical shooter: a rifle kills in three body shots, a bolt-action sniper in one.
//!
//! Pure data: no window, GPU or socket. The legacy ten-firearm prototype keeps its own numbers in [`Weapon::firearm`]; a scene opts into
//! this table with a top-level `loadout` block (`sim::kit`).

use crate::sim::clock::secs_to_ticks;
use crate::weapons::Weapon;

/// What kind of thing a weapon is. Decides which inventory slot it takes and how the trigger behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Knife, hatchet, bat: the melee slot.
    Melee,
    /// Sidearm.
    Pistol,
    /// Submachine gun.
    Smg,
    /// Assault / battle rifle.
    Rifle,
    /// Semi-automatic marksman rifle.
    Marksman,
    /// Scoped precision rifle.
    Sniper,
    /// Shotgun.
    Shotgun,
    /// Machine gun.
    Lmg,
    /// Rocket or grenade launcher: a projectile with splash.
    Launcher,
    /// A thrown explosive or utility grenade.
    Grenade,
}

impl Class {
    /// Whether this class is a gun (occupies one of the two firearm slots).
    pub fn is_gun(self) -> bool {
        !matches!(self, Class::Melee | Class::Grenade)
    }

    /// Display name of the class.
    pub fn name(self) -> &'static str {
        match self {
            Class::Melee => "melee",
            Class::Pistol => "pistol",
            Class::Smg => "SMG",
            Class::Rifle => "rifle",
            Class::Marksman => "marksman rifle",
            Class::Sniper => "sniper rifle",
            Class::Shotgun => "shotgun",
            Class::Lmg => "machine gun",
            Class::Launcher => "launcher",
            Class::Grenade => "grenade",
        }
    }
}

/// What a thrown or launched projectile does when it goes off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    /// Nothing: this weapon is not a projectile weapon.
    None,
    /// Splash damage with falloff and line-of-sight.
    Explosive,
    /// Blinds whoever can see it go off.
    Flash,
    /// A cloud that blocks sight for a while.
    Smoke,
    /// A fire that burns whoever stands in it.
    Fire,
}

/// One weapon's numbers in a loadout match.
#[derive(Debug, Clone, Copy)]
pub struct KitSpec {
    /// Slot and trigger behaviour.
    pub class: Class,
    /// The real-world weapon this is tuned after (shown in the weapon list and the docs).
    pub real: &'static str,
    /// Damage to the body with every pellet landing (melee: a slash; launcher: a direct hit).
    pub damage: u16,
    /// Headshot multiplier in tenths (`40` = 4.0x).
    pub head_x10: u8,
    /// Seconds between shots (melee: a whole swing).
    pub cooldown: f32,
    /// Rounds in a full magazine (launchers: the tube; grenades: 1).
    pub mag: u16,
    /// Rounds carried in reserve when picked up fresh.
    pub reserve: u16,
    /// Seconds to reload (per shell when `per_shell`).
    pub reload: f32,
    /// Reloads one round at a time and can be interrupted by firing.
    pub per_shell: bool,
    /// Holding the trigger repeats shots.
    pub auto: bool,
    /// Rays per shot.
    pub pellets: u8,
    /// Half-angle of the hip-fire cone, degrees (standing still).
    pub spread_hip: f32,
    /// Half-angle while aiming down the sights, degrees.
    pub spread_ads: f32,
    /// Extra cone added while moving at full speed, degrees.
    pub spread_move: f32,
    /// Range at which damage begins to fall off, m.
    pub effective: f32,
    /// Range limit, m.
    pub range: f32,
    /// Share of the damage left at the range limit (1.0 = no falloff).
    pub far_mult: f32,
    /// Movement speed while this is in hand (1.0 = full).
    pub move_mult: f32,
    /// Magnification while aiming (1.0 = none); the field of view narrows by this factor.
    pub zoom: f32,
    /// A second zoom level (toggled by aiming again) for long scopes, or 0.
    pub zoom2: f32,
    /// Aiming shows a scope picture instead of iron sights.
    pub scoped: bool,
    /// Vertical kick per shot, degrees (the client applies it to the player's aim).
    pub kick: f32,
    /// Melee: reach in metres. Projectiles: fuse or flight speed is below.
    pub reach: f32,
    /// What a projectile does when it goes off.
    pub payload: Payload,
    /// Muzzle or throw speed of a projectile, m/s (0 for hitscan).
    pub speed: f32,
    /// Seconds until a grenade goes off (0 = on contact).
    pub fuse: f32,
    /// Blast / cloud / fire radius, m.
    pub radius: f32,
    /// Blast damage at the centre (fire: damage per second).
    pub blast: u16,
    /// Seconds a smoke cloud or fire lasts (flash: seconds of blindness at point blank).
    pub lasts: f32,
    /// Share of gravity a projectile feels (0 = flies straight like a rocket).
    pub gravity: f32,
}

impl KitSpec {
    const BASE: KitSpec = KitSpec {
        class: Class::Rifle,
        real: "",
        damage: 30,
        head_x10: 40,
        cooldown: 0.1,
        mag: 30,
        reserve: 90,
        reload: 2.4,
        per_shell: false,
        auto: true,
        pellets: 1,
        spread_hip: 1.6,
        spread_ads: 0.25,
        spread_move: 2.5,
        effective: 50.0,
        range: 140.0,
        far_mult: 0.75,
        move_mult: 0.95,
        zoom: 1.25,
        zoom2: 0.0,
        scoped: false,
        kick: 0.6,
        reach: 0.0,
        payload: Payload::None,
        speed: 0.0,
        fuse: 0.0,
        radius: 0.0,
        blast: 0,
        lasts: 0.0,
        gravity: 0.0,
    };

    /// Shot cadence in whole simulation ticks (at least 3).
    pub fn cooldown_ticks(&self) -> u32 {
        secs_to_ticks(self.cooldown).max(3)
    }

    /// Reload time in ticks.
    pub fn reload_ticks(&self) -> u32 {
        secs_to_ticks(self.reload).max(3)
    }

    /// Rounds per minute the cooldown allows (what a spec sheet would say).
    pub fn rpm(&self) -> f32 {
        60.0 / (self.cooldown_ticks() as f32 / crate::sim::clock::TICK_RATE_HZ as f32)
    }
}

const fn melee(real: &'static str, damage: u16, cooldown: f32, reach: f32, move_mult: f32) -> KitSpec {
    KitSpec {
        class: Class::Melee,
        real,
        damage,
        head_x10: 10,
        cooldown,
        mag: 0,
        reserve: 0,
        reload: 0.0,
        auto: false,
        pellets: 1,
        spread_hip: 0.0,
        spread_ads: 0.0,
        spread_move: 0.0,
        range: reach,
        effective: reach,
        far_mult: 1.0,
        move_mult,
        zoom: 1.0,
        kick: 0.0,
        reach,
        ..KitSpec::BASE
    }
}

const fn grenade(real: &'static str, payload: Payload, fuse: f32, radius: f32, blast: u16, lasts: f32) -> KitSpec {
    KitSpec {
        class: Class::Grenade,
        real,
        damage: 0,
        head_x10: 10,
        cooldown: 1.0,
        mag: 1,
        reserve: 0,
        reload: 0.0,
        auto: false,
        pellets: 1,
        spread_hip: 0.0,
        spread_ads: 0.0,
        spread_move: 0.0,
        range: 60.0,
        effective: 60.0,
        far_mult: 1.0,
        move_mult: 0.98,
        zoom: 1.0,
        kick: 0.0,
        payload,
        speed: 17.0,
        fuse,
        radius,
        blast,
        lasts,
        gravity: 1.0,
        ..KitSpec::BASE
    }
}

impl Weapon {
    /// This weapon's loadout numbers (every weapon has a row; the legacy ten firearms and the bat have theirs tuned for a loadout match).
    pub fn kit(self) -> KitSpec {
        use Class::*;
        let b = KitSpec::BASE;
        match self {
            // ---- melee -------------------------------------------------------------------------------------------------
            Weapon::Knife => melee("combat knife", 40, 0.45, 2.0, 1.04),
            Weapon::Hatchet => melee("camp hatchet", 55, 0.75, 2.1, 1.0),
            Weapon::Bat => melee("baseball bat", 48, 0.85, 2.3, 0.98),
            // ---- pistols -----------------------------------------------------------------------------------------------
            Weapon::Pistol => KitSpec { class: Pistol, real: "Glock 17, 9x19 mm", damage: 28, head_x10: 40, cooldown: 0.13, mag: 17, reserve: 68, reload: 2.2, auto: false, spread_hip: 1.0, spread_ads: 0.30, spread_move: 1.8, effective: 35.0, range: 90.0, far_mult: 0.6, move_mult: 1.0, zoom: 1.12, kick: 0.9, ..b },
            Weapon::Bulldog => KitSpec { class: Pistol, real: "Colt M1911, .45 ACP", damage: 36, head_x10: 40, cooldown: 0.16, mag: 8, reserve: 40, reload: 2.0, auto: false, spread_hip: 0.9, spread_ads: 0.28, spread_move: 1.8, effective: 35.0, range: 90.0, far_mult: 0.6, move_mult: 1.0, zoom: 1.12, kick: 1.3, ..b },
            Weapon::HandCannon => KitSpec { class: Pistol, real: "IMI Desert Eagle, .50 AE", damage: 58, head_x10: 35, cooldown: 0.36, mag: 7, reserve: 35, reload: 2.3, auto: false, spread_hip: 1.4, spread_ads: 0.35, spread_move: 2.4, effective: 40.0, range: 100.0, far_mult: 0.65, move_mult: 0.97, zoom: 1.12, kick: 3.4, ..b },
            Weapon::Marshal => KitSpec { class: Pistol, real: "Colt Python, .357 Magnum", damage: 52, head_x10: 35, cooldown: 0.42, mag: 6, reserve: 36, reload: 3.0, auto: false, spread_hip: 1.0, spread_ads: 0.25, spread_move: 2.0, effective: 45.0, range: 110.0, far_mult: 0.7, move_mult: 1.0, zoom: 1.12, kick: 2.6, ..b },
            Weapon::MachinePistol => KitSpec { class: Pistol, real: "Glock 18, 9x19 mm", damage: 19, head_x10: 35, cooldown: 0.06, mag: 33, reserve: 99, reload: 2.3, auto: true, spread_hip: 2.3, spread_ads: 0.9, spread_move: 2.6, effective: 22.0, range: 70.0, far_mult: 0.45, move_mult: 1.0, zoom: 1.1, kick: 0.7, ..b },
            // ---- submachine guns ---------------------------------------------------------------------------------------
            Weapon::Smg => KitSpec { class: Smg, real: "H&K MP5, 9x19 mm", damage: 25, head_x10: 40, cooldown: 0.075, mag: 30, reserve: 120, reload: 2.3, spread_hip: 1.5, spread_ads: 0.45, spread_move: 1.9, effective: 28.0, range: 100.0, far_mult: 0.5, move_mult: 1.0, zoom: 1.2, kick: 0.5, ..b },
            Weapon::Stinger => KitSpec { class: Smg, real: "H&K UMP45, .45 ACP", damage: 32, head_x10: 40, cooldown: 0.10, mag: 25, reserve: 100, reload: 2.5, spread_hip: 1.6, spread_ads: 0.5, spread_move: 2.0, effective: 30.0, range: 100.0, far_mult: 0.55, move_mult: 0.99, zoom: 1.2, kick: 0.7, ..b },
            Weapon::Ranger => KitSpec { class: Smg, real: "FN P90, 5.7x28 mm", damage: 24, head_x10: 40, cooldown: 0.067, mag: 50, reserve: 100, reload: 3.3, spread_hip: 1.7, spread_ads: 0.5, spread_move: 1.6, effective: 32.0, range: 110.0, far_mult: 0.55, move_mult: 1.0, zoom: 1.35, kick: 0.4, ..b },
            Weapon::Wasp => KitSpec { class: Smg, real: "KRISS Vector, .45 ACP", damage: 23, head_x10: 40, cooldown: 0.05, mag: 33, reserve: 99, reload: 2.1, spread_hip: 2.0, spread_ads: 0.7, spread_move: 2.2, effective: 25.0, range: 90.0, far_mult: 0.5, move_mult: 1.0, zoom: 1.2, kick: 0.45, ..b },
            // ---- rifles ------------------------------------------------------------------------------------------------
            Weapon::Carbine => KitSpec { class: Rifle, real: "Colt M4A1, 5.56x45 mm", damage: 32, head_x10: 40, cooldown: 0.075, mag: 30, reserve: 90, reload: 2.4, spread_hip: 1.7, spread_ads: 0.25, spread_move: 2.6, effective: 55.0, range: 160.0, far_mult: 0.7, move_mult: 0.96, zoom: 1.3, kick: 0.65, ..b },
            Weapon::Rifle => KitSpec { class: Rifle, real: "AK-47, 7.62x39 mm", damage: 36, head_x10: 40, cooldown: 0.10, mag: 30, reserve: 90, reload: 2.5, spread_hip: 1.9, spread_ads: 0.3, spread_move: 2.8, effective: 55.0, range: 160.0, far_mult: 0.7, move_mult: 0.94, zoom: 1.3, kick: 0.95, ..b },
            Weapon::Bullpup => KitSpec { class: Rifle, real: "Steyr AUG, 5.56x45 mm", damage: 30, head_x10: 40, cooldown: 0.085, mag: 30, reserve: 90, reload: 3.0, spread_hip: 1.6, spread_ads: 0.2, spread_move: 2.4, effective: 60.0, range: 170.0, far_mult: 0.7, move_mult: 0.96, zoom: 1.7, scoped: false, kick: 0.6, ..b },
            Weapon::Ironside => KitSpec { class: Rifle, real: "FN SCAR-H, 7.62x51 mm", damage: 40, head_x10: 40, cooldown: 0.11, mag: 20, reserve: 60, reload: 2.8, spread_hip: 2.0, spread_ads: 0.3, spread_move: 3.0, effective: 65.0, range: 180.0, far_mult: 0.72, move_mult: 0.93, zoom: 1.4, kick: 1.2, ..b },
            // ---- marksman and sniper rifles ----------------------------------------------------------------------------
            Weapon::Marksman => KitSpec { class: Marksman, real: "M14 EBR, 7.62x51 mm", damage: 52, head_x10: 35, cooldown: 0.20, mag: 10, reserve: 40, reload: 2.6, auto: false, spread_hip: 2.6, spread_ads: 0.12, spread_move: 3.5, effective: 80.0, range: 220.0, far_mult: 0.8, move_mult: 0.92, zoom: 2.0, kick: 1.5, ..b },
            Weapon::Gale => KitSpec { class: Sniper, real: "H&K G3SG/1, 7.62x51 mm", damage: 78, head_x10: 30, cooldown: 0.26, mag: 20, reserve: 60, reload: 3.3, auto: false, spread_hip: 3.4, spread_ads: 0.08, spread_move: 4.5, effective: 120.0, range: 300.0, far_mult: 0.9, move_mult: 0.85, zoom: 3.6, zoom2: 8.0, scoped: true, kick: 1.7, ..b },
            Weapon::Scout => KitSpec { class: Sniper, real: "Steyr Scout, 7.62x51 mm", damage: 88, head_x10: 28, cooldown: 1.15, mag: 10, reserve: 40, reload: 3.0, auto: false, spread_hip: 3.0, spread_ads: 0.05, spread_move: 4.0, effective: 130.0, range: 320.0, far_mult: 0.9, move_mult: 0.98, zoom: 3.0, zoom2: 7.0, scoped: true, kick: 2.6, ..b },
            Weapon::Sentinel => KitSpec { class: Sniper, real: "AI L115A3, .338 Lapua", damage: 118, head_x10: 40, cooldown: 1.45, mag: 5, reserve: 25, reload: 3.6, auto: false, spread_hip: 4.0, spread_ads: 0.03, spread_move: 5.5, effective: 200.0, range: 400.0, far_mult: 1.0, move_mult: 0.82, zoom: 4.0, zoom2: 10.0, scoped: true, kick: 3.6, ..b },
            // ---- shotguns ----------------------------------------------------------------------------------------------
            Weapon::Shotgun => KitSpec { class: Shotgun, real: "Mossberg 500, 12 gauge", damage: 160, head_x10: 15, cooldown: 0.85, mag: 8, reserve: 32, reload: 0.5, per_shell: true, auto: false, pellets: 9, spread_hip: 3.4, spread_ads: 2.6, spread_move: 1.2, effective: 9.0, range: 40.0, far_mult: 0.12, move_mult: 0.94, zoom: 1.15, kick: 3.0, ..b },
            Weapon::Auto12 => KitSpec { class: Shotgun, real: "Benelli M4, 12 gauge", damage: 120, head_x10: 15, cooldown: 0.24, mag: 7, reserve: 35, reload: 0.42, per_shell: true, auto: false, pellets: 6, spread_hip: 3.8, spread_ads: 3.0, spread_move: 1.2, effective: 8.0, range: 36.0, far_mult: 0.12, move_mult: 0.94, zoom: 1.15, kick: 2.2, ..b },
            Weapon::Coach => KitSpec { class: Shotgun, real: "Sawed-off double-barrel, 12 gauge", damage: 190, head_x10: 15, cooldown: 0.22, mag: 2, reserve: 16, reload: 2.4, auto: false, pellets: 12, spread_hip: 5.5, spread_ads: 4.6, spread_move: 1.0, effective: 6.0, range: 28.0, far_mult: 0.1, move_mult: 1.0, zoom: 1.1, kick: 4.4, ..b },
            // ---- machine guns ------------------------------------------------------------------------------------------
            Weapon::Lmg => KitSpec { class: Lmg, real: "FN M249 SAW, 5.56x45 mm", damage: 30, head_x10: 40, cooldown: 0.07, mag: 100, reserve: 200, reload: 5.6, spread_hip: 2.6, spread_ads: 0.7, spread_move: 3.6, effective: 55.0, range: 170.0, far_mult: 0.65, move_mult: 0.84, zoom: 1.25, kick: 0.8, ..b },
            Weapon::Hammer => KitSpec { class: Lmg, real: "IWI Negev, 5.56x45 mm", damage: 28, head_x10: 40, cooldown: 0.06, mag: 150, reserve: 150, reload: 5.8, spread_hip: 3.0, spread_ads: 0.95, spread_move: 3.8, effective: 50.0, range: 160.0, far_mult: 0.6, move_mult: 0.82, zoom: 1.25, kick: 0.75, ..b },
            // ---- launchers ---------------------------------------------------------------------------------------------
            Weapon::Lancer => KitSpec { class: Launcher, real: "RPG-7, 40 mm rocket", damage: 90, head_x10: 10, cooldown: 1.0, mag: 1, reserve: 3, reload: 4.0, auto: false, spread_hip: 0.6, spread_ads: 0.1, spread_move: 1.2, effective: 200.0, range: 200.0, far_mult: 1.0, move_mult: 0.85, zoom: 1.5, kick: 4.0, payload: Payload::Explosive, speed: 42.0, fuse: 0.0, radius: 5.5, blast: 190, lasts: 0.0, gravity: 0.0, ..b },
            Weapon::Thumper => KitSpec { class: Launcher, real: "M79 grenade launcher, 40x46 mm", damage: 70, head_x10: 10, cooldown: 0.7, mag: 1, reserve: 8, reload: 2.4, auto: false, spread_hip: 0.5, spread_ads: 0.1, spread_move: 1.0, effective: 120.0, range: 120.0, far_mult: 1.0, move_mult: 0.94, zoom: 1.25, kick: 2.2, payload: Payload::Explosive, speed: 30.0, fuse: 0.0, radius: 4.0, blast: 130, lasts: 0.0, gravity: 0.6, ..b },
            // ---- grenades ----------------------------------------------------------------------------------------------
            Weapon::Frag => grenade("M67 fragmentation grenade", Payload::Explosive, 1.9, 7.0, 105, 0.0),
            Weapon::Flash => grenade("M84 stun grenade", Payload::Flash, 1.5, 22.0, 0, 5.0),
            Weapon::Smoke => grenade("M18 smoke grenade", Payload::Smoke, 1.3, 4.6, 0, 18.0),
            Weapon::Incendiary => grenade("M14 incendiary grenade", Payload::Fire, 1.4, 3.6, 28, 7.0),
        }
    }

    /// The weapon's class.
    pub fn class(self) -> Class {
        self.kit().class
    }

    /// Whether this weapon goes in the melee slot.
    pub fn is_melee(self) -> bool {
        self.class() == Class::Melee
    }

    /// Whether this weapon is a thrown grenade.
    pub fn is_grenade(self) -> bool {
        self.class() == Class::Grenade
    }

    /// Whether this weapon occupies a firearm slot (pistols through launchers).
    pub fn is_gun(self) -> bool {
        self.class().is_gun()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_weapon_has_a_sane_row() {
        for w in Weapon::ROSTER {
            let k = w.kit();
            assert!(!k.real.is_empty(), "{w:?} names its real counterpart");
            assert!(k.cooldown_ticks() >= 3, "{w:?}");
            match k.class {
                Class::Melee => assert!(k.reach >= 1.5 && k.damage > 0, "{w:?}"),
                Class::Grenade => assert!(k.payload != Payload::None && k.fuse > 0.0 && k.radius > 0.0, "{w:?}"),
                Class::Launcher => assert!(k.payload == Payload::Explosive && k.speed > 0.0 && k.blast > 0, "{w:?}"),
                _ => {
                    assert!(k.mag > 0 && k.reserve >= k.mag / 2, "{w:?} has a magazine and ammunition to reload from");
                    assert!(k.damage > 0 && k.range >= k.effective && k.spread_ads <= k.spread_hip, "{w:?}");
                    assert!(k.reload > 0.0 && k.pellets >= 1 && k.move_mult > 0.5 && k.move_mult <= 1.05, "{w:?}");
                }
            }
        }
    }

    #[test]
    fn the_roster_is_large_and_varied() {
        assert!(Weapon::ROSTER.len() >= 25, "at least 25 distinct weapons");
        let classes: std::collections::HashSet<_> = Weapon::ROSTER.iter().map(|w| w.class().name()).collect();
        assert_eq!(classes.len(), 10, "every class is represented: {classes:?}");
        assert_eq!(Weapon::ROSTER.iter().filter(|w| w.is_melee()).count(), 3);
        assert_eq!(Weapon::ROSTER.iter().filter(|w| w.is_grenade()).count(), 4);
        let names: std::collections::HashSet<_> = Weapon::ROSTER.iter().map(|w| w.name()).collect();
        assert_eq!(names.len(), Weapon::ROSTER.len(), "no two weapons share a name");
    }

    #[test]
    fn rates_of_fire_match_the_real_weapons_within_the_tick_rate() {
        // (weapon, real rounds per minute): the simulation runs 60 ticks a second, so the nearest whole-tick cadence is what counts.
        for (w, real) in [(Weapon::Rifle, 600.0), (Weapon::Carbine, 800.0), (Weapon::Smg, 800.0), (Weapon::Lmg, 850.0), (Weapon::Ranger, 900.0)] {
            let got = w.kit().rpm();
            assert!((got - real).abs() / real < 0.2, "{w:?}: {got} rpm vs {real}");
        }
    }

    #[test]
    fn the_intended_kill_times_hold() {
        let body_shots = |w: Weapon| (100.0 / w.kit().damage as f32).ceil() as u32;
        assert_eq!(body_shots(Weapon::Rifle), 3, "an AK kills in three body shots");
        assert_eq!(body_shots(Weapon::Sentinel), 1, "a .338 kills in one");
        assert!(body_shots(Weapon::Scout) >= 2, "a scout rifle needs a second shot or a headshot");
        assert!(Weapon::Scout.kit().damage as f32 * Weapon::Scout.kit().head_x10 as f32 / 10.0 >= 100.0, "but a scout headshot kills");
    }
}
