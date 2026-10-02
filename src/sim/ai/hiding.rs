//! A hider bot for hide-and-seek: find a spot far from where everyone starts, go there, then hold still.
//!
//! Unlike the fighters in `sim::ai`, a [`HideBrain`] never perceives, aims or fights — it is deliberately the
//! simplest brain in this module. Each tick, before inputs are consumed, it looks at the world exactly as a human
//! could and queues one [`PlayerInput`]: walk toward its chosen hiding spot (reusing [`super::route::Route`]
//! exactly as the combat brain and [`super::kart::KartBrain`] do — the nav graph does not care what kind of bot is
//! walking it), then stop and stay put once there. A seeker needs no new brain at all: the ordinary combat
//! [`super::Brain`], team-assigned and armed with only the bat, already "hunts the nearest non-teammate when none
//! is in sight, melee when in reach" — which is search-and-tag once a bot is on the opposing team.

use super::nav::Nav;
use super::route::Route;
use crate::sim::match_sim::MatchSim;
use crate::sim::player::PlayerInput;
use glam::{Vec2, Vec3};

/// One hider's mind.
#[derive(Debug, Clone)]
pub struct HideBrain {
    name: String,
    level: f32,
    seed: u64,
    route: Route,
    /// Chosen on the first tick that sees a nav graph; `None` forever on a map with no `nav` block (the bot just stands where it spawned).
    goal: Option<Vec3>,
    /// Close enough to the goal to stop walking and never start again.
    settled: bool,
}

/// Within this of the goal, a hider stops walking (metres).
const ARRIVED_M: f32 = 1.0;
/// How many of the nav graph's farthest-from-every-spawn nodes count as "good enough to hide at" — more than one
/// candidate so two hiders in the same match do not walk to the same spot.
const CANDIDATE_SPOTS: usize = 6;

impl HideBrain {
    /// A brain called `name`; `seed` (the slot and the tick it joined) spreads multiple hiders across different spots.
    pub fn new(name: impl Into<String>, level: f32, seed: u64) -> HideBrain {
        HideBrain { name: name.into(), level: level.clamp(0.0, 1.0), seed, route: Route::default(), goal: None, settled: false }
    }

    /// The bot's name on the scoreboard.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its skill, `0..=1` (kept for API symmetry with the other brains; a hider's behaviour does not vary by skill yet).
    pub fn level(&self) -> f32 {
        self.level
    }

    /// The hiding spot once chosen, for diagnostics.
    pub fn goal(&self) -> Option<Vec3> {
        self.goal
    }

    /// One of the nav graph's nodes farthest from every spawn point, picked by `seed` among the top
    /// [`CANDIDATE_SPOTS`] so hiders spread out rather than converging on the single farthest node.
    fn pick_hiding_spot(&self, nav: &Nav, spawns: &[Vec2]) -> Option<Vec3> {
        if nav.nodes.is_empty() {
            return None;
        }
        let mut by_min_spawn_dist: Vec<(f32, Vec3)> = nav
            .nodes
            .iter()
            .map(|n| {
                let flat = Vec2::new(n.pos.x, n.pos.z);
                let min_dist = spawns.iter().map(|s| (*s - flat).length()).fold(f32::INFINITY, f32::min);
                (min_dist, n.pos)
            })
            .collect();
        by_min_spawn_dist.sort_by(|a, b| b.0.total_cmp(&a.0));
        let pool = by_min_spawn_dist.len().min(CANDIDATE_SPOTS);
        Some(by_min_spawn_dist[self.seed as usize % pool].1)
    }

    /// What the bot does this tick: `think` is called once per tick, before inputs are consumed, the same contract every brain in this module follows.
    pub(crate) fn think(&mut self, sim: &MatchSim, slot: usize) -> PlayerInput {
        let Some(me) = sim.player(slot) else { return PlayerInput::default() };
        if me.combat.is_dead() {
            return PlayerInput::default();
        }
        let pos = me.state.pos;
        if self.goal.is_none() {
            let spawns: Vec<Vec2> = sim.spawns().iter().map(|s| Vec2::new(s.position[0], s.position[2])).collect();
            self.goal = sim.nav().and_then(|nav| self.pick_hiding_spot(nav, &spawns));
        }
        let mut input = PlayerInput { analog: true, ..Default::default() };
        let Some(goal) = self.goal else { return input };
        let to = Vec2::new(goal.x - pos.x, goal.z - pos.y);
        if self.settled || to.length() < ARRIVED_M {
            self.settled = true;
            return input;
        }
        let (_, ground) = sim.static_world();
        let floor = crate::collide::ground_height_at(ground, pos, me.state.foot_y);
        let grounded = me.state.vy <= 0.0 && (me.state.foot_y - floor).abs() < 0.05;
        let steer = sim.nav().and_then(|nav| self.route.steer(nav, &me.state, grounded, goal, sim.tick()));
        let want = steer.map_or_else(|| to.normalize_or_zero(), |s| s.dir);
        if want.length_squared() > 0.001 {
            // Face the way it is walking: a hider has no reason to look anywhere else.
            input.yaw = want.x.atan2(-want.y);
            input.forward = 127;
            input.sprint = true;
        }
        if me.state.vy.abs() <= 0.5 {
            input.jump = steer.is_some_and(|s| s.jump);
        }
        input
    }
}
