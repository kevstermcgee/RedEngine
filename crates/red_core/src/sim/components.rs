//! Plain-data components for entities that exist in the simulation (as opposed to static prop
//! instances, which have none — see `sim::statics`). Wrap them in [`crate::sim::change::Tracked`] /
//! `TrackedColumn` to get change detection.

use glam::{Quat, Vec3};

/// Where an entity is and how it is turned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// World position, m.
    pub position: Vec3,
    /// World rotation.
    pub rotation: Quat,
}

impl Transform {
    /// At `position`, unrotated.
    pub fn at(position: Vec3) -> Self {
        Transform { position, rotation: Quat::IDENTITY }
    }
}

/// Hit points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Health {
    /// Current hit points (never below 0).
    pub current: u32,
    /// Maximum hit points.
    pub max: u32,
}

impl Health {
    /// Full health of `max`.
    pub fn full(max: u32) -> Self {
        Health { current: max, max }
    }

    /// The health after taking `amount` damage (floored at 0).
    pub fn damaged(self, amount: u32) -> Self {
        Health { current: self.current.saturating_sub(amount), ..self }
    }

    /// Whether no hit points are left.
    pub fn is_dead(self) -> bool {
        self.current == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_floors_at_zero() {
        let h = Health::full(10).damaged(4);
        assert_eq!(h.current, 6);
        assert!(Health::full(10).damaged(99).is_dead());
    }
}
