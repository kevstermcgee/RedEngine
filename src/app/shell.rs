//! The smallest window loop a custom client needs: open a window, collect input, let the game update, draw its scene
//! from its camera with rule-hidden objects removed and its HUD on top, repeat. The game supplies data through
//! [`ClientGame`]; it does not touch `winit` or `wgpu`. Anything this loop does not do (menus, networking, audio) the
//! game can still do itself with the public modules, or it can build its own loop from [`WindowGpu`],
//! [`InputState`], [`LiveRenderer`] and [`HudPainter`] exactly as `re2` does.

use crate::app::camera::ViewCamera;
use crate::app::gpu::WindowGpu;
use crate::app::input::InputState;
use crate::overlay::Overlay;
use crate::schema::Scene;
use crate::ui::Layout;
use crate::viewer::LiveRenderer;
use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// This frame's timing and window size.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    /// Seconds since the previous frame (capped at 0.1 s so a stall does not jump the game).
    pub dt: f32,
    /// Window width, pixels.
    pub width: u32,
    /// Window height, pixels.
    pub height: u32,
}

/// What a custom client provides to [`run`]. Only [`scene`](Self::scene), [`update`](Self::update) and
/// [`camera`](Self::camera) are required.
pub trait ClientGame {
    /// The scene to draw. Its objects are uploaded on the first frame; move or hide them afterwards (tracks are
    /// sampled every frame) rather than adding new ones.
    fn scene(&self) -> &Scene;

    /// Reads input and advances (or observes) gameplay. Return `false` to close the window.
    fn update(&mut self, input: &InputState, frame: Frame) -> bool;

    /// The camera to draw with this frame.
    fn camera(&self, width: u32, height: u32) -> ViewCamera;

    /// Object ids not to draw (for rules: `LocalSession::hidden`).
    fn hidden(&self) -> Vec<String> {
        Vec::new()
    }

    /// Scene animation time for keyframed objects, seconds.
    fn scene_time(&self) -> f32 {
        0.0
    }

    /// A string that changes whenever the HUD would look different (e.g. `HudState::key`); the HUD is repainted only
    /// then or on resize.
    fn hud_key(&self) -> String {
        String::new()
    }

    /// The HUD for a `width` x `height` window (`None` = no HUD). Build it with `crate::ui` or `HudState::layout`.
    fn hud(&self, _width: u32, _height: u32) -> Option<Layout> {
        None
    }

    /// The window title; checked every frame, applied when it changes.
    fn title(&self) -> String {
        "Red Engine".to_string()
    }
}

/// Window settings for [`run`].
#[derive(Debug, Clone)]
pub struct WindowOptions {
    /// Initial inner size, logical pixels.
    pub size: (u32, u32),
    /// Optional outer position, physical pixels (tiling windows for demos).
    pub position: Option<(i32, i32)>,
}

impl Default for WindowOptions {
    fn default() -> Self {
        WindowOptions { size: (1280, 720), position: None }
    }
}

/// Repaints a HUD overlay only when its key or the window size changed (painting is CPU work per pixel).
#[derive(Debug, Default)]
pub struct HudPainter {
    painted: Option<(u32, u32, String)>,
}

impl HudPainter {
    /// Shows the layout `build` returns in `overlay` if `key` or the size changed since the last call; hides the overlay
    /// for `None` or an empty layout.
    pub fn show(
        &mut self,
        overlay: &mut Overlay,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        key: &str,
        build: impl FnOnce() -> Option<Layout>,
    ) {
        if self.painted.as_ref().is_some_and(|(w, h, k)| (*w, *h) == (width, height) && k == key) {
            return;
        }
        match build() {
            Some(layout) if !layout.widgets.is_empty() => overlay.set(device, queue, width, height, &layout.paint().px),
            _ => overlay.hide(),
        }
        self.painted = Some((width, height, key.to_string()));
    }

    /// Forces a repaint next time (after something else drew into the overlay).
    pub fn invalidate(&mut self) {
        self.painted = None;
    }
}

struct Shell<G: ClientGame> {
    game: G,
    options: WindowOptions,
    window: Option<Arc<Window>>,
    gpu: Option<WindowGpu>,
    renderer: Option<LiveRenderer>,
    input: InputState,
    hud: HudPainter,
    title: String,
    last: Instant,
    error: Option<anyhow::Error>,
}

impl<G: ClientGame> Shell<G> {
    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let Some(gpu) = self.gpu.as_ref() else { return };
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        let (width, height) = gpu.size();
        if !self.game.update(&self.input, Frame { dt, width, height }) {
            event_loop.exit();
            return;
        }
        self.input.end_frame();
        let title = self.game.title();
        if title != self.title {
            if let Some(w) = &self.window {
                w.set_title(&title);
            }
            self.title = title;
        }
        let renderer = self.renderer.get_or_insert_with(|| LiveRenderer::world(&gpu.device, gpu.format(), self.game.scene(), width, height));
        let hidden = self.game.hidden();
        renderer.set_hidden_objects(hidden.iter().map(String::as_str));
        let game = &self.game;
        self.hud.show(&mut renderer.overlay, &gpu.device, &gpu.queue, width, height, &game.hud_key(), || game.hud(width, height));
        let Some((frame, reconfigure)) = gpu.acquire() else { return };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer.render_view(&gpu.device, &gpu.queue, game.scene(), game.scene_time(), &game.camera(width, height), &view, None);
        gpu.present(frame, reconfigure);
    }
}

impl<G: ClientGame> ApplicationHandler for Shell<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(self.options.size.0 as f64, self.options.size.1 as f64));
        if let Some((x, y)) = self.options.position {
            attrs = attrs.with_position(winit::dpi::PhysicalPosition::new(x, y));
        }
        let created = event_loop.create_window(attrs).map_err(anyhow::Error::from).and_then(|w| {
            let w = Arc::new(w);
            WindowGpu::new(w.clone()).map(|g| (w, g))
        });
        match created {
            Ok((window, gpu)) => {
                window.request_redraw();
                self.window = Some(window);
                self.gpu = Some(gpu);
                self.last = Instant::now();
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                    let (w, h) = gpu.size();
                    if let Some(r) = self.renderer.as_mut() {
                        r.resize(&gpu.device, w, h);
                    }
                }
                self.hud.invalidate();
            }
            WindowEvent::RedrawRequested => {
                self.frame(event_loop);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => self.input.on_event(&event),
        }
    }
}

/// Opens a window and runs `game` until it returns `false` from `update` or the window is closed.
pub fn run<G: ClientGame>(game: G, options: WindowOptions) -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let title = game.title();
    let mut shell = Shell {
        game,
        options,
        window: None,
        gpu: None,
        renderer: None,
        input: InputState::new(),
        hud: HudPainter::default(),
        title,
        last: Instant::now(),
        error: None,
    };
    event_loop.run_app(&mut shell)?;
    shell.error.map_or(Ok(()), Err)
}
