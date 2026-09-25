//! Window and pause-menu plumbing: pause overlay, mouse grab, fullscreen, first/third-person toggle, swap-chain frame acquisition.

use super::*;

impl App {
    /// Escape: free the mouse and show the pause menu (Resume / Quit).
    pub(crate) fn enter_pause(&mut self) {
        self.paused = true;
        self.pause_hover = None;
        self.keys.clear(); // no key-release events arrive while the menu has the mouse
        self.sprint_held = false;
        self.set_grab(false);
        self.repaint_pause();
    }

    /// Hides the pause menu and takes the mouse back.
    pub(crate) fn leave_pause(&mut self) {
        self.paused = false;
        self.pause_hover = None;
        self.online.painted = None; // the online overlay (if any) is redrawn next frame
        if let Some(live) = self.gpu.as_mut().and_then(|g| g.live.as_mut()) {
            live.overlay.hide();
        }
        self.set_grab(!self.online.takeover);
    }

    /// Redraws the pause menu overlay (after it opens, the window resizes or the hover changes).
    pub(crate) fn repaint_pause(&mut self) {
        let map = self.scene_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let status = self.net.as_ref().map(|n| n.status.clone());
        let hover = self.pause_hover;
        if let Some(gpu) = self.gpu.as_mut() {
            let (w, h) = (gpu.config.width, gpu.config.height);
            if let Some(live) = gpu.live.as_mut() {
                live.overlay.set(&gpu.device, &gpu.queue, w, h, &menu::paint_pause(w, h, &map, status.as_deref(), hover));
            }
        }
    }

    pub(crate) fn set_grab(&mut self, grabbed: bool) {
        let Some(window) = &self.window else { return };
        if grabbed {
            let ok = window.set_cursor_grab(CursorGrabMode::Locked).is_ok() || window.set_cursor_grab(CursorGrabMode::Confined).is_ok();
            if ok {
                window.set_cursor_visible(false);
                self.grabbed = true;
                self.grabbed_at = Instant::now();
            }
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
            self.grabbed = false;
        }
    }

    /// Toggles borderless fullscreen (covers the whole monitor, no taskbar/decorations) against
    /// the maximized windowed state the app launches in. Not exclusive fullscreen — that
    /// involves a display video-mode switch, which is unnecessary here and would fight the
    /// "fit whatever screen it's on" launch behavior.
    pub(crate) fn toggle_fullscreen(&self) {
        let Some(window) = &self.window else { return };
        if window.fullscreen().is_some() {
            window.set_fullscreen(None);
            window.set_maximized(true);
        } else {
            window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        }
    }

    pub(crate) fn toggle_view_mode(&mut self) {
        self.view_mode = match self.view_mode {
            ViewMode::FirstPerson => ViewMode::ThirdPerson,
            ViewMode::ThirdPerson => ViewMode::FirstPerson,
        };
    }
}

/// Gets the next swapchain frame (reconfiguring the surface if it went stale), plus whether it
/// should be reconfigured after presenting; `None` when there is no frame to draw this time.
pub(crate) fn acquire_frame(
    surface: &wgpu::Surface<'static>,
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> Option<(wgpu::SurfaceTexture, bool)> {
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(t) => Some((t, false)),
        wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some((t, true)),
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Validation => None,
        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
            surface.configure(device, config);
            None
        }
    }
}
