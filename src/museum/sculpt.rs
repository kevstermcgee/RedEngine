//! Sculptures and installations: eighteen families of procedural forms, each made from a seed, and each alive in its own way.
//!
//! A piece is a few [`Part`]s: a mesh in the piece's own space (base at the origin, up +Y, facing +Z), a pivot, and the [`Motion`]s that move it every
//! frame. Motions answer the room as well as the clock: [`Stir`] says how near the player is, where they stand (a figure's head turns to watch them, a
//! flower opens as they come close, a heart beats faster), how long ago the piece was touched, and whether it is giving up a key. [`part_transform`] is
//! a pure function, so the headless tests and the renderer agree on where everything is.

use super::shape::{ball, lathe, obox, tube_path};
use super::text::Draw;
use crate::procgen::geo::{Geo, Rgb};
use glam::{Mat3, Mat4, Quat, Vec3};
use std::f32::consts::{PI, TAU};

/// A family of forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// A turned urn or bottle.
    Vessel,
    /// A star-shaped column twisting as it rises.
    Spiral,
    /// Stones balanced on one another, never quite still.
    Cairn,
    /// A tube tied in a torus knot.
    Knot,
    /// Crystal spikes bursting from the floor, trembling when approached.
    Shards,
    /// A standing figure whose head turns to watch the viewer.
    Figure,
    /// Two figures leaning together, almost touching.
    Pair,
    /// Moons circling a sphere on a stem.
    Orbit,
    /// A tall slab pierced by a hole with a light floating in it.
    Monolith,
    /// A flower that opens as one comes near.
    Bloom,
    /// A heart that beats, faster when one is close.
    Heart,
    /// A wave caught as it breaks.
    Wave,
    /// A cage with a small light fluttering inside.
    Cage,
    /// A great hand, palm up, holding a light.
    Hand,
    /// (hangs) Arms and discs turning in the air.
    Mobile,
    /// (hangs) A cloud of small lights drifting.
    Constellation,
    /// (hangs) A ring of tubes on threads that ring when approached.
    Chimes,
    /// (hangs) Rain stopped in mid-fall: hundreds of glass drops on invisible threads.
    Rain,
}

impl Family {
    pub const ALL: [Family; 18] = [
        Family::Vessel,
        Family::Spiral,
        Family::Cairn,
        Family::Knot,
        Family::Shards,
        Family::Figure,
        Family::Pair,
        Family::Orbit,
        Family::Monolith,
        Family::Bloom,
        Family::Heart,
        Family::Wave,
        Family::Cage,
        Family::Hand,
        Family::Mobile,
        Family::Constellation,
        Family::Chimes,
        Family::Rain,
    ];

    /// The name a collection uses.
    pub fn name(self) -> &'static str {
        match self {
            Family::Vessel => "vessel",
            Family::Spiral => "spiral",
            Family::Cairn => "cairn",
            Family::Knot => "knot",
            Family::Shards => "shards",
            Family::Figure => "figure",
            Family::Pair => "pair",
            Family::Orbit => "orbit",
            Family::Monolith => "monolith",
            Family::Bloom => "bloom",
            Family::Heart => "heart",
            Family::Wave => "wave",
            Family::Cage => "cage",
            Family::Hand => "hand",
            Family::Mobile => "mobile",
            Family::Constellation => "constellation",
            Family::Chimes => "chimes",
            Family::Rain => "rain",
        }
    }

    pub fn from_name(s: &str) -> Option<Family> {
        Family::ALL.into_iter().find(|f| f.name() == s)
    }

    /// Hung from the ceiling (an installation) rather than standing on the floor.
    pub fn hangs(self) -> bool {
        matches!(self, Family::Mobile | Family::Constellation | Family::Chimes | Family::Rain)
    }

    /// Footprint radius, height and plinth height for a new piece.
    pub fn proportions(self, d: &mut Draw) -> (f32, f32, f32) {
        match self {
            Family::Vessel => (d.range(0.3, 0.5), d.range(0.8, 1.5), d.range(0.7, 1.0)),
            Family::Spiral => (d.range(0.35, 0.55), d.range(2.0, 3.2), 0.25),
            Family::Cairn => (d.range(0.45, 0.65), d.range(1.2, 1.9), 0.18),
            Family::Knot => (d.range(0.5, 0.75), d.range(1.0, 1.5), d.range(0.8, 1.0)),
            Family::Shards => (d.range(0.7, 1.0), d.range(1.4, 2.4), 0.12),
            Family::Figure => (d.range(0.35, 0.45), d.range(1.7, 2.3), d.range(0.3, 0.6)),
            Family::Pair => (d.range(0.6, 0.8), d.range(1.6, 2.1), 0.35),
            Family::Orbit => (d.range(0.6, 0.85), d.range(1.4, 2.0), 0.5),
            Family::Monolith => (d.range(0.6, 0.9), d.range(2.6, 3.6), 0.1),
            Family::Bloom => (d.range(0.6, 0.9), d.range(1.3, 2.0), 0.45),
            Family::Heart => (d.range(0.4, 0.55), d.range(0.8, 1.1), d.range(0.9, 1.1)),
            Family::Wave => (d.range(0.8, 1.1), d.range(1.2, 1.8), 0.4),
            Family::Cage => (d.range(0.4, 0.55), d.range(1.2, 1.7), d.range(0.6, 0.9)),
            Family::Hand => (d.range(0.5, 0.7), d.range(1.1, 1.5), d.range(0.5, 0.8)),
            Family::Mobile => (d.range(1.6, 2.4), d.range(2.0, 3.0), 0.0),
            Family::Constellation => (d.range(1.4, 2.0), d.range(1.6, 2.6), 0.0),
            Family::Chimes => (d.range(0.8, 1.2), d.range(2.2, 3.0), 0.0),
            Family::Rain => (d.range(1.4, 2.0), d.range(2.0, 3.0), 0.0),
        }
    }
}

/// How a part moves. Angles in radians, rates in radians (or cycles) per second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    /// Turns about its pivot's vertical at `rate`, faster for a while after a touch.
    Spin { rate: f32 },
    /// Rises and falls.
    Bob { amp: f32, rate: f32, phase: f32 },
    /// Swells and shrinks about its pivot.
    Breathe { amp: f32, rate: f32, phase: f32 },
    /// Leans to and fro about the pivot's horizontal `axis` (in the piece's space).
    Sway { axis: Vec3, amp: f32, rate: f32, phase: f32 },
    /// Turns its face (+Z) toward the viewer, slowly, as far as `limit` either way.
    Watch { limit: f32 },
    /// Goes round the piece's vertical axis at `radius` (the pivot is the centre), tilted by `tilt`.
    Orbit { radius: f32, rate: f32, phase: f32, tilt: f32 },
    /// Opens by `angle` about `axis` (piece space) as the viewer comes near, and all the way while giving up a key.
    Open { axis: Vec3, angle: f32 },
    /// A heartbeat: two quick swells a beat, the beat quicker when the viewer is near.
    Beat { amp: f32, rate: f32 },
    /// Shivers when the viewer is near.
    Tremble { amp: f32 },
}

/// One moving part of a piece.
#[derive(Debug, Clone)]
pub struct Part {
    pub geo: Geo,
    /// The point it moves about (piece space).
    pub pivot: Vec3,
    pub motions: Vec<Motion>,
    /// A part it rides on (a mobile's arms).
    pub parent: Option<usize>,
    /// Painted with the piece's material (the shader's `surface`); otherwise its vertex colours.
    pub painted: bool,
    /// How much it shines by itself (0 none), in the piece's light colour.
    pub glow: f32,
}

impl Part {
    fn new(geo: Geo, pivot: Vec3, painted: bool) -> Part {
        Part { geo, pivot, motions: Vec::new(), parent: None, painted, glow: 0.0 }
    }

    fn moving(mut self, m: Motion) -> Part {
        self.motions.push(m);
        self
    }

    fn glowing(mut self, g: f32) -> Part {
        self.glow = g;
        self
    }
}

/// What the room is doing to a piece this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stir {
    /// 0 far away .. 1 standing at it.
    pub near: f32,
    /// The viewer's eye in the piece's space.
    pub eye: Vec3,
    /// Seconds since it was last touched (large: never).
    pub since_touch: f32,
    /// 0..1: how far it has opened to give up its key.
    pub reveal: f32,
}

/// The parts of a piece of `family`, `radius` and `height`, from `seed`. `light` is the colour its glowing parts shine with.
pub fn build(family: Family, radius: f32, height: f32, seed: f32, light: Rgb) -> Vec<Part> {
    let mut d = Draw::new(&[(seed * 1000.0) as i64, family as i64]);
    let white = [1.0, 1.0, 1.0];
    let r = radius;
    let h = height;
    match family {
        Family::Vessel => {
            // An urn: foot, belly, shoulder, neck and lip, each from the seed.
            let belly = d.range(0.75, 1.0) * r;
            let neck = d.range(0.2, 0.45) * r;
            let belly_at = d.range(0.3, 0.55);
            let profile = [
                (r * 0.45, 0.0),
                (r * 0.5, h * 0.04),
                (r * 0.35, h * 0.08),
                (belly * 0.8, h * (belly_at - 0.15)),
                (belly, h * belly_at),
                (belly * 0.85, h * (belly_at + 0.14)),
                (neck * 1.3, h * 0.78),
                (neck, h * 0.86),
                (neck * 1.1, h * 0.95),
                (neck * 1.45, h),
            ];
            let mut g = Geo::default();
            lathe(&mut g, Vec3::ZERO, &profile, 28, |_| white);
            vec![Part::new(g, Vec3::ZERO, true).moving(Motion::Spin { rate: d.range(0.04, 0.1) }).moving(Motion::Breathe {
                amp: 0.012,
                rate: 0.25,
                phase: d.range(0.0, TAU),
            })]
        }
        Family::Spiral => {
            let points = 3 + d.below(4);
            let twist = d.range(1.0, 2.6) * PI * if d.chance(0.5) { 1.0 } else { -1.0 };
            let mut g = Geo::default();
            let rings = 40;
            let inner = d.range(0.35, 0.6);
            let first = g.pos.len() as u32;
            let star = (points * 2) as u32;
            for i in 0..=rings {
                let t = i as f32 / rings as f32;
                let y = t * h;
                let taper = 1.0 - 0.55 * t + 0.15 * (t * PI * 2.0).sin();
                for k in 0..=star {
                    let a = k as f32 / star as f32 * TAU + twist * t;
                    let rr = if k % 2 == 0 { r } else { r * inner } * taper;
                    let p = Vec3::new(a.cos() * rr, y, a.sin() * rr);
                    g.vert(p, Vec3::new(a.cos(), 0.15, a.sin()), white);
                }
            }
            for i in 0..rings {
                for k in 0..star {
                    let a = first + i * (star + 1) + k;
                    let b = a + star + 1;
                    g.idx.extend_from_slice(&[a, b + 1, a + 1, a, b, b + 1]);
                }
            }
            vec![Part::new(g, Vec3::ZERO, true).moving(Motion::Spin { rate: d.range(0.08, 0.2) * twist.signum() })]
        }
        Family::Cairn => {
            let n = 4 + d.below(4);
            let mut parts = Vec::new();
            let mut y = 0.0;
            let mut off = Vec3::ZERO;
            for i in 0..n {
                let k = 1.0 - i as f32 / (n as f32 + 1.5);
                let sz = Vec3::new(r * k * d.range(0.8, 1.1), h / n as f32 * 0.5 * d.range(0.8, 1.2), r * k * d.range(0.7, 1.0));
                let mut g = Geo::default();
                g.blob(Vec3::ZERO, sz, 2, d.below(1000) as u32, 0.12, white, white);
                let at = off + Vec3::Y * (y + sz.y);
                // Each stone rocks a little on the one below.
                parts.push(Part::new(shifted(g, at), at - Vec3::Y * sz.y, true).moving(Motion::Sway {
                    axis: Vec3::new(d.range(-1.0, 1.0), 0.0, d.range(-1.0, 1.0)).normalize_or(Vec3::X),
                    amp: 0.01 + 0.012 * i as f32,
                    rate: d.range(0.3, 0.6),
                    phase: d.range(0.0, TAU),
                }));
                y += sz.y * 1.8;
                off += Vec3::new(d.range(-0.06, 0.06), 0.0, d.range(-0.06, 0.06));
            }
            parts
        }
        Family::Knot => {
            let (p, q) = *d.pick(&[(2.0, 3.0), (3.0, 4.0), (2.0, 5.0), (3.0, 5.0), (3.0, 2.0)]);
            let n = 220;
            let rr = r * 0.62;
            let centre = Vec3::Y * (h * 0.55);
            let pts: Vec<Vec3> = (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32 * TAU;
                    let k = rr * (0.62 + 0.38 * (q * t).cos());
                    centre + Vec3::new(k * (p * t).cos(), rr * 0.6 * (q * t).sin(), k * (p * t).sin())
                })
                .collect();
            let mut g = Geo::default();
            tube_path(&mut g, &pts, &vec![r * 0.11; pts.len()], 10, |_| white);
            vec![Part::new(g, centre, true).moving(Motion::Spin { rate: d.range(0.12, 0.25) }).moving(Motion::Bob { amp: 0.04, rate: 0.5, phase: 0.0 })]
        }
        Family::Shards => {
            let n = 7 + d.below(7);
            let mut g = Geo::default();
            let mut core = Geo::default();
            for i in 0..n {
                let a = i as f32 / n as f32 * TAU + d.range(-0.3, 0.3);
                let lean = d.range(0.05, 0.6);
                let dir = Vec3::new(a.cos() * lean, 1.0, a.sin() * lean).normalize();
                let len = h * d.range(0.45, 1.0);
                let w = r * d.range(0.12, 0.26);
                let base = Vec3::new(a.cos(), 0.0, a.sin()) * r * d.range(0.0, 0.35);
                crystal(&mut g, base, dir, len, w, white);
            }
            ball(&mut core, Vec3::Y * h * 0.25, Vec3::splat(r * 0.3), 1, light);
            vec![
                Part::new(g, Vec3::ZERO, true).moving(Motion::Tremble { amp: 0.02 }),
                Part::new(core, Vec3::Y * h * 0.25, false).glowing(1.4).moving(Motion::Breathe { amp: 0.15, rate: 1.1, phase: 0.0 }),
            ]
        }
        Family::Figure => {
            let mut parts = figure(&mut d, Vec3::ZERO, r, h, 0.0, white);
            if let Some(head) = parts.get_mut(1) {
                head.motions.push(Motion::Watch { limit: d.range(0.8, 1.4) });
            }
            parts[0].motions.push(Motion::Breathe { amp: 0.008, rate: 0.22, phase: 0.0 });
            parts
        }
        Family::Pair => {
            let lean = d.range(0.12, 0.22);
            let mut a = figure(&mut d, Vec3::new(-r * 0.45, 0.0, 0.0), r * 0.55, h, lean, white);
            let hb = h * d.range(0.88, 1.0);
            let mut b = figure(&mut d, Vec3::new(r * 0.45, 0.0, 0.0), r * 0.55, hb, -lean, white);
            // They lean toward each other and back, never quite meeting.
            for (parts, sign) in [(&mut a, 1.0), (&mut b, -1.0)] {
                for p in parts.iter_mut() {
                    p.pivot = Vec3::new(p.pivot.x, 0.0, 0.0);
                    p.motions.push(Motion::Sway { axis: Vec3::Z, amp: 0.025 * sign, rate: 0.18, phase: 0.0 });
                }
            }
            a.extend(b);
            a
        }
        Family::Orbit => {
            let mut stem = Geo::default();
            tube_path(&mut stem, &[Vec3::ZERO, Vec3::Y * h * 0.62], &[r * 0.05, r * 0.03], 8, |_| white);
            let centre = Vec3::Y * h * 0.72;
            let mut sun = Geo::default();
            ball(&mut sun, centre, Vec3::splat(r * 0.28), 2, light);
            let mut parts = vec![
                Part::new(stem, Vec3::ZERO, true),
                Part::new(sun, centre, false).glowing(1.2).moving(Motion::Breathe { amp: 0.06, rate: 0.6, phase: 0.0 }),
            ];
            let n = 3 + d.below(4);
            for i in 0..n {
                let mut m = Geo::default();
                let size = r * d.range(0.06, 0.14);
                ball(&mut m, centre, Vec3::splat(size), 1, white);
                let orbit = r * (0.45 + 0.5 * (i as f32 + 1.0) / n as f32);
                parts.push(Part::new(m, centre, true).moving(Motion::Orbit {
                    radius: orbit,
                    rate: d.range(0.15, 0.6) / (1.0 + i as f32 * 0.4) * if d.chance(0.2) { -1.0 } else { 1.0 },
                    phase: d.range(0.0, TAU),
                    tilt: d.range(-0.5, 0.5),
                }));
            }
            parts
        }
        Family::Monolith => {
            // A slab with a square opening, made of four blocks, and a light floating in the hole.
            let (w, t) = (r * 1.4, r * 0.35);
            let hole = (w * 0.45, h * 0.62, w * 0.22);
            let mut g = Geo::default();
            let ax = [Vec3::X, Vec3::Y, Vec3::Z];
            let below = hole.1 - hole.2;
            obox(&mut g, Vec3::Y * below * 0.5, ax, Vec3::new(w * 0.5, below * 0.5, t * 0.5), white);
            let above = h - (hole.1 + hole.2);
            obox(&mut g, Vec3::Y * (h - above * 0.5), ax, Vec3::new(w * 0.5, above * 0.5, t * 0.5), white);
            let side = (w * 0.5 - hole.2) * 0.5;
            for s in [-1.0, 1.0] {
                obox(&mut g, Vec3::new(s * (hole.2 + side), hole.1, 0.0), ax, Vec3::new(side, hole.2, t * 0.5), white);
            }
            let mut orb = Geo::default();
            ball(&mut orb, Vec3::Y * hole.1, Vec3::splat(hole.2 * 0.55), 2, light);
            vec![
                Part::new(g, Vec3::ZERO, true),
                Part::new(orb, Vec3::Y * hole.1, false).glowing(1.6).moving(Motion::Bob { amp: hole.2 * 0.25, rate: 0.4, phase: 0.0 }),
            ]
        }
        Family::Bloom => {
            let top = Vec3::Y * h * 0.72;
            let mut stem = Geo::default();
            let sway = d.range(-0.15, 0.15);
            let pts: Vec<Vec3> = (0..=10).map(|i| Vec3::new(sway * (i as f32 / 10.0 * PI).sin(), h * 0.72 * i as f32 / 10.0, 0.0)).collect();
            tube_path(&mut stem, &pts, &(0..=10).map(|i| r * (0.08 - 0.03 * i as f32 / 10.0)).collect::<Vec<_>>(), 8, |_| white);
            let top = top + Vec3::X * 0.0;
            let mut core = Geo::default();
            ball(&mut core, top, Vec3::splat(r * 0.16), 2, light);
            let mut parts =
                vec![Part::new(stem, Vec3::ZERO, true), Part::new(core, top, false).glowing(1.5).moving(Motion::Breathe { amp: 0.1, rate: 0.5, phase: 0.0 })];
            let n = 5 + d.below(5);
            let len = r * d.range(0.8, 1.1);
            for i in 0..n {
                let a = i as f32 / n as f32 * TAU;
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let mut g = Geo::default();
                // A cupped petal: a flattened blob leaning out from the core.
                let centre = top + out * len * 0.5 + Vec3::Y * len * 0.35;
                g.blob(Vec3::ZERO, Vec3::new(len * 0.5, len * 0.07, len * 0.22), 1, i as u32, 0.05, white, white);
                let m = Mat3::from_cols(out, Vec3::Y, out.cross(Vec3::Y)) * Mat3::from_rotation_z(0.6);
                let from = 0;
                super::shape::place(&mut g, from, m, centre);
                // Closed it stands up (folded toward the axis); near the viewer it opens out.
                let hinge = out.cross(Vec3::Y).normalize();
                parts.push(Part::new(g, top, true).moving(Motion::Open { axis: hinge, angle: 1.0 }));
            }
            parts
        }
        Family::Heart => {
            let centre = Vec3::Y * h * 0.5;
            let mut g = Geo::default();
            g.blob(centre + Vec3::new(-r * 0.25, r * 0.12, 0.0), Vec3::new(r * 0.55, r * 0.6, r * 0.5), 2, 3, 0.07, white, white);
            g.blob(centre + Vec3::new(r * 0.25, r * 0.12, 0.0), Vec3::new(r * 0.55, r * 0.6, r * 0.5), 2, 4, 0.07, white, white);
            g.blob(centre - Vec3::Y * r * 0.35, Vec3::new(r * 0.45, r * 0.62, r * 0.42), 2, 5, 0.07, white, white);
            for k in 0..3 {
                let a = (k as f32 - 1.0) * 0.4;
                let base = centre + Vec3::new(a * r * 0.6, r * 0.5, 0.0);
                tube_path(
                    &mut g,
                    &[base, base + Vec3::new(a * r * 0.4, r * 0.45, r * 0.1), base + Vec3::new(a * r * 0.9, r * 0.6, r * 0.2)],
                    &[r * 0.13, r * 0.11, r * 0.08],
                    8,
                    |_| white,
                );
            }
            let mut glow = Geo::default();
            ball(&mut glow, centre, Vec3::splat(r * 0.4), 1, light);
            vec![
                Part::new(g, centre, true).moving(Motion::Beat { amp: 0.06, rate: d.range(0.9, 1.2) }),
                Part::new(glow, centre, false).glowing(0.9).moving(Motion::Beat { amp: 0.25, rate: 1.0 }),
            ]
        }
        Family::Wave => {
            // A curling sheet: a grid bent into a breaking wave, thick enough to read from both sides.
            let mut g = Geo::default();
            let (nx, ny) = (24u32, 16u32);
            let curl = d.range(0.55, 0.95);
            let first = g.pos.len() as u32;
            for j in 0..=ny {
                for i in 0..=nx {
                    let u = i as f32 / nx as f32;
                    let v = j as f32 / ny as f32;
                    let x = (u - 0.5) * r * 2.4;
                    let ang = v * PI * curl;
                    let rad = h * 0.5 * (1.0 - 0.35 * v);
                    let p = Vec3::new(x, rad * ang.sin() * (1.0 + 0.08 * (u * 9.0).sin()), -rad * (1.0 - ang.cos()) + r * 0.6);
                    let n = Vec3::new(0.0, ang.cos(), ang.sin()).normalize_or(Vec3::Z);
                    g.vert(p, n, white);
                }
            }
            for j in 0..ny {
                for i in 0..nx {
                    let a = first + j * (nx + 1) + i;
                    let b = a + nx + 1;
                    g.idx.extend_from_slice(&[a, a + 1, b + 1, a, b + 1, b]);
                    g.idx.extend_from_slice(&[a, b + 1, a + 1, a, b, b + 1]);
                }
            }
            vec![Part::new(g, Vec3::ZERO, true).moving(Motion::Sway { axis: Vec3::X, amp: 0.015, rate: 0.3, phase: 0.0 })]
        }
        Family::Cage => {
            let mut g = Geo::default();
            let bars = 10 + d.below(6);
            let top = h * 0.82;
            for i in 0..bars {
                let a = i as f32 / bars as f32 * TAU;
                let p = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
                let bulge = Vec3::new(a.cos(), 0.0, a.sin()) * r * 0.12;
                tube_path(&mut g, &[p, p + bulge + Vec3::Y * top * 0.5, p + Vec3::Y * top, Vec3::Y * h], &[0.012, 0.012, 0.012, 0.02], 5, |_| white);
            }
            for y in [0.02, top * 0.5, top] {
                let ring: Vec<Vec3> = (0..=32)
                    .map(|k| {
                        let a = k as f32 / 32.0 * TAU;
                        let rr = if (y - top * 0.5).abs() < 0.01 { r * 1.12 } else { r };
                        Vec3::new(a.cos() * rr, y, a.sin() * rr)
                    })
                    .collect();
                tube_path(&mut g, &ring, &vec![0.018; ring.len()], 5, |_| white);
            }
            let mut bird = Geo::default();
            let inside = Vec3::Y * top * 0.45;
            ball(&mut bird, inside, Vec3::new(r * 0.16, r * 0.12, r * 0.22), 1, light);
            vec![
                Part::new(g, Vec3::ZERO, true),
                Part::new(bird, inside, false).glowing(1.6).moving(Motion::Bob { amp: top * 0.12, rate: 1.7, phase: 0.0 }).moving(Motion::Spin { rate: 0.7 }),
            ]
        }
        Family::Hand => {
            // An open hand, palm up, fingers curling a little; a small light rests in the palm.
            let mut g = Geo::default();
            let wrist = Vec3::ZERO;
            let palm = Vec3::Y * h * 0.62;
            tube_path(&mut g, &[wrist, Vec3::Y * h * 0.3, palm - Vec3::Y * r * 0.1], &[r * 0.35, r * 0.3, r * 0.38], 14, |_| white);
            g.blob(palm, Vec3::new(r * 0.55, r * 0.18, r * 0.5), 2, 9, 0.03, white, white);
            for f in 0..5 {
                let a = -0.9 + f as f32 * 0.45;
                let thumb = f == 0;
                let root = palm + Vec3::new(a.sin() * r * 0.5, r * 0.05, -a.cos() * r * 0.2 + if thumb { r * 0.3 } else { 0.0 });
                let dir = Vec3::new(a.sin() * 0.6, 0.45, -a.cos() * 0.8).normalize();
                let len = r * if thumb { 0.55 } else { d.range(0.7, 0.95) };
                let curl = d.range(0.35, 0.7);
                let pts: Vec<Vec3> = (0..=5)
                    .map(|i| {
                        let t = i as f32 / 5.0;
                        root + dir * len * t + Vec3::Y * (len * curl * t * t)
                    })
                    .collect();
                tube_path(&mut g, &pts, &(0..=5).map(|i| r * (0.11 - 0.035 * i as f32 / 5.0)).collect::<Vec<_>>(), 8, |_| white);
            }
            let mut orb = Geo::default();
            let rest = palm + Vec3::Y * r * 0.4;
            ball(&mut orb, rest, Vec3::splat(r * 0.2), 2, light);
            vec![Part::new(g, Vec3::ZERO, true), Part::new(orb, rest, false).glowing(1.8).moving(Motion::Bob { amp: r * 0.12, rate: 0.35, phase: 0.0 })]
        }
        Family::Mobile => {
            // A balanced tree of arms hanging from the ceiling: each arm turns on its thread, carrying the arms below.
            let mut parts = Vec::new();
            let mut thread = Geo::default();
            tube_path(&mut thread, &[Vec3::ZERO, -Vec3::Y * h * 0.25], &[0.006, 0.006], 4, |_| white);
            parts.push(Part::new(thread, Vec3::ZERO, false));
            mobile_arm(&mut parts, &mut d, None, -Vec3::Y * h * 0.25, r, h * 0.75, 0, light);
            parts
        }
        Family::Constellation => {
            let mut parts = Vec::new();
            for c in 0..3 {
                let mut g = Geo::default();
                let n = 10 + d.below(10);
                for _ in 0..n {
                    let p = Vec3::new(d.range(-r, r), -d.range(h * 0.1, h), d.range(-r, r));
                    ball(&mut g, p, Vec3::splat(d.range(0.03, 0.09)), 1, light);
                }
                parts.push(
                    Part::new(g, Vec3::ZERO, false)
                        .glowing(1.8 - c as f32 * 0.3)
                        .moving(Motion::Spin { rate: d.range(0.03, 0.12) * if c % 2 == 0 { 1.0 } else { -1.0 } })
                        .moving(Motion::Bob { amp: 0.08, rate: d.range(0.2, 0.5), phase: c as f32 }),
                );
            }
            parts
        }
        Family::Chimes => {
            let mut parts = Vec::new();
            let mut hoop = Geo::default();
            let ring: Vec<Vec3> = (0..=40)
                .map(|k| {
                    let a = k as f32 / 40.0 * TAU;
                    Vec3::new(a.cos() * r, -h * 0.3, a.sin() * r)
                })
                .collect();
            tube_path(&mut hoop, &ring, &vec![0.02; ring.len()], 6, |_| white);
            for k in 0..4 {
                let a = k as f32 / 4.0 * TAU;
                tube_path(&mut hoop, &[Vec3::ZERO, Vec3::new(a.cos() * r, -h * 0.3, a.sin() * r)], &[0.004, 0.004], 3, |_| white);
            }
            parts.push(Part::new(hoop, Vec3::ZERO, true));
            let n = 7 + d.below(6);
            for i in 0..n {
                let a = i as f32 / n as f32 * TAU;
                let top = Vec3::new(a.cos() * r * 0.9, -h * 0.3, a.sin() * r * 0.9);
                let len = h * d.range(0.3, 0.68);
                let mut g = Geo::default();
                tube_path(&mut g, &[top, top - Vec3::Y * 0.25], &[0.003, 0.003], 3, |_| white);
                tube_path(&mut g, &[top - Vec3::Y * 0.25, top - Vec3::Y * (0.25 + len)], &[0.035, 0.035], 10, |_| white);
                parts.push(Part::new(g, top, true).moving(Motion::Sway {
                    axis: Vec3::new(-a.sin(), 0.0, a.cos()),
                    amp: 0.05,
                    rate: d.range(0.6, 1.0),
                    phase: d.range(0.0, TAU),
                }));
            }
            parts
        }
        Family::Rain => {
            let mut g = Geo::default();
            let n = 260 + d.below(140);
            for _ in 0..n {
                let p = Vec3::new(d.range(-r, r), -d.range(0.3, h), d.range(-r, r) * 0.6);
                let len = d.range(0.05, 0.12);
                g.blob(p, Vec3::new(len * 0.3, len, len * 0.3), 0, 1, 0.0, white, white);
            }
            vec![Part::new(g, Vec3::ZERO, true).moving(Motion::Sway { axis: Vec3::X, amp: 0.01, rate: 0.2, phase: 0.0 })]
        }
    }
}

fn shifted(mut g: Geo, by: Vec3) -> Geo {
    for p in g.pos.iter_mut() {
        *p = (Vec3::from(*p) + by).to_array();
    }
    g
}

/// A four-sided crystal: from `base` along `dir`, `len` long and `w` wide.
fn crystal(g: &mut Geo, base: Vec3, dir: Vec3, len: f32, w: f32, c: Rgb) {
    let (u, v) = crate::procgen::geo::basis(dir);
    let mid = base + dir * len * 0.25;
    let tip = base + dir * len;
    let ring: Vec<Vec3> = (0..4).map(|k| mid + (u * (k as f32 * PI * 0.5).cos() + v * (k as f32 * PI * 0.5).sin()) * w).collect();
    for k in 0..4 {
        let (a, b) = (ring[k], ring[(k + 1) % 4]);
        g.tri(a, b, tip, c, c, c);
        g.tri(b, a, base, c, c, c);
    }
}

/// An abstract standing figure at `at`: a turned body, a neck and a head (part 1, which may watch the viewer). `lean` tilts it sideways.
fn figure(d: &mut Draw, at: Vec3, r: f32, h: f32, lean: f32, c: Rgb) -> Vec<Part> {
    let shoulder = h * 0.78;
    let hip = h * d.range(0.42, 0.5);
    let profile = [
        (r * 0.55, 0.0),
        (r * 0.42, h * 0.05),
        (r * 0.3, hip * 0.6),
        (r * d.range(0.5, 0.65), hip),
        (r * 0.42, h * 0.62),
        (r * d.range(0.6, 0.75), shoulder),
        (r * 0.25, h * 0.84),
        (r * 0.12, h * 0.86),
    ];
    let mut body = Geo::default();
    lathe(&mut body, Vec3::ZERO, &profile, 22, |_| c);
    // Arms folded close to the body.
    for s in [-1.0f32, 1.0] {
        let sh = Vec3::new(s * r * 0.62, shoulder - 0.02, 0.0);
        let elbow = Vec3::new(s * r * 0.75, h * 0.56, r * 0.25);
        let hand = Vec3::new(s * r * 0.15, h * d.range(0.55, 0.72), r * 0.45);
        tube_path(&mut body, &[sh, elbow, hand], &[r * 0.13, r * 0.11, r * 0.09], 8, |_| c);
    }
    let tilt = Mat3::from_rotation_z(lean);
    super::shape::place(&mut body, 0, tilt, at);
    let mut head = Geo::default();
    let neck_top = h * 0.88;
    let head_c = Vec3::new(0.0, neck_top + r * 0.42, r * 0.04);
    head.blob(head_c, Vec3::new(r * 0.32, r * 0.42, r * 0.36), 2, d.below(1000) as u32, 0.02, c, c);
    // A nose and a brow so the head has a front that can be seen to turn.
    head.blob(head_c + Vec3::new(0.0, -r * 0.02, r * 0.33), Vec3::new(r * 0.05, r * 0.11, r * 0.07), 1, 3, 0.0, c, c);
    let pivot = tilt * Vec3::Y * neck_top + at;
    super::shape::place(&mut head, 0, tilt, at);
    vec![Part::new(body, at, true), Part::new(head, pivot, true)]
}

/// One arm of a mobile and the arms hanging from it.
#[allow(clippy::too_many_arguments)]
fn mobile_arm(parts: &mut Vec<Part>, d: &mut Draw, parent: Option<usize>, pivot: Vec3, span: f32, drop: f32, level: u32, light: Rgb) {
    let white = [1.0, 1.0, 1.0];
    let mut g = Geo::default();
    let left = pivot + Vec3::new(-span * d.range(0.4, 0.6), -0.05, 0.0);
    let right = pivot + Vec3::new(span * d.range(0.4, 0.6), 0.05, 0.0);
    tube_path(&mut g, &[left, pivot + Vec3::Y * 0.05, right], &[0.012, 0.016, 0.012], 5, |_| white);
    let ends = [left, right];
    let idx = parts.len();
    let mut me = Part::new(g, pivot, true).moving(Motion::Spin { rate: d.range(0.08, 0.3) * if level % 2 == 0 { 1.0 } else { -1.0 } });
    me.parent = parent;
    parts.push(me);
    for (k, end) in ends.iter().enumerate() {
        let hang = drop * d.range(0.25, 0.4);
        let mut tg = Geo::default();
        tube_path(&mut tg, &[*end, *end - Vec3::Y * hang], &[0.004, 0.004], 3, |_| white);
        let below = *end - Vec3::Y * hang;
        if level < 2 && (k == 1 || d.chance(0.5)) {
            let mut thread = Part::new(tg, *end, false);
            thread.parent = Some(idx);
            parts.push(thread);
            mobile_arm(parts, d, Some(idx), below, span * 0.55, drop * 0.6, level + 1, light);
        } else {
            // A disc or a glowing ball at the end.
            if d.chance(0.6) {
                let rr = span * d.range(0.12, 0.22);
                let a = d.range(0.0, PI);
                obox(
                    &mut tg,
                    below - Vec3::Y * rr,
                    [Vec3::new(a.cos(), 0.0, a.sin()), Vec3::Y, Vec3::new(-a.sin(), 0.0, a.cos())],
                    Vec3::new(rr, rr, 0.012),
                    white,
                );
                let mut p = Part::new(tg, *end, true);
                p.parent = Some(idx);
                parts.push(p);
            } else {
                ball(&mut tg, below - Vec3::Y * 0.1, Vec3::splat(span * 0.08), 1, light);
                let mut p = Part::new(tg, *end, false).glowing(1.2);
                p.parent = Some(idx);
                parts.push(p);
            }
        }
    }
}

/// Where part `i` of `parts` is now, in the piece's space (a parent's transform carries its children).
pub fn part_transform(parts: &[Part], i: usize, t: f32, stir: &Stir) -> Mat4 {
    let own = motion_transform(&parts[i], t, stir);
    match parts[i].parent {
        Some(p) if p < i => part_transform(parts, p, t, stir) * own,
        _ => own,
    }
}

fn motion_transform(p: &Part, t: f32, s: &Stir) -> Mat4 {
    let touched = (1.0 - s.since_touch / 4.0).clamp(0.0, 1.0);
    let mut m = Mat4::IDENTITY;
    let about = |rot: Mat4| Mat4::from_translation(p.pivot) * rot * Mat4::from_translation(-p.pivot);
    for motion in &p.motions {
        let step = match *motion {
            Motion::Spin { rate } => about(Mat4::from_rotation_y(t * rate + touched * touched * 2.5 * rate.signum())),
            Motion::Bob { amp, rate, phase } => Mat4::from_translation(Vec3::Y * amp * (t * rate * TAU * 0.5 + phase).sin()),
            Motion::Breathe { amp, rate, phase } => {
                let k = 1.0 + (amp * (1.0 + touched)) * (t * rate * TAU * 0.5 + phase).sin();
                about(Mat4::from_scale(Vec3::splat(k)))
            }
            Motion::Sway { axis, amp, rate, phase } => {
                let a = (amp * (1.0 + 2.0 * touched)) * (t * rate * TAU * 0.5 + phase).sin();
                about(Mat4::from_axis_angle(axis.normalize_or(Vec3::X), a))
            }
            Motion::Watch { limit } => {
                let to = s.eye - p.pivot;
                let want = if s.near > 0.05 { to.x.atan2(to.z).clamp(-limit, limit) * s.near.sqrt() } else { 0.0 };
                // A slow drift adds life when nobody is near.
                let idle = 0.25 * (t * 0.13).sin();
                about(Mat4::from_rotation_y(want + idle * (1.0 - s.near)))
            }
            Motion::Orbit { radius, rate, phase, tilt } => {
                let a = t * rate * (1.0 + 2.0 * touched) + phase;
                let off = Quat::from_rotation_x(tilt) * Vec3::new(a.cos() * radius, 0.0, a.sin() * radius);
                Mat4::from_translation(off)
            }
            Motion::Open { axis, angle } => {
                let open = (s.near * 0.75 + s.reveal).clamp(0.0, 1.0);
                // Closed the petal is folded in by `angle`; open it lies back.
                about(Mat4::from_axis_angle(axis.normalize_or(Vec3::X), angle * (1.0 - open)))
            }
            Motion::Beat { amp, rate } => {
                let r = rate * (1.0 + 0.8 * s.near + touched);
                let ph = (t * r).fract();
                let pulse = (-((ph - 0.08) / 0.05).powi(2)).exp() + 0.6 * (-((ph - 0.3) / 0.05).powi(2)).exp();
                about(Mat4::from_scale(Vec3::splat(1.0 + amp * pulse)))
            }
            Motion::Tremble { amp } => {
                let k = amp * (s.near * s.near + touched);
                about(Mat4::from_rotation_x(k * (t * 37.0).sin()) * Mat4::from_rotation_z(k * (t * 29.0 + 1.0).sin()))
            }
        };
        m = step * m;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_builds_within_a_budget_and_has_a_part() {
        for f in Family::ALL {
            let mut d = Draw::new(&[1]);
            let (r, h, _) = f.proportions(&mut d);
            for seed in [1.0, 222.0, 777.7] {
                let parts = build(f, r, h, seed, [1.0, 0.9, 0.7]);
                assert!(!parts.is_empty(), "{f:?}");
                let tris: usize = parts.iter().map(|p| p.geo.tris()).sum();
                assert!(tris > 20 && tris < 40_000, "{f:?} has {tris} triangles");
                for (i, p) in parts.iter().enumerate() {
                    assert!(p.parent.is_none_or(|q| q < i), "{f:?}: a parent comes first");
                }
                let (lo, hi) = parts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| {
                    let (a, b) = p.geo.bounds();
                    (lo.min(a), hi.max(b))
                });
                if f.hangs() {
                    assert!(hi.y <= 0.05 && lo.y >= -h * 1.3, "{f:?} hangs below its point: {lo:?} {hi:?}");
                } else {
                    assert!(lo.y >= -0.2 && hi.y <= h * 1.4, "{f:?} stands on its base: {lo:?} {hi:?}");
                }
            }
        }
    }

    #[test]
    fn a_figure_watches_the_viewer_and_a_bloom_opens_when_they_come_near() {
        let parts = build(Family::Figure, 0.4, 2.0, 5.0, [1.0; 3]);
        let head = 1;
        let nose = Vec3::new(0.0, 2.0 * 0.88 + 0.4 * 0.42, 0.4 * 0.4);
        let far = Stir { near: 0.0, eye: Vec3::new(5.0, 1.6, 0.0), since_touch: 99.0, reveal: 0.0 };
        let near = Stir { near: 1.0, ..far };
        let a = part_transform(&parts, head, 0.0, &far).transform_point3(nose);
        let b = part_transform(&parts, head, 0.0, &near).transform_point3(nose);
        assert!(b.x > a.x + 0.05, "the face turns toward a viewer on its right: {a:?} -> {b:?}");

        let parts = build(Family::Bloom, 0.8, 1.6, 9.0, [1.0; 3]);
        let petal = parts.len() - 1;
        let tip = |s: &Stir| {
            let (lo, hi) = parts[petal].geo.bounds();
            part_transform(&parts, petal, 0.0, s).transform_point3((lo + hi) * 0.5)
        };
        let closed = tip(&far);
        let open = tip(&Stir { near: 1.0, eye: Vec3::new(0.0, 1.6, 2.0), since_touch: 99.0, reveal: 1.0 });
        let r = |p: Vec3| Vec3::new(p.x, 0.0, p.z).length();
        assert!(r(open) > r(closed) + 0.05, "the petals open outward: {closed:?} -> {open:?}");
    }
}
