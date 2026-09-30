//! The program around a match: the window, the home menu, match setup, hosting and joining, the stats file, and the overlay painting.
//!
//! [`Kc`] knows nothing about winit except in its `ApplicationHandler` impl: it is driven one frame at a time (`frame`) with its input
//! handed in as plain calls (`on_key`, `on_click`, ...), which is how the scripted `KC_SCRIPT` runs play it without a window and take pictures.

use super::game::{Connection, Env, Game, Stage};
use super::input::Controls;
use glam::{Mat4, Vec3};
use red_engine2::audio::Audio;
use red_engine2::capture::Capture;
use red_engine2::net::bot::ClientWorld;
use red_engine2::net::client::ClientConfig;
use red_engine2::net::host::{HostOptions, LocalHost, PublicOptions};
use red_engine2::net::join_code::JoinCode;
use red_engine2::schema::Scene;
use red_engine2::sfx::{KitSounds, SoundBank};
use red_engine2::sim::spawns::parse_spawns;
use red_engine2::stats::{self, Stats, Store};
use red_engine2::ui::killchain as ui;
use red_engine2::ui::online::{ConnectForm, Field};
use red_engine2::ui::{Canvas, Kind, Layout};
use red_engine2::viewer::{FpsCamera, FrameOptions, LiveRenderer};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// What the command line asked of the game.
pub(crate) struct Options {
    /// The map.
    pub scene: PathBuf,
    /// Start in borderless fullscreen.
    pub fullscreen: bool,
    /// The player's name, if given.
    pub name: Option<String>,
}

/// The graphics device the game draws with.
pub struct GpuCtx {
    /// The device.
    pub device: wgpu::Device,
    /// Its queue.
    pub queue: wgpu::Queue,
    /// The colour format of the target.
    pub format: wgpu::TextureFormat,
    /// The size of the target in pixels.
    pub size: (u32, u32),
}

struct WindowState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    Home,
    Setup { hosting: bool },
    Join,
    Stats,
    InGame,
}

/// The program.
pub struct Kc {
    opts: Options,
    gpu: Option<GpuCtx>,
    win: Option<WindowState>,
    capture: Option<Capture>,
    menu_scene: Scene,
    menu_live: Option<LiveRenderer>,
    orbit: (Vec3, f32, f32),
    audio: Option<Audio>,
    sounds: SoundBank,
    kit_sounds: KitSounds,
    ambient_started: bool,
    store: Option<Store>,
    stats: Stats,
    last_save: Instant,
    screen: Screen,
    setup: ui::Setup,
    join_form: ConnectForm,
    note: Option<String>,
    hover: Option<String>,
    cursor: (f32, f32),
    painted: Option<u64>,
    game: Option<Game>,
    host: Option<LocalHost>,
    controls: Controls,
    started: Instant,
    last_frame: Instant,
    focused: bool,
    grabbed: bool,
    quit: bool,
    controller: red_engine2::controller::Controller,
    home_note: Option<String>,
    lost_since: Option<Instant>,
    fullscreen_request: bool,
    shots: Vec<(f32, String)>,
    shot_dir: PathBuf,
    sim_secs: f32,
}

fn layout_hash(l: &Layout, extra: u64) -> u64 {
    let mut h = DefaultHasher::new();
    (l.w, l.h, extra).hash(&mut h);
    for w in &l.widgets {
        (&w.id, w.rect, &w.text, w.scale, w.color, w.fill, w.frame.map(|f| (f.0, f.1))).hash(&mut h);
    }
    h.finish()
}

/// Composites `src` over `dst` (both the same size).
fn blend_over(dst: &mut Canvas, src: &Canvas) {
    for y in 0..src.h {
        for x in 0..src.w {
            let i = ((y * src.w + x) * 4) as usize;
            if src.px[i + 3] > 0 {
                dst.blend(x, y, [src.px[i], src.px[i + 1], src.px[i + 2], src.px[i + 3]]);
            }
        }
    }
}

impl Kc {
    fn new(opts: Options) -> Kc {
        let map_text = std::fs::read_to_string(&opts.scene).unwrap_or_default();
        let (menu_scene, _) = ClientWorld::load(&opts.scene).unwrap_or_else(|e| {
            eprintln!("cannot load {}: {e}", opts.scene.display());
            std::process::exit(1);
        });
        // The menu's camera circles the middle of the spawn points.
        let spawns = parse_spawns(&map_text).unwrap_or_default();
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for s in &spawns {
            let p = Vec3::from(s.position);
            min = min.min(p);
            max = max.max(p);
        }
        let center = if spawns.is_empty() { Vec3::ZERO } else { (min + max) * 0.5 };
        let radius = if spawns.is_empty() { 40.0 } else { ((max - min).length() * 0.55).clamp(30.0, 90.0) };
        let store = Store::default_location();
        let mut stats = store.as_ref().map(Store::load).unwrap_or_default();
        stats.sessions += 1;
        let now = stats::unix_now();
        if stats.first_played == 0 {
            stats.first_played = now;
        }
        stats.last_played = now;
        let name = opts
            .name
            .clone()
            .or_else(|| Some(stats.name.clone()).filter(|n| !n.is_empty()))
            .or_else(|| std::env::var("USERNAME").ok().or_else(|| std::env::var("USER").ok()))
            .unwrap_or_else(|| "Player".to_string());
        stats.name = name.clone();
        let setup = ui::Setup { name: name.clone(), ..Default::default() };
        let join_form = ConnectForm::new("", "", &name);
        Kc {
            opts,
            gpu: None,
            win: None,
            capture: None,
            menu_scene,
            menu_live: None,
            orbit: (center, radius, 0.0),
            audio: Audio::new(),
            sounds: SoundBank::new(),
            kit_sounds: KitSounds::new(),
            ambient_started: false,
            store,
            stats,
            last_save: Instant::now(),
            screen: Screen::Home,
            setup,
            join_form,
            note: None,
            hover: None,
            cursor: (0.0, 0.0),
            painted: None,
            game: None,
            host: None,
            controls: Controls::default(),
            started: Instant::now(),
            last_frame: Instant::now(),
            focused: true,
            grabbed: false,
            quit: false,
            controller: Default::default(),
            home_note: None,
            lost_since: None,
            fullscreen_request: false,
            shots: Vec::new(),
            shot_dir: PathBuf::from("out/kc"),
            sim_secs: 0.0,
        }
    }

    // ---- graphics --------------------------------------------------------------------------------------------------------------

    fn build_menu_renderer(&mut self) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        self.menu_live = Some(LiveRenderer::world(&gpu.device, gpu.format, &self.menu_scene, gpu.size.0, gpu.size.1));
    }

    fn resize(&mut self, w: u32, h: u32) {
        let (w, h) = (w.max(1), h.max(1));
        if let (Some(gpu), Some(win)) = (self.gpu.as_mut(), self.win.as_mut()) {
            win.config.width = w;
            win.config.height = h;
            win.surface.configure(&gpu.device, &win.config);
            gpu.size = (w, h);
        }
        if let Some(gpu) = self.gpu.as_ref() {
            if let Some(m) = self.menu_live.as_mut() {
                m.resize(&gpu.device, w, h);
            }
            if let Some(g) = self.game.as_mut() {
                g.resize(gpu);
            }
        }
        self.painted = None;
    }

    // ---- the overlay -----------------------------------------------------------------------------------------------------------

    /// The layout the current screen shows (`None` when nothing is drawn over the frame).
    fn current_layout(&self) -> Option<Layout> {
        let (w, h) = self.gpu.as_ref()?.size;
        let hover = self.hover.as_deref();
        Some(match self.screen {
            Screen::Home => {
                let mut l = ui::home_layout(w, h, hover, env!("CARGO_PKG_VERSION"));
                if let Some(n) = &self.home_note {
                    let s = (h as i32 / 240).max(1);
                    let x1 = l.rect_of("panel").map_or(w as i32 / 3, |r| r.2);
                    l.label_fit("home_note", None, x1 / 2, h as i32 / 2 + 50 * s, &n.to_uppercase(), s, x1 - 8 * s, [255, 170, 120, 255]);
                }
                l
            }
            Screen::Setup { hosting } => ui::setup_layout(w, h, if hosting { "host a game" } else { "solo" }, &self.setup, self.note.as_deref(), hover),
            Screen::Join => ui::join_layout(w, h, &self.join_form, hover),
            Screen::Stats => ui::stats_layout(w, h, &self.stats, hover),
            Screen::InGame => {
                let g = self.game.as_ref()?;
                if g.paused {
                    return Some(ui::pause_layout(w, h, hover, g.hosting));
                }
                match g.stage() {
                    Stage::Connecting => {
                        let mut l = Layout::new(w, h);
                        let s = (h as i32 / 240).max(1);
                        l.label("connecting", None, w as i32 / 2, h as i32 / 2, "CONNECTING...", s * 3, [236, 238, 240, 255]);
                        l
                    }
                    Stage::Lost => {
                        let mut l = Layout::new(w, h);
                        let s = (h as i32 / 240).max(1);
                        l.label_fit("lost", None, w as i32 / 2, h as i32 / 2, &g.lost_reason(), s * 2, w as i32 - 16 * s, [255, 170, 120, 255]);
                        l
                    }
                    Stage::Lobby | Stage::Watching => ui::lobby_layout(w, h, &g.lobby_view(), hover),
                    Stage::Results => ui::results_layout(w, h, &g.board_view(true), hover),
                    Stage::Countdown | Stage::Playing => {
                        if let Some(k) = g.killcam_view() {
                            ui::killcam_layout(w, h, &k)
                        } else if self.controls.scoreboard() {
                            let mut l = ui::hud_layout(w, h, &g.hud_view());
                            let board = ui::scoreboard_layout(w, h, &g.board_view(false));
                            let offset = l.widgets.len();
                            for mut widget in board.widgets {
                                widget.container = widget.container.map(|i| i + offset);
                                l.widgets.push(widget);
                            }
                            l
                        } else {
                            ui::hud_layout(w, h, &g.hud_view())
                        }
                    }
                }
            }
        })
    }

    /// Paints the overlay when its content changed.
    fn paint_overlay(&mut self) {
        let Some(layout) = self.current_layout() else { return };
        let scoped = self.game.as_ref().is_some_and(|g| self.screen == Screen::InGame && g.scoped());
        let hash = layout_hash(&layout, scoped as u64);
        if self.painted == Some(hash) {
            return;
        }
        self.painted = Some(hash);
        let Some(gpu) = self.gpu.as_ref() else { return };
        let mut canvas = Canvas::new(layout.w as u32, layout.h as u32);
        if scoped {
            ui::paint_scope(&mut canvas);
        }
        blend_over(&mut canvas, &layout.paint());
        let live = match (&mut self.game, self.screen) {
            (Some(g), Screen::InGame) => Some(g.live_mut()),
            _ => self.menu_live.as_mut(),
        };
        if let Some(live) = live {
            live.overlay.set(&gpu.device, &gpu.queue, layout.w as u32, layout.h as u32, &canvas.px);
        }
    }

    // ---- actions ---------------------------------------------------------------------------------------------------------------

    fn go_home(&mut self, note: Option<String>) {
        if let Some(mut g) = self.game.take() {
            g.leave();
        }
        self.host = None;
        self.screen = Screen::Home;
        self.home_note = note;
        self.painted = None;
        self.hover = None;
        self.controls.release_all();
        self.save_stats();
    }

    fn save_stats(&mut self) {
        self.stats.name = self.setup.name.clone();
        if let Some(store) = &self.store {
            if let Err(e) = store.save(&self.stats) {
                eprintln!("cannot save the stats: {e}");
            }
        }
        self.last_save = Instant::now();
    }

    fn clean_name(&self) -> String {
        let n = self.setup.name.trim();
        if n.is_empty() {
            "Player".to_string()
        } else {
            n.to_string()
        }
    }

    /// SOLO and HOST: starts a server on this machine (for friends, too, when hosting) and joins it.
    fn start_hosted(&mut self, hosting: bool) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        let skill = ui::SKILLS.get(self.setup.skill as usize).map_or("normal", |s| s.1);
        let public = hosting.then(|| PublicOptions {
            identity_dir: stats::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("host"),
            port: red_engine2::net::DEFAULT_PORT,
            upnp: true,
            key: red_engine2::net::auth::random_key().ok().map(|k| k[..8].to_string()),
        });
        let opts = HostOptions {
            fill: Some(if self.setup.bots { 12 } else { 0 }),
            bot_skill: Some(skill.to_string()),
            kill_limit: Some(self.setup.kill_limit as u32),
            round_secs: Some(self.setup.minutes as f32 * 60.0),
            public,
            ..Default::default()
        };
        self.note = Some("STARTING...".to_string());
        let host = match LocalHost::start(&self.opts.scene, &opts) {
            Ok(h) => h,
            Err(e) => {
                self.note = Some(e);
                return;
            }
        };
        let mut cfg = ClientConfig::new(host.addr(), 0, 0, 0);
        match host.local_transport() {
            Ok(t) => cfg.transport = t,
            Err(e) => {
                self.note = Some(e);
                return;
            }
        }
        cfg.join_key = host.key().map(str::to_string);
        let name = self.clean_name();
        match Game::start(gpu, &self.opts.scene, Connection { cfg, team: 0 }, true, &name) {
            Ok(mut game) => {
                game.join_codes = host.join_codes().iter().map(JoinCode::format).collect();
                game.notice = host.upnp_note().map(str::to_string);
                self.game = Some(game);
                self.host = Some(host);
                self.screen = Screen::InGame;
                self.note = None;
                self.painted = None;
                self.hover = None;
            }
            Err(e) => self.note = Some(e),
        }
    }

    /// JOIN: parses the code and connects.
    fn try_join(&mut self) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        let code = match JoinCode::parse(&self.join_form.address) {
            Ok(c) => c,
            Err(e) => {
                self.join_form.message = Some(e);
                return;
            }
        };
        let Some(addr) = std::net::ToSocketAddrs::to_socket_addrs(&code.address).ok().and_then(|mut i| i.next()) else {
            self.join_form.message = Some(format!("Cannot find '{}'. Check the code and your internet connection.", code.address));
            return;
        };
        let mut cfg = ClientConfig::new(addr, 0, 0, 0);
        cfg.join_key = code.key.clone();
        cfg.transport = match red_engine2::net::client::ClientTransportConfig::choose(addr, code.fingerprint.as_deref(), None, Some("localhost"), false) {
            Ok(t) => t,
            Err(_) => {
                self.join_form.message =
                    Some("That code has no identity part: ask your friend for the whole code (they can copy it from the lobby).".to_string());
                return;
            }
        };
        self.setup.name = self.join_form.name.clone();
        let name = self.clean_name();
        self.join_form.message = Some("CONNECTING...".to_string());
        match Game::start(gpu, &self.opts.scene, Connection { cfg, team: 0 }, false, &name) {
            Ok(game) => {
                self.game = Some(game);
                self.screen = Screen::InGame;
                self.painted = None;
                self.hover = None;
                self.join_form.message = None;
            }
            Err(e) => self.join_form.message = Some(e),
        }
    }

    fn click(&mut self, id: &str) {
        match self.screen {
            Screen::Home => match ui::home_action(id) {
                Some(ui::HomeAction::Solo) => {
                    self.screen = Screen::Setup { hosting: false };
                    self.note = Some("Bots fill both teams to six a side.".to_string());
                }
                Some(ui::HomeAction::Host) => {
                    self.screen = Screen::Setup { hosting: true };
                    self.note = Some("Friends get a join code in the lobby.".to_string());
                }
                Some(ui::HomeAction::Join) => {
                    self.screen = Screen::Join;
                    self.join_form.focus = Field::Address;
                }
                Some(ui::HomeAction::Stats) => self.screen = Screen::Stats,
                Some(ui::HomeAction::Quit) => self.quit = true,
                None => {}
            },
            Screen::Setup { hosting } => match ui::setup_action(id) {
                Some(ui::SetupAction::Bots(on)) => self.setup.bots = on,
                Some(ui::SetupAction::Skill(s)) if self.setup.bots => self.setup.skill = s,
                Some(ui::SetupAction::Skill(_)) => {}
                Some(ui::SetupAction::Kills(k)) => self.setup.kill_limit = k,
                Some(ui::SetupAction::Minutes(m)) => self.setup.minutes = m,
                Some(ui::SetupAction::Name) => self.setup.typing = true,
                Some(ui::SetupAction::Start) => {
                    self.setup.typing = false;
                    self.start_hosted(hosting);
                }
                Some(ui::SetupAction::Back) => {
                    self.screen = Screen::Home;
                    self.setup.typing = false;
                    self.note = None;
                }
                None => {}
            },
            Screen::Join => match ui::join_action(id) {
                Some(ui::JoinAction::Code) => self.join_form.focus = Field::Address,
                Some(ui::JoinAction::Name) => self.join_form.focus = Field::Name,
                Some(ui::JoinAction::Paste) => {
                    if let Some(t) = red_engine2::clipboard::get_text() {
                        self.join_form.address = t.chars().filter(|c| !c.is_control()).take(200).collect();
                        self.join_form.message = None;
                    } else {
                        self.join_form.message = Some("Nothing to paste. Copy the code first, or type it.".to_string());
                    }
                }
                Some(ui::JoinAction::Connect) => self.try_join(),
                Some(ui::JoinAction::Back) => self.screen = Screen::Home,
                None => {}
            },
            Screen::Stats => {
                if id == "back" {
                    self.screen = Screen::Home;
                }
            }
            Screen::InGame => self.click_in_game(id),
        }
        self.painted = None;
    }

    fn click_in_game(&mut self, id: &str) {
        let Some(game) = self.game.as_mut() else { return };
        if game.paused {
            match ui::pause_choice(id) {
                Some(ui::PauseChoice::Resume) => game.paused = false,
                Some(ui::PauseChoice::Fullscreen) => self.fullscreen_request = true,
                Some(ui::PauseChoice::Home) => self.go_home(None),
                None => {}
            }
            return;
        }
        let now = Instant::now();
        match game.stage() {
            Stage::Lobby | Stage::Watching => match ui::lobby_action(id) {
                Some(ui::LobbyAction::Team(t)) => game.net.client.set_team(t, now),
                Some(ui::LobbyAction::Ready) => {
                    let want = !game.net.client.is_ready();
                    game.net.client.set_ready(want, now);
                }
                Some(ui::LobbyAction::Leave) => self.go_home(None),
                None => {}
            },
            Stage::Results => match ui::results_action(id) {
                Some(ui::ResultsAction::PlayAgain) => {
                    let want = !game.net.client.is_ready();
                    game.net.client.set_ready(want, now);
                }
                Some(ui::ResultsAction::Home) => self.go_home(None),
                None => {}
            },
            _ => {}
        }
    }

    // ---- input -----------------------------------------------------------------------------------------------------------------

    fn on_hover(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
        let hover = self.current_layout().and_then(|l| l.button_at(x, y).map(str::to_string));
        if hover != self.hover {
            self.hover = hover;
            self.painted = None;
        }
    }

    fn on_click_at(&mut self) {
        if let Some(id) = self.current_layout().and_then(|l| l.button_at(self.cursor.0, self.cursor.1).map(str::to_string)) {
            self.click(&id);
        }
    }

    fn typing_target(&mut self) -> Option<&mut String> {
        match self.screen {
            Screen::Setup { .. } if self.setup.typing => Some(&mut self.setup.name),
            Screen::Join => Some(match self.join_form.focus {
                Field::Name => &mut self.join_form.name,
                _ => &mut self.join_form.address,
            }),
            _ => None,
        }
    }

    fn on_key(&mut self, code: KeyCode, text: Option<&str>, pressed: bool, repeat: bool) {
        if !pressed {
            self.controls.key_up(code);
            return;
        }
        let ctrl = self.controls.keys.contains(&KeyCode::ControlLeft) || self.controls.keys.contains(&KeyCode::ControlRight);
        if matches!(code, KeyCode::ControlLeft | KeyCode::ControlRight) {
            self.controls.keys.insert(code);
            return;
        }
        if code == KeyCode::F11 && !repeat {
            self.fullscreen_request = true;
            return;
        }
        // Typing.
        if self.typing_target().is_some() {
            match code {
                KeyCode::Backspace => {
                    if let Some(t) = self.typing_target() {
                        t.pop();
                    }
                }
                KeyCode::Escape => self.setup.typing = false,
                KeyCode::Enter | KeyCode::NumpadEnter => {
                    if self.screen == Screen::Join {
                        self.try_join();
                    } else {
                        self.setup.typing = false;
                    }
                }
                KeyCode::Tab if self.screen == Screen::Join => {
                    self.join_form.focus = if self.join_form.focus == Field::Name { Field::Address } else { Field::Name };
                }
                KeyCode::KeyV if ctrl => {
                    if let Some(clip) = red_engine2::clipboard::get_text() {
                        let max =
                            if self.screen == Screen::Join && self.join_form.focus == Field::Address { 200 } else { red_engine2::net::protocol::MAX_NAME };
                        if let Some(t) = self.typing_target() {
                            t.extend(clip.chars().filter(|c| !c.is_control()).take(max));
                            t.truncate(max);
                        }
                    }
                }
                _ => {
                    let max = if self.screen == Screen::Join && self.join_form.focus == Field::Address { 200 } else { red_engine2::net::protocol::MAX_NAME };
                    if let (Some(text), false) = (text, ctrl) {
                        if let Some(t) = self.typing_target() {
                            for c in text.chars().filter(|c| !c.is_control()) {
                                if t.len() < max {
                                    t.push(c);
                                }
                            }
                        }
                    }
                }
            }
            self.painted = None;
            return;
        }
        if repeat {
            return;
        }
        match self.screen {
            Screen::InGame => self.key_in_game(code),
            _ => {
                if code == KeyCode::Escape && self.screen != Screen::Home {
                    self.screen = Screen::Home;
                    self.painted = None;
                }
            }
        }
    }

    fn key_in_game(&mut self, code: KeyCode) {
        let Some(game) = self.game.as_mut() else { return };
        if code == KeyCode::Escape {
            match game.stage() {
                Stage::Playing | Stage::Countdown => {
                    game.paused = !game.paused;
                    self.controls.release_all();
                    self.painted = None;
                }
                Stage::Lost | Stage::Connecting => self.go_home(None),
                _ => {
                    game.paused = !game.paused;
                    self.painted = None;
                }
            }
            return;
        }
        if game.paused {
            return;
        }
        match game.stage() {
            Stage::Lobby | Stage::Watching => match code {
                KeyCode::Digit1 => self.click("team_1"),
                KeyCode::Digit2 => self.click("team_2"),
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => self.click("ready"),
                _ => {}
            },
            Stage::Results => {
                if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
                    self.click("again");
                }
            }
            Stage::Playing | Stage::Countdown => self.controls.key_down(code),
            _ => {}
        }
    }

    /// Gamepad navigation of the menus: the D-pad moves between buttons, A presses, B goes back, Start pauses.
    fn pad_menu(&mut self) {
        use red_engine2::controller::button as b;
        let p = self.controls.pad;
        if p.pressed == 0 {
            return;
        }
        let Some(layout) = self.current_layout() else { return };
        let buttons: Vec<&str> = layout.widgets.iter().filter(|w| w.kind == Kind::Button).map(|w| w.id.as_str()).collect();
        if buttons.is_empty() {
            return;
        }
        let current = self.hover.as_deref().and_then(|h| buttons.iter().position(|b| *b == h)).unwrap_or(0);
        let step = if p.hit(b::DOWN) || p.hit(b::RIGHT) || p.hit(b::NEXT) {
            1
        } else if p.hit(b::UP) || p.hit(b::LEFT) || p.hit(b::PREVIOUS) {
            buttons.len() - 1
        } else {
            0
        };
        if step > 0 {
            let id = buttons[(current + step) % buttons.len()].to_string();
            self.hover = Some(id);
            self.painted = None;
            return;
        }
        if p.hit(b::JUMP) {
            let id = buttons[current].to_string();
            self.click(&id);
        } else if p.hit(b::CROUCH) {
            for back in ["back", "leave", "home", "resume"] {
                if buttons.contains(&back) {
                    self.click(back);
                    break;
                }
            }
        } else if p.hit(b::PAUSE) && self.screen == Screen::InGame {
            self.key_in_game(KeyCode::Escape);
        }
    }

    // ---- the frame -------------------------------------------------------------------------------------------------------------

    fn start_ambience(&mut self) {
        if self.ambient_started {
            return;
        }
        self.ambient_started = true;
        if let Some(audio) = self.audio.as_mut() {
            // The natural sound of the map: wind, a far-off machine, now and then a distant clank. No music.
            audio.start_music(red_engine2::sfx::ambience(28.0), 0.12);
        }
    }

    /// Advances everything by `dt` and draws into `target` (a swapchain frame or the capture target).
    fn frame(&mut self, dt: f32, target: Option<&wgpu::TextureView>) {
        self.stats.time_in_game_secs += dt as f64;
        self.sim_secs += dt;
        self.start_ambience();
        if self.last_save.elapsed().as_secs() > 30 {
            self.save_stats();
        }
        // The gamepad.
        self.controls.pad = self.controller.poll(self.focused);
        if self.screen == Screen::InGame && self.game.as_ref().is_some_and(|g| !g.paused && matches!(g.stage(), Stage::Playing | Stage::Countdown)) {
            self.controls.apply_pad();
            if self.controls.pad.hit(red_engine2::controller::button::PAUSE) {
                self.key_in_game(KeyCode::Escape);
            }
        } else {
            self.pad_menu();
        }
        // The match.
        if self.screen == Screen::InGame {
            if let (Some(game), Some(gpu)) = (self.game.as_mut(), self.gpu.as_ref()) {
                let aspect = gpu.size.0 as f32 / gpu.size.1.max(1) as f32;
                let mut env = Env { audio: self.audio.as_ref(), sounds: &self.sounds, kit_sounds: &self.kit_sounds, stats: &mut self.stats, aspect };
                game.frame(dt, &mut self.controls, &mut env);
                if game.stage() == Stage::Lost {
                    let since = *self.lost_since.get_or_insert_with(Instant::now);
                    if since.elapsed().as_secs_f32() > 3.0 {
                        let why = game.lost_reason();
                        self.lost_since = None;
                        self.go_home(Some(why));
                    }
                } else {
                    self.lost_since = None;
                }
            }
        }
        // Mouse capture follows what is on screen.
        let want_grab = self.screen == Screen::InGame && self.game.as_ref().is_some_and(|g| !g.wants_cursor());
        if want_grab != self.grabbed {
            self.set_grab(want_grab);
        }
        if std::mem::take(&mut self.fullscreen_request) {
            self.toggle_fullscreen();
        }
        self.paint_overlay();
        // Draw.
        let Some(gpu) = self.gpu.as_ref() else { return };
        let Some(target) = target else { return };
        match (&mut self.game, self.screen) {
            (Some(g), Screen::InGame) => g.draw(gpu, target),
            _ => {
                let Some(live) = self.menu_live.as_mut() else { return };
                let (center, radius, _) = self.orbit;
                let angle = self.started.elapsed().as_secs_f32() * 0.06;
                let eye = center + Vec3::new(angle.cos() * radius, radius * 0.35 + 6.0, angle.sin() * radius);
                let d = center + Vec3::Y * 2.0 - eye;
                let mut cam = FpsCamera::new(eye, 0.0);
                cam.yaw = d.x.atan2(-d.z);
                cam.pitch = (d.y / (d.x * d.x + d.z * d.z).sqrt().max(0.1)).atan();
                cam.fov_deg = 70.0;
                cam.far = 600.0;
                let hidden = Mat4::from_scale(Vec3::splat(0.0005));
                live.render_ex(
                    &gpu.device,
                    &gpu.queue,
                    &self.menu_scene,
                    angle * 10.0,
                    &cam,
                    target,
                    false,
                    hidden,
                    hidden,
                    FrameOptions { crosshair: false, viewmodel: false, ..FrameOptions::default() },
                );
            }
        }
    }

    fn set_grab(&mut self, grab: bool) {
        self.grabbed = grab;
        if !grab {
            self.controls.fire = false;
            self.controls.aim = false;
        }
        let Some(win) = self.win.as_ref() else { return };
        if grab {
            let ok = win.window.set_cursor_grab(CursorGrabMode::Locked).is_ok() || win.window.set_cursor_grab(CursorGrabMode::Confined).is_ok();
            if ok {
                win.window.set_cursor_visible(false);
            } else {
                self.grabbed = false;
            }
        } else {
            let _ = win.window.set_cursor_grab(CursorGrabMode::None);
            win.window.set_cursor_visible(true);
        }
    }

    fn toggle_fullscreen(&mut self) {
        let Some(win) = self.win.as_ref() else { return };
        if win.window.fullscreen().is_some() {
            win.window.set_fullscreen(None);
            win.window.set_maximized(true);
        } else {
            win.window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(win.window.current_monitor())));
        }
    }

    /// Flushes the record before the program ends.
    fn finish(&mut self) {
        if let Some(mut g) = self.game.take() {
            g.leave();
        }
        self.host = None;
        self.save_stats();
    }
}

impl ApplicationHandler for Kc {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.win.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("Killchain").with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)).with_maximized(true);
        let window = Arc::new(event_loop.create_window(attrs).expect("failed to create window"));
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone()).expect("failed to create GPU surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no compatible GPU adapter found (Killchain needs Vulkan, DX12 or Metal)");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("killchain-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .expect("failed to create GPU device");
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        self.gpu = Some(GpuCtx { device, queue, format, size: (config.width, config.height) });
        self.win = Some(WindowState { window: window.clone(), surface, config });
        self.build_menu_renderer();
        if self.opts.fullscreen {
            self.fullscreen_request = true;
        }
        self.last_frame = Instant::now();
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.finish();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => self.resize(size.width, size.height),
            WindowEvent::Focused(f) => {
                self.focused = f;
                if !f {
                    self.controls.release_all();
                    if let Some(g) = self.game.as_mut() {
                        if matches!(g.stage(), Stage::Playing) && !g.paused && !self.host.is_some() {
                            // A match on someone else's server keeps running; we just let go of the keys.
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if !self.grabbed {
                    self.on_hover(position.x as f32, position.y as f32);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                if self.grabbed {
                    match button {
                        MouseButton::Left => self.controls.fire = down,
                        MouseButton::Right => {
                            self.controls.aim = down;
                            self.controls.aim_pressed |= down;
                        }
                        _ => {}
                    }
                } else if down && button == MouseButton::Left {
                    self.on_click_at();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if self.grabbed {
                    self.controls.wheel += lines;
                    self.controls.apply_wheel();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let text = event.text.as_ref().map(|t| t.as_str());
                    self.on_key(code, text, event.state == ElementState::Pressed, event.repeat);
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = now.duration_since(self.last_frame).as_secs_f32().clamp(0.0, 0.1);
                self.last_frame = now;
                let frame = match self.win.as_ref() {
                    Some(w) => match w.surface.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
                        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                            if let Some(g) = self.gpu.as_ref() {
                                w.surface.configure(&g.device, &w.config);
                            }
                            None
                        }
                        _ => None,
                    },
                    None => None,
                };
                let view = frame.as_ref().map(|f| f.texture.create_view(&wgpu::TextureViewDescriptor::default()));
                self.frame(dt, view.as_ref());
                if let (Some(frame), Some(g)) = (frame, self.gpu.as_ref()) {
                    g.queue.present(frame);
                }
                if self.quit {
                    self.finish();
                    event_loop.exit();
                    return;
                }
                if let Some(w) = &self.win {
                    w.window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.grabbed {
                self.controls.mouse.0 += delta.0 as f32;
                self.controls.mouse.1 += delta.1 as f32;
            }
        }
    }
}

// ---- headless, scripted runs -------------------------------------------------------------------------------------------------------

/// A scripted run (`KC_SCRIPT=file.json`): no window; the game is played by the script and pictures are taken offscreen. The script is
/// `{"size": [w, h], "secs": n, "steps": [{"at": 1.5, "do": "click", "id": "solo"}, ...]}`; steps: `click` (a button id of the current screen),
/// `type` (text), `key` (`"W"`, `"Space"`, ... with `down`: true/false), `mouse` (dx, dy pixels), `fire`/`aim` (down), `screen`, `shot` (name),
/// `look` (yaw, pitch degrees), `say` (print).
fn run_script(mut kc: Kc, script_path: &str) {
    let text = std::fs::read_to_string(script_path).unwrap_or_else(|e| {
        eprintln!("cannot read {script_path}: {e}");
        std::process::exit(2);
    });
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("{script_path}: {e}");
        std::process::exit(2);
    });
    let size = v["size"].as_array().map(|a| (a[0].as_u64().unwrap_or(1280) as u32, a[1].as_u64().unwrap_or(720) as u32)).unwrap_or((1280, 720));
    let secs = v["secs"].as_f64().unwrap_or(20.0) as f32;
    kc.shot_dir = PathBuf::from(v["out"].as_str().unwrap_or("out/kc"));
    let gpu = red_engine2::gpu::Gpu::new().unwrap_or_else(|e| {
        eprintln!("no graphics device: {e}");
        std::process::exit(2);
    });
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    kc.capture = Some(Capture::new(&gpu.device, format, size.0, size.1).expect("capture target"));
    kc.gpu = Some(GpuCtx { device: gpu.device, queue: gpu.queue, format, size });
    kc.build_menu_renderer();
    let mut steps: Vec<serde_json::Value> = v["steps"].as_array().cloned().unwrap_or_default();
    steps.sort_by(|a, b| a["at"].as_f64().unwrap_or(0.0).total_cmp(&b["at"].as_f64().unwrap_or(0.0)));
    let mut next = 0;
    let fixed_dt = v["fixed_dt"].as_f64().map(|d| d as f32);
    let mut t = 0.0f32;
    let start = Instant::now();
    let mut frame_no = 0u64;
    // Real time passes between frames so the network and the simulation thread advance like in a real session.
    let frame_pause = std::time::Duration::from_micros((v["pace_ms"].as_f64().unwrap_or(16.0) * 1000.0) as u64);
    while t < secs && !kc.quit {
        while next < steps.len() && steps[next]["at"].as_f64().unwrap_or(0.0) as f32 <= t {
            let step = steps[next].clone();
            next += 1;
            script_step(&mut kc, &step);
        }
        let view = kc.capture.as_ref().map(|c| c.view().clone());
        // Real time passes between frames (a software renderer is slow), so the game and the server thread stay in step.
        let dt = fixed_dt.unwrap_or_else(|| kc.last_frame.elapsed().as_secs_f32().clamp(0.001, 0.1));
        kc.last_frame = Instant::now();
        kc.frame(dt, view.as_ref());
        t += dt;
        frame_no += 1;
        if let Some(pos) = kc.shots.iter().position(|(at, _)| *at <= t) {
            let (_, name) = kc.shots.remove(pos);
            take_shot(&mut kc, &name);
        }
        std::thread::sleep(frame_pause);
    }
    println!("script finished: {frame_no} frames in {:.1}s, screen {:?}", start.elapsed().as_secs_f32(), kc.screen);
    if let Some(g) = kc.game.as_ref() {
        println!("stage {:?}, frames {}", g.stage(), g.frames());
    }
    println!(
        "stats: kills {} deaths {} shots {} hits {} rounds {}",
        kc.stats.kills, kc.stats.deaths, kc.stats.shots_fired, kc.stats.shots_hit, kc.stats.rounds_played
    );
    kc.finish();
}

fn script_key(name: &str) -> Option<KeyCode> {
    Some(match name.to_ascii_uppercase().as_str() {
        "W" => KeyCode::KeyW,
        "A" => KeyCode::KeyA,
        "S" => KeyCode::KeyS,
        "D" => KeyCode::KeyD,
        "R" => KeyCode::KeyR,
        "E" => KeyCode::KeyE,
        "G" => KeyCode::KeyG,
        "Q" => KeyCode::KeyQ,
        "C" => KeyCode::KeyC,
        "TAB" => KeyCode::Tab,
        "SPACE" => KeyCode::Space,
        "ESC" => KeyCode::Escape,
        "ENTER" => KeyCode::Enter,
        "1" => KeyCode::Digit1,
        "2" => KeyCode::Digit2,
        "3" => KeyCode::Digit3,
        "4" => KeyCode::Digit4,
        "CTRL" => KeyCode::ControlLeft,
        _ => return None,
    })
}

fn script_step(kc: &mut Kc, step: &serde_json::Value) {
    let what = step["do"].as_str().unwrap_or("");
    match what {
        "click" => {
            let id = step["id"].as_str().unwrap_or("");
            kc.click(id);
        }
        "type" => {
            let text = step["text"].as_str().unwrap_or("");
            if matches!(kc.screen, Screen::Setup { .. }) {
                kc.setup.typing = true;
            }
            for c in text.chars() {
                kc.on_key(KeyCode::KeyA, Some(&c.to_string()), true, false);
            }
        }
        "key" => {
            if let Some(code) = step["name"].as_str().and_then(script_key) {
                let down = step["down"].as_bool().unwrap_or(true);
                if down {
                    // In a match the key reaches the controls; on a menu it is a key press.
                    kc.on_key(code, None, true, false);
                } else {
                    kc.on_key(code, None, false, false);
                }
            }
        }
        "mouse" => {
            kc.controls.mouse.0 += step["dx"].as_f64().unwrap_or(0.0) as f32;
            kc.controls.mouse.1 += step["dy"].as_f64().unwrap_or(0.0) as f32;
        }
        "fire" => kc.controls.fire = step["down"].as_bool().unwrap_or(true),
        "aim" => {
            let down = step["down"].as_bool().unwrap_or(true);
            kc.controls.aim = down;
            kc.controls.aim_pressed |= down;
        }
        "screen" => {
            kc.screen = match step["name"].as_str().unwrap_or("home") {
                "stats" => Screen::Stats,
                "join" => Screen::Join,
                "solo" => Screen::Setup { hosting: false },
                "host" => Screen::Setup { hosting: true },
                _ => Screen::Home,
            };
            kc.painted = None;
        }
        "set" => {
            if let Some(b) = step["bots"].as_bool() {
                kc.setup.bots = b;
            }
            if let Some(k) = step["kills"].as_u64() {
                kc.setup.kill_limit = k as u16;
            }
            if let Some(m) = step["minutes"].as_u64() {
                kc.setup.minutes = m as u16;
            }
            if let Some(s) = step["skill"].as_u64() {
                kc.setup.skill = s as u8;
            }
        }
        "look" => {
            if let Some(g) = kc.game.as_mut() {
                g.set_look(step["yaw"].as_f64().map(|v| v as f32), step["pitch"].as_f64().map(|v| v as f32));
            }
        }
        "shot" => {
            let name = step["name"].as_str().unwrap_or("shot").to_string();
            kc.shots.push((0.0, name));
        }
        "say" => println!("[script] {}", step["text"].as_str().unwrap_or("")),
        "state" => {
            if let Some(g) = kc.game.as_ref() {
                println!("[state] {}", g.debug_state());
            }
        }
        "quit" => kc.quit = true,
        _ => eprintln!("[script] unknown step {what}"),
    }
}

fn take_shot(kc: &mut Kc, name: &str) {
    let (Some(gpu), Some(capture)) = (kc.gpu.as_ref(), kc.capture.as_ref()) else { return };
    match capture.read_rgba(&gpu.device, &gpu.queue) {
        Ok(img) => {
            let path = kc.shot_dir.join(format!("{name}.png"));
            match red_engine2::capture::save_png(&img, &path) {
                Ok(()) => println!("[shot] {}", path.display()),
                Err(e) => eprintln!("[shot] {e}"),
            }
        }
        Err(e) => eprintln!("[shot] {e}"),
    }
}

/// Runs the game: a window, or `KC_SCRIPT=file.json` for a scripted run with no window.
pub(crate) fn run(opts: Options) {
    let kc = Kc::new(opts);
    if let Ok(script) = std::env::var("KC_SCRIPT") {
        run_script(kc, &script);
        return;
    }
    let mut kc = kc;
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut kc).expect("event loop error");
    // Tearing down the graphics and audio devices should take a moment; if anything wedges, the game still ends.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(4));
        std::process::exit(0);
    });
    drop(kc);
}
