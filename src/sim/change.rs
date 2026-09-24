//! Generational change tracking: "has this changed since generation N?" answered in O(1).
//!
//! - [`Generation`] is a counter; a [`GenClock`] owns the current one and advances once per tick.
//! - [`Tracked<T>`] wraps a component value with the generation it last changed at.
//! - [`TrackedColumn<T>`] is a dense column of them that also remembers the newest change anywhere in
//!   it, so a quiet column answers "anything changed?" without looking at a single slot.
//! - [`ChangeCursor`] is what a *system* keeps: the generation it last processed up to.
//!
//! **The protocol** (what makes "same generation" writes safe):
//! 1. writers stamp with `clock.now()`;
//! 2. a reader (render sync, network delta, ...) asks `changed_since(cursor.last())`, handles what it
//!    finds, then calls [`ChangeCursor::catch_up`], which records `now` **and closes that generation**
//!    (advances the clock). Any later write is therefore strictly newer than what the reader has seen;
//!    without that, a write stamped in the same generation as the read would be invisible next time.
//!
//! Readers need not be tick-aligned (the renderer catches up once per frame); the clock is just a
//! counter and advancing it more often is always safe.
//!
//! Nothing here is wired into networking yet; it is a tested primitive (ADR 0014, step 3).
//!
//! ```
//! use red_engine2::sim::change::{ChangeCursor, GenClock, TrackedColumn};
//! use red_engine2::sim::components::Health;
//! let mut clock = GenClock::default();
//! let mut health: TrackedColumn<Health> = TrackedColumn::default();
//! let a = health.push(Health::full(100), clock.now());
//! let mut net = ChangeCursor::default();
//!
//! assert!(health.changed_since(net.last()));           // freshly created counts as changed
//! net.catch_up(&mut clock);                            // ... the network system has now seen it
//! assert!(!health.changed_since(net.last()));          // quiet: O(1) "no"
//!
//! health.set(a, Health { current: 60, max: 100 }, clock.now());
//! assert!(health.changed_since(net.last()));
//! ```

/// A monotonically increasing change counter. `0` means "never changed" (older than any write).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(pub u64);

/// Owns the current generation; the sim advances it once per tick (see the module docs).
#[derive(Debug, Clone, Copy)]
pub struct GenClock {
    now: Generation,
}

impl Default for GenClock {
    /// Starts at generation 1, so a default `Generation(0)` cursor sees everything ever written.
    fn default() -> Self {
        GenClock { now: Generation(1) }
    }
}

impl GenClock {
    /// The generation writes are stamped with right now.
    pub fn now(&self) -> Generation {
        self.now
    }

    /// Ends the current generation; later writes are strictly newer than anything read so far.
    /// [`ChangeCursor::catch_up`] calls this for you.
    pub fn advance(&mut self) -> Generation {
        self.now.0 += 1;
        self.now
    }
}

/// What one system remembers: the newest generation it has fully processed.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChangeCursor {
    last: Generation,
}

impl ChangeCursor {
    /// Ask components "changed since this?".
    pub fn last(&self) -> Generation {
        self.last
    }

    /// Records that everything stamped up to and including `clock.now()` has been processed, then
    /// closes that generation so later writes are strictly newer. Call it after reading.
    pub fn catch_up(&mut self, clock: &mut GenClock) {
        self.last = clock.now();
        clock.advance();
    }
}

/// A component value plus the generation it last changed at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tracked<T> {
    value: T,
    changed: Generation,
}

impl<T> Tracked<T> {
    /// A new value, stamped as changed at `now`.
    pub fn new(value: T, now: Generation) -> Self {
        Tracked { value, changed: now }
    }

    /// Reads the value (no stamp).
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Mutable access; conservatively stamps the value as changed at `now` (you asked to change it).
    pub fn get_mut(&mut self, now: Generation) -> &mut T {
        self.changed = now;
        &mut self.value
    }

    /// Generation of the last change.
    pub fn changed_at(&self) -> Generation {
        self.changed
    }

    /// Whether it changed after generation `since`.
    pub fn changed_since(&self, since: Generation) -> bool {
        self.changed > since
    }
}

impl<T: PartialEq> Tracked<T> {
    /// Stores `value`, stamping only if it differs from what is there. Returns whether it changed.
    pub fn set(&mut self, value: T, now: Generation) -> bool {
        if self.value == value {
            return false;
        }
        self.value = value;
        self.changed = now;
        true
    }
}

/// A dense column of tracked components (one per entity slot) that also remembers the newest change
/// in the whole column.
#[derive(Debug, Clone)]
pub struct TrackedColumn<T> {
    items: Vec<Tracked<T>>,
    newest: Generation,
}

impl<T> Default for TrackedColumn<T> {
    fn default() -> Self {
        TrackedColumn { items: Vec::new(), newest: Generation(0) }
    }
}

impl<T> TrackedColumn<T> {
    /// Adds a value (stamped changed at `now`); returns its slot.
    pub fn push(&mut self, value: T, now: Generation) -> usize {
        self.items.push(Tracked::new(value, now));
        self.newest = self.newest.max(now);
        self.items.len() - 1
    }

    /// Number of slots.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the column has no slots.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Reads a slot.
    pub fn get(&self, slot: usize) -> &T {
        self.items[slot].get()
    }

    /// Mutable access to a slot; stamps it (and the column) as changed at `now`.
    pub fn get_mut(&mut self, slot: usize, now: Generation) -> &mut T {
        self.newest = self.newest.max(now);
        self.items[slot].get_mut(now)
    }

    /// Overwrites a slot with a new value that replaces it wholesale, stamping regardless of equality
    /// (use for slot reuse: a recycled slot is "changed" even if the value happens to match).
    pub fn replace(&mut self, slot: usize, value: T, now: Generation) {
        self.items[slot] = Tracked::new(value, now);
        self.newest = self.newest.max(now);
    }

    /// Generation of the newest change anywhere in the column.
    pub fn newest(&self) -> Generation {
        self.newest
    }

    /// O(1): did *anything* in the column change after `since`? A quiet column never scans.
    pub fn changed_since(&self, since: Generation) -> bool {
        self.newest > since
    }

    /// O(1): did this slot change after `since`?
    pub fn slot_changed_since(&self, slot: usize, since: Generation) -> bool {
        self.items[slot].changed_since(since)
    }

    /// The `(slot, value)` pairs that changed after `since`, in slot order. A quiet column yields
    /// nothing without visiting a slot.
    pub fn iter_changed_since(&self, since: Generation) -> impl Iterator<Item = (usize, &T)> + '_ {
        let scan: &[Tracked<T>] = if self.changed_since(since) { &self.items } else { &[] };
        scan.iter().enumerate().filter(move |(_, t)| t.changed_since(since)).map(|(i, t)| (i, t.get()))
    }

    /// Appends the slots that changed after `since` to `out` (in slot order). Costs nothing when the
    /// whole column is quiet; otherwise one linear pass. Pass a reusable buffer (`sim::scratch`).
    pub fn collect_changed_since(&self, since: Generation, out: &mut Vec<usize>) {
        if !self.changed_since(since) {
            return;
        }
        out.extend(self.items.iter().enumerate().filter(|(_, t)| t.changed_since(since)).map(|(i, _)| i));
    }
}

impl<T: PartialEq> TrackedColumn<T> {
    /// Stores `value` in `slot`, stamping only if it differs. Returns whether it changed.
    pub fn set(&mut self, slot: usize, value: T, now: Generation) -> bool {
        let changed = self.items[slot].set(value, now);
        if changed {
            self.newest = self.newest.max(now);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::components::{Health, Transform};
    use glam::Vec3;

    #[test]
    fn set_stamps_only_real_changes() {
        let mut clock = GenClock::default();
        let mut t = Tracked::new(5, clock.now());
        clock.advance();
        assert!(!t.set(5, clock.now()), "writing the same value is not a change");
        assert_eq!(t.changed_at(), Generation(1));
        assert!(t.set(6, clock.now()));
        assert_eq!(t.changed_at(), Generation(2));
        assert!(t.changed_since(Generation(1)));
        assert!(!t.changed_since(Generation(2)));
    }

    #[test]
    fn same_generation_write_after_a_reader_is_not_lost() {
        // The bug this protocol prevents: writer stamps G, reader records "seen up to G", writer stamps G
        // again -> looks unchanged. `catch_up` closing the generation fixes it, with no explicit advance.
        let mut clock = GenClock::default();
        let mut col: TrackedColumn<i32> = TrackedColumn::default();
        let slot = col.push(1, clock.now());
        let mut cursor = ChangeCursor::default();
        assert!(col.changed_since(cursor.last()));
        cursor.catch_up(&mut clock);
        col.set(slot, 2, clock.now());
        assert!(col.changed_since(cursor.last()), "a write right after a read must be seen next time");
        cursor.catch_up(&mut clock);
        assert!(!col.changed_since(cursor.last()));
    }

    #[test]
    fn independent_cursors_each_see_every_change_once() {
        let mut clock = GenClock::default();
        let mut col: TrackedColumn<i32> = TrackedColumn::default();
        let slot = col.push(0, clock.now());
        let (mut net, mut audio) = (ChangeCursor::default(), ChangeCursor::default());
        net.catch_up(&mut clock);
        col.set(slot, 1, clock.now());
        // The audio system was not running that tick; it still sees both changes' effects.
        assert!(col.changed_since(net.last()) && col.changed_since(audio.last()));
        net.catch_up(&mut clock);
        assert!(!col.changed_since(net.last()));
        assert!(col.changed_since(audio.last()), "a slower reader is unaffected by a faster one");
        audio.catch_up(&mut clock);
        assert!(!col.changed_since(audio.last()));
    }

    #[test]
    fn column_lists_only_changed_slots_and_is_free_when_quiet() {
        let mut clock = GenClock::default();
        let mut col: TrackedColumn<Transform> = TrackedColumn::default();
        for i in 0..100 {
            col.push(Transform::at(Vec3::new(i as f32, 0.0, 0.0)), clock.now());
        }
        let mut cursor = ChangeCursor::default();
        cursor.catch_up(&mut clock);

        let mut out = Vec::new();
        col.collect_changed_since(cursor.last(), &mut out);
        assert!(out.is_empty(), "quiet column");

        col.get_mut(7, clock.now()).position.y = 1.0;
        col.set(42, Transform::at(Vec3::new(9.0, 9.0, 9.0)), clock.now());
        col.set(43, *col.get(43), clock.now()); // same value: no change
        col.collect_changed_since(cursor.last(), &mut out);
        assert_eq!(out, vec![7, 42]);
        assert!(col.slot_changed_since(7, cursor.last()) && !col.slot_changed_since(8, cursor.last()));
    }

    #[test]
    fn health_tracks_damage() {
        let mut clock = GenClock::default();
        let mut col: TrackedColumn<Health> = TrackedColumn::default();
        let a = col.push(Health::full(100), clock.now());
        let mut cursor = ChangeCursor::default();
        cursor.catch_up(&mut clock);
        let h = col.get(a).damaged(30);
        assert!(col.set(a, h, clock.now()));
        assert_eq!(col.get(a).current, 70);
        assert!(col.changed_since(cursor.last()));
    }

    #[test]
    fn replacing_a_slot_stamps_it_even_if_equal() {
        let mut clock = GenClock::default();
        let mut col: TrackedColumn<i32> = TrackedColumn::default();
        let s = col.push(1, clock.now());
        let mut cursor = ChangeCursor::default();
        cursor.catch_up(&mut clock);
        col.replace(s, 1, clock.now());
        assert!(col.slot_changed_since(s, cursor.last()));
    }
}
