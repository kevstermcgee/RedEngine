//! Seeded gradient noise in two and three dimensions (improved Perlin) and the hashes the generator rolls its dice with. Pure functions of the
//! seed and the coordinates, so a chunk is the same every time it is made, on every machine.

/// A 32-bit integer hash (lowbias32).
pub fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// A hash of a seed and a cell.
pub fn hash3(seed: u32, x: i32, y: i32, z: i32) -> u32 {
    hash32(seed ^ hash32((x as u32).wrapping_mul(0x9e37_79b1) ^ hash32((y as u32).wrapping_mul(0x85eb_ca77) ^ hash32((z as u32).wrapping_mul(0xc2b2_ae3d)))))
}

/// A hash of a seed and a column, in 0..1.
pub fn rand2(seed: u32, x: i32, z: i32) -> f32 {
    (hash3(seed, x, 0x5bd1, z) >> 8) as f32 / (1u32 << 24) as f32
}

/// One noise field: its own shuffle of the gradient table.
#[derive(Clone)]
pub struct Noise {
    perm: [u8; 512],
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn grad3(h: u8, x: f64, y: f64, z: f64) -> f64 {
    let h = h & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

fn grad2(h: u8, x: f64, y: f64) -> f64 {
    match h & 7 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x,
        5 => -x,
        6 => y,
        _ => -y,
    }
}

impl Noise {
    pub fn new(seed: u32) -> Noise {
        let mut p: [u8; 256] = std::array::from_fn(|i| i as u8);
        let mut s = hash32(seed ^ 0xa511_e9b3) | 1;
        for i in (1..256).rev() {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let j = (s % (i as u32 + 1)) as usize;
            p.swap(i, j);
        }
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = p[i & 255];
        }
        Noise { perm }
    }

    /// Noise at a point, about -1..1 (0 on every whole coordinate).
    pub fn at2(&self, x: f64, y: f64) -> f64 {
        let (xf, yf) = (x.floor(), y.floor());
        let (xi, yi) = ((xf as i64 & 255) as usize, (yf as i64 & 255) as usize);
        let (x, y) = (x - xf, y - yf);
        let (u, v) = (fade(x), fade(y));
        let p = &self.perm;
        let (a, b) = (p[xi] as usize + yi, p[xi + 1] as usize + yi);
        let r = lerp(lerp(grad2(p[a], x, y), grad2(p[b], x - 1.0, y), u), lerp(grad2(p[a + 1], x, y - 1.0), grad2(p[b + 1], x - 1.0, y - 1.0), u), v);
        r * 0.7
    }

    /// Noise at a point, about -1..1.
    pub fn at3(&self, x: f64, y: f64, z: f64) -> f64 {
        let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
        let (xi, yi, zi) = ((xf as i64 & 255) as usize, (yf as i64 & 255) as usize, (zf as i64 & 255) as usize);
        let (x, y, z) = (x - xf, y - yf, z - zf);
        let (u, v, w) = (fade(x), fade(y), fade(z));
        let p = &self.perm;
        let a = p[xi] as usize + yi;
        let (aa, ab) = (p[a] as usize + zi, p[a + 1] as usize + zi);
        let b = p[xi + 1] as usize + yi;
        let (ba, bb) = (p[b] as usize + zi, p[b + 1] as usize + zi);
        lerp(
            lerp(lerp(grad3(p[aa], x, y, z), grad3(p[ba], x - 1.0, y, z), u), lerp(grad3(p[ab], x, y - 1.0, z), grad3(p[bb], x - 1.0, y - 1.0, z), u), v),
            lerp(
                lerp(grad3(p[aa + 1], x, y, z - 1.0), grad3(p[ba + 1], x - 1.0, y, z - 1.0), u),
                lerp(grad3(p[ab + 1], x, y - 1.0, z - 1.0), grad3(p[bb + 1], x - 1.0, y - 1.0, z - 1.0), u),
                v,
            ),
            w,
        )
    }

    /// Octaves of [`Noise::at2`] at `x / scale`, each twice as fine and half as strong, normalised to about -1..1.
    pub fn fbm2(&self, x: f64, y: f64, scale: f64, octaves: u32) -> f64 {
        let (mut sum, mut amp, mut norm, mut f) = (0.0, 1.0, 0.0, 1.0 / scale);
        for o in 0..octaves {
            // Each octave is shifted so the zeros of one do not sit on the zeros of the next.
            let off = o as f64 * 31.7;
            sum += amp * self.at2(x * f + off, y * f - off);
            norm += amp;
            amp *= 0.5;
            f *= 2.0;
        }
        sum / norm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_seeded_smooth_and_bounded() {
        let a = Noise::new(1);
        let b = Noise::new(2);
        let mut diff = 0.0;
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for i in 0..2000 {
            let (x, y, z) = (i as f64 * 0.37, i as f64 * 0.11 + 3.0, i as f64 * 0.23 - 7.0);
            let v = a.at3(x, y, z);
            lo = lo.min(v);
            hi = hi.max(v);
            diff += (v - b.at3(x, y, z)).abs();
            let w = a.at2(x, y);
            assert!(w.abs() <= 1.0, "{w}");
            assert!((a.at3(x, y, z) - a.at3(x + 0.001, y, z)).abs() < 0.02, "smooth");
        }
        assert!(lo > -1.2 && hi < 1.2 && hi - lo > 1.0, "{lo}..{hi}");
        assert!(diff > 100.0, "different seeds differ");
        assert_eq!(Noise::new(9).at2(1.5, 2.5), Noise::new(9).at2(1.5, 2.5));
    }

    #[test]
    fn the_dice_are_uniform_enough() {
        let n = 20000;
        let mean: f32 = (0..n).map(|i| rand2(5, i % 141, i / 141)).sum::<f32>() / n as f32;
        assert!((mean - 0.5).abs() < 0.01, "{mean}");
    }
}
