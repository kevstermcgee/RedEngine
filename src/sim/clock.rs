//! The fixed simulation clock: [`TICK_RATE_HZ`] ticks per second, whatever the frame rate.
//!
//! Rendering hands [`TickClock::push_time`] the real seconds a frame took; the sim then runs
//! [`TickClock::next_tick`] until it returns `None`. The leftover fraction of a tick becomes
//! [`TickClock::alpha`], which the renderer uses to interpolate between the last two ticks.
//!
//! ```
//! use red_engine2::sim::clock::{TickClock, TICK_DT};
//! let mut clock = TickClock::default();
//! clock.push_time(0.040); // a 25 fps frame
//! let mut ran = 0;
//! while let Some(_tick) = clock.next_tick() {
//!     ran += 1; // one sim step
//! }
//! assert_eq!(ran, 2); // 40 ms = 2 ticks + a bit
//! assert!(clock.alpha() > 0.0 && clock.alpha() < 1.0);
//! ```

/// Simulation ticks per second. 60 Hz = 16.7 ms per tick: at most one tick (16.7 ms) of
/// input-quantisation delay against the 50 ms latency budget, and every weapon phase in
/// `weapons.rs` is at least 3 ticks long (test `weapons::tests::timings_survive_the_tick_rate`).
/// 30 Hz would spend 33 ms of the budget on the tick alone; 120 Hz doubles the server cost per match.
pub const TICK_RATE_HZ: u32 = 60;
/// Seconds per tick.
pub const TICK_DT: f32 = 1.0 / TICK_RATE_HZ as f32;
/// Most ticks run for one frame. A longer stall (window drag, debugger pause) resumes from where it
/// left off instead of replaying minutes of simulation; the excess real time is dropped.
pub const MAX_CATCHUP_TICKS: u32 = 8;

/// Converts seconds to the nearest whole number of ticks (never 0 for a positive duration).
pub const fn secs_to_ticks(secs: f32) -> u32 {
    let t = (secs * TICK_RATE_HZ as f32 + 0.5) as u32;
    if t == 0 && secs > 0.0 {
        1
    } else {
        t
    }
}

/// Seconds a number of ticks lasts.
pub const fn ticks_to_secs(ticks: u32) -> f32 {
    ticks as f32 * TICK_DT
}

/// Accumulates real time and hands out whole ticks (the "fix your timestep" pattern).
///
/// The accumulator is `f64` so that hours of `f32` frame times do not drift the tick count.
#[derive(Debug, Clone, Default)]
pub struct TickClock {
    accumulator: f64,
    tick: u64,
}

impl TickClock {
    /// Adds `dt` real seconds (a frame's duration). Non-finite or negative values are ignored;
    /// the backlog is capped at [`MAX_CATCHUP_TICKS`] ticks.
    pub fn push_time(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.accumulator = (self.accumulator + dt as f64).min(TICK_DT as f64 * MAX_CATCHUP_TICKS as f64);
        }
    }

    /// Consumes one tick's worth of time if there is one, returning the index of the tick to run
    /// (the first tick is 0). `None` once the backlog is below one tick.
    pub fn next_tick(&mut self) -> Option<u64> {
        if self.accumulator + 1e-9 >= TICK_DT as f64 {
            self.accumulator = (self.accumulator - TICK_DT as f64).max(0.0);
            let t = self.tick;
            self.tick += 1;
            Some(t)
        } else {
            None
        }
    }

    /// How far into the next tick we are, `0.0..1.0` — the interpolation factor between the
    /// previous and the latest completed tick.
    pub fn alpha(&self) -> f32 {
        (self.accumulator / TICK_DT as f64).clamp(0.0, 1.0) as f32
    }

    /// Number of ticks run so far (the index the next [`next_tick`](Self::next_tick) will return).
    pub fn ticks_run(&self) -> u64 {
        self.tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(clock: &mut TickClock, frames: &[f32]) -> u64 {
        for &dt in frames {
            clock.push_time(dt);
            while clock.next_tick().is_some() {}
        }
        clock.ticks_run()
    }

    #[test]
    fn tick_count_does_not_depend_on_frame_rate() {
        // Ten simulated seconds at 30, 60, 144 and a wobbly rate all yield the same tick count.
        let seconds = 10.0;
        let mut counts = Vec::new();
        for fps in [30.0f32, 60.0, 144.0, 240.0, 57.3] {
            let n = (seconds * fps) as usize;
            let mut c = TickClock::default();
            counts.push(run(&mut c, &vec![1.0 / fps; n]));
        }
        for c in &counts {
            assert!((*c as i64 - 600).abs() <= 1, "{counts:?}");
        }
    }

    #[test]
    fn a_long_stall_is_capped() {
        let mut c = TickClock::default();
        c.push_time(30.0);
        let mut n = 0;
        while c.next_tick().is_some() {
            n += 1;
        }
        assert_eq!(n, MAX_CATCHUP_TICKS);
        assert!(c.alpha() < 1e-3);
    }

    #[test]
    fn junk_frame_times_are_ignored() {
        let mut c = TickClock::default();
        for dt in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            c.push_time(dt);
        }
        assert_eq!(c.next_tick(), None);
    }

    #[test]
    fn ticks_are_numbered_from_zero_and_alpha_is_the_remainder() {
        let mut c = TickClock::default();
        c.push_time(TICK_DT * 2.5);
        assert_eq!(c.next_tick(), Some(0));
        assert_eq!(c.next_tick(), Some(1));
        assert_eq!(c.next_tick(), None);
        assert!((c.alpha() - 0.5).abs() < 1e-3);
    }

    #[test]
    fn conversions_round_to_nearest_and_never_hit_zero() {
        assert_eq!(secs_to_ticks(0.09), 5);
        assert_eq!(secs_to_ticks(0.42), 25);
        assert_eq!(secs_to_ticks(0.001), 1);
        assert_eq!(secs_to_ticks(0.0), 0);
        assert!((ticks_to_secs(60) - 1.0).abs() < 1e-6);
    }
}
