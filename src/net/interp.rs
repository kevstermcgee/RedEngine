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
use std::cell::Cell;
use std::collections::VecDeque;

/// How far behind the newest snapshot other players and props are drawn, seconds. Three snapshot
/// intervals at 30 Hz: two packets can be lost or late in a row and motion still interpolates.
pub const INTERP_DELAY: f64 = 0.100;
/// Samples kept per remote thing.
const HISTORY: usize = 24;
/// How far past the newest snapshot a remote *player* is carried along its last heading when snapshots stop arriving, seconds. Covers the
/// [`INTERP_DELAY`] plus a burst of about four lost snapshots; beyond it the player is held still (a real stop, or a dead link) instead of
/// running off across the map. Found by `red_engine2 net-test`: without it a burst of loss froze a remote player, then snapped them forward.
pub const MAX_EXTRAPOLATE: f64 = 0.25;
/// When the position a remote player should be drawn at jumps away from where they *were* drawn (a long outage ended), the drawn position
/// catches up at most this fast, m/s, so it glides instead of snapping. Faster than any player moves, so normal motion is never slowed.
pub const CATCH_UP_SPEED: f32 = 10.0;
/// A jump bigger than this, metres, is a teleport (a respawn) and is not glided.
pub const TELEPORT_DISTANCE: f32 = 4.0;

/// Anything that can be blended between two samples.
pub trait Blend: Copy {
    /// `t = 0` gives `self`, `t = 1` gives `other`.
    fn blend(self, other: Self, t: f32) -> Self;

    /// Where `self` (the newest sample) is `ahead` seconds later, knowing the sample before it was `dt` seconds earlier. The default
    /// holds still: only things with a trustworthy velocity override it.
    fn extrapolate(self, _prev: Self, _dt: f64, _ahead: f64) -> Self {
        self
    }
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
    /// Swinging the bat.
    pub swinging: bool,
    /// Dead (waiting to respawn).
    pub dead: bool,
    /// The weapon in hand (`weapons::Weapon::wire`).
    pub weapon: u8,
    /// The prop being carried (`protocol::NO_PROP` when none).
    pub held: u16,
    /// Hit points.
    pub hp: u8,
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
            swinging: o.swinging,
            dead: o.dead,
            weapon: o.weapon,
            held: o.held,
            hp: o.hp,
        }
    }

    /// Carries the player along the heading of their last displacement at the speed the server reported (`speed` is the true horizontal
    /// speed at the newest sample, so a player who had stopped is not carried anywhere). A jump between the last two samples bigger
    /// than a step could explain is a teleport (respawn): no extrapolation through it.
    fn extrapolate(self, prev: Self, dt: f64, ahead: f64) -> Self {
        let d = Vec3::new(self.pos.x - prev.pos.x, 0.0, self.pos.z - prev.pos.z);
        let moved = d.length();
        let possible = (self.speed.max(prev.speed) as f64 * dt * 2.5 + 0.3) as f32;
        if dt <= 0.0 || moved < 1e-3 || moved > possible || self.speed <= 0.05 {
            return self;
        }
        let step = d / moved * (self.speed as f64 * ahead) as f32;
        PlayerPose { pos: self.pos + step, ..self }
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
        PlayerPose {
            pos: Vec3::from(p.pos),
            yaw: p.yaw,
            pitch: p.pitch,
            speed: p.speed,
            character: p.character,
            crouching: p.flags & 1 != 0,
            swinging: p.flags & 2 != 0,
            dead: p.flags & 4 != 0,
            weapon: p.weapon,
            held: p.held,
            hp: p.hp,
        }
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

    /// The state at server time `rt`: interpolated between the two samples around it; before the first sample, the first; after the newest,
    /// the newest carried on for at most [`MAX_EXTRAPOLATE`] seconds if the thing knows how ([`Blend::extrapolate`]: players do, props hold).
    pub fn sample(&self, rt: f64) -> Option<T> {
        let (first, last) = (self.samples.front()?, self.samples.back()?);
        if rt <= first.0 {
            return Some(first.1);
        }
        if rt >= last.0 {
            let ahead = (rt - last.0).min(MAX_EXTRAPOLATE);
            let prev = self.samples.len().checked_sub(2).and_then(|i| self.samples.get(i));
            return Some(match prev {
                Some(p) if ahead > 0.0 => last.1.extrapolate(p.1, last.0 - p.0, ahead),
                _ => last.1,
            });
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

/// `target` limited so the drawn position never moves faster than [`CATCH_UP_SPEED`] away from where it was last drawn (`last`), except for a
/// teleport. Records the result in `last`. Normal motion is far slower than the limit, so this only ever acts after an outage.
fn limit_catch_up(last: &mut Option<(f64, Vec3)>, now: f64, target: Vec3) -> Vec3 {
    let out = match *last {
        Some((t0, prev)) => {
            let dt = (now - t0).clamp(0.0, 0.1) as f32;
            let d = target - prev;
            let dist = d.length();
            if dist <= TELEPORT_DISTANCE && dist > CATCH_UP_SPEED * dt {
                prev + d / dist * (CATCH_UP_SPEED * dt)
            } else {
                target
            }
        }
        None => target,
    };
    *last = Some((now, out));
    out
}

/// The client's picture of the remote world.
#[derive(Debug, Default)]
pub struct RemoteWorld {
    clock: ServerClock,
    players: [Option<History<PlayerPose>>; MAX_PLAYERS_PER_SNAPSHOT],
    present: [bool; MAX_PLAYERS_PER_SNAPSHOT],
    props: Vec<Option<History<PropPose>>>,
    /// Where each remote player was last drawn `(client time, position)`, for the catch-up limit in [`RemoteWorld::view`].
    drawn: Cell<[Option<(f64, Vec3)>; MAX_PLAYERS_PER_SNAPSHOT]>,
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
                let mut d = self.drawn.get();
                d[id] = None;
                self.drawn.set(d);
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
        let mut drawn = self.drawn.get();
        let players = (0..MAX_PLAYERS_PER_SNAPSHOT)
            .filter(|&i| self.present[i] && Some(i as u8) != me)
            .filter_map(|i| {
                let mut pose = self.players[i].as_ref().and_then(|h| h.sample(rt))?;
                pose.pos = limit_catch_up(&mut drawn[i], local_secs, pose.pos);
                Some((i as u8, pose))
            })
            .collect();
        self.drawn.set(drawn);
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
            players: vec![PlayerSnap {
                id: 1,
                character: 0,
                flags: 0,
                pos: [x, 0.0, 0.0],
                yaw: 0.0,
                pitch: 0.0,
                speed: 3.0,
                vy: 0.0,
                weapon: 0,
                held: crate::net::protocol::NO_PROP,
                hp: 100,
            }],
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
        let pose = |x: f32| PlayerPose {
            pos: Vec3::new(x, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            speed: 0.0,
            character: 0,
            crouching: false,
            swinging: false,
            dead: false,
            weapon: 0,
            held: u16::MAX,
            hp: 100,
        };
        h.push(1.0, pose(0.0));
        h.push(2.0, pose(10.0));
        h.push(1.5, pose(99.0)); // reordered: dropped
        assert_eq!(h.len(), 2);
        assert!((h.sample(1.5).unwrap().pos.x - 5.0).abs() < 1e-5);
        assert_eq!(h.sample(0.0).unwrap().pos.x, 0.0, "before the first sample: the first");
        assert_eq!(h.sample(9.0).unwrap().pos.x, 10.0, "after the last: hold when the player was standing still");
    }

    fn moving(x: f32, speed: f32) -> PlayerPose {
        PlayerPose {
            pos: Vec3::new(x, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            speed,
            character: 0,
            crouching: false,
            swinging: false,
            dead: false,
            weapon: 0,
            held: u16::MAX,
            hp: 100,
        }
    }

    #[test]
    fn a_burst_of_lost_snapshots_carries_a_moving_player_on_instead_of_freezing_then_snapping() {
        let mut h: History<PlayerPose> = History::default();
        h.push(1.000, moving(3.000, 3.0));
        h.push(1.033, moving(3.100, 3.0)); // 3 m/s
                                           // 150 ms later, no new snapshot: the player is drawn 0.45 m further along, not stuck at 3.1.
        let x = h.sample(1.183).unwrap().pos.x;
        assert!((x - (3.1 + 3.0 * 0.15)).abs() < 1e-3, "{x}");
        // But not forever: past MAX_EXTRAPOLATE they are held (a dead link must not send them across the map).
        let far = h.sample(9.0).unwrap().pos.x;
        assert!((far - (3.1 + 3.0 * MAX_EXTRAPOLATE as f32)).abs() < 1e-3, "{far}");
        // A player whose last reported speed was 0 is not carried anywhere, whatever the sample before showed.
        let mut stopped: History<PlayerPose> = History::default();
        stopped.push(1.0, moving(3.0, 3.0));
        stopped.push(1.033, moving(3.1, 0.0));
        assert_eq!(stopped.sample(1.2).unwrap().pos.x, 3.1);
        // A teleport between the last two samples (a respawn) is not extrapolated through.
        let mut tp: History<PlayerPose> = History::default();
        tp.push(1.0, moving(-20.0, 3.0));
        tp.push(1.033, moving(30.0, 3.0));
        assert_eq!(tp.sample(1.2).unwrap().pos.x, 30.0);
    }

    #[test]
    fn a_long_outage_ends_in_a_quick_glide_not_a_snap_but_a_teleport_still_snaps() {
        let mut w = RemoteWorld::default();
        w.apply(&snap(0, 0.0), 0.0);
        w.apply(&snap(2, 0.1), 0.033);
        let at = |w: &RemoteWorld, t: f64| w.view(t, None).players[0].1.pos.x;
        let before = at(&w, 0.2);
        // Half a second of silence, then the player is reported 2 m further on.
        w.apply(&snap(40, 2.0), 0.75);
        w.apply(&snap(42, 2.1), 0.783);
        let mut t = 0.8;
        let mut prev = at(&w, t);
        let mut worst = 0.0f32;
        while t < 1.2 {
            t += 1.0 / 144.0;
            let x = at(&w, t);
            worst = worst.max((x - prev).abs());
            prev = x;
        }
        assert!(before < 1.0 && worst < CATCH_UP_SPEED / 144.0 * 1.2, "worst per-frame step {worst}");
        // A respawn (a jump of 30 m) is not glided across the map.
        w.apply(&snap(80, 30.0), 1.3);
        w.apply(&snap(82, 30.1), 1.333);
        assert!((at(&w, 1.6) - 30.0).abs() < 1.0, "teleports snap: {}", at(&w, 1.6));
    }

    #[test]
    fn extrapolated_motion_meets_the_real_snapshot_without_a_visible_jump() {
        // A player at a steady 3.2 m/s; snapshots 20-23 are lost (a 4-snapshot burst), then 24 arrives.
        let v = 3.2f32;
        let mut world = RemoteWorld::default();
        let mut last = None::<f32>;
        let (mut worst, frame) = (0.0f32, 1.0 / 144.0);
        let mut arrivals: Vec<(f64, Snapshot)> =
            (0..40u32).filter(|k| !(20..24).contains(k)).map(|k| (k as f64 / 30.0 + 0.02, snap(k * 2, v * (k * 2) as f32 / 60.0))).collect();
        arrivals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let (mut t, mut next) = (0.0, 0);
        while t < 1.3 {
            while next < arrivals.len() && arrivals[next].0 <= t {
                world.apply(&arrivals[next].1, arrivals[next].0);
                next += 1;
            }
            if let Some(&(_, p)) = world.view(t, None).players.first() {
                if t > 0.5 {
                    if let Some(l) = last {
                        worst = worst.max((p.pos.x - l).abs());
                    }
                }
                last = Some(p.pos.x);
            }
            t += frame;
        }
        assert!(worst < v * frame as f32 * 2.5, "largest per-frame step {worst} vs ideal {}: extrapolation should bridge the gap", v * frame as f32);
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
        w.apply(
            &{
                let mut s2 = snap(2, 0.0);
                s2.props.push(PropSnap { id: 3, pos: [2.0, 2.0, 3.0], rot: [0.0, 0.0, 0.0, 1.0] });
                s2
            },
            0.033,
        );
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
