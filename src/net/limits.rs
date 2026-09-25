//! Abuse limits for a public UDP port: a token bucket, and the numbers the server applies with it.
//!
//! A real client sends one input packet per 60 Hz tick and two Hellos (challenge, then proof) per 250 ms while joining, so the
//! limits sit far above honest traffic and only bite on a flood. Packets over a limit are dropped
//! before they reach the sim (no queued input, no reply, no session) and counted in `ServerStats`.

use std::time::Instant;

/// Sustained input packets per second one session may send (an honest client sends 60).
pub const INPUT_PACKETS_PER_SEC: f32 = 180.0;
/// Burst allowance for a session's input packets (catching up after a stall).
pub const INPUT_BURST: f32 = 90.0;
/// Lobby packets per second one session may send (an honest client sends about 5, plus one per button press).
pub const LOBBY_PACKETS_PER_SEC: f32 = 30.0;
/// Burst allowance for a session's lobby packets.
pub const LOBBY_BURST: f32 = 30.0;
/// Hellos per second the server answers in total, across every source address (an honest joiner sends 4).
pub const HELLOS_PER_SEC: f32 = 60.0;
/// Burst allowance for Hellos.
pub const HELLO_BURST: f32 = 120.0;
/// Most dropped players remembered for resume; the oldest is forgotten first.
pub const MAX_PARKED: usize = 32;

/// A classic token bucket: `rate` tokens per second up to `burst`; each allowed event costs one.
#[derive(Debug, Clone, Copy)]
pub struct TokenBucket {
    tokens: f32,
    rate: f32,
    burst: f32,
    last: Instant,
}

impl TokenBucket {
    /// A full bucket.
    pub fn new(rate: f32, burst: f32, now: Instant) -> Self {
        TokenBucket { tokens: burst, rate, burst, last: now }
    }

    /// Takes one token if there is one; `false` means "over the limit, drop it".
    pub fn allow(&mut self, now: Instant) -> bool {
        let dt = now.saturating_duration_since(self.last).as_secs_f32();
        self.last = now;
        self.tokens = (self.tokens + dt * self.rate).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_burst_passes_then_the_flood_is_refused_and_the_bucket_refills() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(10.0, 5.0, t0);
        assert_eq!((0..20).filter(|_| b.allow(t0)).count(), 5, "only the burst gets through at one instant");
        assert!(!b.allow(t0 + Duration::from_millis(50)), "half a token is not enough");
        assert!(b.allow(t0 + Duration::from_millis(200)), "two tokens came back after 200 ms at 10/s");
    }

    #[test]
    fn honest_traffic_never_hits_the_limits() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(INPUT_PACKETS_PER_SEC, INPUT_BURST, t0);
        for k in 0..600 {
            assert!(b.allow(t0 + Duration::from_micros(k * 16_667)), "a 60 Hz client is never limited (packet {k})");
        }
    }
}
