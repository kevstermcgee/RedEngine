//! Race progress for Great Outdoors: the scene's `race` block and a deterministic per-player state machine (ADR
//! 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature).
//!
//! The generic rules keep their variables global, so "which lap is *this* player on" cannot be written as rules. This module owns it instead: pure, no
//! renderer, no sockets, folded into the match checksum like the rules state.
//!
//! A course is an ordered list of **gates**, each a scene zone laid across the track. Gate 0 is the start/finish line; the grid sits just *before* it.
//! Everyone begins with gate 1 as the next gate, so the first crossing of the line at the start counts for nothing; a lap is gate 1, 2, ..., N-1 and then
//! the line again. Gates only count **in order** and only **in the direction of travel** (from the previous gate's centre towards the next one's), so
//! cutting the track skips nothing and reversing through a gate earns nothing. Crossing is a segment test against the previous position, so a fast kart
//! cannot step over a thin gate.
//!
//! ```json
//! "zones": [{"id": "line", "rect": [-6, -1, 6, 1]}, {"id": "bend", "rect": [30, 40, 40, 44]}, {"id": "back", "rect": [-6, 79, 6, 81]}],
//! "race": {"laps": 3, "gates": ["line", "bend", "back"], "countdown_secs": 3}
//! ```

use crate::sim::clock::TICK_RATE_HZ;
use crate::strict::check_keys;
use glam::Vec2;
use serde_json::{Map, Value};
use std::sync::Arc;

/// Keys of the `race` block.
pub const RACE_KEYS: &[&str] = &["laps", "gates", "countdown_secs", "finish_grace_secs", "line", "item_boxes", "item_respawn_secs"];
/// Most laps a race may have.
pub const MAX_LAPS: u8 = 20;
/// Fewest gates a course may have (the line and two more, or a kart could cut straight across).
pub const MIN_GATES: usize = 3;

/// One gate: an axis-aligned rectangle across the track, in the ground plane.
#[derive(Debug, Clone, PartialEq)]
pub struct Gate {
    /// The zone id it came from.
    pub id: String,
    /// Smallest x, z corner.
    pub min: Vec2,
    /// Largest x, z corner.
    pub max: Vec2,
    /// Unit vector a kart must be travelling along (roughly) to cross it in the right direction.
    pub forward: Vec2,
}

impl Gate {
    /// The middle of the gate.
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    /// Whether a kart moving from `from` to `to` in one tick crossed this gate the right way round.
    pub fn crossed_by(&self, from: Vec2, to: Vec2) -> bool {
        (to - from).dot(self.forward) > 0.0 && segment_hits_rect(from, to, self.min, self.max)
    }
}

/// Whether the segment `a`-`b` touches the rectangle (a segment that starts inside it counts).
pub fn segment_hits_rect(a: Vec2, b: Vec2, min: Vec2, max: Vec2) -> bool {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for axis in 0..2 {
        if d[axis].abs() < 1e-9 {
            if a[axis] < min[axis] || a[axis] > max[axis] {
                return false;
            }
        } else {
            let (mut u, mut v) = ((min[axis] - a[axis]) / d[axis], (max[axis] - a[axis]) / d[axis]);
            if u > v {
                std::mem::swap(&mut u, &mut v);
            }
            t0 = t0.max(u);
            t1 = t1.min(v);
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// The scene's `race` block, resolved against its zones.
#[derive(Debug, Clone, PartialEq)]
pub struct RaceCourse {
    /// Laps to win (default 3).
    pub laps: u8,
    /// The gates in driving order; `gates[0]` is the start/finish line.
    pub gates: Vec<Gate>,
    /// Frozen countdown before the green light, seconds (default 3).
    pub countdown_secs: f32,
    /// Once the first kart finishes, how long the rest have before the race ends and they are ranked as they stand, seconds (default 30).
    pub finish_grace_secs: f32,
    /// Optional racing line (x, z points along the track, in order) for bots to follow on tight corners; empty = the gate centres.
    pub line: Vec<Vec2>,
    /// Item boxes on the track: the zone id and its rectangle. A kart that touches one gets an item (`sim::items`).
    pub item_boxes: Vec<(String, Vec2, Vec2)>,
    /// How long an item box takes to come back after being taken, seconds (default 5).
    pub item_respawn_secs: f32,
}

/// The rectangle of the zone called `id`, resolved against a scene's `zones`. `path` names the field being read and `what` the thing a zone is being
/// used as, for the messages.
fn zone_rect(zones: &[Value], id: &str, path: &str, what: &str) -> Result<(Vec2, Vec2), String> {
    let zone = zones.iter().find(|z| z.get("id").and_then(Value::as_str) == Some(id)).ok_or_else(|| format!("{path}: no zone '{id}' ({what})"))?;
    let r: Vec<f64> = zone.get("rect").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    if r.len() != 4 || r.iter().any(|n| !n.is_finite()) {
        return Err(format!("{path}: zone '{id}' needs a rect [x0, z0, x1, z1]"));
    }
    Ok((Vec2::new(r[0].min(r[2]) as f32, r[1].min(r[3]) as f32), Vec2::new(r[0].max(r[2]) as f32, r[1].max(r[3]) as f32)))
}

impl RaceCourse {
    /// Reads the `race` block of a scene's JSON text: `Ok(None)` when the scene has none.
    pub fn from_scene_text(text: &str) -> Result<Option<RaceCourse>, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("scene is not JSON: {e}"))?;
        match v.as_object() {
            Some(root) => Self::from_scene_json(root),
            None => Ok(None),
        }
    }

    /// Reads the `race` block of a parsed scene root, resolving gate ids against its `zones`.
    pub fn from_scene_json(root: &Map<String, Value>) -> Result<Option<RaceCourse>, String> {
        let Some(block) = root.get("race") else { return Ok(None) };
        let obj = block.as_object().ok_or("race: must be an object")?;
        let mut errs = Vec::new();
        check_keys(&mut errs, "race", obj, RACE_KEYS);
        if let Some(e) = errs.into_iter().next() {
            return Err(e);
        }
        let laps = match obj.get("laps") {
            None => 3,
            Some(v) => v.as_u64().filter(|n| (1..=MAX_LAPS as u64).contains(n)).ok_or(format!("race.laps: must be a whole number 1..{MAX_LAPS}"))? as u8,
        };
        let secs = |key: &str, default: f32| -> Result<f32, String> {
            match obj.get(key) {
                None => Ok(default),
                Some(v) => v
                    .as_f64()
                    .filter(|n| n.is_finite() && (0.0..=600.0).contains(n))
                    .map(|n| n as f32)
                    .ok_or(format!("race.{key}: must be a number of seconds 0..600")),
            }
        };
        let (countdown_secs, finish_grace_secs) = (secs("countdown_secs", 3.0)?, secs("finish_grace_secs", 30.0)?);
        let ids: Vec<&str> = obj
            .get("gates")
            .and_then(Value::as_array)
            .ok_or("race.gates: give the ordered list of zone ids across the track (the first is the start/finish line)")?
            .iter()
            .map(|g| g.as_str().ok_or("race.gates: every entry must be a zone id string"))
            .collect::<Result<_, _>>()?;
        if ids.len() < MIN_GATES {
            return Err(format!("race.gates: needs at least {MIN_GATES} gates (the line and two more), or a kart could cut straight across"));
        }
        let zones = root.get("zones").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        let mut rects: Vec<(String, Vec2, Vec2)> = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            if ids[..i].contains(id) {
                return Err(format!("race.gates: zone '{id}' is listed twice"));
            }
            let (min, max) = zone_rect(zones, id, &format!("race.gates[{i}]"), "a gate is a scene zone laid across the track")?;
            rects.push((id.to_string(), min, max));
        }
        // Each gate faces along the line from the previous gate's centre to the next one's (a central difference, so it holds on bends).
        let n = rects.len();
        let centre = |i: usize| (rects[i].1 + rects[i].2) * 0.5;
        let mut gates = Vec::with_capacity(n);
        for i in 0..n {
            let forward = (centre((i + 1) % n) - centre((i + n - 1) % n)).normalize_or_zero();
            if forward == Vec2::ZERO {
                return Err(format!("race.gates[{i}]: zone '{}' sits between two gates at the same place, so it has no direction", rects[i].0));
            }
            gates.push(Gate { id: rects[i].0.clone(), min: rects[i].1, max: rects[i].2, forward });
        }
        let mut line = Vec::new();
        if let Some(points) = obj.get("line") {
            for (i, p) in points.as_array().ok_or("race.line: must be a list of [x, z] points")?.iter().enumerate() {
                let xz: Vec<f64> = p.as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
                if xz.len() != 2 || xz.iter().any(|n| !n.is_finite()) {
                    return Err(format!("race.line[{i}]: must be [x, z]"));
                }
                line.push(Vec2::new(xz[0] as f32, xz[1] as f32));
            }
        }
        let mut item_boxes = Vec::new();
        if let Some(list) = obj.get("item_boxes") {
            for (i, id) in list.as_array().ok_or("race.item_boxes: must be a list of zone ids")?.iter().enumerate() {
                let id = id.as_str().ok_or_else(|| format!("race.item_boxes[{i}]: must be a zone id string"))?;
                let (min, max) = zone_rect(zones, id, &format!("race.item_boxes[{i}]"), "an item box is a scene zone where the box sits")?;
                item_boxes.push((id.to_string(), min, max));
            }
        }
        let item_respawn_secs = secs("item_respawn_secs", 5.0)?;
        Ok(Some(RaceCourse { laps, gates, countdown_secs, finish_grace_secs, line, item_boxes, item_respawn_secs }))
    }
}

/// Where a race is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Karts are held on the grid until the light.
    Countdown,
    /// Racing: karts may drive and gates count.
    Racing,
    /// Over: everyone still driving has finished, or the grace period ran out.
    Finished,
}

/// One player's progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Laps completed.
    pub lap: u8,
    /// The next gate to cross (`1` after the start; `0` means the line closes the lap).
    pub next: u8,
    /// The race tick at which they finished, if they have.
    pub finished_at: Option<u32>,
}

/// One row of the standings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    /// The player's slot.
    pub player: usize,
    /// Laps completed.
    pub lap: u8,
    /// The next gate they must cross.
    pub next_gate: u8,
    /// Race tick of their finish, or `None` while still racing (or if they never finished).
    pub finished_at: Option<u32>,
}

/// The race for one match: the course and every player's progress. Advance it once per simulation tick with the karts' positions.
#[derive(Debug, Clone)]
pub struct RaceState {
    course: Arc<RaceCourse>,
    phase: Phase,
    /// Ticks of racing so far.
    tick: u32,
    countdown_left: u32,
    progress: Vec<Progress>,
    last: Vec<Option<Vec2>>,
    started: Vec<bool>,
    deadline: Option<u32>,
    finish_order: Vec<usize>,
}

fn ticks(secs: f32) -> u32 {
    (secs * TICK_RATE_HZ as f32).round() as u32
}

impl RaceState {
    /// A race for `players` slots on `course`, in its countdown.
    pub fn new(course: Arc<RaceCourse>, players: usize) -> RaceState {
        let countdown_left = ticks(course.countdown_secs);
        let phase = if countdown_left == 0 { Phase::Racing } else { Phase::Countdown };
        RaceState {
            course,
            phase,
            tick: 0,
            countdown_left,
            progress: vec![Progress { lap: 0, next: 1, finished_at: None }; players],
            last: vec![None; players],
            started: vec![false; players],
            deadline: None,
            finish_order: Vec::new(),
        }
    }

    /// The course.
    pub fn course(&self) -> &RaceCourse {
        &self.course
    }

    /// Where the race is in its life.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether karts may drive (the light is green and the race is not over).
    pub fn can_drive(&self) -> bool {
        self.phase == Phase::Racing
    }

    /// Racing ticks so far (0 during the countdown).
    pub fn race_tick(&self) -> u32 {
        self.tick
    }

    /// Ticks of countdown left (0 once racing).
    pub fn countdown_ticks_left(&self) -> u32 {
        self.countdown_left
    }

    /// One player's progress.
    pub fn progress(&self, player: usize) -> Option<Progress> {
        self.progress.get(player).copied()
    }

    /// The lap a player is on, counting from 1 (for the HUD); the total once they have finished.
    pub fn current_lap(&self, player: usize) -> u8 {
        self.progress.get(player).map_or(1, |p| if p.finished_at.is_some() { self.course.laps } else { (p.lap + 1).min(self.course.laps) })
    }

    /// Advances one fixed tick. `positions[i]` is player `i`'s kart position, or `None` if that slot is empty or disconnected.
    pub fn tick(&mut self, positions: &[Option<Vec2>]) {
        match self.phase {
            Phase::Finished => return,
            Phase::Countdown => {
                self.countdown_left = self.countdown_left.saturating_sub(1);
                if self.countdown_left == 0 {
                    self.phase = Phase::Racing;
                }
            }
            Phase::Racing => self.tick += 1,
        }
        let racing = self.phase == Phase::Racing;
        for i in 0..self.progress.len() {
            let now = positions.get(i).copied().flatten();
            if now.is_some() {
                self.started[i] = true;
            }
            if let (true, Some(to), Some(from)) = (racing, now, self.last[i]) {
                self.cross(i, from, to);
            }
            self.last[i] = now;
        }
        if racing {
            let all_done = (0..self.progress.len()).all(|i| !self.started[i] || self.last[i].is_none() || self.progress[i].finished_at.is_some());
            if all_done || self.deadline.is_some_and(|d| self.tick >= d) {
                self.phase = Phase::Finished;
            }
        }
    }

    fn cross(&mut self, player: usize, from: Vec2, to: Vec2) {
        let p = self.progress[player];
        if p.finished_at.is_some() || !self.course.gates[p.next as usize].crossed_by(from, to) {
            return;
        }
        let n = self.course.gates.len() as u8;
        let mut p = p;
        if p.next == 0 {
            p.lap += 1;
            p.next = 1;
            if p.lap >= self.course.laps {
                p.finished_at = Some(self.tick);
                self.finish_order.push(player);
                if self.deadline.is_none() {
                    self.deadline = Some(self.tick + ticks(self.course.finish_grace_secs));
                }
            }
        } else {
            p.next = (p.next + 1) % n;
        }
        self.progress[player] = p;
    }

    /// Gates a player has passed in total (laps times gates, plus the ones this lap): the primary key of the standings.
    fn gates_done(&self, p: &Progress) -> u32 {
        let n = self.course.gates.len() as u32;
        p.lap as u32 * n + if p.next == 0 { n } else { p.next as u32 } - 1
    }

    /// Every player who has been on the grid, best first: finishers by finish time, then the rest by gates passed and, on a tie, by how close they
    /// are to the next gate. Equal on everything: lower slot first.
    pub fn standings(&self) -> Vec<Standing> {
        let dist = |i: usize| -> f32 {
            let gate = &self.course.gates[self.progress[i].next as usize];
            self.last[i].map_or(f32::MAX, |p| (gate.center() - p).length())
        };
        let mut order: Vec<usize> = (0..self.progress.len()).filter(|i| self.started[*i]).collect();
        order.sort_by(|&a, &b| {
            let (pa, pb) = (&self.progress[a], &self.progress[b]);
            match (pa.finished_at, pb.finished_at) {
                (Some(x), Some(y)) => x.cmp(&y).then(a.cmp(&b)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => self.gates_done(pb).cmp(&self.gates_done(pa)).then(dist(a).total_cmp(&dist(b))).then(a.cmp(&b)),
            }
        });
        order
            .into_iter()
            .map(|i| Standing { player: i, lap: self.progress[i].lap, next_gate: self.progress[i].next, finished_at: self.progress[i].finished_at })
            .collect()
    }

    /// A player's place, counting from 1, or `None` if they were never on the grid.
    pub fn place_of(&self, player: usize) -> Option<usize> {
        self.standings().iter().position(|s| s.player == player).map(|p| p + 1)
    }

    /// A 64-bit fold of all race state, for the match checksum (replays and prediction check against it).
    pub fn checksum(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        mix(match self.phase {
            Phase::Countdown => 1,
            Phase::Racing => 2,
            Phase::Finished => 3,
        });
        mix(self.tick as u64);
        mix(self.countdown_left as u64);
        mix(self.deadline.map_or(u64::MAX, |d| d as u64));
        for (p, s) in self.progress.iter().zip(&self.started) {
            mix(p.lap as u64 | (p.next as u64) << 8 | (*s as u64) << 16);
            mix(p.finished_at.map_or(u64::MAX, |t| t as u64));
        }
        for who in &self.finish_order {
            mix(*who as u64 + 1);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loop of four gates whose centres are the corners of a diamond of radius 40 around the origin: line at (0,-40), then (40,0), (0,40), (-40,0).
    fn scene(extra: &str) -> String {
        format!(
            r#"{{"zones":[{{"id":"line","rect":[-6,-41,6,-39]}},{{"id":"east","rect":[39,-6,41,6]}},{{"id":"south","rect":[-6,39,6,41]}},{{"id":"west","rect":[-41,-6,-39,6]}}],
               "race":{{"gates":["line","east","south","west"]{extra}}}}}"#
        )
    }

    fn course(extra: &str) -> Arc<RaceCourse> {
        Arc::new(RaceCourse::from_scene_text(&scene(extra)).unwrap().unwrap())
    }

    const CORNERS: [Vec2; 4] = [Vec2::new(0.0, -40.0), Vec2::new(40.0, 0.0), Vec2::new(0.0, 40.0), Vec2::new(-40.0, 0.0)];

    /// Drives player `i` from wherever it is through `points` at `step` metres per tick, ticking the race with everyone else held at `others`.
    fn drive(race: &mut RaceState, positions: &mut [Option<Vec2>], i: usize, points: &[Vec2], step: f32) {
        for &target in points {
            loop {
                let here = positions[i].unwrap();
                let to = target - here;
                if to.length() <= step {
                    positions[i] = Some(target);
                    race.tick(positions);
                    break;
                }
                positions[i] = Some(here + to.normalize() * step);
                race.tick(positions);
            }
        }
    }

    fn racing(players: usize, extra: &str) -> (RaceState, Vec<Option<Vec2>>) {
        let mut race = RaceState::new(course(&format!(r#","countdown_secs":0{extra}"#)), players);
        let start = Some(Vec2::new(0.0, -46.0)); // the grid sits just before the line
        let mut positions = vec![start; players];
        race.tick(&positions);
        positions.iter_mut().for_each(|p| *p = start);
        (race, positions)
    }

    #[test]
    fn the_block_is_read_and_every_mistake_names_its_field() {
        let c = RaceCourse::from_scene_text(&scene("")).unwrap().unwrap();
        assert_eq!((c.laps, c.gates.len(), c.countdown_secs, c.finish_grace_secs), (3, 4, 3.0, 30.0));
        assert!(c.gates[0].forward.dot(Vec2::X) > 0.5, "the line faces along the track (towards the east gate): {:?}", c.gates[0].forward);
        assert_eq!(RaceCourse::from_scene_text(r#"{"zones":[]}"#), Ok(None), "no block: no race");
        let err = |extra: &str| RaceCourse::from_scene_text(&scene(extra)).unwrap_err();
        assert!(err(r#","lap":3"#).contains("lap"), "typo'd key");
        assert!(err(r#","laps":0"#).contains("race.laps") && err(r#","laps":21"#).contains("race.laps") && err(r#","laps":1.5"#).contains("race.laps"));
        assert!(err(r#","countdown_secs":-1"#).contains("countdown_secs"));
        assert!(err(r#","line":[[1]]"#).contains("race.line[0]"));
        let bad = |gates: &str| {
            RaceCourse::from_scene_text(&scene("").replace(r#""gates":["line","east","south","west"]"#, &format!(r#""gates":{gates}"#))).unwrap_err()
        };
        assert!(bad(r#"["line","east"]"#).contains("at least 3"));
        assert!(bad(r#"["line","east","nowhere"]"#).contains("no zone 'nowhere'"));
        assert!(bad(r#"["line","east","east"]"#).contains("twice"));
        assert!(bad(r#"["line",7,"south"]"#).contains("zone id string"));
        assert!(RaceCourse::from_scene_text(r#"{"race":[]}"#).unwrap_err().contains("object"));
        let boxes = RaceCourse::from_scene_text(&scene(r#","item_boxes":["east","west"],"item_respawn_secs":8"#)).unwrap().unwrap();
        assert_eq!((boxes.item_boxes.len(), boxes.item_boxes[0].0.as_str(), boxes.item_respawn_secs), (2, "east", 8.0));
        assert_eq!(c.item_boxes.len(), 0, "no boxes unless asked for");
        assert!(err(r#","item_boxes":["nowhere"]"#).contains("race.item_boxes[0]: no zone 'nowhere'"));
        assert!(err(r#","item_boxes":[3]"#).contains("race.item_boxes[0]"));
        assert!(err(r#","item_boxes":"east""#).contains("list of zone ids"));
        assert!(err(r#","item_respawn_secs":-2"#).contains("item_respawn_secs"));
        let with_line = RaceCourse::from_scene_text(&scene(r#","line":[[0,-40],[20,-30]]"#)).unwrap().unwrap();
        assert_eq!(with_line.line, vec![Vec2::new(0.0, -40.0), Vec2::new(20.0, -30.0)]);
    }

    #[test]
    fn a_gate_counts_only_forward_and_cannot_be_stepped_over() {
        let c = course("");
        let line = &c.gates[0];
        assert!(line.crossed_by(Vec2::new(-4.0, -40.0), Vec2::new(4.0, -40.0)), "along the track through the line");
        assert!(!line.crossed_by(Vec2::new(4.0, -40.0), Vec2::new(-4.0, -40.0)), "backwards through it");
        assert!(line.crossed_by(Vec2::new(-30.0, -40.0), Vec2::new(30.0, -40.0)), "a huge step over a 2 m gate still crosses it");
        assert!(!line.crossed_by(Vec2::new(-30.0, -60.0), Vec2::new(30.0, -60.0)), "a step that misses does not");
        assert!(segment_hits_rect(Vec2::new(1.0, -40.0), Vec2::new(1.0, -40.0), line.min, line.max), "a point inside counts");
    }

    #[test]
    fn the_countdown_holds_the_karts_then_the_light_goes_green() {
        let mut race = RaceState::new(course(r#","countdown_secs":2"#), 2);
        assert_eq!((race.phase(), race.can_drive(), race.countdown_ticks_left()), (Phase::Countdown, false, 120));
        let positions = vec![Some(Vec2::new(0.0, -46.0)); 2];
        for _ in 0..119 {
            race.tick(&positions);
        }
        assert_eq!((race.can_drive(), race.race_tick()), (false, 0));
        race.tick(&positions);
        assert_eq!((race.phase(), race.can_drive()), (Phase::Racing, true));
        race.tick(&positions);
        assert_eq!(race.race_tick(), 1);
        assert_eq!(RaceState::new(course(r#","countdown_secs":0"#), 1).phase(), Phase::Racing, "no countdown: straight to racing");
    }

    #[test]
    fn laps_count_in_order_and_the_start_line_at_the_start_counts_for_nothing() {
        let (mut race, mut pos) = racing(1, r#","laps":2"#);
        // Crossing the line on the way out, as everyone does from the grid, is not a lap.
        drive(&mut race, &mut pos, 0, &[Vec2::new(0.0, -34.0)], 0.4);
        assert_eq!(race.progress(0), Some(Progress { lap: 0, next: 1, finished_at: None }));
        assert_eq!(race.current_lap(0), 1);
        drive(&mut race, &mut pos, 0, &[CORNERS[1], CORNERS[2], CORNERS[3]], 0.4);
        assert_eq!(race.progress(0).unwrap().next, 0, "all the checkpoints done: the line closes the lap");
        drive(&mut race, &mut pos, 0, &[CORNERS[0], Vec2::new(4.0, -34.0)], 0.4);
        assert_eq!(race.progress(0), Some(Progress { lap: 1, next: 1, finished_at: None }));
        assert_eq!((race.current_lap(0), race.phase()), (2, Phase::Racing));
        drive(&mut race, &mut pos, 0, &[CORNERS[1], CORNERS[2], CORNERS[3], CORNERS[0], Vec2::new(4.0, -34.0)], 0.4);
        let p = race.progress(0).unwrap();
        assert_eq!(
            (p.lap, p.finished_at.is_some(), race.phase(), race.can_drive()),
            (2, true, Phase::Finished, false),
            "two laps: finished, and the race is over"
        );
        assert_eq!(race.current_lap(0), 2);
    }

    #[test]
    fn cutting_the_track_and_reversing_earn_nothing() {
        let (mut race, mut pos) = racing(1, "");
        // Skip the east gate: go straight from the start to the south gate, then the west gate and the line.
        drive(&mut race, &mut pos, 0, &[CORNERS[2], CORNERS[3], CORNERS[0], Vec2::new(4.0, -34.0)], 0.4);
        assert_eq!(race.progress(0), Some(Progress { lap: 0, next: 1, finished_at: None }), "the missed gate is still owed");
        // Go the right way round to east, then reverse back over it and forward again: only one pass counts.
        drive(&mut race, &mut pos, 0, &[CORNERS[1]], 0.4);
        assert_eq!(race.progress(0).unwrap().next, 2);
        drive(&mut race, &mut pos, 0, &[Vec2::new(40.0, -10.0), CORNERS[1], Vec2::new(40.0, 10.0)], 0.4);
        assert_eq!(race.progress(0).unwrap().next, 2, "going back over a passed gate changes nothing");
        // Driving the whole course the wrong way round never completes a lap.
        let (mut wrong, mut wpos) = racing(1, "");
        drive(
            &mut wrong,
            &mut wpos,
            0,
            &[CORNERS[3], CORNERS[2], CORNERS[1], CORNERS[0], Vec2::new(-4.0, -34.0), CORNERS[3], CORNERS[2], CORNERS[1], CORNERS[0]],
            0.4,
        );
        assert_eq!(wrong.progress(0).unwrap().lap, 0);
    }

    #[test]
    fn standings_rank_finishers_then_progress_then_closeness() {
        let (mut race, mut pos) = racing(3, r#","laps":1,"finish_grace_secs":600"#);
        pos[1] = Some(Vec2::new(0.0, -46.0));
        pos[2] = Some(Vec2::new(0.0, -46.0));
        // Player 2 clears the east gate; player 1 gets near it; player 0 stays on the grid.
        let only = |i: usize, points: &[Vec2], race: &mut RaceState, pos: &mut [Option<Vec2>]| drive(race, pos, i, points, 0.4);
        only(2, &[Vec2::new(0.0, -34.0), CORNERS[1], Vec2::new(40.0, 5.0)], &mut race, &mut pos);
        only(1, &[Vec2::new(0.0, -34.0), Vec2::new(30.0, -5.0)], &mut race, &mut pos);
        let order: Vec<usize> = race.standings().iter().map(|s| s.player).collect();
        assert_eq!(order, vec![2, 1, 0], "one gate ahead beats none; among equals, nearer the next gate beats the grid");
        assert_eq!((race.place_of(2), race.place_of(0), race.place_of(9)), (Some(1), Some(3), None));
        // Player 1 finishes the lap first: finishers come before everyone still driving, whatever their progress.
        only(1, &[CORNERS[1], CORNERS[2], CORNERS[3], CORNERS[0], Vec2::new(4.0, -34.0)], &mut race, &mut pos);
        assert!(race.progress(1).unwrap().finished_at.is_some());
        assert_eq!(race.standings()[0].player, 1);
        assert_eq!(race.phase(), Phase::Racing, "the others are still driving");
    }

    #[test]
    fn the_grace_period_ends_the_race_and_absent_players_do_not_hold_it_up() {
        let mut race = RaceState::new(course(r#","countdown_secs":0,"laps":1,"finish_grace_secs":1"#), 3);
        let grid = Some(Vec2::new(0.0, -46.0));
        let mut pos = vec![grid, grid, None]; // slot 2 never connected
        race.tick(&pos);
        drive(&mut race, &mut pos, 0, &[Vec2::new(0.0, -34.0), CORNERS[1], CORNERS[2], CORNERS[3], CORNERS[0], Vec2::new(4.0, -34.0)], 0.4);
        assert!(race.progress(0).unwrap().finished_at.is_some());
        assert_eq!(race.phase(), Phase::Racing, "player 1 is on the grid and still has their second");
        let finished = race.progress(0).unwrap().finished_at.unwrap();
        while race.race_tick() < finished + 59 {
            race.tick(&pos);
        }
        assert_eq!(race.phase(), Phase::Racing, "one tick short of the one-second deadline");
        race.tick(&pos);
        assert_eq!(race.phase(), Phase::Finished);
        let order: Vec<usize> = race.standings().iter().map(|s| s.player).collect();
        assert_eq!(order, vec![0, 1], "the winner, then the one who did not finish; a slot that never joined is not ranked");
        // A player who leaves mid-race does not hold the race open either.
        let (mut r, mut p) = racing(2, r#","laps":1"#);
        p[1] = None; // joined the grid, then left
        drive(&mut r, &mut p, 0, &[Vec2::new(0.0, -34.0), CORNERS[1], CORNERS[2], CORNERS[3], CORNERS[0], Vec2::new(4.0, -34.0)], 0.4);
        assert_eq!(r.phase(), Phase::Finished);
        assert_eq!(
            r.standings().iter().map(|s| s.player).collect::<Vec<_>>(),
            vec![0, 1],
            "the one who left is listed as not finished, not dropped from the results"
        );
    }

    #[test]
    fn the_same_run_gives_the_same_checksum_and_progress_changes_it() {
        let run = || {
            let (mut race, mut pos) = racing(2, r#","laps":2"#);
            drive(&mut race, &mut pos, 0, &[Vec2::new(0.0, -34.0), CORNERS[1], CORNERS[2]], 0.4);
            drive(&mut race, &mut pos, 1, &[Vec2::new(0.0, -34.0), CORNERS[1]], 0.3);
            race
        };
        let (a, b) = (run(), run());
        assert_eq!(a.checksum(), b.checksum());
        assert_eq!(a.standings(), b.standings());
        let mut c = run();
        let pos = vec![Some(CORNERS[2]); 2];
        c.tick(&pos);
        assert_ne!(a.checksum(), c.checksum(), "a tick changes the state");
        let fresh = RaceState::new(course(""), 2);
        assert_ne!(fresh.checksum(), a.checksum());
    }
}
