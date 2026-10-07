//! The native 2D player: a `*.game2d.json` game in an OS window, with sound and a save file.
//!
//! [`red2d::host::Host`] is the whole game as plain calls (ticks in, RGBA frame / sound requests / save text out); this module is the platform around it: a `winit` window,
//! `softbuffer` to show the CPU-rendered frame, `rodio` for the synthesized sounds and music, and a fixed 60 Hz tick clock. The game's rules, physics and picture are the
//! same code the headless checks run, so what `verify` proves is what is played.

use red2d::host::{Host, SAVE_LOADED};
use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::rc::Rc;
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
    let audio = if opts.mute { None } else { Audio::open() };
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
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.write_save();
    app.error.map_or(Ok(()), Err)
}

/// Sound effects and one looping music track. Any failure to reach a sound device just means a silent game.
struct Audio {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    voices: HashMap<u32, Option<Vec<f32>>>,
    music: Option<Sink>,
    music_pcm: Option<Option<Vec<f32>>>,
}

impl Audio {
    fn open() -> Option<Audio> {
        let (_stream, handle) = OutputStream::try_default().ok()?;
        Some(Audio { _stream, handle, voices: HashMap::new(), music: None, music_pcm: None })
    }

    fn play(&mut self, host: &Host, i: u32) {
        let pcm = self.voices.entry(i).or_insert_with(|| host.sound_pcm(i as usize).ok());
        if let (Some(pcm), Ok(sink)) = (pcm.as_ref(), Sink::try_new(&self.handle)) {
            sink.append(SamplesBuffer::new(1, crate::synth::SAMPLE_RATE, pcm.clone()));
            sink.detach();
        }
    }

    fn set_music(&mut self, host: &Host, want: bool) {
        if !want {
            if let Some(s) = self.music.take() {
                s.stop();
            }
            return;
        }
        if self.music.is_some() {
            return;
        }
        let pcm = self.music_pcm.get_or_insert_with(|| host.music_pcm().ok().flatten());
        if let (Some(pcm), Ok(sink)) = (pcm.as_ref(), Sink::try_new(&self.handle)) {
            sink.append(SamplesBuffer::new(2, crate::synth::SAMPLE_RATE, pcm.clone()).repeat_infinite());
            self.music = Some(sink);
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
}

impl App {
    fn write_save(&mut self) {
        let (Some(text), Some(path)) = (self.host.take_save(), self.save_path.as_deref()) else { return };
        if let Err(e) = write_atomic(path, &text) {
            eprintln!("could not save progress: {e}");
        }
    }

    /// Everything held is let go (focus lost), and the clock forgets the time that passed.
    fn release_all(&mut self) {
        for a in ["left", "right", "up", "down", "action", "secondary", "pause"] {
            self.host.action(a, false);
        }
        self.last = Instant::now();
        self.acc = Duration::ZERO;
    }

    fn frame(&mut self, el: &ActiveEventLoop) {
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
        let attrs = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(LogicalSize::new(vw * k, vh * k))
            .with_min_inner_size(LogicalSize::new(vw.min(320), vh.min(200)));
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
            }
            Err(e) => {
                self.error = Some(format!("could not open a window: {e}"));
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Focused(false) => self.release_all(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let (PhysicalKey::Code(code), false) = (event.physical_key, event.repeat) {
                    let down = event.state == ElementState::Pressed;
                    if down && code == KeyCode::F11 {
                        if let Some(w) = &self.window {
                            let full = w.fullscreen().is_none();
                            w.set_fullscreen(full.then_some(winit::window::Fullscreen::Borderless(None)));
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
