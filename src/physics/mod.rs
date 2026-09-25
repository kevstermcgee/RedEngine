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

use crate::geometry::{build_stairs_parts, trs};
use crate::hit::object_leaves;
use crate::props::game_collision_boxes;
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

mod classify;
mod fixed;
mod interact;

use classify::object_scale;
pub use classify::{classify, local_bounds, CarryLimits, PropShape, HUMAN_CARRY, RAT_CARRY};
use fixed::add_static;

/// Gravity acting on loose props, m/s^2 (a little brisker than 9.81 so drops feel snappy, like the
/// player's own `player::GRAVITY`).
pub const PROP_GRAVITY: f32 = 14.0;
/// Density of every prop collider, kg/m^3 (furniture and clutter are mostly hollow or light).
pub const PROP_DENSITY: f32 = 120.0;
/// Props never move faster than this, m/s (a bat swing or a thrown-off crate stays sane).
const MAX_SPEED: f32 = 14.0;
/// A prop that falls below this height (out of the map) is put back where it started.
const KILL_Y: f32 = -30.0;

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
    /// The player slot carrying it (slot 0 in single-player).
    holder: usize,
    prop: usize,
    pose: Mat4,
}

/// One player's kinematic cylinder.
struct PlayerBody {
    /// Where the kinematic body was last placed: an unchanged target is not re-set (rapier re-processes, and allocates
    /// for, every body it is told about, so an idle player must cost nothing).
    last_center: Option<Vec3>,
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
    /// Props being carried, one per player at most.
    held: Vec<Held>,
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
            players: vec![Some(PlayerBody { last_center: None, body: player, collider: None, dims: (0.0, 0.0) })],
            held: Vec::new(),
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
            self.players[slot] = Some(PlayerBody { last_center: None, body, collider: None, dims: (0.0, 0.0) });
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
        if pb.last_center != Some(center) {
            pb.last_center = Some(center);
            self.world.bodies[pb.body].set_next_kinematic_position(Pose::from_translation(center));
        }
    }

    /// Removes player `slot`'s body (a player left). Slot numbers are not reused by this call.
    pub fn remove_player_slot(&mut self, slot: usize) {
        self.drop_held_by(slot, Vec3::ZERO); // a leaving player lets go of what they carry
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
        for h in 0..self.held.len() {
            // A carried prop has no active body: publish where it is held so a network snapshot sees it move.
            if let PropState::Dynamic(d) = self.props[self.held[h].prop].state {
                let pose = self.held[h].pose;
                self.entities.transforms.set(d.entity.slot(), transform_of(pose), self.clock.now());
            }
        }
        for k in 0..self.dynamic.len() {
            let i = self.dynamic[k];
            let PropState::Dynamic(d) = self.props[i].state else { continue };
            if self.is_held(i) {
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
        for h in &self.held {
            write_pose(&mut scene.objects[self.props[h.prop].object_index], h.pose);
        }
        let mut changed = self.scratch.changed.take();
        self.entities.transforms.collect_changed_since(self.render_cursor.last(), &mut changed);
        for &slot in &changed {
            let prop = self.entity_prop[slot];
            if self.is_held(prop) {
                continue;
            }
            let t = self.entities.transforms.get(slot);
            write_pose(&mut scene.objects[self.props[prop].object_index], Mat4::from_rotation_translation(t.rotation, t.position));
        }
        self.scratch.changed.give_back(changed);
        self.render_cursor.catch_up(&mut self.clock);
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

#[cfg(test)]
mod tests;
