//! The player's own music (`audio.playlist`) in the live client: a file picker, files dropped on the window, decoding on a worker, the song on the
//! sound card, and what it sounds like handed to the renderer every frame so a game sky can see it. The queue, the analysis and the title timing are
//! `crate::listen` (pure, tested); this file is only the devices and the keys.
//!
//! Keys while playing: O chooses music (replacing the queue), Space pauses and resumes the music and the picture together, Right/Left skip, N mutes.

use super::*;
use crate::listen::{decode, title_alpha, Analyzer, Corner, Heard, Playlist, PlaylistSpec, Song, EXTENSIONS};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

/// How long a message (a file that would not play) stays on screen.
const NOTE_SECS: f32 = 6.0;

pub(crate) struct Listener {
    spec: PlaylistSpec,
    list: Playlist,
    /// Requests to the decoder (request number, file) and its answers.
    to_worker: Sender<(u64, PathBuf)>,
    from_worker: Receiver<(u64, Result<Song, String>)>,
    /// The request whose answer is wanted (older answers are dropped: the player skipped on).
    asked: u64,
    loading: bool,
    song: Option<Song>,
    /// The song's samples have gone to the sound card.
    started: bool,
    paused: bool,
    /// Seconds into the song (counted here, steered toward the sound card's position while that moves), and the card's last position.
    pos: f32,
    card_pos: f32,
    /// Seconds of listening: `sky_time()` with a playlist, so Space freezes the picture with the music.
    clock: f32,
    analyzer: Analyzer,
    heard: Heard,
    play_seed: u64,
    plays: u32,
    /// Seconds since the current song started (for its title).
    title_age: f32,
    note: Option<(String, f32)>,
    /// Files are being dragged over the window.
    pub(crate) hovering: bool,
    /// The picker has been offered once (it opens by itself when the game starts with nothing queued).
    offered: bool,
}

impl Listener {
    pub(crate) fn new(spec: PlaylistSpec, seed: u64) -> Listener {
        let (to_worker, requests) = channel::<(u64, PathBuf)>();
        let (answers, from_worker) = channel();
        std::thread::spawn(move || {
            while let Ok((id, file)) = requests.recv() {
                // Only the newest request matters: skip ones the player has already moved past.
                let (mut id, mut file) = (id, file);
                while let Ok((newer, f)) = requests.try_recv() {
                    (id, file) = (newer, f);
                }
                if answers.send((id, decode(&file))).is_err() {
                    return;
                }
            }
        });
        Listener {
            spec,
            list: Playlist::new(spec.shuffle, seed),
            to_worker,
            from_worker,
            asked: 0,
            loading: false,
            song: None,
            started: false,
            paused: false,
            pos: 0.0,
            card_pos: 0.0,
            clock: 0.0,
            analyzer: Analyzer::new(),
            heard: Heard { listening: true, ..Heard::default() },
            play_seed: seed | 1,
            plays: 0,
            title_age: f32::MAX,
            note: None,
            hovering: false,
            offered: false,
        }
    }

    /// Stops the song and asks the worker for the queue's current file.
    fn load_current(&mut self, audio: Option<&mut Audio>) {
        if let Some(a) = audio {
            a.song_volume(0.0);
            a.song_pause(true);
        }
        self.song = None;
        self.started = false;
        self.pos = 0.0;
        self.card_pos = 0.0;
        if let Some(file) = self.list.current() {
            self.asked += 1;
            self.loading = self.to_worker.send((self.asked, file.to_path_buf())).is_ok();
        }
    }

    /// Replaces the queue (the picker).
    pub(crate) fn replace(&mut self, files: Vec<PathBuf>, audio: Option<&mut Audio>) {
        let n = files.len();
        let kept = self.list.replace(files);
        self.report_kept(n, kept);
        if kept > 0 {
            self.paused = false;
            self.load_current(audio);
        }
    }

    /// Adds dropped files after the current song and plays the first of them.
    pub(crate) fn add(&mut self, files: Vec<PathBuf>, audio: Option<&mut Audio>) {
        let n = files.len();
        let kept = self.list.add(files);
        self.report_kept(n, kept);
        if kept > 0 {
            self.paused = false;
            self.load_current(audio);
        }
    }

    fn report_kept(&mut self, given: usize, kept: usize) {
        if kept < given {
            let skipped = given - kept;
            self.note =
                Some((format!("{skipped} FILE{} SKIPPED: NOT MUSIC ({})", if skipped == 1 { "" } else { "S" }, EXTENSIONS.join(" ").to_uppercase()), 0.0));
        }
    }

    pub(crate) fn next(&mut self, audio: Option<&mut Audio>) {
        self.list.next();
        self.load_current(audio);
    }

    pub(crate) fn previous(&mut self, audio: Option<&mut Audio>) {
        // Like every player: back to the start of this song unless it has only just begun.
        if self.pos > 3.0 {
            self.load_current(audio);
        } else {
            self.list.previous();
            self.load_current(audio);
        }
    }

    pub(crate) fn toggle_pause(&mut self, audio: Option<&Audio>) {
        if self.song.is_none() {
            return;
        }
        self.paused = !self.paused;
        if let Some(a) = audio {
            a.song_pause(self.paused);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Advances by `dt`: collects a decoded song and starts it, follows the playhead, moves on at the end, and returns what the sky hears.
    pub(crate) fn update(&mut self, dt: f32, mut audio: Option<&mut Audio>, music_on: bool) -> Heard {
        while let Ok((id, result)) = self.from_worker.try_recv() {
            if id != self.asked {
                continue;
            }
            self.loading = false;
            match result {
                Ok(song) => {
                    println!("playing {} ({:.0} s)", song.title, song.secs());
                    self.plays += 1;
                    self.play_seed ^= self.play_seed << 13;
                    self.play_seed ^= self.play_seed >> 7;
                    self.play_seed ^= self.play_seed << 17;
                    self.analyzer = Analyzer::new();
                    self.title_age = 0.0;
                    self.song = Some(song);
                }
                Err(e) => {
                    eprintln!("{e}");
                    self.note = Some((e.to_uppercase(), 0.0));
                    // A file that will not play is skipped (unless it is the only one: then there is nothing to skip to).
                    if self.list.len() > 1 {
                        self.list.next();
                        self.load_current(audio.as_deref_mut());
                    }
                }
            }
        }
        let volume = if music_on { 1.0 } else { 0.0 };
        if let (Some(song), false) = (&self.song, self.started) {
            if let Some(a) = audio.as_deref_mut() {
                a.song_play(song.samples.clone(), song.channels, song.rate, volume);
                a.song_pause(self.paused);
            }
            self.started = true;
        }
        if let Some(a) = audio.as_deref() {
            a.song_volume(volume);
        }
        if let Some(n) = self.note.as_mut() {
            n.1 += dt;
        }
        if self.note.as_ref().is_some_and(|n| n.1 > NOTE_SECS) {
            self.note = None;
        }
        if self.paused {
            // Everything holds still, the picture included.
            return self.heard;
        }
        self.clock += dt;
        self.title_age = self.title_age.saturating_add_secs(dt);
        let Some(song) = &self.song else {
            // Nothing playing: the sky rests (it eases toward silence).
            let fade = (-dt / 0.6).exp();
            self.heard.playing *= fade;
            self.heard.level *= fade;
            self.heard.beat *= fade;
            self.heard.bands.iter_mut().for_each(|b| *b *= fade);
            self.heard.clock = self.clock;
            return self.heard;
        };
        // The playhead is counted here; the sound card's own position corrects drift when the two agree (a run faster than real time, a
        // headless one, never waits for the speakers).
        self.pos += dt;
        if let Some(card) = audio.as_deref().and_then(Audio::song_pos) {
            // Only a position that is moving is the speakers' (a device that consumes nothing reports the same number for ever).
            if card > self.card_pos && (card - self.pos).abs() < 0.25 {
                self.pos += (card - self.pos) * 0.2;
            }
            self.card_pos = card;
        }
        let ended = self.pos >= song.secs() || (self.started && self.pos > 1.0 && audio.as_deref().is_some_and(Audio::song_done));
        let mut heard = self.analyzer.hear(&song.mono, self.pos, dt);
        heard.song = song.seed;
        heard.play = (self.play_seed as u32) & 0x00ff_ffff;
        heard.clock = self.clock;
        heard.listening = true;
        heard.playing = (self.heard.playing + dt / 0.8).min(1.0);
        self.heard = heard;
        if ended {
            self.next(audio);
        }
        self.heard
    }

    /// What the listener is doing, for the state dump: the song, where it is, whether it is paused, what the sky hears.
    pub(crate) fn state(&self) -> serde_json::Value {
        let r = |v: f32| (v * 100.0).round() / 100.0;
        serde_json::json!({
            "queued": self.list.len(),
            "song": self.song.as_ref().map(|s| s.title.clone()),
            "loading": self.loading,
            "paused": self.paused,
            "pos": r(self.pos),
            "clock": r(self.clock),
            "plays": self.plays,
            "level": r(self.heard.level),
            "bass": r(self.heard.bass),
            "beats": self.heard.beats,
            "captions": self.captions().into_iter().map(|(t, a, _)| serde_json::json!({"text": t, "alpha": r(a)})).collect::<Vec<_>>(),
        })
    }

    /// The words to show now: (text, opacity, corner). The song's title while it is new; "PAUSED"; a note; or, with nothing queued, how to start.
    pub(crate) fn captions(&self) -> Vec<(String, f32, Corner)> {
        let mut out = Vec::new();
        if self.hovering {
            out.push(("DROP TO PLAY".to_string(), 1.0, Corner::BottomLeft));
            return out;
        }
        if let Some(song) = &self.song {
            let a = title_alpha(self.title_age, self.spec.title_secs);
            if a > 0.0 {
                out.push((crate::listen::ascii_fold(&song.title).to_uppercase(), a, self.spec.corner));
            }
        } else if self.loading {
            out.push(("LOADING...".to_string(), 0.8, self.spec.corner));
        } else if self.list.is_empty() {
            out.push(("PRESS O TO CHOOSE MUSIC, OR DROP FILES HERE".to_string(), 0.9, Corner::BottomLeft));
        }
        if self.paused {
            out.push(("PAUSED (SPACE)".to_string(), 0.9, Corner::TopRight));
        }
        if let Some((text, age)) = &self.note {
            out.push((text.clone(), (1.0 - age / NOTE_SECS * 0.3).clamp(0.0, 1.0), Corner::TopLeft));
        }
        out
    }
}

trait SaturatingSecs {
    fn saturating_add_secs(self, dt: f32) -> f32;
}

impl SaturatingSecs for f32 {
    fn saturating_add_secs(self, dt: f32) -> f32 {
        if self >= f32::MAX - dt {
            self
        } else {
            self + dt
        }
    }
}

impl App {
    /// Starts listening when the scene has an `audio.playlist`: songs given with `RE2_SONGS` (paths separated by the system's path separator, for
    /// scripted runs) are queued, otherwise the picker opens once the window is up (never in a headless run).
    pub(crate) fn start_listening(&mut self) {
        let Some(spec) = self.scene.audio.as_ref().and_then(|a| a.playlist) else { return };
        if self.listen.is_some() {
            return;
        }
        let mut l = Listener::new(spec, crate::play2d::launch_seed(None));
        if let Some(songs) = std::env::var_os("RE2_SONGS") {
            l.replace(std::env::split_paths(&songs).collect(), self.audio.as_mut());
            l.offered = true;
        }
        if !self.dropped.is_empty() {
            l.replace(std::mem::take(&mut self.dropped), self.audio.as_mut());
            l.offered = true;
        }
        self.listen = Some(l);
    }

    /// Opens the file picker (O, or by itself the first time) and queues what the player chose.
    pub(crate) fn pick_music(&mut self) {
        if self.headless || self.listen.is_none() {
            return;
        }
        let was_grabbed = self.grabbed;
        self.set_grab(false);
        let chosen = rfd::FileDialog::new().set_title("Choose music").add_filter("Music", EXTENSIONS).pick_files();
        if let (Some(files), Some(l)) = (chosen, self.listen.as_mut()) {
            l.replace(files, self.audio.as_mut());
        }
        if was_grabbed {
            self.set_grab(true);
        }
        // The dialog held the event loop: do not count its time as a frame.
        self.last_frame = Instant::now();
    }

    /// Per frame: the queue, the song and what the sky hears.
    pub(crate) fn update_listening(&mut self, dt: f32) {
        let Some(l) = self.listen.as_mut() else { return };
        if !self.dropped.is_empty() {
            l.add(std::mem::take(&mut self.dropped), self.audio.as_mut());
            l.offered = true;
        }
        let offer = !l.offered && l.is_empty() && !self.headless && self.phase == Phase::Playing && self.card.is_none();
        if offer {
            l.offered = true;
        }
        let heard = l.update(dt, self.audio.as_mut(), self.music_on);
        if let Some(live) = self.gpu.as_mut().and_then(|g| g.live.as_mut()) {
            live.set_audio(&heard);
        }
        if offer {
            self.pick_music();
        }
    }

    /// A key while playing; true when the listener used it.
    pub(crate) fn listen_key(&mut self, code: KeyCode) -> bool {
        let Some(l) = self.listen.as_mut() else { return false };
        match code {
            KeyCode::Space => l.toggle_pause(self.audio.as_ref()),
            KeyCode::ArrowRight => l.next(self.audio.as_mut()),
            KeyCode::ArrowLeft => l.previous(self.audio.as_mut()),
            KeyCode::KeyO => {
                self.pick_music();
                return true;
            }
            _ => return false,
        }
        true
    }

    /// What the listener's words look like now (empty: nothing to show), to tell when the overlay must be painted again.
    pub(crate) fn listen_caption_key(&self) -> String {
        let Some(l) = self.listen.as_ref() else { return String::new() };
        l.captions().iter().map(|(text, alpha, corner)| format!("{text}|{}|{corner:?};", (alpha * 8.0).round())).filter(|k| !k.contains("|0|")).collect()
    }

    /// The listener's words as overlay widgets (small, in a corner).
    pub(crate) fn listen_captions(&self, layout: &mut crate::ui::Layout, w: u32, h: u32) {
        let Some(l) = self.listen.as_ref() else { return };
        // Small but legible: 2x the bitmap font at 720p and 1080p, 3x at 1440p.
        let s = ((h as i32 + 200) / 450).max(1);
        let margin = 8 * s;
        let mut stacked = [0i32; 4];
        for (i, (text, alpha, corner)) in l.captions().into_iter().enumerate() {
            let a = (alpha * 8.0).round() / 8.0;
            if a <= 0.0 {
                continue;
            }
            let text = crate::ui::ellipsize(&text, w as i32 * 3 / 5, s);
            let tw = crate::ui::text_width(&text, s);
            let th = crate::ui::text_height(s);
            let slot = corner as usize;
            let line = stacked[slot];
            stacked[slot] += 1;
            let x = match corner {
                Corner::TopLeft | Corner::BottomLeft => margin + tw / 2,
                Corner::TopRight | Corner::BottomRight => w as i32 - margin - tw / 2,
            };
            // `label_fit` takes the centre x and the top y.
            let y = match corner {
                Corner::TopLeft | Corner::TopRight => margin + line * (th + 6 * s),
                Corner::BottomLeft | Corner::BottomRight => h as i32 - margin - th - line * (th + 6 * s),
            };
            let alpha8 = (a * 235.0) as u8;
            layout.label_fit(&format!("listen_{i}"), None, x, y, &text, s, w as i32 - 2 * margin, [236, 240, 248, alpha8]);
        }
    }
}
