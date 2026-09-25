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

use crate::collide::{collect_box_colliders_except, collect_ground_candidates_except, Collider2D, GroundCandidates};
use crate::hit::{collect_hit_shapes_where, HitShape};
use crate::physics::PropWorld;
use crate::player::Character;
use crate::schema::Scene;
use crate::sim::interact::Combat;
use crate::sim::player::{step_player, PlayerInput, PlayerState};
use crate::sim::rules::Target;
use crate::sim::rules_run::{Effect, GameEvent, RulePlayer, RulesEngine};
use crate::sim::spawns::Spawn;
use crate::sim::trace::{Entry, Header, Trace};
use glam::{Vec2, Vec3};
use std::collections::{HashMap, VecDeque};

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
    /// Weapon, timers, ammo, health and score (see `sim::interact`).
    pub combat: Combat,
    newest_received_seq: u32,
    queue: VecDeque<PlayerInput>,
}

/// The authoritative world. See the module docs.
pub struct MatchSim {
    pub(super) props: PropWorld,
    colliders: Vec<Collider2D>,
    ground: GroundCandidates,
    /// Exact shapes of the fixed world, for bat swings and bullets.
    pub(super) hit_shapes: Vec<HitShape>,
    /// The scene's weapon numbers.
    pub(super) weapons: crate::weapons::WeaponConfig,
    pub(super) spawns: Vec<Spawn>,
    pub(super) next_spawn: usize,
    pub(super) players: Vec<Option<ServerPlayer>>,
    pub(super) tick: u64,
    /// The scene's game rules, running (see `sim::rules`).
    pub(super) rules: RulesEngine,
    /// Top-level object id to index, to find the prop an `impulse` rule names.
    object_index: HashMap<String, usize>,
    recorder: Option<Trace>,
    events_out: Vec<GameEvent>,
}

impl MatchSim {
    /// Builds the world for `scene`. `spawns` must not be empty (see `sim::spawns::parse_spawns`).
    #[allow(clippy::panic)] // for tests and benches with known-good maps; a server calls `try_new`
    pub fn new(scene: &Scene, spawns: Vec<Spawn>) -> Self {
        match Self::try_new(scene, spawns) {
            Ok(sim) => sim,
            Err(e) => panic!("{e}"),
        }
    }

    /// Like [`MatchSim::new`] but a map with no spawn points is an `Err` naming the fix, not a panic (what a server binary calls).
    pub fn try_new(scene: &Scene, spawns: Vec<Spawn>) -> Result<Self, String> {
        if spawns.is_empty() {
            return Err("a match needs at least one spawn point: add a top-level \"spawns\" array to the scene, e.g. \"spawns\": [{\"id\":\"spawn_a\",\"position\":[0,0,0],\"yaw_deg\":0}]".to_string());
        }
        let props = PropWorld::new(scene, None);
        let loose = props.movable_indices();
        Ok(MatchSim {
            colliders: collect_box_colliders_except(scene, &loose),
            ground: collect_ground_candidates_except(scene, &loose),
            hit_shapes: collect_hit_shapes_where(scene, |i| !loose.contains(&i)),
            weapons: scene.weapons,
            props,
            spawns,
            next_spawn: 0,
            players: (0..MAX_PLAYERS).map(|_| None).collect(),
            tick: 0,
            rules: RulesEngine::new(scene.rules.clone()),
            object_index: scene.objects.iter().enumerate().map(|(i, o)| (o.id.clone(), i)).collect(),
            recorder: None,
            events_out: Vec::new(),
        })
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
        self.players[slot] = Some(ServerPlayer {
            state,
            speed: 0.0,
            crouching: false,
            last_processed_seq: 0,
            combat: Combat::new(&self.weapons),
            newest_received_seq: 0,
            queue: VecDeque::new(),
        });
        let body = state.character.body();
        self.props.set_player_slot(slot, glam::Vec3::new(state.pos.x, state.foot_y, state.pos.y), body.radius, body.body_height);
        if let Some(r) = &mut self.recorder {
            let character = crate::net::protocol::character_to_wire(state.character);
            let state = [state.pos.x, state.pos.y, state.foot_y, state.vy, state.yaw, state.pitch].map(f32::to_bits);
            r.entries.push(Entry::Join { tick: self.tick, slot, character, state });
        }
        Some(slot)
    }

    /// Removes a player, returning their last state.
    pub fn remove_player(&mut self, slot: usize) -> Option<PlayerState> {
        let p = self.players.get_mut(slot)?.take()?;
        self.props.remove_player_slot(slot);
        if let Some(r) = &mut self.recorder {
            r.entries.push(Entry::Leave { tick: self.tick, slot });
        }
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
        let input = input.sanitized();
        if let Some(r) = &mut self.recorder {
            r.entries.push(Entry::Input { tick: self.tick, slot, input });
        }
        p.queue.push_back(input);
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
    /// props, step the physics, run the scene's rules (which may teleport players or shove props).
    pub fn tick_once(&mut self) {
        for slot in 0..self.players.len() {
            self.combat_tick(slot);
            let Some(p) = self.players[slot].as_mut() else { continue };
            let dead = p.combat.is_dead();
            let budget = if p.queue.len() > INPUT_QUEUE_TARGET { 2 } else { 1 };
            let mut inputs = [None; 2];
            for slot_in in inputs.iter_mut().take(budget) {
                *slot_in = p.queue.pop_front();
            }
            for input in inputs.into_iter().flatten() {
                let Some(p) = self.players[slot].as_mut() else { break };
                if !dead {
                    p.speed = step_player(&mut p.state, &input, &self.colliders, &self.ground);
                    p.crouching = input.crouch;
                }
                p.last_processed_seq = input.seq;
                self.handle_actions(slot, &input);
            }
            self.update_held(slot);
            let Some(p) = self.players[slot].as_ref() else { continue };
            let body = p.state.character.body();
            self.props.set_player_slot(slot, glam::Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y), body.radius, body.body_height);
        }
        self.props.step();
        self.tick += 1;
        self.run_rules();
        self.record_checkpoint();
    }

    fn run_rules(&mut self) {
        if !self.rules.has_rules() {
            return;
        }
        let views: Vec<RulePlayer> = self
            .players()
            .map(|(slot, p)| {
                let body = p.state.character.body();
                RulePlayer {
                    slot,
                    pos: Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y),
                    radius: body.radius,
                    height: body.body_height,
                    character: p.state.character,
                }
            })
            .collect();
        for effect in self.rules.step(self.tick, &views) {
            match effect {
                Effect::Teleport { slot, target } => self.teleport(slot, &target),
                Effect::Impulse { object, dir, speed } => {
                    let Some(prop) = self.object_index.get(&object).and_then(|i| self.props.prop_of_object(*i)) else { continue };
                    let at = self.props.prop_pose(prop).w_axis.truncate();
                    let impulse = self.props.mass(prop) * speed;
                    self.apply_impulse(prop, dir.normalize_or_zero(), at, impulse);
                }
            }
        }
        let new = self.rules.take_new_events();
        if let Some(r) = &mut self.recorder {
            r.events.extend(new.iter().map(|e| crate::sim::trace::TraceEvent { tick: e.tick, rule: e.rule.clone(), name: e.name.clone(), slot: e.slot }));
        }
        if self.events_out.len() < 256 {
            self.events_out.extend(new);
        }
    }

    fn teleport(&mut self, slot: usize, target: &Target) {
        let to = match target {
            Target::Point(p) => *p,
            Target::Spawn(id) => match self.spawns.iter().find(|s| &s.id == id) {
                Some(s) => Vec3::from(s.position),
                None => return,
            },
        };
        let Some(Some(p)) = self.players.get_mut(slot) else { return };
        p.state.pos = Vec2::new(to.x, to.z);
        p.state.foot_y = to.y;
        p.state.vy = 0.0;
        let body = p.state.character.body();
        self.props.set_player_slot(slot, to, body.radius, body.body_height);
    }

    /// Shoves prop `prop` along `dir` at `point` with impulse `magnitude` (N·s). Every server-side push goes through
    /// here so a recording captures it and a replay reproduces it.
    pub fn apply_impulse(&mut self, prop: usize, dir: Vec3, point: Vec3, magnitude: f32) {
        if let Some(r) = &mut self.recorder {
            r.entries.push(Entry::Impulse {
                tick: self.tick,
                prop,
                dir: dir.to_array().map(f32::to_bits),
                at: point.to_array().map(f32::to_bits),
                impulse: magnitude.to_bits(),
            });
        }
        self.props.strike_impulse(prop, dir, point, magnitude);
    }

    /// The scene's rules state (variables, hidden objects, outcome, event history).
    pub fn rules(&self) -> &RulesEngine {
        &self.rules
    }

    /// Game events since the last call (a server logs them).
    pub fn take_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.events_out)
    }

    /// Starts recording a [`Trace`]; only possible before the first tick. Players already present are recorded as joins.
    pub fn start_recording(&mut self, header: Header) -> Result<(), String> {
        if self.tick != 0 {
            return Err("recording must start before the first tick".to_string());
        }
        let mut trace = Trace::new(header);
        for (slot, p) in self.players() {
            let s = &p.state;
            trace.entries.push(Entry::Join {
                tick: 0,
                slot,
                character: crate::net::protocol::character_to_wire(s.character),
                state: [s.pos.x, s.pos.y, s.foot_y, s.vy, s.yaw, s.pitch].map(f32::to_bits),
            });
        }
        self.recorder = Some(trace);
        Ok(())
    }

    /// Finishes recording and returns the trace (`None` if it never started).
    pub fn take_trace(&mut self) -> Option<Trace> {
        let mut trace = self.recorder.take()?;
        trace.final_tick = self.tick;
        if trace.dumps.last().is_none_or(|d| d.tick != self.tick) {
            trace.dumps.push(self.dump());
        }
        Some(trace)
    }

    fn record_checkpoint(&mut self) {
        let Some(every) = self.recorder.as_ref().map(|r| (r.header.checkpoint_every as u64, r.header.dump_every as u64)) else { return };
        let checkpoint = self.tick.is_multiple_of(every.0).then(|| self.checkpoint());
        let dump = (every.1 > 0 && self.tick.is_multiple_of(every.1)).then(|| self.dump());
        if let Some(r) = &mut self.recorder {
            r.checkpoints.extend(checkpoint);
            r.dumps.extend(dump);
        }
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
            sim.push_input(
                a,
                PlayerInput { seq: k, forward: 100, strafe: -100, sprint: false, yaw: std::f32::consts::FRAC_PI_2, pitch: f32::NAN, ..Default::default() },
            );
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
