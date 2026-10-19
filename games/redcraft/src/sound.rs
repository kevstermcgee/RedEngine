//! The game's sounds, synthesized once at start (mono, the engine's sample rate): for each material a dig, a break, a place and footsteps
//! (four variants each, so repeats do not sound stamped), and the rest: a hurt, a pickup pop, eating, a splash, a fuse and an explosion, a
//! click for the menus.

use crate::block::Sound;
use std::collections::HashMap;

const RATE: f32 = red_engine2::audio::SAMPLE_RATE as f32;

/// White noise from a seed.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.0 = s;
        (s as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// A band-pass filter (the cookbook's, constant peak gain).
struct BandPass {
    b0: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BandPass {
    fn new(freq: f32, q: f32) -> BandPass {
        let w = std::f32::consts::TAU * freq / RATE;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        BandPass { b0: alpha / a0, b2: -alpha / a0, a1: -2.0 * w.cos() / a0, a2: (1.0 - alpha) / a0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Filtered noise in an envelope (attack, then an exponential fall over `decay` seconds), `grain` > 0 making it crackle.
fn burst(seed: u32, secs: f32, freq: f32, q: f32, decay: f32, grain: f32) -> Vec<f32> {
    let n = (secs * RATE) as usize;
    let mut noise = Noise(seed | 1);
    let mut bp = BandPass::new(freq, q);
    let mut bp2 = BandPass::new(freq * 1.7, q);
    let mut out = Vec::with_capacity(n);
    let mut crackle = 1.0;
    for i in 0..n {
        let t = i as f32 / RATE;
        let env = (t / 0.003).min(1.0) * (-t / decay).exp();
        if grain > 0.0 && i % 64 == 0 {
            crackle = if noise.next() > 1.0 - 2.0 * grain { 1.6 } else { 0.35 };
        }
        let x = noise.next();
        out.push((bp.run(x) + bp2.run(x) * 0.5) * env * if grain > 0.0 { crackle } else { 1.0 });
    }
    out
}

/// A sine that falls in pitch (a knock, a thump).
fn tone(secs: f32, from: f32, to: f32, decay: f32, offset: usize, out: &mut [f32], gain: f32) {
    let n = ((secs * RATE) as usize).min(out.len().saturating_sub(offset));
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / RATE;
        let f = from + (to - from) * (t / secs);
        phase += std::f32::consts::TAU * f / RATE;
        out[offset + i] += phase.sin() * (-t / decay).exp() * (t / 0.002).min(1.0) * gain;
    }
}

fn normalize(mut v: Vec<f32>, peak: f32) -> Vec<f32> {
    let m = v.iter().fold(0.0f32, |a, x| a.max(x.abs())).max(1e-6);
    for x in v.iter_mut() {
        *x *= peak / m;
    }
    v
}

/// What a material's hit sounds like: (centre frequency, sharpness, length of the ring, crackle, a knock's pitch).
fn voice(s: Sound) -> (f32, f32, f32, f32, f32) {
    match s {
        Sound::Stone => (2300.0, 1.4, 0.045, 0.0, 180.0),
        Sound::Wood => (650.0, 2.6, 0.06, 0.0, 240.0),
        Sound::Grass => (3800.0, 0.7, 0.07, 0.25, 0.0),
        Sound::Gravel => (1400.0, 0.8, 0.08, 0.45, 0.0),
        Sound::Sand => (3000.0, 0.6, 0.09, 0.15, 0.0),
        Sound::Glass => (3400.0, 2.0, 0.05, 0.0, 0.0),
        Sound::Snow => (1700.0, 0.9, 0.08, 0.3, 0.0),
        Sound::Plant => (4200.0, 0.8, 0.05, 0.3, 0.0),
    }
}

fn hit(s: Sound, seed: u32, loud: f32, len: f32) -> Vec<f32> {
    let (f, q, decay, grain, knock) = voice(s);
    let pitch = 0.9 + (seed % 5) as f32 * 0.05;
    let mut v = burst(seed.wrapping_mul(2654435761), len, f * pitch, q, decay * loud, grain);
    if knock > 0.0 {
        tone(len.min(0.12), knock * pitch * 1.3, knock * pitch, 0.03 * loud, 0, &mut v, 0.25);
    }
    v
}

fn glass_break(seed: u32) -> Vec<f32> {
    let mut v = burst(seed, 0.5, 3000.0, 1.0, 0.05, 0.4);
    let mut n = Noise(seed ^ 0x9e37);
    for k in 0..9 {
        let f = 2400.0 + (n.next() + 1.0) * 2600.0;
        let start = (k as f32 * 0.025 * RATE) as usize;
        tone(0.35, f, f * 0.98, 0.12, start, &mut v, 0.18);
    }
    v
}

/// Every clip.
pub struct Sounds {
    dig: HashMap<Sound, Vec<Vec<f32>>>,
    broke: HashMap<Sound, Vec<Vec<f32>>>,
    step: HashMap<Sound, Vec<Vec<f32>>>,
    pub hurt: Vec<f32>,
    pub pop: Vec<f32>,
    pub eat: Vec<f32>,
    pub splash: Vec<f32>,
    pub fuse: Vec<f32>,
    pub explode: Vec<f32>,
    pub click: Vec<f32>,
    pub swing: Vec<f32>,
    pub zombie: Vec<f32>,
    pub pig: Vec<f32>,
}

const ALL: [Sound; 8] = [Sound::Stone, Sound::Wood, Sound::Grass, Sound::Gravel, Sound::Sand, Sound::Glass, Sound::Snow, Sound::Plant];

impl Sounds {
    pub fn make() -> Sounds {
        let mut dig = HashMap::new();
        let mut broke = HashMap::new();
        let mut step = HashMap::new();
        for (k, s) in ALL.iter().enumerate() {
            let base = k as u32 * 97 + 11;
            dig.insert(*s, (0..4).map(|i| normalize(hit(*s, base + i, 1.0, 0.12), 0.55)).collect());
            broke.insert(
                *s,
                (0..4)
                    .map(|i| if *s == Sound::Glass { normalize(glass_break(base + i), 0.7) } else { normalize(hit(*s, base + 40 + i, 2.2, 0.28), 0.8) })
                    .collect(),
            );
            step.insert(*s, (0..4).map(|i| normalize(hit(*s, base + 80 + i, 0.8, 0.1), 0.3)).collect());
        }
        let mut hurt = burst(5, 0.25, 900.0, 1.2, 0.08, 0.0);
        tone(0.22, 420.0, 260.0, 0.08, 0, &mut hurt, 0.9);
        let mut pop = vec![0.0; (0.08 * RATE) as usize];
        tone(0.08, 700.0, 1400.0, 0.03, 0, &mut pop, 1.0);
        let eat = (0..3)
            .flat_map(|k| {
                let mut b = burst(70 + k, 0.11, 1600.0, 0.8, 0.04, 0.6);
                b.extend(std::iter::repeat_n(0.0, (0.05 * RATE) as usize));
                b
            })
            .collect();
        let splash = burst(9, 0.7, 1300.0, 0.5, 0.22, 0.35);
        let fuse = burst(13, 1.2, 5200.0, 0.5, 2.0, 0.6);
        let mut explode = burst(17, 1.6, 180.0, 0.5, 0.45, 0.3);
        tone(0.6, 70.0, 35.0, 0.25, 0, &mut explode, 1.2);
        let mut click = vec![0.0; (0.04 * RATE) as usize];
        tone(0.04, 1800.0, 1500.0, 0.01, 0, &mut click, 1.0);
        let swing = burst(21, 0.18, 2600.0, 0.5, 0.06, 0.0);
        let mut zombie = burst(23, 1.1, 260.0, 2.0, 0.6, 0.2);
        tone(1.0, 120.0, 95.0, 0.5, 0, &mut zombie, 0.8);
        let mut pig = vec![0.0; (0.35 * RATE) as usize];
        tone(0.12, 520.0, 380.0, 0.06, 0, &mut pig, 1.0);
        tone(0.14, 560.0, 400.0, 0.07, (0.16 * RATE) as usize, &mut pig, 1.0);
        let pig_noise = burst(29, 0.35, 700.0, 3.0, 0.1, 0.5);
        for (a, b) in pig.iter_mut().zip(pig_noise) {
            *a += b * 0.4;
        }
        Sounds {
            dig,
            broke,
            step,
            hurt: normalize(hurt, 0.7),
            pop: normalize(pop, 0.35),
            eat: normalize(eat, 0.5),
            splash: normalize(splash, 0.55),
            fuse: normalize(fuse, 0.4),
            explode: normalize(explode, 1.0),
            click: normalize(click, 0.3),
            swing: normalize(swing, 0.3),
            zombie: normalize(zombie, 0.6),
            pig: normalize(pig, 0.5),
        }
    }

    fn pick(set: &HashMap<Sound, Vec<Vec<f32>>>, s: Sound, n: u32) -> &[f32] {
        set.get(&s).map_or(&[][..], |v| v[(n % 4) as usize].as_slice())
    }

    /// A blow while mining.
    pub fn dig(&self, s: Sound, n: u32) -> &[f32] {
        Self::pick(&self.dig, s, n)
    }

    /// A block broken.
    pub fn broke(&self, s: Sound, n: u32) -> &[f32] {
        Self::pick(&self.broke, s, n)
    }

    /// A block placed (its dig, a little fuller).
    pub fn place(&self, s: Sound, n: u32) -> &[f32] {
        Self::pick(&self.dig, s, n + 2)
    }

    pub fn step(&self, s: Sound, n: u32) -> &[f32] {
        Self::pick(&self.step, s, n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_material_has_its_clips_and_they_are_short_and_bounded() {
        let s = Sounds::make();
        for m in ALL {
            for n in 0..4 {
                for clip in [s.dig(m, n), s.broke(m, n), s.step(m, n), s.place(m, n)] {
                    assert!(!clip.is_empty() && clip.len() < RATE as usize);
                    assert!(clip.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
                }
            }
        }
        assert!(s.explode.len() > s.pop.len());
        // Stone sounds brighter than wood: more sign changes per second.
        let zc = |v: &[f32]| v.windows(2).filter(|w| (w[0] > 0.0) != (w[1] > 0.0)).count() as f32 / v.len() as f32;
        assert!(zc(s.dig(Sound::Stone, 0)) > zc(s.dig(Sound::Wood, 0)));
    }
}
