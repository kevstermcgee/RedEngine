//! Carrying and striking: what a player can do to a loose prop (`impl PropWorld` continued from `mod.rs`).
//!
//! Pick-up targeting (`pick_target_for`), the per-holder carry state (`pick_up_by` / `drop_held_by` / `set_held_pose_for`, so
//! every player can carry one prop and nobody can take a held one), ray queries against props, bat and bullet impulses, and
//! where a carried prop sits in front of its holder (`hold_pose`).

use super::*;

impl PropWorld {
    /// The top of the highest loose prop under a player standing at `pos` (feet at `foot_y`, body `radius`) that they can
    /// step or stand onto (within `step_up` above their feet), or `None`. Probes straight down at the centre and four points
    /// around the rim; players and the carried prop are ignored. Lets single-player stand on, and jump off, crates and barrels.
    pub fn floor_under(&self, pos: glam::Vec2, foot_y: f32, radius: f32, step_up: f32) -> Option<f32> {
        let is_prop = |_: ColliderHandle, c: &rapier3d::prelude::Collider| prop_of(c.user_data).is_some_and(|p| !self.is_held(p));
        let filter = QueryFilter::default().predicate(&is_prop);
        let mut best: Option<f32> = None;
        let r = radius * 0.7;
        for (dx, dz) in [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)] {
            let from = Vec3::new(pos.x + dx, foot_y + step_up, pos.y + dz);
            let ray = Ray::new(from, -Vec3::Y);
            // A ray that starts inside a prop (toi 0) means its top is above the step-up limit: not a floor.
            if let Some((_, toi)) = self.world.cast_ray(&ray, step_up + 0.05, true, filter).filter(|(_, toi)| *toi > 1e-4) {
                let top = from.y - toi;
                if top >= foot_y - 0.05 {
                    best = Some(best.map_or(top, |b: f32| b.max(top)));
                }
            }
        }
        best
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
        self.pick_target_for(0, origin, dir, reach, limits)
    }

    /// [`pick_target`](Self::pick_target) for player `holder`: `None` while they already carry something, and never a
    /// prop someone else is carrying (pick-up contention is decided here, on the authoritative side).
    pub fn pick_target_for(&self, holder: usize, origin: Vec3, dir: Vec3, reach: f32, limits: &CarryLimits) -> Option<usize> {
        if self.held_by(holder).is_some() {
            return None;
        }
        let (p, _) = self.first_prop_hit(origin, dir, reach)?;
        (self.props[p].shape.carriable(limits) && !self.is_held(p)).then_some(p)
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

    /// The prop the local (slot 0) player is carrying, if any.
    pub fn held(&self) -> Option<usize> {
        self.held_by(0)
    }

    /// The prop player `holder` is carrying, if any.
    pub fn held_by(&self, holder: usize) -> Option<usize> {
        self.held.iter().find(|h| h.holder == holder).map(|h| h.prop)
    }

    /// Whether anyone is carrying `prop`.
    pub fn is_held(&self, prop: usize) -> bool {
        self.held.iter().any(|h| h.prop == prop)
    }

    /// Who is carrying `prop`, if anyone.
    pub fn holder_of(&self, prop: usize) -> Option<usize> {
        self.held.iter().find(|h| h.prop == prop).map(|h| h.holder)
    }

    /// Picks `prop` up as the local player (slot 0): see [`pick_up_by`](Self::pick_up_by).
    pub fn pick_up(&mut self, prop: usize) {
        self.pick_up_by(0, prop);
    }

    /// Player `holder` picks `prop` up: its body is switched off and the object follows
    /// [`set_held_pose_for`](Self::set_held_pose_for). Refused (`false`) if they already carry something or anyone
    /// carries this prop.
    pub fn pick_up_by(&mut self, holder: usize, prop: usize) -> bool {
        if self.held_by(holder).is_some() || self.is_held(prop) {
            return false;
        }
        // Whatever rests on it starts to fall the moment it is lifted away.
        self.activate(prop);
        let Some(body) = self.body_of(prop) else { return false };
        let pose = self.world.bodies[body].position().to_mat4();
        self.world.bodies[body].set_enabled(false);
        self.held.push(Held { holder, prop, pose });
        true
    }

    /// Moves the local player's carried object (its origin frame) to `pose`.
    pub fn set_held_pose(&mut self, pose: Mat4) {
        self.set_held_pose_for(0, pose);
    }

    /// Moves what `holder` carries to `pose`.
    pub fn set_held_pose_for(&mut self, holder: usize, pose: Mat4) {
        if let Some(h) = self.held.iter_mut().find(|h| h.holder == holder) {
            h.pose = pose;
        }
    }

    /// The local player lets go: see [`drop_held_by`](Self::drop_held_by).
    pub fn drop_held(&mut self, velocity: Vec3) -> Option<usize> {
        self.drop_held_by(0, velocity)
    }

    /// `holder` lets go: the prop re-enters the simulation where it is, moving at `velocity`.
    pub fn drop_held_by(&mut self, holder: usize, velocity: Vec3) -> Option<usize> {
        let i = self.held.iter().position(|h| h.holder == holder)?;
        let h = self.held.remove(i);
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
        let rot = Quat::from_rotation_y(libm::atan2f(flat.x, flat.z));
        Mat4::from_rotation_translation(rot, c - rot * s.center)
    }
}
