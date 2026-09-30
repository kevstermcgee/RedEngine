//! Bullet tracers and impact sparks as a pool of glowing boxes in the scene (ADR 0055): a shot is invisible on its own (hitscan), so the client draws where
//! it went. [`add_pool`] adds hidden boxes to the scene before the renderer is built (it takes its meshes from the scene at creation, like the avatar
//! pool); [`Streaks`] then lays one along each shot for a moment and shrinks it away, and puts a spark where it landed. Pure: no window, no GPU.

use crate::schema::{Material, Object, ObjectKind, PrimKind, Scene};
use crate::track::Track;
use glam::{EulerRot, Quat, Vec3};

/// Boxes in the pool: about a second of a firefight (eight players, a burst each).
pub const POOL: usize = 64;
/// How long a tracer lasts, seconds.
pub const TRACER_SECS: f32 = 0.12;
/// How long a spark lasts, seconds.
pub const SPARK_SECS: f32 = 0.18;
/// Thickness of a tracer at its brightest, metres.
pub const TRACER_WIDTH: f32 = 0.08;
/// Edge of a spark at its biggest, metres.
pub const SPARK_SIZE: f32 = 0.16;
/// Scale that hides a box (the renderer has no per-object visibility flag).
const HIDDEN: f32 = 0.0005;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Tracer { from: Vec3, to: Vec3 },
    Spark { at: Vec3 },
}

#[derive(Debug, Clone, Copy)]
struct Streak {
    kind: Kind,
    color: Vec3,
    age: f32,
    life: f32,
}

/// Adds [`POOL`] hidden glowing unit boxes to `scene` and returns the index of the first. Call it before the renderer is built.
pub fn add_pool(scene: &mut Scene) -> usize {
    let first = scene.objects.len();
    for i in 0..POOL {
        scene.objects.push(Object {
            id: format!("fx_streak_{i}"),
            position: Track::constant(Vec3::ZERO),
            rotation: Track::constant(Vec3::ZERO),
            scale: Track::constant(Vec3::splat(HIDDEN)),
            material: Some(Material { color: Track::constant(Vec3::ZERO), metallic: 0.0, roughness: 1.0, emissive: Vec3::ZERO, opacity: 1.0 }),
            collide: false,
            prefab: None,
            movable: Some(false),
            kind: ObjectKind::Prim(PrimKind::Box { size: Vec3::ONE }),
        });
    }
    first
}

/// The live tracers and sparks, and the pool of boxes that draws them.
#[derive(Debug, Clone)]
pub struct Streaks {
    first: usize,
    live: Vec<Streak>,
    /// How many boxes the last [`update`](Streaks::update) placed (the rest are hidden).
    shown: usize,
}

impl Streaks {
    /// Drives the pool that starts at scene object `first` (the value [`add_pool`] returned).
    pub fn new(first: usize) -> Streaks {
        Streaks { first, live: Vec::with_capacity(POOL), shown: 0 }
    }

    /// The ids of the pool's boxes that show nothing as of the last [`update`](Streaks::update): the renderer must not draw them (a box scaled to a
    /// speck is still a draw call, and there are [`POOL`] of them).
    pub fn hidden_ids<'a>(&self, scene: &'a Scene) -> impl Iterator<Item = &'a str> + 'a {
        let first = self.first;
        (self.shown..POOL).filter_map(move |i| scene.objects.get(first + i).map(|o| o.id.as_str()))
    }

    fn add(&mut self, kind: Kind, color: Vec3, life: f32) {
        if self.live.len() >= POOL {
            self.live.remove(0); // the oldest gives way
        }
        self.live.push(Streak { kind, color, age: 0.0, life });
    }

    /// A tracer along a shot from `from` to `to`, glowing `color`.
    pub fn tracer(&mut self, from: Vec3, to: Vec3, color: Vec3) {
        if (to - from).length() > 0.05 {
            self.add(Kind::Tracer { from, to }, color, TRACER_SECS);
        }
    }

    /// A spark where a shot landed.
    pub fn spark(&mut self, at: Vec3, color: Vec3) {
        self.add(Kind::Spark { at }, color, SPARK_SECS);
    }

    /// How many are showing.
    pub fn len(&self) -> usize {
        self.live.len()
    }

    /// Whether nothing is showing.
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// Ages every streak by `dt` and writes the pool's boxes into `scene`: live ones placed, sized and lit by how much life they have left, the rest hidden.
    pub fn update(&mut self, scene: &mut Scene, dt: f32) {
        for s in &mut self.live {
            s.age += dt;
        }
        self.live.retain(|s| s.age < s.life);
        self.shown = self.live.len().min(POOL);
        for i in 0..POOL {
            let Some(o) = scene.objects.get_mut(self.first + i) else { return };
            let Some(s) = self.live.get(i) else {
                o.scale = Track::constant(Vec3::splat(HIDDEN));
                continue;
            };
            let left = (1.0 - s.age / s.life).clamp(0.0, 1.0);
            if let Some(m) = &mut o.material {
                m.emissive = s.color * (1.0 + 3.0 * left);
            }
            match s.kind {
                Kind::Tracer { from, to } => {
                    let d = to - from;
                    let len = d.length();
                    let q = Quat::from_rotation_arc(Vec3::Z, d / len.max(1e-6));
                    let (x, y, z) = q.to_euler(EulerRot::XYZ);
                    o.position = Track::constant((from + to) * 0.5);
                    o.rotation = Track::constant(Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees()));
                    let w = TRACER_WIDTH * left.max(0.05);
                    o.scale = Track::constant(Vec3::new(w, w, len));
                }
                Kind::Spark { at } => {
                    o.position = Track::constant(at);
                    o.rotation = Track::constant(Vec3::new(45.0, 45.0 + 900.0 * s.age, 0.0));
                    o.scale = Track::constant(Vec3::splat(SPARK_SIZE * (0.3 + 0.7 * left)));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Scene {
        crate::schema::parse_scene(
            r##"{"camera":{"position":[0,1.7,0],"target":[0,1.7,-10]},"objects":[{"id":"floor","type":"box","size":[10,0.2,10],"position":[0,-0.1,0]}]}"##,
        )
        .unwrap_or_else(|e| panic!("{e:?}"))
    }

    fn emissive(o: &Object) -> f32 {
        o.material.as_ref().map_or(0.0, |m| m.emissive.length())
    }

    #[test]
    fn the_pool_is_hidden_glowing_boxes_after_the_map() {
        let mut s = scene();
        let first = add_pool(&mut s);
        assert_eq!((first, s.objects.len()), (1, 1 + POOL));
        assert!(s.objects[first..].iter().all(|o| o.scale.sample(0.0).x < 0.01 && !o.collide && o.id.starts_with("fx_streak_")));
    }

    #[test]
    fn the_boxes_that_show_nothing_are_listed_for_the_renderer_to_skip() {
        let mut s = scene();
        let first = add_pool(&mut s);
        let mut fx = Streaks::new(first);
        fx.update(&mut s, 0.0);
        assert_eq!(fx.hidden_ids(&s).count(), POOL, "nothing is showing");
        fx.tracer(Vec3::ZERO, Vec3::new(0.0, 0.0, -5.0), Vec3::ONE);
        fx.spark(Vec3::ONE, Vec3::ONE);
        fx.update(&mut s, 0.0);
        let hidden: Vec<&str> = fx.hidden_ids(&s).collect();
        assert_eq!(hidden.len(), POOL - 2);
        assert!(!hidden.contains(&"fx_streak_0") && !hidden.contains(&"fx_streak_1") && hidden.contains(&"fx_streak_2"), "{:?}", &hidden[..3]);
        fx.update(&mut s, TRACER_SECS + SPARK_SECS);
        assert_eq!(fx.hidden_ids(&s).count(), POOL, "and they are all back when the streaks are over");
    }

    #[test]
    fn a_tracer_lies_along_its_shot_and_shrinks_away() {
        let mut s = scene();
        let first = add_pool(&mut s);
        let mut fx = Streaks::new(first);
        fx.tracer(Vec3::new(0.0, 1.5, 0.0), Vec3::new(0.0, 1.5, -10.0), Vec3::new(1.0, 0.9, 0.5));
        fx.update(&mut s, 0.0);
        let o = &s.objects[first];
        let (pos, scale, rot) = (o.position.sample(0.0), o.scale.sample(0.0), o.rotation.sample(0.0));
        assert!((pos - Vec3::new(0.0, 1.5, -5.0)).length() < 1e-4, "centred on the shot: {pos:?}");
        assert!((scale.z - 10.0).abs() < 1e-4 && (scale.x - TRACER_WIDTH).abs() < 1e-4, "as long as the shot and thin: {scale:?}");
        let q = Quat::from_euler(EulerRot::XYZ, rot.x.to_radians(), rot.y.to_radians(), rot.z.to_radians());
        assert!((q * Vec3::Z - Vec3::NEG_Z).length() < 1e-4, "its long axis points down the shot");
        let bright = emissive(o);
        assert!(bright > 1.0, "it glows");
        // Half way through its life it is thinner and dimmer; at the end it is gone and its box is hidden.
        fx.update(&mut s, TRACER_SECS * 0.5);
        let mid = &s.objects[first];
        assert!(mid.scale.sample(0.0).x < TRACER_WIDTH * 0.6 && emissive(mid) < bright);
        fx.update(&mut s, TRACER_SECS);
        assert!(fx.is_empty() && s.objects[first].scale.sample(0.0).x < 0.01);
    }

    #[test]
    fn a_tracer_in_any_direction_points_that_way_and_a_zero_length_one_is_ignored() {
        let mut s = scene();
        let first = add_pool(&mut s);
        let mut fx = Streaks::new(first);
        for dir in [Vec3::X, Vec3::Y, Vec3::new(1.0, 0.5, -2.0).normalize(), Vec3::new(-0.3, -0.9, 0.1).normalize()] {
            fx.tracer(Vec3::ONE, Vec3::ONE + dir * 7.0, Vec3::ONE);
            fx.update(&mut s, 0.0);
            let o = &s.objects[first + fx.len() - 1];
            let r = o.rotation.sample(0.0);
            let q = Quat::from_euler(EulerRot::XYZ, r.x.to_radians(), r.y.to_radians(), r.z.to_radians());
            assert!((q * Vec3::Z - dir).length() < 1e-3, "{dir:?}");
        }
        fx.tracer(Vec3::ONE, Vec3::ONE, Vec3::ONE);
        assert_eq!(fx.len(), 4, "no streak for a shot that went nowhere");
    }

    #[test]
    fn sparks_pop_and_fade_and_the_pool_recycles_its_oldest() {
        let mut s = scene();
        let first = add_pool(&mut s);
        let mut fx = Streaks::new(first);
        fx.spark(Vec3::new(2.0, 1.0, 3.0), Vec3::new(1.0, 0.2, 0.2));
        fx.update(&mut s, 0.0);
        let big = s.objects[first].scale.sample(0.0).x;
        assert!((big - SPARK_SIZE).abs() < 1e-4 && (s.objects[first].position.sample(0.0) - Vec3::new(2.0, 1.0, 3.0)).length() < 1e-5);
        fx.update(&mut s, SPARK_SECS * 0.6);
        assert!(s.objects[first].scale.sample(0.0).x < big * 0.6);
        for i in 0..POOL + 10 {
            fx.spark(Vec3::new(i as f32, 0.0, 0.0), Vec3::ONE);
        }
        assert_eq!(fx.len(), POOL, "never more than the pool holds");
        fx.update(&mut s, 0.0);
        let newest = s.objects[first + POOL - 1].position.sample(0.0);
        assert_eq!(newest.x, (POOL + 9) as f32, "the newest is kept");
    }
}
