//! A game's own part of each snapshot as a client reads it, and the recent past as snapshots showed it: the client side of the server's
//! `sim::game::MatchGame` (ADR 2026-10-14-a-game-plugs-into-the-server-s-match-simulation-the). The core protocol carries a game's part as an opaque
//! [`GameMsg`]; the game gives the client a [`GameDecoder`] (`NetClient::set_game_decoder`) and reads its [`GameView`] back
//! (`NetClient::game_view`). A game that replays the past (Killchain's killcam) asks the client to keep a [`History`] (`NetClient::keep_history`).
//! No socket: the client lives behind the `quic` feature, these do not.

use crate::net::protocol::{DecodeError, GameMsg, Snapshot};
use crate::sim::clock::TICK_DT;
use std::any::Any;
use std::collections::VecDeque;

/// A game's reading of its own part of a snapshot ([`GameMsg`]): the client side of the server's `sim::game::MatchGame` (ADR
/// 2026-10-14-a-game-plugs-into-the-server-s-match-simulation-the). The game reads it back with `NetClient::game_view`.
pub trait GameView: Any + Send {
    /// How much slower the local player moves (`1.0` = not at all; `None` = the game does not slow anyone). The server applies the game's
    /// `MatchGame::move_mult`, so the prediction must agree.
    fn move_mult(&self) -> Option<f32> {
        None
    }
}

/// Reads a snapshot's game part: `Ok(None)` when it is not this game's kind; an error makes the snapshot a bad packet.
pub type GameDecoder = fn(&GameMsg) -> Result<Option<Box<dyn GameView>>, DecodeError>;

/// The last seconds of a match as snapshots showed them, oldest first (`NetClient::keep_history`). A snapshot older than the newest one held
/// is ignored (a late packet); anything more than the kept seconds older than the newest is dropped.
#[derive(Debug, Default)]
pub struct History {
    snapshots: VecDeque<(f64, Snapshot)>,
    /// Seconds of server time kept (`0` = none).
    keep: f64,
}

impl History {
    /// An empty history that keeps `secs` seconds.
    pub fn new(secs: f64) -> History {
        History { snapshots: VecDeque::new(), keep: secs }
    }

    /// Adds a snapshot (nothing when this history keeps no time).
    pub fn record(&mut self, snap: &Snapshot) {
        if self.keep <= 0.0 {
            return;
        }
        let t = snap.server_tick as f64 * TICK_DT as f64;
        if self.snapshots.back().is_some_and(|(newest, _)| t <= *newest) {
            return;
        }
        self.snapshots.push_back((t, snap.clone()));
        while self.snapshots.front().is_some_and(|(oldest, _)| t - oldest > self.keep) {
            self.snapshots.pop_front();
        }
    }

    /// Keeps `secs` seconds of server time from the next snapshot on (`0` = none).
    pub fn set_keep(&mut self, secs: f64) {
        self.keep = secs;
    }

    /// Forgets everything (a new round).
    pub fn clear(&mut self) {
        self.snapshots.clear();
    }

    /// Each snapshot held with its server time in seconds, oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (f64, &Snapshot)> + '_ {
        self.snapshots.iter().map(|(t, s)| (*t, s))
    }

    /// The newest server time held.
    pub fn newest(&self) -> Option<f64> {
        self.snapshots.back().map(|(t, _)| *t)
    }

    /// The oldest server time held.
    pub fn oldest(&self) -> Option<f64> {
        self.snapshots.front().map(|(t, _)| *t)
    }

    /// How many snapshots are held.
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Whether nothing is held.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    fn snap(tick: u32) -> Snapshot {
        Snapshot { server_tick: tick, ..Default::default() }
    }

    #[test]
    fn only_the_kept_seconds_are_held_and_late_snapshots_are_ignored() {
        let mut h = History::new(10.0);
        for tick in (0..30 * 60).step_by(2) {
            h.record(&snap(tick));
        }
        assert!(h.newest().unwrap() > 29.0);
        assert!(h.oldest().unwrap() >= h.newest().unwrap() - 10.0 - 0.05, "{:?}", h.oldest());
        let len = h.len();
        h.record(&snap(10)); // a late packet
        assert_eq!(h.len(), len);
        h.clear();
        assert!(h.is_empty());
    }

    #[test]
    fn a_history_that_keeps_nothing_holds_nothing() {
        let mut h = History::default();
        h.record(&snap(5));
        assert!(h.is_empty() && h.newest().is_none());
    }
}
