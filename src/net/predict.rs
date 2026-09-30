//! Client-side prediction of the local player, and reconciliation with the server.
//!
//! The client applies its own input immediately (so movement feels instant whatever the latency)
//! using exactly the server's movement function ([`crate::sim::player::step_player`]), and remembers
//! the inputs the server has not yet acknowledged. When a snapshot arrives with the server's state
//! *after* input `ack`, the client takes that state and replays the unacknowledged inputs on top; if
//! prediction and server agreed, nothing changes, and if they did not (a prop nudged them, a wall
//! the client had wrong) the difference is smoothed away over a few ticks instead of popping.

use crate::collide::{Collider2D, GroundCandidates};
use crate::player::{JumpPad, PlayerTuning};
use crate::sim::kart::{step_kart, KartSpec, KartState, Surface};
use crate::sim::player::{step_player_tuned, PlayerInput, PlayerState};
use glam::Vec2;
use std::collections::VecDeque;

/// Corrections bigger than this (m) are snapped instead of smoothed (a teleport, a respawn).
const SNAP_DISTANCE: f32 = 1.5;
/// Fraction of the visual correction left after each tick.
const CORRECTION_DECAY: f32 = 0.8;
/// Unacknowledged inputs kept (about two seconds).
const MAX_PENDING: usize = 128;

/// What a predicted kart remembers beyond the [`PlayerState`]: its memory (boost, drift, spin-out) and its driver's numbers.
#[derive(Debug, Clone)]
struct KartMode {
    kart: KartState,
    spec: KartSpec,
    /// The race course, for the surface under the kart at each step (the server's own lookup).
    course: Option<std::sync::Arc<crate::sim::race::RaceCourse>>,
}

/// The local player's predicted state.
#[derive(Debug, Clone)]
pub struct Predictor {
    /// The predicted authoritative-equivalent state (what the server will compute).
    pub state: PlayerState,
    /// Set in a race match: the player drives a kart, predicted with `sim::kart::step_kart` (the server's own function).
    kart_mode: Option<KartMode>,
    /// The race course (set with `set_course`), handed to the kart mode when it starts.
    course: Option<std::sync::Arc<crate::sim::race::RaceCourse>>,
    pending: VecDeque<PlayerInput>,
    next_seq: u32,
    correction: Vec2,
    /// Total reconciliations that changed the state by more than a millimetre (diagnostics).
    pub corrections: u32,
    /// Largest such change, m.
    pub worst_correction: f32,
}

impl Predictor {
    /// Starts predicting from `state` (the spawn the server gave).
    pub fn new(state: PlayerState) -> Self {
        Predictor { state, kart_mode: None, course: None, pending: VecDeque::new(), next_seq: 1, correction: Vec2::ZERO, corrections: 0, worst_correction: 0.0 }
    }

    /// The sequence number the next input gets.
    pub fn next_seq(&mut self) -> u32 {
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }

    /// Applies one local tick of input immediately and remembers it until acknowledged. Returns the
    /// horizontal speed (for the local walk animation).
    pub fn apply_local(&mut self, input: PlayerInput, colliders: &[Collider2D], ground: &GroundCandidates) -> f32 {
        self.apply_local_tuned(input, colliders, ground, PlayerTuning::default(), &[])
    }

    /// [`apply_local`](Self::apply_local) with map-authored movement and jump pads.
    pub fn apply_local_tuned(
        &mut self,
        input: PlayerInput,
        colliders: &[Collider2D],
        ground: &GroundCandidates,
        tuning: PlayerTuning,
        jump_pads: &[JumpPad],
    ) -> f32 {
        let speed = step_player_tuned(&mut self.state, &input, colliders, ground, tuning, jump_pads);
        self.pending.push_back(input);
        while self.pending.len() > MAX_PENDING {
            self.pending.pop_front();
        }
        self.correction *= CORRECTION_DECAY;
        speed
    }

    /// The server says: after processing input `ack_seq`, the player is in `server`. Take that, replay
    /// what the server has not seen yet, and smooth any difference from what was on screen.
    pub fn reconcile(&mut self, server: PlayerState, ack_seq: u32, colliders: &[Collider2D], ground: &GroundCandidates) {
        self.reconcile_tuned(server, ack_seq, colliders, ground, PlayerTuning::default(), &[]);
    }

    /// [`reconcile`](Self::reconcile) with map-authored movement and jump pads.
    pub fn reconcile_tuned(
        &mut self,
        server: PlayerState,
        ack_seq: u32,
        colliders: &[Collider2D],
        ground: &GroundCandidates,
        tuning: PlayerTuning,
        jump_pads: &[JumpPad],
    ) {
        let before = self.state;
        while self.pending.front().is_some_and(|i| (i.seq.wrapping_sub(ack_seq) as i32) <= 0) {
            self.pending.pop_front();
        }
        self.state = server;
        for input in self.pending.iter() {
            step_player_tuned(&mut self.state, input, colliders, ground, tuning, jump_pads);
        }
        // The short way round: on a looping world the two states can straddle the seam, a whole period apart on paper and centimetres in fact.
        let err = tuning.expanse.delta(self.state.pos, before.pos);
        let mag = err.length();
        if mag > 0.001 {
            self.corrections += 1;
            self.worst_correction = self.worst_correction.max(mag);
        }
        let snap_distance = if tuning.acceleration > 0.0 { SNAP_DISTANCE.max(tuning.max_speed * 0.12) } else { SNAP_DISTANCE };
        if mag > snap_distance {
            self.correction = Vec2::ZERO;
        } else {
            self.correction += err;
        }
    }

    /// Starts predicting a kart (a race match): from now on use [`apply_local_kart`](Self::apply_local_kart) and
    /// [`reconcile_kart`](Self::reconcile_kart). `kart` is the server's kart memory, `spec` the driver's numbers.
    pub fn enable_kart(&mut self, kart: KartState, spec: KartSpec) {
        let course = self.course.clone();
        self.kart_mode = Some(KartMode { kart, spec, course });
    }

    /// Tells the predictor which race course it is on, so it looks surfaces (mud, water ...) up exactly as the server does. Call before the first snapshot.
    pub fn set_course(&mut self, course: Option<std::sync::Arc<crate::sim::race::RaceCourse>>) {
        self.course = course.clone();
        if let Some(mode) = self.kart_mode.as_mut() {
            mode.course = course;
        }
    }

    /// The predicted kart memory (boost, drift, spin-out), in a race match.
    pub fn kart(&self) -> Option<&KartState> {
        self.kart_mode.as_ref().map(|k| &k.kart)
    }

    /// One local tick of kart input, applied immediately and remembered until acknowledged. `can_drive` is whether the race's light is green: until
    /// then the server ignores the driver's input, so the prediction ignores it too (the remembered input is the one that counted, so a replay
    /// agrees with the server exactly). Returns the speed after the tick. Without kart mode it predicts a walk.
    pub fn apply_local_kart(&mut self, input: PlayerInput, can_drive: bool, colliders: &[Collider2D], ground: &GroundCandidates) -> f32 {
        let Some(mode) = self.kart_mode.as_mut() else { return self.apply_local(input, colliders, ground) };
        let effective = if can_drive { input } else { PlayerInput { seq: input.seq, ..Default::default() } };
        let surface = mode.course.as_ref().map_or(Surface::Road, |c| c.surface_at(self.state.pos));
        let speed = step_kart(&mut self.state, &mut mode.kart, &effective, &mode.spec, surface, colliders, ground);
        self.pending.push_back(effective);
        while self.pending.len() > MAX_PENDING {
            self.pending.pop_front();
        }
        self.correction *= CORRECTION_DECAY;
        speed
    }

    /// The server says: after processing input `ack_seq`, the kart is in `server` with memory `server_kart`. Take that, replay what the server has not
    /// seen yet with the kart step, and smooth any difference from what was on screen.
    pub fn reconcile_kart(&mut self, server: PlayerState, server_kart: KartState, ack_seq: u32, colliders: &[Collider2D], ground: &GroundCandidates) {
        let Some(mode) = self.kart_mode.as_mut() else { return self.reconcile(server, ack_seq, colliders, ground) };
        let before = self.state;
        while self.pending.front().is_some_and(|i| (i.seq.wrapping_sub(ack_seq) as i32) <= 0) {
            self.pending.pop_front();
        }
        self.state = server;
        mode.kart = server_kart;
        for input in self.pending.iter() {
            let surface = mode.course.as_ref().map_or(Surface::Road, |c| c.surface_at(self.state.pos));
            step_kart(&mut self.state, &mut mode.kart, input, &mode.spec, surface, colliders, ground);
        }
        let err = self.state.pos - before.pos;
        let mag = err.length();
        if mag > 0.001 {
            self.corrections += 1;
            self.worst_correction = self.worst_correction.max(mag);
        }
        // A kart covers up to half a metre a tick: a correction under about a tenth of a second of travel is smoothed, more is a snap.
        if mag > SNAP_DISTANCE.max(mode.spec.top_speed * 0.12) {
            self.correction = Vec2::ZERO;
        } else {
            self.correction += err;
        }
    }

    /// [`reconcile_kart`](Self::reconcile_kart) or [`reconcile_tuned`](Self::reconcile_tuned), whichever the snapshot's record calls for: a record
    /// with a kart block puts the predictor in kart mode (with that driver's numbers) and reconciles as a kart; one without reconciles as a walker.
    #[allow(clippy::too_many_arguments)]
    pub fn reconcile_snapshot(
        &mut self,
        server: PlayerState,
        kart: Option<&crate::net::protocol::KartSnap>,
        ack_seq: u32,
        colliders: &[Collider2D],
        ground: &GroundCandidates,
        tuning: PlayerTuning,
        jump_pads: &[JumpPad],
    ) {
        match kart.and_then(|k| crate::sim::kart::Driver::from_wire(k.driver).map(|d| (k, d.spec()))) {
            Some((k, spec)) => {
                if self.kart_mode.as_ref().is_none_or(|m| m.spec != spec) {
                    self.enable_kart(k.to_state(), spec);
                }
                self.reconcile_kart(server, k.to_state(), ack_seq, colliders, ground);
            }
            None => self.reconcile_tuned(server, ack_seq, colliders, ground, tuning, jump_pads),
        }
    }

    /// One local tick as a kart if the predictor is in kart mode (`can_drive`: the light is green), as a walker otherwise.
    pub fn apply_local_auto(
        &mut self,
        input: PlayerInput,
        can_drive: bool,
        colliders: &[Collider2D],
        ground: &GroundCandidates,
        tuning: PlayerTuning,
        jump_pads: &[JumpPad],
    ) -> f32 {
        if self.kart_mode.is_some() {
            self.apply_local_kart(input, can_drive, colliders, ground)
        } else {
            self.apply_local_tuned(input, colliders, ground, tuning, jump_pads)
        }
    }

    /// Jumps to `state` with nothing pending (first join, or a resume after reconnecting).
    pub fn teleport(&mut self, state: PlayerState) {
        self.state = state;
        self.pending.clear();
        self.correction = Vec2::ZERO;
    }

    /// Where to draw the local player: the predicted position plus whatever correction is still fading.
    pub fn visual_pos(&self) -> Vec2 {
        self.state.pos + self.correction
    }

    /// Inputs sent but not yet acknowledged.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn arena_packet_loss_corrections_are_smoothed_within_the_movement_envelope() {
        let (colliders, ground) = world();
        let mut predictor = Predictor::new(start());
        let before = predictor.visual_pos();
        let server = PlayerState { pos: before + Vec2::new(2.0, 0.0), ..predictor.state };
        let tuning = crate::player::PlayerTuning { acceleration: 12.0, max_speed: 20.0, ..Default::default() };
        predictor.reconcile_tuned(server, 0, &colliders, &ground, tuning, &[]);
        assert_eq!(predictor.state.pos, server.pos);
        assert!((predictor.visual_pos() - before).length() < 0.001);
    }
    use super::*;
    use crate::collide::{collect_box_colliders, collect_ground_candidates};
    use crate::player::Character;
    use crate::sim::player::step_player;
    use std::path::Path;

    fn world() -> (Vec<Collider2D>, GroundCandidates) {
        let scene = crate::load_scene(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
        (collect_box_colliders(&scene), collect_ground_candidates(&scene))
    }

    fn start() -> PlayerState {
        PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human)
    }

    #[test]
    fn momentum_reconciliation_replays_air_control_exactly() {
        let (c, g) = world();
        let tuning = PlayerTuning { acceleration: 12.0, air_acceleration: 2.0, ..Default::default() };
        let mut client = Predictor::new(start());
        let mut acknowledged = start();
        for seq in 1..=24 {
            let input = PlayerInput { seq, forward: 1, strafe: i8::from(seq > 12), jump: seq == 10, yaw: 1.0, ..Default::default() };
            client.apply_local_tuned(input, &c, &g, tuning, &[]);
            if seq <= 12 {
                step_player_tuned(&mut acknowledged, &input, &c, &g, tuning, &[]);
            }
        }
        let expected = client.state;
        client.reconcile_tuned(acknowledged, 12, &c, &g, tuning, &[]);
        assert_eq!(client.state, expected);
        assert!(client.state.velocity.length() > 0.0);
    }

    fn input(p: &mut Predictor, forward: i8, jump: bool) -> PlayerInput {
        PlayerInput { seq: p.next_seq(), forward, jump, yaw: 90f32.to_radians(), ..Default::default() }
    }

    #[test]
    fn when_the_server_agrees_reconciliation_changes_nothing() {
        let (c, g) = world();
        let mut client = Predictor::new(start());
        let mut server = start();
        let mut sent = Vec::new();
        for k in 0..60 {
            let i = input(&mut client, 1, k == 10);
            client.apply_local(i, &c, &g);
            sent.push(i);
        }
        // The server has processed the first 40 inputs (the rest are still in flight).
        for i in &sent[..40] {
            step_player(&mut server, i, &c, &g);
        }
        let predicted = client.state;
        client.reconcile(server, sent[39].seq, &c, &g);
        assert_eq!(client.state, predicted, "same function, same inputs: bit-identical");
        assert_eq!(client.corrections, 0);
        assert_eq!(client.pending_len(), 20);
        assert!(client.visual_pos().distance(predicted.pos) < 1e-6);
    }

    #[test]
    fn a_disagreement_is_corrected_and_smoothed_not_popped() {
        let (c, g) = world();
        let mut client = Predictor::new(start());
        let mut server = start();
        let mut last = 0;
        for _ in 0..30 {
            let i = input(&mut client, 1, false);
            client.apply_local(i, &c, &g);
            step_player(&mut server, &i, &c, &g);
            last = i.seq;
        }
        server.pos.x -= 0.4; // something shoved the player on the server (a prop, say)
        let on_screen = client.visual_pos();
        client.reconcile(server, last, &c, &g);
        assert!((client.state.pos.x - server.pos.x).abs() < 1e-5, "the prediction now matches the server");
        assert!((client.visual_pos() - on_screen).length() < 1e-4, "but the *drawn* position did not pop");
        // ... and it glides to the corrected position over the next ticks.
        for _ in 0..40 {
            let i = input(&mut client, 0, false);
            client.apply_local(i, &c, &g);
        }
        assert!((client.visual_pos() - client.state.pos).length() < 0.01, "correction has faded");
        assert!(client.corrections == 1 && (client.worst_correction - 0.4).abs() < 0.01);
    }

    #[test]
    fn a_huge_correction_snaps() {
        let (c, g) = world();
        let mut client = Predictor::new(start());
        let mut server = start();
        server.pos = Vec2::new(-10.0, 0.0);
        client.reconcile(server, 0, &c, &g);
        assert_eq!(client.visual_pos(), server.pos, "a teleport is not smoothed");
    }

    #[test]
    fn jump_state_is_reconciled_exactly() {
        let (c, g) = world();
        let mut client = Predictor::new(start());
        let mut server = start();
        let mut sent = Vec::new();
        for k in 0..20 {
            let i = input(&mut client, 0, k == 0);
            client.apply_local(i, &c, &g);
            sent.push(i);
        }
        for i in &sent[..10] {
            step_player(&mut server, i, &c, &g);
        }
        let predicted = client.state;
        client.reconcile(server, sent[9].seq, &c, &g);
        assert_eq!(client.state, predicted, "mid-jump replay (vy included) is exact");
    }

    /// A server stand-in: the same kart step, fed the same inputs `lag` ticks late.
    struct Server {
        state: PlayerState,
        kart: KartState,
        spec: KartSpec,
    }

    impl Server {
        fn new(start: PlayerState, spec: KartSpec) -> Server {
            Server { state: start, kart: KartState::default(), spec }
        }

        fn process(&mut self, input: &PlayerInput, can_drive: bool, colliders: &[Collider2D], ground: &GroundCandidates) {
            let input = if can_drive { *input } else { PlayerInput { seq: input.seq, ..Default::default() } };
            step_kart(&mut self.state, &mut self.kart, &input, &self.spec, Surface::Road, colliders, ground);
        }
    }

    fn kart_input(predictor: &mut Predictor, t: u32) -> PlayerInput {
        PlayerInput { seq: predictor.next_seq(), forward: 1, strafe: ((t / 45) % 3) as i8 - 1, jump: t % 180 < 70, ..Default::default() }
    }

    #[test]
    fn a_predicted_kart_agrees_with_the_server_through_lag_drifts_hops_and_a_wall() {
        use crate::sim::kart::Driver;
        let wall = [Collider2D { min: Vec2::new(60.0, -50.0), max: Vec2::new(62.0, 50.0), min_y: 0.0, max_y: 3.0 }];
        let ground = GroundCandidates::default();
        for driver in Driver::ALL {
            let spec = driver.spec();
            let start = PlayerState::spawn(0.0, 0.0, 0.0, 90.0, Character::Human);
            let mut predictor = Predictor::new(start);
            predictor.enable_kart(KartState::default(), spec);
            let mut server = Server::new(start, spec);
            let (lag, mut sent) = (6usize, Vec::new());
            for t in 0..900u32 {
                let input = kart_input(&mut predictor, t);
                predictor.apply_local_kart(input, true, &wall, &ground);
                sent.push(input);
                if sent.len() > lag {
                    let processed = sent[sent.len() - 1 - lag];
                    server.process(&processed, true, &wall, &ground);
                    if t % 4 == 0 {
                        predictor.reconcile_kart(server.state, server.kart, processed.seq, &wall, &ground);
                    }
                }
            }
            assert_eq!(predictor.corrections, 0, "{}: the same function on the same inputs never needs correcting", driver.name());
            for input in &sent[sent.len() - lag..] {
                server.process(input, true, &wall, &ground);
            }
            assert_eq!(predictor.state, server.state, "{}: once the server catches up they are the same kart", driver.name());
            assert_eq!(predictor.kart(), Some(&server.kart));
            assert!(server.state.pos.x > 20.0, "{} drove somewhere: {:?}", driver.name(), server.state.pos);
        }
    }

    #[test]
    fn the_prediction_holds_the_kart_through_the_countdown_like_the_server() {
        use crate::sim::kart::Driver;
        let ground = GroundCandidates::default();
        let spec = Driver::Deer.spec();
        let start = PlayerState::spawn(0.0, 0.0, 0.0, 90.0, Character::Human);
        let mut predictor = Predictor::new(start);
        predictor.enable_kart(KartState::default(), spec);
        let mut server = Server::new(start, spec);
        let (lag, mut sent) = (5usize, Vec::new());
        for t in 0..300u32 {
            let green = t >= 120;
            let input = kart_input(&mut predictor, t);
            predictor.apply_local_kart(input, green, &[], &ground);
            sent.push(input);
            if t < 119 {
                assert!((predictor.state.pos - start.pos).length() < 1e-6, "held on the grid before the light at tick {t}");
            }
            if sent.len() > lag {
                let index = sent.len() - 1 - lag;
                let processed = sent[index];
                // The server's light turned green at its own tick 120, which is the same input the client marked green.
                server.process(&processed, index as u32 >= 120, &[], &ground);
                if t % 3 == 0 {
                    predictor.reconcile_kart(server.state, server.kart, processed.seq, &[], &ground);
                }
            }
        }
        assert_eq!(predictor.corrections, 0);
        assert!(predictor.state.pos.x > 10.0, "and it drove once the light went green: {:?}", predictor.state.pos);
    }

    #[test]
    fn a_hit_the_client_did_not_predict_is_taken_from_the_server_and_carried_forward() {
        use crate::sim::kart::Driver;
        let ground = GroundCandidates::default();
        let spec = Driver::Bunny.spec();
        let start = PlayerState::spawn(0.0, 0.0, 0.0, 90.0, Character::Human);
        let mut predictor = Predictor::new(start);
        predictor.enable_kart(KartState::default(), spec);
        let input = PlayerInput { forward: 1, ..Default::default() };
        let mut sent = Vec::new();
        for _ in 0..30 {
            let input = PlayerInput { seq: predictor.next_seq(), ..input };
            predictor.apply_local_kart(input, true, &[], &ground);
            sent.push(input);
        }
        // The server saw a Bear hit us at input 20: it reports a spin-out and a kart that has lost its way.
        let mut hit = predictor.kart().copied().unwrap();
        hit.spin_out(45);
        let server = PlayerState { yaw: predictor.state.yaw + 1.0, ..predictor.state };
        predictor.reconcile_kart(server, hit, sent[19].seq, &[], &ground);
        assert_eq!(predictor.pending_len(), 10);
        assert_eq!(predictor.kart().unwrap().spin_ticks, 45 - 10, "the replay carried the spin-out forward by the ten unacknowledged ticks");
        assert!(predictor.state.yaw > 1.0, "and took the server's heading");
        // A predictor that was never told it is a kart falls back to walking rather than misbehaving.
        let mut walker = Predictor::new(start);
        walker.apply_local_kart(PlayerInput { seq: 1, forward: 1, ..Default::default() }, true, &[], &ground);
        assert!(walker.kart().is_none() && walker.state.pos != start.pos);
    }

    #[test]
    fn a_predicted_item_takes_effect_the_moment_the_button_is_pressed_and_the_server_agrees() {
        use crate::net::protocol::KartSnap;
        use crate::sim::kart::{Driver, Item};
        let ground = GroundCandidates::default();
        for (item, driver) in [(Item::Mushroom, Driver::Wolf), (Item::Bubble, Driver::Bear), (Item::Acorn, Driver::Duck)] {
            let spec = driver.spec();
            let start = PlayerState::spawn(0.0, 0.0, 0.0, 90.0, Character::Human);
            // A snapshot's kart block says the box gave this kart an item; that is how a client learns what it holds.
            let mut predictor = Predictor::new(start);
            let snap = KartSnap { driver: driver.wire(), item: item.wire(), ..Default::default() };
            predictor.reconcile_snapshot(start, Some(&snap), 0, &[], &ground, PlayerTuning::default(), &[]);
            assert_eq!(predictor.kart().unwrap().item, item);
            let mut server = Server::new(start, spec);
            server.kart.item = item;
            let (lag, mut sent) = (5usize, Vec::new());
            for t in 0..300u32 {
                let input = PlayerInput { seq: predictor.next_seq(), forward: 1, attack: (150..152).contains(&t), ..Default::default() };
                predictor.apply_local_kart(input, true, &[], &ground);
                sent.push(input);
                if t == 151 {
                    match item {
                        Item::Mushroom => assert!(predictor.kart().unwrap().boost_ticks > 0, "the boost is on screen at once, not a round trip later"),
                        Item::Bubble => assert!(predictor.kart().unwrap().shield_ticks > 0, "so is the shield"),
                        _ => assert_eq!(predictor.kart().unwrap().item, Item::None, "the acorn left the hand"),
                    }
                }
                if sent.len() > lag {
                    let processed = sent[sent.len() - 1 - lag];
                    server.process(&processed, true, &[], &ground);
                    if t % 3 == 0 {
                        predictor.reconcile_kart(server.state, server.kart, processed.seq, &[], &ground);
                    }
                }
            }
            assert_eq!(predictor.corrections, 0, "{item:?}: using an item never needs a correction");
            for input in &sent[sent.len() - lag..] {
                server.process(input, true, &[], &ground);
            }
            assert_eq!(predictor.state, server.state, "{item:?}");
            assert_eq!(predictor.kart(), Some(&server.kart), "{item:?}");
        }
    }

    #[test]
    fn the_wire_block_round_trips_the_kart_memory_a_client_needs() {
        use crate::net::protocol::KartSnap;
        use crate::sim::kart::Item;
        let k = KartState {
            boost_ticks: 42,
            drift_dir: -1,
            drift_charge: 1.234,
            spin_ticks: 7,
            jump_held: true,
            slip_charge: 0.5,
            item: Item::Acorn,
            shield_ticks: 250,
            ability_cooldown: 200,
            attack_held: true,
            interact_held: true,
        };
        let snap = KartSnap {
            driver: 0,
            boost_ticks: 42,
            spin_ticks: 7,
            drift_dir: -1,
            jump_held: true,
            drift_charge_ms: 1234,
            slip: 100,
            item: Item::Acorn.wire(),
            shield_ticks: 250,
            ability_cooldown: 200,
            attack_held: true,
            interact_held: true,
            ..Default::default()
        };
        assert_eq!(snap.to_state(), k, "every field a prediction depends on survives the trip");
    }

    #[test]
    fn prediction_agrees_with_the_server_across_the_edge_of_a_mud_patch() {
        use crate::sim::kart::Driver;
        use crate::sim::race::RaceCourse;
        let text = r#"{"zones":[{"id":"a","rect":[-1,-1,1,1]},{"id":"b","rect":[9,-1,11,1]},{"id":"c","rect":[19,-1,21,1]},{"id":"pool","rect":[40,-30,70,30]},{"id":"pond","rect":[100,-30,130,30]}],
            "race":{"gates":["a","b","c"],"surfaces":[{"zone":"pool","kind":"mud"},{"zone":"pond","kind":"water"}]}}"#;
        let course = std::sync::Arc::new(RaceCourse::from_scene_text(text).unwrap().unwrap());
        let ground = GroundCandidates::default();
        for driver in [Driver::Duck, Driver::Deer, Driver::Beaver] {
            let spec = driver.spec();
            let start = PlayerState::spawn(0.0, 0.0, 0.0, 90.0, Character::Human);
            let mut predictor = Predictor::new(start);
            predictor.set_course(Some(course.clone()));
            predictor.enable_kart(KartState::default(), spec);
            let mut server = Server::new(start, spec);
            let (lag, mut sent) = (6usize, Vec::new());
            for t in 0..500u32 {
                // Small alternating steering pulses: the kart weaves but stays roughly on course, so it crosses each patch's edges at slightly different angles.
                let strafe = match t % 150 {
                    0..=7 => 1,
                    75..=82 => -1,
                    _ => 0,
                };
                let input = PlayerInput { seq: predictor.next_seq(), forward: 1, strafe, ..Default::default() };
                predictor.apply_local_kart(input, true, &[], &ground);
                sent.push(input);
                if sent.len() > lag {
                    let processed = sent[sent.len() - 1 - lag];
                    // The server looks the surface up at the start of its tick, exactly as the prediction does.
                    let surface = course.surface_at(server.state.pos);
                    step_kart(&mut server.state, &mut server.kart, &processed, &server.spec, surface, &[], &ground);
                    if t % 4 == 0 {
                        predictor.reconcile_kart(server.state, server.kart, processed.seq, &[], &ground);
                    }
                }
            }
            assert_eq!(predictor.corrections, 0, "{}: crossing mud and water never needs a correction", driver.name());
            assert!(server.state.pos.x > 100.0, "{} drove through both patches: {:?}", driver.name(), server.state.pos);
        }
    }
}
