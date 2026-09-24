//! Prop-hunt prop library.
//!
//! Each [`PropKind`] expands into a handful of primitive parts placed at fixed local
//! transforms — the same "primitive-composed" technique already used for `humanoid`'s bone
//! rig (see `skeleton::pose_to_parts`) and the bat viewmodel (see
//! `viewer::build_held_parts`), just authored as a schema-level object kind
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

/// Every built-in prop (39). Origin = middle of the base, front = local +Z.
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
    // House set, added for the suburban-home map — see props.rs module doc. All reusable later
    // by the school/office/store maps, per the shared-library approach.
    Sofa,
    Bed,
    DiningTable,
    Tv,
    KitchenCounter,
    Refrigerator,
    Stove,
    Sink,
    Toilet,
    Bathtub,
    WasherDryer,
    Mailbox,
    FenceSection,
    // Interior extras + outdoor/landscaping set (trees, shrubs, flowers, ...) — see the
    // landscaping section near the bottom of this file.
    CoffeeTable,
    Nightstand,
    Wardrobe,
    Desk,
    Rug,
    Armchair,
    TreeOak,
    TreePine,
    Bush,
    FlowerPatch,
    Hedge,
    Boulder,
    Grill,
    PicnicTable,
}

impl PropKind {
    /// Every `PropKind`, in catalogue order (its length is asserted by tests).
    pub const ALL: [PropKind; 39] = [
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
        PropKind::Sofa,
        PropKind::Bed,
        PropKind::DiningTable,
        PropKind::Tv,
        PropKind::KitchenCounter,
        PropKind::Refrigerator,
        PropKind::Stove,
        PropKind::Sink,
        PropKind::Toilet,
        PropKind::Bathtub,
        PropKind::WasherDryer,
        PropKind::Mailbox,
        PropKind::FenceSection,
        PropKind::CoffeeTable,
        PropKind::Nightstand,
        PropKind::Wardrobe,
        PropKind::Desk,
        PropKind::Rug,
        PropKind::Armchair,
        PropKind::TreeOak,
        PropKind::TreePine,
        PropKind::Bush,
        PropKind::FlowerPatch,
        PropKind::Hedge,
        PropKind::Boulder,
        PropKind::Grill,
        PropKind::PicnicTable,
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
            PropKind::Sofa => "sofa",
            PropKind::Bed => "bed",
            PropKind::DiningTable => "dining_table",
            PropKind::Tv => "tv",
            PropKind::KitchenCounter => "kitchen_counter",
            PropKind::Refrigerator => "refrigerator",
            PropKind::Stove => "stove",
            PropKind::Sink => "sink",
            PropKind::Toilet => "toilet",
            PropKind::Bathtub => "bathtub",
            PropKind::WasherDryer => "washer_dryer",
            PropKind::Mailbox => "mailbox",
            PropKind::FenceSection => "fence_section",
            PropKind::CoffeeTable => "coffee_table",
            PropKind::Nightstand => "nightstand",
            PropKind::Wardrobe => "wardrobe",
            PropKind::Desk => "desk",
            PropKind::Rug => "rug",
            PropKind::Armchair => "armchair",
            PropKind::TreeOak => "tree_oak",
            PropKind::TreePine => "tree_pine",
            PropKind::Bush => "bush",
            PropKind::FlowerPatch => "flower_patch",
            PropKind::Hedge => "hedge",
            PropKind::Boulder => "boulder",
            PropKind::Grill => "grill",
            PropKind::PicnicTable => "picnic_table",
        }
    }

    /// Parses a prop name from a scene (`"sofa"`, `"vending_machine"`, ...); `None` if unknown.
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

    /// A non-uniformly scaled part (e.g. a sphere squashed into a low mound or boulder).
    fn scaled(shape: PrimKind, pos: Vec3, scale: Vec3) -> Self {
        PropPart {
            shape,
            local_transform: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, pos),
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

/// Shifts every part up by `dy`. A few of the original props were authored around their own
/// center; the library-wide rule is now "a prop's origin is the middle of its base, resting on
/// the floor" (enforced by `props::tests::props_rest_on_the_floor`), so `position.y` is always
/// simply the height of the surface the prop stands on.
fn lifted(mut parts: Vec<PropPart>, dy: f32) -> Vec<PropPart> {
    for p in &mut parts {
        p.local_transform = Mat4::from_translation(Vec3::new(0.0, dy, 0.0)) * p.local_transform;
    }
    parts
}

/// The primitive parts a prop is drawn from; every part is relative to the prop's base origin.
pub fn prop_parts(kind: PropKind) -> Vec<PropPart> {
    match kind {
        PropKind::Crate => lifted(crate_parts(), 0.28),
        PropKind::Barrel => lifted(barrel_parts(), 0.425),
        PropKind::TrafficCone => lifted(traffic_cone_parts(), 0.31),
        PropKind::BoxStack => box_stack_parts(),
        PropKind::Chair => chair_parts(),
        PropKind::TrashCan => lifted(trash_can_parts(), 0.315),
        PropKind::VendingMachine => vending_machine_parts(),
        PropKind::Bench => bench_parts(),
        PropKind::FireExtinguisher => lifted(fire_extinguisher_parts(), 0.25),
        PropKind::FilingCabinet => filing_cabinet_parts(),
        PropKind::PottedPlant => lifted(potted_plant_parts(), 0.14),
        PropKind::Bookshelf => bookshelf_parts(),
        PropKind::Sofa => sofa_parts(),
        PropKind::Bed => bed_parts(),
        PropKind::DiningTable => dining_table_parts(),
        PropKind::Tv => tv_parts(),
        PropKind::KitchenCounter => kitchen_counter_parts(),
        PropKind::Refrigerator => refrigerator_parts(),
        PropKind::Stove => stove_parts(),
        PropKind::Sink => sink_parts(),
        PropKind::Toilet => toilet_parts(),
        PropKind::Bathtub => bathtub_parts(),
        PropKind::WasherDryer => washer_dryer_parts(),
        PropKind::Mailbox => mailbox_parts(),
        PropKind::FenceSection => fence_section_parts(),
        PropKind::CoffeeTable => coffee_table_parts(),
        PropKind::Nightstand => nightstand_parts(),
        PropKind::Wardrobe => wardrobe_parts(),
        PropKind::Desk => desk_parts(),
        PropKind::Rug => rug_parts(),
        PropKind::Armchair => armchair_parts(),
        PropKind::TreeOak => tree_oak_parts(),
        PropKind::TreePine => tree_pine_parts(),
        PropKind::Bush => lifted(bush_parts(), 0.04),
        PropKind::FlowerPatch => flower_patch_parts(),
        PropKind::Hedge => hedge_parts(),
        PropKind::Boulder => boulder_parts(),
        PropKind::Grill => grill_parts(),
        PropKind::PicnicTable => picnic_table_parts(),
    }
}

/// How a prop blocks the player. Most props block with one collider covering the union of all
/// their parts; a few need something else so they behave like the real thing:
/// - trees: only the trunk blocks (the canopy is overhead — you walk right up to the trunk);
/// - bushes: a tighter box than their bounding volume (a round clump, not a square);
/// - flowers, rugs: walk-through (no collider at all).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Collision {
    /// One collider spanning every part (the default).
    Union,
    /// Nothing blocks the player and nothing can be stood on.
    None,
    /// A fixed local-space box, ignoring the parts (e.g. a tree trunk).
    Box { min: Vec3, max: Vec3 },
}

/// How a prop blocks the player: one footprint collider, several, or none.
pub fn collision(kind: PropKind) -> Collision {
    match kind {
        PropKind::FlowerPatch | PropKind::Rug => Collision::None,
        PropKind::TreeOak | PropKind::TreePine => {
            Collision::Box { min: Vec3::new(-0.2, 0.0, -0.2), max: Vec3::new(0.2, 4.0, 0.2) }
        }
        // A shrub is a round clump; its bounding box would be a generous square around it.
        PropKind::Bush => Collision::Box { min: Vec3::new(-0.6, 0.0, -0.6), max: Vec3::new(0.6, 0.9, 0.6) },
        _ => Collision::Union,
    }
}

/// Local-space AABB (min, max) of all of a prop's parts — the visual extent, used by tools
/// (`lint`, `scatter`, `plan`) that need a footprint without instantiating meshes.
pub fn local_bounds(kind: PropKind) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for part in prop_parts(kind) {
        let half = part.shape.half_extent();
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                for sz in [-1.0f32, 1.0] {
                    let p = part.local_transform.transform_point3(Vec3::new(half.x * sx, half.y * sy, half.z * sz));
                    min = min.min(p);
                    max = max.max(p);
                }
            }
        }
    }
    (min, max)
}

/// Local-space box that actually blocks the player for this prop, or `None` if it doesn't
/// block at all. See [`Collision`].
pub fn collision_box(kind: PropKind) -> Option<(Vec3, Vec3)> {
    match collision(kind) {
        Collision::Union => Some(local_bounds(kind)),
        Collision::None => None,
        Collision::Box { min, max } => Some((min, max)),
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

// ---------------------------------------------------------------------------------------------
// House set
// ---------------------------------------------------------------------------------------------

/// A 3-seat sofa: seat cushion, backrest, two armrests, four short legs.
fn sofa_parts() -> Vec<PropPart> {
    let leg = b(0.06, 0.12, 0.06);
    let seat = b(1.8, 0.3, 0.8);
    let back = b(1.8, 0.55, 0.18);
    let arm = b(0.16, 0.4, 0.8);
    vec![
        PropPart::new(leg, Vec3::new(0.85, 0.06, 0.32)),
        PropPart::new(leg, Vec3::new(-0.85, 0.06, 0.32)),
        PropPart::new(leg, Vec3::new(0.85, 0.06, -0.32)),
        PropPart::new(leg, Vec3::new(-0.85, 0.06, -0.32)),
        PropPart::new(seat, Vec3::new(0.0, 0.27, 0.0)),
        PropPart::new(back, Vec3::new(0.0, 0.695, -0.31)),
        PropPart::new(arm, Vec3::new(0.9, 0.32, 0.0)),
        PropPart::new(arm, Vec3::new(-0.9, 0.32, 0.0)),
    ]
}

/// A bed: frame, mattress, headboard, and a pillow (color-overridden white regardless of the
/// instance's material, like the potted plant's foliage — bedding reads oddly if it takes
/// whatever color the frame is).
fn bed_parts() -> Vec<PropPart> {
    let frame = b(1.6, 0.25, 2.0);
    let mattress = b(1.5, 0.22, 1.9);
    let headboard = b(1.6, 0.6, 0.08);
    let pillow = b(0.5, 0.12, 0.35);
    vec![
        PropPart::new(frame, Vec3::new(0.0, 0.125, 0.0)),
        PropPart::new(mattress, Vec3::new(0.0, 0.36, 0.0)),
        PropPart::new(headboard, Vec3::new(0.0, 0.3, -1.04)),
        PropPart::new(pillow, Vec3::new(0.0, 0.53, -0.7)).color(Vec3::new(0.92, 0.92, 0.9)),
    ]
}

/// A dining table: top + four legs — the same composition `room.json`'s original hand-authored
/// `table` used, promoted to a reusable prop.
fn dining_table_parts() -> Vec<PropPart> {
    let top = b(1.6, 0.06, 0.9);
    let leg = b(0.08, 0.72, 0.08);
    vec![
        PropPart::new(top, Vec3::new(0.0, 0.75, 0.0)),
        PropPart::new(leg, Vec3::new(0.7, 0.36, -0.37)),
        PropPart::new(leg, Vec3::new(-0.7, 0.36, -0.37)),
        PropPart::new(leg, Vec3::new(0.7, 0.36, 0.37)),
        PropPart::new(leg, Vec3::new(-0.7, 0.36, 0.37)),
    ]
}

/// A flat-screen TV on a small stand. The screen is color-overridden near-black regardless of
/// the instance's material — a TV screen doesn't take a paint color.
fn tv_parts() -> Vec<PropPart> {
    let base = b(0.4, 0.04, 0.25);
    let neck = b(0.06, 0.35, 0.06);
    let screen = b(1.1, 0.65, 0.05);
    vec![
        PropPart::new(base, Vec3::new(0.0, 0.02, 0.0)),
        PropPart::new(neck, Vec3::new(0.0, 0.195, 0.0)),
        PropPart::new(screen, Vec3::new(0.0, 0.695, 0.0)).color(Vec3::new(0.03, 0.03, 0.035)),
    ]
}

/// A kitchen counter run: cabinet body plus a glossier overhanging countertop.
fn kitchen_counter_parts() -> Vec<PropPart> {
    let body = b(1.8, 0.9, 0.6);
    let top = b(1.86, 0.05, 0.64);
    vec![
        PropPart::new(body, Vec3::new(0.0, 0.45, 0.0)),
        PropPart::new(top, Vec3::new(0.0, 0.925, 0.0)).metallic(0.15).roughness(-0.25),
    ]
}

/// A refrigerator: tall body, a darker seam between the freezer and fridge sections, and a
/// metallic door handle.
fn refrigerator_parts() -> Vec<PropPart> {
    let body = b(0.75, 1.8, 0.7);
    let seam = b(0.77, 0.03, 0.72);
    let handle = b(0.04, 0.35, 0.04);
    vec![
        PropPart::new(body, Vec3::new(0.0, 0.9, 0.0)),
        PropPart::new(seam, Vec3::new(0.0, 1.3, 0.0)).color(Vec3::new(0.05, 0.05, 0.05)),
        PropPart::new(handle, Vec3::new(0.3, 1.1, 0.37)).metallic(0.3).roughness(-0.2),
    ]
}

/// A stove: body, four dark burners on top, and an oven door panel.
fn stove_parts() -> Vec<PropPart> {
    let body = b(0.6, 0.9, 0.6);
    let burner = cyl(0.08, 0.02);
    let oven_door = b(0.55, 0.5, 0.03);
    let mut parts = vec![
        PropPart::new(body, Vec3::new(0.0, 0.45, 0.0)),
        PropPart::new(oven_door, Vec3::new(0.0, 0.35, 0.31)).metallic(0.1),
    ];
    for &(x, z) in &[(0.15, 0.15), (-0.15, 0.15), (0.15, -0.15), (-0.15, -0.15)] {
        parts.push(
            PropPart::new(burner, Vec3::new(x, 0.91, z)).color(Vec3::new(0.05, 0.05, 0.05)).metallic(0.4).roughness(-0.2),
        );
    }
    parts
}

/// A sink: cabinet base, counter slab, a light porcelain basin, and a metallic faucet.
fn sink_parts() -> Vec<PropPart> {
    let cabinet = b(0.6, 0.8, 0.55);
    let counter = b(0.64, 0.04, 0.58);
    let basin = b(0.4, 0.15, 0.35);
    let faucet = cyl(0.02, 0.25);
    vec![
        PropPart::new(cabinet, Vec3::new(0.0, 0.4, 0.0)),
        PropPart::new(counter, Vec3::new(0.0, 0.82, 0.0)),
        PropPart::new(basin, Vec3::new(0.0, 0.84, 0.0)).color(Vec3::new(0.9, 0.9, 0.88)).roughness(-0.3),
        PropPart::new(faucet, Vec3::new(0.0, 0.945, -0.2)).metallic(0.4).roughness(-0.2),
    ]
}

/// A toilet: base, bowl rim, tank, and lid — all porcelain-white regardless of instance color
/// (like the bathtub), since a toilet's color isn't something a map author should need to
/// think about.
fn toilet_parts() -> Vec<PropPart> {
    let porcelain = Vec3::new(0.92, 0.92, 0.9);
    let base = cyl(0.2, 0.4);
    let bowl_top = cyl(0.22, 0.08);
    let tank = b(0.42, 0.4, 0.18);
    let lid = cyl(0.23, 0.03);
    vec![
        PropPart::new(base, Vec3::new(0.0, 0.2, 0.0)).color(porcelain),
        PropPart::new(bowl_top, Vec3::new(0.0, 0.42, 0.0)).color(porcelain),
        PropPart::new(tank, Vec3::new(0.0, 0.6, -0.25)).color(porcelain),
        PropPart::new(lid, Vec3::new(0.0, 0.46, 0.0)).color(porcelain),
    ]
}

/// A bathtub: porcelain-white shell, a rim lip, and a metallic faucet.
fn bathtub_parts() -> Vec<PropPart> {
    let porcelain = Vec3::new(0.92, 0.92, 0.9);
    let shell = b(1.6, 0.55, 0.75);
    let rim = b(1.7, 0.05, 0.8);
    let faucet = cyl(0.02, 0.2);
    vec![
        PropPart::new(shell, Vec3::new(0.0, 0.275, 0.0)).color(porcelain),
        PropPart::new(rim, Vec3::new(0.0, 0.575, 0.0)).color(porcelain),
        PropPart::new(faucet, Vec3::new(0.0, 0.65, -0.32)).metallic(0.4).roughness(-0.2),
    ]
}

/// A stacked washer/dryer unit: body, a mid seam, and two dark porthole doors (rotated so their
/// flat face points forward along local `+Z`, the same "rotate a Y-axis cylinder 90° about X"
/// trick the bat viewmodel's grip collars use).
fn washer_dryer_parts() -> Vec<PropPart> {
    let body = b(0.65, 1.7, 0.65);
    let seam = b(0.67, 0.03, 0.67);
    let door = cyl(0.22, 0.03);
    vec![
        PropPart::new(body, Vec3::new(0.0, 0.85, 0.0)),
        PropPart::new(seam, Vec3::new(0.0, 0.85, 0.0)),
        PropPart::rotated(door, Vec3::new(0.0, 1.2, 0.34), Vec3::new(90.0, 0.0, 0.0))
            .color(Vec3::new(0.05, 0.05, 0.05))
            .roughness(-0.2),
        PropPart::rotated(door, Vec3::new(0.0, 0.45, 0.34), Vec3::new(90.0, 0.0, 0.0))
            .color(Vec3::new(0.05, 0.05, 0.05))
            .roughness(-0.2),
    ]
}

/// A post-mounted mailbox with a small flag.
fn mailbox_parts() -> Vec<PropPart> {
    let post = cyl(0.04, 0.9);
    let box_body = b(0.15, 0.18, 0.35);
    let flag = b(0.02, 0.1, 0.03);
    vec![
        PropPart::new(post, Vec3::new(0.0, 0.45, 0.0)),
        PropPart::new(box_body, Vec3::new(0.0, 0.99, 0.0)),
        PropPart::new(flag, Vec3::new(0.09, 0.95, 0.1)).metallic(0.2),
    ]
}

/// A tileable ~2m fence segment: two end posts and three horizontal rails, meant to be placed
/// repeatedly along a perimeter (the same pattern the room maps already use for straight walls
/// built from plain `box` prims, just packaged as a reusable prop).
fn fence_section_parts() -> Vec<PropPart> {
    let post = b(0.08, 1.8, 0.08);
    let rail = b(2.0, 0.08, 0.03);
    vec![
        PropPart::new(post, Vec3::new(1.0, 0.9, 0.0)),
        PropPart::new(post, Vec3::new(-1.0, 0.9, 0.0)),
        PropPart::new(rail, Vec3::new(0.0, 1.5, 0.0)),
        PropPart::new(rail, Vec3::new(0.0, 0.9, 0.0)),
        PropPart::new(rail, Vec3::new(0.0, 0.3, 0.0)),
    ]
}

// ---------------------------------------------------------------------------------------------
// Interior extras
// ---------------------------------------------------------------------------------------------

/// A low living-room coffee table: top + four legs.
fn coffee_table_parts() -> Vec<PropPart> {
    let top = b(1.0, 0.05, 0.55);
    let leg = b(0.05, 0.4, 0.05);
    vec![
        PropPart::new(top, Vec3::new(0.0, 0.425, 0.0)),
        PropPart::new(leg, Vec3::new(0.44, 0.2, 0.22)),
        PropPart::new(leg, Vec3::new(-0.44, 0.2, 0.22)),
        PropPart::new(leg, Vec3::new(0.44, 0.2, -0.22)),
        PropPart::new(leg, Vec3::new(-0.44, 0.2, -0.22)),
    ]
}

/// A bedside table with a drawer face on its `+Z` side.
fn nightstand_parts() -> Vec<PropPart> {
    let body = b(0.45, 0.5, 0.4);
    let top = b(0.49, 0.03, 0.44);
    let drawer = b(0.38, 0.16, 0.02);
    vec![
        PropPart::new(body, Vec3::new(0.0, 0.25, 0.0)),
        PropPart::new(top, Vec3::new(0.0, 0.515, 0.0)),
        PropPart::new(drawer, Vec3::new(0.0, 0.34, 0.205)).metallic(0.15).roughness(-0.15),
    ]
}

/// A tall two-door wardrobe (doors face `+Z`).
fn wardrobe_parts() -> Vec<PropPart> {
    let body = b(1.2, 2.0, 0.6);
    let seam = b(0.02, 1.9, 0.02);
    let handle = b(0.03, 0.22, 0.03);
    let dark = Vec3::new(0.06, 0.06, 0.06);
    vec![
        PropPart::new(body, Vec3::new(0.0, 1.0, 0.0)),
        PropPart::new(seam, Vec3::new(0.0, 1.0, 0.31)).color(dark),
        PropPart::new(handle, Vec3::new(-0.08, 1.0, 0.32)).metallic(0.4).roughness(-0.25),
        PropPart::new(handle, Vec3::new(0.08, 1.0, 0.32)).metallic(0.4).roughness(-0.25),
    ]
}

/// A writing desk: top, a drawer pedestal on the right, one leg on the left (front is `+Z`).
fn desk_parts() -> Vec<PropPart> {
    let top = b(1.4, 0.05, 0.7);
    let pedestal = b(0.4, 0.72, 0.62);
    let leg = b(0.05, 0.72, 0.05);
    vec![
        PropPart::new(top, Vec3::new(0.0, 0.745, 0.0)),
        PropPart::new(pedestal, Vec3::new(0.48, 0.36, 0.0)),
        PropPart::new(leg, Vec3::new(-0.65, 0.36, 0.28)),
        PropPart::new(leg, Vec3::new(-0.65, 0.36, -0.28)),
    ]
}

/// A floor rug — walk-through (see [`collision`]).
fn rug_parts() -> Vec<PropPart> {
    vec![PropPart::new(b(2.0, 0.02, 1.4), Vec3::new(0.0, 0.01, 0.0)).roughness(0.3)]
}

/// An upholstered armchair (faces `+Z`).
fn armchair_parts() -> Vec<PropPart> {
    let seat = b(0.8, 0.25, 0.75);
    let back = b(0.8, 0.5, 0.18);
    let arm = b(0.15, 0.3, 0.75);
    let leg = b(0.05, 0.12, 0.05);
    vec![
        PropPart::new(leg, Vec3::new(0.34, 0.06, 0.3)),
        PropPart::new(leg, Vec3::new(-0.34, 0.06, 0.3)),
        PropPart::new(leg, Vec3::new(0.34, 0.06, -0.3)),
        PropPart::new(leg, Vec3::new(-0.34, 0.06, -0.3)),
        PropPart::new(seat, Vec3::new(0.0, 0.245, 0.0)),
        PropPart::new(back, Vec3::new(0.0, 0.62, -0.285)),
        PropPart::new(arm, Vec3::new(0.395, 0.4, 0.0)),
        PropPart::new(arm, Vec3::new(-0.395, 0.4, 0.0)),
    ]
}

// ---------------------------------------------------------------------------------------------
// Landscaping. Convention for the plants: the instance `material.color` is the *foliage* (or
// bloom) color, so a map author or `scatter` can tint each one; trunks/stems/soil are fixed.
// ---------------------------------------------------------------------------------------------

const TRUNK_BROWN: Vec3 = Vec3::new(0.24, 0.14, 0.07);

/// A broadleaf tree ~4.5m tall: trunk plus a lumpy cluster of canopy spheres. Only the trunk
/// blocks the player (see [`collision`]).
fn tree_oak_parts() -> Vec<PropPart> {
    let trunk = cyl(0.17, 2.4);
    let big = PrimKind::Sphere { radius: 1.15 };
    let mid = PrimKind::Sphere { radius: 0.85 };
    let top = PrimKind::Sphere { radius: 0.75 };
    vec![
        PropPart::new(trunk, Vec3::new(0.0, 1.2, 0.0)).color(TRUNK_BROWN).roughness(0.3),
        PropPart::new(big, Vec3::new(0.0, 3.1, 0.0)).roughness(0.2),
        PropPart::new(mid, Vec3::new(0.75, 2.65, 0.3)).roughness(0.2),
        PropPart::new(mid, Vec3::new(-0.7, 2.75, -0.4)).roughness(0.2),
        PropPart::new(top, Vec3::new(0.1, 3.85, -0.2)).roughness(0.2),
    ]
}

/// A conifer ~5m tall: a short trunk under four stacked cones.
fn tree_pine_parts() -> Vec<PropPart> {
    let trunk = cyl(0.14, 1.3);
    vec![
        PropPart::new(trunk, Vec3::new(0.0, 0.65, 0.0)).color(TRUNK_BROWN).roughness(0.3),
        PropPart::new(PrimKind::Cone { radius: 1.15, height: 1.7 }, Vec3::new(0.0, 1.75, 0.0)).roughness(0.25),
        PropPart::new(PrimKind::Cone { radius: 0.9, height: 1.6 }, Vec3::new(0.0, 2.75, 0.0)).roughness(0.25),
        PropPart::new(PrimKind::Cone { radius: 0.62, height: 1.5 }, Vec3::new(0.0, 3.75, 0.0)).roughness(0.25),
        PropPart::new(PrimKind::Cone { radius: 0.35, height: 1.0 }, Vec3::new(0.0, 4.6, 0.0)).roughness(0.25),
    ]
}

/// A shrub ~0.9m tall: a clump of overlapping foliage spheres.
fn bush_parts() -> Vec<PropPart> {
    vec![
        PropPart::new(PrimKind::Sphere { radius: 0.5 }, Vec3::new(0.0, 0.42, 0.0)).roughness(0.2),
        PropPart::new(PrimKind::Sphere { radius: 0.4 }, Vec3::new(0.45, 0.34, 0.15)).roughness(0.2),
        PropPart::new(PrimKind::Sphere { radius: 0.42 }, Vec3::new(-0.42, 0.36, -0.1)).roughness(0.2),
        PropPart::new(PrimKind::Sphere { radius: 0.36 }, Vec3::new(0.05, 0.55, -0.3)).roughness(0.2),
    ]
}

/// A small flower clump: a green mound with colored blooms (instance color) and a few pale
/// accent blooms. Walk-through (see [`collision`]).
fn flower_patch_parts() -> Vec<PropPart> {
    let mound = PrimKind::Sphere { radius: 0.32 };
    let bloom = PrimKind::Sphere { radius: 0.075 };
    let leaf_green = Vec3::new(0.13, 0.36, 0.12);
    let accent = Vec3::new(0.97, 0.9, 0.55);
    vec![
        PropPart::scaled(mound, Vec3::new(0.0, 0.08, 0.0), Vec3::new(1.0, 0.4, 1.0)).color(leaf_green).roughness(0.3),
        PropPart::new(bloom, Vec3::new(0.1, 0.24, 0.05)),
        PropPart::new(bloom, Vec3::new(-0.12, 0.22, 0.1)),
        PropPart::new(bloom, Vec3::new(0.02, 0.27, -0.13)),
        PropPart::new(bloom, Vec3::new(0.17, 0.2, -0.1)),
        PropPart::new(bloom, Vec3::new(-0.17, 0.2, -0.09)).color(accent),
        PropPart::new(bloom, Vec3::new(-0.03, 0.22, 0.17)).color(accent),
    ]
}

/// A 1.8m-long clipped hedge with a rounded top.
fn hedge_parts() -> Vec<PropPart> {
    vec![
        PropPart::new(b(1.8, 0.8, 0.6), Vec3::new(0.0, 0.4, 0.0)).roughness(0.25),
        PropPart::rotated(cyl(0.3, 1.8), Vec3::new(0.0, 0.8, 0.0), Vec3::new(0.0, 0.0, 90.0)).roughness(0.25),
    ]
}

/// A weathered boulder: two squashed spheres.
fn boulder_parts() -> Vec<PropPart> {
    vec![
        PropPart::scaled(PrimKind::Sphere { radius: 0.5 }, Vec3::new(0.0, 0.3, 0.0), Vec3::new(1.0, 0.65, 0.85)).roughness(0.3),
        PropPart::scaled(PrimKind::Sphere { radius: 0.28 }, Vec3::new(0.48, 0.16, 0.18), Vec3::new(1.0, 0.7, 0.9)).roughness(0.3),
    ]
}

/// A backyard barbecue on a cart (front is `+Z`).
fn grill_parts() -> Vec<PropPart> {
    let leg = b(0.04, 0.65, 0.04);
    let steel = Vec3::new(0.55, 0.56, 0.58);
    vec![
        PropPart::new(b(0.8, 0.4, 0.5), Vec3::new(0.0, 0.85, 0.0)),
        PropPart::rotated(cyl(0.25, 0.8), Vec3::new(0.0, 1.12, 0.0), Vec3::new(0.0, 0.0, 90.0)),
        PropPart::new(b(0.36, 0.03, 0.4), Vec3::new(0.58, 0.8, 0.0)).color(steel).metallic(0.4),
        PropPart::new(b(0.5, 0.03, 0.03), Vec3::new(0.0, 1.32, 0.3)).color(steel).metallic(0.5).roughness(-0.25),
        PropPart::new(leg, Vec3::new(0.36, 0.325, 0.2)).color(steel).metallic(0.4),
        PropPart::new(leg, Vec3::new(-0.36, 0.325, 0.2)).color(steel).metallic(0.4),
        PropPart::new(leg, Vec3::new(0.36, 0.325, -0.2)).color(steel).metallic(0.4),
        PropPart::new(leg, Vec3::new(-0.36, 0.325, -0.2)).color(steel).metallic(0.4),
    ]
}

/// A wooden picnic table with attached benches (long axis along `X`).
fn picnic_table_parts() -> Vec<PropPart> {
    let table_leg = b(0.06, 0.75, 0.06);
    let bench_leg = b(0.06, 0.45, 0.06);
    vec![
        PropPart::new(b(1.8, 0.06, 0.75), Vec3::new(0.0, 0.76, 0.0)),
        PropPart::new(b(1.8, 0.05, 0.28), Vec3::new(0.0, 0.45, 0.6)),
        PropPart::new(b(1.8, 0.05, 0.28), Vec3::new(0.0, 0.45, -0.6)),
        PropPart::new(table_leg, Vec3::new(0.75, 0.375, 0.32)),
        PropPart::new(table_leg, Vec3::new(-0.75, 0.375, 0.32)),
        PropPart::new(table_leg, Vec3::new(0.75, 0.375, -0.32)),
        PropPart::new(table_leg, Vec3::new(-0.75, 0.375, -0.32)),
        PropPart::new(bench_leg, Vec3::new(0.75, 0.225, 0.6)),
        PropPart::new(bench_leg, Vec3::new(-0.75, 0.225, 0.6)),
        PropPart::new(bench_leg, Vec3::new(0.75, 0.225, -0.6)),
        PropPart::new(bench_leg, Vec3::new(-0.75, 0.225, -0.6)),
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
    fn collision_boxes_are_sane() {
        for kind in PropKind::ALL {
            if let Some((min, max)) = collision_box(kind) {
                assert!(min.x < max.x && min.y < max.y && min.z < max.z, "{:?} has a degenerate collider", kind);
            }
        }
        assert!(collision_box(PropKind::FlowerPatch).is_none(), "flowers must be walk-through");
        let (min, max) = collision_box(PropKind::TreeOak).unwrap();
        assert!(max.x - min.x < 0.6, "only a tree's trunk should block, not its canopy");
    }

    #[test]
    fn props_rest_on_the_floor() {
        // Every prop's lowest point sits at y=0 (so `position.y` is the floor it stands on).
        let bad: Vec<String> = PropKind::ALL
            .iter()
            .filter_map(|&kind| {
                let (min, _) = local_bounds(kind);
                (!(min.y > -0.05 && min.y < 0.06)).then(|| format!("{}(min y={:.3})", kind.name(), min.y))
            })
            .collect();
        assert!(bad.is_empty(), "props whose origin is not at their base: {}", bad.join(", "));
    }

    #[test]
    fn every_kind_round_trips_through_its_name() {
        for kind in PropKind::ALL {
            assert_eq!(PropKind::from_name(kind.name()), Some(kind));
        }
    }
}
