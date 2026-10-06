//! The 3D player for a browser: the engine's own renderer ([`crate::viewer::LiveRenderer`]) and simulation ([`crate::app::session::LocalSession`]) behind a canvas.
//!
//! This is a *platform layer* like `red2d::web` is for 2D games, with the same split of jobs. Rust owns the world: the match, the camera, the picture. The page owns what
//! only a browser has: the animation-frame loop, the keyboard and mouse events, the canvas size. Nothing here is a second renderer or a second simulation, so a scene that
//! plays on the desktop plays here, and a frame drawn here is drawn by the same pipelines (`gpu.rs`, `viewer.rs`, the WGSL in `shaders/`).
//!
//! One player, offline: no network (`net`'s transports are the `native` feature), no threads (the streamed world builds its chunks a few per frame on the one thread there is),
//! no files (the scene text is passed in). Built with `--target wasm32-unknown-unknown --no-default-features --features web` and wrapped by `wasm-bindgen`.

use crate::app::camera::{FpsCamera, ViewCamera};
use crate::app::session::LocalSession;
use crate::sim::player::PlayerInput;
use crate::ui::game;
use crate::viewer::LiveRenderer;
use wasm_bindgen::prelude::*;

/// What is held down, set by the page from `KeyboardEvent.code`.
#[derive(Default, Clone, Copy)]
struct Keys {
    forward: bool,
    back: bool,
    left: bool,
    right: bool,
    sprint: bool,
    jump: bool,
    crouch: bool,
}

/// Which card is up, as in the desktop client: the start card holds the game until its button is used; the end card, after an outcome, has the restart button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Card {
    Start,
    End,
}

/// A running 3D game on a canvas.
#[wasm_bindgen]
pub struct Web3d {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The colour format the world is drawn in (the sRGB view of the surface's format).
    color: wgpu::TextureFormat,
    renderer: LiveRenderer,
    session: LocalSession,
    /// The scene as text, to start the game over from the top.
    scene_text: String,
    card: Option<Card>,
    card_hover: bool,
    /// Pointer position in canvas pixels.
    pointer: (f32, f32),
    /// The page has the game paused (focus left it).
    paused: bool,
    /// What the overlay shows now, to repaint it only when that changes.
    overlay_key: String,
    /// The save last handed to the page, to hand over another only when something changed.
    saved: std::collections::BTreeMap<String, f64>,
    keys: Keys,
    yaw: f32,
    pitch: f32,
    last_ms: Option<f64>,
    info: String,
}

fn js<E: std::fmt::Display>(what: &str) -> impl Fn(E) -> JsValue + '_ {
    move |e| JsValue::from_str(&format!("{what}: {e}"))
}

/// Waits a moment on the page's event loop (a browser resolves GPU buffer maps between tasks, never inside one).
async fn tick() -> Result<(), JsValue> {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 5);
        }
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.map(|_| ())
}

/// Starts the game in `<canvas id=canvas_id>` from the scene JSON text. `saved` is what [`Web3d::save_text`] last returned (empty for a first visit): the variables the scene keeps
/// between sessions (Marcel's days) come back before the first tick. WebGPU only: without it the error says so.
#[wasm_bindgen]
pub async fn web3d_start(canvas_id: &str, scene_text: &str, saved: &str) -> Result<Web3d, JsValue> {
    console_error_panic_hook::set_once();
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let canvas: web_sys::HtmlCanvasElement = document.get_element_by_id(canvas_id).ok_or("no such canvas")?.dyn_into()?;
    let (w, h) = (canvas.width().max(1), canvas.height().max(1));
    let mut session = LocalSession::from_json(scene_text).map_err(|e| JsValue::from_str(&e.join("\n")))?;
    let saved: std::collections::BTreeMap<String, f64> = serde_json::from_str::<std::collections::BTreeMap<String, f64>>(saved).unwrap_or_default();
    session.restore_vars(&saved);

    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::BROWSER_WEBGPU, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas)).map_err(js("surface"))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        })
        .await
        .map_err(|_| JsValue::from_str("this game needs WebGPU, which this browser does not have (try a current Chrome, Edge or Safari)"))?;
    let ai = adapter.get_info();
    let info = format!("{:?} | {} | {:?}", ai.backend, ai.name, ai.device_type);
    let limits = wgpu::Limits::default();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor { label: Some("web3d-device"), required_limits: limits, ..Default::default() })
        .await
        .map_err(js("no GPU device"))?;

    // The world is lit in linear light and must be written through an sRGB curve, as on the desktop. A canvas offers an sRGB format directly (WebGL2) or only the plain one
    // (WebGPU), where a view of the same texture adds the curve; a device that can do neither draws the plain format (darker, and the page is told).
    let caps = surface.get_capabilities(&adapter);
    let direct = caps.formats.iter().copied().find(|f| f.is_srgb());
    let view_formats_ok = adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS);
    let (base, color, views) = match direct {
        Some(f) => (f, f, vec![]),
        None => {
            let base = caps.formats.first().copied().ok_or("the canvas reports no formats")?;
            if view_formats_ok {
                (base, base.add_srgb_suffix(), vec![base.add_srgb_suffix()])
            } else {
                (base, base, vec![])
            }
        }
    };
    let info = if color.is_srgb() { info } else { format!("{info} (no sRGB output: darker than intended)") };
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: base,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: w,
        height: h,
        present_mode: wgpu::PresentMode::AutoVsync,
        desired_maximum_frame_latency: 2,
        alpha_mode: caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto),
        view_formats: views,
    };
    surface.configure(&device, &config);
    let renderer = LiveRenderer::world(&device, color, session.scene(), w, h);
    let yaw = session.player().yaw;
    let card = session.scene().ui.as_ref().is_some_and(|u| u.start.is_some()).then_some(Card::Start);
    Ok(Web3d {
        device,
        queue,
        surface,
        config,
        color,
        renderer,
        session,
        scene_text: scene_text.to_string(),
        card,
        card_hover: false,
        pointer: (0.0, 0.0),
        paused: false,
        overlay_key: String::new(),
        saved,
        keys: Keys::default(),
        yaw,
        pitch: 0.0,
        last_ms: None,
        info,
    })
}

#[wasm_bindgen]
impl Web3d {
    /// `backend | adapter | device type`, for the page and the test.
    pub fn info(&self) -> String {
        self.info.clone()
    }

    /// The canvas changed size.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
        self.renderer.resize(&self.device, self.config.width, self.config.height);
    }

    /// A key went down or up (`KeyboardEvent.code`).
    pub fn key(&mut self, code: &str, down: bool) {
        if self.card.is_some() {
            // No key-up arrives for a key held through a card, so nothing stays held across one; the card's button answers to Enter, Space and E.
            self.keys = Keys::default();
            if down && matches!(code, "Enter" | "NumpadEnter" | "Space" | "KeyE") {
                self.card_activate();
            }
            return;
        }
        match code {
            "KeyW" | "ArrowUp" => self.keys.forward = down,
            "KeyS" | "ArrowDown" => self.keys.back = down,
            "KeyA" | "ArrowLeft" => self.keys.left = down,
            "KeyD" | "ArrowRight" => self.keys.right = down,
            "ShiftLeft" | "ShiftRight" => self.keys.sprint = down,
            "Space" => self.keys.jump = down,
            "KeyC" | "ControlLeft" => self.keys.crouch = down,
            _ => {}
        }
    }

    /// The mouse (or a finger) turned the head by `(dx, dy)` pixels.
    pub fn look(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * 0.0022;
        self.pitch = (self.pitch - dy * 0.0022).clamp(-FpsCamera::PITCH_LIMIT, FpsCamera::PITCH_LIMIT);
    }

    /// Where the player stands: `x, foot y, z`.
    pub fn position(&self) -> Vec<f32> {
        let p = self.session.player_feet();
        vec![p.x, p.y, p.z]
    }

    /// Simulation ticks run so far.
    pub fn ticks(&self) -> f64 {
        self.session.tick() as f64
    }

    /// Chunks of the streamed world resident, as `loaded`, or -1 for a scene without one.
    pub fn chunks(&self) -> i32 {
        self.renderer.stream_draw_stats().map_or(-1, |s| s.resident as i32)
    }

    /// The pointer moved over the canvas (canvas pixels): highlights the card's button.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.pointer = (x, y);
        let over = self.card_layout().is_some_and(|l| l.button_at(x, y).is_some());
        self.card_hover = over;
    }

    /// A click at `(x, y)` (canvas pixels): uses the card's button if that is where it landed. Returns whether it did (the page then takes the mouse for looking).
    pub fn pointer_click(&mut self, x: f32, y: f32) -> bool {
        self.pointer = (x, y);
        if self.card_layout().is_some_and(|l| l.button_at(x, y).is_some()) {
            return self.card_activate();
        }
        false
    }

    /// Runs `ticks` simulation ticks with the keys as they are now, without drawing (a card or a pause still holds the game): scripted play, and what tests use so they do not
    /// depend on how fast a GPU is. Returns the ticks run so far.
    pub fn step(&mut self, ticks: u32) -> f64 {
        if self.card.is_none() && !self.paused {
            let input = self.input();
            for _ in 0..ticks {
                self.session.step(input);
            }
        }
        self.session.tick() as f64
    }

    /// The page paused or resumed the game (the tab lost focus, the mouse was let go): the simulation waits.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if paused {
            self.keys = Keys::default();
        }
    }

    /// Whether a card (start or end) is up.
    pub fn card_up(&self) -> bool {
        self.card.is_some()
    }

    /// What the game keeps between sessions as JSON text, when it differs from what this last returned; the page stores it and hands it back to `web3d_start` next time.
    pub fn save_text(&mut self) -> Option<String> {
        let now = self.session.persisted();
        if now.is_empty() || now == self.saved {
            return None;
        }
        self.saved = now.clone();
        serde_json::to_string(&now).ok()
    }

    fn card_content(&self) -> Option<(crate::ui_config::Card, &'static str)> {
        let ui = self.session.scene().ui.as_ref()?;
        let hud = self.session.hud();
        let vars: Vec<(&str, f64)> = hud.vars.iter().map(|(n, v)| (n.as_str(), *v)).collect();
        match self.card? {
            Card::Start => ui.start_card(&vars).map(|c| (c, "start")),
            Card::End => ui.filled_end_card(self.session.rules().ended()?, &vars).map(|c| (c, "restart")),
        }
    }

    fn card_layout(&self) -> Option<crate::ui::Layout> {
        let (card, id) = self.card_content()?;
        Some(game::card_layout(self.config.width, self.config.height, &card, id, self.card_hover))
    }

    /// The card's button was used. False when the card has none.
    fn card_activate(&mut self) -> bool {
        if !self.card_content().is_some_and(|(c, _)| c.button.is_some()) {
            return false;
        }
        match self.card {
            Some(Card::Start) => self.card = None,
            Some(Card::End) => {
                // Play again from the top, keeping what the game keeps between sessions (the days lived).
                if let Ok(mut fresh) = LocalSession::from_json(&self.scene_text) {
                    fresh.restore_vars(&self.session.persisted());
                    self.session = fresh;
                    self.yaw = self.session.player().yaw;
                    self.pitch = 0.0;
                    self.last_ms = None;
                    self.card = None;
                }
            }
            None => return false,
        }
        self.card_hover = false;
        true
    }

    /// Paints the overlay (a card, or the rules HUD) when what it shows has changed.
    fn sync_overlay(&mut self) {
        let (w, h) = (self.config.width, self.config.height);
        let hud = self.session.hud();
        let key = format!("{:?}|{}|{w}x{h}|{}", self.card, self.card_hover, hud.key());
        if key == self.overlay_key {
            return;
        }
        self.overlay_key = key;
        let layout = match self.card_layout() {
            Some(card) => card,
            None => {
                let vars: Vec<(&str, f64)> = hud.vars.iter().map(|(n, v)| (n.as_str(), *v)).collect();
                // While a card is up the HUD is hidden behind it; with none, the HUD of the scene's `ui` block, or the generic panel.
                game::rules_overlay(self.session.scene(), w, h, &vars, hud.event.as_deref(), hud.outcome.as_deref(), false)
            }
        };
        if layout.widgets.is_empty() {
            self.renderer.overlay.hide();
        } else {
            self.renderer.overlay.set(&self.device, &self.queue, w, h, &layout.paint().px);
        }
    }

    fn input(&self) -> PlayerInput {
        let k = self.keys;
        PlayerInput {
            forward: i8::from(k.forward) - i8::from(k.back),
            strafe: i8::from(k.right) - i8::from(k.left),
            jump: k.jump,
            sprint: k.sprint,
            crouch: k.crouch,
            yaw: self.yaw,
            pitch: self.pitch,
            ..Default::default()
        }
    }

    fn camera(&self) -> ViewCamera {
        let p = self.session.player_feet();
        let body = self.session.player().character.body();
        let mut cam = FpsCamera::new(p + glam::Vec3::Y * body.stand_eye, 0.0);
        cam.yaw = self.yaw;
        cam.pitch = self.pitch;
        cam.fov_deg = self.session.scene().player.fov_deg;
        cam.near = body.near_plane;
        cam.far = self.session.scene().camera.far;
        cam.view()
    }

    /// One animation frame at `now_ms` (`performance.now()`): runs the whole simulation ticks the elapsed time covers, then draws.
    pub fn frame(&mut self, now_ms: f64) -> Result<(), JsValue> {
        let dt = self.last_ms.map_or(0.0, |l| ((now_ms - l) / 1000.0).clamp(0.0, 0.1)) as f32;
        self.last_ms = Some(now_ms);
        let input = self.input();
        if self.card.is_none() && !self.paused {
            self.session.advance(dt, |_| input);
        }
        // An outcome with an end card declared for it puts the card up, as the desktop client does.
        if self.card.is_none() {
            if let (Some(ui), Some(outcome)) = (self.session.scene().ui.as_ref(), self.session.rules().ended()) {
                if ui.end_card(outcome).is_some() {
                    self.card = Some(Card::End);
                    self.keys = Keys::default();
                }
            }
        }
        self.sync_overlay();
        self.renderer.set_hidden_objects(self.session.hidden().map(str::to_string).collect::<Vec<_>>().iter().map(String::as_str));
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            _ => return Ok(()),
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(self.color), ..Default::default() });
        let t = (self.session.tick() as f32 + self.session.alpha()) / crate::sim::clock::TICK_RATE_HZ as f32;
        let cam = self.camera();
        self.renderer.render_view(&self.device, &self.queue, self.session.scene(), t, &cam, &view, None);
        self.queue.present(frame);
        Ok(())
    }

    /// Draws one frame into a texture and reads the pixels back (RGBA, row-major, `width * height * 4` bytes): what a test compares, independent of whether the browser
    /// composites a canvas the way a screenshot can see. `settle` first builds every chunk of a streamed world, so the picture is the whole of it (a player's frames
    /// fill the world in over a second or two instead).
    pub async fn snapshot(&mut self, settle: bool) -> Result<Vec<u8>, JsValue> {
        let (w, h) = (self.config.width, self.config.height);
        self.sync_overlay();
        let cam = self.camera();
        if settle {
            self.renderer.settle_stream(&self.device, &self.queue, cam.eye);
        }
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("web3d-snapshot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.color,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let t = self.session.tick() as f32 / crate::sim::clock::TICK_RATE_HZ as f32;
        self.renderer.render_view(&self.device, &self.queue, self.session.scene(), t, &cam, &view, None);
        let row = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let out = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &out, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        out.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r.is_ok());
        });
        let mut mapped = false;
        for _ in 0..8000 {
            let _ = self.device.poll(wgpu::PollType::Poll);
            if let Ok(ok) = rx.try_recv() {
                mapped = ok;
                break;
            }
            tick().await?;
        }
        if !mapped {
            return Err(JsValue::from_str("the GPU never finished the snapshot"));
        }
        let view = out.slice(..).get_mapped_range().map_err(js("map"))?;
        let bgra = matches!(self.color, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            for px in view[y * row as usize..][..(w * 4) as usize].as_chunks::<4>().0 {
                rgba.extend_from_slice(&if bgra { [px[2], px[1], px[0], 255] } else { [px[0], px[1], px[2], 255] });
            }
        }
        Ok(rgba)
    }
}
