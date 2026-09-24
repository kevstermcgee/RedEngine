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
//! floors, stair treads and prop footprints become fixed colliders. Each loose object starts as a
//! **static instance** (`sim::statics`): fixed colliders at the pose the map author chose — solid, and
//! hit by bats and bullets — but *no rigid body and no entity*, so a whole map of clutter costs almost
//! nothing and never shuffles at load. The first time something disturbs one (the player walking into
//! it, a bat, a bullet, a moving prop touching it, being picked up) it is **promoted**: a dynamic body
//! takes over its colliders, it gets an entity with a tracked transform, and so does whatever rests on
//! or touches it. The player is a kinematic cylinder that shoves whatever it walks into. Carrying
//! disables the body and pins the object in front of the player; dropping re-enables it with the
//! player's momentum, so it falls, bounces, tumbles and knocks smaller things about.
//!
//! No window or GPU types here (ADR 0010): the physics is callable headless. The player's own
//! walking still uses `crate::player` / `crate::viewer` (ADR 0003); loose props are removed from
//! *those* collider lists (see [`PropWorld::movable_indices`]) and handled here instead.

use crate::hit::object_leaves;
use crate::props::game_collision_boxes;
use crate::render::{build_stairs_parts, trs};
use crate::schema::{Object, ObjectKind, PrimKind, Scene};
use crate::sim::change::{ChangeCursor, GenClock};
use crate::sim::components::Transform;
use crate::sim::entities::{Entities, EntityId};
use crate::sim::scratch::ScratchVec;
use crate::sim::statics::{DynamicProp, PropState, StaticInstance};
use crate::track::Track;
use glam::{EulerRot, Mat4, Quat, Vec3};
use rapier3d::prelude::{Aabb, ColliderBuilder, ColliderHandle, PhysicsWorld, Pose, QueryFilter, Ray, RigidBodyBuilder, RigidBodyHandle, SharedShape};
use std::collections::HashSet;

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

/// One loose object: what it is in the scene, and whether it is still a cheap static instance or
/// has been promoted to a dynamic entity (see `sim::statics`).
pub struct Prop {
    /// Index into `scene.objects`.
    pub object_index: usize,
    /// Bounding box in the object's frame.
    pub shape: PropShape,
    /// Where the map author put it (respawn point, and the frame its colliders are relative to).
    spawn: Mat4,
    state: PropState,
}

struct Held {
    prop: usize,
    pose: Mat4,
}

/// One player's kinematic cylinder.
struct PlayerBody {
    body: RigidBodyHandle,
    collider: Option<ColliderHandle>,
    dims: (f32, f32),
}

/// `user_data` of a player's collider: it is neither a prop (0 = map geometry, `id + 1` = a prop) nor
/// something a ray or an AABB query for props should return.
const PLAYER_TAG: u128 = u128::MAX;

/// The rigid-body world for a map's loose props (see the module docs).
///
/// Prop ids (`usize`) index [`props`](Self::props); every prop collider carries `id + 1` in its
/// `user_data` (0 = not a prop), which is how a ray hit or an AABB query maps back to a prop.
pub struct PropWorld {
    world: PhysicsWorld,
    props: Vec<Prop>,
    /// Colliders of every static instance, flat; each [`StaticInstance`] owns a range of it.
    collider_pool: Vec<ColliderHandle>,
    /// Ids of promoted props, in promotion order (the only props the per-tick loops visit).
    dynamic: Vec<usize>,
    /// Dynamic entities and their tracked transforms.
    entities: Entities,
    /// `entity slot -> prop id`.
    entity_prop: Vec<usize>,
    clock: GenClock,
    /// What `sync_scene` (the renderer) has already written to the scene.
    render_cursor: ChangeCursor,
    /// Kinematic bodies of the players (slot = player id); slot 0 always exists (single-player).
    players: Vec<Option<PlayerBody>>,
    held: Option<Held>,
    scratch: PropScratch,
}

/// Per-tick temporary lists, reused every tick instead of allocated (ADR 0014, `sim::scratch`).
#[derive(Default)]
struct PropScratch {
    /// Props to wake this tick.
    hit: ScratchVec<usize>,
    /// Awake, moving bodies whose neighbourhood is checked for static props.
    moving: ScratchVec<RigidBodyHandle>,
    /// [`PropWorld::activate`]'s breadth-first queue.
    queue: ScratchVec<usize>,
    /// Static props found touching the one being promoted.
    touching: ScratchVec<usize>,
    /// The props [`PropWorld::activate`] decided to promote, in order.
    promote: ScratchVec<usize>,
    /// Entity slots whose transform changed since the last `sync_scene`.
    changed: ScratchVec<usize>,
}

/// The prop id a collider's `user_data` names, if it belongs to a prop.
fn prop_of(user_data: u128) -> Option<usize> {
    if user_data == PLAYER_TAG {
        return None;
    }
    user_data.checked_sub(1).map(|p| p as usize)
}

fn not_a_player(_: ColliderHandle, c: &rapier3d::prelude::Collider) -> bool {
    c.user_data != PLAYER_TAG
}

/// The scene's loose props in the order [`PropWorld`] numbers them: `(object index, shape)`. A
/// networked client uses this to map the prop ids in a snapshot to its own scene objects without
/// building a physics world.
pub fn loose_props(scene: &Scene, skip: Option<usize>) -> Vec<(usize, PropShape)> {
    scene.objects.iter().enumerate().filter(|(i, _)| Some(*i) != skip).filter_map(|(i, o)| classify(o).map(|s| (i, s))).collect()
}

/// Writes a world pose into an object's (constant) position and rotation tracks.
pub fn set_object_pose(o: &mut Object, position: Vec3, rotation: Quat) {
    write_pose(o, Mat4::from_rotation_translation(rotation, position));
}

fn transform_of(m: Mat4) -> Transform {
    let (_, rotation, position) = m.to_scale_rotation_translation();
    Transform { position, rotation }
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

/// A prop collider: the one recipe used both when it is a static instance and when it is re-attached
/// to a body on promotion, so the two can never drift apart.
fn prop_collider(shape: SharedShape, pose: Mat4, prop: usize) -> ColliderBuilder {
    ColliderBuilder::new(shape).position(pose_of(pose)).density(PROP_DENSITY).friction(0.7).restitution(0.2).user_data(prop as u128 + 1)
}

impl PropWorld {
    /// Builds the world for `scene`: fixed colliders for the solid map, a static instance (fixed
    /// colliders, no body) for every loose prop, and the player's kinematic body. `skip` is a
    /// top-level object index to leave out entirely (the player's own body model).
    pub fn new(scene: &Scene, skip: Option<usize>) -> Self {
        let mut world = PhysicsWorld::default();
        world.gravity = Vec3::new(0.0, -PROP_GRAVITY, 0.0);
        world.integration_parameters.dt = crate::player::FIXED_DT;
        // Interpenetrating things (a globe placed a hair into its table, two chairs pushed together)
        // separate gently instead of being fired apart.
        world.integration_parameters.normalized_max_corrective_velocity = 1.5;

        // A floor under everything, so nothing can ever fall out of the map.
        world.insert_collider(ColliderBuilder::cuboid(500.0, 0.5, 500.0).position(Pose::from_translation(Vec3::new(0.0, -0.5, 0.0))).friction(0.8), None);

        let loose = loose_props(scene, skip);
        let loose_set: HashSet<usize> = loose.iter().map(|(i, _)| *i).collect();

        for (i, o) in scene.objects.iter().enumerate() {
            if Some(i) != skip && !loose_set.contains(&i) {
                add_static(&mut world, o, Mat4::IDENTITY);
            }
        }

        let mut props = Vec::with_capacity(loose.len());
        let mut collider_pool = Vec::new();
        for (i, shape) in loose {
            let o = &scene.objects[i];
            let spawn = object_body_mat(o);
            let s = Mat4::from_scale(object_scale(o));
            let start = collider_pool.len() as u32;
            // A static instance: fixed colliders at the authored pose, no rigid body. `activate`
            // re-attaches them to a dynamic body the first time something disturbs the prop.
            for (leaf, local) in object_leaves(o) {
                let (sh, rel) = leaf_collider(&leaf, s * local);
                collider_pool.push(world.insert_collider(prop_collider(sh, spawn * rel, props.len()), None));
            }
            let collider_count = collider_pool.len() as u32 - start;
            props.push(Prop { object_index: i, shape, spawn, state: PropState::Static(StaticInstance { collider_start: start, collider_count }) });
        }

        // One step builds the broad phase, so ray queries work from the first frame.
        world.step();
        let player = world.insert_body(RigidBodyBuilder::kinematic_position_based().pose(Pose::from_translation(Vec3::new(0.0, -10.0, 0.0))));
        PropWorld {
            world,
            props,
            collider_pool,
            dynamic: Vec::new(),
            entities: Entities::default(),
            entity_prop: Vec::new(),
            clock: GenClock::default(),
            render_cursor: ChangeCursor::default(),
            players: vec![Some(PlayerBody { body: player, collider: None, dims: (0.0, 0.0) })],
            held: None,
            scratch: PropScratch::default(),
        }
    }

    /// Top-level object indices that are loose props (remove them from the player's static
    /// collider/ground lists and from static hit shapes: they move).
    pub fn movable_indices(&self) -> HashSet<usize> {
        self.props.iter().map(|p| p.object_index).collect()
    }

    /// Times a per-tick scratch buffer had to grow (each is one allocation). Zero in steady state;
    /// `tests/alloc_budget.rs` asserts it stops changing.
    pub fn scratch_grows(&self) -> u32 {
        let s = &self.scratch;
        s.hit.grows() + s.moving.grows() + s.queue.grows() + s.touching.grows() + s.promote.grows() + s.changed.grows()
    }

    /// The loose props.
    pub fn props(&self) -> &[Prop] {
        &self.props
    }

    /// Index into [`props`](Self::props) of the prop for scene object `object_index`.
    pub fn prop_of_object(&self, object_index: usize) -> Option<usize> {
        self.props.iter().position(|p| p.object_index == object_index)
    }

    /// Whether `prop` is still a static instance (untouched: no body, no entity).
    pub fn is_static(&self, prop: usize) -> bool {
        matches!(self.props[prop].state, PropState::Static(_))
    }

    /// The rigid body of a promoted prop.
    fn body_of(&self, prop: usize) -> Option<RigidBodyHandle> {
        match self.props[prop].state {
            PropState::Dynamic(d) => Some(d.body),
            PropState::Static(_) => None,
        }
    }

    /// Number of props promoted to dynamic entities so far.
    pub fn dynamic_count(&self) -> usize {
        self.dynamic.len()
    }

    /// The prop id of the entity in tracked-transform slot `slot` (the reverse of [`entity_of`](Self::entity_of)).
    pub fn prop_of_entity(&self, slot: usize) -> usize {
        self.entity_prop[slot]
    }

    /// Number of rigid bodies in the physics world (one per player plus one per promoted prop).
    pub fn body_count(&self) -> usize {
        self.world.bodies.len()
    }

    /// The dynamic entities (tracked transforms) — what a network layer would iterate.
    pub fn entities(&self) -> &Entities {
        &self.entities
    }

    /// The current generation clock and a way to close it: for a system (e.g. network) that reads
    /// [`entities`](Self::entities) with its own `ChangeCursor`.
    pub fn clock_mut(&mut self) -> &mut GenClock {
        &mut self.clock
    }

    /// The dynamic entity of `prop`, if it has been promoted.
    pub fn entity_of(&self, prop: usize) -> Option<EntityId> {
        match self.props[prop].state {
            PropState::Dynamic(d) => Some(d.entity),
            PropState::Static(_) => None,
        }
    }

    /// Whether `prop` is asleep (at rest). A static instance always is.
    pub fn is_asleep(&self, prop: usize) -> bool {
        match self.props[prop].state {
            PropState::Static(_) => true,
            PropState::Dynamic(d) => self.world.bodies[d.body].is_sleeping(),
        }
    }

    /// The prop's current pose (its object's origin frame) in world space.
    pub fn prop_pose(&self, prop: usize) -> Mat4 {
        match self.props[prop].state {
            PropState::Static(_) => self.props[prop].spawn,
            PropState::Dynamic(d) => self.world.bodies[d.body].position().to_mat4(),
        }
    }

    /// How many props are awake right now.
    pub fn awake_count(&self) -> usize {
        self.dynamic.iter().filter(|&&p| !self.is_asleep(p)).count()
    }

    /// Places the (single-player) player's kinematic cylinder (feet at `foot`), which shoves whatever it
    /// touches. Same as [`set_player_slot`](Self::set_player_slot) with slot 0.
    pub fn set_player(&mut self, foot: Vec3, radius: f32, height: f32) {
        self.set_player_slot(0, foot, radius, height);
    }

    /// Places player `slot`'s kinematic cylinder (creating its body the first time), which shoves
    /// whatever it touches. Players do not collide with each other here.
    pub fn set_player_slot(&mut self, slot: usize, foot: Vec3, radius: f32, height: f32) {
        if self.players.len() <= slot {
            self.players.resize_with(slot + 1, || None);
        }
        if self.players[slot].is_none() {
            let body = self.world.insert_body(RigidBodyBuilder::kinematic_position_based().pose(Pose::from_translation(foot)));
            self.players[slot] = Some(PlayerBody { body, collider: None, dims: (0.0, 0.0) });
        }
        let pb = self.players[slot].as_mut().expect("just created");
        if pb.dims != (radius, height) {
            if let Some(c) = pb.collider.take() {
                self.world.remove_collider(c);
            }
            pb.collider = Some(self.world.insert_collider(ColliderBuilder::cylinder(height * 0.5, radius).friction(0.3).user_data(PLAYER_TAG), Some(pb.body)));
            pb.dims = (radius, height);
        }
        let center = foot + Vec3::new(0.0, height * 0.5, 0.0);
        self.world.bodies[pb.body].set_next_kinematic_position(Pose::from_translation(center));
    }

    /// Removes player `slot`'s body (a player left). Slot numbers are not reused by this call.
    pub fn remove_player_slot(&mut self, slot: usize) {
        if let Some(pb) = self.players.get_mut(slot).and_then(Option::take) {
            self.world.remove_body(pb.body);
        }
    }

    /// How many player bodies exist.
    pub fn player_count(&self) -> usize {
        self.players.iter().flatten().count()
    }

    /// Advances the simulation one fixed step ([`crate::player::FIXED_DT`]). Only promoted props are
    /// visited; their new poses are published to their tracked transforms.
    pub fn step(&mut self) {
        self.wake_disturbed();
        self.world.step();
        let held = self.held.as_ref().map(|h| h.prop);
        for k in 0..self.dynamic.len() {
            let i = self.dynamic[k];
            let PropState::Dynamic(d) = self.props[i].state else { continue };
            if Some(i) == held {
                continue;
            }
            // Look with `&` first: taking `&mut` on a body marks it modified, which makes rapier
            // re-process it (and allocate) every tick, even for props that are asleep.
            if self.world.bodies[d.body].is_sleeping() {
                if !d.settled {
                    // Fell asleep since the last tick: publish its resting pose once, then it is free.
                    self.entities.transforms.set(d.entity.slot(), transform_of(self.world.bodies[d.body].position().to_mat4()), self.clock.now());
                    self.set_settled(i, true);
                }
                continue;
            }
            let b = &mut self.world.bodies[d.body];
            if b.translation().y < KILL_Y {
                b.set_position(pose_of(self.props[i].spawn), true);
                b.set_linvel(Vec3::ZERO, true);
                b.set_angvel(Vec3::ZERO, true);
            } else if b.linvel().length() > MAX_SPEED {
                let v = b.linvel().normalize() * MAX_SPEED;
                b.set_linvel(v, true);
            }
            self.entities.transforms.set(d.entity.slot(), transform_of(b.position().to_mat4()), self.clock.now());
            if d.settled {
                self.set_settled(i, false);
            }
        }
    }

    fn set_settled(&mut self, prop: usize, settled: bool) {
        if let PropState::Dynamic(d) = &mut self.props[prop].state {
            d.settled = settled;
        }
    }

    /// Promotes a static prop to a dynamic entity, and (breadth-first, a few links deep) every static
    /// prop touching or resting on it, so a moved table takes its lamp with it and a lifted crate
    /// drops what was on top. Already-dynamic props are left alone.
    pub fn activate(&mut self, prop: usize) {
        if !self.is_static(prop) {
            return;
        }
        let mut queue = self.scratch.queue.take();
        let mut touching = self.scratch.touching.take();
        let mut order = self.scratch.promote.take();
        // Phase 1: decide who is promoted, querying the world as it is (nothing has been changed yet).
        queue.push(prop);
        while let Some(i) = queue.pop() {
            if !self.is_static(i) || order.contains(&i) || order.len() >= 40 {
                continue;
            }
            order.push(i);
            touching.clear();
            if let Some(a) = self.static_aabb(i) {
                self.props_in(loosen(a, 0.05), &mut touching);
            }
            queue.extend(touching.iter().copied());
        }
        // Phase 2: promote them.
        for &i in &order {
            self.promote(i);
        }
        self.scratch.queue.give_back(queue);
        self.scratch.touching.give_back(touching);
        self.scratch.promote.give_back(order);
    }

    /// Turns one static instance into a dynamic entity: a new body at its authored pose takes over
    /// its colliders (re-created relative to the body), and it gets an entity slot.
    fn promote(&mut self, prop: usize) {
        let PropState::Static(st) = self.props[prop].state else { return };
        let spawn = self.props[prop].spawn;
        let body = self.world.insert_body(RigidBodyBuilder::dynamic().pose(pose_of(spawn)).linear_damping(0.15).angular_damping(0.8).ccd_enabled(true));
        let to_body = spawn.inverse();
        for k in st.collider_range() {
            if let Some(c) = self.world.remove_collider(self.collider_pool[k]) {
                let rel = to_body * c.position().to_mat4();
                self.world.insert_collider(prop_collider(c.shared_shape().clone(), rel, prop), Some(body));
            }
        }
        let entity = self.entities.spawn(transform_of(spawn), self.clock.now());
        debug_assert_eq!(entity.slot(), self.entity_prop.len());
        self.entity_prop.push(prop);
        self.props[prop].state = PropState::Dynamic(DynamicProp { body, entity, settled: false });
        self.dynamic.push(prop);
    }

    /// Union of a static prop's collider bounds.
    fn static_aabb(&self, prop: usize) -> Option<Aabb> {
        let PropState::Static(st) = self.props[prop].state else { return None };
        let mut out: Option<Aabb> = None;
        for k in st.collider_range() {
            let a = self.world.colliders[self.collider_pool[k]].compute_aabb();
            out = Some(match out {
                Some(o) => Aabb::new(o.mins.min(a.mins), o.maxs.max(a.maxs)),
                None => a,
            });
        }
        out
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

    /// Appends (without duplicates) the static props whose colliders may intersect `aabb` to `out`.
    fn props_in(&self, aabb: Aabb, out: &mut Vec<usize>) {
        for (_, c) in self.world.intersect_aabb_conservative(aabb, QueryFilter::default().predicate(&not_a_player)) {
            if let Some(p) = prop_of(c.user_data) {
                if self.is_static(p) && !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }

    /// Promotes static props the player is touching or that a moving prop has run into.
    fn wake_disturbed(&mut self) {
        let mut hit = self.scratch.hit.take();
        let mut moving = self.scratch.moving.take();
        for pb in self.players.iter().flatten() {
            if let Some(c) = pb.collider {
                self.props_in(loosen(self.world.colliders[c].compute_aabb(), 0.03), &mut hit);
            }
        }
        for &i in &self.dynamic {
            if let PropState::Dynamic(d) = self.props[i].state {
                let b = &self.world.bodies[d.body];
                if !b.is_sleeping() && b.is_enabled() && b.linvel().length_squared() > 0.0025 {
                    moving.push(d.body);
                }
            }
        }
        for &b in &moving {
            if let Some(a) = self.body_aabb(b) {
                self.props_in(loosen(a, 0.04), &mut hit);
            }
        }
        for &p in &hit {
            self.activate(p);
        }
        self.scratch.hit.give_back(hit);
        self.scratch.moving.give_back(moving);
    }

    /// Writes into `scene` the pose of every prop that moved since the last call (plus the carried
    /// one, which follows the hold pose). Static props are never touched.
    pub fn sync_scene(&mut self, scene: &mut Scene) {
        let held = self.held.as_ref().map(|h| h.prop);
        if let Some(h) = &self.held {
            write_pose(&mut scene.objects[self.props[h.prop].object_index], h.pose);
        }
        let mut changed = self.scratch.changed.take();
        self.entities.transforms.collect_changed_since(self.render_cursor.last(), &mut changed);
        for &slot in &changed {
            let prop = self.entity_prop[slot];
            if Some(prop) == held {
                continue;
            }
            let t = self.entities.transforms.get(slot);
            write_pose(&mut scene.objects[self.props[prop].object_index], Mat4::from_rotation_translation(t.rotation, t.position));
        }
        self.scratch.changed.give_back(changed);
        self.render_cursor.catch_up(&mut self.clock);
    }

    /// The nearest thing a ray meets among *everything* solid (so a wall in the way hides a prop):
    /// `Some(prop)` only if that thing is a loose prop.
    fn first_prop_hit(&self, origin: Vec3, dir: Vec3, reach: f32) -> Option<(usize, f32)> {
        let ray = Ray::new(origin, dir.normalize_or_zero());
        let filter = QueryFilter::default().predicate(&not_a_player);
        let (col, toi) = self.world.cast_ray(&ray, reach, true, filter)?;
        prop_of(self.world.colliders[col].user_data).map(|p| (p, toi))
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
        // Static instances have no body, so filter by "is a prop collider", not by body type.
        let is_prop = |_: ColliderHandle, c: &rapier3d::prelude::Collider| prop_of(c.user_data).is_some();
        let filter = QueryFilter::default().predicate(&is_prop);
        let (col, toi) = self.world.cast_ray(&ray, reach, true, filter)?;
        prop_of(self.world.colliders[col].user_data).map(|p| (p, toi))
    }

    /// Whacks `prop` (a bat swing): an impulse along `dir` at `point`, scaled so light things fly and
    /// heavy ones just shuffle.
    pub fn strike(&mut self, prop: usize, dir: Vec3, point: Vec3) {
        let mass = self.mass(prop);
        self.strike_impulse(prop, dir, point, 6.0 * mass.min(4.0));
    }

    /// Mass of a prop, kg.
    pub fn mass(&self, prop: usize) -> f32 {
        match self.props[prop].state {
            PropState::Dynamic(d) => self.world.bodies[d.body].mass(),
            PropState::Static(st) => st.collider_range().map(|k| self.world.colliders[self.collider_pool[k]].mass()).sum(),
        }
    }

    /// Gives `prop` an impulse of `magnitude` N·s along `dir` at `point` (a bullet, a shove). Promotes
    /// it first if it is still static.
    pub fn strike_impulse(&mut self, prop: usize, dir: Vec3, point: Vec3, magnitude: f32) {
        self.activate(prop);
        if let Some(body) = self.body_of(prop) {
            self.world.bodies[body].apply_impulse_at_point(dir.normalize_or_zero() * magnitude, point, true);
        }
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
        let Some(body) = self.body_of(prop) else { return };
        let pose = self.world.bodies[body].position().to_mat4();
        self.world.bodies[body].set_enabled(false);
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
        let body = self.body_of(h.prop)?;
        let b = &mut self.world.bodies[body];
        b.set_enabled(true);
        b.set_position(pose_of(h.pose), true);
        b.set_linvel(velocity, true);
        b.set_angvel(Vec3::ZERO, true);
        self.set_settled(h.prop, false);
        Some(h.prop)
    }

    /// Distance to the nearest fixed surface along a horizontal ray, up to `max` (for keeping a
    /// carried object out of walls).
    pub fn wall_distance(&self, origin: Vec3, dir: Vec3, max: f32) -> f32 {
        let ray = Ray::new(origin, dir.normalize_or_zero());
        self.world.cast_ray(&ray, max, true, QueryFilter::only_fixed()).map_or(max, |(_, t)| t)
    }

    /// Where to hold `prop` so it sits in front of a player at `eye` looking along `look`: upright,
    /// pulled in if a wall is close. Looking **up** lifts it (overhead when looking straight up, kept
    /// under any ceiling); looking down lowers it toward the floor. At level gaze it sits `drop` metres
    /// below eye level, like a carried box. Returns the object's origin-frame transform. `radius` is the
    /// player's collision radius, `floor_y` their feet.
    pub fn hold_pose(&self, prop: usize, eye: Vec3, look: Vec3, radius: f32, drop: f32, floor_y: f32) -> Mat4 {
        let s = self.props[prop].shape;
        let look = look.normalize_or_zero();
        let flat = Vec3::new(look.x, 0.0, look.z).normalize_or_zero();
        let flat = if flat == Vec3::ZERO { Vec3::Z } else { flat };
        let (sin_p, cos_p) = (look.y.clamp(-1.0, 1.0), (1.0 - look.y * look.y).max(0.0).sqrt());
        let reach_r = 0.5 * s.extents.x.max(s.extents.z);
        let wanted = radius + reach_r + 0.12;
        let clear = self.wall_distance(eye, flat, wanted + reach_r + 0.05);
        let dist = wanted.min((clear - reach_r - 0.03).max(reach_r * 0.5));
        // Horizontal reach shrinks as the gaze rises (overhead is right above you); the vertical part
        // follows the pitch, and the level-gaze `drop` fades out as you look up.
        let mut c = eye + flat * dist * cos_p.max(0.2) + Vec3::Y * (wanted * sin_p) - Vec3::Y * drop * (1.0 - sin_p.max(0.0));
        // Never through a ceiling; never below the floor (the floor wins).
        let up_room = self.wall_distance(eye, Vec3::Y, 3.0);
        c.y = c.y.min(eye.y + up_room - s.extents.y * 0.5 - 0.03);
        c.y = c.y.max(floor_y + s.extents.y * 0.5 + 0.02);
        let rot = Quat::from_rotation_y(flat.x.atan2(flat.z));
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

    // ---- static instances and promotion (ADR 0014, step 4) ----------------------------------------

    const THREE_APART: &str = r##"{"id":"a","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}},
        {"id":"b","type":"prop","prop":"crate","position":[6,0,0],"material":{"color":"#a07040"}},
        {"id":"c","type":"prop","prop":"barrel","position":[0,0,6],"material":{"color":"#3a6ea5"}}"##;

    #[test]
    fn untouched_props_have_no_body_and_no_entity() {
        let s = scene(THREE_APART);
        let mut w = PropWorld::new(&s, None);
        w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
        for _ in 0..120 {
            w.step();
        }
        assert_eq!(w.props().len(), 3);
        assert!((0..3).all(|i| w.is_static(i) && w.entity_of(i).is_none() && w.is_asleep(i)));
        assert_eq!(w.dynamic_count(), 0);
        assert_eq!(w.entities().len(), 0, "no entities for static props");
        assert_eq!(w.body_count(), 1, "only the player's body exists");
    }

    #[test]
    fn touching_one_prop_promotes_only_that_one() {
        let s = scene(THREE_APART);
        let mut w = PropWorld::new(&s, None);
        w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
        w.activate(1);
        assert_eq!((w.dynamic_count(), w.body_count(), w.entities().len()), (1, 2, 1));
        assert!(!w.is_static(1) && w.is_static(0) && w.is_static(2));
        assert_eq!(w.entity_of(1).map(|e| e.slot()), Some(0));
        w.activate(1); // idempotent
        assert_eq!((w.dynamic_count(), w.body_count()), (1, 2));
    }

    #[test]
    fn a_promoted_prop_is_where_it_was_and_still_solid_and_hittable() {
        let mut s = scene(THREE_APART);
        let mut w = PropWorld::new(&s, None);
        w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
        let before = w.prop_pose(1);
        w.activate(1);
        assert!(before.abs_diff_eq(w.prop_pose(1), 1e-6), "promotion must not move it");
        for _ in 0..90 {
            w.step();
        }
        w.sync_scene(&mut s);
        assert!((pos(&s, "b") - Vec3::new(6.0, 0.0, 0.0)).length() < 0.02, "it rests where it was authored: {:?}", pos(&s, "b"));
        assert!(w.is_asleep(1), "and falls asleep again");
        let hit = w.ray_props(Vec3::new(6.0, 0.3, 4.0), -Vec3::Z, 10.0).expect("a ray still hits the promoted crate");
        assert_eq!(hit.0, 1);
        let hit = w.ray_props(Vec3::new(0.0, 0.3, 4.0), -Vec3::Z, 10.0).expect("and a static one");
        assert_eq!(hit.0, 0);
    }

    #[test]
    fn promoting_a_table_promotes_what_rests_on_it() {
        let s = scene(
            r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"movable":true,"material":{"color":"#a07040"}},
                {"id":"apple","type":"prefab","prefab":"apple_red","position":[0,0.56,0]},
                {"id":"far","type":"prop","prop":"crate","position":[8,0,0],"material":{"color":"#a07040"}}"##,
        );
        let mut w = PropWorld::new(&s, None);
        w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
        let crate_id = w.prop_of_object(1).unwrap();
        w.activate(crate_id);
        assert_eq!(w.dynamic_count(), 2, "the crate and its apple");
        assert!(w.is_static(w.prop_of_object(3).unwrap()), "but not the crate across the room");
    }

    #[test]
    fn sync_scene_never_touches_static_props_and_writes_moved_ones_once() {
        let mut s = scene(THREE_APART);
        let mut w = PropWorld::new(&s, None);
        w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
        // Sentinel positions: sync must leave the static ones alone.
        for id in ["a", "c"] {
            let o = s.objects.iter_mut().find(|o| o.id == id).unwrap();
            o.position = Track::constant(Vec3::splat(123.0));
        }
        w.strike_impulse(1, Vec3::X, Vec3::new(6.0, 0.3, 0.0), 6.0);
        w.step();
        w.sync_scene(&mut s);
        assert_eq!(pos(&s, "a"), Vec3::splat(123.0));
        assert_eq!(pos(&s, "c"), Vec3::splat(123.0));
        assert!((pos(&s, "b") - Vec3::new(6.0, 0.0, 0.0)).length() > 0.0, "the struck crate moved");
        // With nothing moving, a second sync writes nothing (it does not overwrite a sentinel).
        for _ in 0..600 {
            w.step();
        }
        w.sync_scene(&mut s);
        let o = s.objects.iter_mut().find(|o| o.id == "b").unwrap();
        o.position = Track::constant(Vec3::splat(-9.0));
        w.sync_scene(&mut s);
        assert_eq!(pos(&s, "b"), Vec3::splat(-9.0), "a sleeping prop is not rewritten");
    }

    #[test]
    fn a_sleeping_promoted_prop_publishes_its_resting_pose_then_goes_quiet() {
        use crate::sim::change::ChangeCursor;
        let s = scene(THREE_APART);
        let mut w = PropWorld::new(&s, None);
        w.set_player(Vec3::new(30.0, 0.0, 30.0), 0.35, 1.75);
        w.pick_up(1);
        w.set_held_pose(Mat4::from_translation(Vec3::new(6.0, 1.0, 0.0)));
        w.drop_held(Vec3::ZERO);
        for _ in 0..300 {
            w.step();
        }
        assert!(w.is_asleep(1));
        let slot = w.entity_of(1).unwrap().slot();
        let published = *w.entities().transforms.get(slot);
        let actual = w.prop_pose(1).to_scale_rotation_translation();
        assert!(published.position.abs_diff_eq(actual.2, 1e-5) && published.rotation.abs_diff_eq(actual.1, 1e-5), "the final pose was published");
        // A network-style reader sees it once, then nothing while the prop sleeps.
        let mut cursor = ChangeCursor::default();
        assert!(w.entities().transforms.changed_since(cursor.last()));
        cursor.catch_up(w.clock_mut());
        for _ in 0..60 {
            w.step();
        }
        assert!(!w.entities().transforms.changed_since(cursor.last()), "a sleeping prop generates no changes");
    }

    // ---- carrying: where you look decides how high the held prop rides ----------------------------

    #[test]
    fn looking_up_lifts_a_carried_prop_and_looking_down_lowers_it() {
        let s = scene(r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}}"##);
        let w = PropWorld::new(&s, None);
        let eye = Vec3::new(0.0, 1.7, 3.0);
        let base_y = |look: Vec3| w.hold_pose(0, eye, look, 0.35, 0.55, 0.0).w_axis.y;
        let level = base_y(Vec3::new(0.0, 0.0, -1.0));
        let up45 = base_y(Vec3::new(0.0, 1.0, -1.0));
        let overhead = base_y(Vec3::Y);
        let down = base_y(Vec3::new(0.0, -1.0, -0.4));
        assert!((level - (1.7 - 0.55 - w.props()[0].shape.extents.y * 0.5)).abs() < 0.05, "level gaze keeps the old carry height: {level}");
        assert!(up45 > level + 0.3, "looking up 45 degrees lifts it: {level} -> {up45}");
        assert!(overhead > up45, "straight up is higher still: {overhead}");
        assert!(overhead > 2.0, "high enough to hold well over your head: {overhead}");
        assert!(down < level, "looking down lowers it");
        // Looking straight up puts it over the player, not out in front.
        let over = w.hold_pose(0, eye, Vec3::Y, 0.35, 0.55, 0.0).w_axis;
        assert!((over.z - eye.z).abs() < 0.3, "overhead, not out in front: z {}", over.z);
    }

    #[test]
    fn a_ceiling_stops_a_lifted_prop_and_the_floor_stops_a_lowered_one() {
        let s = scene(
            r##"{"id":"crate","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#a07040"}},
                {"id":"ceil","type":"box","position":[0,2.7,3],"size":[8,0.2,8],"material":{"color":"#888888"}}"##,
        );
        let w = PropWorld::new(&s, None);
        let ext = w.props()[0].shape.extents.y;
        let eye = Vec3::new(0.0, 1.7, 3.0);
        let top = w.hold_pose(0, eye, Vec3::Y, 0.35, 0.55, 0.0).w_axis.y + ext;
        assert!(top <= 2.6, "the ceiling is at 2.6 m; the crate's top is at {top}");
        let low = w.hold_pose(0, eye, Vec3::new(0.0, -1.0, -0.1), 0.35, 0.55, 0.0).w_axis.y;
        assert!(low >= -0.001, "never below the floor: {low}");
    }
}
