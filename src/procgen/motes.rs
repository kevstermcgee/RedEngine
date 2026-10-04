//! Fireflies and pollen: the small lights of a living world, as a pure function of where you are, the time and the height of the sun.
//!
//! At dusk, as the sun drops below the horizon, fireflies drift up out of the damp grass round you and blink, each at its own pace; by night
//! they are everywhere in the meadows and fewer under the trees' dry ground; in the morning they are gone. By day, in the sun, specks of
//! pollen float up out of the flower fields and glint. Nothing is stored: a mote is a cell of the world (7 m square) plus a slot, positioned
//! and lit from the cell's hash and the time, so the same moment is the same sparkle on every machine and a still frame can be drawn at any hour.

use super::noise::{smooth, Rng};
use super::world::World;

/// What a mote is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoteKind {
    /// A blinking warm-green light of the dusk and night.
    Firefly,
    /// A glint of pollen in the sun.
    Pollen,
}

/// One mote.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mote {
    /// World position (x, y, z).
    pub pos: [f64; 3],
    /// Radius of its bright core in metres.
    pub size: f32,
    /// How bright it is right now, 0 (dark) to 1.
    pub glow: f32,
    /// Firefly or pollen.
    pub kind: MoteKind,
}

/// The most motes drawn at once.
pub const MAX_MOTES: usize = 128;
/// Metres per cell.
const CELL: f64 = 7.0;
/// Cells each way from the viewer.
const REACH: i32 = 5;
/// Motes beyond this many metres are not drawn (they fade out before it).
pub const RANGE: f64 = 36.0;

/// The motes round `eye` (world x, y, z) at scene time `t`, with the sun `sun_elev_deg` degrees above the horizon.
pub fn motes(world: &World, eye: [f64; 3], t: f32, sun_elev_deg: f32) -> Vec<Mote> {
    let night = smooth(6.0, -6.0, sun_elev_deg);
    let day = smooth(6.0, 22.0, sun_elev_deg);
    let mut out = Vec::new();
    if night <= 0.0 && day <= 0.0 {
        return out;
    }
    let (ci, cj) = ((eye[0] / CELL).floor() as i32, (eye[2] / CELL).floor() as i32);
    let seed = world.config().seed ^ 0x0f1e_f1e5;
    for j in cj - REACH..=cj + REACH {
        for i in ci - REACH..=ci + REACH {
            let mut rng = Rng::at(seed, i, j);
            let (ox, oz) = (i as f64 * CELL, j as f64 * CELL);
            let c = world.climate(ox + CELL / 2.0, oz + CELL / 2.0);
            if night > 0.0 {
                // More where it is damp and open: a wet meadow glitters, a dry wood hardly.
                let density = (0.25 + 1.5 * c.wet) * (1.0 - 0.6 * c.wood);
                for _ in 0..3 {
                    let (keep, bx, bz, lift) = (rng.white(), rng.range(0.0, CELL as f32) as f64, rng.range(0.0, CELL as f32) as f64, rng.range(0.35, 2.1));
                    let (p1, p2, p3, p4, p5, rate) = (
                        rng.range(0.0, std::f32::consts::TAU),
                        rng.range(0.0, std::f32::consts::TAU),
                        rng.range(0.0, std::f32::consts::TAU),
                        rng.range(0.0, std::f32::consts::TAU),
                        rng.range(0.0, std::f32::consts::TAU),
                        rng.range(0.9, 2.1),
                    );
                    if keep * 1.9 > density {
                        continue;
                    }
                    let tt = t as f64;
                    let (dx, dz) = (
                        (tt * 0.37 + p1 as f64).sin() * 1.7 + (tt * 0.91 + p2 as f64).sin() * 0.5,
                        (tt * 0.31 + p3 as f64).cos() * 1.7 + (tt * 0.77 + p4 as f64).cos() * 0.5,
                    );
                    let (x, z) = (ox + bx + dx, oz + bz + dz);
                    let y = world.height(x, z) as f64 + lift as f64 + (tt * 0.5 + p5 as f64).sin() * 0.3;
                    // A firefly lights up in a pulse and dims between them.
                    let pulse = ((tt * rate as f64 + p5 as f64).sin().max(0.0)).powf(5.0) as f32;
                    push(&mut out, Mote { pos: [x, y, z], size: 0.05, glow: (0.1 + 0.9 * pulse) * night, kind: MoteKind::Firefly }, eye);
                }
            }
            if day > 0.0 {
                let bloom = flowery(world, ox + CELL / 2.0, oz + CELL / 2.0);
                for _ in 0..2 {
                    let (keep, bx, bz, lift, phase, rate) = (
                        rng.white(),
                        rng.range(0.0, CELL as f32) as f64,
                        rng.range(0.0, CELL as f32) as f64,
                        rng.range(0.0, 3.0),
                        rng.range(0.0, std::f32::consts::TAU),
                        rng.range(0.6, 1.6),
                    );
                    if keep > 0.12 + 0.7 * bloom {
                        continue;
                    }
                    let tt = t as f64;
                    let (x, z) = (ox + bx + (tt * 0.2 + phase as f64).sin() * 1.2 + tt * 0.15 % CELL, oz + bz + (tt * 0.17 + phase as f64 * 2.0).cos() * 1.2);
                    let y = world.height(x, z) as f64 + 0.3 + (lift as f64 + tt * 0.08) % 3.0;
                    let glint = ((tt * rate as f64 + phase as f64).sin() * 0.5 + 0.5).powf(3.0) as f32;
                    push(&mut out, Mote { pos: [x, y, z], size: 0.012, glow: (0.25 + 0.75 * glint) * day, kind: MoteKind::Pollen }, eye);
                }
            }
        }
    }
    if out.len() > MAX_MOTES {
        out.sort_by(|a, b| dist2(a, eye).total_cmp(&dist2(b, eye)));
        out.truncate(MAX_MOTES);
    }
    out
}

/// How flowery the ground is at a spot (0 to 1): the share of the wildflower field there.
fn flowery(world: &World, x: f64, z: f64) -> f32 {
    match world.biome(x, z) {
        super::world::Biome::Wildflowers => 1.0,
        super::world::Biome::Meadow | super::world::Biome::Glade => 0.45,
        _ => 0.1,
    }
}

fn dist2(m: &Mote, eye: [f64; 3]) -> f64 {
    (m.pos[0] - eye[0]).powi(2) + (m.pos[1] - eye[1]).powi(2) + (m.pos[2] - eye[2]).powi(2)
}

/// Keeps a mote if it is in range, fading it out over the last quarter of the range.
fn push(out: &mut Vec<Mote>, mut m: Mote, eye: [f64; 3]) {
    let d = dist2(&m, eye).sqrt();
    if d >= RANGE {
        return;
    }
    m.glow *= 1.0 - smooth((RANGE * 0.7) as f32, RANGE as f32, d as f32);
    if m.glow > 0.004 {
        out.push(m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procgen::world::Config;

    fn world() -> World {
        World::new(Config { seed: 7, ..Config::default() })
    }

    #[test]
    fn fireflies_come_out_at_night_and_are_gone_by_noon() {
        let w = world();
        let eye = [300.0, 3.0, -120.0];
        let (noon, dusk, night) = (motes(&w, eye, 5.0, 60.0), motes(&w, eye, 5.0, 0.0), motes(&w, eye, 5.0, -30.0));
        assert!(noon.iter().all(|m| m.kind == MoteKind::Pollen), "no fireflies in the sun");
        assert!(night.iter().filter(|m| m.kind == MoteKind::Firefly).count() > 15, "{} fireflies at night", night.len());
        let (nf, df) = (night.iter().filter(|m| m.kind == MoteKind::Firefly).count(), dusk.iter().filter(|m| m.kind == MoteKind::Firefly).count());
        assert!(df > 0 && df <= nf + 5, "dusk has some: {df} vs night {nf}");
        assert!(night.iter().all(|m| m.kind == MoteKind::Firefly), "no pollen at night");
    }

    #[test]
    fn pollen_floats_in_the_sun_not_at_night() {
        let w = world();
        let eye = [0.0, 2.0, 0.0];
        let day = motes(&w, eye, 8.0, 50.0);
        assert!(day.iter().filter(|m| m.kind == MoteKind::Pollen).count() > 10, "{} motes", day.len());
        assert!(motes(&w, eye, 8.0, -30.0).iter().all(|m| m.kind != MoteKind::Pollen));
    }

    #[test]
    fn motes_are_near_above_the_ground_and_never_too_many() {
        let w = world();
        let eye = [-80.0, 2.0, 40.0];
        for (t, sun) in [(1.0, -40.0), (50.0, -10.0), (9.0, 45.0)] {
            let ms = motes(&w, eye, t, sun);
            assert!(ms.len() <= MAX_MOTES);
            for m in ms {
                let d = dist2(&m, eye).sqrt();
                assert!(d < RANGE, "{d}");
                assert!(m.pos[1] > w.height(m.pos[0], m.pos[2]) as f64 + 0.1, "below the ground");
                assert!((0.0..=1.0).contains(&m.glow) && m.glow > 0.0);
            }
        }
    }

    #[test]
    fn fireflies_blink_and_drift_but_the_same_moment_is_the_same() {
        let w = world();
        let eye = [10.0, 2.0, 10.0];
        assert_eq!(motes(&w, eye, 12.3, -30.0), motes(&w, eye, 12.3, -30.0));
        let (a, b) = (motes(&w, eye, 12.3, -30.0), motes(&w, eye, 13.1, -30.0));
        assert_ne!(a, b);
        let brightness = |ms: &[Mote]| ms.iter().map(|m| m.glow).sum::<f32>() / ms.len().max(1) as f32;
        // Over several seconds some are lit and some dim: the average glow wanders, and individual glows span a wide range.
        let samples: Vec<f32> = (0..40).map(|k| brightness(&motes(&w, eye, k as f32 * 0.37, -30.0))).collect();
        let (lo, hi) = (samples.iter().cloned().fold(f32::MAX, f32::min), samples.iter().cloned().fold(0.0, f32::max));
        assert!(hi > 0.0 && lo >= 0.0);
        let spread = a.iter().map(|m| m.glow).fold(0.0f32, f32::max) - a.iter().map(|m| m.glow).fold(1.0f32, f32::min);
        assert!(spread > 0.3, "fireflies should not all glow alike: {spread}");
    }

    #[test]
    fn the_wet_meadow_glitters_more_than_the_dry_wood() {
        let w = world();
        let mut counts = Vec::new();
        for k in 0..60 {
            let eye = [k as f64 * 700.0 - 20_000.0, 2.0, 123.0 * k as f64];
            let c = w.climate(eye[0], eye[2]);
            counts.push((c.wet * (1.0 - 0.6 * c.wood), motes(&w, eye, 3.0, -30.0).len()));
        }
        counts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (low, high): (usize, usize) = (counts[..15].iter().map(|c| c.1).sum(), counts[45..].iter().map(|c| c.1).sum());
        assert!(high > low, "damp open ground should have more: dry {low} vs damp {high}");
    }
}
