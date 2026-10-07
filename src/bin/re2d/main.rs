//! `re2d`: the native window for a 2D game (`NAME.game2d.json`), the Windows/Linux counterpart of the browser player.
//!
//! It is only the platform half of `red2d::host::Host`: this file feeds in time, keys and the mouse, blits the RGBA picture into a window (softbuffer, no GPU), plays the
//! game's sounds and music through the engine's mixer and keeps the game's save in the user's data folder. All rules, drawing and sound come from the same `Host` the
//! browser, `verify` and `frame` use, so a game that passes `verify` plays the same here.
//!
//! usage: `re2d [GAME.game2d.json]` (with no argument: `game.game2d.json` next to the program, which is how `package2d` ships one). `F11` toggles fullscreen.

use red2d::host::{Host, SAVE_CORRUPT, SAVE_INCOMPATIBLE};
use red_engine2::audio::Audio;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

const TPS: f64 = 60.0;

struct Player {
    host: Host,
    game_id: String,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    audio: Option<Audio>,
    clips: HashMap<u32, Vec<f32>>,
    music_started: bool,
    last: Instant,
    acc: f64,
    cursor: PhysicalPosition<f64>,
    save_path: PathBuf,
}

/// Where a game keeps its save: the platform's per-user data folder, one file per game id.
fn save_path(id: &str) -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("RedEngine2d").join(format!("{id}.save.json"))
}

impl Player {
    fn new(text: &str) -> Result<Player, String> {
        let mut host = Host::default();
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        host.init(text, seed)?;
        let game_id = host.def().map_or_else(|| "game".to_string(), |d| d.id.clone());
        let save_path = save_path(&game_id);
        if let Ok(saved) = std::fs::read_to_string(&save_path) {
            let (code, why) = host.load_save(&saved);
            if code == SAVE_INCOMPATIBLE || code == SAVE_CORRUPT {
                eprintln!("re2d: ignoring the saved progress: {why}");
            }
        }
        Ok(Player {
            host,
            game_id,
            window: None,
            surface: None,
            audio: Audio::new(),
            clips: HashMap::new(),
            music_started: false,
            last: Instant::now(),
            acc: 0.0,
            cursor: PhysicalPosition::new(0.0, 0.0),
            save_path,
        })
    }

    fn pointer_to_view(&self) -> Option<[f32; 2]> {
        self.host.window_to_view(self.cursor.x as f32, self.cursor.y as f32)
    }

    /// One frame: run the ticks the clock owes, play what the game asked for, keep the save, draw.
    fn frame(&mut self) {
        let now = Instant::now();
        self.acc = (self.acc + now.duration_since(self.last).as_secs_f64()).min(0.25);
        self.last = now;
        let n = (self.acc * TPS).floor();
        if n >= 1.0 {
            self.acc -= n / TPS;
            self.host.step(n as u32);
        }
        self.host.render();
        self.play_sounds();
        if let Some(text) = self.host.take_save() {
            if let Some(dir) = self.save_path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&self.save_path, text) {
                eprintln!("re2d: could not save progress to {}: {e}", self.save_path.display());
            }
        }
        self.present();
    }

    fn play_sounds(&mut self) {
        let wanted = self.host.take_sounds();
        let music_on = self.host.music_on();
        let Some(audio) = self.audio.as_mut() else { return };
        if music_on && !self.music_started {
            if let Ok(Some(pcm)) = self.host.music_pcm() {
                audio.start_music(pcm, 1.0);
                self.music_started = true;
            }
        }
        if self.music_started {
            audio.set_music_volume(if music_on { 1.0 } else { 0.0 });
        }
        for i in wanted {
            if !self.clips.contains_key(&i) {
                match self.host.sound_pcm(i as usize) {
                    Ok(pcm) => {
                        self.clips.insert(i, pcm);
                    }
                    Err(e) => eprintln!("re2d: sound {i}: {e}"),
                }
            }
            if let Some(clip) = self.clips.get(&i) {
                audio.play(clip);
            }
        }
    }

    /// Scales the virtual screen (nearest neighbour, as the browser does) into the window with bars where it does not fit.
    fn present(&mut self) {
        let (Some(window), Some(surface)) = (self.window.as_ref(), self.surface.as_mut()) else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if surface.resize(w, h).is_err() {
            return;
        }
        let layout = self.host.set_window(size.width, size.height);
        let (vw, vh) = self.host.view_size();
        let src = self.host.frame();
        let Ok(mut buf) = surface.buffer_mut() else { return };
        buf.fill(0);
        for y in 0..layout.h as usize {
            let wy = layout.y + y as i32;
            if wy < 0 || wy >= size.height as i32 {
                continue;
            }
            let sy = (y * vh as usize / layout.h as usize).min(vh as usize - 1);
            for x in 0..layout.w as usize {
                let wx = layout.x + x as i32;
                if wx < 0 || wx >= size.width as i32 {
                    continue;
                }
                let sx = (x * vw as usize / layout.w as usize).min(vw as usize - 1);
                let p = (sy * vw as usize + sx) * 4;
                buf[wy as usize * size.width as usize + wx as usize] = u32::from(src[p]) << 16 | u32::from(src[p + 1]) << 8 | u32::from(src[p + 2]);
            }
        }
        let _ = buf.present();
    }
}

impl ApplicationHandler for Player {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (vw, vh) = self.host.view_size();
        let title = self.host.def().map_or_else(|| self.game_id.clone(), |d| d.title.clone());
        let scale = 3.0;
        let attrs = Window::default_attributes().with_title(title).with_inner_size(LogicalSize::new(f64::from(vw) * scale, f64::from(vh) * scale));
        let window = match el.create_window(attrs) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("re2d: could not open a window: {e}");
                el.exit();
                return;
            }
        };
        let surface = softbuffer::Context::new(window.clone()).and_then(|c| softbuffer::Surface::new(&c, window.clone()));
        match surface {
            Ok(s) => self.surface = Some(s),
            Err(e) => {
                eprintln!("re2d: could not create a drawing surface: {e}");
                el.exit();
                return;
            }
        }
        self.window = Some(window);
        self.last = Instant::now();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(_) => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let down = event.state == ElementState::Pressed;
                    if code == KeyCode::F11 && down && !event.repeat {
                        if let Some(w) = &self.window {
                            w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) });
                        }
                    } else if !event.repeat {
                        // winit's `KeyCode` names are the browser's `KeyboardEvent.code` names (`KeyA`, `ArrowLeft`, `Space`, `Enter`).
                        self.host.key(&format!("{code:?}"), down);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                if let Some([x, y]) = self.pointer_to_view() {
                    self.host.pointer(x, y);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some([x, y]) = self.pointer_to_view() {
                    self.host.click(x, y);
                }
            }
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

fn main() {
    let arg = std::env::args().nth(1);
    let path =
        arg.map(PathBuf::from).unwrap_or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("game.game2d.json"))).unwrap_or_default());
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("re2d: cannot read the game {}: {e}\nusage: re2d GAME.game2d.json", path.display());
            std::process::exit(2);
        }
    };
    let mut player = match Player::new(&text) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("re2d: {} does not validate:\n{e}\n(run `red_engine2 validate {}` for the way out)", path.display(), path.display());
            std::process::exit(1);
        }
    };
    let el = EventLoop::new().expect("an event loop");
    el.set_control_flow(ControlFlow::Poll);
    if let Err(e) = el.run_app(&mut player) {
        eprintln!("re2d: {e}");
        std::process::exit(1);
    }
}
