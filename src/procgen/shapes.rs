//! The shape of every species: a parametric low-poly model, built from a height, a seed and a colour tint.
//!
//! Each model is drawn from what makes the real plant recognisable at a glance: an oak's broad crown of rounded clumps on a short stout trunk,
//! a birch's white bark with dark scars and a light, narrow crown, a spruce's stack of drooping tiers, a Scots pine's bare orange trunk under a
//! flat cap, a willow's curtains of hanging strands, a cherry's pink cloud; a daisy's white rays round a yellow eye, a poppy's red cup, a
//! lavender's purple spikes, a bluebell's nodding bells, a foxglove's tall tower of bells. The seed varies lean, proportion and clumping so no
//! two plants are alike, but the same seed always gives the same plant. Models stand on the origin, point up +Y, and are budgeted in triangles
//! (a tree under about 560, a flower under about 100, a grass tuft about 12) because a chunk holds thousands of them.

use super::flora::{self, SpeciesId};
use super::geo::{hex, mix, shade, Geo, Rgb};
use super::noise::Rng;
use glam::Vec3;
use std::f32::consts::TAU;

/// The most triangles a model of each kind may have (tested): chunks hold thousands of plants.
pub const BUDGET: [(flora::Kind, usize); 4] = [(flora::Kind::Tree, 560), (flora::Kind::Shrub, 260), (flora::Kind::Flower, 110), (flora::Kind::Grass, 40)];

/// A point on a species' palette: `tint` 0 to 1 slides along the listed colours.
pub fn tint_colour(id: SpeciesId, tint: f32) -> Rgb {
    let pal = flora::species(id).palette();
    let x = tint.clamp(0.0, 0.9999) * (pal.len() - 1).max(1) as f32;
    if pal.len() == 1 {
        return pal[0];
    }
    let i = (x as usize).min(pal.len() - 2);
    mix(pal[i], pal[i + 1], x - i as f32)
}

/// Builds the model of a species at a height (metres), varied by a seed and coloured by a tint (0 to 1).
pub fn build(id: SpeciesId, height: f32, seed: u32, tint: f32) -> Geo {
    build_lod(id, height, seed, tint, false)
}

/// How much of a species' height sways in the wind: grass and flowers move freely, willow strands and crowns a little, trunks hardly.
fn sway_amount(key: &str) -> f32 {
    match key {
        "meadow_grass" | "timothy" => 1.0,
        "bracken" | "bluebell" | "harebell" | "ox_eye_daisy" | "poppy" | "cornflower" | "buttercup" | "dandelion" | "red_clover" => 0.9,
        "lavender" | "foxglove" => 0.6,
        "willow" => 0.45,
        "birch" | "cherry" => 0.3,
        "oak" | "hawthorn" => 0.22,
        "pine" => 0.16,
        _ => 0.1,
    }
}

/// [`build`], or (`far`) a cheaper model for distant plants: spherical clumps drop from 80 to 20 triangles.
pub fn build_lod(id: SpeciesId, height: f32, seed: u32, tint: f32, far: bool) -> Geo {
    let mut rng = Rng::at(seed, id.0 as i32, 7);
    let c = tint_colour(id, tint);
    let h = height;
    let mut g = Geo { max_subdiv: if far { 0 } else { 2 }, far, ..Geo::default() };
    match flora::species(id).key {
        "oak" => oak(&mut g, h, c, &mut rng),
        "birch" => birch(&mut g, h, c, &mut rng),
        "spruce" => spruce(&mut g, h, c, &mut rng),
        "pine" => pine(&mut g, h, c, &mut rng),
        "willow" => willow(&mut g, h, c, &mut rng),
        "cherry" => cherry(&mut g, h, c, &mut rng),
        "hawthorn" => hawthorn(&mut g, h, c, &mut rng),
        "bracken" => bracken(&mut g, h, c, &mut rng),
        "ox_eye_daisy" => daisy(&mut g, h, c, &mut rng),
        "poppy" => poppy(&mut g, h, c, &mut rng),
        "cornflower" => cornflower(&mut g, h, c, &mut rng),
        "lavender" => lavender(&mut g, h, c, &mut rng),
        "bluebell" => bluebell(&mut g, h, c, &mut rng),
        "dandelion" => dandelion(&mut g, h, c, &mut rng),
        "buttercup" => buttercup(&mut g, h, c, &mut rng),
        "red_clover" => clover(&mut g, h, c, &mut rng),
        "foxglove" => foxglove(&mut g, h, c, &mut rng),
        "harebell" => harebell(&mut g, h, c, &mut rng),
        "meadow_grass" => grass(&mut g, h, c, &mut rng, 6, false),
        "timothy" => grass(&mut g, h, c, &mut rng, 5, true),
        other => unreachable!("no shape for species {other}"),
    }
    g.set_sway(sway_amount(flora::species(id).key), h);
    g
}

fn dir(angle: f32) -> Vec3 {
    Vec3::new(angle.cos(), 0.0, angle.sin())
}

fn leafy(c: Rgb) -> (Rgb, Rgb) {
    (shade(mix(c, [0.0, 0.05, 0.0], 0.15), 0.42), shade(c, 1.12))
}

const STEM: &str = "#4f8a2e";
const STEM_TOP: &str = "#6aa23c";

/// A thin stem leaning from the origin to `top`, bent in the middle.
fn stem(g: &mut Geo, top: Vec3, bow: Vec3, r: f32) {
    let (lo, hi) = (hex(STEM), hex(STEM_TOP));
    let mid = top * 0.5 + bow;
    g.tube(Vec3::ZERO, mid, r * 1.2, r, 3, lo, mix(lo, hi, 0.5), false);
    g.tube(mid, top, r, r * 0.8, 3, mix(lo, hi, 0.5), hi, false);
}

/// A strap leaf rising from `from`, leaning out toward `towards` and arching over.
fn strap(g: &mut Geo, from: Vec3, towards: Vec3, len: f32, width: f32, c: Rgb) {
    let side = Vec3::Y.cross(towards).normalize_or_zero();
    let mid = from + towards * len * 0.45 + Vec3::Y * len * 0.5;
    let tip = from + towards * len * 0.95 + Vec3::Y * len * 0.5;
    g.ribbon(&[from, mid, tip], &[width, width * 0.9, 0.0], side, shade(c, 0.55), c);
}

// ----------------------------------------------------------------------------------------------------------------- trees

fn oak(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let (bark_lo, bark) = (hex("#3c2f24"), hex("#5b4733"));
    let trunk = 0.40 * h;
    let r = 0.03 * h;
    g.tube(Vec3::ZERO, Vec3::new(0.0, trunk, 0.0), r * 1.7, r * 0.85, 8, bark_lo, bark, false);
    let a0 = rng.range(0.0, TAU);
    for i in 0..4 {
        let d = dir(a0 + i as f32 * TAU / 4.0 + rng.range(-0.3, 0.3));
        let from = Vec3::new(0.0, trunk * 0.9, 0.0);
        if !g.far {
            g.tube(from, from + d * 0.2 * h + Vec3::Y * 0.2 * h, r * 0.7, r * 0.3, 5, bark, bark, false);
        }
    }
    let (lo, hi) = leafy(c);
    g.blob(Vec3::new(0.0, 0.68 * h, 0.0), Vec3::new(0.27, 0.22, 0.27) * h, 1, rng.bits(), 0.17, lo, hi);
    let n = 5;
    for i in 0..n {
        let d = dir(a0 + i as f32 * TAU / n as f32 + rng.range(-0.3, 0.3));
        let at = d * rng.range(0.15, 0.21) * h + Vec3::new(0.0, rng.range(0.56, 0.70) * h, 0.0);
        let k = rng.range(0.85, 1.1);
        g.blob(at, Vec3::new(0.19, 0.16, 0.19) * h * k, 1, rng.bits(), 0.17, lo, shade(hi, rng.range(0.92, 1.06)));
    }
}

fn birch(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let (white, scar) = (hex("#e6e2d4"), hex("#3f372f"));
    let lean = dir(rng.range(0.0, TAU)) * rng.range(0.01, 0.05) * h;
    let top = 0.82 * h;
    let segs = 6;
    let mut prev = Vec3::ZERO;
    for i in 1..=segs {
        let t = i as f32 / segs as f32;
        let p = lean * t * t + Vec3::Y * top * t;
        let (r0, r1) = (0.020 * h * (1.0 - 0.55 * (t - 1.0 / segs as f32)), 0.020 * h * (1.0 - 0.55 * t));
        g.tube(prev, p, r0 * 1.3, r1 * 1.3, 6, white, white, false);
        // A dark scar where the segments meet.
        if !g.far {
            g.tube(p - Vec3::Y * 0.012 * h, p + Vec3::Y * 0.012 * h, r1 * 1.12, r1 * 1.12, 6, scar, scar, false);
        }
        prev = p;
    }
    let (lo, hi) = leafy(c);
    let a0 = rng.range(0.0, TAU);
    for i in 0..5 {
        let t = i as f32 / 4.0;
        let at = lean * (0.5 + 0.5 * t) + dir(a0 + i as f32 * 2.1) * (0.05 * h) * (1.0 - 0.3 * t) + Vec3::Y * (0.46 + 0.42 * t) * h;
        let s = (1.0 - 0.4 * t) * rng.range(0.92, 1.08);
        g.blob(at, Vec3::new(0.17, 0.17, 0.17) * h * s, 1, rng.bits(), 0.2, lo, shade(hi, 1.05));
    }
}

fn spruce(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let bark = hex("#4a3a2c");
    g.tube(Vec3::ZERO, Vec3::new(0.0, 0.96 * h, 0.0), 0.02 * h, 0.006 * h, 5, bark, bark, false);
    let tiers = 7;
    let (lo, hi) = (shade(c, 0.5), shade(c, 1.08));
    for i in 0..tiers {
        let t = i as f32 / (tiers - 1) as f32;
        let r = (0.21 * (1.0 - t).powf(0.85) + 0.02) * h * rng.range(0.92, 1.08);
        let base = Vec3::new(rng.range(-0.01, 0.01) * h, (0.09 + 0.78 * t) * h, rng.range(-0.01, 0.01) * h);
        g.cone(base, r, 0.22 * h, 9, rng.range(0.0, TAU), lo, shade(hi, 0.95 + 0.1 * t));
    }
}

fn pine(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let (grey, orange) = (hex("#6a5748"), hex("#b8693a"));
    let lean = dir(rng.range(0.0, TAU)) * rng.range(0.0, 0.05) * h;
    let top = 0.76 * h;
    g.tube(Vec3::ZERO, lean * 0.3 + Vec3::Y * top * 0.5, 0.028 * h, 0.02 * h, 7, grey, mix(grey, orange, 0.5), false);
    g.tube(lean * 0.3 + Vec3::Y * top * 0.5, lean + Vec3::Y * top, 0.02 * h, 0.012 * h, 7, mix(grey, orange, 0.5), orange, false);
    let (lo, hi) = leafy(c);
    g.blob(lean + Vec3::new(0.0, 0.9 * h, 0.0), Vec3::new(0.2, 0.09, 0.2) * h, 1, rng.bits(), 0.2, lo, hi);
    let a0 = rng.range(0.0, TAU);
    for i in 0..3 {
        let d = dir(a0 + i as f32 * TAU / 3.0 + rng.range(-0.3, 0.3));
        let y = rng.range(0.76, 0.86);
        g.tube(lean + Vec3::Y * top * 0.95, lean + d * 0.1 * h + Vec3::Y * y * h, 0.01 * h, 0.006 * h, 4, orange, orange, false);
        g.blob(lean + d * 0.14 * h + Vec3::Y * (y + 0.04) * h, Vec3::new(0.14, 0.07, 0.14) * h, 1, rng.bits(), 0.2, lo, shade(hi, 0.95));
    }
}

fn willow(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let bark = hex("#5a4a36");
    let a0 = rng.range(0.0, TAU);
    let lean = dir(a0) * 0.04 * h;
    g.tube(Vec3::ZERO, lean + Vec3::Y * 0.38 * h, 0.05 * h, 0.03 * h, 7, shade(bark, 0.7), bark, false);
    for i in 0..2 {
        let d = dir(a0 + i as f32 * 3.0);
        g.tube(lean + Vec3::Y * 0.34 * h, d * 0.12 * h + Vec3::Y * 0.6 * h, 0.025 * h, 0.012 * h, 5, bark, bark, false);
    }
    let (lo, hi) = leafy(c);
    g.blob(Vec3::new(0.0, 0.66 * h, 0.0), Vec3::new(0.34, 0.2, 0.34) * h, 1, rng.bits(), 0.14, shade(lo, 0.8), shade(hi, 0.9));
    let n = if g.far { 10 } else { 26 };
    for i in 0..n {
        let a = a0 + i as f32 / n as f32 * TAU + rng.range(-0.15, 0.15);
        let d = dir(a);
        let start = d * rng.range(0.22, 0.36) * h + Vec3::Y * 0.60 * h;
        let drop = rng.range(0.42, 0.5) * h;
        let pts = [start, start + d * 0.04 * h - Vec3::Y * drop * 0.4, start + d * 0.05 * h - Vec3::Y * drop * 0.8, start + d * 0.03 * h - Vec3::Y * drop];
        g.ribbon(&pts, &[0.07 * h, 0.065 * h, 0.045 * h, 0.0], d.cross(Vec3::Y), shade(hi, 0.9), mix(hi, [0.5, 0.55, 0.12], 0.45));
    }
}

fn cherry(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let bark = hex("#523a3c");
    let a0 = rng.range(0.0, TAU);
    g.tube(Vec3::ZERO, Vec3::new(0.0, 0.34 * h, 0.0), 0.045 * h, 0.03 * h, 7, shade(bark, 0.7), bark, false);
    for i in 0..3 {
        let d = dir(a0 + i as f32 * TAU / 3.0);
        g.tube(Vec3::new(0.0, 0.3 * h, 0.0), d * 0.2 * h + Vec3::Y * 0.6 * h, 0.025 * h, 0.01 * h, 5, bark, bark, false);
    }
    let (lo, hi) = (shade(c, 0.62), shade(c, 1.05));
    g.blob(Vec3::new(0.0, 0.68 * h, 0.0), Vec3::new(0.24, 0.2, 0.24) * h, 1, rng.bits(), 0.17, lo, hi);
    for i in 0..4 {
        let d = dir(a0 + i as f32 * TAU / 4.0 + rng.range(-0.3, 0.3));
        let at = d * 0.2 * h + Vec3::new(0.0, rng.range(0.58, 0.74) * h, 0.0);
        g.blob(at, Vec3::new(0.17, 0.14, 0.17) * h, 1, rng.bits(), 0.17, lo, shade(hi, rng.range(0.95, 1.08)));
    }
    // A few pale blossom clusters on the surface of the crown.
    for _ in 0..5 {
        let d = dir(rng.range(0.0, TAU)) * rng.range(0.1, 0.3) * h;
        g.blob(d + Vec3::new(0.0, rng.range(0.74, 0.88) * h, 0.0), Vec3::splat(0.045 * h), 0, rng.bits(), 0.2, shade(c, 1.1), hex("#ffe4ec"));
    }
}

// --------------------------------------------------------------------------------------------------------------- shrubs

fn hawthorn(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let bark = hex("#4c3b30");
    g.tube(Vec3::ZERO, Vec3::new(0.0, 0.3 * h, 0.0), 0.04 * h, 0.025 * h, 5, bark, bark, false);
    let (lo, hi) = leafy(c);
    let a0 = rng.range(0.0, TAU);
    g.blob(Vec3::new(0.0, 0.5 * h, 0.0), Vec3::new(0.34, 0.3, 0.34) * h, 1, rng.bits(), 0.18, lo, hi);
    for i in 0..3 {
        let d = dir(a0 + i as f32 * TAU / 3.0 + rng.range(-0.4, 0.4));
        g.blob(
            d * 0.25 * h + Vec3::new(0.0, rng.range(0.36, 0.55) * h, 0.0),
            Vec3::new(0.22, 0.2, 0.22) * h,
            0,
            rng.bits(),
            0.18,
            lo,
            shade(hi, rng.range(0.95, 1.08)),
        );
    }
    for _ in 0..5 {
        let d = dir(rng.range(0.0, TAU)) * rng.range(0.1, 0.4) * h;
        g.blob(d + Vec3::new(0.0, rng.range(0.6, 0.78) * h, 0.0), Vec3::splat(0.05 * h), 0, rng.bits(), 0.15, hex("#e8e4d8"), hex("#fffaf0"));
    }
}

fn bracken(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let fronds = 7;
    let a0 = rng.range(0.0, TAU);
    for i in 0..fronds {
        let a = a0 + i as f32 / fronds as f32 * TAU + rng.range(-0.25, 0.25);
        let d = dir(a);
        let reach = rng.range(0.55, 0.8) * h;
        let tall = rng.range(0.8, 1.0);
        let pts: Vec<Vec3> = (0..5)
            .map(|k| {
                let t = k as f32 / 4.0;
                d * reach * t + Vec3::Y * h * tall * (2.3 * t - 1.6 * t * t) / 0.827
            })
            .collect();
        let w = 0.3 * h;
        g.ribbon(&pts, &[0.04 * h, w * 0.75, w, w * 0.6, 0.0], d.cross(Vec3::Y), shade(c, 0.5), shade(c, rng.range(0.95, 1.15)));
    }
}

// -------------------------------------------------------------------------------------------------------------- flowers

/// A ring of `n` rays round a centre, each a spearhead (wide at the root, pointed at the tip), raised a little toward the outside.
#[allow(clippy::too_many_arguments)]
fn rays(g: &mut Geo, at: Vec3, n: u32, inner: f32, outer: f32, half_width: f32, lift: f32, spin: f32, near: Rgb, far: Rgb) {
    for k in 0..n {
        let d = dir(spin + k as f32 / n as f32 * TAU);
        let side = d.cross(Vec3::Y);
        let root = at + d * inner;
        let tip = at + d * outer + Vec3::Y * lift;
        g.tri2(root + side * half_width, root - side * half_width, tip, near, near, far);
    }
}

/// `n` stems standing in a clump, each lean-out to a head that `head` draws, plus the clump's leaf at the base.
fn clump(g: &mut Geo, h: f32, rng: &mut Rng, n: u32, stem_r: f32, mut head: impl FnMut(&mut Geo, Vec3, &mut Rng)) {
    let a0 = rng.range(0.0, TAU);
    let (lo, hi) = (hex(STEM), hex(STEM_TOP));
    for i in 0..n {
        let a = a0 + i as f32 * TAU / n as f32 + rng.range(-0.5, 0.5);
        let top = dir(a) * rng.range(0.04, 0.16) * h + Vec3::Y * h * rng.range(0.72, 1.0);
        g.tube(dir(a) * 0.01 * h, top - Vec3::Y * 0.01 * h, stem_r * 1.4, stem_r, 3, lo, hi, false);
        head(g, top, rng);
    }
    strap(g, Vec3::ZERO, dir(a0 + 1.0), 0.32 * h, 0.07 * h, lo);
}

fn disc(g: &mut Geo, at: Vec3, r: f32, sides: u32, c: Rgb) {
    for k in 0..sides {
        let (a, b) = (dir(k as f32 / sides as f32 * TAU), dir((k + 1) as f32 / sides as f32 * TAU));
        g.tri(at + Vec3::Y * r * 0.15, at + b * r, at + a * r, c, shade(c, 0.85), shade(c, 0.85));
    }
}

fn daisy(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    clump(g, h, rng, 3, 0.012 * h, |g, top, rng| {
        let r = 0.21 * h;
        rays(g, top, 8, r * 0.2, r * 1.5, r * 0.34, r * 0.15, rng.range(0.0, TAU), shade(c, 0.85), c);
        disc(g, top + Vec3::Y * r * 0.12, r * 0.5, 6, hex("#f2b807"));
    });
}

fn poppy(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    clump(g, h, rng, 2, 0.012 * h, |g, top, rng| {
        let r = 0.27 * h;
        rays(g, top, 4, r * 0.1, r * 1.45, r * 0.95, r * 0.7, rng.range(0.0, TAU), shade(c, 0.6), c);
        disc(g, top + Vec3::Y * r * 0.2, r * 0.32, 6, hex("#2a1a1a"));
    });
}

fn cornflower(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    clump(g, h, rng, 3, 0.011 * h, |g, top, rng| {
        let r = 0.18 * h;
        rays(g, top, 8, r * 0.2, r * 1.7, r * 0.32, r * 0.5, rng.range(0.0, TAU), shade(c, 0.75), c);
        disc(g, top + Vec3::Y * r * 0.2, r * 0.5, 6, mix(c, hex("#3a1c5a"), 0.7));
    });
}

fn lavender(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let leaf = hex("#7c9a6a");
    for i in 0..3 {
        strap(g, Vec3::ZERO, dir(i as f32 * 2.1 + rng.range(0.0, 1.0)), 0.35 * h, 0.04 * h, leaf);
    }
    for _ in 0..3 {
        let d = dir(rng.range(0.0, TAU)) * rng.range(0.02, 0.18) * h;
        let top = d + Vec3::Y * rng.range(0.8, 1.0) * h;
        let base = top - Vec3::Y * 0.28 * h;
        g.tube(Vec3::ZERO, base, 0.006 * h, 0.005 * h, 3, hex("#7c9a6a"), hex("#7c9a6a"), false);
        g.tube(base, top, 0.04 * h, 0.018 * h, 5, shade(c, 0.7), c, true);
    }
}

fn bluebell(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let d = dir(rng.range(0.0, TAU));
    let top = d * 0.28 * h + Vec3::Y * 0.9 * h;
    stem(g, top, Vec3::Y * 0.12 * h + d * 0.06 * h, 0.012 * h);
    for k in 0..4 {
        let t = 0.35 + 0.15 * k as f32;
        let on = d * 0.28 * h * t * t + Vec3::Y * 0.9 * h * (0.55 + 0.45 * t);
        let on = on.lerp(top, 0.0);
        g.tube(on, on - Vec3::Y * 0.22 * h + d * 0.02 * h, 0.015 * h, 0.07 * h, 5, shade(c, 1.2), c, false);
    }
    for i in 0..3 {
        strap(g, Vec3::ZERO, dir(i as f32 * 2.1 + rng.range(0.0, 1.0)), 0.9 * h, 0.16 * h, hex("#4a8a3a"));
    }
}

fn dandelion(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    for i in 0..3 {
        let d = dir(i as f32 * 2.1 + rng.range(0.0, 1.0));
        let side = d.cross(Vec3::Y);
        g.ribbon(
            &[Vec3::ZERO, d * 0.5 * h + Vec3::Y * 0.12 * h, d * 0.95 * h + Vec3::Y * 0.05 * h],
            &[0.1 * h, 0.2 * h, 0.0],
            side,
            shade(hex(STEM), 0.8),
            hex(STEM_TOP),
        );
    }
    let n = 1 + (rng.white() * 2.0) as u32;
    for i in 0..n {
        let top = dir(i as f32 * 3.0 + 1.0) * 0.08 * h * i as f32 + Vec3::Y * h * (1.0 - 0.15 * i as f32);
        g.tube(Vec3::ZERO, top - Vec3::Y * 0.01 * h, 0.03 * h, 0.022 * h, 3, hex(STEM), hex(STEM_TOP), false);
        rays(g, top, 10, 0.02 * h, 0.3 * h, 0.07 * h, 0.05 * h, rng.range(0.0, TAU), shade(c, 0.85), c);
    }
}

fn buttercup(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    clump(g, h, rng, 3, 0.011 * h, |g, top, rng| {
        let r = 0.14 * h;
        rays(g, top, 5, 0.0, r * 1.5, r * 0.8, r * 0.5, rng.range(0.0, TAU), shade(c, 0.8), c);
    });
}

fn clover(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    clump(g, h, rng, 2, 0.014 * h, |g, top, rng| {
        g.blob(top, Vec3::new(0.27, 0.32, 0.27) * h, 0, rng.bits(), 0.12, shade(c, 0.7), shade(c, 1.1));
    });
    for i in 0..3 {
        let d = dir(i as f32 * 2.1 + rng.range(0.0, 1.0));
        let at = d * 0.12 * h + Vec3::Y * 0.3 * h;
        let side = d.cross(Vec3::Y);
        g.tri2(at, at + d * 0.3 * h + side * 0.13 * h, at + d * 0.3 * h - side * 0.13 * h, hex("#3c7a38"), hex("#5a9a48"), hex("#5a9a48"));
    }
}

fn foxglove(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let lean = dir(rng.range(0.0, TAU)) * 0.04 * h;
    g.tube(Vec3::ZERO, lean + Vec3::Y * h, 0.014 * h, 0.006 * h, 4, hex(STEM), hex(STEM_TOP), false);
    let side = dir(rng.range(0.0, TAU));
    for k in 0..8 {
        let t = 0.35 + 0.62 * k as f32 / 7.0;
        let at = lean * t + Vec3::Y * h * t;
        let size = 1.0 - 0.6 * (t - 0.35);
        let tilt = side * 0.03 * h;
        g.tube(at + tilt, at + tilt * 2.4 - Vec3::Y * 0.1 * h * size, 0.012 * h, 0.05 * h * size, 5, shade(c, 1.25), shade(c, 0.9), false);
    }
    for i in 0..3 {
        strap(g, Vec3::ZERO, dir(i as f32 * 2.1 + rng.range(0.0, 1.0)), 0.3 * h, 0.07 * h, hex("#4c8a3c"));
    }
}

fn harebell(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng) {
    let a0 = rng.range(0.0, TAU);
    for j in 0..2 {
        let d = dir(a0 + j as f32 * 2.6);
        let hh = h * (1.0 - 0.15 * j as f32);
        let top = d * 0.2 * h + Vec3::Y * 0.95 * hh;
        stem(g, top, d * 0.1 * h, 0.008 * h);
        for k in 0..3 {
            let t = 0.55 + 0.22 * k as f32;
            let on = d * 0.2 * h * t * t + Vec3::Y * 0.95 * hh * (0.4 + 0.6 * t);
            g.tube(on, on - Vec3::Y * 0.2 * h, 0.01 * h, 0.06 * h, 5, shade(c, 1.2), c, false);
        }
    }
    strap(g, Vec3::ZERO, dir(a0 + 1.0), 0.3 * h, 0.04 * h, hex("#5c8a48"));
}

// --------------------------------------------------------------------------------------------------------------- grasses

fn grass(g: &mut Geo, h: f32, c: Rgb, rng: &mut Rng, blades: u32, seedheads: bool) {
    for i in 0..blades {
        let a = i as f32 / blades as f32 * TAU + rng.range(-0.4, 0.4);
        let d = dir(a);
        let len = h * rng.range(0.65, 1.0);
        let lean = rng.range(0.15, 0.55);
        let tip = d * lean * len + Vec3::Y * len;
        let w = 0.11 * h.max(0.35);
        g.ribbon(&[d * 0.02, tip], &[w, 0.0], d.cross(Vec3::Y), shade(c, 0.5), shade(c, rng.range(1.0, 1.2)));
    }
    if seedheads {
        for _ in 0..3 {
            let d = dir(rng.range(0.0, TAU)) * rng.range(0.02, 0.1);
            let top = d + Vec3::Y * h;
            // One tapered tube: a green stalk that thickens into a golden seed head.
            g.tube(Vec3::ZERO, top, 0.004, 0.011, 4, shade(c, 0.9), hex("#d8c880"), false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::geo::assert_wound_outward;
    use super::*;

    fn pos_eq(a: &Geo, b: &Geo) -> bool {
        a.pos == b.pos && a.idx == b.idx && a.col == b.col
    }

    #[test]
    fn every_species_builds_within_its_triangle_budget() {
        for (i, s) in flora::SPECIES.iter().enumerate() {
            let budget = BUDGET.iter().find(|(k, _)| *k == s.kind).unwrap().1;
            for seed in 0..12 {
                for h in [s.height.0, s.height.1] {
                    let g = build(SpeciesId(i as u8), h, seed, seed as f32 / 12.0);
                    assert!(g.tris() >= 4, "{} is empty", s.key);
                    assert!(g.tris() <= budget, "{}: {} triangles, budget {budget}", s.key, g.tris());
                    assert!(g.idx.iter().all(|i| (*i as usize) < g.pos.len()));
                    assert!(g.pos.iter().flatten().all(|v| v.is_finite()));
                }
            }
        }
    }

    #[test]
    fn plants_stand_on_the_ground_at_about_their_height() {
        for (i, s) in flora::SPECIES.iter().enumerate() {
            let h = (s.height.0 + s.height.1) / 2.0;
            for seed in 0..8 {
                let g = build(SpeciesId(i as u8), h, seed, 0.5);
                let (lo, hi) = g.bounds();
                assert!(lo.y > -0.05 * h - 0.02 && lo.y < 0.2 * h, "{} bottom {}", s.key, lo.y);
                assert!(hi.y > 0.7 * h && hi.y < 1.3 * h, "{} is {:.2} tall for a height of {h}", s.key, hi.y);
                let width = (hi.x - lo.x).max(hi.z - lo.z);
                assert!(width < 2.4 * (s.spread * h).max(0.25 * h) + 0.4, "{} is {width:.2} wide", s.key);
            }
        }
    }

    #[test]
    fn a_seed_makes_the_same_plant_and_another_seed_a_different_one() {
        for (i, s) in flora::SPECIES.iter().enumerate() {
            let id = SpeciesId(i as u8);
            let h = s.height.1;
            assert!(pos_eq(&build(id, h, 5, 0.3), &build(id, h, 5, 0.3)), "{} is not deterministic", s.key);
            assert!(!pos_eq(&build(id, h, 5, 0.3), &build(id, h, 6, 0.3)), "{} ignores its seed", s.key);
        }
    }

    #[test]
    fn solid_parts_face_outward() {
        for key in ["oak", "birch", "spruce", "pine", "willow", "cherry", "hawthorn"] {
            let id = flora::by_key(key).unwrap();
            assert_wound_outward(&build(id, flora::species(id).height.1, 3, 0.5), key);
        }
    }

    #[test]
    fn colours_stay_in_range_and_the_tint_changes_them() {
        for (i, s) in flora::SPECIES.iter().enumerate() {
            let id = SpeciesId(i as u8);
            let g = build(id, s.height.0, 1, 0.5);
            assert!(g.col.iter().flatten().all(|c| (0.0..=1.6).contains(c)), "{} colour out of range", s.key);
        }
        let poppy = flora::by_key("poppy").unwrap();
        assert_ne!(tint_colour(poppy, 0.0), tint_colour(poppy, 1.0));
    }

    #[test]
    fn the_flowers_have_the_colour_of_their_name() {
        let avg = |key: &str| {
            let id = flora::by_key(key).unwrap();
            let g = build(id, 0.5, 2, 0.5);
            let n = g.col.len() as f32;
            let mut s = [0.0f32; 3];
            // The colour of the vertices that are not green (stems and leaves are green; the flower is not).
            let mut k = 0.0;
            for c in &g.col {
                if !(c[1] > c[0] && c[1] > c[2]) {
                    for i in 0..3 {
                        s[i] += c[i];
                    }
                    k += 1.0;
                }
            }
            assert!(k / n > 0.1, "{key} has no flower");
            [s[0] / k, s[1] / k, s[2] / k]
        };
        let poppy = avg("poppy");
        assert!(poppy[0] > 2.0 * poppy[2] && poppy[0] > poppy[1], "poppy {poppy:?}");
        let corn = avg("cornflower");
        assert!(corn[2] > corn[0] && corn[2] > corn[1], "cornflower {corn:?}");
        let bell = avg("bluebell");
        assert!(bell[2] > bell[1], "bluebell {bell:?}");
        let butter = avg("buttercup");
        assert!(butter[0] > butter[2] * 3.0 && butter[1] > butter[2] * 3.0, "buttercup {butter:?}");
    }
}
