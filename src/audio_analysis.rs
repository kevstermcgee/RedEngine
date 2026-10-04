//! Measuring sound without ears: loudness, peaks, clipping, silence, the loop seam, the spectrum and the pitch, plus a picture of the wave and
//! its spectrogram. A sound cannot be looked at, but all of this can be read as numbers (one line per clip) or seen as one PNG, so an AI author
//! (or CI) can tell a thin click from a boom, a quiet cue from a loud one, a loop with a seam from a clean one.
//!
//! Pure and deterministic (no device, no GPU): the same samples give the same [`Report`]. Loudness follows ITU-R BS.1770 / EBU R128 (K-weighting,
//! 400 ms blocks, absolute and relative gates), so the numbers mean what a mastering engineer means by LUFS.

use serde_json::{json, Value};
use std::f32::consts::PI;

/// Samples at or beyond this magnitude count as clipped.
const CLIP_LEVEL: f32 = 0.999;
/// Below this (dBFS) a sample region counts as silent.
const SILENCE_DBFS: f32 = -60.0;
/// FFT size of the spectral analysis.
const FFT_N: usize = 4096;

/// Interleaved samples to analyse.
#[derive(Debug, Clone, Copy)]
pub struct Clip<'a> {
    /// `f32` samples, `channels` per frame.
    pub samples: &'a [f32],
    /// 1 (mono) or 2 (stereo).
    pub channels: usize,
    /// Samples per second.
    pub rate: u32,
}

impl Clip<'_> {
    /// Frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }

    fn channel(&self, c: usize) -> Vec<f32> {
        self.samples.iter().skip(c).step_by(self.channels.max(1)).copied().collect()
    }

    /// Mono mixdown (the mean of the channels).
    fn mono(&self) -> Vec<f32> {
        if self.channels <= 1 {
            return self.samples.to_vec();
        }
        self.samples.chunks(self.channels).map(|f| f.iter().sum::<f32>() / self.channels as f32).collect()
    }
}

/// What the numbers say about one clip.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Length in seconds.
    pub secs: f32,
    /// Channels analysed.
    pub channels: usize,
    /// Samples that are NaN or infinite (must be 0).
    pub non_finite: usize,
    /// Highest sample magnitude, dBFS.
    pub peak_dbfs: f32,
    /// Root-mean-square level, dBFS.
    pub rms_dbfs: f32,
    /// Peak minus RMS, dB (how punchy against how loud).
    pub crest_db: f32,
    /// Integrated loudness, LUFS (BS.1770; whole-clip when shorter than 400 ms; a mono clip is counted as dual mono, as the engine plays it).
    pub lufs: f32,
    /// Samples at full scale.
    pub clipped: usize,
    /// Mean sample value (a DC offset wastes headroom and clicks on start/stop).
    pub dc: f32,
    /// Silence before the first sound and after the last, milliseconds.
    pub lead_ms: f32,
    /// See `lead_ms`.
    pub tail_ms: f32,
    /// Largest magnitude in the last 8 samples, dBFS: near -100 when the clip fades to zero, high when it is cut off (a click).
    pub end_dbfs: f32,
    /// Loop seam: the jump from the last frame to the first, as a multiple of the clip's typical frame-to-frame step (about 1 is seamless).
    pub seam: f32,
    /// Left/right correlation, -1..1 (stereo only; 1 is mono-compatible, negative cancels in mono).
    pub correlation: Option<f32>,
    /// Spectral centroid, Hz (the brightness).
    pub centroid_hz: f32,
    /// Share of the energy in sub (<60 Hz), bass (60-250), low-mid (250-2k), high-mid (2k-6k) and air (>6k), percent.
    pub bands: [f32; 5],
    /// The strongest frequency, Hz (0 when the clip is silent).
    pub dominant_hz: f32,
    /// Spectral flatness 0..1 (0 a pure tone, near 1 white noise).
    pub flatness: f32,
}

/// Names of [`Report::bands`].
pub const BAND_NAMES: [&str; 5] = ["sub", "bass", "lowmid", "highmid", "air"];

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// A second-order filter section (direct form I).
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn run(&mut self, input: f32) -> f64 {
        let x0 = input as f64;
        let y0 = self.b[0] * x0 + self.b[1] * self.x[0] + self.b[2] * self.x[1] - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        self.x = [x0, self.x[0]];
        self.y = [y0, self.y[0]];
        y0
    }
}

/// The two K-weighting stages of BS.1770 (a high shelf and a high-pass), designed for any sample rate.
fn k_weighting(rate: u32) -> [Biquad; 2] {
    let fs = rate as f64;
    let (f0, g, q) = (1681.974450955533f64, 3.999843853973347f64, 0.7071752369554196f64);
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [(vh + vb * k / q + k * k) / a0, 2.0 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        x: [0.0; 2],
        y: [0.0; 2],
    };
    let (f0, q) = (38.13547087602444f64, 0.5003270373238773f64);
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let high = Biquad { b: [1.0, -2.0, 1.0], a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0], x: [0.0; 2], y: [0.0; 2] };
    [shelf, high]
}

/// Integrated loudness in LUFS (gated per BS.1770-4 when the clip is at least 400 ms, else the whole clip). `-120` for silence.
pub fn lufs(clip: Clip) -> f32 {
    let frames = clip.frames();
    if frames == 0 {
        return -120.0;
    }
    // Mono is dual mono: the engine sends a mono clip to both speakers.
    let chans: Vec<Vec<f32>> =
        if clip.channels == 1 { vec![clip.channel(0), clip.channel(0)] } else { (0..2.min(clip.channels)).map(|c| clip.channel(c)).collect() };
    let weighted: Vec<Vec<f64>> = chans
        .iter()
        .map(|ch| {
            let mut stages = k_weighting(clip.rate);
            ch.iter().map(|s| stages.iter_mut().fold(*s, |v, st| st.run(v) as f32) as f64).collect()
        })
        .collect();
    let block = (0.4 * clip.rate as f64) as usize;
    let hop = (0.1 * clip.rate as f64) as usize;
    let energy = |from: usize, to: usize| -> f64 { weighted.iter().map(|w| w[from..to].iter().map(|v| v * v).sum::<f64>() / (to - from) as f64).sum() };
    let loud = |z: f64| -0.691 + 10.0 * z.max(1e-12).log10();
    if frames < block + hop {
        return loud(energy(0, frames)) as f32;
    }
    let blocks: Vec<f64> = (0..=(frames - block) / hop).map(|i| energy(i * hop, i * hop + block)).collect();
    let above: Vec<f64> = blocks.iter().copied().filter(|z| loud(*z) > -70.0).collect();
    if above.is_empty() {
        return -120.0;
    }
    let relative = loud(above.iter().sum::<f64>() / above.len() as f64) - 10.0;
    let kept: Vec<f64> = above.into_iter().filter(|z| loud(*z) > relative).collect();
    if kept.is_empty() {
        return -120.0;
    }
    loud(kept.iter().sum::<f64>() / kept.len() as f64) as f32
}

/// In-place iterative radix-2 FFT of `re`/`im` (length a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
}

/// The average power spectrum of `mono` (Hann windows of [`FFT_N`], half overlapped; a short clip is zero-padded): `FFT_N/2` bins.
pub fn power_spectrum(mono: &[f32]) -> Vec<f32> {
    let hann: Vec<f32> = (0..FFT_N).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / FFT_N as f32).cos()).collect();
    let mut acc = vec![0.0f32; FFT_N / 2];
    let mut windows = 0;
    let mut start = 0;
    loop {
        let mut re = vec![0.0f32; FFT_N];
        let mut im = vec![0.0f32; FFT_N];
        for i in 0..FFT_N {
            re[i] = mono.get(start + i).copied().unwrap_or(0.0) * hann[i];
        }
        fft(&mut re, &mut im);
        for k in 0..FFT_N / 2 {
            acc[k] += re[k] * re[k] + im[k] * im[k];
        }
        windows += 1;
        start += FFT_N / 2;
        if start + FFT_N / 2 >= mono.len() {
            break;
        }
    }
    acc.iter_mut().for_each(|v| *v /= windows as f32);
    acc
}

/// A spectrogram of `mono`: `rows` log-spaced frequencies (row 0 the highest, 40 Hz to just under Nyquist) by `cols` evenly spaced moments, in dB
/// (an amplitude-1 sine is about 0, silence about -120). Windows are 2048 samples, so a column is about 46 ms wide at 44.1 kHz.
pub fn spectrogram(mono: &[f32], rate: u32, cols: usize, rows: usize) -> Vec<Vec<f32>> {
    const N: usize = 2048;
    let hann: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / N as f32).cos()).collect();
    let nyquist = rate as f32 / 2.0 * 0.98;
    let bin_of = |hz: f32| ((hz * N as f32 / rate as f32) as usize).clamp(1, N / 2 - 1);
    let mut out = vec![vec![-120.0f32; cols]; rows];
    for c in 0..cols {
        let start = if cols > 1 { (mono.len().saturating_sub(N)) * c / (cols - 1) } else { 0 };
        let mut re = vec![0.0f32; N];
        let mut im = vec![0.0f32; N];
        for i in 0..N {
            re[i] = mono.get(start + i).copied().unwrap_or(0.0) * hann[i];
        }
        fft(&mut re, &mut im);
        for r in 0..rows {
            let t0 = r as f32 / rows as f32;
            let t1 = (r + 1) as f32 / rows as f32;
            // Row 0 is the top: the highest frequency.
            let f = |t: f32| 40.0 * (nyquist / 40.0).powf(1.0 - t);
            let (hi, lo) = (f(t0), f(t1));
            let (b0, b1) = (bin_of(lo), bin_of(hi).max(bin_of(lo)));
            let peak = (b0..=b1).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt()).fold(0.0f32, f32::max);
            out[r][c] = 20.0 * (peak / (N as f32 / 4.0)).max(1e-6).log10();
        }
    }
    out
}

/// Analyses a clip.
pub fn analyze(clip: Clip) -> Report {
    let s = clip.samples;
    let rate = clip.rate as f32;
    let frames = clip.frames();
    let non_finite = s.iter().filter(|v| !v.is_finite()).count();
    let clean: Vec<f32> = s.iter().map(|v| if v.is_finite() { *v } else { 0.0 }).collect();
    let clip = Clip { samples: &clean, ..clip };
    let peak = clean.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let rms = (clean.iter().map(|v| v * v).sum::<f32>() / clean.len().max(1) as f32).sqrt();
    let mono = clip.mono();
    let dc = mono.iter().sum::<f32>() / mono.len().max(1) as f32;
    let silent = 10f32.powf(SILENCE_DBFS / 20.0);
    let first = mono.iter().position(|v| v.abs() > silent);
    let last = mono.iter().rposition(|v| v.abs() > silent);
    let (lead, tail) = match (first, last) {
        (Some(f), Some(l)) => (f as f32 / rate * 1000.0, (mono.len() - 1 - l) as f32 / rate * 1000.0),
        _ => (mono.len() as f32 / rate * 1000.0, mono.len() as f32 / rate * 1000.0),
    };
    let end_peak = mono[mono.len().saturating_sub(8)..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    // Seam: how big is the step from the last frame back to the first, against the typical step inside the clip?
    let seam = if frames >= 3 {
        let step = |a: usize, b: usize| (a..a + clip.channels).zip(b..b + clip.channels).map(|(i, j)| (clean[i] - clean[j]).abs()).fold(0.0f32, f32::max);
        let ch = clip.channels;
        let typical = (1..frames).map(|f| step((f - 1) * ch, f * ch)).sum::<f32>() / (frames - 1) as f32;
        step((frames - 1) * ch, 0) / typical.max(1e-6)
    } else {
        0.0
    };
    let correlation = (clip.channels == 2).then(|| {
        let (l, r) = (clip.channel(0), clip.channel(1));
        let (mut ll, mut rr, mut lr) = (0.0f64, 0.0f64, 0.0f64);
        for (a, b) in l.iter().zip(&r) {
            ll += (*a as f64) * (*a as f64);
            rr += (*b as f64) * (*b as f64);
            lr += (*a as f64) * (*b as f64);
        }
        if ll * rr < 1e-18 {
            1.0
        } else {
            (lr / (ll * rr).sqrt()) as f32
        }
    });
    let ps = power_spectrum(&mono);
    let hz = |k: usize| k as f32 * rate / FFT_N as f32;
    let total: f32 = ps.iter().skip(1).sum::<f32>().max(1e-20);
    let centroid = ps.iter().enumerate().skip(1).map(|(k, p)| hz(k) * p).sum::<f32>() / total;
    let mut bands = [0.0f32; 5];
    for (k, p) in ps.iter().enumerate().skip(1) {
        let f = hz(k);
        let b = if f < 60.0 {
            0
        } else if f < 250.0 {
            1
        } else if f < 2000.0 {
            2
        } else if f < 6000.0 {
            3
        } else {
            4
        };
        bands[b] += p / total * 100.0;
    }
    let (lo, hi) = ((50.0 / (rate / FFT_N as f32)) as usize, ((10000.0 / (rate / FFT_N as f32)) as usize).min(ps.len() - 2));
    let dominant = if peak < 1e-5 {
        0.0
    } else {
        let k = (lo..=hi).max_by(|a, b| ps[*a].total_cmp(&ps[*b])).unwrap_or(lo);
        // Parabolic interpolation of the peak in dB for a sub-bin estimate.
        let (a, b, c) = (ps[k - 1].max(1e-20).ln(), ps[k].max(1e-20).ln(), ps[k + 1].max(1e-20).ln());
        let denom = a - 2.0 * b + c;
        hz(k) + if denom.abs() > 1e-9 { 0.5 * (a - c) / denom * rate / FFT_N as f32 } else { 0.0 }
    };
    let band: Vec<f32> = ps[lo..=hi].iter().map(|p| p.max(1e-20)).collect();
    let flatness = ((band.iter().map(|p| p.ln()).sum::<f32>() / band.len() as f32).exp() / (band.iter().sum::<f32>() / band.len() as f32)).clamp(0.0, 1.0);
    Report {
        secs: frames as f32 / rate,
        channels: clip.channels,
        non_finite,
        peak_dbfs: db(peak),
        rms_dbfs: db(rms),
        crest_db: db(peak) - db(rms),
        lufs: lufs(clip),
        clipped: clean.iter().filter(|v| v.abs() >= CLIP_LEVEL).count(),
        dc,
        lead_ms: lead,
        tail_ms: tail,
        end_dbfs: db(end_peak),
        seam,
        correlation,
        centroid_hz: centroid,
        bands,
        dominant_hz: dominant,
        flatness,
    }
}

/// `A4`, `C#3` ... for a frequency, with the cents off (`+12`), or `-` for none.
pub fn note_name(hz: f32) -> String {
    if hz < 20.0 {
        return "-".to_string();
    }
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let n = midi.round();
    let cents = ((midi - n) * 100.0).round() as i32;
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}{:+}c", NAMES[(n as i32).rem_euclid(12) as usize], (n as i32).div_euclid(12) - 1, cents)
}

impl Report {
    /// One compact line: what an author reads to judge a sound.
    pub fn line(&self) -> String {
        format!(
            "{:.2}s {} peak {:.1} rms {:.1} lufs {:.1} crest {:.1} | clip {} dc {:.3} lead {:.0}ms tail {:.0}ms end {:.0}dB seam {:.1}{} | bright {:.0}Hz {} flat {:.2} | {}",
            self.secs,
            if self.channels == 2 { "st" } else { "mono" },
            self.peak_dbfs,
            self.rms_dbfs,
            self.lufs,
            self.crest_db,
            self.clipped,
            self.dc,
            self.lead_ms,
            self.tail_ms,
            self.end_dbfs,
            self.seam,
            self.correlation.map(|c| format!(" corr {c:.2}")).unwrap_or_default(),
            self.centroid_hz,
            note_name(self.dominant_hz),
            self.flatness,
            BAND_NAMES.iter().zip(self.bands).map(|(n, b)| format!("{n} {b:.0}%")).collect::<Vec<_>>().join(" "),
        )
    }

    /// The same numbers as JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "secs": r2(self.secs), "channels": self.channels, "non_finite": self.non_finite,
            "peak_dbfs": r2(self.peak_dbfs), "rms_dbfs": r2(self.rms_dbfs), "crest_db": r2(self.crest_db), "lufs": r2(self.lufs),
            "clipped": self.clipped, "dc": r2(self.dc * 1000.0) / 1000.0, "lead_ms": r2(self.lead_ms), "tail_ms": r2(self.tail_ms),
            "end_dbfs": r2(self.end_dbfs), "seam": r2(self.seam), "correlation": self.correlation.map(r2),
            "centroid_hz": r2(self.centroid_hz), "dominant_hz": r2(self.dominant_hz), "note": note_name(self.dominant_hz), "flatness": r2(self.flatness),
            "bands_pct": BAND_NAMES.iter().zip(self.bands).map(|(n, b)| (n.to_string(), json!(r2(b)))).collect::<serde_json::Map<_, _>>(),
        })
    }
}

fn r2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

/// What kind of clip a standard applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A short effect that plays once.
    OneShot,
    /// A clip that repeats (music, ambience): its ends must meet.
    Loop,
}

/// A standard the engine's sounds are held to: the problems a [`Report`] has for a clip of this kind, as sentences (empty = fine).
pub fn problems(r: &Report, kind: Kind) -> Vec<String> {
    let mut out = Vec::new();
    if r.non_finite > 0 {
        out.push(format!("{} sample(s) are NaN or infinite", r.non_finite));
    }
    if r.clipped > 0 {
        out.push(format!("{} sample(s) at full scale (peak {:.1} dBFS): it will clip on the speakers once mixed", r.clipped, r.peak_dbfs));
    }
    if r.peak_dbfs < -40.0 {
        out.push(format!("peak is {:.1} dBFS: effectively silent", r.peak_dbfs));
    }
    // A short effect built on a low thump has a little DC by nature; a loop's offset repeats forever.
    let dc_limit = if kind == Kind::Loop { 0.01 } else { 0.03 };
    if r.dc.abs() > dc_limit {
        out.push(format!("DC offset {:.3}: wastes headroom and clicks when it starts or stops", r.dc));
    }
    match kind {
        Kind::OneShot => {
            // Cut off = the clip stops while still sounding (its last samples are audible) AND jumps to silence (the end-to-start step dwarfs the usual
            // step). A loud tone the engine fades over its last 2 ms ends audible but does not jump; a faint tail that stops inaudibly is not a click.
            if r.end_dbfs > -50.0 && r.seam > 6.0 {
                out.push(format!(
                    "it is cut off instead of fading to zero: its last samples are at {:.0} dBFS and the jump to silence is {:.0}x the usual step (a click)",
                    r.end_dbfs, r.seam
                ));
            }
            if r.secs > 6.0 {
                out.push(format!("{:.1} s is long for an effect (over 6 s)", r.secs));
            }
        }
        Kind::Loop => {
            if r.seam > 6.0 {
                out.push(format!("loop seam {:.1}x the usual step: the end does not meet the start (a click every cycle)", r.seam));
            }
            if r.lufs < -40.0 {
                out.push(format!("{:.1} LUFS: inaudible", r.lufs));
            }
        }
    }
    out
}

/// Things worth a look that do not fail a sound: its loudness against the median of its group (`group` = the other sounds of the same kind).
pub fn warnings(r: &Report, group_median_lufs: Option<f32>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(m) = group_median_lufs {
        if r.lufs > -100.0 && (r.lufs - m).abs() > 10.0 {
            out.push(format!(
                "{:.1} LUFS is {:.0} LU {} the median of its group ({m:.1}): it will {} the others in a mix",
                r.lufs,
                (r.lufs - m).abs(),
                if r.lufs > m { "above" } else { "below" },
                if r.lufs > m { "drown" } else { "vanish under" }
            ));
        }
    }
    out
}

/// Reads a 16-bit PCM or 32-bit float WAV file into `(interleaved samples, channels, rate)`.
pub fn read_wav(bytes: &[u8]) -> Result<(Vec<f32>, usize, u32), String> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".to_string());
    }
    let (mut channels, mut rate, mut bits, mut format, mut data) = (0usize, 0u32, 0u16, 0u16, None);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            format = u16::from_le_bytes([body[0], body[1]]);
            channels = u16::from_le_bytes([body[2], body[3]]) as usize;
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            data = Some(body);
        }
        at += 8 + size + (size & 1);
    }
    let data = data.ok_or("no data chunk")?;
    if channels == 0 || channels > 2 {
        return Err(format!("{channels} channels: only mono and stereo are supported"));
    }
    let samples: Vec<f32> = match (format, bits) {
        (1, 16) => data.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / i16::MAX as f32).collect(),
        (3, 32) => data.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect(),
        _ => return Err(format!("format {format} at {bits} bits: only 16-bit PCM and 32-bit float are supported")),
    };
    Ok((samples, channels, rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, secs: f32, amp: f32, rate: u32, channels: usize) -> Vec<f32> {
        (0..(secs * rate as f32) as usize).flat_map(|i| std::iter::repeat_n(amp * (2.0 * PI * hz * i as f32 / rate as f32).sin(), channels)).collect()
    }

    fn rep(samples: &[f32], channels: usize) -> Report {
        analyze(Clip { samples, channels, rate: 44100 })
    }

    #[test]
    fn loudness_matches_the_ebu_reference_a_minus_23_dbfs_1khz_stereo_sine_is_minus_23_lufs() {
        let s = sine(1000.0, 5.0, 10f32.powf(-23.0 / 20.0), 44100, 2);
        let l = lufs(Clip { samples: &s, channels: 2, rate: 44100 });
        assert!((l + 23.0).abs() < 0.2, "{l}");
        // Mono is counted as dual mono, so the same sine in one channel file reads the same.
        let m = sine(1000.0, 5.0, 10f32.powf(-23.0 / 20.0), 44100, 1);
        assert!((lufs(Clip { samples: &m, channels: 1, rate: 44100 }) - l).abs() < 0.05);
        // Twice the amplitude is 6.02 LU louder; another rate gives the same number.
        let loud = sine(1000.0, 5.0, 2.0 * 10f32.powf(-23.0 / 20.0), 44100, 2);
        assert!((lufs(Clip { samples: &loud, channels: 2, rate: 44100 }) - l - 6.02).abs() < 0.05);
        let s48 = sine(1000.0, 5.0, 10f32.powf(-23.0 / 20.0), 48000, 2);
        assert!((lufs(Clip { samples: &s48, channels: 2, rate: 48000 }) + 23.0).abs() < 0.2);
    }

    #[test]
    fn gating_ignores_silence_so_a_quiet_gap_does_not_lower_the_loudness() {
        let mut s = sine(1000.0, 3.0, 0.1, 44100, 2);
        let alone = lufs(Clip { samples: &s, channels: 2, rate: 44100 });
        s.extend(std::iter::repeat_n(0.0, 44100 * 2 * 6));
        let with_gap = lufs(Clip { samples: &s, channels: 2, rate: 44100 });
        assert!((alone - with_gap).abs() < 0.3, "{alone} vs {with_gap}");
        assert!(lufs(Clip { samples: &[0.0; 100], channels: 1, rate: 44100 }) <= -100.0);
    }

    #[test]
    fn a_sine_reads_as_its_pitch_with_a_narrow_flat_spectrum_and_the_right_levels() {
        let s = sine(440.0, 2.0, 0.5, 44100, 1);
        let r = rep(&s, 1);
        assert!((r.dominant_hz - 440.0).abs() < 3.0 && note_name(r.dominant_hz).starts_with("A4"), "{} {}", r.dominant_hz, note_name(r.dominant_hz));
        assert!((r.peak_dbfs + 6.02).abs() < 0.1 && (r.rms_dbfs + 9.03).abs() < 0.1 && (r.crest_db - 3.01).abs() < 0.1, "{r:?}");
        assert!(r.flatness < 0.05, "a pure tone is not flat: {}", r.flatness);
        assert!(r.bands[2] > 95.0, "440 Hz is low-mid: {:?}", r.bands);
        assert!((r.centroid_hz - 440.0).abs() < 40.0, "{}", r.centroid_hz);
        assert_eq!((r.clipped, r.non_finite), (0, 0));
        assert!(r.dc.abs() < 0.01);
    }

    #[test]
    fn noise_is_flat_and_bright_and_a_low_sine_lives_in_the_bass() {
        let mut x = 0x1234_5678u32;
        let noise: Vec<f32> = (0..44100 * 2)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect();
        let r = rep(&noise, 1);
        assert!(r.flatness > 0.5 && r.centroid_hz > 8000.0, "{r:?}");
        let low = rep(&sine(110.0, 2.0, 0.5, 44100, 1), 1);
        assert!(low.bands[1] > 90.0 && low.centroid_hz < 200.0, "{low:?}");
    }

    #[test]
    fn clipping_dc_silence_and_the_seam_are_each_measured() {
        let mut clipped = sine(300.0, 1.0, 1.4, 44100, 1);
        clipped.iter_mut().for_each(|v| *v = v.clamp(-1.0, 1.0));
        assert!(rep(&clipped, 1).clipped > 1000);
        let offset: Vec<f32> = sine(300.0, 1.0, 0.2, 44100, 1).iter().map(|v| v + 0.3).collect();
        assert!((rep(&offset, 1).dc - 0.3).abs() < 0.01);
        let mut padded = vec![0.0f32; 4410];
        padded.extend(sine(300.0, 0.5, 0.5, 44100, 1));
        padded.extend(vec![0.0f32; 8820]);
        let r = rep(&padded, 1);
        assert!((r.lead_ms - 100.0).abs() < 2.0 && (r.tail_ms - 200.0).abs() < 4.0, "{} {}", r.lead_ms, r.tail_ms);
        assert!(r.end_dbfs < -100.0, "{}", r.end_dbfs);
        // A whole number of cycles loops cleanly; a cut mid-cycle jumps.
        let clean = sine(441.0, 1.0, 0.5, 44100, 1);
        assert!(rep(&clean, 1).seam < 2.0, "{}", rep(&clean, 1).seam);
        let cut: Vec<f32> = sine(441.0, 1.0, 0.5, 44100, 1).into_iter().take(44100 - 25).collect();
        assert!(rep(&cut, 1).seam > 6.0, "{}", rep(&cut, 1).seam);
    }

    #[test]
    fn stereo_correlation_tells_mono_from_out_of_phase() {
        let m = sine(300.0, 1.0, 0.5, 44100, 2);
        assert!(rep(&m, 2).correlation.unwrap() > 0.99);
        let flipped: Vec<f32> = m.chunks(2).flat_map(|f| [f[0], -f[1]]).collect();
        assert!(rep(&flipped, 2).correlation.unwrap() < -0.99);
        assert!(rep(&m, 1).correlation.is_none());
    }

    #[test]
    fn the_standard_flags_each_defect_in_words_and_passes_a_clean_effect() {
        let mut fade = sine(500.0, 0.4, 0.5, 44100, 1);
        let n = fade.len();
        fade.iter_mut().enumerate().for_each(|(i, v)| *v *= (1.0 - i as f32 / n as f32).powi(3));
        assert!(problems(&rep(&fade, 1), Kind::OneShot).is_empty(), "{:?}", problems(&rep(&fade, 1), Kind::OneShot));
        // 0.4 s of 500 Hz is a whole number of cycles (it happens to end on a zero crossing); stopping a quarter cycle early ends it mid-wave.
        let cut: Vec<f32> = sine(500.0, 0.4, 0.5, 44100, 1).into_iter().take(44100 * 4 / 10 - 22).collect();
        assert!(problems(&rep(&cut, 1), Kind::OneShot).iter().any(|p| p.contains("cut off")), "{:?}", rep(&cut, 1));
        let tone_with_engine_fade = crate::dsp::finish(sine(500.0, 0.4, 0.5, 44100, 1), 0.5);
        assert!(problems(&rep(&tone_with_engine_fade, 1), Kind::OneShot).is_empty(), "a loud tone ended by the 2 ms fade is not a click");
        let mut hot = sine(500.0, 0.2, 1.5, 44100, 1);
        hot.iter_mut().for_each(|v| *v = v.clamp(-1.0, 1.0));
        assert!(problems(&rep(&hot, 1), Kind::OneShot).iter().any(|p| p.contains("full scale")));
        assert!(problems(&rep(&sine(441.0, 1.0, 0.5, 44100, 1), 1), Kind::Loop).is_empty());
        let jump: Vec<f32> = sine(441.0, 1.0, 0.5, 44100, 1).into_iter().take(44100 - 25).collect();
        assert!(problems(&rep(&jump, 1), Kind::Loop).iter().any(|p| p.contains("seam")));
    }

    #[test]
    fn a_wav_written_by_the_engine_reads_back_and_a_bad_file_says_why() {
        let s = sine(300.0, 0.2, 0.5, 44100, 2);
        let bytes = crate::synth::wav_bytes_i16(&s, 44100, 2);
        let (back, ch, rate) = read_wav(&bytes).unwrap();
        assert_eq!((ch, rate, back.len()), (2, 44100, s.len()));
        assert!(back.iter().zip(&s).all(|(a, b)| (a - b).abs() < 1e-3));
        assert!(read_wav(b"nope").unwrap_err().contains("RIFF"));
    }

    #[test]
    fn note_names_and_the_line_and_json_are_stable() {
        assert_eq!(note_name(440.0), "A4+0c");
        assert_eq!(note_name(261.63), "C4+0c");
        assert_eq!(note_name(5.0), "-");
        let r = rep(&sine(440.0, 1.0, 0.5, 44100, 1), 1);
        assert!(r.line().contains("lufs") && r.line().contains("A4"), "{}", r.line());
        assert_eq!(r.to_json()["note"], "A4+0c");
        assert!(r.to_json()["bands_pct"]["lowmid"].as_f64().unwrap() > 95.0);
    }
}
