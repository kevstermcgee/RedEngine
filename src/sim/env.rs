//! A conventional way to *play* a match: `reset(seed)`, `observe(agent)`, `step(actions)`, `terminated()`, `debug_state()`, in the shape of PettingZoo's
//! `ParallelEnv` (every agent acts at once, observations are per agent, a finished episode is either terminated by the game or truncated by a tick limit, resets are
//! seeded). It drives the real [`MatchSim`] (the same `push_input` + `tick_once` the server and `sim` scenarios use), so there is no second copy of any rule: a bot, a
//! regression test or an AI agent plays exactly the game a person would be in.
//!
//! **What an agent may know.** An [`Observation`] is what a player could see: its own body, the other players it has line of sight to inside its field of view
//! (the hitscan ray, [`MatchSim::probe`], from its eye, so walls, furniture, loose props and other bodies block it), and the rule variables the HUD shows
//! (the names not starting with `_`). The server's own per-client filter (`net::visible_players`) is deliberately *not* used: it is a bandwidth filter by room
//! relevance, so a neighbouring room counts as "relevant" and a map without rooms sends everyone. Everything else (where every player is, `_` variables, the game
//! events, the checksum) is **privileged** and only in [`Env::debug_state`], which a test or a debugger may read and an agent must not.
//!
//! Not here on purpose: rewards (the rules language has none; judge an episode by [`Env::terminated`] and the public variables) and any learning machinery.

use super::match_sim::MatchSim;
use super::player::{PlayerInput, PlayerState};
use super::spawns::Spawn;
use crate::player::Character;
use crate::schema::Scene;
use glam::Vec3;

/// How far an agent can see another player, metres.
pub const DEFAULT_SIGHT_RANGE_M: f32 = 40.0;
/// The horizontal field of view, degrees: a player does not see behind their head.
pub const DEFAULT_FOV_DEG: f32 = 110.0;

/// One participant: the id the caller uses, which body it wears, and where it starts (`None` = a spawn chosen by the seed).
#[derive(Debug, Clone)]
pub struct AgentSpec {
    /// The caller's name for the agent (unique).
    pub id: String,
    /// The body.
    pub character: Character,
    /// A spawn point id of the scene, or `None` to be assigned one by the seeded shuffle of the scene's spawns.
    pub spawn: Option<String>,
}

/// What an agent can see of others.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sight {
    /// Farthest visible distance, metres.
    pub range_m: f32,
    /// Horizontal field of view, degrees.
    pub fov_deg: f32,
}

impl Default for Sight {
    fn default() -> Self {
        Sight { range_m: DEFAULT_SIGHT_RANGE_M, fov_deg: DEFAULT_FOV_DEG }
    }
}

/// Another player an agent can see right now.
#[derive(Debug, Clone, PartialEq)]
pub struct Seen {
    /// The other agent's id.
    pub agent: String,
    /// Where it stands (x, z), metres.
    pub pos: [f32; 2],
    /// Its feet's height, metres.
    pub foot_y: f32,
    /// Distance from the observer's eye to its body centre, metres.
    pub distance_m: f32,
}

/// Everything one agent is allowed to know on a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// The simulation tick.
    pub tick: u64,
    /// Whose view this is.
    pub agent: String,
    /// Own position (x, z), metres.
    pub pos: [f32; 2],
    /// Own feet height, metres.
    pub foot_y: f32,
    /// Own look direction, radians (0 = -Z, increasing clockwise seen from above).
    pub yaw: f32,
    /// Own look pitch, radians.
    pub pitch: f32,
    /// Whether it carries a prop.
    pub holding: bool,
    /// Whether it is alive.
    pub alive: bool,
    /// The other players in view.
    pub seen: Vec<Seen>,
    /// The public rule variables (the HUD's), in declaration order.
    pub vars: Vec<(String, f64)>,
}

/// What [`Env::step`] returns.
#[derive(Debug, Clone, PartialEq)]
pub struct StepResult {
    /// One observation per agent, in agent order.
    pub observations: Vec<Observation>,
    /// The game ended the episode: its outcome (`victory`, ...).
    pub terminated: Option<String>,
    /// The tick limit ended the episode, not the game.
    pub truncated: bool,
}

/// **Privileged**: the whole authoritative state, for tests and debugging. An agent must never be given this.
#[derive(Debug, Clone, PartialEq)]
pub struct DebugState {
    /// The simulation tick.
    pub tick: u64,
    /// Every agent: `(id, slot, x, z, foot_y, alive)`.
    pub players: Vec<(String, usize, f32, f32, f32, bool)>,
    /// Every rule variable, including the internal `_` ones.
    pub vars: Vec<(String, f64)>,
    /// Every game event so far: `(tick, rule, name)`.
    pub events: Vec<(u64, String, String)>,
    /// The match outcome, if it has ended.
    pub ended: Option<String>,
    /// The exact 64-bit checksum of the simulation state (two runs agree iff they played the same game).
    pub checksum: u64,
    /// The seed the episode was reset with.
    pub seed: u64,
}

/// A running episode.
pub struct Env {
    sim: MatchSim,
    agents: Vec<(String, usize)>,
    seq: Vec<u32>,
    sight: Sight,
    seed: u64,
    max_ticks: u64,
}

/// A small deterministic generator (xorshift64*), so a seed means the same thing on every platform and needs no crate.
fn next_u64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

impl Env {
    /// Starts an episode: the scene's match with `agents` joined, spawns assigned (named ones as asked, the rest by a shuffle of the scene's spawns seeded
    /// with `seed`), and at most `max_ticks` ticks before it is truncated. Returns the episode and every agent's first observation.
    pub fn reset(scene: &Scene, spawns: &[Spawn], agents: &[AgentSpec], seed: u64, max_ticks: u64) -> Result<(Env, Vec<Observation>), String> {
        for (i, a) in agents.iter().enumerate() {
            if agents[..i].iter().any(|b| b.id == a.id) {
                return Err(format!("two agents are called `{}`", a.id));
            }
        }
        let mut sim = MatchSim::try_new(scene, spawns.to_vec())?;
        let mut order: Vec<usize> = (0..spawns.len()).collect();
        let mut rng = seed ^ 0x9E37_79B9_7F4A_7C15;
        if rng == 0 {
            rng = 1;
        }
        for i in (1..order.len()).rev() {
            order.swap(i, (next_u64(&mut rng) % (i as u64 + 1)) as usize);
        }
        let mut unnamed = 0usize;
        let mut slots = Vec::new();
        for a in agents {
            let slot = match (&a.spawn, spawns.is_empty()) {
                (Some(id), _) => {
                    let s = spawns
                        .iter()
                        .find(|s| &s.id == id)
                        .ok_or_else(|| format!("agent `{}` starts at `{id}`, which is not a spawn point of this scene", a.id))?;
                    sim.add_player_with(PlayerState::spawn(s.position[0], s.position[2], s.position[1], s.yaw_deg, a.character))
                }
                (None, false) => {
                    let s = &spawns[order[unnamed % order.len()]];
                    unnamed += 1;
                    sim.add_player_with(PlayerState::spawn(s.position[0], s.position[2], s.position[1], s.yaw_deg, a.character))
                }
                (None, true) => sim.add_player(a.character),
            };
            slots.push(slot.ok_or_else(|| format!("the match is full (at most {} players)", super::match_sim::MAX_PLAYERS))?);
        }
        let env =
            Env { sim, agents: agents.iter().map(|a| a.id.clone()).zip(slots).collect(), seq: vec![0; agents.len()], sight: Sight::default(), seed, max_ticks };
        let first = env.observe_all();
        Ok((env, first))
    }

    /// Changes what agents can see (default: [`Sight::default`]).
    pub fn set_sight(&mut self, sight: Sight) {
        self.sight = sight;
    }

    /// The agents, in order.
    pub fn agents(&self) -> Vec<&str> {
        self.agents.iter().map(|(id, _)| id.as_str()).collect()
    }

    /// The game's outcome if it has ended the episode.
    pub fn terminated(&self) -> Option<&str> {
        self.sim.rules().ended()
    }

    /// Whether the tick limit has been reached without the game ending.
    pub fn truncated(&self) -> bool {
        self.terminated().is_none() && self.sim.tick() >= self.max_ticks
    }

    /// Advances one tick. `actions` holds the agents that act (any order); an agent left out stands with its current look and no buttons. An unknown agent is an error,
    /// and so is stepping an episode that is over (`reset` again).
    pub fn step(&mut self, actions: &[(&str, PlayerInput)]) -> Result<StepResult, String> {
        if self.terminated().is_some() || self.truncated() {
            return Err("the episode is over: call reset".to_string());
        }
        for (id, _) in actions {
            if !self.agents.iter().any(|(a, _)| a == id) {
                return Err(format!("no agent called `{id}` (agents: {})", self.agents().join(", ")));
            }
        }
        for (i, (id, slot)) in self.agents.iter().enumerate() {
            let mut input = match actions.iter().find(|(a, _)| a == id) {
                Some((_, input)) => *input,
                None => match self.sim.player(*slot) {
                    Some(p) => PlayerInput { yaw: p.state.yaw, pitch: p.state.pitch, ..Default::default() },
                    None => PlayerInput::default(),
                },
            };
            self.seq[i] += 1;
            input.seq = self.seq[i];
            self.sim.push_input(*slot, input);
        }
        self.sim.tick_once();
        Ok(StepResult { observations: self.observe_all(), terminated: self.terminated().map(str::to_string), truncated: self.truncated() })
    }

    fn observe_all(&self) -> Vec<Observation> {
        self.agents.iter().filter_map(|(id, _)| self.observe(id).ok()).collect()
    }

    /// What `agent` can know right now (see the module documentation for what is deliberately left out).
    pub fn observe(&self, agent: &str) -> Result<Observation, String> {
        let (_, slot) = self.agents.iter().find(|(a, _)| a == agent).ok_or_else(|| format!("no agent called `{agent}`"))?;
        let me = self.sim.player(*slot).ok_or_else(|| format!("agent `{agent}` has left the match"))?;
        let body = me.state.character.body();
        let eye = Vec3::new(me.state.pos.x, me.state.foot_y + body.stand_eye, me.state.pos.y);
        let forward = Vec3::new(libm::sinf(me.state.yaw), 0.0, -libm::cosf(me.state.yaw));
        let half_fov = (self.sight.fov_deg * 0.5).to_radians();
        let mut seen = Vec::new();
        for (other_id, other_slot) in &self.agents {
            if other_slot == slot {
                continue;
            }
            let Some(other) = self.sim.player(*other_slot) else { continue };
            if other.combat.is_dead() {
                continue;
            }
            let ob = other.state.character.body();
            let center = Vec3::new(other.state.pos.x, other.state.foot_y + ob.body_height * 0.5, other.state.pos.y);
            let to = center - eye;
            let distance = to.length();
            if distance > self.sight.range_m || distance < 1e-4 {
                continue;
            }
            let flat = Vec3::new(to.x, 0.0, to.z);
            if flat.length() > 1e-4 && forward.dot(flat.normalize()).clamp(-1.0, 1.0).acos() > half_fov {
                continue; // behind or beside the viewer's field of view
            }
            // Line of sight: the first thing the ray from the eye toward the body meets must be that player.
            if matches!(self.sim.probe(eye, to, distance + 0.5, *slot), Some(hit) if hit.target == super::interact::RayTarget::Player(*other_slot)) {
                seen.push(Seen { agent: other_id.clone(), pos: [other.state.pos.x, other.state.pos.y], foot_y: other.state.foot_y, distance_m: distance });
            }
        }
        let vars = self.sim.rules().vars().into_iter().filter(|(n, _)| !n.starts_with('_')).map(|(n, v)| (n.to_string(), v)).collect();
        Ok(Observation {
            tick: self.sim.tick(),
            agent: agent.to_string(),
            pos: [me.state.pos.x, me.state.pos.y],
            foot_y: me.state.foot_y,
            yaw: me.state.yaw,
            pitch: me.state.pitch,
            holding: self.sim.props().held_by(*slot).is_some(),
            alive: !me.combat.is_dead(),
            seen,
            vars,
        })
    }

    /// **Privileged.** The whole authoritative state. For tests and debugging only: handing this to an agent defeats the point of [`Env::observe`].
    pub fn debug_state(&self) -> DebugState {
        DebugState {
            tick: self.sim.tick(),
            players: self
                .agents
                .iter()
                .filter_map(|(id, slot)| self.sim.player(*slot).map(|p| (id.clone(), *slot, p.state.pos.x, p.state.pos.y, p.state.foot_y, !p.combat.is_dead())))
                .collect(),
            vars: self.sim.rules().vars().into_iter().map(|(n, v)| (n.to_string(), v)).collect(),
            events: self.sim.rules().history().iter().map(|e| (e.tick, e.rule.clone(), e.name.clone())).collect(),
            ended: self.terminated().map(str::to_string),
            checksum: self.sim.checksum(),
            seed: self.seed,
        }
    }
}
