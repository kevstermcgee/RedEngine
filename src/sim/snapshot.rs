//! Snapshot encoding: dynamic entities to bytes, full or as a delta of what changed.
//!
//! **A placeholder, not a network protocol** — there is no networking yet and this layout is
//! unversioned. It exists so the cost of encoding a snapshot has a number (`cargo bench --bench sim`,
//! `snapshot/*`) and so change tracking (`sim::change`) has a real consumer to be measured against.
//! Encoding appends to a caller-owned buffer, so a steady-state encode allocates nothing.
//!
//! Layout (little endian): `tick: u64`, `count: u32`, then per entity `id: u32`, position `3 x f32`,
//! rotation `4 x f32` ([`ENTITY_BYTES`] = 32 bytes).

use crate::sim::change::Generation;
use crate::sim::components::Transform;
use crate::sim::entities::Entities;

/// Bytes of the header (`tick` + `count`).
pub const HEADER_BYTES: usize = 12;
/// Bytes per encoded entity.
pub const ENTITY_BYTES: usize = 32;

fn put_header(out: &mut Vec<u8>, tick: u64, count: u32) {
    out.extend_from_slice(&tick.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
}

fn put_entity(out: &mut Vec<u8>, slot: usize, t: &Transform) {
    out.extend_from_slice(&(slot as u32).to_le_bytes());
    for v in t.position.to_array().into_iter().chain(t.rotation.to_array()) {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Appends a snapshot of every entity to `out` (which is *not* cleared). Returns how many were encoded.
pub fn encode_full(entities: &Entities, tick: u64, out: &mut Vec<u8>) -> usize {
    let n = entities.len();
    out.reserve(HEADER_BYTES + n * ENTITY_BYTES);
    put_header(out, tick, n as u32);
    for slot in 0..n {
        put_entity(out, slot, entities.transforms.get(slot));
    }
    n
}

/// Appends a snapshot of only the entities whose transform changed after `since`. A quiet world
/// encodes just the header without visiting a single entity (the column's O(1) `changed_since`).
/// Returns how many were encoded.
pub fn encode_delta(entities: &Entities, since: Generation, tick: u64, out: &mut Vec<u8>) -> usize {
    let header_at = out.len();
    put_header(out, tick, 0);
    let mut count = 0u32;
    for (slot, t) in entities.transforms.iter_changed_since(since) {
        put_entity(out, slot, t);
        count += 1;
    }
    out[header_at + 8..header_at + 12].copy_from_slice(&count.to_le_bytes());
    count as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::change::{ChangeCursor, GenClock};
    use glam::{Quat, Vec3};

    fn decode(bytes: &[u8]) -> (u64, Vec<(u32, Transform)>) {
        let tick = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        assert_eq!(bytes.len(), HEADER_BYTES + count * ENTITY_BYTES, "length matches the header's count");
        let f = |b: &[u8]| f32::from_le_bytes(b.try_into().unwrap());
        let items = (0..count)
            .map(|i| {
                let e = &bytes[HEADER_BYTES + i * ENTITY_BYTES..][..ENTITY_BYTES];
                let id = u32::from_le_bytes(e[0..4].try_into().unwrap());
                let v = |k: usize| f(&e[4 + k * 4..8 + k * 4]);
                (id, Transform { position: Vec3::new(v(0), v(1), v(2)), rotation: Quat::from_xyzw(v(3), v(4), v(5), v(6)) })
            })
            .collect();
        (tick, items)
    }

    fn world(n: usize, clock: &GenClock) -> Entities {
        let mut e = Entities::default();
        for i in 0..n {
            e.spawn(Transform { position: Vec3::new(i as f32, 1.0, -2.0), rotation: Quat::from_rotation_y(i as f32 * 0.1) }, clock.now());
        }
        e
    }

    #[test]
    fn full_snapshot_round_trips() {
        let clock = GenClock::default();
        let e = world(5, &clock);
        let mut buf = Vec::new();
        assert_eq!(encode_full(&e, 42, &mut buf), 5);
        let (tick, items) = decode(&buf);
        assert_eq!(tick, 42);
        assert_eq!(items.len(), 5);
        for (i, (id, t)) in items.iter().enumerate() {
            assert_eq!(*id as usize, i);
            assert_eq!(t, e.transforms.get(i));
        }
    }

    #[test]
    fn delta_carries_only_what_changed_and_is_empty_when_quiet() {
        let mut clock = GenClock::default();
        let mut e = world(100, &clock);
        let mut net = ChangeCursor::default();

        let mut buf = Vec::new();
        assert_eq!(encode_delta(&e, net.last(), 1, &mut buf), 100, "everything is new to a fresh cursor");
        net.catch_up(&mut clock);

        buf.clear();
        assert_eq!(encode_delta(&e, net.last(), 2, &mut buf), 0);
        assert_eq!(buf.len(), HEADER_BYTES, "a quiet world is just a header");

        e.transforms.get_mut(7, clock.now()).position.y = 9.0;
        e.transforms.get_mut(93, clock.now()).position.y = 9.0;
        buf.clear();
        assert_eq!(encode_delta(&e, net.last(), 3, &mut buf), 2);
        let (_, items) = decode(&buf);
        assert_eq!(items.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![7, 93]);
        assert_eq!(items[0].1.position.y, 9.0);
    }

    #[test]
    fn steady_state_encoding_reuses_the_buffer() {
        let clock = GenClock::default();
        let e = world(64, &clock);
        let mut buf = Vec::new();
        encode_full(&e, 0, &mut buf);
        let cap = buf.capacity();
        for tick in 1..50 {
            buf.clear();
            encode_full(&e, tick, &mut buf);
        }
        assert_eq!(buf.capacity(), cap, "no regrowth once warm");
    }
}
