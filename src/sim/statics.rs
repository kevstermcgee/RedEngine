//! Static prop instances and their promotion to dynamic entities.
//!
//! Most props in a Prop Hunt map (cans, books, chairs, store stock) are never touched in a whole
//! match. A prop therefore starts as a [`StaticInstance`]: its authored pose plus fixed colliders in
//! the physics world (so it is solid, and a bat or bullet can hit it) — **no rigid body, no entity
//! (no tracked transform), nothing for a network layer to replicate**. The first time something
//! interacts with it (grabbed, shot, struck, walked into, something on it moves) it is *promoted*:
//! its colliders are re-attached to a new dynamic rigid body and it gets an [`EntityId`] with a
//! tracked transform (`PropWorld::activate`, which also promotes what rests against it).
//!
//! Promotion is one-way here (a settled prop stays dynamic-but-asleep, which costs nothing per tick).
//!
//! Cost model, measured by `cargo bench --bench sim` (ADR 0014): a static prop costs one
//! [`StaticInstance`] plus its colliders; promotion costs a body, an entity slot and re-inserting the
//! colliders — a few microseconds, paid only by props that are actually touched.

use crate::sim::entities::EntityId;
use rapier3d::prelude::RigidBodyHandle;

/// A prop nobody has touched: just which colliders (a range into the world's flat collider pool)
/// make it solid. Its authored pose lives on the prop itself.
#[derive(Debug, Clone, Copy)]
pub struct StaticInstance {
    /// Index of the first of this prop's colliders in the world's collider pool.
    pub collider_start: u32,
    /// How many colliders it has (one per leaf shape).
    pub collider_count: u32,
}

impl StaticInstance {
    /// The prop's collider slots in the pool.
    pub fn collider_range(&self) -> std::ops::Range<usize> {
        self.collider_start as usize..(self.collider_start + self.collider_count) as usize
    }
}

/// The full-fat form a prop takes once promoted.
#[derive(Debug, Clone, Copy)]
pub struct DynamicProp {
    /// Its rigid body.
    pub body: RigidBodyHandle,
    /// Its entity (tracked transform).
    pub entity: EntityId,
    /// Whether its final resting pose has been published to the entity after it fell asleep (so a
    /// sleeping prop costs nothing per tick, yet its last pose is never lost).
    pub settled: bool,
}

/// What a loose prop currently is.
#[derive(Debug, Clone, Copy)]
pub enum PropState {
    /// Untouched: data and fixed colliders only.
    Static(StaticInstance),
    /// Promoted: rigid body + entity.
    Dynamic(DynamicProp),
}
