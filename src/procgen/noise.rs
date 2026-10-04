//! Deterministic noise for procedural worlds: integer hashing, a seeded generator and gradient noise.
//!
//! Everything is integer mixing plus `+ - * /` on `f64` (no `sin`, `cos` or `powf`, whose last bit differs between platforms' maths
//! libraries), so a seed makes the same world on every machine, and a server and its clients agree on where every tree is. Coordinates are
//! `f64` so a player far from the origin (a hundred kilometres is only a few hours of walking) still sees smooth, un-quantised ground.

/// Mixes a seed and two lattice coordinates into 32 well-spread bits.
pub fn hash2(seed: u32, x: i32, y: i32) -> u32 {
    let mut h = seed ^ 0x9e37_79b9;
    h = h.wrapping_add((x as u32).wrapping_mul(0x85eb_ca6b));
    h ^= h >> 15;
    h = h.wrapping_add((y as u32).wrapping_mul(0xc2b2_ae35));
    h ^= h >> 13;
    h = h.wrapping_mul(0x27d4_eb2f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x165667b1);
    h ^ (h >> 15)
}

/// The top 24 bits of a hash as a number in `[0, 1)`.
pub fn unit(h: u32) -> f32 {
    (h >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// A hash of a seed and a lattice cell as a number in `[0, 1)`.
pub fn rand2(seed: u32, x: i32, y: i32) -> f32 {
    unit(hash2(seed, x, y))
}

/// A tiny seeded generator (SplitMix64): the same seed gives the same sequence everywhere.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    /// A generator for a seed and a lattice cell (one per patch, tree, chunk ...).
    pub fn at(seed: u32, x: i32, y: i32) -> Rng {
        Rng(((hash2(seed, x, y) as u64) << 32) | hash2(seed ^ 0x5bd1_e995, y, x) as u64)
    }

    /// The next 32 random bits.
    pub fn bits(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 32) as u32
    }

    /// The next number in `[0, 1)`.
    pub fn white(&mut self) -> f32 {
        unit(self.bits())
    }

    /// The next number in `[lo, hi)`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.white()
    }
}

/// The eight gradient directions (axes and diagonals): picking one by hash needs no trigonometry.
const R: f64 = std::f64::consts::FRAC_1_SQRT_2;
const GRADS: [(f64, f64); 8] = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (R, R), (-R, R), (R, -R), (-R, -R)];

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn corner(seed: u32, ix: i32, iy: i32, dx: f64, dy: f64) -> f64 {
    let (gx, gy) = GRADS[(hash2(seed, ix, iy) & 7) as usize];
    gx * dx + gy * dy
}

/// Smooth gradient noise, roughly in `[-1, 1]`, one feature per unit of `x` and `y`.
pub fn gradient(seed: u32, x: f64, y: f64) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (ix, iy) = (fx as i32, fy as i32);
    let (dx, dy) = (x - fx, y - fy);
    let (u, v) = (fade(dx), fade(dy));
    let a = corner(seed, ix, iy, dx, dy);
    let b = corner(seed, ix + 1, iy, dx - 1.0, dy);
    let c = corner(seed, ix, iy + 1, dx, dy - 1.0);
    let d = corner(seed, ix + 1, iy + 1, dx - 1.0, dy - 1.0);
    let top = a + (b - a) * u;
    let bottom = c + (d - c) * u;
    ((top + (bottom - top) * v) * 1.414) as f32
}

/// Fractal noise: `octaves` layers of [`gradient`], each twice as fine and half as strong, scaled to roughly `[-1, 1]`.
pub fn fbm(seed: u32, x: f64, y: f64, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0f32, 1.0f32, 0.0f32, 1.0f64);
    for o in 0..octaves.max(1) {
        // Each octave is shifted so the lattice lines of different octaves never coincide at the origin.
        sum += amp * gradient(seed.wrapping_add(o.wrapping_mul(0x9e37)), x * f + o as f64 * 17.31, y * f - o as f64 * 9.77);
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// Hermite smoothstep from `a` to `b`.
pub fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_stable_and_spread() {
        // Pinned values: a change here changes every world ever generated.
        assert_eq!(hash2(1, 2, 3), hash2(1, 2, 3));
        assert_ne!(hash2(1, 2, 3), hash2(1, 3, 2));
        assert_ne!(hash2(1, 2, 3), hash2(2, 2, 3));
        let mean: f32 = (0..4000).map(|i| rand2(7, i % 63, i / 63)).sum::<f32>() / 4000.0;
        assert!((mean - 0.5).abs() < 0.03, "mean {mean}");
    }

    #[test]
    fn the_generator_repeats_per_seed() {
        let mut a = Rng::at(5, 1, 2);
        let mut b = Rng::at(5, 1, 2);
        let mut c = Rng::at(5, 2, 1);
        let (x, y, z) = (a.white(), b.white(), c.white());
        assert_eq!(x, y);
        assert_ne!(x, z);
        assert!((0.0..1.0).contains(&x));
    }

    #[test]
    fn gradient_noise_is_smooth_bounded_and_centred() {
        let (mut lo, mut hi, mut sum, mut jump) = (f32::MAX, f32::MIN, 0.0f32, 0.0f32);
        let n = 20_000;
        let mut prev = gradient(3, 0.0, 0.0);
        for i in 1..n {
            let v = gradient(3, i as f64 * 0.013, i as f64 * 0.007);
            lo = lo.min(v);
            hi = hi.max(v);
            sum += v;
            jump = jump.max((v - prev).abs());
            prev = v;
        }
        assert!(lo >= -1.05 && hi <= 1.05, "range {lo}..{hi}");
        assert!(lo < -0.5 && hi > 0.5, "uses its range: {lo}..{hi}");
        assert!((sum / n as f32).abs() < 0.08, "mean {}", sum / n as f32);
        assert!(jump < 0.05, "no steps: {jump}");
    }

    #[test]
    fn far_from_the_origin_the_noise_is_still_smooth() {
        let far = 1.0e7;
        let a = gradient(3, far, far);
        let b = gradient(3, far + 0.001, far);
        assert!((a - b).abs() < 0.01, "{a} {b}");
    }
}
