//! Weapon timing in whole simulation ticks: the melee swing, the shot cooldown and the weapon
//! switch. Everything that decides *when a hit lands* lives here, so it cannot depend on the
//! frame rate. Call each state's `tick()` exactly once per simulation tick, *before* handling
//! that tick's input (an action started on tick T is first advanced on tick T+1).
//!
//! The renderer reads `*_secs(alpha)` to animate smoothly between ticks; those are outputs only.

use crate::sim::clock::{ticks_to_secs, TICK_DT};
use crate::weapons::{Weapon, SWING_TOTAL_TICKS, SWING_WINDUP_TICKS, SWITCH_TICKS};

/// A bat swing: windup, strike, recover. The hit is resolved once, on the tick the windup ends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeleeSwing {
    /// Ticks since the swing started, `None` when idle.
    ticks: Option<u32>,
    struck: bool,
}

impl MeleeSwing {
    /// Whether no swing is in progress.
    pub fn is_idle(&self) -> bool {
        self.ticks.is_none()
    }

    /// Starts a swing; `false` (and no change) if one is already under way.
    pub fn start(&mut self) -> bool {
        if self.ticks.is_some() {
            return false;
        }
        self.ticks = Some(0);
        self.struck = false;
        true
    }

    /// Abandons the swing (weapon switched, prop picked up).
    pub fn cancel(&mut self) {
        self.ticks = None;
    }

    /// Advances one tick. Returns `true` on the single tick where the strike lands
    /// ([`SWING_WINDUP_TICKS`] ticks after [`start`](Self::start)).
    pub fn tick(&mut self) -> bool {
        let Some(t) = self.ticks else { return false };
        let t = t + 1;
        let strike = t >= SWING_WINDUP_TICKS && !self.struck;
        self.struck |= strike;
        self.ticks = if t >= SWING_TOTAL_TICKS { None } else { Some(t) };
        strike
    }

    /// Seconds into the swing for animation, including the `alpha` fraction of the next tick;
    /// `None` when idle. Render-side only.
    pub fn elapsed_secs(&self, alpha: f32) -> Option<f32> {
        self.ticks.map(|t| ticks_to_secs(t) + alpha * TICK_DT)
    }
}

/// A countdown in ticks (shot cooldown).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cooldown(u32);

impl Cooldown {
    /// Blocks until `ticks` more ticks have passed.
    pub fn start(&mut self, ticks: u32) {
        self.0 = ticks;
    }

    /// Advances one tick.
    pub fn tick(&mut self) {
        self.0 = self.0.saturating_sub(1);
    }

    /// Whether the action is allowed again.
    pub fn ready(&self) -> bool {
        self.0 == 0
    }
}

/// Lowering one weapon and raising the next. The new weapon becomes the *active* one immediately
/// (the caller changes `weapon` when starting); `from` is what is still drawn until the halfway point.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WeaponSwitch {
    state: Option<(Weapon, u32)>,
}

impl WeaponSwitch {
    /// Whether a switch is under way (firing and switching again are blocked).
    pub fn is_active(&self) -> bool {
        self.state.is_some()
    }

    /// Starts switching away from `from`.
    pub fn start(&mut self, from: Weapon) {
        self.state = Some((from, 0));
    }

    /// Advances one tick.
    pub fn tick(&mut self) {
        if let Some((from, t)) = self.state {
            self.state = if t + 1 >= SWITCH_TICKS { None } else { Some((from, t + 1)) };
        }
    }

    /// `(weapon being lowered, seconds into the switch incl. alpha)` for animation.
    pub fn elapsed_secs(&self, alpha: f32) -> Option<(Weapon, f32)> {
        self.state.map(|(from, t)| (from, ticks_to_secs(t) + alpha * TICK_DT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swing_strikes_exactly_once_after_the_windup() {
        let mut s = MeleeSwing::default();
        assert!(s.start());
        assert!(!s.start(), "a swing in progress cannot be restarted");
        let mut strikes = Vec::new();
        for tick in 1..=SWING_TOTAL_TICKS + 2 {
            if s.tick() {
                strikes.push(tick);
            }
        }
        assert_eq!(strikes, vec![SWING_WINDUP_TICKS]);
        assert!(s.is_idle(), "the swing ends after its total duration");
        assert!(s.start(), "and can start again");
    }

    /// The whole point of the fixed tick: a click at t = 0.1 s lands its hit on the same *tick* (and so
    /// the same simulated time) whether the game renders at 30, 60, 144 or 240 fps.
    #[test]
    fn strike_tick_is_independent_of_frame_rate() {
        use crate::sim::clock::TickClock;
        let mut landed = Vec::new();
        for fps in [30.0f32, 60.0, 144.0, 240.0] {
            let (mut clock, mut swing) = (TickClock::default(), MeleeSwing::default());
            let (mut clicked, mut strike_tick) = (false, None);
            let mut real = 0.0f32;
            while strike_tick.is_none() && real < 2.0 {
                real += 1.0 / fps;
                clock.push_time(1.0 / fps);
                // A click is an input *event*: it is queued and consumed by the next tick, like `App::attack_queued`.
                let click_now = !clicked && real >= 0.1;
                clicked |= click_now;
                let mut queued = click_now;
                while let Some(tick) = clock.next_tick() {
                    if swing.tick() {
                        strike_tick = Some(tick);
                    }
                    if std::mem::take(&mut queued) {
                        swing.start();
                    }
                }
            }
            landed.push(strike_tick.expect("swing never struck"));
        }
        // Real time is only used to *place the click*, so the strike tick may differ by the tick the
        // click landed in (one tick of input quantisation) — never by the frame rate itself.
        let (lo, hi) = (*landed.iter().min().unwrap(), *landed.iter().max().unwrap());
        assert!(hi - lo <= 1, "{landed:?}");
    }

    #[test]
    fn a_cancelled_swing_never_strikes() {
        let mut s = MeleeSwing::default();
        s.start();
        s.tick();
        s.cancel();
        assert!((0..SWING_TOTAL_TICKS).all(|_| !s.tick()));
    }

    #[test]
    fn swing_animation_time_is_monotonic_across_ticks() {
        let mut s = MeleeSwing::default();
        s.start();
        let mut last = -1.0;
        for _ in 0..SWING_TOTAL_TICKS - 1 {
            for a in [0.0, 0.5, 0.99] {
                let e = s.elapsed_secs(a).unwrap();
                assert!(e > last);
                last = e;
            }
            s.tick();
        }
    }

    #[test]
    fn cooldown_counts_down_in_ticks() {
        let mut c = Cooldown::default();
        assert!(c.ready());
        c.start(3);
        for _ in 0..2 {
            c.tick();
            assert!(!c.ready());
        }
        c.tick();
        assert!(c.ready());
    }

    #[test]
    fn switch_lasts_its_tick_count() {
        let mut s = WeaponSwitch::default();
        s.start(Weapon::Bat);
        for _ in 0..SWITCH_TICKS - 1 {
            assert!(s.is_active());
            s.tick();
        }
        s.tick();
        assert!(!s.is_active());
    }
}
