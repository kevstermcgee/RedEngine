//! Client-side prediction of the local player, and reconciliation with the server.
//!
//! The client applies its own input immediately (so movement feels instant whatever the latency)
//! using exactly the server's movement function ([`crate::sim::player::step_player`]), and remembers
//! the inputs the server has not yet acknowledged. When a snapshot arrives with the server's state
//! *after* input `ack`, the client takes that state and replays the unacknowledged inputs on top; if
//! prediction and server agreed, nothing changes, and if they did not (a prop nudged them, a wall
//! the client had wrong) the difference is smoothed away over a few ticks instead of popping.

use crate::collide::{Collider2D, GroundCandidates};
use crate::sim::player::{step_player, PlayerInput, PlayerState};
use glam::Vec2;
use std::collections::VecDeque;

/// Corrections bigger than this (m) are snapped instead of smoothed (a teleport, a respawn).
const SNAP_DISTANCE: f32 = 1.5;
/// Fraction of the visual correction left after each tick.
const CORRECTION_DECAY: f32 = 0.8;
/// Unacknowledged inputs kept (about two seconds).
const MAX_PENDING: usize = 128;

/// The local player's predicted state.
#[derive(Debug, Clone)]
pub struct Predictor {
    /// The predicted authoritative-equivalent state (what the server will compute).
    pub state: PlayerState,
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
        Predictor { state, pending: VecDeque::new(), next_seq: 1, correction: Vec2::ZERO, corrections: 0, worst_correction: 0.0 }
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
        let speed = step_player(&mut self.state, &input, colliders, ground);
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
        let before = self.state;
        while self.pending.front().is_some_and(|i| (i.seq.wrapping_sub(ack_seq) as i32) <= 0) {
            self.pending.pop_front();
        }
        self.state = server;
        for input in self.pending.iter() {
            step_player(&mut self.state, input, colliders, ground);
        }
        let err = before.pos - self.state.pos;
        let mag = err.length();
        if mag > 0.001 {
            self.corrections += 1;
            self.worst_correction = self.worst_correction.max(mag);
        }
        if mag > SNAP_DISTANCE {
            self.correction = Vec2::ZERO;
        } else {
            self.correction += err;
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
    use super::*;
    use crate::collide::{collect_box_colliders, collect_ground_candidates};
    use crate::player::Character;
    use std::path::Path;

    fn world() -> (Vec<Collider2D>, GroundCandidates) {
        let scene = crate::load_scene(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")).unwrap();
        (collect_box_colliders(&scene), collect_ground_candidates(&scene))
    }

    fn start() -> PlayerState {
        PlayerState::spawn(-22.0, -3.0, 0.0, 90.0, Character::Human)
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
}
