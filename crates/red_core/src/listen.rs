//! Listening to the player's own music: a scene's `audio.playlist` plays audio files the player chooses, and what is heard drives a game's sky
//! (`sky.shader`) through [`Heard`]: 32 spectrum bands, the level, bass/mid/treble, beats, and a seed for the song and for this play of it.
//!
//! Everything here is pure and headless: the [`Analyzer`] turns mono samples at a playhead into a [`Heard`] (the same numbers whether the live client
//! is playing the song or `frame --song` is drawing a moment of it), and [`Playlist`] is the queue (next, previous, wrap, the title's ten seconds).
//! Decoding files is [`decode`] (with `gfx`); the client in `client/listen.rs` only does the sound card, the file picker and the keys.
//!
//! The game ships no audio (ADR 0008 is about what games ship): the files are the player's, read from their disk at run time and never copied.

use crate::audio_analysis::fft;
use serde_json::{Map, Value};
use std::f32::consts::PI;

/// Bands of the spectrum a sky sees, log spaced from [`LOW_HZ`] to [`HIGH_HZ`].
pub const BANDS: usize = 32;
/// The lowest band's lower edge.
pub const LOW_HZ: f32 = 30.0;
/// The highest band's upper edge.
pub const HIGH_HZ: f32 = 11_000.0;
/// The rate the analysis runs at: songs are mixed to mono and resampled to this once, when they are decoded.
pub const RATE: u32 = 22_050;
/// Samples per analysis window (about 93 ms at [`RATE`]).
const WINDOW: usize = 2048;

/// What a sky hears at one moment (all 0 to 1 unless said otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Heard {
    /// Loudness of each band relative to the loudest moments so far, eased (rises fast, falls in about a quarter second).
    pub bands: [f32; BANDS],
    /// Overall level.
    pub level: f32,
    /// Bands 0-5 (to ~120 Hz), 6-17 (to ~1.2 kHz), 18-31.
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    /// 1 at a beat, falling to 0 in about half a second.
    pub beat: f32,
    /// Beats so far in this song (a shader can change something every N beats).
    pub beats: f32,
    /// Seconds into the song.
    pub song_time: f32,
    /// 0 with no music, 1 while a song plays (eased in and out).
    pub playing: f32,
    /// The same for every play of a song (from its name and length), below 2^24.
    pub song: u32,
    /// New each time a song starts, below 2^24: the same song looks different every time.
    pub play: u32,
    /// Seconds of listening, which stop while the music is paused: with a playlist a game sky's `sky_time()` is this, so Space freezes the picture
    /// and the music together.
    pub clock: f32,
    /// Whether a playlist is driving the sky (then `sky_time()` is [`Heard::clock`]).
    pub listening: bool,
}

impl Heard {
    /// Packed as the shader reads it (`globals.audio`, 12 vec4): bands in 0..8, then (level, bass, mid, treble), (beat, beats, song_time, playing),
    /// (song, play, clock, listening), and one spare.
    pub fn uniform(&self) -> [[f32; 4]; 12] {
        let mut u = [[0.0f32; 4]; 12];
        for (i, b) in self.bands.iter().enumerate() {
            u[i / 4][i % 4] = *b;
        }
        u[8] = [self.level, self.bass, self.mid, self.treble];
        u[9] = [self.beat, self.beats, self.song_time, self.playing];
        u[10] = [self.song as f32, self.play as f32, self.clock, if self.listening { 1.0 } else { 0.0 }];
        u
    }
}

/// The band edges in Hz (`BANDS + 1` of them).
fn edges() -> [f32; BANDS + 1] {
    let mut e = [0.0f32; BANDS + 1];
    for (i, v) in e.iter_mut().enumerate() {
        *v = LOW_HZ * (HIGH_HZ / LOW_HZ).powf(i as f32 / BANDS as f32);
    }
    e
}

/// Turns music into [`Heard`], one call per frame. Stateful (easing, the loudest-so-far reference, the beat detector): feed it moments in order.
#[derive(Debug, Clone)]
pub struct Analyzer {
    hann: Vec<f32>,
    /// FFT bin range of each band.
    ranges: Vec<(usize, usize)>,
    eased: [f32; BANDS],
    level: f32,
    /// The loudest band (dB) heard recently, decaying slowly: everything is measured against it, so quiet songs fill the sky too.
    reference_db: f32,
    /// The low-frequency energy of the last frame, and a running mean and variance of its rise (spectral flux), for onsets.
    last_low: f32,
    flux_mean: f32,
    flux_var: f32,
    since_beat: f32,
    beat: f32,
    beats: u32,
}

impl Default for Analyzer {
    fn default() -> Self {
        Analyzer::new()
    }
}

impl Analyzer {
    /// A fresh analyzer (a new song starts with one).
    pub fn new() -> Analyzer {
        let hann: Vec<f32> = (0..WINDOW).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / WINDOW as f32).cos()).collect();
        let hz_per_bin = RATE as f32 / WINDOW as f32;
        let e = edges();
        let ranges = (0..BANDS)
            .map(|b| {
                let lo = (e[b] / hz_per_bin).floor() as usize;
                let hi = ((e[b + 1] / hz_per_bin).ceil() as usize).max(lo + 1);
                (lo.max(1), hi.min(WINDOW / 2))
            })
            .collect();
        Analyzer {
            hann,
            ranges,
            eased: [0.0; BANDS],
            level: 0.0,
            reference_db: -30.0,
            last_low: 0.0,
            flux_mean: 0.0,
            flux_var: 0.0,
            since_beat: 10.0,
            beat: 0.0,
            beats: 0,
        }
    }

    /// What is heard at `at_secs` into `mono` (samples at [`RATE`]), `dt` seconds after the previous call.
    pub fn hear(&mut self, mono: &[f32], at_secs: f32, dt: f32) -> Heard {
        let dt = dt.clamp(1e-4, 0.25);
        let centre = (at_secs.max(0.0) * RATE as f32) as usize;
        let start = centre.saturating_sub(WINDOW / 2);
        let mut re = vec![0.0f32; WINDOW];
        let mut im = vec![0.0f32; WINDOW];
        let mut sq = 0.0f32;
        for i in 0..WINDOW {
            let s = mono.get(start + i).copied().unwrap_or(0.0);
            sq += s * s;
            re[i] = s * self.hann[i];
        }
        fft(&mut re, &mut im);
        let power = |k: usize| re[k] * re[k] + im[k] * im[k];
        // Each band's energy in dB (mean power over its bins; a full-scale sine is about +48).
        let mut db = [0.0f32; BANDS];
        for (b, (lo, hi)) in self.ranges.iter().enumerate() {
            let p: f32 = (*lo..*hi).map(power).sum::<f32>() / (hi - lo) as f32;
            db[b] = 10.0 * (p + 1e-9).log10();
        }
        let loudest = db.iter().copied().fold(f32::MIN, f32::max);
        // The reference follows a louder moment at once and sinks 3 dB a second, never below where silence would sit.
        self.reference_db = if loudest > self.reference_db { loudest } else { (self.reference_db - 3.0 * dt).max(-10.0) };
        let (up, down) = (1.0 - (-dt / 0.03).exp(), 1.0 - (-dt / 0.25).exp());
        for b in 0..BANDS {
            // 42 dB below the reference is nothing.
            let v = ((db[b] - self.reference_db) / 42.0 + 1.0).clamp(0.0, 1.0);
            let k = if v > self.eased[b] { up } else { down };
            self.eased[b] += (v - self.eased[b]) * k;
        }
        let rms_db = 10.0 * (sq / WINDOW as f32 + 1e-12).log10();
        let level = ((rms_db + 42.0) / 36.0).clamp(0.0, 1.0);
        self.level += (level - self.level) * if level > self.level { up } else { down };
        // Onsets: the rise of low-frequency energy against its own recent statistics (about a second of memory), at most four beats a second.
        let low_power: f32 = self.ranges[..12].iter().map(|(lo, hi)| (*lo..*hi).map(power).sum::<f32>()).sum();
        let total_power: f32 = (1..WINDOW / 2).map(power).sum::<f32>() + 1e-9;
        let low = low_power.sqrt();
        let flux = (low - self.last_low).max(0.0);
        self.last_low = low;
        let a = 1.0 - (-dt / 1.0).exp();
        let threshold = self.flux_mean + 1.5 * self.flux_var.sqrt() + 1e-4;
        self.since_beat += dt;
        // A real rise, not the window sliding over a steady sound: at least a tenth of the energy, and above the recent spread.
        // (And the low end must carry a real share of the sound: a high note's leakage into it is not a kick.)
        if flux > threshold && flux > 0.1 * low && low_power > 0.02 * total_power && self.since_beat > 0.25 && low > 1e-3 {
            self.beat = 1.0;
            self.beats += 1;
            self.since_beat = 0.0;
        } else {
            self.beat *= (-dt / 0.18).exp();
        }
        let d = flux - self.flux_mean;
        self.flux_mean += a * d;
        self.flux_var = (1.0 - a) * (self.flux_var + a * d * d);
        let avg = |r: std::ops::Range<usize>| r.clone().map(|b| self.eased[b]).sum::<f32>() / r.len() as f32;
        Heard {
            bands: self.eased,
            level: self.level,
            bass: avg(0..6),
            mid: avg(6..18),
            treble: avg(18..BANDS),
            beat: self.beat,
            beats: self.beats as f32,
            song_time: at_secs,
            playing: 1.0,
            song: 0,
            play: 0,
            clock: at_secs,
            listening: true,
        }
    }

    /// What is heard at `at_secs`, as a fresh analyzer that has listened for the `warm` seconds before it would (offline frames: the easing,
    /// the reference and the beat count then look like a moment of a real listen, and the same arguments always give the same answer).
    pub fn moment(mono: &[f32], at_secs: f32, warm: f32) -> Heard {
        let mut a = Analyzer::new();
        let dt = 1.0 / 60.0;
        let mut t = (at_secs - warm).max(0.0);
        let mut heard = a.hear(mono, t, dt);
        while t + dt <= at_secs {
            t += dt;
            heard = a.hear(mono, t, dt);
        }
        heard
    }
}

/// A seed below 2^24 for a song: the same file name and length always give the same one.
pub fn song_seed(name: &str, samples: usize) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes().chain(samples.to_le_bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    ((h ^ (h >> 29)) as u32) & 0x00ff_ffff
}

/// The title to show for a file: its tags' "artist - title" when it has them, else its file name without the extension, tidied
/// (`03_my-song.mp3` -> `03 my song`).
pub fn title_of(path: &std::path::Path, artist: Option<&str>, title: Option<&str>) -> String {
    match (artist.map(str::trim).filter(|s| !s.is_empty()), title.map(str::trim).filter(|s| !s.is_empty())) {
        (Some(a), Some(t)) => format!("{a} - {t}"),
        (None, Some(t)) => t.to_string(),
        _ => path.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ").split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default(),
    }
}

/// `text` with accented Latin letters folded to plain ones (`Sigur Rós` -> `Sigur Ros`), so the engine's ASCII bitmap font shows them instead of gaps.
pub fn ascii_fold(text: &str) -> String {
    const FOLD: &[(&str, char)] = &[
        ("ÀÁÂÃÄÅĀĂĄàáâãäåāăą", 'a'),
        ("ÇĆČçćč", 'c'),
        ("ĎĐďđ", 'd'),
        ("ÈÉÊËĒĖĘĚèéêëēėęě", 'e'),
        ("ĞğĢģ", 'g'),
        ("ÌÍÎÏĪĮİìíîïīįı", 'i'),
        ("ĹĽŁĺľł", 'l'),
        ("ÑŃŇñńň", 'n'),
        ("ÒÓÔÕÖØŌŐòóôõöøōő", 'o'),
        ("ŔŘŕř", 'r'),
        ("ŚŞŠßśşš", 's'),
        ("ŢŤţť", 't'),
        ("ÙÚÛÜŪŮŰŲùúûüūůűų", 'u'),
        ("ÝŸýÿ", 'y'),
        ("ŹŻŽźżž", 'z'),
        ("’‘`", '\''),
        ("“”", '"'),
        ("–—", '-'),
    ];
    text.chars()
        .map(|c| match FOLD.iter().find(|(from, _)| from.contains(c)) {
            Some((_, to)) if c.is_uppercase() => to.to_ascii_uppercase(),
            Some((_, to)) => *to,
            None => c,
        })
        .collect()
}

/// File extensions the picker offers and [`decode`] reads.
pub const EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "oga", "wav", "m4a", "aac", "mp4"];

/// A scene's `audio.playlist`: the game plays the player's own music.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaylistSpec {
    /// Seconds the song's title shows when it starts (0: never).
    pub title_secs: f32,
    /// Where the title shows.
    pub corner: Corner,
    /// Play the songs in a shuffled order.
    pub shuffle: bool,
}

/// A corner of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// `audio.playlist` keys.
pub const PLAYLIST_KEYS: &[&str] = &["title_secs", "corner", "shuffle"];

/// Parses `audio.playlist` (`o` is the `audio` block).
pub fn parse_playlist(o: &Map<String, Value>, errs: &mut Vec<String>) -> Option<PlaylistSpec> {
    let p = match o.get("playlist")? {
        Value::Bool(true) => Map::new(),
        Value::Object(p) => p.clone(),
        _ => {
            errs.push("audio.playlist: true, or an object like {\"title_secs\": 10, \"corner\": \"bottom_left\"}".to_string());
            return None;
        }
    };
    crate::strict::check_keys(errs, "audio.playlist", &p, PLAYLIST_KEYS);
    let title_secs = match p.get("title_secs") {
        None => 10.0,
        Some(v) => match v.as_f64().filter(|x| (0.0..=600.0).contains(x)) {
            Some(x) => x as f32,
            None => {
                errs.push("audio.playlist.title_secs: seconds the title shows, 0 to 600".to_string());
                10.0
            }
        },
    };
    let corner = match p.get("corner").and_then(Value::as_str) {
        None if p.get("corner").is_none() => Corner::BottomLeft,
        Some("top_left") => Corner::TopLeft,
        Some("top_right") => Corner::TopRight,
        Some("bottom_left") => Corner::BottomLeft,
        Some("bottom_right") => Corner::BottomRight,
        _ => {
            errs.push("audio.playlist.corner: top_left, top_right, bottom_left or bottom_right".to_string());
            Corner::BottomLeft
        }
    };
    let shuffle = match p.get("shuffle") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            errs.push("audio.playlist.shuffle: true or false".to_string());
            false
        }
    };
    Some(PlaylistSpec { title_secs, corner, shuffle })
}

/// The queue: which file plays, what comes next, and how long the title has shown. No sound card, no files: the client asks it what to do.
#[derive(Debug, Clone, Default)]
pub struct Playlist {
    files: Vec<std::path::PathBuf>,
    /// Play order (indices into `files`).
    order: Vec<usize>,
    at: usize,
    shuffle: bool,
    rng: u64,
}

impl Playlist {
    /// An empty queue; `seed` orders a shuffle.
    pub fn new(shuffle: bool, seed: u64) -> Playlist {
        Playlist { shuffle, rng: seed | 1, ..Default::default() }
    }

    fn next_rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn reorder(&mut self) {
        self.order = (0..self.files.len()).collect();
        if self.shuffle {
            for i in (1..self.order.len()).rev() {
                let j = (self.next_rand() % (i as u64 + 1)) as usize;
                self.order.swap(i, j);
            }
        }
    }

    /// Replaces the queue with `files` (those with a known extension, in the given order) and starts at the first; returns how many it kept.
    pub fn replace(&mut self, files: Vec<std::path::PathBuf>) -> usize {
        self.files = files
            .into_iter()
            .filter(|f| f.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())))
            .collect();
        self.reorder();
        self.at = 0;
        self.files.len()
    }

    /// Adds `files` after the current one (dropped onto the window) and makes the first of them current.
    pub fn add(&mut self, files: Vec<std::path::PathBuf>) -> usize {
        if self.files.is_empty() {
            return self.replace(files);
        }
        let before = self.files.len();
        let kept: Vec<_> = files
            .into_iter()
            .filter(|f| f.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())))
            .collect();
        let n = kept.len();
        self.files.extend(kept);
        let at = (self.at + 1).min(self.order.len());
        self.order.splice(at..at, before..before + n);
        if n > 0 {
            self.at = at;
        }
        n
    }

    /// The file to play now.
    pub fn current(&self) -> Option<&std::path::Path> {
        self.order.get(self.at).map(|i| self.files[*i].as_path())
    }

    /// Moves to the next song (round to the first after the last; a shuffled queue is shuffled again).
    pub fn next(&mut self) {
        if self.order.is_empty() {
            return;
        }
        self.at += 1;
        if self.at >= self.order.len() {
            self.at = 0;
            if self.shuffle && self.order.len() > 2 {
                let last = self.order[self.order.len() - 1];
                self.reorder();
                if self.order[0] == last {
                    self.order.swap(0, 1);
                }
            }
        }
    }

    /// Moves to the previous song (round to the last from the first).
    pub fn previous(&mut self) {
        if !self.order.is_empty() {
            self.at = (self.at + self.order.len() - 1) % self.order.len();
        }
    }

    /// How many songs are queued.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// How visible a song's title is `t` seconds after it started, when it shows for `secs`: fades in over half a second and out over the last second.
pub fn title_alpha(t: f32, secs: f32) -> f32 {
    if secs <= 0.0 || t < 0.0 || t > secs {
        return 0.0;
    }
    (t / 0.5).min(1.0).min((secs - t) / 1.0).clamp(0.0, 1.0)
}

/// A decoded song: interleaved samples to play, mono samples at [`RATE`] to listen to, and its title.
#[cfg(feature = "decode")]
pub struct Song {
    /// Interleaved samples at `rate`, `channels` wide.
    pub samples: Vec<f32>,
    pub channels: u16,
    pub rate: u32,
    /// Mono at [`RATE`], for the [`Analyzer`].
    pub mono: Vec<f32>,
    pub title: String,
    /// [`song_seed`].
    pub seed: u32,
}

#[cfg(feature = "decode")]
impl Song {
    /// Length in seconds.
    pub fn secs(&self) -> f32 {
        self.samples.len() as f32 / self.channels.max(1) as f32 / self.rate.max(1) as f32
    }
}

/// Decodes an audio file (mp3, flac, ogg vorbis, wav, m4a/aac) with symphonia. Errors say which file and why, in words a player can act on.
#[cfg(feature = "decode")]
pub fn decode(path: &std::path::Path) -> Result<Song, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::{MetadataOptions, StandardTagKey};
    use symphonia::core::probe::Hint;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let file = std::fs::File::open(path).map_err(|e| format!("{name}: cannot be opened ({e})"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("{name}: not a music file this can read ({e}); mp3, flac, ogg, wav and m4a work"))?;
    let (mut artist, mut title) = (None, None);
    let mut read_tags = |rev: &symphonia::core::meta::MetadataRevision| {
        for tag in rev.tags() {
            match tag.std_key {
                Some(StandardTagKey::Artist) => artist = Some(tag.value.to_string()),
                Some(StandardTagKey::TrackTitle) => title = Some(tag.value.to_string()),
                _ => {}
            }
        }
    };
    if let Some(rev) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
        read_tags(rev);
    }
    let mut format = probed.format;
    if let Some(rev) = format.metadata().current() {
        read_tags(rev);
    }
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or_else(|| format!("{name}: has no audio track"))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("{name}: its audio cannot be decoded ({e})"))?;
    let mut samples: Vec<f32> = Vec::new();
    let (mut channels, mut rate) = (0u16, 0u32);
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(format!("{name}: stopped reading ({e})")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                channels = spec.channels.count() as u16;
                rate = spec.rate;
                let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
                sb.copy_interleaved_ref(buf);
                samples.extend_from_slice(sb.samples());
            }
            // A damaged frame is skipped, as players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("{name}: stopped decoding ({e})")),
        }
        // Two hours is plenty for one song (and bounds memory: ~2.5 GB of stereo f32 would be next).
        if samples.len() > 2 * 48_000 * 60 * 120 {
            break;
        }
    }
    if samples.is_empty() || channels == 0 || rate == 0 {
        return Err(format!("{name}: has no sound in it"));
    }
    let mono = to_analysis_rate(&samples, channels as usize, rate);
    let title = title_of(path, artist.as_deref(), title.as_deref());
    let seed = song_seed(&name, samples.len());
    Ok(Song { samples, channels, rate, mono, title, seed })
}

/// Mixes interleaved samples to mono and resamples them to [`RATE`] (linear: for analysis, not for listening).
pub fn to_analysis_rate(samples: &[f32], channels: usize, rate: u32) -> Vec<f32> {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    let mono: Vec<f32> = (0..frames).map(|f| samples[f * channels..f * channels + channels].iter().sum::<f32>() / channels as f32).collect();
    if rate == RATE {
        return mono;
    }
    let out_len = (frames as f64 * RATE as f64 / rate as f64) as usize;
    let step = rate as f64 / RATE as f64;
    (0..out_len)
        .map(|i| {
            let x = i as f64 * step;
            let j = x as usize;
            let f = (x - j as f64) as f32;
            let a = mono.get(j).copied().unwrap_or(0.0);
            let b = mono.get(j + 1).copied().unwrap_or(a);
            a + (b - a) * f
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, secs: f32, amp: f32) -> Vec<f32> {
        (0..(secs * RATE as f32) as usize).map(|i| amp * (2.0 * PI * hz * i as f32 / RATE as f32).sin()).collect()
    }

    #[test]
    fn a_low_tone_lights_the_bass_and_a_high_one_the_treble() {
        let low = Analyzer::moment(&tone(80.0, 3.0, 0.5), 2.0, 1.5);
        let high = Analyzer::moment(&tone(5000.0, 3.0, 0.5), 2.0, 1.5);
        // A pure tone lights its own band and its neighbours: the bass average (6 bands) is well above the rest.
        assert!(low.bass > 0.3 && low.bass > 3.0 * low.mid && low.treble < 0.05, "{low:?}");
        assert!(high.treble > 0.05 && high.bass == 0.0 && high.mid == 0.0, "{high:?}");
        assert!(low.beats <= 1.0 && high.beats <= 1.0, "a steady tone is not a beat: {} {}", low.beats, high.beats);
        let peak = |h: &Heard| h.bands.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        let e = edges();
        let p = peak(&low);
        assert!(e[p] <= 80.0 * 1.2 && e[p + 1] >= 80.0 / 1.2, "the 80 Hz tone peaks in its own band: band {p} is {}..{} Hz", e[p], e[p + 1]);
        let silence = Analyzer::moment(&vec![0.0; RATE as usize * 3], 2.0, 1.5);
        assert!(silence.level < 0.05 && silence.bands.iter().all(|b| *b < 0.05), "{silence:?}");
    }

    #[test]
    fn a_steady_kick_is_counted_as_beats_at_its_tempo() {
        // A kick (a 60 Hz thump) every half second over a quiet pad: 120 beats a minute.
        let secs = 12.0;
        let mut s = tone(330.0, secs, 0.05);
        for k in 0..(secs * 2.0) as usize {
            let at = k * RATE as usize / 2;
            for i in 0..(RATE as usize / 8) {
                if let Some(v) = s.get_mut(at + i) {
                    *v += 0.8 * (-(i as f32) / 900.0).exp() * (2.0 * PI * 60.0 * i as f32 / RATE as f32).sin();
                }
            }
        }
        let mut a = Analyzer::new();
        let (mut t, dt) = (0.0, 1.0 / 60.0);
        let mut heard = Heard::default();
        while t < secs {
            heard = a.hear(&s, t, dt);
            t += dt;
        }
        assert!((20.0..=25.0).contains(&heard.beats), "about 24 kicks heard as beats: {}", heard.beats);
    }

    #[test]
    fn the_same_moment_is_always_the_same_and_the_uniform_is_laid_out_as_the_shader_reads_it() {
        let s = tone(220.0, 4.0, 0.4);
        assert_eq!(Analyzer::moment(&s, 3.0, 2.0), Analyzer::moment(&s, 3.0, 2.0));
        let mut h = Analyzer::moment(&s, 3.0, 2.0);
        h.song = 7;
        h.play = 9;
        let u = h.uniform();
        assert_eq!((u[0][0], u[7][3], u[8][0], u[9][2], u[10][0], u[10][1]), (h.bands[0], h.bands[31], h.level, h.song_time, 7.0, 9.0));
    }

    #[test]
    fn a_playlist_keeps_only_music_wraps_and_adds_after_the_current_song() {
        let p = |n: &str| std::path::PathBuf::from(n);
        let mut list = Playlist::new(false, 1);
        assert_eq!(list.replace(vec![p("a.mp3"), p("notes.txt"), p("b.FLAC"), p("c.ogg")]), 3);
        assert_eq!(list.current(), Some(std::path::Path::new("a.mp3")));
        list.previous();
        assert_eq!(list.current(), Some(std::path::Path::new("c.ogg")), "previous from the first is the last");
        list.next();
        list.next();
        assert_eq!(list.current(), Some(std::path::Path::new("b.FLAC")));
        assert_eq!(list.add(vec![p("d.wav")]), 1);
        assert_eq!(list.current(), Some(std::path::Path::new("d.wav")), "a dropped song plays next, now");
        list.next();
        assert_eq!(list.current(), Some(std::path::Path::new("c.ogg")));
        // Shuffled: every song once per round.
        let mut s = Playlist::new(true, 42);
        s.replace((0..6).map(|i| p(&format!("{i}.mp3"))).collect());
        let mut seen: Vec<String> = (0..6)
            .map(|_| {
                let c = s.current().unwrap().display().to_string();
                s.next();
                c
            })
            .collect();
        seen.sort();
        assert_eq!(seen, (0..6).map(|i| format!("{i}.mp3")).collect::<Vec<_>>());
    }

    #[test]
    fn titles_come_from_tags_or_a_tidied_file_name_and_fade_in_and_out() {
        let f = std::path::Path::new("/music/03_my-song.mp3");
        assert_eq!(title_of(f, Some("Band"), Some("Song")), "Band - Song");
        assert_eq!(title_of(f, None, Some(" Only Title ")), "Only Title");
        assert_eq!(title_of(f, Some("Band"), None), "03 my song");
        assert_eq!(ascii_fold("Sigur Rós – Hoppípolla, Björk’s ÉCLAT"), "Sigur Ros - Hoppipolla, Bjork's ECLAT");
        assert_eq!(
            (title_alpha(-0.1, 10.0), title_alpha(0.25, 10.0), title_alpha(5.0, 10.0), title_alpha(9.5, 10.0), title_alpha(10.5, 10.0)),
            (0.0, 0.5, 1.0, 0.5, 0.0)
        );
        assert_eq!(title_alpha(1.0, 0.0), 0.0);
        assert_ne!(song_seed("a.mp3", 100), song_seed("b.mp3", 100));
        assert!(song_seed("a.mp3", 100) < 1 << 24);
    }

    #[test]
    fn a_playlist_block_is_checked() {
        let parse = |v: Value| {
            let mut errs = Vec::new();
            let p = parse_playlist(v.as_object().unwrap(), &mut errs);
            (p, errs)
        };
        let (p, e) = parse(serde_json::json!({"playlist": true}));
        assert_eq!((p, e.len()), (Some(PlaylistSpec { title_secs: 10.0, corner: Corner::BottomLeft, shuffle: false }), 0));
        let (p, _) = parse(serde_json::json!({"playlist": {"title_secs": 4, "corner": "top_right", "shuffle": true}}));
        assert_eq!(p, Some(PlaylistSpec { title_secs: 4.0, corner: Corner::TopRight, shuffle: true }));
        for (bad, needle) in [
            (serde_json::json!({"playlist": 3}), "audio.playlist"),
            (serde_json::json!({"playlist": {"corner": "middle"}}), "corner"),
            (serde_json::json!({"playlist": {"title_secs": -1}}), "title_secs"),
            (serde_json::json!({"playlist": {"loud": true}}), "loud"),
        ] {
            let (_, e) = parse(bad);
            assert!(e.iter().any(|m| m.contains(needle)), "{needle}: {e:?}");
        }
    }

    #[cfg(feature = "decode")]
    #[test]
    fn a_wav_file_decodes_to_samples_mono_a_title_and_a_seed_and_a_bad_file_says_why() {
        let dir = std::env::temp_dir().join(format!("re2-listen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stereo: Vec<f32> = (0..44_100)
            .flat_map(|i| {
                let v = 0.5 * (2.0 * PI * 220.0 * i as f32 / 44_100.0).sin();
                [v, -v]
            })
            .collect();
        let file = dir.join("01_quiet-tune.wav");
        crate::synth::write_wav_i16(&file, &stereo, 44_100, 2).unwrap();
        let song = decode(&file).unwrap();
        assert_eq!((song.channels, song.rate, song.title.as_str()), (2, 44_100, "01 quiet tune"));
        assert!((song.secs() - 1.0).abs() < 0.01 && song.mono.len() == RATE as usize, "{} s, {} mono", song.secs(), song.mono.len());
        assert_eq!(song.seed, song_seed("01_quiet-tune.wav", song.samples.len()));
        let bad = dir.join("not-music.mp3");
        std::fs::write(&bad, b"hello").unwrap();
        let e = decode(&bad).err().unwrap();
        assert!(e.contains("not-music.mp3"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mono_at_the_analysis_rate_keeps_length_and_pitch() {
        let stereo: Vec<f32> = (0..44_100)
            .flat_map(|i| {
                let v = (2.0 * PI * 441.0 * i as f32 / 44_100.0).sin();
                [v, v]
            })
            .collect();
        let m = to_analysis_rate(&stereo, 2, 44_100);
        assert_eq!(m.len(), RATE as usize);
        // 441 Hz at 22050: one cycle every 50 samples.
        assert!((m[50] - m[0]).abs() < 0.05 && (m[25] + m[0]).abs() < 0.1, "{} {} {}", m[0], m[25], m[50]);
    }
}
