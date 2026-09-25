//! Exact melee hit-testing: a ray against the real shapes of every object, not their bounding boxes.
//!
//! The first version of "did the bat hit something" tested a ray against one coarse AABB per
//! top-level object. That reports a hit whenever the ray passes through the *empty* part of a box:
//! the doorway inside a wall group's box, the air over an L-shaped counter, the space between a
//! tree's trunk and the corner of its canopy's box. Swinging at nothing then played the impact
//! thunk. This module tests every leaf shape (box, sphere, cylinder, cone, capsule, plane) in its
//! own local space, so a swing only "hits" where there is actually geometry.
//!
//! Pure functions, no GPU or window types — a future headless server can call them too.

use crate::characters::{human_parts, rat_parts, RatPose};
use crate::geometry::{build_stairs_parts, trs};
use crate::props::prop_parts;
use crate::schema::{Object, ObjectKind, PrimKind, Scene};
use crate::skeleton::{HumanoidRig, PoseSample};
use glam::{Mat4, Vec3};

/// One solid leaf shape placed in the world (pose sampled at `t = 0`, like the colliders).
pub struct HitShape {
    /// Index into `scene.objects` of the top-level object this shape belongs to.
    pub object_index: usize,
    shape: PrimKind,
    world_to_local: Mat4,
}

/// The nearest thing a ray struck.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Index into `scene.objects`.
    pub object_index: usize,
    /// Distance along the ray (world units).
    pub distance: f32,
}

fn add(out: &mut Vec<HitShape>, object_index: usize, shape: PrimKind, world: Mat4) {
    if world.determinant().abs() < 1e-12 {
        return; // scaled to nothing (e.g. a hidden player body)
    }
    out.push(HitShape { object_index, shape, world_to_local: world.inverse() });
}

/// Every leaf shape of `o` in the object's *own* frame (its own position/rotation/scale is not
/// applied, nested groups' are): what `physics` builds colliders from and what hit-testing places
/// in the world.
pub fn object_leaves(o: &Object) -> Vec<(PrimKind, Mat4)> {
    let mut out = Vec::new();
    leaves_of(o, Mat4::IDENTITY, true, &mut out);
    out
}

fn leaves_of(o: &Object, parent: Mat4, is_root: bool, out: &mut Vec<(PrimKind, Mat4)>) {
    // The root's own transform is the caller's business; a nested object's is part of its leaves.
    let world = if is_root { parent } else { parent * trs(o.position.sample(0.0), o.rotation.sample(0.0), o.scale.sample(0.0)) };
    match &o.kind {
        ObjectKind::Prim(p) => out.push((*p, world)),
        ObjectKind::Group(children) => {
            for c in children {
                leaves_of(c, world, false, out);
            }
        }
        ObjectKind::Humanoid(h) => {
            let rig = HumanoidRig::new(h.height, h.build);
            for part in human_parts(&rig, &PoseSample::default(), &h.look) {
                out.push((part.shape, world * part.local));
            }
        }
        ObjectKind::Rat(_) => {
            for part in rat_parts(&RatPose::default()) {
                out.push((part.shape, world * part.local));
            }
        }
        ObjectKind::Prop(p) => {
            for part in prop_parts(p.kind) {
                out.push((part.shape, world * part.local_transform));
            }
        }
        ObjectKind::Stairs(s) => {
            for (shape, local) in build_stairs_parts(s) {
                out.push((shape, world * local));
            }
        }
    }
}

fn collect(o: &Object, object_index: usize, parent: Mat4, out: &mut Vec<HitShape>) {
    let world = parent * trs(o.position.sample(0.0), o.rotation.sample(0.0), o.scale.sample(0.0));
    for (shape, local) in object_leaves(o) {
        add(out, object_index, shape, world * local);
    }
}

/// Every solid leaf shape of every top-level object except `skip` (the player's own body, which
/// the bat must never hit).
pub fn collect_hit_shapes(scene: &Scene, skip: Option<usize>) -> Vec<HitShape> {
    collect_hit_shapes_where(scene, |i| Some(i) != skip)
}

/// [`collect_hit_shapes`] for the top-level objects `keep` accepts (loose physics props are left out:
/// they move, so `physics::PropWorld` ray-casts them instead).
pub fn collect_hit_shapes_where(scene: &Scene, keep: impl Fn(usize) -> bool) -> Vec<HitShape> {
    let mut out = Vec::new();
    for (i, o) in scene.objects.iter().enumerate() {
        if keep(i) {
            collect(o, i, Mat4::IDENTITY, &mut out);
        }
    }
    out
}

/// Smallest `t >= 0` where `o + t*d` crosses the boundary of `shape` (in the shape's local space,
/// where every mesh is centred on the origin), or `None`.
fn hit_local(shape: &PrimKind, o: Vec3, d: Vec3) -> Option<f32> {
    match *shape {
        PrimKind::Box { size } => slab(o, d, size * 0.5),
        PrimKind::Plane { size } => {
            if d.y.abs() < 1e-9 {
                return None;
            }
            let t = -o.y / d.y;
            let p = o + d * t;
            (t >= 0.0 && p.x.abs() <= size.0 * 0.5 && p.z.abs() <= size.1 * 0.5).then_some(t)
        }
        PrimKind::Sphere { radius } => sphere(o, d, Vec3::ZERO, radius),
        PrimKind::Cylinder { radius, height } => cylinder(o, d, radius, height * 0.5),
        PrimKind::Capsule { radius, height } => {
            let half_cyl = ((height - 2.0 * radius).max(0.0)) * 0.5;
            let mut best = side_hit(o, d, radius, half_cyl);
            for c in [half_cyl, -half_cyl] {
                best = min_opt(best, sphere(o, d, Vec3::new(0.0, c, 0.0), radius));
            }
            best
        }
        PrimKind::Cone { radius, height } => cone(o, d, radius, height * 0.5),
    }
}

fn min_opt(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// Ray vs axis-aligned box centred on the origin. A ray starting inside reports `t = 0`.
fn slab(o: Vec3, d: Vec3, half: Vec3) -> Option<f32> {
    let (mut t0, mut t1) = (0.0f32, f32::INFINITY);
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if o[i].abs() > half[i] {
                return None;
            }
        } else {
            let (a, b) = ((-half[i] - o[i]) / d[i], (half[i] - o[i]) / d[i]);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
    }
    (t0 <= t1).then_some(t0)
}

/// Smallest `t >= 0` with `|o + t d - c| = r`.
fn sphere(o: Vec3, d: Vec3, c: Vec3, r: f32) -> Option<f32> {
    let m = o - c;
    let (a, b, cc) = (d.dot(d), m.dot(d), m.dot(m) - r * r);
    let disc = b * b - a * cc;
    if disc < 0.0 || a < 1e-12 {
        return None;
    }
    let s = disc.sqrt();
    let (t0, t1) = ((-b - s) / a, (-b + s) / a);
    if t0 >= 0.0 {
        Some(t0)
    } else if t1 >= 0.0 {
        Some(0.0) // starting inside: count it as contact
    } else {
        None
    }
}

/// The open side of a Y-aligned cylinder of `radius`, limited to `|y| <= half`.
fn side_hit(o: Vec3, d: Vec3, radius: f32, half: f32) -> Option<f32> {
    let a = d.x * d.x + d.z * d.z;
    if a < 1e-12 {
        return None;
    }
    let b = o.x * d.x + o.z * d.z;
    let c = o.x * o.x + o.z * o.z - radius * radius;
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    [(-b - s) / a, (-b + s) / a].into_iter().find(|&t| t >= 0.0 && (o.y + d.y * t).abs() <= half)
}

fn cylinder(o: Vec3, d: Vec3, radius: f32, half: f32) -> Option<f32> {
    let mut best = side_hit(o, d, radius, half);
    if d.y.abs() > 1e-9 {
        for y in [half, -half] {
            let t = (y - o.y) / d.y;
            let p = o + d * t;
            if t >= 0.0 && p.x * p.x + p.z * p.z <= radius * radius {
                best = min_opt(best, Some(t));
            }
        }
    }
    best
}

/// Cone with its apex at `+half`, base circle (radius `r`) at `-half`.
fn cone(o: Vec3, d: Vec3, r: f32, half: f32) -> Option<f32> {
    let k = r / (2.0 * half).max(1e-6);
    // x^2 + z^2 = k^2 (half - y)^2
    let (oy, dy) = (half - o.y, -d.y);
    let a = d.x * d.x + d.z * d.z - k * k * dy * dy;
    let b = o.x * d.x + o.z * d.z - k * k * oy * dy;
    let c = o.x * o.x + o.z * o.z - k * k * oy * oy;
    let mut best = None;
    if a.abs() > 1e-12 {
        let disc = b * b - a * c;
        if disc >= 0.0 {
            let s = disc.sqrt();
            for t in [(-b - s) / a, (-b + s) / a] {
                let y = o.y + d.y * t;
                if t >= 0.0 && y >= -half && y <= half {
                    best = min_opt(best, Some(t));
                }
            }
        }
    }
    if d.y.abs() > 1e-9 {
        let t = (-half - o.y) / d.y;
        let p = o + d * t;
        if t >= 0.0 && p.x * p.x + p.z * p.z <= r * r {
            best = min_opt(best, Some(t));
        }
    }
    best
}

/// The nearest shape struck by the ray `origin + t * dir` within `max_dist` (`dir` need not be
/// normalised), or `None` — including when the ray only passes through empty space inside some
/// object's bounding box.
pub fn raycast_shapes(origin: Vec3, dir: Vec3, max_dist: f32, shapes: &[HitShape]) -> Option<Hit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let mut best: Option<Hit> = None;
    for s in shapes {
        // Transforming the direction without renormalising keeps `t` in world units.
        let o = s.world_to_local.transform_point3(origin);
        let d = s.world_to_local.transform_vector3(dir);
        if let Some(t) = hit_local(&s.shape, o, d) {
            if t <= max_dist && best.is_none_or(|b| t < b.distance) {
                best = Some(Hit { object_index: s.object_index, distance: t });
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(shape: PrimKind, world: Mat4) -> HitShape {
        HitShape { object_index: 0, shape, world_to_local: world.inverse() }
    }

    #[test]
    fn a_box_is_hit_face_on_and_missed_when_beside_it() {
        let s = [shape(PrimKind::Box { size: Vec3::new(2.0, 2.0, 2.0) }, Mat4::from_translation(Vec3::new(0.0, 0.0, -5.0)))];
        let h = raycast_shapes(Vec3::ZERO, -Vec3::Z, 10.0, &s).expect("straight at the box");
        assert!((h.distance - 4.0).abs() < 1e-4);
        assert!(raycast_shapes(Vec3::new(3.0, 0.0, 0.0), -Vec3::Z, 10.0, &s).is_none());
        assert!(raycast_shapes(Vec3::ZERO, -Vec3::Z, 3.0, &s).is_none(), "out of reach");
    }

    #[test]
    fn a_doorway_between_two_wall_boxes_is_not_a_hit() {
        // Two wall slabs with a 1 m gap: the union's AABB spans the gap, the geometry does not.
        let left = shape(PrimKind::Box { size: Vec3::new(2.0, 3.0, 0.2) }, Mat4::from_translation(Vec3::new(-1.5, 1.5, -2.0)));
        let right = shape(PrimKind::Box { size: Vec3::new(2.0, 3.0, 0.2) }, Mat4::from_translation(Vec3::new(1.5, 1.5, -2.0)));
        let walls = [left, right];
        assert!(raycast_shapes(Vec3::new(0.0, 1.5, 0.0), -Vec3::Z, 5.0, &walls).is_none(), "the swing goes through the doorway");
        assert!(raycast_shapes(Vec3::new(1.5, 1.5, 0.0), -Vec3::Z, 5.0, &walls).is_some());
    }

    #[test]
    fn round_shapes_are_hit_on_their_surface_not_their_corners() {
        let sph = [shape(PrimKind::Sphere { radius: 1.0 }, Mat4::from_translation(Vec3::new(0.0, 0.0, -4.0)))];
        assert!((raycast_shapes(Vec3::ZERO, -Vec3::Z, 9.0, &sph).unwrap().distance - 3.0).abs() < 1e-4);
        // A ray that grazes the sphere's bounding-box corner misses the sphere.
        assert!(raycast_shapes(Vec3::new(0.9, 0.9, 0.0), -Vec3::Z, 9.0, &sph).is_none());
        let cyl = [shape(PrimKind::Cylinder { radius: 0.5, height: 2.0 }, Mat4::from_translation(Vec3::new(0.0, 0.0, -3.0)))];
        assert!((raycast_shapes(Vec3::ZERO, -Vec3::Z, 9.0, &cyl).unwrap().distance - 2.5).abs() < 1e-4);
        assert!(raycast_shapes(Vec3::new(0.45, 0.0, 0.0), -Vec3::Z, 9.0, &cyl).is_some());
        assert!(raycast_shapes(Vec3::new(0.7, 0.0, 0.0), -Vec3::Z, 9.0, &cyl).is_none());
        let cap = [shape(PrimKind::Capsule { radius: 0.3, height: 2.0 }, Mat4::from_translation(Vec3::new(0.0, 0.0, -3.0)))];
        assert!(raycast_shapes(Vec3::new(0.0, 0.95, 0.0), -Vec3::Z, 9.0, &cap).is_some(), "rounded cap");
        assert!(raycast_shapes(Vec3::new(0.25, 0.95, 0.0), -Vec3::Z, 9.0, &cap).is_none(), "outside the cap's curve");
        let cone = [shape(PrimKind::Cone { radius: 1.0, height: 2.0 }, Mat4::from_translation(Vec3::new(0.0, 0.0, -3.0)))];
        assert!(raycast_shapes(Vec3::new(0.0, -0.9, 0.0), -Vec3::Z, 9.0, &cone).is_some(), "wide base");
        assert!(raycast_shapes(Vec3::new(0.6, 0.9, 0.0), -Vec3::Z, 9.0, &cone).is_none(), "narrow tip");
    }

    #[test]
    fn scaled_ellipsoids_hit_in_world_units() {
        // A unit sphere squashed to 0.1 deep and moved 2 m away: the hit is at 1.9 m.
        let w = Mat4::from_scale_rotation_translation(Vec3::new(1.0, 1.0, 0.1), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, -2.0));
        let s = [shape(PrimKind::Sphere { radius: 1.0 }, w)];
        assert!((raycast_shapes(Vec3::ZERO, -Vec3::Z, 5.0, &s).unwrap().distance - 1.9).abs() < 1e-4);
    }
}
