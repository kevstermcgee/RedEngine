//! The player's body: a box 0.6 wide and 1.8 tall (1.5 sneaking) moving through the block grid as the original's does. Gravity pulls it at
//! 32 blocks/s² (a jump clears 1.25 blocks); it walks at 4.3 blocks/s, sprints at 5.6, sneaks at 1.3 and will not walk off an edge while
//! sneaking; it swims (slow, floating up while jump is held) and, in creative, flies. It collides one axis at a time (up and down first, then
//! across), so it slides along walls; with auto-jump it hops a one-block step it walks into.

use crate::block::Block;
use crate::world::World;
use glam::{IVec3, Vec2, Vec3};

pub const HALF_WIDTH: f32 = 0.3;
pub const HEIGHT: f32 = 1.8;
pub const SNEAK_HEIGHT: f32 = 1.5;
pub const EYE: f32 = 1.62;
pub const SNEAK_EYE: f32 = 1.27;
pub const GRAVITY: f32 = 32.0;
pub const TERMINAL: f32 = 78.0;
/// Clears 1.25 blocks with the 60 Hz step (v0^2 / 2g less half a step's travel).
pub const JUMP: f32 = 9.2;
pub const WALK: f32 = 4.317;
pub const SPRINT: f32 = 5.612;
pub const SNEAK: f32 = 1.31;
pub const FLY: f32 = 10.9;
pub const FLY_SPRINT: f32 = 21.8;

/// What the player asks of the body this tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// Forward (+1) and right (+1), each -1..1.
    pub forward: f32,
    pub strafe: f32,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
    /// Facing, radians (0 looks north, -Z).
    pub yaw: f32,
}

/// The body's state.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    /// The middle of the feet.
    pub pos: Vec3,
    pub vel: Vec3,
    pub on_ground: bool,
    pub flying: bool,
    pub sneaking: bool,
    pub sprinting: bool,
    /// The feet are in water (or lava).
    pub in_water: bool,
    pub in_lava: bool,
    /// The eyes are under water.
    pub head_in_water: bool,
    /// The highest point of the current fall, for the damage when it ends.
    pub fall_from: f32,
    /// Hops a one-block step walked into.
    pub auto_jump: bool,
    /// How far the body has walked on the ground (for footsteps).
    pub walked: f32,
    /// Half its width and its height (a player 0.3 and 1.8; a pig, a zombie or a dropped item its own).
    pub half: f32,
    pub tall: f32,
    /// A tick in which a jump started (for the sound).
    pub jumped: bool,
}

/// What a tick ended in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Landing {
    /// Blocks fallen when it hit the ground this tick (0: no landing).
    pub fell: f32,
    /// It splashed into water this tick.
    pub splash: bool,
}

impl Body {
    pub fn new(pos: Vec3) -> Body {
        Body {
            pos,
            vel: Vec3::ZERO,
            on_ground: false,
            flying: false,
            sneaking: false,
            sprinting: false,
            in_water: false,
            in_lava: false,
            head_in_water: false,
            fall_from: pos.y,
            auto_jump: true,
            walked: 0.0,
            jumped: false,
            half: HALF_WIDTH,
            tall: HEIGHT,
        }
    }

    /// A body of another size (a mob, a dropped item).
    pub fn sized(pos: Vec3, half: f32, tall: f32) -> Body {
        Body { half, tall, ..Body::new(pos) }
    }

    pub fn height(&self) -> f32 {
        if self.sneaking && !self.flying {
            self.tall - (HEIGHT - SNEAK_HEIGHT)
        } else {
            self.tall
        }
    }

    pub fn eye_height(&self) -> f32 {
        if self.sneaking && !self.flying {
            SNEAK_EYE
        } else {
            EYE
        }
    }

    /// The box: min and max corners.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        bounds_at(self.pos, self.half, self.height())
    }

    /// One tick of `dt` seconds.
    pub fn step(&mut self, world: &World, c: &Controls, dt: f32) -> Landing {
        let mut landing = Landing::default();
        self.jumped = false;
        let was_water = self.in_water;
        self.in_water = touches(world, self.pos, self.half, 0.4, |b| b.is_water());
        self.in_lava = touches(world, self.pos, self.half, 0.4, |b| b == Block::LAVA);
        let eye = self.pos + Vec3::Y * self.eye_height();
        self.head_in_water = world.block(eye.x.floor() as i32, eye.y.floor() as i32, eye.z.floor() as i32).is_water();
        if self.in_water && !was_water && self.vel.y < -4.0 {
            landing.splash = true;
        }
        // A sneaking flyer goes down instead.
        self.sneaking = c.sneak && !self.flying;
        let wish = {
            let (s, cs) = c.yaw.sin_cos();
            let fwd = Vec2::new(s, -cs);
            let right = Vec2::new(cs, s);
            let v = fwd * c.forward + right * c.strafe;
            if v.length() > 1.0 {
                v.normalize()
            } else {
                v
            }
        };
        self.sprinting = c.sprint && c.forward > 0.3 && !self.sneaking && (self.sprinting || self.on_ground || self.flying || self.in_water);
        let liquid = self.in_water || self.in_lava;
        let speed = if self.flying {
            if self.sprinting {
                FLY_SPRINT
            } else {
                FLY
            }
        } else if self.sneaking {
            SNEAK
        } else if self.sprinting {
            SPRINT
        } else {
            WALK
        } * if liquid && !self.flying {
            if self.in_lava {
                0.35
            } else {
                0.5
            }
        } else {
            1.0
        };
        let target = wish * speed;
        // How quickly the body reaches the speed asked for: at once on the ground, slowly in the air.
        let grip = if self.flying {
            10.0
        } else if self.on_ground {
            if wish == Vec2::ZERO {
                22.0
            } else {
                16.0
            }
        } else if liquid {
            6.0
        } else {
            3.2
        };
        let k = (grip * dt).min(1.0);
        let horiz = Vec2::new(self.vel.x, self.vel.z);
        let horiz = horiz + (target - horiz) * k;
        self.vel.x = horiz.x;
        self.vel.z = horiz.y;

        if self.flying {
            let up = (c.jump as i32 - c.sneak as i32) as f32 * 7.5;
            self.vel.y += (up - self.vel.y) * (12.0 * dt).min(1.0);
        } else if liquid {
            // Sink slowly, float up while jump is held, and climb out at an edge.
            let target_y = if c.jump { 2.6 } else { -1.8 };
            self.vel.y += (target_y - self.vel.y) * (4.0 * dt).min(1.0);
            if c.jump && self.blocked_ahead(world, wish) {
                self.vel.y = self.vel.y.max(4.2);
            }
        } else {
            if c.jump && self.on_ground {
                self.vel.y = JUMP;
                self.jumped = true;
                if self.sprinting {
                    self.vel.x += wish.x * 2.0;
                    self.vel.z += wish.y * 2.0;
                }
            } else if self.auto_jump && self.on_ground && wish != Vec2::ZERO && self.step_ahead(world, wish) {
                self.vel.y = JUMP;
                self.jumped = true;
            }
            self.vel.y = (self.vel.y - GRAVITY * dt).max(-TERMINAL);
        }

        let before_ground = self.on_ground;
        let d = self.vel * dt;
        // Up and down first.
        let (moved_y, hit_y) = self.sweep(world, 1, d.y);
        self.pos.y += moved_y;
        self.on_ground = hit_y && d.y < 0.0;
        if hit_y {
            self.vel.y = 0.0;
        }
        for axis in [0usize, 2] {
            let want = if axis == 0 { d.x } else { d.z };
            if want == 0.0 {
                continue;
            }
            let (moved, hit) = self.sweep(world, axis, want);
            let mut moved = moved;
            // Sneaking on the ground never walks off an edge.
            if self.sneaking && self.on_ground && moved != 0.0 {
                let mut probe = self.pos;
                probe[axis] += moved;
                if !self.ground_under(world, probe) {
                    moved = 0.0;
                }
            }
            self.pos[axis] += moved;
            if hit || moved != want {
                if axis == 0 {
                    self.vel.x = 0.0;
                } else {
                    self.vel.z = 0.0;
                }
            }
        }
        if self.on_ground {
            self.walked += Vec2::new(d.x, d.z).length();
        }
        // Falls: from the highest point since the feet left the ground; water and flight forgive them.
        if self.on_ground || liquid || self.flying {
            if self.on_ground && !before_ground && !liquid && !self.flying {
                landing.fell = (self.fall_from - self.pos.y).max(0.0);
            }
            self.fall_from = self.pos.y;
        } else {
            self.fall_from = self.fall_from.max(self.pos.y);
        }
        if self.flying && self.on_ground {
            self.flying = false;
        }
        landing
    }

    /// How far the box can move along `axis` by `d` before it touches a solid block, and whether it did.
    fn sweep(&self, world: &World, axis: usize, d: f32) -> (f32, bool) {
        if d == 0.0 {
            return (0.0, false);
        }
        let (lo, hi) = self.bounds();
        let mut lo2 = lo;
        let mut hi2 = hi;
        if d > 0.0 {
            hi2[axis] += d;
        } else {
            lo2[axis] += d;
        }
        let eps = 1e-4;
        let a = (lo2 + Vec3::splat(eps)).floor().as_ivec3();
        let b = (hi2 - Vec3::splat(eps)).floor().as_ivec3();
        let mut allowed = d;
        let mut hit = false;
        for y in a.y..=b.y {
            for z in a.z..=b.z {
                for x in a.x..=b.x {
                    let p = IVec3::new(x, y, z);
                    if !solid_at(world, p) {
                        continue;
                    }
                    // A block the box already overlaps does not stop it (it is pushed out by the others).
                    let (blo, bhi) = (p.as_vec3(), p.as_vec3() + Vec3::ONE);
                    let overlaps_other = (0..3).filter(|&k| k != axis).all(|k| lo[k] < bhi[k] - eps && hi[k] > blo[k] + eps);
                    if !overlaps_other {
                        continue;
                    }
                    if d > 0.0 {
                        if blo[axis] >= hi[axis] - eps {
                            let gap = blo[axis] - hi[axis];
                            if gap < allowed {
                                allowed = gap.max(0.0);
                                hit = true;
                            }
                        }
                    } else if bhi[axis] <= lo[axis] + eps {
                        let gap = bhi[axis] - lo[axis];
                        if gap > allowed {
                            allowed = gap.min(0.0);
                            hit = true;
                        }
                    }
                }
            }
        }
        (allowed, hit)
    }

    /// Whether something solid would hold the box up at `pos` (within a little over half a block down).
    fn ground_under(&self, world: &World, pos: Vec3) -> bool {
        let (lo, hi) = bounds_at(pos, self.half, self.height());
        let y = (lo.y - 0.6).floor() as i32;
        let (x0, x1) = ((lo.x + 1e-3).floor() as i32, (hi.x - 1e-3).floor() as i32);
        let (z0, z1) = ((lo.z + 1e-3).floor() as i32, (hi.z - 1e-3).floor() as i32);
        (x0..=x1).any(|x| (z0..=z1).any(|z| (y..=lo.y.floor() as i32 - 1).any(|yy| solid_at(world, IVec3::new(x, yy, z)))))
    }

    /// A one-block step right ahead with room above it (auto-jump).
    fn step_ahead(&self, world: &World, wish: Vec2) -> bool {
        let dir = wish.normalize_or_zero();
        let probe = self.pos + Vec3::new(dir.x, 0.0, dir.y) * (self.half + 0.35);
        let foot = IVec3::new(probe.x.floor() as i32, (self.pos.y + 0.01).floor() as i32, probe.z.floor() as i32);
        let low = solid_at(world, foot);
        let head_room = (1..=2).all(|k| !solid_at(world, foot + IVec3::Y * k))
            && !solid_at(world, IVec3::new(self.pos.x.floor() as i32, (self.pos.y + self.height() + 0.5).floor() as i32, self.pos.z.floor() as i32));
        low && head_room
    }

    fn blocked_ahead(&self, world: &World, wish: Vec2) -> bool {
        if wish == Vec2::ZERO {
            return false;
        }
        let dir = wish.normalize();
        let probe = self.pos + Vec3::new(dir.x, 0.0, dir.y) * (self.half + 0.2);
        solid_at(world, IVec3::new(probe.x.floor() as i32, (self.pos.y + 0.5).floor() as i32, probe.z.floor() as i32))
    }

    /// Whether the box at its place overlaps nothing solid.
    pub fn fits(&self, world: &World) -> bool {
        let (lo, hi) = self.bounds();
        let a = (lo + Vec3::splat(1e-3)).floor().as_ivec3();
        let b = (hi - Vec3::splat(1e-3)).floor().as_ivec3();
        for y in a.y..=b.y {
            for z in a.z..=b.z {
                for x in a.x..=b.x {
                    if solid_at(world, IVec3::new(x, y, z)) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Whether a block placed at `cell` would overlap the body.
    pub fn overlaps_cell(&self, cell: IVec3) -> bool {
        let (lo, hi) = self.bounds();
        let (blo, bhi) = (cell.as_vec3(), cell.as_vec3() + Vec3::ONE);
        (0..3).all(|k| lo[k] < bhi[k] - 1e-3 && hi[k] > blo[k] + 1e-3)
    }
}

fn bounds_at(pos: Vec3, half: f32, height: f32) -> (Vec3, Vec3) {
    (pos - Vec3::new(half, 0.0, half), pos + Vec3::new(half, height, half))
}

/// Whether the cell holds something the body collides with (a chunk that is not loaded yet is solid, so nobody falls out of the world).
fn solid_at(world: &World, p: IVec3) -> bool {
    if p.y >= crate::chunk::H {
        return false;
    }
    if p.y < 0 || !world.chunks.contains_key(&crate::chunk::ChunkPos::of(p.x, p.z)) {
        return true;
    }
    world.block(p.x, p.y, p.z).solid()
}

/// Whether any cell the box of half height `h` at `pos` touches holds a block that `f` picks.
fn touches(world: &World, pos: Vec3, half: f32, h: f32, f: impl Fn(Block) -> bool) -> bool {
    let (lo, hi) = (pos - Vec3::new(half, 0.0, half), pos + Vec3::new(half, h, half));
    let a = (lo + Vec3::splat(1e-3)).floor().as_ivec3();
    let b = (hi - Vec3::splat(1e-3)).floor().as_ivec3();
    for y in a.y..=b.y {
        for z in a.z..=b.z {
            for x in a.x..=b.x {
                if f(world.block(x, y, z)) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{ChunkData, ChunkPos};
    use crate::world::Slot;
    use std::sync::Arc;

    /// A flat world: stone up to y 9 (the ground at y 10), and `extra` blocks.
    fn flat(extra: &[(i32, i32, i32, Block)]) -> World {
        let mut w = World::new(0);
        for cz in -2..=2 {
            for cx in -2..=2 {
                let mut c = ChunkData::default();
                for y in 0..10 {
                    for z in 0..16 {
                        for x in 0..16 {
                            c.set(x, y, z, Block::STONE);
                        }
                    }
                }
                w.chunks.insert(ChunkPos::new(cx, cz), Slot { data: Arc::new(c), lit: true, dirty: 0, meshed: true, version: 0 });
            }
        }
        for &(x, y, z, b) in extra {
            let s = w.chunks.get_mut(&ChunkPos::of(x, z)).unwrap();
            Arc::make_mut(&mut s.data).set(x.rem_euclid(16), y, z.rem_euclid(16), b);
        }
        w
    }

    fn run(w: &World, b: &mut Body, c: Controls, secs: f32) -> Landing {
        let mut worst = Landing::default();
        for _ in 0..(secs * 60.0) as i32 {
            let l = b.step(w, &c, 1.0 / 60.0);
            if l.fell > worst.fell {
                worst = l;
            }
        }
        worst
    }

    #[test]
    fn it_falls_onto_the_ground_and_walks_at_walking_speed() {
        let w = flat(&[]);
        let mut b = Body::new(Vec3::new(0.5, 14.0, 0.5));
        let l = run(&w, &mut b, Controls::default(), 1.5);
        assert!((b.pos.y - 10.0).abs() < 1e-4 && b.on_ground, "{:?}", b.pos);
        assert!((l.fell - 4.0).abs() < 0.05, "{l:?}");
        let start = b.pos;
        run(&w, &mut b, Controls { forward: 1.0, ..Default::default() }, 2.0);
        let speed = (b.pos - start).length() / 2.0;
        assert!((speed - WALK).abs() < 0.2, "walked at {speed}");
        assert!(b.pos.z < start.z, "forward is north (-Z) at yaw 0");
    }

    #[test]
    fn a_jump_clears_one_block_and_not_two() {
        let mut extra: Vec<(i32, i32, i32, Block)> = (-12..=-2).flat_map(|z| (-1..2).map(move |x| (x, 10, z, Block::STONE))).collect();
        extra.extend([(5, 10, -2, Block::STONE), (5, 11, -2, Block::STONE)]);
        let w = flat(&extra);
        let mut b = Body::new(Vec3::new(0.5, 10.0, 0.5));
        b.auto_jump = false;
        b.on_ground = true;
        let mut peak: f32 = 0.0;
        b.step(&w, &Controls { jump: true, ..Default::default() }, 1.0 / 60.0);
        for _ in 0..60 {
            b.step(&w, &Controls::default(), 1.0 / 60.0);
            peak = peak.max(b.pos.y - 10.0);
        }
        assert!(peak > 1.2 && peak < 1.35, "a jump rises {peak}");
        // Into a one-block step with auto-jump: up onto it.
        let mut b = Body::new(Vec3::new(0.5, 10.0, 0.5));
        run(&w, &mut b, Controls { forward: 1.0, ..Default::default() }, 1.5);
        assert!((b.pos.y - 11.0).abs() < 1e-3, "auto-jumped onto the step: {:?}", b.pos);
        // A two-block wall stops it.
        let mut b = Body::new(Vec3::new(5.5, 10.0, 0.5));
        run(&w, &mut b, Controls { forward: 1.0, jump: true, ..Default::default() }, 2.0);
        assert!(b.pos.z > -1.0 + HALF_WIDTH - 0.01 && b.pos.y < 11.5, "{:?}", b.pos);
    }

    #[test]
    fn sneaking_never_walks_off_an_edge() {
        // A one-block-high pillar platform 3 x 3 at y 10.
        let mut extra = Vec::new();
        for x in 0..3 {
            for z in 0..3 {
                extra.push((x, 10, z, Block::STONE));
            }
        }
        let w = flat(&extra);
        let mut b = Body::new(Vec3::new(1.5, 11.0, 1.5));
        run(&w, &mut b, Controls::default(), 0.2);
        run(&w, &mut b, Controls { forward: 1.0, sneak: true, ..Default::default() }, 3.0);
        assert!((b.pos.y - 11.0).abs() < 1e-3, "still on the platform: {:?}", b.pos);
        assert!(b.pos.z < 0.0 + HALF_WIDTH && b.pos.z > -0.31, "at the very edge: {:?}", b.pos);
        run(&w, &mut b, Controls { forward: 1.0, ..Default::default() }, 1.0);
        assert!(b.pos.y < 10.5, "without sneaking it steps off");
    }

    #[test]
    fn it_slides_along_a_wall_and_never_ends_inside_one() {
        let mut extra = Vec::new();
        for x in -5..30 {
            extra.push((x, 10, -2, Block::STONE));
            extra.push((x, 11, -2, Block::STONE));
        }
        let w = flat(&extra);
        let mut b = Body::new(Vec3::new(0.5, 10.0, 0.5));
        b.auto_jump = false;
        let c = Controls { forward: 1.0, yaw: 0.6, ..Default::default() };
        run(&w, &mut b, c, 2.0);
        assert!(b.fits(&w));
        assert!((b.pos.z - (-1.0 + HALF_WIDTH)).abs() < 1e-3, "against the wall: {:?}", b.pos);
        assert!(b.pos.x > 2.0, "slid along it: {:?}", b.pos);
    }

    #[test]
    fn water_slows_and_holds_up_a_swimmer_and_creative_flight_hovers() {
        let mut extra = Vec::new();
        for y in 10..14 {
            for x in -3..3 {
                for z in -3..3 {
                    extra.push((x, y, z, Block::WATER));
                }
            }
        }
        let w = flat(&extra);
        let mut b = Body::new(Vec3::new(0.5, 13.0, 0.5));
        run(&w, &mut b, Controls::default(), 3.0);
        assert!(b.in_water && b.pos.y >= 10.0);
        run(&w, &mut b, Controls { jump: true, ..Default::default() }, 2.0);
        assert!(b.pos.y > 12.0, "swimming up: {:?}", b.pos);
        let mut f = Body::new(Vec3::new(10.5, 20.0, 10.5));
        f.flying = true;
        run(&w, &mut f, Controls::default(), 1.0);
        assert!((f.pos.y - 20.0).abs() < 0.1, "hovering: {:?}", f.pos);
        run(&w, &mut f, Controls { forward: 1.0, ..Default::default() }, 1.0);
        assert!(f.pos.z < 10.5 - 8.0, "flying fast: {:?}", f.pos);
    }
}
