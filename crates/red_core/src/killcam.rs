//! The killcam: after you die, the last seconds of the fight from your killer's eyes (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! Nothing extra travels on the network. Every snapshot already lists every player's position, look direction, weapon and stance, so the
//! client keeps the last [`KEEP_SECS`] of them (`NetClient::keep_history`); when the server says who killed us, [`playback`] cuts out the
//! window from a few seconds before the death and [`Playback::at`] returns the interpolated poses of the killer (whose eyes the camera
//! takes) and of everyone else at that moment, which the client draws exactly like live players. Pure data: no window, no GPU.
//!
//! Used by Killchain alone and offered by nothing: it moves to `games/killchain` (ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or,
//! system audit, docs/analysis/2026-10-14-system-audit.md).

use crate::net::game_view::History;
use crate::net::interp::{Blend, PlayerPose};

/// Seconds of the past the client keeps for a replay (`NetClient::keep_history`).
pub const KEEP_SECS: f64 = 10.0;
/// Seconds before the death that a replay begins.
pub const LEAD_SECS: f64 = 5.0;
/// Seconds a killcam lasts in all.
pub const LENGTH_SECS: f64 = 8.0;

/// Everyone as one snapshot showed them.
#[derive(Debug, Clone)]
struct Frame {
    /// Server time, seconds.
    t: f64,
    /// Every player in the snapshot.
    players: Vec<(u8, PlayerPose)>,
}

/// The replay of a kill: from [`LEAD_SECS`] before `death_t` (or the oldest snapshot held) for [`LENGTH_SECS`], following `killer`. `None` when
/// the killer is not in the history (they had left, or the history is empty).
pub fn playback(history: &History, killer: u8, death_t: f64) -> Option<Playback> {
    let oldest = history.oldest()?;
    let start = (death_t - LEAD_SECS).max(oldest);
    let frames: Vec<Frame> = history
        .iter()
        .filter(|(t, _)| *t >= start - 0.2)
        .map(|(t, snap)| Frame { t, players: snap.players.iter().map(|p| (p.id, PlayerPose::from(p))).collect() })
        .collect();
    if !frames.iter().any(|f| f.players.iter().any(|(id, _)| *id == killer)) {
        return None;
    }
    Some(Playback { frames, start, killer, death_t })
}

/// A cut-out replay.
#[derive(Debug, Clone)]
pub struct Playback {
    frames: Vec<Frame>,
    start: f64,
    killer: u8,
    death_t: f64,
}

/// One instant of a replay.
#[derive(Debug, Clone)]
pub struct Shot {
    /// The killer, whose eyes the camera takes.
    pub killer: PlayerPose,
    /// Every player (the killer and the victim included) as they were.
    pub players: Vec<(u8, PlayerPose)>,
    /// The server time shown.
    pub t: f64,
    /// Whether the shot is before the moment of death (the kill itself is at `false`/`true` boundary).
    pub before_death: bool,
}

impl Playback {
    /// The killer's id.
    pub fn killer(&self) -> u8 {
        self.killer
    }

    /// The server time the replay begins at.
    pub fn start(&self) -> f64 {
        self.start
    }

    /// Seconds from the start of the replay to the moment of death.
    pub fn death_offset(&self) -> f64 {
        self.death_t - self.start
    }

    /// The instant `elapsed` seconds into the replay. Past the newest snapshot it holds the last one.
    pub fn at(&self, elapsed: f32) -> Option<Shot> {
        let t = self.start + elapsed.max(0.0) as f64;
        let first = self.frames.first()?;
        let last = self.frames.last()?;
        let t = t.clamp(first.t, last.t);
        let i = self.frames.partition_point(|f| f.t <= t).saturating_sub(1).min(self.frames.len() - 1);
        let (a, b) = (&self.frames[i], self.frames.get(i + 1).unwrap_or(&self.frames[i]));
        let k = if b.t > a.t { ((t - a.t) / (b.t - a.t)).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let mut players = Vec::with_capacity(a.players.len());
        for (id, pa) in &a.players {
            let pose = match b.players.iter().find(|(bid, _)| bid == id) {
                Some((_, pb)) if (pb.pos - pa.pos).length() < 6.0 => pa.blend(*pb, k),
                Some((_, pb)) if k >= 0.5 => *pb,
                _ => *pa,
            };
            players.push((*id, pose));
        }
        let killer = players.iter().find(|(id, _)| *id == self.killer).map(|(_, p)| *p)?;
        Some(Shot { killer, players, t, before_death: t < self.death_t })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocol::{PlayerSnap, Snapshot};

    fn snap(tick: u32, killer_x: f32, victim_x: f32) -> Snapshot {
        let p = |id: u8, x: f32| PlayerSnap { id, pos: [x, 0.0, 0.0], yaw: 1.0, ..Default::default() };
        Snapshot { server_tick: tick, players: vec![p(1, killer_x), p(2, victim_x)], ..Default::default() }
    }

    fn recorded(seconds: u32) -> History {
        let mut h = History::new(KEEP_SECS);
        for tick in (0..seconds * 60).step_by(2) {
            h.record(&snap(tick, tick as f32 * 0.05, 100.0));
        }
        h
    }

    #[test]
    fn the_replay_starts_before_the_death_and_follows_the_killer_smoothly() {
        let h = recorded(20);
        let death_t = 15.0;
        let pb = playback(&h, 1, death_t).expect("the killer is in the history");
        assert!((pb.start() - (death_t - LEAD_SECS)).abs() < 0.05);
        assert!((pb.death_offset() - LEAD_SECS).abs() < 0.05);
        let s0 = pb.at(0.0).unwrap();
        let s1 = pb.at(0.0167).unwrap();
        let s2 = pb.at(0.05).unwrap();
        // The killer moves 3 m/s (0.05 m per tick): positions interpolate between snapshots.
        assert!(s1.killer.pos.x > s0.killer.pos.x && s2.killer.pos.x > s1.killer.pos.x);
        assert!((s2.killer.pos.x - s0.killer.pos.x - 0.15).abs() < 0.03, "{} -> {}", s0.killer.pos.x, s2.killer.pos.x);
        assert!(s0.before_death && !pb.at(5.5).unwrap().before_death);
        assert_eq!(s0.players.len(), 2, "the victim is in the picture too");
        // Past the end it holds the newest snapshot instead of failing.
        assert!(pb.at(60.0).is_some());
    }

    #[test]
    fn a_killer_who_is_not_in_the_history_gives_no_replay() {
        let h = recorded(5);
        assert!(playback(&h, 9, 4.0).is_none());
        assert!(playback(&History::new(KEEP_SECS), 1, 1.0).is_none());
    }

    #[test]
    fn a_short_history_starts_where_it_starts() {
        let h = recorded(3);
        let pb = playback(&h, 1, 2.9).unwrap();
        assert!(pb.start() < 0.1, "only three seconds exist: the replay begins at the start of them ({})", pb.start());
    }
}
