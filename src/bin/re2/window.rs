//! Window and pause-menu plumbing: pause overlay, mouse grab, fullscreen, first/third-person toggle (frame acquisition is the shared
//! `red_engine2::app::WindowGpu`).

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
        self.rule_hud.invalidate(); // likewise for the offline rules HUD
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
            let (w, h) = (gpu.win.config.width, gpu.win.config.height);
            if let Some(live) = gpu.live.as_mut() {
                live.overlay.set(&gpu.win.device, &gpu.win.queue, w, h, &menu::paint_pause(w, h, &map, status.as_deref(), hover));
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
