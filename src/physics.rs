//! Loose props: pick up, carry, drop, knock over — rigid-body physics for the things a person or a
//! rat could move, on top of the game's own (2-D, kinematic) player physics.
//!
//! **What is loose?** [`classify`] decides per top-level scene object: a `prop` (crate, chair,
//! potted plant, ...) or a floor-mounted prefab (an apple, a mug, a stack of books, a desk lamp...)
//! that is small enough for a human to lift ([`HUMAN_CARRY`]) and is not a fixture (toilet, tree,
//! rug, ...). `"movable": true|false` on an object overrides the guess. Everything else — walls,
//! floors, stairs, big furniture — is a fixed collider. A rat can only carry the smaller subset
//! ([`RAT_CARRY`]) but can shove any loose prop.
//!
//! **How it works.** [`PropWorld`] wraps a [rapier](https://rapier.rs) world: the map's solid boxes,
//! floors, stair treads and prop footprints become fixed colliders; each loose object becomes a
//! body with one collider per leaf shape — *dormant* (fixed exactly where the map author put it, so
//! a whole map of clutter costs nothing and never shuffles at load) until something disturbs it: the
//! player walking into it, a bat, a moving prop touching it, being picked up. A disturbed prop becomes
//! dynamic (and drags along whatever rests on or touches it); the player is a kinematic cylinder that
//! shoves whatever it walks into. Carrying
//! disables the body and pins the object in front of the player; dropping re-enables it with the
//! player's momentum, so it falls, bounces, tumbles and knocks smaller things about.
//!
//! No window or GPU types here (ADR 0010): the physics is callable headless. The player's own
//! walking still uses `crate::player` / `crate::viewer` (ADR 0003); loose props are removed from
//! *those* collider lists (see [`PropWorld::movable_indices`]) and handled here instead.

use crate::hit::object_leaves;
use crate::props::collision_box;
use crate::render::{build_stairs_parts, trs};
use crate::schema::{Object, ObjectKind, PrimKind, Scene};
use crate::track::Track;
use glam::{EulerRot, Mat4, Quat, Vec3};
use rapier3d::prelude::{Aabb, ColliderBuilder, ColliderHandle, PhysicsWorld, Pose, QueryFilter, Ray, RigidBodyBuilder, RigidBodyHandle, RigidBodyType, SharedShape};
use std::collections::{HashMap, HashSet};

/// Gravity acting on loose props, m/s^2 (a little brisker than 9.81 so drops feel snappy, like the
/// player's own `player::GRAVITY`).
pub const PROP_GRAVITY: f32 = 14.0;
/// Density of every prop collider, kg/m^3 (furniture and clutter are mostly hollow or light).
pub const PROP_DENSITY: f32 = 120.0;
/// Props never move faster than this, m/s (a bat swing or a thrown-off crate stays sane).
const MAX_SPEED: f32 = 14.0;
/// A prop that falls below this height (out of the map) is put back where it started.
const KILL_Y: f32 = -30.0;

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
    "kitchen_counter", "refrigerator", "stove", "sink", "toilet", "bathtub", "washer_dryer", "mailbox", "fence_section", "rug",
    "tree_oak", "tree_pine", "bush", "flower_patch", "hedge", "boulder", "grill", "picnic_table", "bench", "vending_machine",
    "filing_cabinet", "bookshelf", "wardrobe", "bed", "sofa",
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

fn object_scale(o: &Object) -> Vec3 {
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

/// One loose object and its rigid body.
pub struct Prop {
    /// Index into `scene.objects`.
    pub object_index: usize,
    /// Bounding box in the object's frame.
    pub shape: PropShape,
    body: RigidBodyHandle,
    spawn: Mat4,
    last_written: Mat4,
    /// Fixed where it was placed until something disturbs it (see the module docs).
    dormant: bool,
}

struct Held {
    prop: usize,
    pose: Mat4,
}

/// The rigid-body world for a map's loose props (see the module docs).
pub struct PropWorld {
    world: PhysicsWorld,
    props: Vec<Prop>,
    by_body: HashMap<RigidBodyHandle, usize>,
    player: RigidBodyHandle,
    player_collider: Option<ColliderHandle>,
    player_dims: (f32, f32),
    held: Option<Held>,
}

fn loosen(a: Aabb, by: f32) -> Aabb {
    Aabb::new(a.mins - Vec3::splat(by), a.maxs + Vec3::splat(by))
}

fn pose_of(m: Mat4) -> Pose {
    Pose::from_mat4(m)
}

/// Rigid transform (no scale) of an object's origin.
fn object_body_mat(o: &Object) -> Mat4 {
    let r = o.rotation.sample(0.0);
    let q = Quat::from_euler(EulerRot::XYZ, r.x.to_radians(), r.y.to_radians(), r.z.to_radians());
    Mat4::from_rotation_translation(q, o.position.sample(0.0))
}

/// The rapier shape for a leaf, sized by the (possibly non-uniform) scale in `m`, and its pose in
/// the body frame. Non-uniform scale on round shapes is averaged.
fn leaf_collider(shape: &PrimKind, m: Mat4) -> (SharedShape, Mat4) {
    let (sc, rot, tr) = m.to_scale_rotation_translation();
    let sc = sc.abs().max(Vec3::splat(1e-3));
    let radial = 0.5 * (sc.x + sc.z);
    let min = 0.004;
    let s = match *shape {
        PrimKind::Box { size } => {
            let h = (size * 0.5 * sc).max(Vec3::splat(min));
            SharedShape::cuboid(h.x, h.y, h.z)
        }
        PrimKind::Plane { size } => SharedShape::cuboid((size.0 * 0.5 * sc.x).max(min), 0.01, (size.1 * 0.5 * sc.z).max(min)),
        PrimKind::Sphere { radius } => SharedShape::ball((radius * (sc.x * sc.y * sc.z).cbrt()).max(min)),
        PrimKind::Cylinder { radius, height } => SharedShape::cylinder((height * 0.5 * sc.y).max(min), (radius * radial).max(min)),
        PrimKind::Cone { radius, height } => SharedShape::cone((height * 0.5 * sc.y).max(min), (radius * radial).max(min)),
        PrimKind::Capsule { radius, height } => {
            let r = (radius * radial).max(min);
            SharedShape::capsule_y(((height * 0.5 - radius).max(0.0) * sc.y).max(0.0), r)
        }
    };
    (s, Mat4::from_rotation_translation(rot, tr))
}

impl PropWorld {
    /// Builds the world for `scene`: fixed colliders for the solid map, a dynamic body for every
    /// loose prop, and the player's kinematic body. `skip` is a top-level object index to leave out
    /// entirely (the player's own body model).
    pub fn new(scene: &Scene, skip: Option<usize>) -> Self {
        let mut world = PhysicsWorld::default();
        world.gravity = Vec3::new(0.0, -PROP_GRAVITY, 0.0);
        world.integration_parameters.dt = crate::player::FIXED_DT;
        // Interpenetrating things (a globe placed a hair into its table, two chairs pushed together)
        // separate gently instead of being fired apart.
        world.integration_parameters.normalized_max_corrective_velocity = 1.5;

        // A floor under everything, so nothing can ever fall out of the map.
        world.insert_collider(ColliderBuilder::cuboid(500.0, 0.5, 500.0).position(Pose::from_translation(Vec3::new(0.0, -0.5, 0.0))).friction(0.8), None);

        let loose: Vec<(usize, PropShape)> =
            scene.objects.iter().enumerate().filter(|(i, _)| Some(*i) != skip).filter_map(|(i, o)| classify(o).map(|s| (i, s))).collect();
        let loose_set: HashSet<usize> = loose.iter().map(|(i, _)| *i).collect();

        for (i, o) in scene.objects.iter().enumerate() {
            if Some(i) != skip && !loose_set.contains(&i) {
                add_static(&mut world, o, Mat4::IDENTITY);
            }
        }

        let mut props = Vec::with_capacity(loose.len());
        let mut by_body = HashMap::new();
        for (i, shape) in loose {
            let o = &scene.objects[i];
            let spawn = object_body_mat(o);
            // Fixed for now: `activate` turns it dynamic the first time something disturbs it. The
            // damping and CCD settings carry over to that moment.
            let body = world.insert_body(RigidBodyBuilder::fixed().pose(pose_of(spawn)).linear_damping(0.15).angular_damping(0.8).ccd_enabled(true));
            let s = Mat4::from_scale(object_scale(o));
            for (leaf, local) in object_leaves(o) {
                let (sh, rel) = leaf_collider(&leaf, s * local);
                world.insert_collider(ColliderBuilder::new(sh).position(pose_of(rel)).density(PROP_DENSITY).friction(0.7).restitution(0.2), Some(body));
            }
            by_body.insert(body, props.len());
            props.push(Prop { object_index: i, shape, body, spawn, last_written: spawn, dormant: true });
        }

        // One step builds the broad phase, so ray queries work from the first frame.
        world.step();
        let player = world.insert_body(RigidBodyBuilder::kinematic_position_based().pose(Pose::from_translation(Vec3::new(0.0, -10.0, 0.0))));
        PropWorld { world, props, by_body, player, player_collider: None, player_dims: (0.0, 0.0), held: None }
    }

    /// Top-level object indices that are loose props (remove them from the player's static
    /// collider/ground lists and from static hit shapes: they move).
    pub fn movable_indices(&self) -> HashSet<usize> {
        self.props.iter().map(|p| p.object_index).collect()
    }

    /// The loose props.
    pub fn props(&self) -> &[Prop] {
        &self.props
    }

    /// Index into [`props`](Self::props) of the prop for scene object `object_index`.
    pub fn prop_of_object(&self, object_index: usize) -> Option<usize> {
        self.props.iter().position(|p| p.object_index == object_index)
    }

    /// Whether `prop` is asleep (at rest).
    pub fn is_asleep(&self, prop: usize) -> bool {
        let p = &self.props[prop];
        p.dormant || self.world.bodies[p.body].is_sleeping()
    }

    /// The prop's current pose (its object's origin frame) in world space.
    pub fn prop_pose(&self, prop: usize) -> Mat4 {
        self.world.bodies[self.props[prop].body].position().to_mat4()
    }

    /// How many props are awake right now.
    pub fn awake_count(&self) -> usize {
        (0..self.props.len()).filter(|&i| !self.is_asleep(i)).count()
    }

    /// Places the player's kinematic cylinder (feet at `foot`), which shoves whatever it touches.
    pub fn set_player(&mut self, foot: Vec3, radius: f32, height: f32) {
        if self.player_dims != (radius, height) {
            if let Some(c) = self.player_collider.take() {
                self.world.remove_collider(c);
            }
            self.player_collider = Some(self.world.insert_collider(ColliderBuilder::cylinder(height * 0.5, radius).friction(0.3), Some(self.player)));
            self.player_dims = (radius, height);
        }
        let center = foot + Vec3::new(0.0, height * 0.5, 0.0);
        self.world.bodies[self.player].set_next_kinematic_position(Pose::from_translation(center));
    }

    /// Advances the simulation one fixed step ([`crate::player::FIXED_DT`]).
    pub fn step(&mut self) {
        self.wake_disturbed();
        self.world.step();
        let held = self.held.as_ref().map(|h| h.prop);
        for (i, p) in self.props.iter().enumerate() {
            let b = &mut self.world.bodies[p.body];
            if Some(i) == held || p.dormant || b.is_sleeping() {
                continue;
            }
            if b.translation().y < KILL_Y {
                b.set_position(pose_of(p.spawn), true);
                b.set_linvel(Vec3::ZERO, true);
                b.set_angvel(Vec3::ZERO, true);
            } else if b.linvel().length() > MAX_SPEED {
                let v = b.linvel().normalize() * MAX_SPEED;
                b.set_linvel(v, true);
            }
        }
    }

    /// Turns a dormant prop dynamic, and (breadth-first, a few links deep) everything touching or
    /// resting on it, so a moved table takes its lamp with it and a lifted crate drops what was on top.
    pub fn activate(&mut self, prop: usize) {
        let mut queue = vec![prop];
        let mut done = 0;
        while let Some(i) = queue.pop() {
            if !self.props[i].dormant || done >= 40 {
                continue;
            }
            done += 1;
            self.props[i].dormant = false;
            let handle = self.props[i].body;
            let touching = self.body_aabb(handle).map(|a| self.props_in(loosen(a, 0.05))).unwrap_or_default();
            self.world.bodies[handle].set_body_type(RigidBodyType::Dynamic, true);
            queue.extend(touching);
        }
    }

    /// Union of a body's collider bounds.
    fn body_aabb(&self, body: RigidBodyHandle) -> Option<Aabb> {
        let mut out: Option<Aabb> = None;
        for &c in self.world.bodies[body].colliders() {
            let a = self.world.colliders[c].compute_aabb();
            out = Some(match out {
                Some(o) => Aabb::new(o.mins.min(a.mins), o.maxs.max(a.maxs)),
                None => a,
            });
        }
        out
    }

    /// Dormant props whose colliders may intersect `aabb`.
    fn props_in(&self, aabb: Aabb) -> Vec<usize> {
        let mut out = Vec::new();
        for (_, c) in self.world.intersect_aabb_conservative(aabb, QueryFilter::default().exclude_rigid_body(self.player)) {
            if let Some(&p) = c.parent().and_then(|b| self.by_body.get(&b)) {
                if self.props[p].dormant && !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }

    /// Wakes dormant props the player is touching or that a moving prop has run into.
    fn wake_disturbed(&mut self) {
        let mut hit: Vec<usize> = Vec::new();
        if let Some(c) = self.player_collider {
            hit.extend(self.props_in(loosen(self.world.colliders[c].compute_aabb(), 0.03)));
        }
        let moving: Vec<RigidBodyHandle> = self
            .props
            .iter()
            .filter(|p| !p.dormant && !self.world.bodies[p.body].is_sleeping() && self.world.bodies[p.body].is_enabled() && self.world.bodies[p.body].linvel().length_squared() > 0.0025)
            .map(|p| p.body)
            .collect();
        for b in moving {
            if let Some(a) = self.body_aabb(b) {
                hit.extend(self.props_in(loosen(a, 0.04)));
            }
        }
        for p in hit {
            self.activate(p);
        }
    }

    /// Writes every prop's current pose into its scene object (only when it changed).
    pub fn sync_scene(&mut self, scene: &mut Scene) {
        for i in 0..self.props.len() {
            let m = match &self.held {
                Some(h) if h.prop == i => h.pose,
                _ => self.world.bodies[self.props[i].body].position().to_mat4(),
            };
            let p = &mut self.props[i];
            if m != p.last_written {
                write_pose(&mut scene.objects[p.object_index], m);
                p.last_written = m;
            }
        }
    }

    /// The nearest thing a ray meets among *everything* solid (so a wall in the way hides a prop):
    /// `Some(prop)` only if that thing is a loose prop.
    fn first_prop_hit(&self, origin: Vec3, dir: Vec3, reach: f32) -> Option<(usize, f32)> {
        let ray = Ray::new(origin, dir.normalize_or_zero());
        let filter = QueryFilter::default().exclude_rigid_body(self.player);
        let (col, toi) = self.world.cast_ray(&ray, reach, true, filter)?;
        let body = self.world.colliders[col].parent()?;
        self.by_body.get(&body).map(|&p| (p, toi))
    }

    /// The prop under a ray from `origin` that `limits` allows lifting, within `reach` (`None` while
    /// already carrying something).
    pub fn pick_target(&self, origin: Vec3, dir: Vec3, reach: f32, limits: &CarryLimits) -> Option<usize> {
        if self.held.is_some() {
            return None;
        }
        let (p, _) = self.first_prop_hit(origin, dir, reach)?;
        self.props[p].shape.carriable(limits).then_some(p)
    }

    /// The nearest loose prop a ray touches, ignoring fixed geometry (the caller compares with the
    /// static hit distance): `(prop, distance)`.
    pub fn ray_props(&self, origin: Vec3, dir: Vec3, reach: f32) -> Option<(usize, f32)> {
        let ray = Ray::new(origin, dir.normalize_or_zero());
        // Loose props only — dormant ones are *fixed* bodies, so filter by "belongs to a prop", not by body type.
        let is_prop = |_: ColliderHandle, c: &rapier3d::prelude::Collider| c.parent().is_some_and(|b| self.by_body.contains_key(&b));
        let filter = QueryFilter::default().exclude_rigid_body(self.player).predicate(&is_prop);
        let (col, toi) = self.world.cast_ray(&ray, reach, true, filter)?;
        let body = self.world.colliders[col].parent()?;
        self.by_body.get(&body).map(|&p| (p, toi))
    }

    /// Whacks `prop` (a bat swing): an impulse along `dir` at `point`, scaled so light things fly and
    /// heavy ones just shuffle.
    pub fn strike(&mut self, prop: usize, dir: Vec3, point: Vec3) {
        let mass = self.mass(prop);
        self.strike_impulse(prop, dir, point, 6.0 * mass.min(4.0));
    }

    /// Mass of a prop, kg.
    pub fn mass(&self, prop: usize) -> f32 {
        self.world.bodies[self.props[prop].body].mass()
    }

    /// Gives `prop` an impulse of `magnitude` N·s along `dir` at `point` (a bullet, a shove).
    pub fn strike_impulse(&mut self, prop: usize, dir: Vec3, point: Vec3, magnitude: f32) {
        self.activate(prop);
        self.world.bodies[self.props[prop].body].apply_impulse_at_point(dir.normalize_or_zero() * magnitude, point, true);
    }

    /// The prop being carried, if any.
    pub fn held(&self) -> Option<usize> {
        self.held.as_ref().map(|h| h.prop)
    }

    /// Picks `prop` up: its body is switched off and the object follows [`set_held_pose`](Self::set_held_pose).
    pub fn pick_up(&mut self, prop: usize) {
        if self.held.is_some() {
            return;
        }
        // Whatever rests on it starts to fall the moment it is lifted away.
        self.activate(prop);
        let pose = self.world.bodies[self.props[prop].body].position().to_mat4();
        self.world.bodies[self.props[prop].body].set_enabled(false);
        self.held = Some(Held { prop, pose });
    }

    /// Moves the carried object (its origin frame) to `pose`.
    pub fn set_held_pose(&mut self, pose: Mat4) {
        if let Some(h) = &mut self.held {
            h.pose = pose;
        }
    }

    /// Lets go: the prop re-enters the simulation where it is, moving at `velocity`.
    pub fn drop_held(&mut self, velocity: Vec3) -> Option<usize> {
        let h = self.held.take()?;
        let b = &mut self.world.bodies[self.props[h.prop].body];
        b.set_enabled(true);
        b.set_position(pose_of(h.pose), true);
        b.set_linvel(velocity, true);
        b.set_angvel(Vec3::ZERO, true);
        Some(h.prop)
    }

    /// Distance to the nearest fixed surface along a horizontal ray, up to `max` (for keeping a
    /// carried object out of walls).
    pub fn wall_distance(&self, origin: Vec3, dir: Vec3, max: f32) -> f32 {
        let ray = Ray::new(origin, dir.normalize_or_zero());
        self.world.cast_ray(&ray, max, true, QueryFilter::only_fixed()).map_or(max, |(_, t)| t)
    }

    /// Where to hold `prop` so it sits in front of a player at `eye` facing `forward` (horizontal),
    /// upright, `drop` metres below eye level, pulled in if a wall is close. Returns the object's
    /// origin-frame transform. `radius` is the player's collision radius, `floor_y` their feet.
    pub fn hold_pose(&self, prop: usize, eye: Vec3, forward: Vec3, radius: f32, drop: f32, floor_y: f32) -> Mat4 {
        let s = self.props[prop].shape;
        let fwd = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
        let fwd = if fwd == Vec3::ZERO { Vec3::Z } else { fwd };
        let reach_r = 0.5 * s.extents.x.max(s.extents.z);
        let wanted = radius + reach_r + 0.12;
        let clear = self.wall_distance(eye, fwd, wanted + reach_r + 0.05);
        let dist = wanted.min((clear - reach_r - 0.03).max(reach_r * 0.5));
        let mut c = eye + fwd * dist - Vec3::Y * drop;
        c.y = c.y.max(floor_y + s.extents.y * 0.5 + 0.02);
        let rot = Quat::from_rotation_y(fwd.x.atan2(fwd.z));
        Mat4::from_rotation_translation(rot, c - rot * s.center)
    }
}

fn write_pose(o: &mut Object, m: Mat4) {
    let (_, r, t) = m.to_scale_rotation_translation();
    let (x, y, z) = r.to_euler(EulerRot::XYZ);
    o.position = Track::constant(t);
    o.rotation = Track::constant(Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees()));
}

// ---------------------------------------------------------------------------------------------
// Fixed colliders
// ---------------------------------------------------------------------------------------------

fn add_box(world: &mut PhysicsWorld, world_mat: Mat4, size: Vec3) {
    let (sc, rot, tr) = world_mat.to_scale_rotation_translation();
    let h = (size * 0.5 * sc.abs()).max(Vec3::splat(0.004));
    world.insert_collider(ColliderBuilder::cuboid(h.x, h.y, h.z).position(pose_of(Mat4::from_rotation_translation(rot, tr))).friction(0.8), None);
}

/// Adds `o`'s solid parts as fixed colliders, following the same rules as the player's own colliders
/// (`viewer::collect_box_colliders`): boxes block, props use their footprint policy, stairs give
/// real treads, floor planes are thin slabs and `"collide": false` objects don't count — with one
/// difference: round primitives are solid to props (see below).
fn add_static(world: &mut PhysicsWorld, o: &Object, parent: Mat4) {
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
            if let Some((lo, hi)) = collision_box(p.kind) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(objects: &str) -> Scene {
        let text = format!(
            r##"{{"camera":{{"position":[0,1.7,5],"target":[0,1,0]}},"objects":[
                {{"id":"floor","type":"box","position":[0,-0.1,0],"size":[40,0.2,40],"material":{{"color":"#888888"}}}},
                {objects}]}}"##
        );
        crate::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}"))
    }

    fn settle(w: &mut PropWorld, scene: &mut Scene, ticks: usize) {
        for _ in 0..ticks {
            w.step();
        }
        w.sync_scene(scene);
    }

    fn pos(scene: &Scene, id: &str) -> Vec3 {
        scene.objects.iter().find(|o| o.id == id).unwrap().position.sample(0.0)
    }

    fn up_y(scene: &Scene, id: &str) -> f32 {
        let o = scene.objects.iter().find(|o| o.id == id).unwrap();
        let r = o.rotation.sample(0.0);
        (Quat::from_euler(EulerRot::XYZ, r.x.to_radians(), r.y.to_radians(), r.z.to_radians()) * Vec3::Y).y
    }

    #[test]
    fn classification_follows_what_a_person_can_lift() {
        let s = scene(
            r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}},
                {"id":"fridge","type":"prop","prop":"refrigerator","position":[3,0,0],"material":{"color":"#dddddd"}},
                {"id":"sofa","type":"prop","prop":"sofa","position":[5,0,0],"material":{"color":"#dddddd"}},
                {"id":"toilet","type":"prop","prop":"toilet","position":[7,0,0],"material":{"color":"#ffffff"}},
                {"id":"apple","type":"prefab","prefab":"apple_red","position":[0,0,3]},
                {"id":"tree","type":"prop","prop":"tree_oak","position":[9,0,0],"material":{"color":"#336633"}},
                {"id":"pinned","type":"prop","prop":"chair","position":[11,0,0],"movable":false,"material":{"color":"#336633"}}"##,
        );
        let by = |id: &str| classify(s.objects.iter().find(|o| o.id == id).unwrap());
        assert!(by("crate").is_some() && by("apple").is_some());
        assert!(by("fridge").is_none() && by("sofa").is_none() && by("toilet").is_none() && by("tree").is_none());
        assert!(by("pinned").is_none(), "movable:false wins");
        assert!(by("apple").unwrap().carriable(&RAT_CARRY), "a rat can carry an apple");
        assert!(!by("crate").unwrap().carriable(&RAT_CARRY), "but not a crate");
        assert!(by("crate").unwrap().carriable(&HUMAN_CARRY));
    }

    #[test]
    fn a_dropped_crate_falls_and_comes_to_rest_on_the_floor() {
        let mut s = scene(r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}}"##);
        let mut w = PropWorld::new(&s, None);
        w.set_player(Vec3::new(4.0, 0.0, 4.0), 0.35, 1.75);
        w.pick_up(0);
        w.set_held_pose(Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0)));
        w.sync_scene(&mut s);
        assert!((pos(&s, "crate").y - 1.5).abs() < 1e-4, "carried object follows the hold pose");
        w.drop_held(Vec3::ZERO);
        settle(&mut w, &mut s, 240);
        let p = pos(&s, "crate");
        assert!(p.y.abs() < 0.03, "rests on the floor (origin at its base), got y = {}", p.y);
        assert!(up_y(&s, "crate") > 0.99, "and is still upright");
        assert!(w.is_asleep(0), "at rest it sleeps again");
    }

    #[test]
    fn a_shoved_crate_topples_a_fire_extinguisher() {
        let mut s = scene(
            r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}},
                {"id":"cone","type":"prop","prop":"fire_extinguisher","position":[0.9,0,0],"material":{"color":"#cc2222"}}"##,
        );
        let mut w = PropWorld::new(&s, None);
        w.set_player(Vec3::new(6.0, 0.0, 6.0), 0.35, 1.75);
        let cone_before = pos(&s, "cone");
        // Let go of the crate just above the floor while moving toward the cone (a shove / a throw).
        w.pick_up(0);
        w.set_held_pose(Mat4::from_translation(Vec3::new(-0.2, 0.15, 0.0)));
        w.drop_held(Vec3::new(4.0, 0.0, 0.0));
        settle(&mut w, &mut s, 300);
        let moved = (pos(&s, "cone") - cone_before).length();
        assert!(moved > 0.2 || up_y(&s, "cone") < 0.8, "the cone was knocked (moved {moved}, up {})", up_y(&s, "cone"));
        assert!(pos(&s, "cone").y > -0.05, "and it did not fall through the floor");
    }

    #[test]
    fn a_dropped_crate_lands_on_and_pushes_a_small_apple_off_a_table() {
        // A crate stands in for a table top at 0.56 m; the apple sits on its edge.
        let mut s = scene(
            r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"movable":false,"material":{"color":"#a07040"}},
                {"id":"apple","type":"prefab","prefab":"apple_red","position":[0.26,0.56,0]},
                {"id":"box2","type":"prop","prop":"box_stack","position":[0.0,0,3],"material":{"color":"#a07040"}}"##,
        );
        let mut w = PropWorld::new(&s, None);
        w.set_player(Vec3::new(6.0, 0.0, 6.0), 0.35, 1.75);
        let apple = w.prop_of_object(2).expect("the apple is loose");
        assert!(w.is_asleep(apple), "it sits still until touched");
        // Bump the apple's neighbour: a box dropped beside it rolls it off the edge.
        let b = w.prop_of_object(3).expect("box_stack is loose");
        w.pick_up(b);
        w.set_held_pose(Mat4::from_translation(Vec3::new(0.62, 0.7, 0.0)));
        w.drop_held(Vec3::new(-1.5, 0.0, 0.0));
        settle(&mut w, &mut s, 300);
        let a = pos(&s, "apple");
        assert!(a.y < 0.3, "the apple ended up on the floor, not still on the ledge: {a:?}");
    }

    #[test]
    fn a_bat_hit_sends_a_light_prop_flying_and_only_shuffles_a_heavy_one() {
        let mut s = scene(
            r##"{"id":"apple","type":"prefab","prefab":"apple_red","position":[0,0,0]},
                {"id":"crate","type":"prop","prop":"crate","position":[3,0,0],"material":{"color":"#a07040"}}"##,
        );
        let mut w = PropWorld::new(&s, None);
        w.set_player(Vec3::new(9.0, 0.0, 9.0), 0.35, 1.75);
        let (apple, crate_) = (w.prop_of_object(1).unwrap(), w.prop_of_object(2).unwrap());
        w.strike(apple, Vec3::X, Vec3::new(-0.03, 0.05, 0.0));
        w.strike(crate_, Vec3::X, Vec3::new(2.7, 0.3, 0.0));
        for _ in 0..90 {
            w.step();
        }
        w.sync_scene(&mut s);
        let (a, c) = (pos(&s, "apple").x, pos(&s, "crate").x - 3.0);
        assert!(a > 1.0, "the apple flew: {a}");
        assert!(c > 0.0 && c < a, "the crate only shuffled: {c} vs {a}");
    }

    #[test]
    fn rays_find_dormant_props_too_so_a_bat_or_bullet_can_hit_something_nobody_has_touched() {
        let s = scene(r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,-3],"material":{"color":"#a07040"}}"##);
        let w = PropWorld::new(&s, None);
        assert!(w.is_asleep(0), "untouched, so still dormant");
        let hit = w.ray_props(Vec3::new(0.0, 0.3, 0.0), -Vec3::Z, 10.0).expect("a ray finds the dormant crate");
        assert_eq!(hit.0, 0);
        assert!((hit.1 - 2.72).abs() < 0.05, "distance to its near face: {}", hit.1);
    }

    #[test]
    fn walking_into_a_small_prop_pushes_it() {
        let mut s = scene(r##"{"id":"cone","type":"prop","prop":"traffic_cone","position":[0,0,0],"material":{"color":"#ff6a00"}}"##);
        let mut w = PropWorld::new(&s, None);
        for i in 0..90 {
            let x = -1.0 + i as f32 * 0.02;
            w.set_player(Vec3::new(x, 0.0, 0.0), 0.35, 1.75);
            w.step();
        }
        w.sync_scene(&mut s);
        assert!(pos(&s, "cone").x > 0.2, "the cone was shoved along by the walker: {:?}", pos(&s, "cone"));
    }

    #[test]
    fn a_wall_hides_a_prop_from_the_pick_up_ray_and_a_held_prop_is_not_pickable() {
        let s = scene(
            r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,-3],"material":{"color":"#a07040"}},
                {"id":"wall","type":"box","position":[0,1,-1.5],"size":[6,2,0.2],"material":{"color":"#dddddd"}}"##,
        );
        let mut w = PropWorld::new(&s, None);
        let eye = Vec3::new(0.0, 0.5, 0.0);
        assert!(w.pick_target(eye, -Vec3::Z, 5.0, &HUMAN_CARRY).is_none(), "wall in the way");
        assert!(w.pick_target(Vec3::new(0.0, 0.5, -2.0), -Vec3::Z, 5.0, &HUMAN_CARRY).is_some(), "clear line of sight");
        w.pick_up(0);
        w.step(); // disabling a body takes effect on the next step
        assert!(w.pick_target(Vec3::new(0.0, 0.5, -2.0), -Vec3::Z, 5.0, &HUMAN_CARRY).is_none(), "already carrying");
        assert!(w.ray_props(Vec3::new(0.0, 0.5, -2.0), -Vec3::Z, 5.0).is_none(), "a carried prop can't be batted");
    }
}
