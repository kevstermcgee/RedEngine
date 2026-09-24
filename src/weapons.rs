//! The human's weapons: the baseball bat (primary) and the silver revolver (mouse scroll switches).
//!
//! Pure data and rules, no window/GPU types (ADR 0010): which weapons exist, the revolver's numbers
//! (fire rate, range, knock-back) and its [`Ammo`] — **infinite for now**; the limited variant is
//! already here so the day the revolver gets a cap it is a one-line change (`REVOLVER_AMMO`).
//! The models live in `viewer::build_held_parts` (bat) and `revolver::build_revolver_parts`; the
//! synthesized sounds in `audio`; the wiring in `bin/re2.rs`.

/// A weapon the human can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weapon {
    /// The wooden bat: melee, the primary weapon and the one the human starts with.
    Bat,
    /// The silver revolver: hitscan, one shot per click.
    Revolver,
}

impl Weapon {
    /// Every weapon, in scroll order.
    pub const ALL: [Weapon; 2] = [Weapon::Bat, Weapon::Revolver];

    /// The next weapon in scroll order (wraps around); `-1` goes the other way.
    pub fn cycle(self, direction: i32) -> Weapon {
        let i = Weapon::ALL.iter().position(|w| *w == self).unwrap_or(0) as i32;
        Weapon::ALL[(i + direction.signum()).rem_euclid(Weapon::ALL.len() as i32) as usize]
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Weapon::Bat => "baseball bat",
            Weapon::Revolver => "silver revolver",
        }
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

/// What the revolver carries. `Infinite` never runs out; `Limited` counts rounds in the cylinder and
/// a reserve to reload from (not used yet — flip [`REVOLVER_AMMO`] to switch it on).
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

/// The revolver's ammunition when a game starts. Infinite for now; later, e.g.
/// `Ammo::Limited { loaded: 6, capacity: REVOLVER_CYLINDER, reserve: 24 }`.
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
        assert_eq!(Weapon::Revolver.cycle(1), Weapon::Bat);
        assert_eq!(Weapon::Bat.cycle(-1), Weapon::Revolver);
        assert_eq!(Weapon::Revolver.cycle(-3), Weapon::Bat, "only the direction matters");
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
