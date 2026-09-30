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

use crate::collide::{collect_box_colliders_grouped_except, collect_ground_candidates_grouped_except, Collider2D, GroundCandidates};
use crate::hit::{collect_hit_shapes_where, HitShape};
use crate::physics::PropWorld;
use crate::player::Character;
use crate::schema::Scene;
use crate::sim::interact::Combat;
use crate::sim::items::{roll_item, Hazard, HazardPool, ItemBox, ItemBoxes, ACORN_SPEED_BONUS};
use crate::sim::kart::{kart_bump, step_kart_ex, Driver, Item, KartEvents, KartState, Surface};
use crate::sim::player::{step_player_tuned, PlayerInput, PlayerState};
use crate::sim::race::RaceState;
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
/// How many ticks of every player's position are remembered, so a shot can be judged against the world its shooter saw (lag compensation,
/// ADR 0053). A shooter is never rewound further than `HISTORY_TICKS - 1` (about a quarter of a second).
pub const HISTORY_TICKS: usize = 16;

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
    /// How many ticks behind the present this player's view of the others is (their interpolation delay plus latency): their shots and
    /// swings are judged against where the others were then. `0` = the present (bots, a local player).
    pub view_lag: u8,
    newest_received_seq: u32,
    queue: VecDeque<PlayerInput>,
}

/// The authoritative world. See the module docs.
pub struct MatchSim {
    pub(super) props: PropWorld,
    colliders: Vec<Collider2D>,
    ground: GroundCandidates,
    collider_groups: Vec<Vec<Collider2D>>,
    ground_groups: Vec<GroundCandidates>,
    collision_object_ids: Vec<String>,
    /// Exact shapes of the fixed world, for bat swings and bullets.
    pub(super) hit_shapes: Vec<HitShape>,
    /// The scene's weapon numbers.
    pub(super) weapons: crate::weapons::WeaponConfig,
    /// Respawn delay, spawn policy, spawn protection and regeneration (the scene's `combat` block).
    pub(super) combat_cfg: crate::sim::combat_cfg::CombatConfig,
    /// The brain of every bot slot (`None` for humans and empty slots): see [`crate::sim::ai`].
    pub(super) bots: Vec<Option<Box<crate::sim::ai::Brain>>>,
    /// The scene's `bots` block (who the server should fill empty slots with).
    bots_cfg: crate::sim::ai::BotsConfig,
    /// The scene's waypoint graph, if any.
    nav: Option<crate::sim::ai::nav::Nav>,
    pub(super) player_tuning: crate::player::PlayerTuning,
    pub(super) jump_pads: Vec<crate::player::JumpPad>,
    pub(super) spawns: Vec<Spawn>,
    pub(super) next_spawn: usize,
    pub(super) players: Vec<Option<ServerPlayer>>,
    /// Where every player stood at the end of each of the last [`HISTORY_TICKS`] ticks (oldest first): `(x/z, foot y)`.
    history: VecDeque<[Option<(Vec2, f32)>; MAX_PLAYERS]>,
    pub(super) tick: u64,
    /// The scene's game rules, running (see `sim::rules`).
    pub(super) rules: RulesEngine,
    /// Every loose prop as the rules see it this tick, rebuilt in place (only when a rule looks at props).
    prop_views: Vec<crate::sim::rules_run::RuleProp>,
    /// Top-level object id to index, to find the prop an `impulse` rule names.
    object_index: HashMap<String, usize>,
    /// Any-depth object id to the shared compact dictionary used by network rule presentation.
    rule_object_index: HashMap<String, u16>,
    recorder: Option<Trace>,
    events_out: Vec<GameEvent>,
    /// The kart race (the scene's `race` block), if this match is one: every player then drives a kart (`sim::kart`) and progress is tracked here.
    pub(super) race: Option<RaceState>,
    /// Every slot's kart memory (boost, drift, spin-out); untouched outside a race.
    pub(super) karts: Vec<KartState>,
    /// Every slot's driver, which decides its kart's numbers.
    pub(super) drivers: Vec<Driver>,
    /// Acorns in flight and planks on the track (race matches).
    pub(super) hazards: HazardPool,
    /// The track's item boxes and their respawn timers (race matches).
    pub(super) item_boxes: ItemBoxes,
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
        let collider_groups = collect_box_colliders_grouped_except(scene, &loose);
        let ground_groups = collect_ground_candidates_grouped_except(scene, &loose);
        let colliders = collider_groups.iter().flatten().copied().collect();
        let mut ground = GroundCandidates::default();
        for group in &ground_groups {
            ground.append(group);
        }
        let object_index: HashMap<String, usize> = scene.objects.iter().enumerate().map(|(i, o)| (o.id.clone(), i)).collect();
        let mut rules = RulesEngine::new(scene.rules.clone()).with_wrap(scene.player.expanse.wrap);
        rules.bind_props(|id| object_index.get(id).and_then(|i| props.prop_of_object(*i)));
        Ok(MatchSim {
            colliders,
            ground,
            collider_groups,
            ground_groups,
            collision_object_ids: scene.objects.iter().map(|object| object.id.clone()).collect(),
            hit_shapes: collect_hit_shapes_where(scene, |i| !loose.contains(&i)),
            weapons: scene.weapons,
            combat_cfg: scene.combat,
            bots: (0..MAX_PLAYERS).map(|_| None).collect(),
            bots_cfg: scene.bots.clone(),
            nav: scene.nav.clone(),
            player_tuning: scene.player,
            jump_pads: scene.jump_pads.clone(),
            props,
            spawns,
            next_spawn: 0,
            players: (0..MAX_PLAYERS).map(|_| None).collect(),
            history: VecDeque::with_capacity(HISTORY_TICKS + 1),
            tick: 0,
            rules,
            prop_views: Vec::new(),
            object_index,
            rule_object_index: crate::schema::object_ids(&scene.objects)
                .into_iter()
                .take(u16::MAX as usize + 1)
                .enumerate()
                .map(|(i, id)| (id, i as u16))
                .collect(),
            recorder: None,
            events_out: Vec::new(),
            race: scene.race.clone().map(|course| RaceState::new(course, MAX_PLAYERS)),
            karts: vec![KartState::default(); MAX_PLAYERS],
            drivers: (0..MAX_PLAYERS).map(|slot| Driver::ALL[slot % Driver::ALL.len()]).collect(),
            hazards: HazardPool::default(),
            item_boxes: match &scene.race {
                Some(course) => ItemBoxes::new(
                    course.item_boxes.iter().map(|(_, min, max)| ItemBox { min: *min, max: *max }).collect(),
                    (course.item_respawn_secs * crate::sim::clock::TICK_RATE_HZ as f32).round() as u32,
                ),
                None => ItemBoxes::new(Vec::new(), 0),
            },
        })
    }

    /// The race, if this match is one: its phase, countdown, laps and standings.
    pub fn race(&self) -> Option<&RaceState> {
        self.race.as_ref()
    }

    /// The Acorns in flight and planks on the track.
    pub fn hazards(&self) -> &HazardPool {
        &self.hazards
    }

    /// The track's item boxes.
    pub fn item_boxes(&self) -> &ItemBoxes {
        &self.item_boxes
    }

    /// A slot's kart memory (boost, drift, spin-out), if it has a player.
    pub fn kart(&self, slot: usize) -> Option<&KartState> {
        self.players.get(slot)?.as_ref().map(|_| &self.karts[slot])
    }

    /// The driver a slot drives as (the default is one animal per slot, in order; the lobby's character choice replaces it).
    pub fn driver(&self, slot: usize) -> Option<Driver> {
        self.drivers.get(slot).copied()
    }

    /// Chooses a slot's driver. Only meaningful before the light goes green.
    pub fn set_driver(&mut self, slot: usize, driver: Driver) -> bool {
        match self.drivers.get_mut(slot) {
            Some(d) => {
                *d = driver;
                true
            }
            None => false,
        }
    }

    /// Puts `item` in a slot's hand (practice, tests, a scripted start). `false` if there is no player in the slot.
    pub fn give_item(&mut self, slot: usize, item: Item) -> bool {
        if self.players.get(slot).is_some_and(Option::is_some) {
            self.karts[slot].item = item;
            true
        } else {
            false
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

    /// A map-authored single-character policy, enforced by the authoritative server.
    pub fn forced_character(&self) -> Option<Character> {
        self.player_tuning.character
    }

    /// The map's spawn points.
    pub fn spawns(&self) -> &[Spawn] {
        &self.spawns
    }

    /// The movement profile every player of this match moves with: the scene's tuning and its jump pads (tools replay routes with it).
    pub fn movement(&self) -> (crate::player::PlayerTuning, &[crate::player::JumpPad]) {
        (self.player_tuning, &self.jump_pads)
    }

    /// The scene's waypoint graph, if it has one.
    pub fn nav(&self) -> Option<&crate::sim::ai::nav::Nav> {
        self.nav.as_ref()
    }

    /// The scene's `bots` block.
    pub fn bots_config(&self) -> &crate::sim::ai::BotsConfig {
        &self.bots_cfg
    }

    /// Adds a player at the next spawn point (round robin). `None` when the match is full.
    pub fn add_player(&mut self, character: Character) -> Option<usize> {
        let s = self.pick_spawn(usize::MAX);
        self.add_player_with(PlayerState::spawn(s.position[0], s.position[2], s.position[1], s.yaw_deg, character))
    }

    /// Adds a player in exactly `state` (a reconnecting player resuming where they were), in the first free slot.
    pub fn add_player_with(&mut self, state: PlayerState) -> Option<usize> {
        let slot = self.players.iter().position(Option::is_none)?;
        self.add_player_at(slot, state).then_some(slot)
    }

    /// Adds a player at the next spawn point (round robin) in a chosen `slot`: how a lobby keeps every player's id the same from the
    /// lobby into the round. `false` when the slot is taken or out of range.
    pub fn add_player_in_slot(&mut self, slot: usize, character: Character) -> bool {
        if self.players.get(slot).is_none_or(Option::is_some) {
            return false;
        }
        let s = self.pick_spawn(slot);
        self.add_player_at(slot, PlayerState::spawn(s.position[0], s.position[2], s.position[1], s.yaw_deg, character))
    }

    /// Adds a player in exactly `state` in exactly `slot`. `false` when the slot is taken or out of range.
    pub fn add_player_at(&mut self, slot: usize, state: PlayerState) -> bool {
        if self.players.get(slot).is_none_or(Option::is_some) {
            return false;
        }
        self.players[slot] = Some(ServerPlayer {
            state,
            speed: 0.0,
            crouching: false,
            last_processed_seq: 0,
            combat: {
                let mut combat = Combat::new(&self.weapons);
                if self.combat_cfg.protect_ticks > 0 {
                    combat.protected_until = self.tick + self.combat_cfg.protect_ticks;
                }
                combat
            },
            view_lag: 0,
            newest_received_seq: 0,
            queue: VecDeque::new(),
        });
        self.karts[slot] = KartState::default();
        let body = state.character.body();
        self.props.set_player_slot(slot, glam::Vec3::new(state.pos.x, state.foot_y, state.pos.y), body.radius, body.body_height);
        if let Some(r) = &mut self.recorder {
            let character = crate::net::protocol::character_to_wire(state.character);
            let state = [state.pos.x, state.pos.y, state.foot_y, state.vy, state.yaw, state.pitch, state.velocity.x, state.velocity.y].map(f32::to_bits);
            r.entries.push(Entry::Join { tick: self.tick, slot, character, state });
        }
        true
    }

    /// Removes a player, returning their last state.
    pub fn remove_player(&mut self, slot: usize) -> Option<PlayerState> {
        let p = self.players.get_mut(slot)?.take()?;
        self.bots[slot] = None;
        self.props.remove_player_slot(slot);
        if let Some(r) = &mut self.recorder {
            r.entries.push(Entry::Leave { tick: self.tick, slot });
        }
        Some(p.state)
    }

    /// Sets how many ticks behind the present `slot` sees the others (clamped to what is remembered): the server derives it from the
    /// client's round-trip time and interpolation delay. Recorded in the trace when it changes, so a replay judges shots the same way.
    pub fn set_view_lag(&mut self, slot: usize, ticks: u8) {
        let ticks = ticks.min((HISTORY_TICKS - 1) as u8);
        let Some(Some(p)) = self.players.get_mut(slot) else { return };
        if p.view_lag == ticks {
            return;
        }
        p.view_lag = ticks;
        if let Some(r) = &mut self.recorder {
            r.entries.push(Entry::ViewLag { tick: self.tick, slot, lag: ticks });
        }
    }

    /// Where `slot` stood `lag` ticks ago (`None` for the present, for a `lag` beyond what is remembered, or if nobody was in the slot then).
    pub(super) fn rewound(&self, slot: usize, lag: usize) -> Option<(Vec2, f32)> {
        if lag == 0 {
            return None;
        }
        let frame = self.history.get(self.history.len().checked_sub(lag)?)?;
        *frame.get(slot)?
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
        self.run_bots();
        // In a race every player is a kart driver: no weapons, no carrying, and the countdown holds the karts on the grid.
        let racing = self.race.as_ref().map(RaceState::can_drive);
        let mut kart_events = [KartEvents::default(); MAX_PLAYERS];
        for slot in 0..self.players.len() {
            if racing.is_none() {
                self.combat_tick(slot);
            }
            let Some(p) = self.players[slot].as_mut() else { continue };
            let dead = p.combat.is_dead();
            let budget = if p.queue.len() > INPUT_QUEUE_TARGET { 2 } else { 1 };
            let mut inputs = [None; 2];
            for slot_in in inputs.iter_mut().take(budget) {
                *slot_in = p.queue.pop_front();
            }
            for input in inputs.into_iter().flatten() {
                let Some(p) = self.players[slot].as_mut() else { break };
                if let Some(green) = racing {
                    let input = if green { input } else { PlayerInput { seq: input.seq, ..Default::default() } };
                    let spec = self.drivers[slot].spec();
                    let (speed, events) = step_kart_ex(&mut p.state, &mut self.karts[slot], &input, &spec, Surface::Road, &self.colliders, &self.ground);
                    p.speed = speed;
                    kart_events[slot].throw_acorn |= events.throw_acorn;
                    kart_events[slot].lay_plank |= events.lay_plank;
                    p.crouching = false;
                    p.last_processed_seq = input.seq;
                    continue;
                }
                if !dead {
                    p.speed = step_player_tuned(&mut p.state, &input, &self.colliders, &self.ground, self.player_tuning, &self.jump_pads);
                    p.crouching = input.crouch;
                }
                p.last_processed_seq = input.seq;
                self.handle_actions(slot, &input);
            }
            if racing.is_none() {
                self.update_held(slot);
            }
            let Some(p) = self.players[slot].as_ref() else { continue };
            let body = p.state.character.body();
            self.props.set_player_slot(slot, glam::Vec3::new(p.state.pos.x, p.state.foot_y, p.state.pos.y), body.radius, body.body_height);
        }
        if self.race.is_some() {
            self.kart_bumps_and_race(&kart_events);
        }
        self.props.step();
        let mut frame = [None; MAX_PLAYERS];
        for (slot, p) in self.players.iter().enumerate().take(MAX_PLAYERS) {
            frame[slot] = p.as_ref().map(|p| (p.state.pos, p.state.foot_y));
        }
        if self.history.len() >= HISTORY_TICKS {
            self.history.pop_front();
        }
        self.history.push_back(frame);
        self.tick += 1;
        self.run_rules();
        self.record_checkpoint();
    }

    /// Karts touching each other trade momentum (a Bear spins the other out), then the race counts gates from where everyone ended up.
    fn kart_bumps_and_race(&mut self, events: &[KartEvents; MAX_PLAYERS]) {
        for a in 0..self.players.len() {
            for b in a + 1..self.players.len() {
                let (Some(pa), Some(pb)) = (&self.players[a], &self.players[b]) else { continue };
                let bump = kart_bump(pa.state.pos, pa.state.velocity, &self.drivers[a].spec(), pb.state.pos, pb.state.velocity, &self.drivers[b].spec());
                let Some(bump) = bump else { continue };
                for (slot, push, dv, spin) in [(a, bump.push_a, bump.dv_a, bump.spin_a), (b, bump.push_b, bump.dv_b, bump.spin_b)] {
                    if let Some(p) = self.players[slot].as_mut() {
                        p.state.pos += push;
                        p.state.velocity += dv;
                    }
                    if spin > 0 {
                        self.karts[slot].spin_out(spin);
                    }
                }
            }
        }
        let positions: Vec<Option<Vec2>> = self.players.iter().map(|p| p.as_ref().map(|p| p.state.pos)).collect();
        if let Some(race) = self.race.as_mut() {
            race.tick(&positions);
        }
        self.hazards_and_boxes(events, &positions);
    }

    /// Throws what the karts threw, lays what the Beaver laid, lets the hazards hit, and hands out the item boxes.
    fn hazards_and_boxes(&mut self, events: &[KartEvents; MAX_PLAYERS], positions: &[Option<Vec2>]) {
        for (slot, ev) in events.iter().enumerate() {
            let Some(p) = self.players.get(slot).and_then(Option::as_ref) else { continue };
            let (sin, cos) = libm::sincosf(p.state.yaw);
            let fwd = Vec2::new(sin, -cos);
            if ev.throw_acorn {
                let speed = p.state.velocity.dot(fwd).max(0.0) + ACORN_SPEED_BONUS;
                self.hazards.spawn(Hazard::acorn(slot as u8, p.state.pos + fwd * 1.2, fwd * speed));
            }
            if ev.lay_plank {
                self.hazards.spawn(Hazard::plank(slot as u8, p.state.pos - fwd * 1.8));
            }
        }
        let karts = &mut self.karts;
        self.hazards.step(positions, &self.colliders, |slot, spin| {
            karts[slot].spin_out(spin);
        });
        // Boxes: only a kart with a free hand can take one; the roll knows the taker's place.
        let free: Vec<bool> = (0..positions.len()).map(|slot| self.karts[slot].item == Item::None).collect();
        let mut grants: [Option<(usize, usize)>; MAX_PLAYERS] = [None; MAX_PLAYERS];
        let mut n = 0;
        self.item_boxes.step(
            positions,
            |slot| free[slot],
            |slot, b| {
                if n < MAX_PLAYERS {
                    grants[n] = Some((slot, b));
                    n += 1;
                }
            },
        );
        let players = self.players.iter().flatten().count();
        for (slot, b) in grants.into_iter().flatten() {
            let place = self.race.as_ref().and_then(|r| r.place_of(slot)).unwrap_or(1);
            self.karts[slot].item = roll_item(self.tick, slot, b, place, players);
        }
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
        if self.rules.needs_props() {
            self.prop_views.clear();
            for k in 0..self.props.props().len() {
                let view = crate::sim::rules_run::RuleProp::of(&self.props, k);
                self.prop_views.push(view);
            }
        }
        let collision_before: Vec<String> = self.rules.collision_disabled().map(str::to_string).collect();
        let effects = self.rules.step_props(self.tick, &views, &self.prop_views);
        for effect in effects {
            match effect {
                Effect::Teleport { slot, target } => self.teleport(slot, &target),
                Effect::Impulse { object, dir, speed } => {
                    let Some(prop) = self.object_index.get(&object).and_then(|i| self.props.prop_of_object(*i)) else { continue };
                    let at = self.props.prop_pose(prop).w_axis.truncate();
                    let impulse = self.props.mass(prop) * speed;
                    self.shove(prop, dir.normalize_or_zero(), at, impulse);
                }
                Effect::Reset { prop } => {
                    self.props.reset_prop(prop);
                }
                Effect::Place { prop, at } => {
                    self.props.place_prop(prop, at);
                }
            }
        }
        if !self.rules.collision_disabled().eq(collision_before.iter().map(String::as_str)) {
            self.rebuild_static_world();
        }
        let new = self.rules.take_new_events();
        if let Some(r) = &mut self.recorder {
            r.events.extend(new.iter().map(|e| crate::sim::trace::TraceEvent { tick: e.tick, rule: e.rule.clone(), name: e.name.clone(), slot: e.slot }));
        }
        if self.events_out.len() < 256 {
            self.events_out.extend(new);
        }
    }

    fn rebuild_static_world(&mut self) {
        self.colliders.clear();
        self.ground = GroundCandidates::default();
        for (i, id) in self.collision_object_ids.iter().enumerate() {
            if self.rules.collision_disabled().any(|disabled| disabled == id) {
                continue;
            }
            if let Some(group) = self.collider_groups.get(i) {
                self.colliders.extend_from_slice(group);
            }
            if let Some(group) = self.ground_groups.get(i) {
                self.ground.append(group);
            }
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
        p.state.velocity = glam::Vec2::ZERO;
        let body = p.state.character.body();
        self.props.set_player_slot(slot, to, body.radius, body.body_height);
    }

    /// An **external** push of prop `prop` along `dir` at `point` with impulse `magnitude` (N·s): something outside the
    /// simulation decided it (the server's demo kick, an operator command), so it is recorded like an input and a replay
    /// applies it at the same tick. A push the simulation derives itself from recorded inputs and rules (a bat strike, a
    /// bullet, a rule `impulse`) must use [`shove`](Self::shove) instead: a replay re-derives those, and recording them
    /// too would apply them twice (ADR 2026-09-29-replay-applies-each-shove-once).
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
        self.shove(prop, dir, point, magnitude);
    }

    /// Shoves prop `prop` along `dir` at `point` with impulse `magnitude` (N·s) **without recording it**: the push is an
    /// output of this tick (a strike resolved from an input, a rule action), which a replay reproduces by re-running the
    /// same inputs and rules. Recording it as well would double it; see [`apply_impulse`](Self::apply_impulse).
    pub(crate) fn shove(&mut self, prop: usize, dir: Vec3, point: Vec3, magnitude: f32) {
        self.props.strike_impulse(prop, dir, point, magnitude);
    }

    /// The scene's rules state (variables, hidden objects, outcome, event history).
    pub fn rules(&self) -> &RulesEngine {
        &self.rules
    }

    /// The physics prop of top-level object `id`, if it is a loose prop.
    pub fn prop_named(&self, id: &str) -> Option<usize> {
        self.object_index.get(id).and_then(|i| self.props.prop_of_object(*i))
    }

    /// Loose prop `prop` as the rules (and `sim` expectations) see it right now.
    pub fn prop_view(&self, prop: usize) -> crate::sim::rules_run::RuleProp {
        crate::sim::rules_run::RuleProp::of(&self.props, prop)
    }

    /// Shared presentation-dictionary index for an authored object id at any nesting depth.
    pub fn rule_object_index(&self, id: &str) -> Option<u16> {
        self.rule_object_index.get(id).copied()
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
                state: [s.pos.x, s.pos.y, s.foot_y, s.vy, s.yaw, s.pitch, s.velocity.x, s.velocity.y].map(f32::to_bits),
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
    fn a_rule_can_disable_static_collision_authoritatively() {
        let scene = crate::schema::parse_scene(
            r#"{
                "camera":{"position":[-2,1.7,0]},
                "rules":[{"id":"open","when":{"start":true},"do":[{"collision":["gate",false]}]}],
                "objects":[
                    {"id":"floor","type":"plane","size":[10,10]},
                    {"id":"gate","type":"box","size":[0.2,2,4],"position":[0,1,0]}
                ]
            }"#,
        )
        .unwrap();
        let spawn = Spawn { id: "start".into(), position: [-2.0, 0.0, 0.0], yaw_deg: 90.0, group: String::new() };
        let mut sim = MatchSim::new(&scene, vec![spawn]);
        assert!(!sim.static_world().0.is_empty());
        sim.add_player(Character::Human).unwrap();
        sim.tick_once();
        assert!(sim.rules().collision_disabled().any(|id| id == "gate"));
        assert!(sim.static_world().0.is_empty(), "the gate collider should be removed after the start rule");
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
