//! State hashing and dumping for a [`MatchSim`]: the "authoritative state checksum hooks".
//!
//! The exact checksum covers every player and every dynamic prop bit for bit plus the rules state; the *coarse* one
//! quantises floats to millimetres so two machines whose float maths differs in the last bit still agree unless the
//! game itself diverged. A [`Dump`] is the same state in readable form, for the diff a divergence report prints.

use super::match_sim::MatchSim;
use super::trace::{Checkpoint, Dump};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

struct Hasher(u64);

impl Hasher {
    fn new() -> Self {
        Hasher(FNV_OFFSET)
    }
    fn mix(&mut self, v: u64) {
        self.0 ^= v;
        self.0 = self.0.wrapping_mul(FNV_PRIME);
    }
    fn f(&mut self, x: f32) {
        self.mix(x.to_bits() as u64);
    }
    /// A float quantised to millimetres (so last-bit platform differences vanish).
    fn q(&mut self, x: f32) {
        self.mix((x * 1000.0).round() as i64 as u64);
    }
}

impl MatchSim {
    /// The exact checksums of the three parts of the state: `(players, props, rules)`.
    pub fn checksum_parts(&self) -> (u64, u64, u64) {
        let mut players = Hasher::new();
        for (slot, p) in self.players() {
            players.mix(slot as u64);
            for f in [p.state.pos.x, p.state.pos.y, p.state.foot_y, p.state.vy, p.state.velocity.x, p.state.velocity.y, p.state.yaw, p.state.pitch] {
                players.f(f);
            }
            players.mix(p.combat.state_hash());
            players.mix(self.props.held_by(slot).map_or(u64::MAX, |h| h as u64));
        }
        let mut props = Hasher::new();
        let e = self.props.entities();
        for slot in 0..e.len() {
            let t = e.transforms.get(slot);
            props.mix(self.props.prop_of_entity(slot) as u64);
            for f in t.position.to_array().into_iter().chain(t.rotation.to_array()) {
                props.f(f);
            }
        }
        // A kart race adds its own state to the rules part; a match without one hashes exactly as before, so older traces still replay.
        let mut rules = self.rules.checksum();
        if let Some(race) = &self.race {
            let mut karts = Hasher::new();
            karts.mix(race.checksum());
            for (slot, _) in self.players() {
                let k = &self.karts[slot];
                karts.mix(slot as u64 | (self.drivers[slot].wire() as u64) << 8);
                karts.mix(k.boost_ticks as u64 | (k.drift_dir as u8 as u64) << 16 | (k.spin_ticks as u64) << 24 | (k.jump_held as u64) << 40);
                karts.f(k.drift_charge);
                karts.f(k.slip_charge);
            }
            rules ^= karts.0.rotate_left(17);
        }
        (players.0, props.0, rules)
    }

    /// A 64-bit checksum of the whole simulation state (players, promoted props' poses, rules), bit-exact.
    /// Two runs fed the same inputs on the same platform must agree; a mismatch means they diverged.
    pub fn checksum(&self) -> u64 {
        let (p, q, r) = self.checksum_parts();
        Checkpoint { tick: self.tick, players: p, props: q, rules: r, coarse: 0 }.combined()
    }

    /// The checksums a [`Trace`](super::trace::Trace) records after each tick.
    pub fn checkpoint(&self) -> Checkpoint {
        let (players, props, rules) = self.checksum_parts();
        let mut c = Hasher::new();
        for (slot, p) in self.players() {
            c.mix(slot as u64);
            for f in [p.state.pos.x, p.state.pos.y, p.state.foot_y, p.state.vy, p.state.velocity.x, p.state.velocity.y] {
                c.q(f);
            }
            c.mix(p.combat.state_hash());
            c.mix(self.props.held_by(slot).map_or(u64::MAX, |h| h as u64));
        }
        let e = self.props.entities();
        for slot in 0..e.len() {
            let t = e.transforms.get(slot);
            c.mix(self.props.prop_of_entity(slot) as u64);
            for f in t.position.to_array() {
                c.q(f);
            }
        }
        c.mix(rules);
        Checkpoint { tick: self.tick, players, props, rules, coarse: c.0 }
    }

    /// A readable snapshot of the state (see [`Dump`]).
    pub fn dump(&self) -> Dump {
        let e = self.props.entities();
        Dump {
            tick: self.tick,
            players: self
                .players()
                .map(|(slot, p)| {
                    let s = &p.state;
                    [
                        slot as f64,
                        s.pos.x as f64,
                        s.foot_y as f64,
                        s.pos.y as f64,
                        s.yaw as f64,
                        s.pitch as f64,
                        s.vy as f64,
                        s.velocity.x as f64,
                        s.velocity.y as f64,
                    ]
                })
                .collect(),
            props: (0..e.len())
                .map(|slot| {
                    let t = e.transforms.get(slot);
                    let (p, q) = (t.position, t.rotation);
                    [self.props.prop_of_entity(slot) as f64, p.x as f64, p.y as f64, p.z as f64, q.x as f64, q.y as f64, q.z as f64, q.w as f64]
                })
                .collect(),
            vars: self.rules.vars().into_iter().map(|(n, v)| (n.to_string(), v)).collect(),
            hidden: self.rules.hidden().map(str::to_string).collect(),
            collision_disabled: self.rules.collision_disabled().map(str::to_string).collect(),
            ended: self.rules.ended().map(str::to_string),
        }
    }
}
