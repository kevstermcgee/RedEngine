//! Tick-scoped scratch buffers: reusable `Vec`s that are *reset*, not freed, so a hot path that
//! needs a temporary list every tick (query results, collision contacts, network delta events)
//! allocates only while it is still growing to its high-water mark, then never again.
//!
//! ```
//! use red_engine2::sim::scratch::ScratchVec;
//! let mut hits: ScratchVec<u32> = ScratchVec::with_capacity(16);
//! for _tick in 0..100 {
//!     let mut buf = hits.take();          // empty, capacity kept
//!     buf.extend([1, 2, 3]);              // ... fill it, read it ...
//!     hits.give_back(buf);                // capacity stays for the next tick
//! }
//! assert_eq!(hits.grows(), 0);            // never had to grow past the 16 it started with
//! ```
//!
//! `take`/`give_back` (instead of a `&mut Vec` accessor) lets the owner keep using `&mut self` while
//! it fills the buffer, which is what a physics world needs (`for x in &buf { self.mutate(x) }`).

/// A reusable buffer plus counters an objective test can assert on.
#[derive(Debug, Default)]
pub struct ScratchVec<T> {
    buf: Vec<T>,
    high_water: usize,
    grows: u32,
    capacity_at_take: usize,
}

impl<T> ScratchVec<T> {
    /// A buffer that starts with room for `n` items (so a known worst case never reallocates).
    pub fn with_capacity(n: usize) -> Self {
        ScratchVec { buf: Vec::with_capacity(n), high_water: 0, grows: 0, capacity_at_take: n }
    }

    /// Hands out the buffer, emptied. Pair with [`give_back`](Self::give_back); until then the
    /// scratch holds an empty placeholder (which does not allocate).
    pub fn take(&mut self) -> Vec<T> {
        let mut v = std::mem::take(&mut self.buf);
        v.clear();
        self.capacity_at_take = v.capacity();
        v
    }

    /// Returns the buffer for reuse next tick, recording its size and whether it had to grow.
    pub fn give_back(&mut self, v: Vec<T>) {
        self.high_water = self.high_water.max(v.len());
        if v.capacity() > self.capacity_at_take {
            self.grows += 1;
        }
        self.buf = v;
    }

    /// Largest number of items any single use has held.
    pub fn high_water(&self) -> usize {
        self.high_water
    }

    /// How many times a use outgrew the buffer's capacity (each is one heap allocation). Zero in
    /// steady state; an objective "did this stop allocating?" number.
    pub fn grows(&self) -> u32 {
        self.grows
    }

    /// Current capacity, items.
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuse_keeps_capacity_and_counts_growth() {
        let mut s: ScratchVec<u32> = ScratchVec::default();
        // Warm-up: the first big use grows it.
        let mut v = s.take();
        v.extend(0..100);
        s.give_back(v);
        assert_eq!(s.grows(), 1);
        assert_eq!(s.high_water(), 100);
        let cap = s.capacity();
        assert!(cap >= 100);
        // Steady state: smaller uses never grow it and never shrink it.
        for n in [10, 100, 50] {
            let mut v = s.take();
            assert!(v.is_empty(), "take() must hand out an empty buffer");
            v.extend(0..n);
            s.give_back(v);
        }
        assert_eq!(s.grows(), 1);
        assert_eq!(s.capacity(), cap);
    }

    #[test]
    fn preallocated_capacity_never_grows_within_it() {
        let mut s: ScratchVec<u8> = ScratchVec::with_capacity(32);
        let mut v = s.take();
        v.extend(std::iter::repeat_n(0u8, 32));
        s.give_back(v);
        assert_eq!(s.grows(), 0);
    }
}
