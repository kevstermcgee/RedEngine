//! The mixing desk: buses with levels, a limit on how many sounds play at once (and who gets dropped), ducking of the music under important sounds, a reverb
//! for dry clips, and what several listeners (split-screen) hear.
//!
//! Pure, so it is tested without a sound card: [`Mixer::request`] answers "may this sound start now, and at what level?" given the time, and the client does what
//! it says. The pieces:
//!
//! * **Buses.** [`Bus::Music`], [`Bus::Sfx`], [`Bus::Ui`], [`Bus::Ambience`] under a master. [`Levels`] holds a gain for each (the player's switches and volume are
//!   levels: off is 0), and [`Levels::gain`] is master times bus.
//! * **Voices.** Each bus has a limit ([`Bus::voices`]): when a sound arrives with every voice busy the least important one is stolen (lowest [`Priority`], then
//!   the quietest, then the one closest to ending) if the newcomer is more important or clearly louder, otherwise the newcomer is dropped. A burst of the same
//!   sound ([`Cue::key`]) is thinned (at most [`SAME_SOUND`] within [`SAME_SOUND_WINDOW`] seconds), which is what a flurry of shots or footsteps needs.
//! * **Ducking.** A cue may carry a [`Duck`]: the music bus drops by `depth`, quickly, stays down while the cue plays, and comes back over `release` seconds.
//! * **Reverb.** [`wet`] puts a dry mono clip in a room (a scene's `audio.reverb`), returning stereo with the tail.
//! * **Listeners.** With several players on one screen each is a [`Listener`]; [`hear`] answers where a sound at some point lands: the loudest listener's gain
//!   and pan, so a shot beside player two is loud for the room, not four times as loud.

use crate::audio_fx::Reverb;
use crate::synth::SAMPLE_RATE;

/// A group of sounds with one level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bus {
    /// Scores and music layers.
    Music,
    /// Gameplay sounds: shots, footsteps, pickups.
    Sfx,
    /// Menus and cards.
    Ui,
    /// The world's background: wind, birds, crickets.
    Ambience,
}

impl Bus {
    /// Every bus.
    pub const ALL: [Bus; 4] = [Bus::Music, Bus::Sfx, Bus::Ui, Bus::Ambience];

    fn index(self) -> usize {
        self as usize
    }

    /// How many sounds may play on this bus at once.
    pub fn voices(self) -> usize {
        match self {
            Bus::Music => 8,
            Bus::Sfx => 24,
            Bus::Ui => 4,
            Bus::Ambience => 6,
        }
    }
}

/// How much a sound matters when there are too many.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Atmosphere: first to go.
    Low,
    /// Ordinary gameplay sounds.
    Normal,
    /// Sounds the player must hear (a hit taken, a warning).
    High,
}

/// The music bus drops under a sound that matters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Duck {
    /// How far the music falls, 0 (not at all) to 1 (silent).
    pub depth: f32,
    /// Seconds to get there.
    pub attack: f32,
    /// Seconds to come back once the sound is over.
    pub release: f32,
}

impl Default for Duck {
    fn default() -> Self {
        Duck { depth: 0.5, attack: 0.08, release: 1.2 }
    }
}

/// A request to start a sound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cue {
    /// Which bus it plays on.
    pub bus: Bus,
    /// Its loudness before the bus levels, 0 to 1.
    pub gain: f32,
    /// Left (-1) to right (1).
    pub pan: f32,
    /// How much it matters.
    pub priority: Priority,
    /// Which sound it is (so bursts of the same one can be thinned); 0 for "never thin".
    pub key: u64,
    /// How long it lasts, seconds.
    pub secs: f32,
    /// Whether it ducks the music, and how.
    pub duck: Option<Duck>,
}

impl Cue {
    /// An ordinary gameplay sound.
    pub fn sfx(gain: f32, pan: f32, key: u64, secs: f32) -> Cue {
        Cue { bus: Bus::Sfx, gain, pan, priority: Priority::Normal, key, secs, duck: None }
    }

    /// A background sound.
    pub fn ambience(gain: f32, pan: f32, key: u64, secs: f32) -> Cue {
        Cue { bus: Bus::Ambience, gain, pan, priority: Priority::Low, key, secs, duck: None }
    }
}

/// Bus levels, 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Levels {
    /// Everything.
    pub master: f32,
    /// Per bus, in [`Bus::ALL`] order.
    pub bus: [f32; 4],
}

impl Default for Levels {
    fn default() -> Self {
        Levels { master: 1.0, bus: [1.0; 4] }
    }
}

impl Levels {
    /// The gain of a bus: master times bus.
    pub fn gain(&self, bus: Bus) -> f32 {
        self.master.clamp(0.0, 1.0) * self.bus[bus.index()].clamp(0.0, 1.0)
    }

    /// Sets a bus level.
    pub fn set(&mut self, bus: Bus, level: f32) {
        self.bus[bus.index()] = level.clamp(0.0, 1.0);
    }
}

/// At most this many starts of the same sound within [`SAME_SOUND_WINDOW`] seconds.
pub const SAME_SOUND: usize = 3;
/// See [`SAME_SOUND`].
pub const SAME_SOUND_WINDOW: f64 = 0.06;

/// Why a sound did not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    /// The bus is silent (switched off): nothing to do, and nothing counted.
    Silent,
    /// The same sound had just started several times.
    Burst,
    /// Every voice on the bus is busy with something that matters as much or more.
    Busy,
}

/// The answer to a [`Cue`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// Start it: at this final gain and pan; `stolen` is the voice to cut off to make room, if any; `id` names the new voice.
    Play {
        /// The identity of the new voice.
        id: u64,
        /// The final loudness: the cue's gain times its bus.
        gain: f32,
        /// Left (-1) to right (1).
        pan: f32,
        /// A voice to stop first.
        stolen: Option<u64>,
    },
    /// Do not start it.
    Drop(Dropped),
}

#[derive(Debug, Clone, Copy)]
struct Playing {
    id: u64,
    bus: Bus,
    priority: Priority,
    gain: f32,
    key: u64,
    start: f64,
    end: f64,
}

/// Counters, for tests and a debug overlay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Sounds started.
    pub started: u64,
    /// Voices cut off to make room.
    pub stolen: u64,
    /// Sounds thinned out of a burst.
    pub thinned: u64,
    /// Sounds dropped because every voice mattered more.
    pub dropped_busy: u64,
}

/// The state of the mixing desk.
#[derive(Debug, Clone, Default)]
pub struct Mixer {
    levels: Levels,
    playing: Vec<Playing>,
    next_id: u64,
    duck_start: f64,
    duck_until: f64,
    duck: Option<Duck>,
    stats: Stats,
}

impl Mixer {
    /// A desk with these levels.
    pub fn new(levels: Levels) -> Mixer {
        Mixer { levels, duck_until: f64::NEG_INFINITY, duck_start: f64::NEG_INFINITY, ..Default::default() }
    }

    /// The bus levels.
    pub fn levels(&self) -> Levels {
        self.levels
    }

    /// Changes the bus levels.
    pub fn set_levels(&mut self, levels: Levels) {
        self.levels = levels;
    }

    /// Counters.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// How many voices are playing on a bus at time `now`.
    pub fn active(&self, bus: Bus, now: f64) -> usize {
        self.playing.iter().filter(|p| p.bus == bus && p.end > now).count()
    }

    /// The multiplier on the music bus at time `now`: 1, or lower while a ducking sound plays and as it recovers.
    pub fn duck_multiplier(&self, now: f64) -> f32 {
        let Some(d) = self.duck else { return 1.0 };
        let low = 1.0 - d.depth.clamp(0.0, 1.0);
        if now >= self.duck_start && now < self.duck_start + d.attack as f64 {
            let u = ((now - self.duck_start) / d.attack.max(1e-3) as f64) as f32;
            1.0 + (low - 1.0) * u
        } else if now < self.duck_until {
            low
        } else {
            let u = (((now - self.duck_until) / d.release.max(1e-3) as f64) as f32).clamp(0.0, 1.0);
            low + (1.0 - low) * u
        }
    }

    /// Ducks the music now: it falls by `duck.depth` and stays down for `hold` seconds, then recovers (a scene's `audio.duck` on a game event).
    pub fn duck_for(&mut self, now: f64, duck: Duck, hold: f64) {
        if self.duck.is_none_or(|old| duck.depth >= old.depth || now >= self.duck_until) {
            self.duck = Some(duck);
            self.duck_start = now;
        }
        self.duck_until = self.duck_until.max(now + hold);
    }

    /// The gain a bus has right now (music includes the duck).
    pub fn bus_gain(&self, bus: Bus, now: f64) -> f32 {
        self.levels.gain(bus) * if bus == Bus::Music { self.duck_multiplier(now) } else { 1.0 }
    }

    /// Asks to start a sound at time `now` (seconds on any steady clock).
    pub fn request(&mut self, now: f64, cue: &Cue) -> Verdict {
        self.playing.retain(|p| p.end > now);
        let bus_gain = self.levels.gain(cue.bus);
        if bus_gain <= 0.0 {
            return Verdict::Drop(Dropped::Silent);
        }
        if cue.key != 0 && self.playing.iter().filter(|p| p.key == cue.key && now - p.start < SAME_SOUND_WINDOW).count() >= SAME_SOUND {
            self.stats.thinned += 1;
            return Verdict::Drop(Dropped::Burst);
        }
        let gain = (cue.gain * bus_gain).clamp(0.0, 1.0);
        let mut stolen = None;
        if self.playing.iter().filter(|p| p.bus == cue.bus).count() >= cue.bus.voices() {
            // The victim: least important, then quietest, then the one nearest its end.
            let victim = self
                .playing
                .iter()
                .filter(|p| p.bus == cue.bus)
                .min_by(|a, b| a.priority.cmp(&b.priority).then(a.gain.total_cmp(&b.gain)).then(a.end.total_cmp(&b.end)))
                .copied();
            match victim {
                Some(v) if cue.priority > v.priority || (cue.priority == v.priority && gain > v.gain * 1.25) => {
                    self.playing.retain(|p| p.id != v.id);
                    self.stats.stolen += 1;
                    stolen = Some(v.id);
                }
                _ => {
                    self.stats.dropped_busy += 1;
                    return Verdict::Drop(Dropped::Busy);
                }
            }
        }
        self.next_id += 1;
        let id = self.next_id;
        self.playing.push(Playing { id, bus: cue.bus, priority: cue.priority, gain, key: cue.key, start: now, end: now + cue.secs.max(0.0) as f64 });
        self.stats.started += 1;
        if let Some(d) = cue.duck {
            // A later, deeper duck replaces an earlier one only if it asks for more; the hold lasts as long as the sound.
            self.duck_for(now, d, cue.secs as f64);
        }
        Verdict::Play { id, gain, pan: cue.pan.clamp(-1.0, 1.0), stolen }
    }
}

/// A dry mono clip in a room: stereo (interleaved) with the reverb's tail added, long enough to hold it.
pub fn wet(clip: &[f32], reverb: &Reverb) -> Vec<f32> {
    let tail = (reverb.decay * 0.7 * SAMPLE_RATE as f32) as usize;
    let mut dry = clip.to_vec();
    dry.extend(std::iter::repeat_n(0.0, tail));
    let sent = (dry.clone(), dry.clone());
    let (wl, wr) = reverb.process(&sent);
    let mut out = Vec::with_capacity(dry.len() * 2);
    for i in 0..dry.len() {
        out.push(dry[i] + wl[i]);
        out.push(dry[i] + wr[i]);
    }
    out
}

/// Someone who hears: a position (x, y, z) and where they face (the engine's yaw).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    /// Where they are.
    pub pos: [f32; 3],
    /// Which way they face, radians (0 is -Z, clockwise from above).
    pub yaw: f32,
}

/// Where a sound at `source` lands for the loudest of the `listeners`: `(gain, pan)`. With none, the sound is heard at full level, centred.
pub fn hear(listeners: &[Listener], source: [f32; 3]) -> (f32, f32) {
    listeners.iter().map(|l| crate::sfx::spatial(l.pos, l.yaw, source)).max_by(|a, b| a.0.total_cmp(&b.0)).unwrap_or((1.0, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(m: &mut Mixer, now: f64, cue: Cue) -> Verdict {
        m.request(now, &cue)
    }

    #[test]
    fn levels_multiply_down_the_buses_and_a_silent_bus_plays_nothing() {
        let mut l = Levels { master: 0.5, ..Default::default() };
        l.set(Bus::Sfx, 0.5);
        assert_eq!(l.gain(Bus::Sfx), 0.25);
        assert_eq!(l.gain(Bus::Music), 0.5);
        let mut m = Mixer::new(l);
        let Verdict::Play { gain, .. } = play(&mut m, 0.0, Cue::sfx(0.8, 0.0, 1, 1.0)) else { panic!("should play") };
        assert!((gain - 0.2).abs() < 1e-6);
        l.set(Bus::Sfx, 0.0);
        m.set_levels(l);
        assert_eq!(play(&mut m, 0.1, Cue::sfx(0.8, 0.0, 2, 1.0)), Verdict::Drop(Dropped::Silent));
        assert_eq!(m.stats().started, 1, "a muted bus counts nothing");
    }

    #[test]
    fn a_burst_of_the_same_sound_is_thinned_but_different_sounds_are_not() {
        let mut m = Mixer::new(Levels::default());
        let shot = Cue::sfx(0.8, 0.0, 7, 0.5);
        let results: Vec<Verdict> = (0..6).map(|k| play(&mut m, k as f64 * 0.005, shot)).collect();
        assert_eq!(results.iter().filter(|v| matches!(v, Verdict::Play { .. })).count(), SAME_SOUND);
        assert!(results[5] == Verdict::Drop(Dropped::Burst));
        // Spread out, they all play; and another sound at the same instant plays.
        assert!(matches!(play(&mut m, 1.0, shot), Verdict::Play { .. }));
        assert!(matches!(play(&mut m, 1.0, Cue::sfx(0.8, 0.0, 8, 0.5)), Verdict::Play { .. }));
        // Key 0 is never thinned.
        let free = Cue::sfx(0.5, 0.0, 0, 0.5);
        assert!((0..10).all(|k| matches!(play(&mut m, 2.0 + k as f64 * 0.001, free), Verdict::Play { .. })));
    }

    #[test]
    fn a_full_bus_steals_from_the_least_important_and_drops_what_is_less() {
        let mut m = Mixer::new(Levels::default());
        let n = Bus::Ui.voices();
        for k in 0..n {
            let cue = Cue { bus: Bus::Ui, gain: 0.5, pan: 0.0, priority: Priority::Normal, key: 100 + k as u64, secs: 5.0, duck: None };
            assert!(matches!(play(&mut m, 0.0, cue), Verdict::Play { stolen: None, .. }));
        }
        assert_eq!(m.active(Bus::Ui, 0.1), n);
        let quiet = Cue { bus: Bus::Ui, gain: 0.3, pan: 0.0, priority: Priority::Normal, key: 1, secs: 1.0, duck: None };
        assert_eq!(play(&mut m, 0.1, quiet), Verdict::Drop(Dropped::Busy), "no more important than what is playing");
        let loud = Cue { gain: 0.9, key: 2, ..quiet };
        assert!(matches!(play(&mut m, 0.1, loud), Verdict::Play { stolen: Some(_), .. }), "clearly louder: steals");
        let urgent = Cue { priority: Priority::High, gain: 0.2, key: 3, ..quiet };
        assert!(matches!(play(&mut m, 0.1, urgent), Verdict::Play { stolen: Some(_), .. }), "more important: steals even when quiet");
        let low = Cue { priority: Priority::Low, gain: 1.0, key: 4, ..quiet };
        assert_eq!(play(&mut m, 0.1, low), Verdict::Drop(Dropped::Busy), "atmosphere never evicts gameplay");
        assert_eq!(m.active(Bus::Ui, 0.1), n, "the count never exceeds the limit");
        assert_eq!((m.stats().stolen, m.stats().dropped_busy), (2, 2));
        // When they end, room again.
        assert!(matches!(play(&mut m, 6.0, quiet), Verdict::Play { stolen: None, .. }));
    }

    #[test]
    fn the_stolen_voice_is_the_quietest_of_the_least_important() {
        let mut m = Mixer::new(Levels::default());
        let mut ids = Vec::new();
        for k in 0..Bus::Ui.voices() {
            let cue = Cue { bus: Bus::Ui, gain: 0.4 + 0.1 * k as f32, pan: 0.0, priority: Priority::Normal, key: 10 + k as u64, secs: 5.0, duck: None };
            let Verdict::Play { id, .. } = play(&mut m, 0.0, cue) else { panic!() };
            ids.push(id);
        }
        let Verdict::Play { stolen: Some(victim), .. } =
            play(&mut m, 0.1, Cue { bus: Bus::Ui, gain: 1.0, pan: 0.0, priority: Priority::Normal, key: 99, secs: 1.0, duck: None })
        else {
            panic!()
        };
        assert_eq!(victim, ids[0], "the quietest goes");
    }

    #[test]
    fn important_sounds_duck_the_music_and_it_comes_back() {
        let mut m = Mixer::new(Levels::default());
        assert_eq!(m.duck_multiplier(0.0), 1.0);
        let boom = Cue { duck: Some(Duck { depth: 0.6, attack: 0.1, release: 2.0 }), priority: Priority::High, ..Cue::sfx(1.0, 0.0, 5, 1.0) };
        assert!(matches!(play(&mut m, 10.0, boom), Verdict::Play { .. }));
        assert!((m.duck_multiplier(10.0) - 1.0).abs() < 1e-5, "it starts at full");
        assert!((m.duck_multiplier(10.05) - 0.7).abs() < 0.01, "halfway down after half the attack");
        assert!((m.duck_multiplier(10.5) - 0.4).abs() < 1e-5, "held down while the sound plays");
        assert!((m.duck_multiplier(12.0) - 0.7).abs() < 0.01, "halfway back a second after it ends");
        assert!((m.duck_multiplier(13.5) - 1.0).abs() < 1e-5);
        assert!((m.bus_gain(Bus::Music, 10.5) - 0.4).abs() < 1e-5 && m.bus_gain(Bus::Sfx, 10.5) == 1.0, "only the music ducks");
        // Never louder than 1 or quieter than the depth allows, however many pile up.
        for k in 0..20 {
            let _ = play(&mut m, 20.0 + k as f64 * 0.01, Cue { key: 0, ..boom });
        }
        assert!((0.39..=1.0).contains(&m.duck_multiplier(20.1)));
    }

    #[test]
    fn a_dry_clip_gains_a_tail_and_stays_finite() {
        let clip: Vec<f32> = (0..2000).map(|i| ((i as f32 * 0.3).sin() * (1.0 - i as f32 / 2000.0)) * 0.5).collect();
        let r = Reverb { decay: 1.0, size: 1.0, damp: 0.5, predelay: 0.0, mix: 0.3 };
        let out = wet(&clip, &r);
        assert!(out.len() > clip.len() * 2 + 20_000, "{} frames", out.len() / 2);
        assert!(out.iter().all(|s| s.is_finite()));
        let tail_energy: f32 = out[clip.len() * 2..].iter().map(|s| s * s).sum();
        assert!(tail_energy > 1e-4, "the room rings after the sound stops");
        assert_ne!(out[2 * 9000], out[2 * 9000 + 1], "left and right differ: the room is wide");
    }

    #[test]
    fn several_listeners_hear_a_sound_as_loud_as_the_nearest_hears_it() {
        let a = Listener { pos: [0.0, 0.0, 0.0], yaw: 0.0 };
        let b = Listener { pos: [40.0, 0.0, 0.0], yaw: 0.0 };
        let near_b = [38.0, 0.0, 0.0];
        let (g_both, _) = hear(&[a, b], near_b);
        let (g_a, _) = hear(&[a], near_b);
        let (g_b, _) = hear(&[b], near_b);
        assert!(g_both >= g_a && (g_both - g_b).abs() < 1e-6, "{g_both} {g_a} {g_b}");
        assert!(g_both <= 1.0, "never louder than for one listener standing there");
        assert_eq!(hear(&[], near_b), (1.0, 0.0));
        let (_, pan) = hear(&[a, b], [38.0, 0.0, -1.0]);
        assert!(pan < 0.0, "ahead and to the left of b: heard on the left: {pan}");
    }
}
