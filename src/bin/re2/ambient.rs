//! The countryside and its music in the live client: beds and mood scores rendered on a worker thread, volumes set from [`Ambience`] every frame, bird
//! calls played as they come. The state machine is `red_engine2::ambience`; this file only turns its answers into sound (and keeps what it did for the dump,
//! so a headless script with no sound card can still say "the dawn chorus began").

use super::*;
use red_engine2::ambience::{Ambience, AudioSpec, Context, Frame, Heard, Mood};
use red_engine2::nature::{Bed, Call};
use red_engine2::procgen::Biome;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};

/// A rendered loop on its way from the worker.
enum Loaded {
    Bed(usize, Vec<f32>),
    Mood(usize, Vec<f32>),
}

/// How fast the player's own music and sound switches fade, per second.
const GATE_SLEW: f32 = 1.2;

pub(crate) struct Ambient {
    spec: AudioSpec,
    state: Ambience,
    rx: Receiver<Loaded>,
    bed_layers: [Option<usize>; 5],
    mood_layers: [Option<usize>; 4],
    /// The player's music and sound switches, eased so a toggle is a fade not a click.
    music_gate: f32,
    sound_gate: f32,
    clips: HashMap<(usize, u32), Vec<f32>>,
    biome: Biome,
    biome_age: f32,
    /// The last frame's answer, for the dump.
    pub(crate) last: Frame,
    /// The last few calls heard, newest last.
    pub(crate) recent: Vec<Heard>,
    /// How many loops the worker has finished.
    pub(crate) loaded: usize,
}

impl Ambient {
    /// Starts rendering what the scene's `audio` block asks for. `first_mood` (the mood of the hour the game starts at) is rendered first.
    pub(crate) fn new(spec: AudioSpec, seed: u32, first_mood: Mood, render: bool) -> Ambient {
        let (tx, rx) = channel();
        let music = spec.music.clone();
        let nature = spec.nature;
        // With no sound card there is nothing to play the loops on, so a headless run does not spend minutes rendering them.
        std::thread::spawn(move || {
            if !render {
                return;
            }
            if nature {
                for (i, bed) in Bed::ALL.into_iter().enumerate() {
                    if tx.send(Loaded::Bed(i, bed.render())).is_err() {
                        return;
                    }
                }
            }
            let mut order = music;
            order.sort_by_key(|(m, _)| (*m != first_mood, *m));
            for (mood, path) in order {
                let rendered = std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string()))
                    .and_then(|v| red_engine2::score::parse_score(&v).map_err(|e| e.join("; ")))
                    .map(|score| score.render().samples);
                match rendered {
                    Ok(samples) => {
                        let index = Mood::ALL.iter().position(|m| *m == mood).unwrap_or(0);
                        if tx.send(Loaded::Mood(index, samples)).is_err() {
                            return;
                        }
                    }
                    Err(e) => eprintln!("audio.music.{}: {}: {e}", mood.name(), path.display()),
                }
            }
        });
        Ambient {
            spec,
            state: Ambience::new(seed),
            rx,
            bed_layers: [None; 5],
            mood_layers: [None; 4],
            music_gate: 1.0,
            sound_gate: 1.0,
            clips: HashMap::new(),
            biome: Biome::Meadow,
            biome_age: 99.0,
            last: Frame::default(),
            recent: Vec::new(),
            loaded: 0,
        }
    }

    /// Whether the scene has music of its own (so the synthwave loop stays out of it).
    pub(crate) fn has_music(&self) -> bool {
        !self.spec.music.is_empty()
    }

    /// Advances by `dt` seconds: the hour, the place and the player's switches in, sound out.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(&mut self, dt: f32, audio: Option<&mut Audio>, ctx: Context, biome_at: impl FnOnce() -> Biome, music_on: bool, sound_on: bool) {
        self.biome_age += dt;
        if self.biome_age > 0.5 {
            self.biome = biome_at();
            self.biome_age = 0.0;
        }
        let ctx = Context { biome: self.biome, ..ctx };
        let frame = self.state.step(dt, &ctx);
        let ease = |gate: &mut f32, on: bool| *gate += ((if on { 1.0 } else { 0.0 }) - *gate).clamp(-GATE_SLEW * dt, GATE_SLEW * dt);
        ease(&mut self.music_gate, music_on);
        ease(&mut self.sound_gate, sound_on);
        if let Some(audio) = audio {
            // Loops that have finished rendering join, silent, and are then brought in by the volumes below.
            while let Ok(loaded) = self.rx.try_recv() {
                self.loaded += 1;
                match loaded {
                    Loaded::Bed(i, samples) => self.bed_layers[i] = audio.add_layer(samples),
                    Loaded::Mood(i, samples) => self.mood_layers[i] = audio.add_layer(samples),
                }
            }
            for (i, layer) in self.bed_layers.iter().enumerate() {
                if let Some(id) = layer {
                    audio.set_layer_volume(*id, frame.beds[i] * self.spec.ambience_volume * self.sound_gate);
                }
            }
            for (i, layer) in self.mood_layers.iter().enumerate() {
                if let Some(id) = layer {
                    audio.set_layer_volume(*id, frame.music[i] * self.spec.music_volume * self.music_gate);
                }
            }
            if self.spec.nature {
                for heard in &frame.calls {
                    let index = Call::ALL.iter().position(|c| *c == heard.call).unwrap_or(0);
                    let clip = self.clips.entry((index, heard.seed)).or_insert_with(|| heard.call.render(heard.seed));
                    audio.play_at(clip, heard.gain * self.spec.ambience_volume, heard.pan);
                }
            }
        } else {
            // No sound card (a headless run): the loops are not wanted, so let the worker's results go.
            while self.rx.try_recv().is_ok() {
                self.loaded += 1;
            }
        }
        self.recent.extend(frame.calls.iter().copied());
        if self.recent.len() > 12 {
            let drop = self.recent.len() - 12;
            self.recent.drain(..drop);
        }
        self.last = frame;
    }
}

impl App {
    /// Starts the ambience and the mood music if the scene has an `audio` block.
    pub(crate) fn start_ambient(&mut self) {
        let Some(spec) = self.scene.audio.clone() else { return };
        if std::env::var("RE2_AMBIENCE").is_ok_and(|v| v == "0") {
            return;
        }
        let ctx = self.ambient_context();
        let first = Mood::ALL[mood_of(&ctx)];
        self.ambient = Some(Ambient::new(spec, 0x5eed_u32 ^ self.scene_path.display().to_string().len() as u32, first, self.audio.is_some()));
        self.music_on = self.settings.music && std::env::var("RE2_MUSIC").map_or(true, |v| v != "0");
    }

    /// The hour and speed the soundscape is told about (the place is asked for separately: it is a little costly).
    fn ambient_context(&self) -> Context {
        let (sun, rising) = match &self.scene.clock {
            Some(clock) => {
                let s = clock.state(self.scene_time(), 0);
                (s.sun_elev_deg, s.time < 0.5)
            }
            None => (45.0, true),
        };
        Context { sun_elev_deg: sun, rising, biome: Biome::Meadow, speed: self.last_move_speed }
    }

    /// Per frame: advances the soundscape.
    pub(crate) fn update_ambient(&mut self, dt: f32) {
        if self.ambient.is_none() {
            return;
        }
        let ctx = self.ambient_context();
        let pos = self.physics_pos;
        let ground = self.ground.clone();
        let (music_on, sound_on) = (self.music_on, self.sfx_on);
        let biome_at = move || ground.procgen().map_or(Biome::Meadow, |g| g.world().biome(pos.x as f64, pos.y as f64));
        if let Some(a) = self.ambient.as_mut() {
            a.update(dt, self.audio.as_mut(), ctx, biome_at, music_on, sound_on);
        }
    }

    /// What the soundscape is doing, for the state dump.
    pub(crate) fn ambient_state(&self) -> Option<serde_json::Value> {
        use serde_json::json;
        let a = self.ambient.as_ref()?;
        let calls: serde_json::Map<String, serde_json::Value> = Call::ALL.iter().zip(a.state_totals()).map(|(c, n)| (c.name().to_string(), json!(n))).collect();
        Some(json!({
            "beds": Bed::ALL.iter().zip(a.last.beds).map(|(b, g)| (b.name().to_string(), json!((g * 100.0).round() / 100.0))).collect::<serde_json::Map<_, _>>(),
            "music": Mood::ALL.iter().zip(a.last.music).map(|(m, g)| (m.name().to_string(), json!((g * 100.0).round() / 100.0))).collect::<serde_json::Map<_, _>>(),
            "calls": calls,
            "recent": a.recent.iter().map(|h| json!({"call": h.call.name(), "gain": (h.gain * 100.0).round() / 100.0, "pan": (h.pan * 100.0).round() / 100.0})).collect::<Vec<_>>(),
            "loops_ready": a.loaded,
            "music_on": self.music_on,
            "sound_on": self.sfx_on,
        }))
    }
}

/// The index in [`Mood::ALL`] of the strongest mood for a context.
fn mood_of(ctx: &Context) -> usize {
    let w = red_engine2::ambience::mood_weights(ctx.sun_elev_deg, ctx.rising);
    w.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i)
}

impl Ambient {
    fn state_totals(&self) -> [u32; 7] {
        self.state.totals
    }
}
