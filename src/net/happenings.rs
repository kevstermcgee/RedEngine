//! What a snapshot says *happened*: the shots other players fired, and what the receiving client did and suffered.
//!
//! The server sends counters that only grow (`PlayerSnap::shots`, the per-client [`Feedback`]). A client that remembers the last value it saw
//! can turn each new snapshot into events without any of them being lost when a datagram is: a dropped snapshot just means the next one
//! reports two shots instead of one. [`Watcher`] does that, purely (no sockets, no clock), so it is tested directly.
//!
//! The first snapshot after joining, resuming or a new round only sets the baseline: nothing is reported for what happened before we were
//! looking (a player joining an old match must not hear every shot of the match so far).

use super::protocol::{Feedback, Snapshot, MAX_PLAYERS_PER_SNAPSHOT};
use glam::Vec3;

/// Most shots one snapshot may report for one player: a burst larger than this is a stale counter, not a firefight.
const MAX_SHOTS_PER_SNAPSHOT: u8 = 8;

/// Someone else fired.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotHeard {
    /// Who fired (a player id).
    pub id: u8,
    /// The weapon in their hands (`weapons::Weapon::wire`).
    pub weapon: u8,
    /// How many shots since the last snapshot.
    pub shots: u8,
    /// Where they stood (x, foot_y, z).
    pub pos: Vec3,
    /// Where they were looking.
    pub yaw: f32,
    /// Their look pitch.
    pub pitch: f32,
}

/// Everything one snapshot adds to what the client already knew.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Happenings {
    /// Shots fired by other players.
    pub shots: Vec<ShotHeard>,
    /// Attacks of ours that damaged someone.
    pub hits: u8,
    /// Times we were damaged.
    pub hurt: u8,
    /// Kills we scored.
    pub kills: u8,
    /// When `hurt > 0`: the world yaw (radians) toward whoever damaged us last.
    pub bearing: Option<f32>,
}

impl Happenings {
    /// Whether nothing happened.
    pub fn is_empty(&self) -> bool {
        self.shots.is_empty() && self.hits == 0 && self.hurt == 0 && self.kills == 0
    }
}

/// Remembers the counters of the previous snapshot.
#[derive(Debug, Clone, Default)]
pub struct Watcher {
    feedback: Option<Feedback>,
    shots: [Option<u8>; MAX_PLAYERS_PER_SNAPSHOT],
}

impl Watcher {
    /// Forgets everything (joined, resumed, a new round): the next snapshot only sets the baseline.
    pub fn reset(&mut self) {
        *self = Watcher::default();
    }

    /// Compares `snap` with the previous one. `me` is our own player id (our own shots are not "heard": we play those when we pull the trigger).
    pub fn observe(&mut self, snap: &Snapshot, me: Option<u8>) -> Happenings {
        let mut out = Happenings::default();
        if let Some(prev) = self.feedback.replace(snap.fx) {
            out.hits = snap.fx.hits.wrapping_sub(prev.hits);
            out.hurt = snap.fx.hurt.wrapping_sub(prev.hurt);
            out.kills = snap.fx.kills.wrapping_sub(prev.kills);
            if out.hurt > 0 {
                out.bearing = Some(snap.fx.bearing_rad());
            }
        }
        let mut seen = [false; MAX_PLAYERS_PER_SNAPSHOT];
        for p in &snap.players {
            let id = p.id as usize;
            if id >= MAX_PLAYERS_PER_SNAPSHOT {
                continue;
            }
            seen[id] = true;
            let before = self.shots[id].replace(p.shots);
            if Some(p.id) == me {
                continue;
            }
            if let Some(before) = before {
                let n = p.shots.wrapping_sub(before);
                if n > 0 && n <= MAX_SHOTS_PER_SNAPSHOT {
                    out.shots.push(ShotHeard { id: p.id, weapon: p.weapon, shots: n, pos: Vec3::from(p.pos), yaw: p.yaw, pitch: p.pitch });
                }
            }
        }
        // Someone not listed (left, or out of interest range): their next appearance is a new baseline, not a burst of shots.
        for (id, seen) in seen.iter().enumerate() {
            if !seen {
                self.shots[id] = None;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::protocol::{PlayerSnap, NO_PROP};

    fn snap(fx: Feedback, players: &[(u8, u8)]) -> Snapshot {
        Snapshot {
            seq: 1,
            server_tick: 1,
            ack_input_seq: 0,
            echo_time_ms: 0,
            echo_hold_ms: 0,
            fx,
            players: players
                .iter()
                .map(|&(id, shots)| PlayerSnap {
                    id,
                    character: 0,
                    flags: 0,
                    pos: [id as f32, 0.0, 0.0],
                    yaw: 0.5,
                    pitch: -0.1,
                    speed: 0.0,
                    vy: 0.0,
                    velocity: [0.0; 2],
                    weapon: 4,
                    held: NO_PROP,
                    hp: 100,
                    shots,
                    kart: None,
                })
                .collect(),
            props: vec![],
            race: None,
        }
    }

    #[test]
    fn the_first_snapshot_only_sets_the_baseline() {
        let mut w = Watcher::default();
        let fx = Feedback { hits: 40, hurt: 9, kills: 3, bearing: 0, respawn: 0 };
        assert!(w.observe(&snap(fx, &[(1, 200), (2, 17)]), Some(1)).is_empty(), "nothing is reported for the history before we looked");
    }

    #[test]
    fn counters_that_grow_become_events_and_survive_lost_snapshots() {
        let mut w = Watcher::default();
        w.observe(&snap(Feedback::default(), &[(0, 0), (2, 0)]), Some(0));
        let h = w.observe(&snap(Feedback { hits: 1, hurt: 2, kills: 1, bearing: 64, respawn: 0 }, &[(0, 3), (2, 1)]), Some(0));
        assert_eq!((h.hits, h.hurt, h.kills), (1, 2, 1));
        assert!((h.bearing.unwrap() - std::f32::consts::FRAC_PI_2).abs() < 1e-5, "64/256 of a turn is 90 degrees: {:?}", h.bearing);
        assert_eq!(h.shots.len(), 1, "our own player's shots are not 'heard'");
        assert_eq!((h.shots[0].id, h.shots[0].shots, h.shots[0].weapon), (2, 1, 4));
        // Two snapshots lost: the counter jumped by three, and all three are reported at once.
        let h = w.observe(&snap(Feedback { hits: 1, hurt: 2, kills: 1, bearing: 64, respawn: 0 }, &[(0, 3), (2, 4)]), Some(0));
        assert_eq!(h.shots.iter().map(|s| s.shots).sum::<u8>(), 3);
        assert_eq!((h.hits, h.hurt, h.kills, h.bearing), (0, 0, 0, None), "counters that did not change report nothing");
    }

    #[test]
    fn counters_wrap_at_256_without_a_phantom_burst() {
        let mut w = Watcher::default();
        w.observe(&snap(Feedback { hits: 255, ..Default::default() }, &[(2, 254)]), Some(0));
        let h = w.observe(&snap(Feedback { hits: 1, ..Default::default() }, &[(2, 1)]), Some(0));
        assert_eq!(h.hits, 2, "255 -> 1 is two hits");
        assert_eq!(h.shots[0].shots, 3, "254 -> 1 is three shots");
    }

    #[test]
    fn a_player_who_reappears_is_a_new_baseline_and_a_stale_counter_is_not_a_firefight() {
        let mut w = Watcher::default();
        w.observe(&snap(Feedback::default(), &[(2, 10)]), Some(0));
        assert!(w.observe(&snap(Feedback::default(), &[]), Some(0)).is_empty(), "left view");
        assert!(w.observe(&snap(Feedback::default(), &[(2, 90)]), Some(0)).shots.is_empty(), "back in view with a very different counter: baseline only");
        assert!(w.observe(&snap(Feedback::default(), &[(2, 200)]), Some(0)).shots.is_empty(), "a jump of 110 is not 110 shots");
        w.reset();
        assert!(w.observe(&snap(Feedback { kills: 9, ..Default::default() }, &[(2, 1)]), Some(0)).is_empty(), "a new round starts from a fresh baseline");
    }
}
