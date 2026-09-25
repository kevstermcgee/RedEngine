//! Fixed colliders: how the solid map (boxes, floors, stair treads, prop footprints) becomes static rapier shapes.
//!
//! The same rules the player's own colliders follow (`collide::collect_box_colliders`), with one difference: round
//! primitives are solid to props.

use super::*;

fn add_box(world: &mut PhysicsWorld, world_mat: Mat4, size: Vec3) {
    let (sc, rot, tr) = world_mat.to_scale_rotation_translation();
    let h = (size * 0.5 * sc.abs()).max(Vec3::splat(0.004));
    world.insert_collider(ColliderBuilder::cuboid(h.x, h.y, h.z).position(pose_of(Mat4::from_rotation_translation(rot, tr))).friction(0.8), None);
}

/// Adds `o`'s solid parts as fixed colliders, following the same rules as the player's own colliders
/// (`collide::collect_box_colliders`): boxes block, props use their footprint policy, stairs give
/// real treads, floor planes are thin slabs and `"collide": false` objects don't count — with one
/// difference: round primitives are solid to props (see below).
pub(super) fn add_static(world: &mut PhysicsWorld, o: &Object, parent: Mat4) {
    if !o.collide {
        return;
    }
    let m = parent * trs(o.position.sample(0.0), o.rotation.sample(0.0), o.scale.sample(0.0));
    match &o.kind {
        ObjectKind::Prim(PrimKind::Box { size }) => add_box(world, m, *size),
        // A floor: a slab whose top is the plane, thick enough that nothing tunnels through.
        ObjectKind::Prim(PrimKind::Plane { size }) => add_box(world, m * Mat4::from_translation(Vec3::new(0.0, -0.05, 0.0)), Vec3::new(size.0, 0.1, size.1)),
        // Round scenery (a cylindrical pedestal, a spherical lamp) does not stop the *player* (only
        // boxes do) but props must not fall through it, so it is solid here.
        ObjectKind::Prim(p) => {
            let (shape, rel) = leaf_collider(p, m);
            world.insert_collider(ColliderBuilder::new(shape).position(pose_of(rel)).friction(0.8), None);
        }
        ObjectKind::Humanoid(_) | ObjectKind::Rat(_) => {}
        ObjectKind::Group(children) => {
            for c in children {
                add_static(world, c, m);
            }
        }
        ObjectKind::Prop(p) => {
            for (lo, hi) in game_collision_boxes(p.kind) {
                add_box(world, m * Mat4::from_translation((lo + hi) * 0.5), hi - lo);
            }
        }
        ObjectKind::Stairs(s) => {
            for (shape, local) in build_stairs_parts(s) {
                if let PrimKind::Box { size } = shape {
                    add_box(world, m * local, size);
                }
            }
        }
    }
}
