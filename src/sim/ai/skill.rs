//! How good a bot is and how it likes to fight: difficulty as numbers a game designer can read, personality styles, and what each
//! weapon wants (how close, how far, whether it needs a fresh trigger pull).
//!
//! A [`Skill`] is derived from one `level` in `0.0..=1.0` (`rookie` ... `nightmare`), so a scene can say `"skill": 0.6` or `"skill": "hard"` and a
//! roster can spread its bots across a range. Everything is in seconds, degrees and metres; the brain converts to whole ticks itself.

use crate::weapons::Weapon;

/// Named difficulty presets and the level each stands for.
pub const PRESETS: &[(&str, f32)] = &[("rookie", 0.12), ("easy", 0.30), ("normal", 0.50), ("hard", 0.72), ("nightmare", 0.95)];

/// The level for a preset name or a number written as text (`"hard"`, `"0.6"`).
pub fn level_from_name(name: &str) -> Option<f32> {
    let n = name.trim().to_ascii_lowercase();
    PRESETS.iter().find(|(p, _)| *p == n).map(|(_, l)| *l).or_else(|| n.parse::<f32>().ok().filter(|l| (0.0..=1.0).contains(l)))
}

/// Personality: how a bot picks its fights. It scales distance and jumping, never the aim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Fights at the weapon's natural range.
    Balanced,
    /// Closes in, strafes fast, wants the enemy inside its comfortable range.
    Rusher,
    /// Holds back, crouches to shoot, prefers long lines.
    Sniper,
    /// Jumps constantly and sprints through fights: hard to hit, and it knows it.
    Acrobat,
}

impl Style {
    /// Parses a scene's style word.
    pub fn parse(s: &str) -> Option<Style> {
        match s.trim().to_ascii_lowercase().as_str() {
            "balanced" => Some(Style::Balanced),
            "rusher" => Some(Style::Rusher),
            "sniper" => Some(Style::Sniper),
            "acrobat" => Some(Style::Acrobat),
            _ => None,
        }
    }

    /// Multiplier on the distance a bot likes to keep from its target.
    pub fn range_scale(self) -> f32 {
        match self {
            Style::Balanced => 1.0,
            Style::Rusher => 0.6,
            Style::Sniper => 1.5,
            Style::Acrobat => 0.9,
        }
    }

    /// Multiplier on how often it jumps.
    pub fn jumpiness(self) -> f32 {
        match self {
            Style::Acrobat => 4.0,
            Style::Rusher => 1.3,
            Style::Sniper => 0.2,
            Style::Balanced => 1.0,
        }
    }

    /// 0..1: how willing it is to walk into the open to close a gap.
    pub fn aggression(self) -> f32 {
        match self {
            Style::Rusher => 1.0,
            Style::Acrobat => 0.75,
            Style::Balanced => 0.55,
            Style::Sniper => 0.2,
        }
    }
}

/// The numbers that make one bot easier or harder than another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Skill {
    /// Seconds between first seeing an enemy and the first shot.
    pub reaction_secs: f32,
    /// Fastest the crosshair can swing, degrees per second.
    pub aim_turn_dps: f32,
    /// How eagerly the crosshair closes the remaining angle each tick (0..1): low is sluggish, high is snappy.
    pub aim_gain: f32,
    /// Aim wobble on first contact: the angular radius the shots scatter over, degrees.
    pub aim_error_deg: f32,
    /// Fraction of that wobble left once it has settled (0..1).
    pub aim_floor: f32,
    /// Seconds of continuous sight for the wobble to settle.
    pub aim_settle_secs: f32,
    /// Shortest and longest time between changes of strafe direction, seconds.
    pub strafe_secs: (f32, f32),
    /// Chance per second of a dodging jump while fighting.
    pub jump_per_sec: f32,
    /// Seconds an unseen enemy is remembered.
    pub memory_secs: f32,
    /// Enemies closer than this are noticed without seeing them (footsteps), metres.
    pub hearing_m: f32,
    /// Seconds to rest between bursts of an automatic weapon.
    pub burst_rest_secs: f32,
}

impl Skill {
    /// The skill for a `level` (clamped to 0..=1).
    pub fn from_level(level: f32) -> Skill {
        let l = level.clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * l;
        Skill {
            reaction_secs: lerp(0.70, 0.16),
            aim_turn_dps: lerp(110.0, 520.0),
            aim_gain: lerp(0.10, 0.40),
            aim_error_deg: lerp(7.0, 1.3),
            aim_floor: lerp(0.65, 0.25),
            aim_settle_secs: lerp(1.7, 0.7),
            strafe_secs: (lerp(0.9, 0.35), lerp(1.9, 0.9)),
            jump_per_sec: lerp(0.04, 0.45),
            memory_secs: lerp(2.0, 6.0),
            hearing_m: lerp(4.0, 11.0),
            burst_rest_secs: lerp(0.55, 0.10),
        }
    }
}

/// What a weapon wants from the person holding it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponProfile {
    /// Distance it likes to fight from, metres.
    pub ideal: f32,
    /// Farthest it will open fire from, metres.
    pub reach: f32,
    /// Needs a new trigger pull per shot.
    pub semi_auto: bool,
    /// Shots per burst before a rest, `(fewest, most)`; ignored for semi-autos.
    pub burst: (u32, u32),
    /// Scale on aim wobble: scoped and precision weapons are steadier.
    pub steadiness: f32,
    /// Melee: walk into the target instead of standing off.
    pub melee: bool,
}

impl WeaponProfile {
    /// The profile of `weapon`, tuned against the numbers in [`Weapon::firearm`] (damage, cadence, range, spread).
    pub fn of(weapon: Weapon) -> WeaponProfile {
        let p = |ideal, reach, semi_auto, burst: (u32, u32), steadiness| WeaponProfile { ideal, reach, semi_auto, burst, steadiness, melee: false };
        match weapon {
            Weapon::Bat => WeaponProfile { ideal: 1.4, reach: 2.0, semi_auto: true, burst: (1, 1), steadiness: 1.0, melee: true },
            Weapon::Pistol => p(13.0, 40.0, true, (1, 1), 0.9),
            Weapon::Revolver => p(14.0, 45.0, true, (1, 1), 0.85),
            Weapon::MachinePistol => p(8.0, 24.0, false, (4, 9), 1.15),
            Weapon::Smg => p(8.5, 28.0, false, (5, 12), 1.1),
            Weapon::Carbine => p(15.0, 55.0, false, (3, 6), 0.9),
            Weapon::Rifle => p(17.0, 70.0, false, (3, 6), 0.8),
            Weapon::Bullpup => p(16.0, 65.0, false, (3, 6), 0.85),
            Weapon::Marksman => p(26.0, 90.0, true, (1, 1), 0.55),
            Weapon::Shotgun => p(5.5, 14.0, true, (1, 1), 1.0),
            Weapon::Lmg => p(15.0, 55.0, false, (8, 18), 1.0),
            Weapon::Scout => p(34.0, 110.0, true, (1, 1), 0.45),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harder_levels_react_faster_aim_truer_and_turn_quicker() {
        let (easy, hard) = (Skill::from_level(0.0), Skill::from_level(1.0));
        assert!(hard.reaction_secs < easy.reaction_secs && hard.aim_error_deg < easy.aim_error_deg && hard.aim_turn_dps > easy.aim_turn_dps);
        assert!(hard.aim_floor < easy.aim_floor && hard.burst_rest_secs < easy.burst_rest_secs);
        assert_eq!(Skill::from_level(7.0), Skill::from_level(1.0), "levels are clamped");
        for level in [0.0, 0.3, 0.5, 0.72, 1.0] {
            let s = Skill::from_level(level);
            assert!(s.reaction_secs >= 0.1 && s.aim_error_deg > 0.5 && s.strafe_secs.0 < s.strafe_secs.1, "{level}: {s:?}");
        }
    }

    #[test]
    fn presets_and_numbers_both_name_a_level() {
        assert_eq!(level_from_name("Hard"), Some(0.72));
        assert_eq!(level_from_name("0.4"), Some(0.4));
        assert_eq!(level_from_name("2"), None);
        assert_eq!(level_from_name("godlike"), None);
        assert!(PRESETS.windows(2).all(|w| w[0].1 < w[1].1), "presets are ordered easiest to hardest");
    }

    #[test]
    fn every_weapon_has_a_sensible_profile() {
        for w in Weapon::ALL {
            let p = WeaponProfile::of(w);
            assert!(p.ideal > 0.0 && p.reach >= p.ideal && p.steadiness > 0.0 && p.burst.0 >= 1 && p.burst.0 <= p.burst.1, "{w:?}: {p:?}");
            if let Some(spec) = w.firearm() {
                assert!(p.reach <= spec.range, "{w:?}: a bot never fires beyond the weapon's own range");
            }
        }
        assert!(WeaponProfile::of(Weapon::Bat).melee && !WeaponProfile::of(Weapon::Scout).melee);
        assert!(WeaponProfile::of(Weapon::Scout).ideal > WeaponProfile::of(Weapon::Shotgun).ideal * 3.0);
    }

    #[test]
    fn styles_parse_and_shape_the_fight() {
        assert_eq!(Style::parse(" Rusher "), Some(Style::Rusher));
        assert_eq!(Style::parse("camper"), None);
        assert!(Style::Rusher.range_scale() < Style::Sniper.range_scale());
        assert!(Style::Acrobat.jumpiness() > Style::Sniper.jumpiness());
    }
}
