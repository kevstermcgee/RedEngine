//! A fixed pool of scene objects, added before the renderer exists and claimed, moved and hidden afterwards (ADR 2026-09-28-seeing-what-the-player-sees).
//!
//! The live renderer takes its meshes from the scene when it is built, so anything that appears later (another player's body, a tracer, a spark) must already
//! be in the scene, hidden. Every such thing used to size, claim and hide its own objects: the avatars, the tracers, the player's own body. That is where "the
//! enemy is invisible" came from (a pool sized by the wrong rule, and the client skipped whoever it had no avatar for). A [`ScenePool`] owns the arrangement in one
//! tested place: which scene objects belong to it, which are claimed, hiding, and counters that turn "the pool was too small" into a number somebody can read.
//!
//! Hiding is two-fold: [`hide_object`] scales an object to nothing (a scene without a live renderer, like the tools, and the tests, sees it as invisible) and
//! [`ScenePool::hidden_ids`] lists the unclaimed objects so `LiveRenderer::set_hidden_objects` can skip their draws entirely (a scaled-down mesh is still a draw call).

use crate::schema::{Object, Scene};
use crate::track::Track;
use glam::Vec3;

/// Scale that makes an object effectively invisible to anything that does not honour the renderer's hidden set.
pub const HIDDEN_SCALE: f32 = 0.0005;

/// Scales `scene.objects[index]` to nothing.
pub fn hide_object(scene: &mut Scene, index: usize) {
    if let Some(o) = scene.objects.get_mut(index) {
        o.scale = Track::constant(Vec3::splat(HIDDEN_SCALE));
    }
}

/// Counters of one pool: what it holds and how it was used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PoolStats {
    /// Objects in the pool.
    pub capacity: usize,
    /// Objects claimed right now.
    pub in_use: usize,
    /// The most that were claimed at once.
    pub high_water: usize,
    /// Successful claims, ever.
    pub claims: u64,
    /// Claims that found the pool empty, ever: each one is something that was asked for and not drawn.
    pub failed_claims: u64,
}

/// `count` consecutive scene objects, some claimed, the rest hidden.
#[derive(Debug, Clone)]
pub struct ScenePool {
    first: usize,
    used: Vec<bool>,
    stats: PoolStats,
}

impl ScenePool {
    /// Appends `count` objects to `scene`, made by `make(k)` for `k` in `0..count` (the caller chooses their ids and looks), all hidden and unclaimed.
    /// Call it before the renderer is built.
    pub fn add(scene: &mut Scene, count: usize, mut make: impl FnMut(usize) -> Object) -> ScenePool {
        let first = scene.objects.len();
        for k in 0..count {
            let mut o = make(k);
            o.scale = Track::constant(Vec3::splat(HIDDEN_SCALE));
            scene.objects.push(o);
        }
        ScenePool { first, used: vec![false; count], stats: PoolStats { capacity: count, ..PoolStats::default() } }
    }

    /// Objects in the pool.
    pub fn capacity(&self) -> usize {
        self.used.len()
    }

    /// Where slot `slot` is in `scene.objects`.
    pub fn object_index(&self, slot: usize) -> usize {
        self.first + slot
    }

    /// Whether slot `slot` is claimed.
    pub fn is_used(&self, slot: usize) -> bool {
        self.used.get(slot).copied().unwrap_or(false)
    }

    /// Claims the first free slot, or counts a failed claim and returns `None`.
    pub fn claim(&mut self) -> Option<usize> {
        match self.used.iter().position(|u| !u) {
            Some(slot) => {
                self.used[slot] = true;
                self.stats.claims += 1;
                self.stats.in_use += 1;
                self.stats.high_water = self.stats.high_water.max(self.stats.in_use);
                Some(slot)
            }
            None => {
                self.stats.failed_claims += 1;
                None
            }
        }
    }

    /// Claims exactly `slot` (a pool whose slots mean something, like one model per driver). `false`, counting a failed claim, if it is taken or out of range.
    pub fn claim_slot(&mut self, slot: usize) -> bool {
        match self.used.get(slot) {
            Some(false) => {
                self.used[slot] = true;
                self.stats.claims += 1;
                self.stats.in_use += 1;
                self.stats.high_water = self.stats.high_water.max(self.stats.in_use);
                true
            }
            _ => {
                self.stats.failed_claims += 1;
                false
            }
        }
    }

    /// Frees slot `slot` and hides its object (a no-op for a slot that is not claimed).
    pub fn release(&mut self, scene: &mut Scene, slot: usize) {
        if self.is_used(slot) {
            self.used[slot] = false;
            self.stats.in_use -= 1;
            hide_object(scene, self.object_index(slot));
        }
    }

    /// The ids of the unclaimed objects: what `LiveRenderer::set_hidden_objects` should not draw.
    pub fn hidden_ids<'a>(&'a self, scene: &'a Scene) -> impl Iterator<Item = &'a str> + 'a {
        (0..self.used.len()).filter(|&slot| !self.used[slot]).filter_map(|slot| scene.objects.get(self.first + slot).map(|o| o.id.as_str()))
    }

    /// The counters.
    pub fn stats(&self) -> PoolStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{ObjectKind, PrimKind};

    fn scene() -> Scene {
        crate::schema::parse_scene(r##"{"camera":{"position":[0,1.7,0],"target":[0,1.7,-10]},"objects":[{"id":"floor","type":"box","size":[10,0.2,10]}]}"##)
            .unwrap_or_else(|e| panic!("{e:?}"))
    }

    fn make(prefix: &str) -> impl FnMut(usize) -> Object + '_ {
        move |k| Object {
            id: format!("{prefix}_{k}"),
            position: Track::constant(Vec3::ZERO),
            rotation: Track::constant(Vec3::ZERO),
            scale: Track::constant(Vec3::ONE),
            material: None,
            collide: false,
            prefab: None,
            movable: Some(false),
            kind: ObjectKind::Prim(PrimKind::Box { size: Vec3::ONE }),
        }
    }

    #[test]
    fn a_pool_appends_hidden_objects_after_the_map() {
        let mut s = scene();
        let pool = ScenePool::add(&mut s, 3, make("p"));
        assert_eq!((pool.capacity(), pool.object_index(0), s.objects.len()), (3, 1, 4));
        assert!(s.objects[1..].iter().all(|o| o.scale.sample(0.0).x < 0.01), "made visible by the caller, hidden by the pool");
        assert_eq!(pool.hidden_ids(&s).collect::<Vec<_>>(), ["p_0", "p_1", "p_2"]);
        assert_eq!(pool.stats(), PoolStats { capacity: 3, ..PoolStats::default() });
    }

    #[test]
    fn claims_take_the_first_free_slot_and_an_empty_pool_counts_the_failure() {
        let mut s = scene();
        let mut pool = ScenePool::add(&mut s, 2, make("p"));
        assert_eq!((pool.claim(), pool.claim()), (Some(0), Some(1)));
        assert_eq!(pool.claim(), None, "nothing left");
        assert_eq!(pool.claim(), None);
        let st = pool.stats();
        assert_eq!((st.in_use, st.high_water, st.claims, st.failed_claims), (2, 2, 2, 2), "{st:?}");
        assert!(pool.hidden_ids(&s).next().is_none(), "every object is claimed, none is hidden");
        pool.release(&mut s, 0);
        assert_eq!(pool.hidden_ids(&s).collect::<Vec<_>>(), ["p_0"]);
        assert_eq!(pool.claim(), Some(0), "a released slot is reused first");
        assert_eq!(pool.stats().high_water, 2);
    }

    #[test]
    fn releasing_hides_the_object_and_only_a_claimed_slot_can_be_released() {
        let mut s = scene();
        let mut pool = ScenePool::add(&mut s, 2, make("p"));
        let slot = pool.claim().unwrap();
        s.objects[pool.object_index(slot)].scale = Track::constant(Vec3::ONE); // the caller shows it
        pool.release(&mut s, slot);
        assert!(s.objects[pool.object_index(slot)].scale.sample(0.0).x < 0.01);
        assert_eq!(pool.stats().in_use, 0);
        pool.release(&mut s, slot); // again: nothing happens, the count does not underflow
        pool.release(&mut s, 99);
        assert_eq!(pool.stats().in_use, 0);
        assert!(!pool.is_used(0) && !pool.is_used(99));
    }

    #[test]
    fn a_specific_slot_can_be_claimed_once() {
        let mut s = scene();
        let mut pool = ScenePool::add(&mut s, 3, make("p"));
        assert!(pool.claim_slot(2) && pool.is_used(2) && !pool.is_used(0));
        assert!(!pool.claim_slot(2), "already taken");
        assert!(!pool.claim_slot(9), "out of range");
        assert_eq!(pool.claim(), Some(0), "the sequential claim still finds the first free slot");
        let stats = pool.stats();
        assert_eq!((stats.in_use, stats.claims, stats.failed_claims), (2, 2, 2));
        pool.release(&mut s, 2);
        assert!(!pool.is_used(2));
    }
}
