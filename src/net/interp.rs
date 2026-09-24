//! Rendering other players and props smoothly from 30 Hz snapshots.
//!
//! Every snapshot is stamped with a server time. The client keeps a short history of each remote
//! thing and draws it *slightly in the past* ([`INTERP_DELAY`]), at a time that always lies between
//! two received samples, so motion is a smooth interpolation rather than a jump to the newest packet.
//! The mapping between the client's clock and the server's ([`ServerClock`]) is estimated from
//! packet arrivals, favouring the least-delayed packets so jitter does not make things stutter.
//! Pure functions of `(samples, times)`: no sockets, no `Instant`, fully unit-tested.

use crate::net::protocol::{PlayerSnap, PropSnap, Snapshot, MAX_PLAYERS_PER_SNAPSHOT};
use crate::sim::clock::TICK_DT;
use glam::{Quat, Vec3};
use std::collections::VecDeque;

/// How far behind the newest snapshot other players and props are drawn, seconds. Three snapshot
/// intervals at 30 Hz: two packets can be lost or late in a row and motion still interpolates.
pub const INTERP_DELAY: f64 = 0.100;
/// Samples kept per remote thing.
const HISTORY: usize = 24;

/// Anything that can be blended between two samples.
pub trait Blend: Copy {
    /// `t = 0` gives `self`, `t = 1` gives `other`.
    fn blend(self, other: Self, t: f32) -> Self;
}

/// A remote player's drawn state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPose {
    /// World position (x, foot_y, z).
    pub pos: Vec3,
    /// Look yaw, radians.
    pub yaw: f32,
    /// Look pitch, radians.
    pub pitch: f32,
    /// Horizontal speed, m/s.
    pub speed: f32,
    /// `0` human, `1` rat.
    pub character: u8,
    /// Crouching.
    pub crouching: bool,
}

/// Shortest-arc blend of two angles (radians).
pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let mut d = (b - a) % std::f32::consts::TAU;
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    } else if d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    a + d * t
}

impl Blend for PlayerPose {
    fn blend(self, o: Self, t: f32) -> Self {
        PlayerPose {
            pos: self.pos.lerp(o.pos, t),
            yaw: lerp_angle(self.yaw, o.yaw, t),
            pitch: self.pitch + (o.pitch - self.pitch) * t,
            speed: self.speed + (o.speed - self.speed) * t,
            character: o.character,
            crouching: if t < 0.5 { self.crouching } else { o.crouching },
        }
    }
}

/// A prop's drawn pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PropPose {
    /// World position.
    pub pos: Vec3,
    /// World rotation.
    pub rot: Quat,
}

impl Blend for PropPose {
    fn blend(self, o: Self, t: f32) -> Self {
        PropPose { pos: self.pos.lerp(o.pos, t), rot: self.rot.slerp(o.rot, t) }
    }
}

impl From<&PlayerSnap> for PlayerPose {
    fn from(p: &PlayerSnap) -> Self {
        PlayerPose { pos: Vec3::from(p.pos), yaw: p.yaw, pitch: p.pitch, speed: p.speed, character: p.character, crouching: p.flags & 1 != 0 }
    }
}

impl From<&PropSnap> for PropPose {
    fn from(p: &PropSnap) -> Self {
        PropPose { pos: Vec3::from(p.pos), rot: Quat::from_array(p.rot).normalize() }
    }
}

/// Timed samples of one thing, oldest first.
#[derive(Debug, Clone)]
pub struct History<T: Blend> {
    samples: VecDeque<(f64, T)>,
}

impl<T: Blend> Default for History<T> {
    fn default() -> Self {
        History { samples: VecDeque::new() }
    }
}

impl<T: Blend> History<T> {
    /// Records a sample at server time `t` (seconds). Samples older than the newest are ignored
    /// (a reordered packet).
    pub fn push(&mut self, t: f64, v: T) {
        if self.samples.back().is_some_and(|(last, _)| t <= *last) {
            return;
        }
        self.samples.push_back((t, v));
        while self.samples.len() > HISTORY {
            self.samples.pop_front();
        }
    }

    /// The state at server time `rt`: interpolated between the two samples around it, or the nearest
    /// end (no extrapolation: a thing that stopped reporting has stopped moving).
    pub fn sample(&self, rt: f64) -> Option<T> {
        let (first, last) = (self.samples.front()?, self.samples.back()?);
        if rt <= first.0 {
            return Some(first.1);
        }
        if rt >= last.0 {
            return Some(last.1);
        }
        let hi = self.samples.iter().position(|(t, _)| *t >= rt)?;
        let (t0, a) = self.samples[hi - 1];
        let (t1, b) = self.samples[hi];
        Some(a.blend(b, ((rt - t0) / (t1 - t0)) as f32))
    }

    /// Server time of the newest sample.
    pub fn newest_time(&self) -> Option<f64> {
        self.samples.back().map(|s| s.0)
    }

    /// Number of samples held.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// Estimates `server_time - client_time` from snapshot arrivals.
#[derive(Debug, Clone, Default)]
pub struct ServerClock {
    offset: Option<f64>,
}

impl ServerClock {
    /// A snapshot stamped `server_secs` arrived at client time `local_secs`. Jitter only ever makes a
    /// packet look *later* than it was, so an estimate that would move the offset earlier is applied
    /// slowly, and one that moves it later (a quicker packet) quickly.
    pub fn observe(&mut self, server_secs: f64, local_secs: f64) {
        let sample = server_secs - local_secs;
        self.offset = Some(match self.offset {
            None => sample,
            Some(o) if sample > o => o + (sample - o) * 0.5,
            Some(o) => o + (sample - o) * 0.02,
        });
    }

    /// The server time corresponding to client time `local_secs` (`None` before any snapshot).
    pub fn server_time(&self, local_secs: f64) -> Option<f64> {
        self.offset.map(|o| local_secs + o)
    }
}

/// Everything the client draws of other players and of props, as of a moment.
#[derive(Debug, Clone, Default)]
pub struct View {
    /// Remote players `(id, pose)`, excluding the local one.
    pub players: Vec<(u8, PlayerPose)>,
    /// Props that have been reported at least once `(prop id, pose)`.
    pub props: Vec<(u16, PropPose)>,
}

/// The client's picture of the remote world.
#[derive(Debug, Default)]
pub struct RemoteWorld {
    clock: ServerClock,
    players: [Option<History<PlayerPose>>; MAX_PLAYERS_PER_SNAPSHOT],
    present: [bool; MAX_PLAYERS_PER_SNAPSHOT],
    props: Vec<Option<History<PropPose>>>,
    /// Snapshots applied.
    pub snapshots: u64,
}

impl RemoteWorld {
    /// Applies a snapshot that arrived at client time `local_secs`.
    pub fn apply(&mut self, snap: &Snapshot, local_secs: f64) {
        let t = snap.server_tick as f64 * TICK_DT as f64;
        self.clock.observe(t, local_secs);
        self.present = [false; MAX_PLAYERS_PER_SNAPSHOT];
        for p in &snap.players {
            let id = p.id as usize;
            if id >= MAX_PLAYERS_PER_SNAPSHOT {
                continue;
            }
            self.present[id] = true;
            self.players[id].get_or_insert_with(History::default).push(t, PlayerPose::from(p));
        }
        // Someone who is no longer listed has left: forget them so a newcomer in the same slot does not
        // glide in from the previous player's last position.
        for id in 0..MAX_PLAYERS_PER_SNAPSHOT {
            if !self.present[id] {
                self.players[id] = None;
            }
        }
        for q in &snap.props {
            let id = q.id as usize;
            if self.props.len() <= id {
                self.props.resize_with(id + 1, || None);
            }
            self.props[id].get_or_insert_with(History::default).push(t, PropPose::from(q));
        }
        self.snapshots += 1;
    }

    /// The client time to draw remote things at, as a server time; `None` before the first snapshot.
    pub fn render_time(&self, local_secs: f64) -> Option<f64> {
        self.clock.server_time(local_secs).map(|t| t - INTERP_DELAY)
    }

    /// The remote world at client time `local_secs`, without player `me`.
    pub fn view(&self, local_secs: f64, me: Option<u8>) -> View {
        let Some(rt) = self.render_time(local_secs) else { return View::default() };
        let players = (0..MAX_PLAYERS_PER_SNAPSHOT)
            .filter(|&i| self.present[i] && Some(i as u8) != me)
            .filter_map(|i| self.players[i].as_ref().and_then(|h| h.sample(rt)).map(|p| (i as u8, p)))
            .collect();
        let props = self.props.iter().enumerate().filter_map(|(i, h)| h.as_ref().and_then(|h| h.sample(rt)).map(|p| (i as u16, p))).collect();
        View { players, props }
    }

    /// Forgets everything (after a reconnect).
    pub fn reset(&mut self) {
        *self = RemoteWorld::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(tick: u32, x: f32) -> Snapshot {
        Snapshot {
            seq: tick,
            server_tick: tick,
            ack_input_seq: 0,
            echo_time_ms: 0,
            echo_hold_ms: 0,
            players: vec![PlayerSnap { id: 1, character: 0, flags: 0, pos: [x, 0.0, 0.0], yaw: 0.0, pitch: 0.0, speed: 3.0, vy: 0.0 }],
            props: vec![],
        }
    }

    #[test]
    fn angles_blend_the_short_way_round() {
        let a = lerp_angle(3.0, -3.0, 0.5); // 3 rad and -3 rad are close across +-pi
        assert!(a.abs() > 3.0, "{a}");
        assert!((lerp_angle(0.0, 1.0, 0.25) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn history_interpolates_holds_at_the_ends_and_ignores_reordered_samples() {
        let mut h: History<PlayerPose> = History::default();
        let pose = |x: f32| PlayerPose { pos: Vec3::new(x, 0.0, 0.0), yaw: 0.0, pitch: 0.0, speed: 0.0, character: 0, crouching: false };
        h.push(1.0, pose(0.0));
        h.push(2.0, pose(10.0));
        h.push(1.5, pose(99.0)); // reordered: dropped
        assert_eq!(h.len(), 2);
        assert!((h.sample(1.5).unwrap().pos.x - 5.0).abs() < 1e-5);
        assert_eq!(h.sample(0.0).unwrap().pos.x, 0.0, "before the first sample: the first");
        assert_eq!(h.sample(9.0).unwrap().pos.x, 10.0, "after the last: hold, do not extrapolate");
    }

    /// The smoothness guarantee: a player moving at constant speed, snapshots at 30 Hz arriving with
    /// +-15 ms jitter and 10% loss, drawn at 144 fps, never jumps and never goes backwards.
    #[test]
    fn jittery_lossy_snapshots_still_render_a_smooth_monotonic_path() {
        let v = 3.2f32; // m/s
        let mut rng = 0x9e37_79b9_7f4a_7c15u64;
        let mut rand = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng >> 11) as f64 / (1u64 << 53) as f64
        };
        // Build the arrival schedule: (arrival_local_time, snapshot).
        let mut arrivals = Vec::new();
        for k in 0..300u32 {
            let tick = k * 2;
            let sent = tick as f64 / 60.0;
            if rand() < 0.10 {
                continue; // lost
            }
            let jitter = (rand() - 0.5) * 0.030;
            arrivals.push((sent + 0.020 + jitter, snap(tick, v * tick as f32 / 60.0)));
        }
        arrivals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap()); // jitter can reorder packets
        let mut world = RemoteWorld::default();
        let (mut next, mut last_x, mut worst_step, mut frames) = (0usize, f32::NEG_INFINITY, 0.0f32, 0);
        let frame = 1.0 / 144.0;
        let mut t = 0.0;
        while t < 4.5 {
            while next < arrivals.len() && arrivals[next].0 <= t {
                world.apply(&arrivals[next].1, arrivals[next].0);
                next += 1;
            }
            if let Some(&(_, p)) = world.view(t, None).players.first() {
                if t > 0.8 {
                    assert!(p.pos.x >= last_x - 1e-4, "went backwards at t={t}: {} -> {}", last_x, p.pos.x);
                    worst_step = worst_step.max(p.pos.x - last_x);
                    frames += 1;
                }
                last_x = p.pos.x;
            }
            t += frame;
        }
        let per_frame = v / 144.0;
        assert!(frames > 400);
        assert!(worst_step < per_frame * 3.0, "largest per-frame step {worst_step} vs ideal {per_frame}");
    }

    #[test]
    fn a_player_who_leaves_is_forgotten_and_a_newcomer_does_not_glide_in() {
        let mut w = RemoteWorld::default();
        w.apply(&snap(0, 0.0), 0.0);
        w.apply(&snap(2, 1.0), 0.033);
        assert_eq!(w.view(0.2, None).players.len(), 1);
        let mut gone = snap(4, 0.0);
        gone.players.clear();
        w.apply(&gone, 0.066);
        assert!(w.view(0.2, None).players.is_empty(), "left");
        w.apply(&snap(6, 50.0), 0.1); // a new player in slot 1, far away
        w.apply(&snap(8, 50.0), 0.133);
        let v = w.view(0.3, None);
        assert!((v.players[0].1.pos.x - 50.0).abs() < 1e-3, "starts where the newcomer is: {}", v.players[0].1.pos.x);
    }

    #[test]
    fn the_local_player_is_excluded_and_props_appear_once_reported() {
        let mut w = RemoteWorld::default();
        let mut s = snap(0, 0.0);
        s.props.push(PropSnap { id: 3, pos: [1.0, 2.0, 3.0], rot: [0.0, 0.0, 0.0, 1.0] });
        w.apply(&s, 0.0);
        w.apply(&{ let mut s2 = snap(2, 0.0); s2.props.push(PropSnap { id: 3, pos: [2.0, 2.0, 3.0], rot: [0.0, 0.0, 0.0, 1.0] }); s2 }, 0.033);
        let v = w.view(0.2, Some(1));
        assert!(v.players.is_empty(), "player 1 is me");
        assert_eq!(v.props.len(), 1);
        assert_eq!(v.props[0].0, 3);
        assert!(w.view(0.2, None).players.len() == 1);
    }

    #[test]
    fn the_clock_prefers_the_least_delayed_packets() {
        let mut c = ServerClock::default();
        // Steady 20 ms delay, then one 80 ms straggler: the estimate must barely move.
        for k in 0..50 {
            let t = k as f64 * 0.033;
            c.observe(t, t + 0.020);
        }
        let before = c.server_time(10.0).unwrap();
        c.observe(1.65, 1.65 + 0.080);
        let after = c.server_time(10.0).unwrap();
        assert!((before - after).abs() < 0.005, "{before} -> {after}");
    }
}
