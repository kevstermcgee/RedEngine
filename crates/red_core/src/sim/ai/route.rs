//! Following a route along the scene's `nav` graph: the part of a bot that knows about floors, ramps, jump pads and ledges.
//!
//! A [`Route`] plans with A* from the node nearest the bot to the node nearest its goal, then walks the bot from node to node. What it does at
//! each node depends on how the edge into it is travelled (see [`super::nav::EdgeKind`]): `walk` just heads straight for it; `jump` jumps when
//! close; `pad` first walks onto the pad *exactly* (so it launches), then steers through the air toward the landing; `drop` steps off and steers
//! down. `red_engine2 nav` proves every edge with the same movement code, so the follower can trust the graph.
//!
//! It answers "which way now?" ([`Steer`]) or `None` when steering straight at the goal is right (no graph, no nearby node, the route is
//! finished, or the bot has been stalled and the brain's own unsticking should take over).

use super::nav::{EdgeKind, Nav};
use crate::sim::player::PlayerState;
use glam::{Vec2, Vec3};

/// How close to a node counts as having reached it, metres (the start of a pad edge is stricter: the bot must be *on* the pad).
const ARRIVE_M: f32 = 1.5;
const PAD_ARRIVE_M: f32 = 0.4;
/// Vertical slack when deciding whether a bot is at a node's floor, metres.
const LEVEL_M: f32 = 1.2;
/// Ticks without getting meaningfully closer to the current node before the route gives up (and replans).
const STALL_TICKS: u64 = 150;
/// Route again this often even when nothing seems wrong (the goal moves), ticks.
const REPLAN_TICKS: u64 = 150;

/// What a route wants the bot to do this tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Steer {
    /// Wish direction in the horizontal plane (unit).
    pub dir: Vec2,
    /// Press jump this tick.
    pub jump: bool,
    /// Whether the bot is committed to a launch or a fall (do not veer, do not stop to fight the ground).
    pub airborne_leg: bool,
}

/// The state of following a path along the graph.
#[derive(Debug, Clone, Default)]
pub struct Route {
    goal: Option<usize>,
    path: Vec<usize>,
    at: usize,
    planned: u64,
    best: f32,
    best_tick: u64,
}

fn flat(a: Vec3, b: Vec3) -> f32 {
    Vec2::new(a.x - b.x, a.z - b.z).length()
}

impl Route {
    /// Forgets the route (the bot respawned, got stuck, or has a new goal in mind).
    pub fn clear(&mut self) {
        *self = Route::default();
    }

    /// Whether there is a path being followed.
    pub fn active(&self) -> bool {
        self.at < self.path.len()
    }

    /// The node ids of the path being followed, for diagnostics.
    pub fn describe(&self, nav: &Nav) -> String {
        self.path.iter().enumerate().map(|(i, n)| format!("{}{}", if i == self.at { ">" } else { "" }, nav.nodes[*n].id)).collect::<Vec<_>>().join(" ")
    }

    /// The next steering decision toward `goal`, or `None` to steer straight at it.
    pub fn steer(&mut self, nav: &Nav, st: &PlayerState, grounded: bool, goal: Vec3, now: u64) -> Option<Steer> {
        let here = Vec3::new(st.pos.x, st.foot_y, st.pos.y);
        let goal_node = nav.nearest(goal, 2.6)?;
        let stalled = self.active() && now >= self.best_tick + STALL_TICKS && grounded;
        if self.goal != Some(goal_node) || !self.active() || now >= self.planned + REPLAN_TICKS || stalled {
            // Flying or falling: keep the plan (it started somewhere else and the landing is the point).
            if grounded || !self.active() {
                let start = nav.nearest(here, 2.6)?;
                let path = nav.path(start, goal_node)?;
                let mut at = 0;
                while at + 1 < path.len() {
                    let (a, b) = (nav.nodes[path[at]].pos, nav.nodes[path[at + 1]].pos);
                    if flat(here, b) < flat(a, b) && (here.y - b.y).abs() < LEVEL_M + 1.0 {
                        at += 1;
                    } else {
                        break;
                    }
                }
                self.path = path;
                self.goal = Some(goal_node);
                self.at = at;
                self.planned = now;
                self.best = f32::INFINITY;
                self.best_tick = now;
                if stalled {
                    // Something in the way that the graph does not know about: let the brain unstick itself first.
                    self.best_tick = now;
                }
            }
        }
        loop {
            let &n = self.path.get(self.at)?;
            let node = nav.nodes[n].pos;
            let prev_kind = if self.at > 0 { nav.edge_between(self.path[self.at - 1], n).map(|e| e.kind) } else { None };
            let next_kind = self.path.get(self.at + 1).and_then(|m| nav.edge_between(n, *m)).map(|e| e.kind);
            let d = flat(here, node);
            let level = (here.y - node.y).abs() < LEVEL_M;
            let lands = matches!(prev_kind, Some(EdgeKind::Pad | EdgeKind::Drop | EdgeKind::Jump));
            let launched = next_kind == Some(EdgeKind::Pad) && !grounded && st.vy > 3.0;
            let radius = if next_kind == Some(EdgeKind::Pad) { PAD_ARRIVE_M } else { ARRIVE_M };
            if launched || (d < radius && level && (grounded || !lands)) {
                self.at += 1;
                self.best = f32::INFINITY;
                self.best_tick = now;
                continue;
            }
            if d < self.best - 0.5 {
                self.best = d;
                self.best_tick = now;
            }
            let dir = Vec2::new(node.x - here.x, node.z - here.z).normalize_or_zero();
            let jump = prev_kind == Some(EdgeKind::Jump) && grounded && d < 3.5;
            let airborne_leg = !grounded && matches!(prev_kind, Some(EdgeKind::Pad | EdgeKind::Drop | EdgeKind::Jump));
            return Some(Steer { dir, jump, airborne_leg });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Character;
    use crate::sim::ai::nav::parse_nav;

    fn nav() -> Nav {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"nav":{"nodes":[{"id":"a","pos":[0,0,0]},{"id":"b","pos":[10,0,0]},{"id":"pad","pos":[10,0,6]},{"id":"deck","pos":[10,3.6,15]}],
            "edges":[["a","b"],["b","pad"],["pad","deck","pad"]]}}"#,
        )
        .unwrap();
        parse_nav(v.as_object().unwrap()).unwrap().unwrap()
    }

    fn at(x: f32, y: f32, z: f32, vy: f32) -> PlayerState {
        let mut s = PlayerState::spawn(x, z, y, 0.0, Character::Human);
        s.vy = vy;
        s
    }

    #[test]
    fn it_walks_node_to_node_and_then_hands_back_to_straight_steering() {
        let nav = nav();
        let mut r = Route::default();
        let goal = Vec3::new(10.0, 3.6, 15.0);
        let s = r.steer(&nav, &at(-1.0, 0.0, 0.0, 0.0), true, goal, 0).unwrap();
        assert!(s.dir.x > 0.9 && !s.jump, "heads for node b along +x: {s:?}");
        assert_eq!(r.describe(&nav), "a >b pad deck", "describe marks the current node");
        // At b (within 1.5 m) it moves on to the pad node, which is 6 m along +z.
        let s = r.steer(&nav, &at(9.2, 0.0, 0.0, 0.0), true, goal, 10).unwrap();
        assert!(s.dir.y > 0.9, "now toward the pad: {s:?}");
    }

    #[test]
    fn a_pad_start_needs_the_bot_on_the_pad_and_a_launch_moves_it_on_to_the_landing() {
        let nav = nav();
        let mut r = Route::default();
        let goal = Vec3::new(10.0, 3.6, 15.0);
        // Two metres from the pad centre is not on the pad: keep walking to it, do not turn for the deck yet.
        let s = r.steer(&nav, &at(10.0, 0.0, 4.0, 0.0), true, goal, 0).unwrap();
        assert!(s.dir.y > 0.9 && !s.airborne_leg, "{s:?}");
        // Airborne and rising: the launch happened, steer for the landing node.
        let s = r.steer(&nav, &at(10.0, 1.0, 6.1, 12.0), false, goal, 5).unwrap();
        assert!(s.dir.y > 0.9 && s.airborne_leg, "toward the deck: {s:?}");
        // Landing on the deck completes the route: straight steering takes over.
        assert!(r.steer(&nav, &at(10.0, 3.6, 14.5, 0.0), true, goal, 90).is_none());
    }

    #[test]
    fn no_nearby_node_or_an_unreachable_goal_means_steer_straight() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"nav":{"nodes":[{"id":"a","pos":[0,0,0]},{"id":"b","pos":[10,0,0]},{"id":"island","pos":[100,0,100]}],"edges":[["a","b"]]}}"#,
        )
        .unwrap();
        let nav = parse_nav(v.as_object().unwrap()).unwrap().unwrap();
        let mut r = Route::default();
        assert!(r.steer(&nav, &at(0.0, 40.0, 0.0, 0.0), true, Vec3::new(10.0, 0.0, 0.0), 0).is_none(), "no node within reach of the bot's floor");
        assert!(r.steer(&nav, &at(0.0, 0.0, 0.0, 0.0), true, Vec3::new(100.0, 0.0, 100.0), 0).is_none(), "the island cannot be reached along the graph");
        assert!(!r.active());
        assert!(r.steer(&nav, &at(0.0, 0.0, 0.0, 0.0), true, Vec3::new(10.0, 0.0, 0.0), 0).is_some(), "a reachable goal is routed");
    }

    #[test]
    fn a_stalled_bot_replans_from_where_it_is() {
        let nav = nav();
        let mut r = Route::default();
        let goal = Vec3::new(10.0, 0.0, 0.0);
        r.steer(&nav, &at(0.0, 0.0, 0.0, 0.0), true, goal, 0).unwrap();
        let path_before = r.describe(&nav);
        // Two seconds later still standing at the same place: it plans again (the route text may match, the planning tick moves on).
        let s = r.steer(&nav, &at(0.0, 0.0, 0.0, 0.0), true, goal, 200);
        assert!(s.is_some());
        assert_eq!(r.planned, 200, "re-planned at tick 200 (was: {path_before})");
    }
}
