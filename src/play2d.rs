//! The native 2D player: a `*.game2d.json` game in an OS window, with sound and a save file.
//!
//! [`red2d::host::Host`] is the whole game as plain calls (ticks in, RGBA frame / sound requests / save text out); this module is the platform around it: a `winit` window,
//! `softbuffer` to show the CPU-rendered frame, `rodio` for the synthesized sounds and music, `gilrs` for a gamepad, and a fixed 60 Hz tick clock. The game's rules,
//! physics and picture are the same code the headless checks run, so what `verify` proves is what is played.
//!
//! Every 2D game gets the same platform keys: `F` or `F11` toggles borderless fullscreen. A gamepad drives the game's actions (stick or d-pad: directions; A: `action`;
//! X, B or the right trigger: `secondary`; Start or Back: `pause`). The game's `icon` is the window's (title bar and taskbar). What the rules ask of the platform
//! (`window`, `save`, `export_music`) is done here: an exported track is written to the player's Music folder, named after its `title`.
//!
//! Launched for a player (`RE2_PLAYER=1`, or `--fullscreen` / `--intro`), the window opens in borderless fullscreen and the "RedEngine Games" title card
//! ([`crate::ui::title_card`]) plays before the first tick; the game waits for it, and any key or click skips it.

use crate::controller::{button, Controller};
use crate::ui::title_card::TitleCard;
use red2d::host::{Host, SAVE_LOADED};
use red2d::sim::Request;
use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const TICK: Duration = Duration::from_micros(16_667);
/// A stall (a dragged window, a debugger) never turns into a burst of more than this many ticks.
const MAX_CATCH_UP: u32 = 5;
const SAVE_EVERY: Duration = Duration::from_secs(2);

/// What the player was asked to do.
pub struct Options {
    /// The game file.
    pub game: PathBuf,
    /// The simulation seed.
    pub seed: u64,
    /// Where progress is kept; default is the per-game save directory.
    pub save: Option<PathBuf>,
    /// No sound device.
    pub mute: bool,
    /// Quit after this many ticks (a smoke test).
    pub max_ticks: Option<u64>,
    /// Open in borderless fullscreen (`F` or `F11` toggles it, as does a rule's `{"window": "fullscreen"}`).
    pub fullscreen: bool,
    /// Play the "RedEngine Games" title card before the game starts.
    pub intro: bool,
}

/// The seed of one launch: `given` when the player asked for one (`--seed N`, to replay a run), otherwise a new one from the clock, so a shipped game does
/// not play the same `random(a, b)` draws every time it starts. The caller prints it (`seed N`), which is how a run is reported and reproduced.
pub fn launch_seed(given: Option<u64>) -> u64 {
    given.unwrap_or_else(|| {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        // A seed a person can retype: the low 9 decimal digits, never 0.
        (nanos % 1_000_000_000).max(1)
    })
}

/// Opens the window and runs until it is closed. Returns an error before the window opens if the game does not load.
pub fn run(opts: Options) -> Result<(), String> {
    let text = std::fs::read_to_string(&opts.game).map_err(|e| format!("{}: {e}", opts.game.display()))?;
    let mut host = Host::default();
    host.init(&text, opts.seed).map_err(|e| format!("{}: {e}", opts.game.display()))?;
    let (id, title) = {
        let d = host.def().ok_or("no game")?;
        (d.id.clone(), d.title.clone())
    };
    let save_path = opts.save.clone().or_else(|| crate::settings::game_state_path(&id, "progress.json"));
    if let Some(Ok(saved)) = save_path.as_deref().map(std::fs::read_to_string) {
        let (code, msg) = host.load_save(&saved);
        if code != SAVE_LOADED {
            eprintln!("progress not restored: {msg}");
        }
    }
    let audio = if opts.mute { None } else { Audio::open(&host) };
    let event_loop = EventLoop::new().map_err(|e| format!("no window system: {e}"))?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        host,
        title,
        save_path,
        audio,
        max_ticks: opts.max_ticks,
        ticks: 0,
        window: None,
        surface: None,
        last: Instant::now(),
        acc: Duration::ZERO,
        last_save: Instant::now(),
        pointer: None,
        error: None,
        fullscreen: opts.fullscreen,
        intro: opts.intro.then(|| (TitleCard::default(), Instant::now())),
        card_px: Vec::new(),
        pad: Controller::default(),
        pad_held: [false; PAD_ACTIONS.len()],
        focused: true,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.write_save();
    if let (Some(m), None) = (app.max_ticks, &app.error) {
        println!("play2d: ran {} ticks (limit {m}), no crash", app.ticks);
    }
    app.error.map_or(Ok(()), Err)
}

/// The game's music tracks, synthesized one by one on a background thread: `None` until a track is ready (or if it cannot be made).
type Tracks = Arc<Mutex<Vec<Option<Vec<f32>>>>>;

/// Sound effects and the looping music track the game asks for (a level can change it). Any failure to reach a sound device just means a silent game.
struct Audio {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    voices: HashMap<u32, Option<Vec<f32>>>,
    /// The sink playing music, and which track it plays.
    music: Option<(Sink, usize)>,
    tracks: Tracks,
}

impl Audio {
    fn open(host: &Host) -> Option<Audio> {
        let (_stream, handle) = OutputStream::try_default().ok()?;
        // Synthesizing a score takes a moment: do every track off the window's thread (the one playing now first), so the first frame and a level change never wait for it.
        let tracks: Tracks = Arc::new(Mutex::new(Vec::new()));
        if let Some(def) = host.def() {
            let n = def.music.len();
            tracks.lock().map(|mut t| t.resize(n, None)).ok()?;
            let first = host.music_track().min(n.saturating_sub(1));
            let (def, out) = (def.clone(), tracks.clone());
            std::thread::spawn(move || {
                for i in std::iter::once(first).chain((0..n).filter(|&i| i != first)) {
                    let pcm = red2d::sound::music_pcm(&def, i).ok().map(|(p, _)| p);
                    if let Ok(mut t) = out.lock() {
                        t[i] = pcm;
                    }
                }
            });
        }
        Some(Audio { _stream, handle, voices: HashMap::new(), music: None, tracks })
    }

    fn play(&mut self, host: &Host, i: u32) {
        let pcm = self.voices.entry(i).or_insert_with(|| host.sound_pcm(i as usize).ok());
        if let (Some(pcm), Ok(sink)) = (pcm.as_ref(), Sink::try_new(&self.handle)) {
            sink.append(SamplesBuffer::new(1, crate::synth::SAMPLE_RATE, pcm.clone()));
            sink.detach();
        }
    }

    fn set_music(&mut self, host: &Host, want: bool) {
        let track = host.music_track();
        if !want || self.music.as_ref().is_some_and(|(_, t)| *t != track) {
            if let Some((s, _)) = self.music.take() {
                s.stop();
            }
        }
        if !want || self.music.is_some() {
            return;
        }
        // Not synthesized yet: silent for now, asked again next frame.
        let pcm = self.tracks.lock().ok().and_then(|t| t.get(track).cloned().flatten());
        if let (Some(pcm), Ok(sink)) = (pcm, Sink::try_new(&self.handle)) {
            sink.append(SamplesBuffer::new(2, crate::synth::SAMPLE_RATE, pcm).repeat_infinite());
            self.music = Some((sink, track));
        }
    }
}

struct App {
    host: Host,
    title: String,
    save_path: Option<PathBuf>,
    audio: Option<Audio>,
    max_ticks: Option<u64>,
    ticks: u64,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    last: Instant,
    acc: Duration,
    last_save: Instant,
    pointer: Option<[f32; 2]>,
    error: Option<String>,
    fullscreen: bool,
    /// The title card and when it started (the clock starts when the window opens); `None` once the game runs.
    intro: Option<(TitleCard, Instant)>,
    card_px: Vec<u8>,
    /// The first connected gamepad, and which of the game's actions it held last frame (it sends only changes, so the keyboard keeps working beside it).
    pad: Controller,
    pad_held: [bool; PAD_ACTIONS.len()],
    focused: bool,
}

/// The game actions a gamepad drives, in the order `pad_actions` reports them.
const PAD_ACTIONS: [&str; 7] = ["left", "right", "up", "down", "action", "secondary", "pause"];

/// Which game actions a gamepad sample holds (Xbox names; gilrs maps the PlayStation and Nintendo positions): the left stick or the d-pad move, A is `action`
/// (jump, confirm), X, B or the right trigger is `secondary` (run, throw, back), Start or Back is `pause`. Pure, so it is tested without a pad.
fn pad_actions(p: &crate::controller::Sample) -> [bool; PAD_ACTIONS.len()] {
    let m = p.movement;
    [
        m.x < -0.45 || p.down(button::LEFT),
        m.x > 0.45 || p.down(button::RIGHT),
        m.y > 0.6 || p.down(button::UP),
        m.y < -0.6 || p.down(button::DOWN),
        p.down(button::JUMP),
        p.down(button::INTERACT) || p.down(button::CROUCH) || p.down(button::FIRE),
        p.down(button::PAUSE) || p.down(button::VIEW),
    ]
}

/// Where an exported music track goes: the player's Music folder (`%USERPROFILE%\Music` on Windows, `~/Music` elsewhere), in a folder named after the game.
fn music_folder(game_title: &str) -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join("Music").join(file_safe(game_title)))
}

/// A name that is a valid file name everywhere (no path separators or reserved characters), never empty.
fn file_safe(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_'.,()&!".contains(c) { c } else { '_' }).collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() {
        "track".into()
    } else {
        s
    }
}

impl App {
    fn write_save(&mut self) {
        let (Some(text), Some(path)) = (self.host.take_save(), self.save_path.as_deref()) else { return };
        if let Err(e) = write_atomic(path, &text) {
            eprintln!("could not save progress: {e}");
        }
    }

    /// Feeds the gamepad into the game: only the actions whose state changed, so a key held on the keyboard is not released by an idle pad.
    fn poll_pad(&mut self) {
        let sample = self.pad.poll(self.focused);
        self.host.set_gamepad(self.pad.connected());
        let now = pad_actions(&sample);
        for (k, name) in PAD_ACTIONS.iter().enumerate() {
            if now[k] != self.pad_held[k] {
                self.host.action(name, now[k]);
            }
        }
        self.pad_held = now;
    }

    fn toggle_fullscreen(&self) {
        if let Some(w) = &self.window {
            let full = w.fullscreen().is_none();
            w.set_fullscreen(full.then_some(winit::window::Fullscreen::Borderless(None)));
        }
    }

    /// Does what the rules asked (`window`, `save`, `export_music`).
    fn serve_requests(&mut self, el: &ActiveEventLoop) {
        for r in self.host.take_requests() {
            match r {
                Request::Fullscreen => self.toggle_fullscreen(),
                Request::Close => {
                    self.write_save();
                    el.exit();
                }
                Request::Save => {
                    self.write_save();
                    self.last_save = Instant::now();
                }
                Request::ExportMusic(i) => {
                    let event = match self.export_music(i) {
                        Ok(path) => {
                            eprintln!("saved the music to {}", path.display());
                            "music_exported"
                        }
                        Err(e) => {
                            eprintln!("could not save the music: {e}");
                            "music_not_exported"
                        }
                    };
                    self.host.notify(event);
                }
            }
        }
    }

    /// Writes music track `i` as a WAV file named after its title into the player's Music folder; returns where.
    fn export_music(&mut self, i: usize) -> Result<PathBuf, String> {
        let def = self.host.def().ok_or("no game")?.clone();
        let cached = self.audio.as_ref().and_then(|a| a.tracks.lock().ok().and_then(|t| t.get(i).cloned().flatten()));
        let pcm = match cached {
            Some(p) => p,
            None => red2d::sound::music_pcm(&def, i).map(|(p, _)| p)?,
        };
        let title = self.host.music_title(i).unwrap_or_else(|| "track".into());
        let dir = music_folder(&def.title).ok_or("no home folder to put the Music folder in")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(format!("{}.wav", file_safe(&title)));
        std::fs::write(&path, crate::synth::wav_bytes_i16(&pcm, crate::synth::SAMPLE_RATE, 2)).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    /// Everything held is let go (focus lost), and the clock forgets the time that passed.
    fn release_all(&mut self) {
        for a in PAD_ACTIONS {
            self.host.action(a, false);
        }
        self.pad_held = [false; PAD_ACTIONS.len()];
        self.last = Instant::now();
        self.acc = Duration::ZERO;
    }

    /// Shows the title card while it plays; false once it is over (or skipped).
    fn draw_intro(&mut self) -> bool {
        let Some((card, start)) = self.intro.as_mut() else { return false };
        let (Some(window), Some(surface)) = (self.window.as_ref(), self.surface.as_mut()) else { return true };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return true };
        let (cw, ch) = TitleCard::size_for(size.width, size.height);
        if !card.paint(cw, ch, start.elapsed().as_secs_f32(), &mut self.card_px) {
            return false;
        }
        if surface.resize(w, h).is_err() {
            return true;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return true };
        let px = &self.card_px;
        for y in 0..size.height as usize {
            let sy = y * ch as usize / size.height as usize;
            let row = y * size.width as usize;
            for x in 0..size.width as usize {
                let p = (sy * cw as usize + x * cw as usize / size.width as usize) * 4;
                buf[row + x] = u32::from(px[p]) << 16 | u32::from(px[p + 1]) << 8 | u32::from(px[p + 2]);
            }
        }
        window.pre_present_notify();
        let _ = buf.present();
        true
    }

    /// The game begins: the card is gone and the clock starts from now (no burst of catch-up ticks).
    fn end_intro(&mut self) {
        self.intro = None;
        self.last = Instant::now();
        self.acc = Duration::ZERO;
    }

    fn frame(&mut self, el: &ActiveEventLoop) {
        if self.intro.is_some() {
            if self.draw_intro() {
                return;
            }
            self.end_intro();
        }
        let now = Instant::now();
        self.acc += now - self.last;
        self.last = now;
        let mut n = 0;
        while self.acc >= TICK && n < MAX_CATCH_UP {
            self.acc -= TICK;
            n += 1;
        }
        if n == MAX_CATCH_UP {
            self.acc = Duration::ZERO;
        }
        self.poll_pad();
        if n > 0 {
            self.host.step(n);
            self.ticks += u64::from(n);
            self.host.render();
        }
        if let Some(a) = self.audio.as_mut() {
            for i in self.host.take_sounds() {
                a.play(&self.host, i);
            }
            let want = self.host.music_on();
            a.set_music(&self.host, want);
        }
        self.serve_requests(el);
        if self.last_save.elapsed() >= SAVE_EVERY {
            self.last_save = Instant::now();
            self.write_save();
        }
        self.draw();
        if self.max_ticks.is_some_and(|m| self.ticks >= m) {
            el.exit();
        }
    }

    fn draw(&mut self) {
        let (Some(window), Some(surface)) = (self.window.as_ref(), self.surface.as_mut()) else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        let (vw, vh) = self.host.view_size();
        let l = self.host.set_window(size.width, size.height);
        let rgba = self.host.frame();
        buf.fill(0);
        for y in 0..l.h {
            let sy = (u64::from(y) * u64::from(vh) / u64::from(l.h)) as usize;
            let row = (l.y as usize + y as usize) * size.width as usize + l.x as usize;
            for x in 0..l.w {
                let sx = (u64::from(x) * u64::from(vw) / u64::from(l.w)) as usize;
                let p = (sy * vw as usize + sx) * 4;
                buf[row + x as usize] = u32::from(rgba[p]) << 16 | u32::from(rgba[p + 1]) << 8 | u32::from(rgba[p + 2]);
            }
        }
        window.pre_present_notify();
        let _ = buf.present();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (vw, vh) = self.host.view_size();
        let k = (960 / vw.max(1)).min(640 / vh.max(1)).max(1);
        let icon = self.host.icon_rgba(64).and_then(|(w, h, rgba)| winit::window::Icon::from_rgba(rgba, w, h).ok());
        #[allow(unused_mut)]
        let mut attrs = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(LogicalSize::new(vw * k, vh * k))
            .with_min_inner_size(LogicalSize::new(vw.min(320), vh.min(200)))
            .with_window_icon(icon.clone())
            .with_fullscreen(self.fullscreen.then_some(winit::window::Fullscreen::Borderless(None)));
        // Windows shows the small icon in the title bar and the big one in the taskbar and Alt-Tab: give it both.
        #[cfg(windows)]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs = attrs.with_taskbar_icon(icon);
        }
        let made = el.create_window(attrs).map_err(|e| e.to_string()).and_then(|w| {
            let w = Rc::new(w);
            let ctx = softbuffer::Context::new(w.clone()).map_err(|e| e.to_string())?;
            let surface = softbuffer::Surface::new(&ctx, w.clone()).map_err(|e| e.to_string())?;
            Ok((w, surface))
        });
        match made {
            Ok((w, s)) => {
                self.window = Some(w);
                self.surface = Some(s);
                self.last = Instant::now();
                if let Some((_, start)) = self.intro.as_mut() {
                    *start = Instant::now();
                }
            }
            Err(e) => {
                self.error = Some(format!("could not open a window: {e}"));
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        // While the title card plays, a key (other than the fullscreen keys, F and F11) or a click skips it, and no input reaches the game.
        if self.intro.is_some() {
            match &event {
                WindowEvent::KeyboardInput { event: k, .. } if !matches!(k.physical_key, PhysicalKey::Code(KeyCode::F11 | KeyCode::KeyF)) => {
                    if k.state == ElementState::Pressed && !k.repeat {
                        self.end_intro();
                    }
                    return;
                }
                WindowEvent::MouseInput { state, .. } => {
                    if *state == ElementState::Pressed {
                        self.end_intro();
                    }
                    return;
                }
                _ => {}
            }
        }
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Focused(f) => {
                self.focused = f;
                if !f {
                    self.release_all();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let (PhysicalKey::Code(code), false) = (event.physical_key, event.repeat) {
                    let down = event.state == ElementState::Pressed;
                    // F and F11 are the window's in every 2D game (no game reads them).
                    if code == KeyCode::F11 || code == KeyCode::KeyF {
                        if down {
                            self.toggle_fullscreen();
                        }
                    } else {
                        // winit names its key codes after the DOM's `KeyboardEvent.code` ("KeyA", "ArrowLeft", "Space"), which is what the game's rules use.
                        self.host.key(&format!("{code:?}"), down);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = self.host.window_to_view(position.x as f32, position.y as f32);
                if let Some([x, y]) = self.pointer {
                    self.host.pointer(x, y);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some([x, y]) = self.pointer {
                    self.host.click(x, y);
                }
            }
            WindowEvent::RedrawRequested => self.frame(el),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(4)));
            w.request_redraw();
        }
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::Sample;
    use glam::Vec2;

    #[test]
    fn a_gamepad_moves_jumps_runs_and_pauses_a_2d_game() {
        let none = pad_actions(&Sample::default());
        assert_eq!(none, [false; 7], "a pad at rest holds nothing");
        let s = Sample { movement: Vec2::new(-0.8, 0.0), held: button::JUMP | button::INTERACT, ..Default::default() };
        assert_eq!(pad_actions(&s), [true, false, false, false, true, true, false], "stick left, A jumps, X runs");
        let d = Sample { held: button::DOWN | button::PAUSE | button::CROUCH, ..Default::default() };
        assert_eq!(pad_actions(&d), [false, false, false, true, false, true, true], "d-pad down, B is secondary too, Start pauses");
        let small = Sample { movement: Vec2::new(0.3, 0.5), ..Default::default() };
        assert_eq!(pad_actions(&small), [false; 7], "a slight tilt is not a direction");
    }

    #[test]
    fn an_exported_track_gets_a_file_name_that_works_everywhere() {
        assert_eq!(file_safe("Meadow Path"), "Meadow Path");
        assert_eq!(file_safe("Thorns: Act/2?"), "Thorns_ Act_2_");
        assert_eq!(file_safe("  ..  "), "track");
    }
}
