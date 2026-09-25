//! Which scene objects are loose props, and what a character can lift: [`classify`], [`PropShape`], [`CarryLimits`].
//!
//! Pure data and rules over the scene (no physics world needed): the server, a client mapping snapshot ids to objects and
//! the tools all use this to agree on what "loose" means.

use crate::hit::object_leaves;
use crate::schema::{Object, ObjectKind};
use crate::track::Track;
use glam::{Mat4, Vec3};

/// What a character can pick up: the longest side of the prop's bounding box and its volume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CarryLimits {
    /// Longest bounding-box side, m.
    pub max_dim: f32,
    /// Bounding-box volume, m^3.
    pub max_volume: f32,
}

/// A person carries a chair, crate, barrel, potted plant, TV — not a desk, sofa or vending machine.
pub const HUMAN_CARRY: CarryLimits = CarryLimits { max_dim: 1.25, max_volume: 0.45 };
/// A rat carries an apple, a mug, a book — things about the size of the rat's own head.
pub const RAT_CARRY: CarryLimits = CarryLimits { max_dim: 0.34, max_volume: 0.015 };

/// Prop kinds that are part of the building or landscape even when small enough to lift.
const FIXTURE_PROPS: &[&str] = &[
    "kitchen_counter",
    "refrigerator",
    "stove",
    "sink",
    "toilet",
    "bathtub",
    "washer_dryer",
    "mailbox",
    "fence_section",
    "rug",
    "tree_oak",
    "tree_pine",
    "bush",
    "flower_patch",
    "hedge",
    "boulder",
    "grill",
    "picnic_table",
    "bench",
    "vending_machine",
    "filing_cabinet",
    "bookshelf",
    "wardrobe",
    "bed",
    "sofa",
];

/// Bounding box of a loose-prop candidate, in the object's own frame with its scale applied.
#[derive(Debug, Clone, Copy)]
pub struct PropShape {
    /// Box side lengths, m.
    pub extents: Vec3,
    /// Box centre relative to the object's origin, m.
    pub center: Vec3,
}

impl PropShape {
    /// Bounding-box volume, m^3.
    pub fn volume(&self) -> f32 {
        self.extents.x * self.extents.y * self.extents.z
    }

    /// Whether `limits` allows lifting this.
    pub fn carriable(&self, limits: &CarryLimits) -> bool {
        self.extents.max_element() <= limits.max_dim && self.volume() <= limits.max_volume
    }
}

pub(super) fn object_scale(o: &Object) -> Vec3 {
    o.scale.sample(0.0)
}

/// Bounds of every leaf of `o` in its own frame with its scale applied, or `None` if it has none.
pub fn local_bounds(o: &Object) -> Option<PropShape> {
    let s = Mat4::from_scale(object_scale(o));
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for (shape, local) in object_leaves(o) {
        let m = s * local;
        let half = shape.half_extent();
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                for sz in [-1.0f32, 1.0] {
                    let p = m.transform_point3(Vec3::new(half.x * sx, half.y * sy, half.z * sz));
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
            }
        }
    }
    (lo.x.is_finite()).then(|| PropShape { extents: (hi - lo).max(Vec3::splat(0.005)), center: (lo + hi) * 0.5 })
}

fn is_constant<T: crate::track::Lerp>(t: &Track<T>) -> bool {
    matches!(t, Track::Constant(_))
}

/// Decides whether a top-level object is a loose prop a *human* could lift, returning its bounds
/// if so. See the module docs for the rules; `"movable"` overrides the guess.
pub fn classify(o: &Object) -> Option<PropShape> {
    if o.movable == Some(false) || !(is_constant(&o.position) && is_constant(&o.rotation) && is_constant(&o.scale)) {
        return None;
    }
    let looks_loose = match &o.kind {
        ObjectKind::Prop(p) => !FIXTURE_PROPS.contains(&p.kind.name()),
        ObjectKind::Group(_) => o.prefab.as_ref().is_some_and(|t| t.mount == "floor"),
        _ => false,
    };
    if !(looks_loose || o.movable == Some(true)) {
        return None;
    }
    let shape = local_bounds(o)?;
    (o.movable == Some(true) || shape.carriable(&HUMAN_CARRY)).then_some(shape)
}
