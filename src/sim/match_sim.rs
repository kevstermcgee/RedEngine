//! The authoritative match world: players and physics props, ticked at a fixed rate, with **no
//! window, GPU, audio or socket** — the server runs exactly this, and tests drive it directly.
//!
//! A player's state is a pure function of the inputs the server has *processed* for them, in order:
//! each tick every player consumes one queued [`PlayerInput`] (two if their queue has backed up, to
//! catch up), and a player with nothing queued simply does not move that tick (no extrapolation).
//! That makes client-side prediction exact: replaying the inputs the server has not yet acknowledged
//! on top of the server's state reproduces what the server will compute.
//!
//! Players do not collide with each other; they shove loose props (kinematic cylinders in
//! [`PropWorld`]), and the props are simulated here, so a prop moved by one player is seen moved by
//! every other.

use crate::physics::PropWorld;
use crate::player::Character;
use crate::schema::Scene;
use crate::sim::player::{step_player, PlayerInput, PlayerState};
use crate::sim::spawns::Spawn;
use crate::viewer::{collect_box_colliders_except, collect_ground_candidates_except, Collider2D, GroundCandidates};
use std::collections::VecDeque;

/// Most players in one match.
pub const MAX_PLAYERS: usize = 8;
/// A player's input queue is trimmed to this many entries (older ones are dropped).
const INPUT_QUEUE_CAP: usize = 8;
/// With more than this many inputs queued, a player processes two per tick to catch up.
const INPUT_QUEUE_TARGET: usize = 3;

/// A connected player as the server sees them.
#[derive(Debug, Clone)]
pub struct ServerPlayer {
    /// Authoritative state.
    pub state: PlayerState,
    /// Horizontal speed at the last processed input, m/s (animation).
    pub speed: f32,
    /// Crouching at the last processed input.
    pub crouching: bool,
    /// Sequence number of the newest input processed (`0` = none yet).
    pub last_processed_seq: u32,
    newest_received_seq: u32,
    queue: VecDeque<PlayerInput>,
}

/// The authoritative world. See the module docs.
pub struct MatchSim {
    props: PropWorld,
    colliders: Vec<Collider2D>,
    ground: GroundCandidates,
    spawns: Vec<Spawn>,
    next_spawn: usize,
    players: Vec<Option<ServerPlayer>>,
    tick: u64,
}

impl MatchSim {
    /// Builds the world for `scene`. `spawns` must not be empty (see `sim::spawns::parse_spawns`).
    pub fn new(scene: &Scene, spawns: Vec<Spawn>) -> Self {
        assert!(!spawns.is_empty(), "a match needs at least one spawn point");
        let props = PropWorld::new(scene, None);
        let loose = props.movable_indices();
        MatchSim {
            colliders: collect_box_colliders_except(scene, &loose),
            ground: collect_ground_candidates_except(scene, &loose),
            props,
            spawns,
            next_spawn: 0,
            players: (0..MAX_PLAYERS).map(|_| None).collect(),
            tick: 0,
        }
    }

    /// Ticks run so far.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// The prop physics world (read-only: for snapshots).
    pub fn props(&self) -> &PropWorld {
        &self.props
    }

    /// The prop physics world (to close change generations, or apply a server-side impulse).
    pub fn props_mut(&mut self) -> &mut PropWorld {
        &mut self.props
    }

    /// The static collision world players walk through (a client predicts against the same data).
    pub fn static_world(&self) -> (&[Collider2D], &GroundCandidates) {
        (&self.colliders, &self.ground)
    }

    /// The map's spawn points.
    pub fn spawns(&self) -> &[Spawn] {
        &self.spawns
    }

    /// Adds a player at the next spawn point (round robin). `None` when the match is full.
    pub fn add_player(&mut self, character: Character) -> Option<usize> {
        let s = self.spawns[self.next_spawn % self.spawns.len()].clone();
        self.next_spawn += 1;
        self.add_player_with(PlayerState::spawn(s.position[0], s.position[2], s.position[1], s.yaw_deg, character))
    }

    /// Adds a player in exactly `state` (a reconnecting player resuming where they were).
    pub fn add_player_with(&mut self, state: PlayerState) -> Option<usize> {
        let slot = self.players.iter().position(Option::is_none)?;
        self.players[slot] = Some(ServerPlayer { state, speed: 0.0, crouching: false, last_processed_seq: 0, newest_received_seq: 0, queue: VecDeque::new() });
        let body = state.character.body();
        self.props.set_player_slot(slot, glam::Vec3::new(state.pos.x, state.foot_y, state.pos.y), body.radius, body.body_height);
        Some(slot)
    }

    /// Removes a player, returning their last state.
    pub fn remove_player(&mut self, slot: usize) -> Option<PlayerState> {
        let p = self.players.get_mut(slot)?.take()?;
        self.props.remove_player_slot(slot);
        Some(p.state)
    }

    /// Queues an input for `slot`. Ignored if it is not newer than the newest already received (a
    /// duplicate or a late packet). Returns whether it was queued.
    pub fn push_input(&mut self, slot: usize, input: PlayerInput) -> bool {
        let Some(Some(p)) = self.players.get_mut(slot) else { return false };
        if (input.seq.wrapping_sub(p.newest_received_seq) as i32) <= 0 {
            return false;
        }
        p.newest_received_seq = input.seq;
        p.queue.push_back(input.sanitized());
        while p.queue.len() > INPUT_QUEUE_CAP {
            p.queue.pop_front();
        }
        true
    }

    /// One player, if `slot` is in use.
    pub fn player(&self, slot: usize) -> Option<&ServerPlayer> {
        self.players.get(slot)?.as_ref()
    }

    /// Every connected player as `(slot, player)`.
    pub fn players(&self) -> impl Iterator<Item = (usize, &ServerPlayer)> {
        self.players.iter().enumerate().filter_map(|(i, p)| p.as_ref().map(|p| (i, p)))
    }

    /// Number of connected players.
    pub fn player_count(&self) -> usize {
        self.players().count()
    }

    /// Advances the whole match one tick: process queued inputs, move the player bodies through the
    /// props, step the physics.
    pub fn tick_once(&mut self) {
        for slot in 0..self.players.len() {
            let Some(p) = self.players[slot].as_mut() else { continue };
            let budget = if p.queue.len() > INPUT_QUEUE_TARGET { 2 } else { 1 };
            for _ in 0..budget {
                let Some(input) = p.queue.pop_front() else { break };
                p.speed = step_player(&mut p.state, &input, &self.colliders, &self.ground);
                p.crouching = input.crouch;
                p.last_processed_seq = input.seq;
            }
            let body = p.state.character.body();
            self.props.set_player_slot(slot, glam::Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y), body.radius, body.body_height);
        }
        self.props.step();
        self.tick += 1;
    }

    /// A 64-bit checksum of the simulation state (players and promoted props' poses, bit-exact).
    /// Two runs that were fed the same inputs on the same platform must agree; a mismatch means they
    /// diverged. Used by tests now, and the basis for desync detection later.
    pub fn checksum(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |v: u32| {
            h ^= v as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for (slot, p) in self.players() {
            mix(slot as u32);
            for f in [p.state.pos.x, p.state.pos.y, p.state.foot_y, p.state.vy, p.state.yaw, p.state.pitch] {
                mix(f.to_bits());
            }
        }
        let e = self.props.entities();
        for slot in 0..e.len() {
            let t = e.transforms.get(slot);
            mix(self.props.prop_of_entity(slot) as u32);
            for f in t.position.to_array().into_iter().chain(t.rotation.to_array()) {
                mix(f.to_bits());
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::spawns::parse_spawns;
    use glam::Vec3;
    use std::path::Path;

    fn lab_sim(group: &str) -> MatchSim {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json");
        let scene = crate::load_scene(&path).unwrap();
        let mut spawns = parse_spawns(&std::fs::read_to_string(&path).unwrap()).unwrap();
        if !group.is_empty() {
            spawns.retain(|s| s.group == group);
        }
        MatchSim::new(&scene, spawns)
    }

    fn input(seq: u32, forward: i8, yaw_deg: f32) -> PlayerInput {
        PlayerInput { seq, forward, yaw: yaw_deg.to_radians(), ..Default::default() }
    }

    #[test]
    fn players_join_at_distinct_spawns_and_the_match_fills_up() {
        let mut sim = lab_sim("duel");
        let a = sim.add_player(Character::Human).unwrap();
        let b = sim.add_player(Character::Rat).unwrap();
        assert_ne!(a, b);
        assert_ne!(sim.player(a).unwrap().state.pos, sim.player(b).unwrap().state.pos);
        for _ in 2..MAX_PLAYERS {
            assert!(sim.add_player(Character::Human).is_some());
        }
        assert!(sim.add_player(Character::Human).is_none(), "match is full");
        sim.remove_player(a);
        assert_eq!(sim.add_player(Character::Human), Some(a), "the freed slot is reused");
    }

    #[test]
    fn a_player_moves_only_by_processed_inputs_and_does_not_move_without_them() {
        let mut sim = lab_sim("duel");
        let a = sim.add_player(Character::Human).unwrap();
        let start = sim.player(a).unwrap().state.pos;
        for _ in 0..30 {
            sim.tick_once(); // no inputs: no movement (and no extrapolation)
        }
        assert_eq!(sim.player(a).unwrap().state.pos, start);
        for k in 1..=60 {
            assert!(sim.push_input(a, input(k, 1, 90.0)));
            sim.tick_once();
        }
        let p = sim.player(a).unwrap();
        assert_eq!(p.last_processed_seq, 60);
        assert!((p.state.pos.x - (start.x + 3.2)).abs() < 0.05, "one second forward: {}", p.state.pos.x - start.x);
    }

    #[test]
    fn duplicate_and_stale_inputs_are_ignored_and_a_backlog_catches_up() {
        let mut sim = lab_sim("duel");
        let a = sim.add_player(Character::Human).unwrap();
        assert!(sim.push_input(a, input(5, 1, 90.0)));
        assert!(!sim.push_input(a, input(5, 1, 90.0)), "duplicate");
        assert!(!sim.push_input(a, input(4, 1, 90.0)), "stale");
        for k in 6..=12 {
            sim.push_input(a, input(k, 1, 90.0));
        }
        sim.tick_once();
        assert_eq!(sim.player(a).unwrap().last_processed_seq, 6, "queue over the target: two inputs in one tick (5 then 6)");
        for _ in 0..10 {
            sim.tick_once();
        }
        assert_eq!(sim.player(a).unwrap().last_processed_seq, 12);
    }

    #[test]
    fn a_hostile_client_cannot_move_faster_than_full_speed() {
        let mut sim = lab_sim("duel");
        let a = sim.add_player(Character::Human).unwrap();
        let start = sim.player(a).unwrap().state.pos;
        for k in 1..=60 {
            sim.push_input(a, PlayerInput { seq: k, forward: 100, strafe: -100, sprint: false, yaw: std::f32::consts::FRAC_PI_2, pitch: f32::NAN, ..Default::default() });
            sim.tick_once();
        }
        assert!((sim.player(a).unwrap().state.pos - start).length() <= 3.3);
    }

    #[test]
    fn walking_into_a_barrel_moves_it_authoritatively_for_everyone() {
        let mut sim = lab_sim("props");
        let a = sim.add_player(Character::Human).unwrap(); // spawn_props_a faces -Z toward domino_0
        assert_eq!(sim.props().dynamic_count(), 0, "nothing is promoted until something touches it");
        let before: Vec<Vec3> = (0..sim.props().props().len()).map(|i| sim.props().prop_pose(i).w_axis.truncate()).collect();
        for k in 1..=120 {
            sim.push_input(a, input(k, 1, 0.0)); // walk toward -Z, into the barrels at z = 4
            sim.tick_once();
        }
        assert!(sim.props().dynamic_count() >= 1, "the barrel was promoted");
        let moved = (0..before.len()).filter(|&i| (sim.props().prop_pose(i).w_axis.truncate() - before[i]).length() > 0.1).count();
        assert!(moved >= 1, "and it moved");
        let e = sim.props().entities();
        assert!(!e.is_empty() && (0..e.len()).any(|s| e.transforms.get(s).position != before[sim.props().prop_of_entity(s)]));
    }

    #[test]
    fn identical_inputs_give_identical_checksums_and_different_inputs_do_not() {
        let run = |turn: f32| {
            let mut sim = lab_sim("props");
            let a = sim.add_player(Character::Human).unwrap();
            for k in 1..=150 {
                sim.push_input(a, input(k, 1, turn));
                sim.tick_once();
            }
            sim.checksum()
        };
        assert_eq!(run(0.0), run(0.0), "deterministic on one machine");
        assert_ne!(run(0.0), run(20.0));
    }

    #[test]
    fn removing_a_player_removes_their_physics_body_too() {
        let mut sim = lab_sim("duel");
        let a = sim.add_player(Character::Human).unwrap();
        let b = sim.add_player(Character::Human).unwrap();
        let bodies = sim.props().body_count();
        sim.remove_player(a);
        assert_eq!(sim.props().body_count(), bodies - 1);
        assert_eq!(sim.player_count(), 1);
        assert!(sim.player(b).is_some() && sim.player(a).is_none());
    }
}
