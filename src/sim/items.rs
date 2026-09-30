//! Pickups and hazards for Great Outdoors (ADR 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature): item boxes that hand out a Mushroom, an
//! Acorn or a Bubble, and the things thrown or laid on the track (an Acorn in flight, the Beaver's planks).
//!
//! What an item does *to the kart that holds it* (a Mushroom's boost, a Bubble's shield) lives in `sim::kart::step_kart_ex`, so a client predicts it
//! exactly. What it does to the *world* lives here, in the simulation only: a fixed-size [`HazardPool`] (no allocation per tick, so the allocation
//! budget holds) and [`ItemBoxes`] with their respawn timers. Everything is deterministic: a roll depends only on the tick, the slot, the box and the
//! place, never on a global random state, so a replay draws the same items.

use crate::collide::{collider_blocks_at, Collider2D};
use crate::player::FIXED_DT;
use crate::sim::kart::{Item, KART_RADIUS};
use glam::Vec2;

/// Most hazards on the track at once (Acorns in flight and planks together).
pub const MAX_HAZARDS: usize = 24;
/// An Acorn leaves the kart this much faster than the kart was going, m/s.
pub const ACORN_SPEED_BONUS: f32 = 20.0;
/// How long an Acorn flies before it is gone, ticks (5 s).
pub const ACORN_TTL_TICKS: u16 = 300;
/// How long a plank stays, ticks (20 s).
pub const PLANK_TTL_TICKS: u16 = 1200;
/// Spin-out from an Acorn, ticks.
pub const ACORN_SPIN_TICKS: u16 = 60;
/// Spin-out from a plank, ticks.
pub const PLANK_SPIN_TICKS: u16 = 30;
/// Collision radius of a hazard, m.
pub const HAZARD_RADIUS: f32 = 0.5;
/// An Acorn steers towards a kart within this many metres ahead of it...
pub const ACORN_SEEK_RANGE: f32 = 40.0;
/// ...that is within this angle of its heading (cosine of 28 degrees)...
const ACORN_SEEK_COS: f32 = 0.883;
/// ...turning at most this fast, radians per second (about 110 degrees).
const ACORN_TURN_RATE: f32 = 1.9;
/// A hazard cannot hit the kart that made it for this long after it appears, ticks (so a thrown Acorn does not hit its thrower).
pub const OWNER_GRACE_TICKS: u16 = 45;

/// What a hazard is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HazardKind {
    /// A thrown nut: flies straight ahead, spins out the first kart it hits, and shatters on a wall.
    Acorn,
    /// A plank laid on the track by the Beaver: stays put until something hits it or it rots.
    Plank,
}

/// One hazard on the track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hazard {
    /// What it is.
    pub kind: HazardKind,
    /// The slot that made it.
    pub owner: u8,
    /// Where it is (x, z).
    pub pos: Vec2,
    /// How it is moving, m/s (zero for a plank).
    pub vel: Vec2,
    /// Ticks since it appeared.
    pub age: u16,
    /// Ticks left before it is gone.
    pub ttl: u16,
}

impl Hazard {
    /// An Acorn thrown from `pos` with velocity `vel`.
    pub fn acorn(owner: u8, pos: Vec2, vel: Vec2) -> Hazard {
        Hazard { kind: HazardKind::Acorn, owner, pos, vel, age: 0, ttl: ACORN_TTL_TICKS }
    }

    /// A plank laid at `pos`.
    pub fn plank(owner: u8, pos: Vec2) -> Hazard {
        Hazard { kind: HazardKind::Plank, owner, pos, vel: Vec2::ZERO, age: 0, ttl: PLANK_TTL_TICKS }
    }
}

/// Every hazard on the track, in a fixed number of slots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HazardPool {
    slots: [Option<Hazard>; MAX_HAZARDS],
}

impl Default for HazardPool {
    fn default() -> Self {
        HazardPool { slots: [None; MAX_HAZARDS] }
    }
}

impl HazardPool {
    /// The hazards in slot order.
    pub fn iter(&self) -> impl Iterator<Item = &Hazard> {
        self.slots.iter().flatten()
    }

    /// How many hazards there are.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Adds a hazard. When the pool is full the oldest plank is replaced; if there is none the new hazard is dropped and this returns `false`.
    pub fn spawn(&mut self, hazard: Hazard) -> bool {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.is_none()) {
            *slot = Some(hazard);
            return true;
        }
        let oldest = self.slots.iter_mut().filter(|s| s.is_some_and(|h| h.kind == HazardKind::Plank)).max_by_key(|s| s.map_or(0, |h| h.age));
        match oldest {
            Some(slot) => {
                *slot = Some(hazard);
                true
            }
            None => false,
        }
    }

    /// Advances every hazard one tick. `karts[i]` is slot `i`'s position (`None` if empty). A hazard that reaches a kart other than its owner (or its
    /// owner after the grace period) calls `hit(slot, spin_ticks)` and is used up; an Acorn that reaches a wall shatters; a hazard whose time is up goes.
    pub fn step(&mut self, karts: &[Option<Vec2>], colliders: &[Collider2D], mut hit: impl FnMut(usize, u16)) {
        for slot in self.slots.iter_mut() {
            let Some(h) = slot else { continue };
            h.age = h.age.saturating_add(1);
            h.ttl = h.ttl.saturating_sub(1);
            if h.kind == HazardKind::Acorn {
                // A mild homing: on a wide track a nut thrown down the lane would almost never meet a kart, so it turns towards the nearest kart ahead of it
                // (not its thrower in the first moments), a little at a time. It stays a dodgeable nut: it cannot turn past its cone or faster than its rate.
                let speed = h.vel.length();
                if speed > 1.0 {
                    let dir = h.vel / speed;
                    let target = karts
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != h.owner as usize || h.age > OWNER_GRACE_TICKS)
                        .filter_map(|(_, k)| *k)
                        .map(|p| (p - h.pos, (p - h.pos).length()))
                        .filter(|(rel, d)| *d < ACORN_SEEK_RANGE && *d > 0.5 && rel.dot(dir) / *d > ACORN_SEEK_COS)
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    if let Some((rel, _)) = target {
                        let want = rel.normalize_or_zero();
                        let cross = dir.x * want.y - dir.y * want.x;
                        let angle = cross.atan2(dir.dot(want)).clamp(-ACORN_TURN_RATE * FIXED_DT, ACORN_TURN_RATE * FIXED_DT);
                        let (s, c) = libm::sincosf(angle);
                        h.vel = Vec2::new(dir.x * c - dir.y * s, dir.x * s + dir.y * c) * speed;
                    }
                }
            }
            h.pos += h.vel * FIXED_DT;
            let in_wall = h.kind == HazardKind::Acorn
                && colliders.iter().any(|c| collider_blocks_at(c, 0.0) && h.pos.x >= c.min.x && h.pos.x <= c.max.x && h.pos.y >= c.min.y && h.pos.y <= c.max.y);
            if h.ttl == 0 || in_wall {
                *slot = None;
                continue;
            }
            let victim = karts
                .iter()
                .enumerate()
                .find(|(i, k)| k.is_some_and(|p| (*i != h.owner as usize || h.age > OWNER_GRACE_TICKS) && (p - h.pos).length() < KART_RADIUS + HAZARD_RADIUS));
            if let Some((i, _)) = victim {
                hit(i, if h.kind == HazardKind::Acorn { ACORN_SPIN_TICKS } else { PLANK_SPIN_TICKS });
                *slot = None;
            }
        }
    }

    /// A 64-bit fold of the pool, for the match checksum.
    pub fn checksum(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        };
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(z) = slot {
                mix(i as u64 | (z.kind as u64) << 8 | (z.owner as u64) << 16 | (z.age as u64) << 24 | (z.ttl as u64) << 40);
                for f in [z.pos.x, z.pos.y, z.vel.x, z.vel.y] {
                    mix(f.to_bits() as u64);
                }
            }
        }
        h
    }
}

/// An item box: a rectangle on the track that hands out an item to the first kart that touches it, then respawns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemBox {
    /// Smallest x, z corner.
    pub min: Vec2,
    /// Largest x, z corner.
    pub max: Vec2,
}

/// The track's item boxes and their respawn timers.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemBoxes {
    boxes: Vec<ItemBox>,
    respawn_ticks: u32,
    ready_in: Vec<u32>,
}

impl ItemBoxes {
    /// Boxes that come back `respawn_ticks` after being taken.
    pub fn new(boxes: Vec<ItemBox>, respawn_ticks: u32) -> ItemBoxes {
        let ready_in = vec![0; boxes.len()];
        ItemBoxes { boxes, respawn_ticks, ready_in }
    }

    /// The boxes.
    pub fn boxes(&self) -> &[ItemBox] {
        &self.boxes
    }

    /// Which boxes are ready to be taken, as a bit mask (bit `i` = box `i`; boxes beyond the 32nd are not reported). What a snapshot tells the clients.
    pub fn ready_mask(&self) -> u32 {
        self.ready_in.iter().take(32).enumerate().fold(0, |mask, (i, t)| if *t == 0 { mask | 1 << i } else { mask })
    }

    /// Whether box `i` is there to be taken.
    pub fn is_ready(&self, i: usize) -> bool {
        self.ready_in.get(i).is_some_and(|t| *t == 0)
    }

    /// Advances one tick. A ready box goes to the lowest slot whose kart touches it and `can_take` says has a free hand: `grant(slot, box)` is called and
    /// the box waits out its respawn.
    pub fn step(&mut self, karts: &[Option<Vec2>], can_take: impl Fn(usize) -> bool, mut grant: impl FnMut(usize, usize)) {
        for (i, b) in self.boxes.iter().enumerate() {
            if self.ready_in[i] > 0 {
                self.ready_in[i] -= 1;
                continue;
            }
            let taker = karts.iter().enumerate().find(|(slot, k)| {
                k.is_some_and(|p| p.x >= b.min.x - KART_RADIUS && p.x <= b.max.x + KART_RADIUS && p.y >= b.min.y - KART_RADIUS && p.y <= b.max.y + KART_RADIUS)
                    && can_take(*slot)
            });
            if let Some((slot, _)) = taker {
                grant(slot, i);
                self.ready_in[i] = self.respawn_ticks;
            }
        }
    }

    /// A 64-bit fold of the timers, for the match checksum.
    pub fn checksum(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for t in &self.ready_in {
            h ^= *t as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
}

/// splitmix64: a good 64-bit mix, so a roll depends on nothing but its inputs.
fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Which item a box gives. Deterministic in `(tick, slot, box_index)`, and weighted by `place` (1 = first): the leader is more likely to get a Bubble
/// to defend the lead, the tail a Mushroom to catch up. `players` is how many are racing.
pub fn roll_item(tick: u64, slot: usize, box_index: usize, place: usize, players: usize) -> Item {
    let r = (mix64(tick ^ (slot as u64) << 40 ^ (box_index as u64) << 48) % 100) as u32;
    let third = if players <= 1 { 1 } else { (place.saturating_sub(1) * 3 / players).min(2) };
    // (Mushroom, Acorn, Bubble) out of 100.
    let (mushroom, acorn) = match third {
        0 => (20, 30),
        1 => (40, 35),
        _ => (60, 30),
    };
    if r < mushroom {
        Item::Mushroom
    } else if r < mushroom + acorn {
        Item::Acorn
    } else {
        Item::Bubble
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall(min: Vec2, max: Vec2) -> Collider2D {
        Collider2D { min, max, min_y: 0.0, max_y: 3.0 }
    }

    #[test]
    fn an_acorn_flies_straight_hits_the_first_other_kart_and_is_used_up() {
        let mut pool = HazardPool::default();
        assert!(pool.spawn(Hazard::acorn(0, Vec2::new(0.0, 0.0), Vec2::new(30.0, 0.0))));
        let mut hits = Vec::new();
        // Slot 0 is the thrower, right where it threw; slot 1 is 12 m ahead; slot 2 is off to the side.
        let karts = [Some(Vec2::new(0.0, 0.0)), Some(Vec2::new(12.0, 0.2)), Some(Vec2::new(12.0, 6.0))];
        for _ in 0..60 {
            pool.step(&karts, &[], |slot, spin| hits.push((slot, spin)));
        }
        assert_eq!(hits, vec![(1, ACORN_SPIN_TICKS)], "the thrower is safe, the side kart is missed, the one ahead is hit once");
        assert!(pool.is_empty(), "and the acorn is used up");
    }

    #[test]
    fn an_acorn_thrown_a_little_off_line_still_finds_the_kart_ahead_but_not_one_far_to_the_side() {
        let run = |target: Vec2| {
            let mut pool = HazardPool::default();
            pool.spawn(Hazard::acorn(0, Vec2::ZERO, Vec2::new(30.0, 0.0)));
            let karts = [Some(Vec2::new(-5.0, 0.0)), Some(target)];
            let mut hits = Vec::new();
            for _ in 0..180 {
                pool.step(&karts, &[], |slot, _| hits.push(slot));
            }
            hits
        };
        assert_eq!(run(Vec2::new(25.0, 5.0)), vec![1], "5 m off the line at 25 m ahead is inside the cone: the acorn steers onto it");
        assert_eq!(run(Vec2::new(25.0, 20.0)), Vec::<usize>::new(), "a kart far off to the side is not chased");
        assert_eq!(run(Vec2::new(-30.0, 2.0)), Vec::<usize>::new(), "and one behind is left alone");
    }

    #[test]
    fn an_acorn_shatters_on_a_wall_and_rots_away_if_nothing_stops_it() {
        let mut pool = HazardPool::default();
        pool.spawn(Hazard::acorn(0, Vec2::ZERO, Vec2::new(30.0, 0.0)));
        let colliders = [wall(Vec2::new(10.0, -5.0), Vec2::new(11.0, 5.0))];
        for _ in 0..30 {
            pool.step(&[None], &colliders, |_, _| panic!("nobody to hit"));
        }
        assert!(pool.is_empty(), "shattered on the wall before reaching 15 m");
        pool.spawn(Hazard::acorn(0, Vec2::ZERO, Vec2::new(0.0, 1.0)));
        for _ in 0..ACORN_TTL_TICKS {
            pool.step(&[None], &[], |_, _| {});
        }
        assert!(pool.is_empty(), "an acorn that hits nothing is gone after its time");
    }

    #[test]
    fn a_plank_waits_for_a_victim_and_spares_its_owner_only_at_first() {
        let mut pool = HazardPool::default();
        pool.spawn(Hazard::plank(1, Vec2::new(5.0, 5.0)));
        let mut hits = Vec::new();
        let owner_on_it = [None, Some(Vec2::new(5.0, 5.0))];
        for _ in 0..OWNER_GRACE_TICKS {
            pool.step(&owner_on_it, &[], |slot, spin| hits.push((slot, spin)));
        }
        assert!(hits.is_empty() && pool.len() == 1, "the owner drives over their own plank in the grace period");
        pool.step(&owner_on_it, &[], |slot, spin| hits.push((slot, spin)));
        assert_eq!(hits, vec![(1, PLANK_SPIN_TICKS)], "afterwards it hits anyone, its owner included");
        assert!(pool.is_empty());
        pool.spawn(Hazard::plank(1, Vec2::new(5.0, 5.0)));
        for _ in 0..PLANK_TTL_TICKS {
            pool.step(&[None, None], &[], |_, _| {});
        }
        assert!(pool.is_empty(), "an unhit plank rots");
    }

    #[test]
    fn the_pool_is_fixed_size_and_a_full_pool_replaces_the_oldest_plank() {
        let mut pool = HazardPool::default();
        for i in 0..MAX_HAZARDS {
            assert!(pool.spawn(Hazard::plank(0, Vec2::new(i as f32 * 3.0, 0.0))));
            pool.step(&[None], &[], |_, _| {}); // each one a tick older than the next
        }
        assert_eq!(pool.len(), MAX_HAZARDS);
        assert!(pool.spawn(Hazard::plank(0, Vec2::new(99.0, 0.0))), "full, but the oldest plank makes room");
        assert_eq!(pool.len(), MAX_HAZARDS);
        assert!(pool.iter().any(|h| h.pos.x == 99.0) && pool.iter().all(|h| h.pos.x != 0.0), "and it was the oldest that went");
        let mut acorns = HazardPool::default();
        for i in 0..MAX_HAZARDS {
            acorns.spawn(Hazard::acorn(0, Vec2::new(i as f32 * 3.0, 50.0), Vec2::ZERO));
        }
        assert!(!acorns.spawn(Hazard::plank(0, Vec2::ZERO)), "a pool full of acorns has no plank to replace: the new one is dropped");
    }

    #[test]
    fn a_box_goes_to_the_lowest_slot_that_can_take_it_then_respawns() {
        let mut boxes = ItemBoxes::new(vec![ItemBox { min: Vec2::new(0.0, 0.0), max: Vec2::new(4.0, 4.0) }], 10);
        let karts = [Some(Vec2::new(50.0, 50.0)), Some(Vec2::new(2.0, 2.0)), Some(Vec2::new(3.0, 3.0))];
        let mut grants = Vec::new();
        boxes.step(&karts, |slot| slot != 1, |slot, b| grants.push((slot, b)));
        assert_eq!(grants, vec![(2, 0)], "slot 1 is on it but cannot take (its hands are full): slot 2 does; slot 0 is far away");
        assert!(!boxes.is_ready(0));
        for _ in 0..10 {
            boxes.step(&karts, |_| true, |slot, b| grants.push((slot, b)));
        }
        assert_eq!(grants.len(), 1, "nothing while it respawns");
        assert!(boxes.is_ready(0));
        boxes.step(&karts, |_| true, |slot, b| grants.push((slot, b)));
        assert_eq!(grants[1], (1, 0), "then the lowest slot on it");
        let touching_edge = [Some(Vec2::new(4.0 + KART_RADIUS * 0.9, 2.0))];
        let mut b2 = ItemBoxes::new(vec![ItemBox { min: Vec2::ZERO, max: Vec2::new(4.0, 4.0) }], 10);
        let mut g2 = 0;
        b2.step(&touching_edge, |_| true, |_, _| g2 += 1);
        assert_eq!(g2, 1, "a kart whose body overlaps the box takes it");
    }

    #[test]
    fn the_ready_mask_shows_which_boxes_can_be_taken() {
        let boxes = vec![
            ItemBox { min: Vec2::ZERO, max: Vec2::ONE },
            ItemBox { min: Vec2::new(10.0, 0.0), max: Vec2::new(11.0, 1.0) },
            ItemBox { min: Vec2::new(20.0, 0.0), max: Vec2::new(21.0, 1.0) },
        ];
        let mut b = ItemBoxes::new(boxes, 5);
        assert_eq!(b.ready_mask(), 0b111);
        b.step(&[Some(Vec2::new(10.5, 0.5))], |_| true, |_, _| {});
        assert_eq!(b.ready_mask(), 0b101, "box 1 was taken");
        for _ in 0..5 {
            b.step(&[None], |_| true, |_, _| {});
        }
        assert_eq!(b.ready_mask(), 0b111, "and is back");
    }

    #[test]
    fn rolls_are_deterministic_weighted_by_place_and_use_every_item() {
        assert_eq!(roll_item(100, 2, 1, 3, 8), roll_item(100, 2, 1, 3, 8));
        let count = |place: usize| {
            let mut c = [0u32; 3];
            for tick in 0..6000u64 {
                match roll_item(tick, 0, 0, place, 8) {
                    Item::Mushroom => c[0] += 1,
                    Item::Acorn => c[1] += 1,
                    Item::Bubble => c[2] += 1,
                    Item::None => panic!("a box always gives something"),
                }
            }
            c
        };
        let (leader, tail) = (count(1), count(8));
        assert!(leader.iter().all(|n| *n > 500) && tail.iter().all(|n| *n > 500), "every item comes up: {leader:?} {tail:?}");
        assert!(tail[0] > leader[0] + 1500, "the tail gets Mushrooms far more often: {leader:?} vs {tail:?}");
        assert!(leader[2] > tail[2] + 1500, "the leader gets Bubbles far more often: {leader:?} vs {tail:?}");
        assert_ne!(
            (0..50).map(|t| roll_item(t, 0, 0, 4, 8)).collect::<Vec<_>>(),
            (0..50).map(|t| roll_item(t, 1, 0, 4, 8)).collect::<Vec<_>>(),
            "slots differ"
        );
        assert!(matches!(roll_item(5, 0, 0, 1, 1), Item::Mushroom | Item::Acorn | Item::Bubble));
    }

    #[test]
    fn pool_and_box_checksums_see_changes() {
        let mut a = HazardPool::default();
        let empty = a.checksum();
        a.spawn(Hazard::plank(0, Vec2::new(1.0, 2.0)));
        assert_ne!(a.checksum(), empty);
        let before = a.checksum();
        a.step(&[None], &[], |_, _| {});
        assert_ne!(a.checksum(), before, "age and time left are part of the state");
        let mut boxes = ItemBoxes::new(vec![ItemBox { min: Vec2::ZERO, max: Vec2::ONE }], 5);
        let idle = boxes.checksum();
        boxes.step(&[Some(Vec2::new(0.5, 0.5))], |_| true, |_, _| {});
        assert_ne!(boxes.checksum(), idle);
    }
}
