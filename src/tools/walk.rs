//! `red_engine2 walk`: replay a walking route with the game's real per-tick physics.
//!
//! [`super::reach`] answers "can the player get there?" with a grid flood-fill; this answers
//! "does *this exact route* work?" by driving the very same `player::step_horizontal` /
//! `player::vertical_step` the live viewer runs, at the same 60 Hz, steering straight at each
//! waypoint in turn. It reports where the player actually ends up (position and foot height), so
//! a route through a door, up a staircase, or around furniture is verified rather than assumed —
//! and if the player gets stuck, where.

use super::world::MapWorld;
use crate::player::{step_horizontal, vertical_step, FIXED_DT, WALK_SPEED};
use glam::Vec2;

/// One leg of a replayed route: target, whether it was reached, and where the player ended (position and floor height).
#[derive(Debug, Clone)]
pub struct WalkStep {
    pub target: Vec2,
    pub reached: bool,
    /// Where the player is when this leg ended (reached or gave up).
    pub pos: Vec2,
    pub foot_y: f32,
    pub ticks: u32,
}

/// A finished route as JSON: `{ok, legs:[{target, reached, pos, foot_y, ticks}], stuck_on_leg?}`.
pub fn to_json(steps: &[WalkStep], waypoints: usize) -> serde_json::Value {
    let round = |v: f32| (v * 100.0).round() / 100.0;
    let ok = steps.len() == waypoints && steps.iter().all(|s| s.reached);
    let mut v = serde_json::json!({
        "ok": ok,
        "legs": steps.iter().map(|s| serde_json::json!({
            "target": [round(s.target.x), round(s.target.y)], "reached": s.reached,
            "pos": [round(s.pos.x), round(s.pos.y)], "foot_y": round(s.foot_y), "ticks": s.ticks,
        })).collect::<Vec<_>>(),
    });
    if let Some(i) = steps.iter().position(|s| !s.reached) {
        v["stuck_on_leg"] = serde_json::json!(i + 1);
    }
    v
}

const REACHED_DIST: f32 = 0.2;
/// A leg is abandoned after this long without getting meaningfully closer.
const STUCK_TICKS: u32 = 120;
const MAX_LEG_TICKS: u32 = 60 * 60;

/// Walks from `start` (feet on the floor at the spawn's ground height) through each waypoint.
/// Stops at the first leg that fails.
pub fn walk(world: &MapWorld, start: Vec2, path: &[Vec2]) -> Vec<WalkStep> {
    let mut pos = start;
    let mut foot_y = crate::collide::ground_height_at(&world.ground, start, 0.0);
    let mut vy = 0.0f32;
    let mut out = Vec::new();
    for &target in path {
        let mut best = (pos - target).length();
        let mut since_progress = 0u32;
        let mut ticks = 0u32;
        let mut reached = false;
        while ticks < MAX_LEG_TICKS {
            let to = target - pos;
            if to.length() <= REACHED_DIST {
                reached = true;
                break;
            }
            let step = to.normalize() * (WALK_SPEED * FIXED_DT).min(to.length());
            pos = step_horizontal(&world.colliders, pos, foot_y, step);
            let (y, v) = vertical_step(&world.ground, pos, foot_y, vy, false);
            foot_y = y;
            vy = v;
            ticks += 1;
            let d = (pos - target).length();
            if d < best - 0.2 {
                best = d;
                since_progress = 0;
            } else {
                since_progress += 1;
                if since_progress >= STUCK_TICKS {
                    break;
                }
            }
        }
        out.push(WalkStep { target, reached, pos, foot_y, ticks });
        if !reached {
            break;
        }
    }
    out
}

/// Formats a walk result, naming where the player got stuck.
pub fn format_walk(steps: &[WalkStep], planned: usize) -> String {
    let mut s = String::new();
    for (i, w) in steps.iter().enumerate() {
        s.push_str(&format!(
            "{:>2}. to ({:>6.2}, {:>6.2})  {}  now ({:>6.2}, {:>6.2}) at y={:.2}   {} ticks\n",
            i + 1,
            w.target.x,
            w.target.y,
            if w.reached { "reached " } else { "STUCK   " },
            w.pos.x,
            w.pos.y,
            w.foot_y,
            w.ticks
        ));
    }
    if steps.len() < planned || steps.last().is_some_and(|l| !l.reached) {
        s.push_str("route FAILED: the player could not complete it\n");
    } else {
        s.push_str("route OK\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn walking_through_a_doorway_and_up_stairs_works() {
        let w = MapWorld::from_text(
            r##"{"camera":{"position":[0,1.7,-3]},"objects":[
            {"id":"wall","type":"wall","from":[-4,0],"to":[4,0],"openings":[{"at":4.0,"width":1.0}]},
            {"id":"st","type":"stairs","position":[0,0,3],"width":1.2,"run":4.0,"rise":2.8,"steps":14},
            {"id":"deck","type":"box","size":[6,0.2,4],"position":[0,2.7,7]}]}"##,
            Path::new("t.json"),
        )
        .unwrap();
        let path = [Vec2::new(0.0, -1.0), Vec2::new(0.0, 1.0), Vec2::new(0.0, 4.0), Vec2::new(0.0, 6.5)];
        let steps = walk(&w, w.spawn, &path);
        assert!(steps.iter().all(|s| s.reached) && steps.len() == 4, "{}", format_walk(&steps, 4));
        assert!((steps[2].foot_y - 2.0).abs() < 0.3, "3/4 of the way up (z=4 of 1..5), got {}", steps[2].foot_y);
        assert!((steps[3].foot_y - 2.8).abs() < 0.05, "must end up on the deck, got {}", steps[3].foot_y);
    }

    #[test]
    fn a_solid_wall_stops_the_route() {
        let w = MapWorld::from_text(
            r##"{"camera":{"position":[0,1.7,-3]},"objects":[
            {"id":"wall","type":"wall","from":[-4,0],"to":[4,0]}]}"##,
            Path::new("t.json"),
        )
        .unwrap();
        let steps = walk(&w, w.spawn, &[Vec2::new(0.0, 3.0)]);
        assert!(!steps[0].reached, "must not pass through a wall");
        assert!(steps[0].pos.y < -0.3, "should be stopped by the wall, at {:?}", steps[0].pos);
    }
}
