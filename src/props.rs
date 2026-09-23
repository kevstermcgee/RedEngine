//! Prop-hunt prop library.
//!
//! Each [`PropKind`] expands into a handful of primitive parts placed at fixed local
//! transforms — the same "primitive-composed" technique already used for `humanoid`'s bone
//! rig (see `skeleton::pose_to_parts`) and the crowbar viewmodel (see
//! `viewer::build_crowbar_mesh`), just authored as a schema-level object kind
//! (`schema::ObjectKind::Prop`) instead of a Rust-only viewmodel helper, so a map author
//! places one `"type": "prop"` object instead of hand-nesting a dozen boxes every time (the
//! same reasoning `humanoid` was given over hand-authoring a capsule rig per instance).
//!
//! [`prop_parts`] is deliberately cheap (no mesh generation, just shapes + transforms) so it
//! can be called every frame for object transforms as well as once at startup for mesh
//! upload — mirroring how `pose_to_parts` is called every frame while the actual
//! `uv_sphere`/`capsule` mesh generation for a humanoid only happens once.

use crate::schema::PrimKind;
use glam::{Mat4, Quat, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    Crate,
    Barrel,
    TrafficCone,
    BoxStack,
    Chair,
    TrashCan,
    VendingMachine,
    Bench,
    FireExtinguisher,
    FilingCabinet,
    PottedPlant,
    Bookshelf,
}

impl PropKind {
    pub const ALL: [PropKind; 12] = [
        PropKind::Crate,
        PropKind::Barrel,
        PropKind::TrafficCone,
        PropKind::BoxStack,
        PropKind::Chair,
        PropKind::TrashCan,
        PropKind::VendingMachine,
        PropKind::Bench,
        PropKind::FireExtinguisher,
        PropKind::FilingCabinet,
        PropKind::PottedPlant,
        PropKind::Bookshelf,
    ];

    /// The `"prop"` string a scene JSON uses to name this kind.
    pub fn name(self) -> &'static str {
        match self {
            PropKind::Crate => "crate",
            PropKind::Barrel => "barrel",
            PropKind::TrafficCone => "traffic_cone",
            PropKind::BoxStack => "box_stack",
            PropKind::Chair => "chair",
            PropKind::TrashCan => "trash_can",
            PropKind::VendingMachine => "vending_machine",
            PropKind::Bench => "bench",
            PropKind::FireExtinguisher => "fire_extinguisher",
            PropKind::FilingCabinet => "filing_cabinet",
            PropKind::PottedPlant => "potted_plant",
            PropKind::Bookshelf => "bookshelf",
        }
    }

    pub fn from_name(name: &str) -> Option<PropKind> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// One primitive part of a composed prop: a shape at a transform relative to the prop's own
/// origin, plus optional small nudges off the prop instance's one shared `Material` (every
/// other object kind carries exactly one material, so a prop does too — these nudges are
/// computed here, not schema-configurable, and exist only because a prop with every part
/// rendered in flat-identical color/finish reads far less convincingly than the real thing:
/// a barrel's rim bands are a bit more metallic than its body, a potted plant's foliage is
/// green regardless of what color the map author gave the pot, and so on).
pub struct PropPart {
    pub shape: PrimKind,
    pub local_transform: Mat4,
    pub metallic_delta: f32,
    pub roughness_delta: f32,
    pub color_override: Option<Vec3>,
}

impl PropPart {
    fn new(shape: PrimKind, pos: Vec3) -> Self {
        PropPart {
            shape,
            local_transform: Mat4::from_translation(pos),
            metallic_delta: 0.0,
            roughness_delta: 0.0,
            color_override: None,
        }
    }

    fn rotated(shape: PrimKind, pos: Vec3, rot_deg: Vec3) -> Self {
        let rot = Quat::from_euler(
            glam::EulerRot::XYZ,
            rot_deg.x.to_radians(),
            rot_deg.y.to_radians(),
            rot_deg.z.to_radians(),
        );
        PropPart {
            shape,
            local_transform: Mat4::from_rotation_translation(rot, pos),
            metallic_delta: 0.0,
            roughness_delta: 0.0,
            color_override: None,
        }
    }

    fn metallic(mut self, delta: f32) -> Self {
        self.metallic_delta = delta;
        self
    }

    fn roughness(mut self, delta: f32) -> Self {
        self.roughness_delta = delta;
        self
    }

    fn color(mut self, c: Vec3) -> Self {
        self.color_override = Some(c);
        self
    }
}

fn b(x: f32, y: f32, z: f32) -> PrimKind {
    PrimKind::Box { size: Vec3::new(x, y, z) }
}

fn cyl(radius: f32, height: f32) -> PrimKind {
    PrimKind::Cylinder { radius, height }
}

pub fn prop_parts(kind: PropKind) -> Vec<PropPart> {
    match kind {
        PropKind::Crate => crate_parts(),
        PropKind::Barrel => barrel_parts(),
        PropKind::TrafficCone => traffic_cone_parts(),
        PropKind::BoxStack => box_stack_parts(),
        PropKind::Chair => chair_parts(),
        PropKind::TrashCan => trash_can_parts(),
        PropKind::VendingMachine => vending_machine_parts(),
        PropKind::Bench => bench_parts(),
        PropKind::FireExtinguisher => fire_extinguisher_parts(),
        PropKind::FilingCabinet => filing_cabinet_parts(),
        PropKind::PottedPlant => potted_plant_parts(),
        PropKind::Bookshelf => bookshelf_parts(),
    }
}

/// A wooden crate: body, four corner posts standing a little proud of the top/bottom faces,
/// and a metal strap belt around the middle.
fn crate_parts() -> Vec<PropPart> {
    let body = b(0.5, 0.5, 0.5);
    let post = b(0.06, 0.56, 0.06);
    let belt_x = b(0.54, 0.06, 0.06);
    let belt_z = b(0.06, 0.06, 0.54);
    vec![
        PropPart::new(body, Vec3::ZERO),
        PropPart::new(post, Vec3::new(0.22, 0.0, 0.22)),
        PropPart::new(post, Vec3::new(-0.22, 0.0, 0.22)),
        PropPart::new(post, Vec3::new(0.22, 0.0, -0.22)),
        PropPart::new(post, Vec3::new(-0.22, 0.0, -0.22)),
        PropPart::new(belt_x, Vec3::new(0.0, 0.0, 0.25)).metallic(0.2).roughness(-0.1),
        PropPart::new(belt_x, Vec3::new(0.0, 0.0, -0.25)).metallic(0.2).roughness(-0.1),
        PropPart::new(belt_z, Vec3::new(0.25, 0.0, 0.0)).metallic(0.2).roughness(-0.1),
        PropPart::new(belt_z, Vec3::new(-0.25, 0.0, 0.0)).metallic(0.2).roughness(-0.1),
    ]
}

/// A barrel: cylindrical body plus three raised rim bands, shinier/more metallic than the
/// body like a real steel drum's rolled hoops.
fn barrel_parts() -> Vec<PropPart> {
    let body = cyl(0.28, 0.85);
    let band = cyl(0.30, 0.05);
    vec![
        PropPart::new(body, Vec3::ZERO),
        PropPart::new(band, Vec3::new(0.0, 0.32, 0.0)).metallic(0.35).roughness(-0.25),
        PropPart::new(band, Vec3::new(0.0, 0.0, 0.0)).metallic(0.35).roughness(-0.25),
        PropPart::new(band, Vec3::new(0.0, -0.32, 0.0)).metallic(0.35).roughness(-0.25),
    ]
}

/// A traffic cone on its flat base plate.
fn traffic_cone_parts() -> Vec<PropPart> {
    let cone = PrimKind::Cone { radius: 0.18, height: 0.55 };
    let base = b(0.34, 0.04, 0.34);
    vec![PropPart::new(base, Vec3::new(0.0, -0.29, 0.0)), PropPart::new(cone, Vec3::ZERO)]
}

/// A lopsided stack of three cardboard boxes, each smaller and slightly offset/rotated from
/// the one below it.
fn box_stack_parts() -> Vec<PropPart> {
    let b1 = b(0.42, 0.35, 0.42);
    let b2 = b(0.34, 0.30, 0.34);
    let b3 = b(0.26, 0.24, 0.30);
    vec![
        PropPart::new(b1, Vec3::new(0.0, 0.175, 0.0)),
        PropPart::rotated(b2, Vec3::new(0.05, 0.50, -0.03), Vec3::new(0.0, 8.0, 0.0)),
        PropPart::rotated(b3, Vec3::new(-0.06, 0.77, 0.05), Vec3::new(0.0, -12.0, 0.0)),
    ]
}

/// A simple four-legged chair with a seat and backrest.
fn chair_parts() -> Vec<PropPart> {
    let seat = b(0.42, 0.05, 0.42);
    let back = b(0.42, 0.46, 0.05);
    let leg = b(0.04, 0.45, 0.04);
    let leg_off = 0.18;
    vec![
        PropPart::new(seat, Vec3::new(0.0, 0.45, 0.0)),
        PropPart::new(back, Vec3::new(0.0, 0.68, -0.185)),
        PropPart::new(leg, Vec3::new(leg_off, 0.225, leg_off)),
        PropPart::new(leg, Vec3::new(-leg_off, 0.225, leg_off)),
        PropPart::new(leg, Vec3::new(leg_off, 0.225, -leg_off)),
        PropPart::new(leg, Vec3::new(-leg_off, 0.225, -leg_off)),
    ]
}

/// A cylindrical trash can with a lid and a base rim, both a touch shinier than the body.
fn trash_can_parts() -> Vec<PropPart> {
    let body = cyl(0.22, 0.6);
    let lid = cyl(0.24, 0.04);
    let base_rim = cyl(0.235, 0.03);
    vec![
        PropPart::new(body, Vec3::ZERO),
        PropPart::new(lid, Vec3::new(0.0, 0.32, 0.0)).metallic(0.2).roughness(-0.1),
        PropPart::new(base_rim, Vec3::new(0.0, -0.3, 0.0)).metallic(0.2).roughness(-0.1),
    ]
}

/// A vending machine: body, a glossier recessed face plate, and a dark button panel.
fn vending_machine_parts() -> Vec<PropPart> {
    let body = b(0.7, 1.8, 0.6);
    let face = b(0.6, 1.55, 0.05);
    let panel = b(0.18, 0.3, 0.02);
    vec![
        PropPart::new(body, Vec3::new(0.0, 0.9, 0.0)),
        PropPart::new(face, Vec3::new(0.0, 0.95, 0.325)).metallic(0.25).roughness(-0.35),
        PropPart::new(panel, Vec3::new(0.18, 1.15, 0.36)).color(Vec3::new(0.08, 0.08, 0.08)),
    ]
}

/// A park bench: seat, a slightly reclined backrest, and two end supports.
fn bench_parts() -> Vec<PropPart> {
    let seat = b(1.2, 0.06, 0.35);
    let back = b(1.2, 0.4, 0.06);
    let support = b(0.06, 0.45, 0.32);
    vec![
        PropPart::new(seat, Vec3::new(0.0, 0.45, 0.0)),
        PropPart::rotated(back, Vec3::new(0.0, 0.63, -0.16), Vec3::new(-8.0, 0.0, 0.0)),
        PropPart::new(support, Vec3::new(0.55, 0.225, 0.0)),
        PropPart::new(support, Vec3::new(-0.55, 0.225, 0.0)),
    ]
}

/// A fire extinguisher: red body, plus a cap and handle overridden to a fixed silver
/// regardless of the instance's material color (real extinguishers are always red-bodied
/// with a silver/steel head, no matter the paint job).
fn fire_extinguisher_parts() -> Vec<PropPart> {
    let body = cyl(0.09, 0.5);
    let cap = cyl(0.095, 0.06);
    let handle = b(0.02, 0.05, 0.12);
    let steel = Vec3::new(0.75, 0.76, 0.78);
    vec![
        PropPart::new(body, Vec3::ZERO),
        PropPart::new(cap, Vec3::new(0.0, 0.28, 0.0)).color(steel).metallic(0.5).roughness(-0.3),
        PropPart::new(handle, Vec3::new(0.0, 0.32, 0.0)).color(steel).metallic(0.5).roughness(-0.3),
    ]
}

/// A filing cabinet: body plus three drawer face strips, each a little shinier than the body.
fn filing_cabinet_parts() -> Vec<PropPart> {
    let body = b(0.45, 1.3, 0.6);
    let drawer = b(0.42, 0.35, 0.03);
    vec![
        PropPart::new(body, Vec3::new(0.0, 0.65, 0.0)),
        PropPart::new(drawer, Vec3::new(0.0, 0.25, 0.315)).metallic(0.15).roughness(-0.15),
        PropPart::new(drawer, Vec3::new(0.0, 0.65, 0.315)).metallic(0.15).roughness(-0.15),
        PropPart::new(drawer, Vec3::new(0.0, 1.05, 0.315)).metallic(0.15).roughness(-0.15),
    ]
}

/// A potted plant: cylindrical pot plus a clump of spheres for foliage, overridden to green
/// regardless of the instance's material color (the pot itself still takes the instance
/// color, so a map author can still make it terracotta, glazed blue, etc.).
fn potted_plant_parts() -> Vec<PropPart> {
    let pot = cyl(0.2, 0.28);
    let leaf_big = PrimKind::Sphere { radius: 0.22 };
    let leaf_mid = PrimKind::Sphere { radius: 0.17 };
    let leaf_small = PrimKind::Sphere { radius: 0.13 };
    let green = Vec3::new(0.16, 0.42, 0.14);
    vec![
        PropPart::new(pot, Vec3::ZERO),
        PropPart::new(leaf_big, Vec3::new(0.0, 0.38, 0.0)).color(green),
        PropPart::new(leaf_mid, Vec3::new(0.14, 0.5, 0.08)).color(green),
        PropPart::new(leaf_mid, Vec3::new(-0.13, 0.48, -0.1)).color(green),
        PropPart::new(leaf_small, Vec3::new(0.05, 0.62, -0.1)).color(green),
    ]
}

/// A bookshelf: back panel, two sides, four shelf boards, and a few colored "book" boxes on
/// one shelf (books overridden to fixed colors, same reasoning as the potted plant's foliage
/// — the same trick already used for `book1`/`book2` in `examples/room.json`, just baked into
/// the prop instead of hand-authored per map).
fn bookshelf_parts() -> Vec<PropPart> {
    let side = b(0.04, 1.6, 0.3);
    let back = b(0.9, 1.6, 0.04);
    let shelf = b(0.86, 0.03, 0.28);
    let book = b(0.06, 0.28, 0.2);
    vec![
        PropPart::new(back, Vec3::new(0.0, 0.8, -0.13)),
        PropPart::new(side, Vec3::new(0.43, 0.8, 0.0)),
        PropPart::new(side, Vec3::new(-0.43, 0.8, 0.0)),
        PropPart::new(shelf, Vec3::new(0.0, 0.08, 0.0)),
        PropPart::new(shelf, Vec3::new(0.0, 0.53, 0.0)),
        PropPart::new(shelf, Vec3::new(0.0, 0.98, 0.0)),
        PropPart::new(shelf, Vec3::new(0.0, 1.43, 0.0)),
        PropPart::new(book, Vec3::new(-0.32, 0.68, 0.0)).color(Vec3::new(0.65, 0.15, 0.13)),
        PropPart::new(book, Vec3::new(-0.24, 0.68, 0.0)).color(Vec3::new(0.18, 0.55, 0.22)),
        PropPart::new(book, Vec3::new(-0.16, 0.68, 0.0)).color(Vec3::new(0.2, 0.35, 0.7)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Mesh;
    use crate::render::build_prim_mesh;

    fn assert_valid(m: &Mesh) {
        assert!(!m.vertices.is_empty());
        assert!(!m.indices.is_empty());
        assert_eq!(m.indices.len() % 3, 0);
        for &i in &m.indices {
            assert!((i as usize) < m.vertices.len());
        }
    }

    #[test]
    fn every_prop_has_parts_with_valid_meshes() {
        for kind in PropKind::ALL {
            let parts = prop_parts(kind);
            assert!(!parts.is_empty(), "{:?} has no parts", kind);
            for part in &parts {
                assert_valid(&build_prim_mesh(&part.shape));
                assert!((part.metallic_delta + part.roughness_delta).is_finite());
            }
        }
    }

    #[test]
    fn every_kind_round_trips_through_its_name() {
        for kind in PropKind::ALL {
            assert_eq!(PropKind::from_name(kind.name()), Some(kind));
        }
    }
}
