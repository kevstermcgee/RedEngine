//! Window and pause-menu plumbing: pause overlay, mouse grab, fullscreen, first/third-person toggle, swap-chain frame acquisition.

use super::*;

impl App {
    /// Escape: free the mouse and show the pause menu (Resume / Quit).
    pub(crate) fn enter_pause(&mut self) {
        self.paused = true;
        self.set_host_paused(true);
        self.pause_hover = None;
        self.keys.clear(); // no key-release events arrive while the menu has the mouse
        self.sprint_held = false;
        self.set_grab(false);
        self.repaint_pause();
    }

    /// Hides the pause menu and takes the mouse back.
    pub(crate) fn leave_pause(&mut self) {
        self.paused = false;
        self.set_host_paused(false);
        self.pause_hover = None;
        self.online.painted = None; // the online overlay (if any) is redrawn next frame
        self.rule_hud_painted = None; // likewise for the offline rules HUD
        if let Some(live) = self.gpu.as_mut().and_then(|g| g.live.as_mut()) {
            live.overlay.hide();
        }
        self.set_grab(!self.online.takeover);
    }

    /// Freezes or thaws the match this game hosts (nothing to do when it plays on another machine's server).
    fn set_host_paused(&self, paused: bool) {
        if let Some(flag) = &self.host_pause {
            flag.store(paused, std::sync::atomic::Ordering::Relaxed);
        }
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
        if !grabbed {
            self.pad = Default::default();
            self.controller.reset();
            self.attack_held = false;
            self.attack_queued = false;
            self.ads_held = false;
            self.net_pulse[1] = 0;
        }
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

    /// Whether the window is in borderless fullscreen right now (asked of the window, not remembered, so it cannot drift from what the
    /// desktop actually did: Windows can leave fullscreen by itself on a monitor change or Alt-Tab).
    pub(crate) fn is_fullscreen(&self) -> bool {
        self.window.as_ref().is_some_and(|w| w.fullscreen().is_some())
    }

    /// `F` / `F11` / the pause menu's button: borderless fullscreen (covers the whole monitor it is on, no taskbar or decorations) against
    /// the maximized windowed state the game opens in. Not exclusive fullscreen: that switches the display's video mode, which is
    /// unnecessary here and fights the "fit whatever screen it is on" launch behavior.
    pub(crate) fn toggle_fullscreen(&mut self) {
        self.set_fullscreen(!self.is_fullscreen());
    }

    /// Enters or leaves borderless fullscreen (a no-op when already there). Changing the window mode makes Windows drop the cursor grab,
    /// so a grabbed game is flagged to take the mouse back as soon as the resize that follows arrives (`regrab`, see `Resized`).
    pub(crate) fn set_fullscreen(&mut self, on: bool) {
        let Some(window) = &self.window else { return };
        if window.fullscreen().is_some() == on {
            return;
        }
        if on {
            window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(window.current_monitor())));
        } else {
            window.set_fullscreen(None);
            window.set_maximized(true);
        }
        if self.grabbed {
            self.regrab = true;
        }
        self.online.painted = None;
        self.rule_hud_painted = None;
        if self.paused {
            self.repaint_pause();
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
