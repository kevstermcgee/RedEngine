//! The store of *dynamic* simulation entities: things that move, so they are worth a tracked
//! [`Transform`], a rigid body and (later) a place in network snapshots. A prop nobody has touched
//! is **not** an entity (see [`crate::sim::statics`]); it becomes one when promoted.
//!
//! Append-only for now: nothing can die yet (loose props never return to static). When players or
//! projectiles need despawning, add a free list and a per-slot generation to [`EntityId`] here.

use crate::sim::change::{Generation, TrackedColumn};
use crate::sim::components::Transform;

/// Handle of a dynamic entity (its slot in the columns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(pub u32);

impl EntityId {
    /// The slot index, for indexing columns.
    pub fn slot(self) -> usize {
        self.0 as usize
    }
}

/// Dynamic entities and their tracked components.
#[derive(Debug, Default)]
pub struct Entities {
    /// Every entity's pose, with change tracking (`changed_since`).
    pub transforms: TrackedColumn<Transform>,
}

impl Entities {
    /// Creates an entity at `transform`, stamped as changed at `now`.
    pub fn spawn(&mut self, transform: Transform, now: Generation) -> EntityId {
        EntityId(self.transforms.push(transform, now) as u32)
    }

    /// Number of entities.
    pub fn len(&self) -> usize {
        self.transforms.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.transforms.is_empty()
    }
}
