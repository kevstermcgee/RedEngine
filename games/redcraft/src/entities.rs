//! What moves in the world besides the player: dropped items (they pop out of broken blocks, bob, and fly to a player who comes near), pigs
//! that wander the grass by day, zombies and creepers that come out in the dark (a zombie hits, burns in the sun; a creeper creeps close, hisses
//! and explodes) and lit TNT. Each has a body that moves through the blocks like the player's, a little mind, and a model of boxes.
//!
//! The pure parts live here (the minds, the spawning rules, the models); [`crate::play::Play`] runs them every tick against its world.

use crate::block::{tex, Block};
use crate::item::{id, Item, Stack};
use crate::physics::{Body, Controls};
use crate::world::World;
use glam::{Mat4, Vec3};

/// What an entity is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Item(Stack),
    Pig,
    Zombie,
    Creeper,
    /// Lit TNT, seconds of fuse left.
    Tnt,
}

impl Kind {
    pub fn hostile(self) -> bool {
        matches!(self, Kind::Zombie | Kind::Creeper)
    }

    pub fn mob(self) -> bool {
        matches!(self, Kind::Pig | Kind::Zombie | Kind::Creeper)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Item(_) => "item",
            Kind::Pig => "pig",
            Kind::Zombie => "zombie",
            Kind::Creeper => "creeper",
            Kind::Tnt => "tnt",
        }
    }

    fn size(self) -> (f32, f32) {
        match self {
            Kind::Item(_) => (0.125, 0.25),
            Kind::Pig => (0.45, 0.9),
            Kind::Zombie => (0.3, 1.95),
            Kind::Creeper => (0.3, 1.7),
            Kind::Tnt => (0.49, 0.98),
        }
    }

    fn max_health(self) -> f32 {
        match self {
            Kind::Pig => 10.0,
            Kind::Zombie | Kind::Creeper => 20.0,
            _ => 1.0,
        }
    }
}

/// One thing that moves.
#[derive(Clone, Debug)]
pub struct Entity {
    pub id: u32,
    pub kind: Kind,
    pub body: Body,
    /// Facing (as the player's yaw: 0 looks north, -Z).
    pub yaw: f32,
    pub health: f32,
    /// The red flash of a blow, 1 falling to 0.
    pub hurt: f32,
    pub age: f32,
    /// The legs' swing.
    pub walk: f32,
    /// Where it is wandering (a facing) and for how long more; walking or standing.
    pub wander_yaw: f32,
    pub wander_left: f32,
    pub wandering: bool,
    /// A creeper's or TNT's fuse, seconds (negative: not lit).
    pub fuse: f32,
    /// Seconds since it died (negative: alive).
    pub dying: f32,
    pub cooldown: f32,
    /// An item cannot be picked up before this age.
    pub pickup_after: f32,
    /// It is burning in the sun.
    pub burning: bool,
    pub noise_in: f32,
}

impl Entity {
    pub fn new(id: u32, kind: Kind, pos: Vec3) -> Entity {
        let (half, tall) = kind.size();
        let mut body = Body::sized(pos, half, tall);
        body.auto_jump = kind.mob();
        Entity {
            id,
            kind,
            body,
            yaw: (id as f32 * 2.399).rem_euclid(std::f32::consts::TAU),
            health: kind.max_health(),
            hurt: 0.0,
            age: 0.0,
            walk: 0.0,
            wander_yaw: 0.0,
            wander_left: 0.0,
            wandering: false,
            fuse: if kind == Kind::Tnt { 4.0 } else { -1.0 },
            dying: -1.0,
            cooldown: 0.0,
            pickup_after: 0.6,
            burning: false,
            noise_in: 3.0 + (id % 7) as f32,
        }
    }

    pub fn alive(&self) -> bool {
        self.dying < 0.0
    }

    /// Its box.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.body.bounds()
    }

    /// Where a ray from `from` along `dir` enters its box, if within `reach`.
    pub fn ray(&self, from: Vec3, dir: Vec3, reach: f32) -> Option<f32> {
        let (lo, hi) = self.bounds();
        let inv = Vec3::ONE / dir;
        let t1 = (lo - from) * inv;
        let t2 = (hi - from) * inv;
        let near = t1.min(t2).max_element();
        let far = t1.max(t2).min_element();
        (far >= near.max(0.0) && near <= reach).then_some(near.max(0.0))
    }

    /// What it drops when it dies (`n` a die 0..1).
    pub fn drops(&self, n: f32) -> Vec<Stack> {
        match self.kind {
            Kind::Pig => vec![Stack::new(Item(if self.burning { id::COOKED_PORKCHOP } else { id::PORKCHOP }), 1 + (n * 3.0) as u8)],
            Kind::Zombie if n < 0.7 => vec![Stack::new(Item(id::ROTTEN_FLESH), 1 + (n * 2.0) as u8)],
            Kind::Creeper if n < 0.25 => vec![Stack::new(Item::of(Block::TNT), 1)],
            _ => Vec::new(),
        }
    }
}

/// What a mob wants to do this tick: its controls, and whether it strikes the player.
pub struct Mind {
    pub controls: Controls,
    pub strike: bool,
    /// A creeper's fuse was just lit.
    pub hiss: bool,
}

/// A mob's mind for one tick: `player` where the player stands (`None` when it ignores them: creative, dead), `roll` a die, `dt` the tick.
pub fn think(e: &mut Entity, world: &World, player: Option<Vec3>, roll: f32, dt: f32) -> Mind {
    let mut c = Controls { yaw: e.yaw, ..Default::default() };
    let mut strike = false;
    let mut hiss = false;
    e.cooldown = (e.cooldown - dt).max(0.0);
    let chase = player.filter(|p| e.kind.hostile() && (*p - e.body.pos).length() < 18.0);
    if let Some(p) = chase {
        let to = p - e.body.pos;
        let flat = glam::Vec2::new(to.x, to.z);
        let want = flat.x.atan2(-flat.y);
        e.yaw = turn_toward(e.yaw, want, 6.0 * dt);
        c.yaw = e.yaw;
        let dist = flat.length();
        match e.kind {
            Kind::Zombie => {
                c.forward = if dist > 0.7 { 0.55 } else { 0.0 };
                if dist < 1.2 && to.y.abs() < 1.6 && e.cooldown <= 0.0 {
                    strike = true;
                    e.cooldown = 1.0;
                }
            }
            Kind::Creeper => {
                if dist < 3.0 || e.fuse >= 0.0 {
                    if e.fuse < 0.0 {
                        e.fuse = 0.0;
                        hiss = true;
                    }
                    if dist > 7.0 {
                        e.fuse = -1.0;
                    } else {
                        e.fuse += dt;
                    }
                } else {
                    c.forward = 0.5;
                }
            }
            _ => {}
        }
    } else {
        if e.kind == Kind::Creeper {
            e.fuse = -1.0;
        }
        // Wander: stand a while, walk a while, never off a drop of more than three blocks.
        e.wander_left -= dt;
        if e.wander_left <= 0.0 {
            e.wandering = roll < 0.45;
            e.wander_left = if e.wandering { 1.5 + roll * 4.0 } else { 2.0 + roll * 6.0 };
            e.wander_yaw = roll * 97.0 % std::f32::consts::TAU;
        }
        if e.wandering {
            e.yaw = turn_toward(e.yaw, e.wander_yaw, 3.0 * dt);
            c.yaw = e.yaw;
            let ahead = e.body.pos + crate::tool::look_dir(e.yaw, 0.0) * 0.9;
            let below = (0..4).any(|k| world.block(ahead.x.floor() as i32, (e.body.pos.y - 0.5).floor() as i32 - k, ahead.z.floor() as i32).solid());
            let into_water = world.block(ahead.x.floor() as i32, (e.body.pos.y - 0.5).floor() as i32, ahead.z.floor() as i32).is_liquid();
            if below && !into_water {
                c.forward = if e.kind == Kind::Pig { 0.5 } else { 0.4 };
            } else {
                e.wander_yaw += std::f32::consts::PI;
            }
        }
    }
    if e.body.in_water {
        c.jump = true;
    }
    Mind { controls: c, strike, hiss }
}

fn turn_toward(from: f32, to: f32, max: f32) -> f32 {
    let mut d = (to - from).rem_euclid(std::f32::consts::TAU);
    if d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    from + d.clamp(-max, max)
}

/// Where a mob of `kind` may appear in column (x, z) near `player`: the cell its feet go in, when the ground and the light allow it. `light`
/// is (sky, block) at that cell; `daylight` 0..1.
pub fn spawn_spot(world: &World, kind: Kind, x: i32, z: i32, y_hint: Option<i32>, daylight: f32) -> Option<Vec3> {
    let y = match y_hint {
        Some(y) => y,
        None => world.ground(x, z)? + 1,
    };
    let floor = world.block(x, y - 1, z);
    if !floor.solid() || !floor.cull() || floor == Block::BEDROCK || !world.block(x, y, z).is_air() || !world.block(x, y + 1, z).is_air() {
        return None;
    }
    let (sky, glow) = world.light(x, y, z);
    let light = (sky as f32 * daylight).max(glow as f32);
    let ok = match kind {
        Kind::Pig => matches!(floor, Block::GRASS | Block::SNOWY_GRASS) && sky >= 9,
        Kind::Zombie | Kind::Creeper => light <= 7.0 && glow <= 7,
        _ => false,
    };
    ok.then(|| Vec3::new(x as f32 + 0.5, y as f32, z as f32 + 0.5))
}

/// A box of a model: its middle and size (metres, the model's own space: feet at the origin, facing -Z), the colour it is painted with, and the
/// joint it swings on (a leg: its hip, and which way).
#[derive(Clone, Copy, Debug)]
pub struct Part {
    pub centre: Vec3,
    pub size: Vec3,
    pub tex: u8,
    /// (the joint, swing direction +1 / -1 / 0, swings about x as a leg (0) or held forward as a zombie's arm (1)).
    pub joint: Option<(Vec3, f32, u8)>,
}

const fn part(c: [f32; 3], s: [f32; 3], tex: u8) -> Part {
    Part { centre: Vec3::new(c[0], c[1], c[2]), size: Vec3::new(s[0], s[1], s[2]), tex, joint: None }
}

const fn limb(c: [f32; 3], s: [f32; 3], tex: u8, hip: [f32; 3], dir: f32, kind: u8) -> Part {
    Part { centre: Vec3::new(c[0], c[1], c[2]), size: Vec3::new(s[0], s[1], s[2]), tex, joint: Some((Vec3::new(hip[0], hip[1], hip[2]), dir, kind)) }
}

const PIG: &[Part] = &[
    part([0.0, 0.63, 0.0], [0.62, 0.5, 1.0], tex::PIG),
    part([0.0, 0.76, -0.68], [0.5, 0.5, 0.38], tex::PIG),
    part([0.0, 0.67, -0.9], [0.25, 0.19, 0.08], tex::PIG_SNOUT),
    part([-0.15, 0.84, -0.875], [0.08, 0.08, 0.02], tex::EYE),
    part([0.15, 0.84, -0.875], [0.08, 0.08, 0.02], tex::EYE),
    limb([-0.18, 0.19, -0.33], [0.22, 0.38, 0.22], tex::PIG, [-0.18, 0.38, -0.33], 1.0, 0),
    limb([0.18, 0.19, -0.33], [0.22, 0.38, 0.22], tex::PIG, [0.18, 0.38, -0.33], -1.0, 0),
    limb([-0.18, 0.19, 0.33], [0.22, 0.38, 0.22], tex::PIG, [-0.18, 0.38, 0.33], -1.0, 0),
    limb([0.18, 0.19, 0.33], [0.22, 0.38, 0.22], tex::PIG, [0.18, 0.38, 0.33], 1.0, 0),
];

const ZOMBIE: &[Part] = &[
    limb([-0.125, 0.375, 0.0], [0.25, 0.75, 0.25], tex::PANTS, [-0.125, 0.75, 0.0], 1.0, 0),
    limb([0.125, 0.375, 0.0], [0.25, 0.75, 0.25], tex::PANTS, [0.125, 0.75, 0.0], -1.0, 0),
    part([0.0, 1.125, 0.0], [0.5, 0.75, 0.25], tex::SHIRT),
    limb([-0.375, 1.125, 0.0], [0.25, 0.75, 0.25], tex::ZOMBIE, [-0.375, 1.375, 0.0], -1.0, 1),
    limb([0.375, 1.125, 0.0], [0.25, 0.75, 0.25], tex::ZOMBIE, [0.375, 1.375, 0.0], 1.0, 1),
    part([0.0, 1.75, 0.0], [0.5, 0.5, 0.5], tex::ZOMBIE),
    part([-0.125, 1.78, -0.255], [0.1, 0.06, 0.02], tex::EYE),
    part([0.125, 1.78, -0.255], [0.1, 0.06, 0.02], tex::EYE),
];

const CREEPER: &[Part] = &[
    limb([-0.125, 0.19, -0.19], [0.25, 0.375, 0.25], tex::CREEPER, [-0.125, 0.375, -0.19], 1.0, 0),
    limb([0.125, 0.19, -0.19], [0.25, 0.375, 0.25], tex::CREEPER, [0.125, 0.375, -0.19], -1.0, 0),
    limb([-0.125, 0.19, 0.19], [0.25, 0.375, 0.25], tex::CREEPER, [-0.125, 0.375, 0.19], -1.0, 0),
    limb([0.125, 0.19, 0.19], [0.25, 0.375, 0.25], tex::CREEPER, [0.125, 0.375, 0.19], 1.0, 0),
    part([0.0, 0.75, 0.0], [0.5, 0.75, 0.25], tex::CREEPER),
    part([0.0, 1.375, 0.0], [0.5, 0.5, 0.5], tex::CREEPER_LIGHT),
    part([-0.125, 1.47, -0.255], [0.125, 0.125, 0.02], tex::CREEPER_DARK),
    part([0.125, 1.47, -0.255], [0.125, 0.125, 0.02], tex::CREEPER_DARK),
    part([0.0, 1.31, -0.255], [0.125, 0.19, 0.02], tex::CREEPER_DARK),
    part([-0.09, 1.24, -0.255], [0.06, 0.125, 0.02], tex::CREEPER_DARK),
    part([0.09, 1.24, -0.255], [0.06, 0.125, 0.02], tex::CREEPER_DARK),
];

/// A mob's boxes.
pub fn parts(kind: Kind) -> &'static [Part] {
    match kind {
        Kind::Pig => PIG,
        Kind::Zombie => ZOMBIE,
        Kind::Creeper => CREEPER,
        _ => &[],
    }
}

/// Each box's world transform (of a unit cube 0..1) for a mob as it stands now.
pub fn pose(e: &Entity) -> Vec<(u8, Mat4)> {
    let swing = (e.walk * 1.6).sin() * 0.7 * (glam::Vec2::new(e.body.vel.x, e.body.vel.z).length() / 2.5).min(1.0);
    let mut place = Mat4::from_translation(e.body.pos) * Mat4::from_rotation_y(-e.yaw);
    if e.dying >= 0.0 {
        // Falls over onto its side.
        let t = (e.dying / 0.5).min(1.0);
        place *= Mat4::from_rotation_z(t * std::f32::consts::FRAC_PI_2);
    }
    // A lit creeper swells.
    if e.kind == Kind::Creeper && e.fuse > 0.0 {
        let s = 1.0 + (e.fuse / 1.5).min(1.0) * 0.12 * (1.0 + (e.fuse * 20.0).sin() * 0.3);
        place *= Mat4::from_scale(Vec3::new(s, 1.0 + (s - 1.0) * 0.5, s));
    }
    parts(e.kind)
        .iter()
        .map(|p| {
            let local = Mat4::from_translation(p.centre - p.size * 0.5) * Mat4::from_scale(p.size);
            let m = match p.joint {
                Some((hip, dir, 0)) => Mat4::from_translation(hip) * Mat4::from_rotation_x(swing * dir) * Mat4::from_translation(-hip) * local,
                Some((hip, dir, _)) => {
                    // A zombie's arms reach forward, and sway a little.
                    let a = -std::f32::consts::FRAC_PI_2 + (e.age * 2.0).sin() * 0.06 * dir;
                    Mat4::from_translation(hip) * Mat4::from_rotation_x(a) * Mat4::from_translation(-hip) * local
                }
                None => local,
            };
            (p.tex, place * m)
        })
        .collect()
}

/// A unit cube (0..1) painted in one of the flat colours, lit with (sky, glow): every face, so a mob is whole from any side.
pub fn cube_mesh(tex: u8, sky: f32, glow: f32) -> red_engine2::mesh::Mesh {
    let mut m = red_engine2::mesh::Mesh::default();
    for (d, f) in crate::mesh::FACES.iter().enumerate() {
        let n = crate::block::DIRS[d];
        let base = m.vertices.len() as u32;
        for c in f {
            m.vertices.push(red_engine2::mesh::Vertex {
                pos: *c,
                normal: [n[0] as f32, n[1] as f32, n[2] as f32],
                color: [tex as f32 / 255.0, sky, glow],
                sway: 0.0,
            });
        }
        m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobs_stand_on_their_feet_and_face_their_way() {
        for k in [Kind::Pig, Kind::Zombie, Kind::Creeper] {
            let e = Entity::new(1, k, Vec3::new(5.0, 70.0, 5.0));
            let boxes = pose(&e);
            assert!(!boxes.is_empty());
            let lowest = boxes.iter().map(|(_, m)| m.transform_point3(Vec3::ZERO).y.min(m.transform_point3(Vec3::ONE).y)).fold(f32::MAX, f32::min);
            assert!((lowest - 70.0).abs() < 0.05, "{k:?} feet at {lowest}");
        }
        // Facing north, a zombie's eyes are on its -Z side.
        let mut z = Entity::new(2, Kind::Zombie, Vec3::ZERO);
        z.yaw = 0.0;
        let eye = pose(&z).iter().find(|(t, _)| *t == tex::EYE).map(|(_, m)| m.transform_point3(Vec3::splat(0.5))).unwrap();
        assert!(eye.z < -0.2, "{eye}");
    }

    #[test]
    fn a_ray_hits_a_mob_box() {
        let e = Entity::new(3, Kind::Pig, Vec3::new(0.0, 0.0, -3.0));
        let d = e.ray(Vec3::new(0.0, 0.5, 0.0), Vec3::NEG_Z, 5.0).unwrap();
        assert!((d - 2.55).abs() < 1e-3, "{d}");
        assert!(e.ray(Vec3::new(3.0, 0.5, 0.0), Vec3::NEG_Z, 5.0).is_none());
        assert!(e.drops(0.5).iter().all(|s| s.item == Item(id::PORKCHOP)));
    }

    #[test]
    fn turning_takes_the_short_way() {
        let y = turn_toward(0.1, std::f32::consts::TAU - 0.1, 1.0);
        assert!((y - (-0.1)).abs() < 1e-5, "{y}");
    }
}
