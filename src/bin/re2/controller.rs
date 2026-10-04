//! Route native gamepad actions through the existing gameplay and menu actions.
use super::*;
use red_engine2::controller::button as b;

impl App {
    pub(crate) fn poll_controller(&mut self, dt: f32, event_loop: &ActiveEventLoop) {
        self.pad = self.controller.poll(self.focused);
        let p = self.pad;
        if !self.focused {
            return;
        }
        if self.map_selection.is_some() {
            self.pad = Default::default();
            for (button, key) in [
                (b::UP, KeyCode::ArrowUp),
                (b::DOWN, KeyCode::ArrowDown),
                (b::PREVIOUS, KeyCode::ArrowLeft),
                (b::NEXT, KeyCode::ArrowRight),
                (b::JUMP, KeyCode::Enter),
                (b::CROUCH, KeyCode::Escape),
            ] {
                if p.hit(button) {
                    self.map_key(key);
                    break;
                }
            }
            return;
        }
        if self.phase == Phase::Connect {
            self.pad = Default::default();
            if p.hit(b::CROUCH) {
                self.close_connect();
            }
            if p.hit(b::JUMP) {
                self.try_connect();
            }
            if p.hit(b::DOWN) || p.hit(b::UP) {
                self.online.form.next_field();
            }
            return;
        }
        if self.paused {
            self.pad = Default::default();
            if p.hit(b::RELOAD) {
                self.open_maps();
                return;
            }
            // Top to bottom as the menu actually lays the buttons out (`ui::screens::pause_layout`); the arrow
            // (save the music) gets its own stop so it is reachable without a mouse.
            const ORDER: [PauseAction; 6] =
                [PauseAction::Resume, PauseAction::ToggleMusic, PauseAction::DownloadMusic, PauseAction::ToggleSfx, PauseAction::Fullscreen, PauseAction::Quit];
            let index = self.pause_hover.and_then(|h| ORDER.iter().position(|a| *a == h)).unwrap_or(0);
            if p.hit(b::UP) {
                self.pause_hover = Some(ORDER[index.saturating_sub(1)]);
                self.repaint_pause();
            }
            if p.hit(b::DOWN) {
                self.pause_hover = Some(ORDER[(index + 1).min(ORDER.len() - 1)]);
                self.repaint_pause();
            }
            if p.hit(b::JUMP) {
                match self.pause_hover {
                    Some(PauseAction::ToggleMusic) => {
                        self.toggle_music();
                        self.repaint_pause();
                    }
                    Some(PauseAction::DownloadMusic) => self.download_music(),
                    Some(PauseAction::ToggleSfx) => {
                        self.toggle_sfx();
                        self.repaint_pause();
                    }
                    Some(PauseAction::Fullscreen) => self.toggle_fullscreen(),
                    Some(PauseAction::Quit) => event_loop.exit(),
                    Some(PauseAction::Resume) | None => {
                        self.leave_pause();
                        self.controller.reset();
                        self.pad = Default::default();
                    }
                }
            } else if p.hit(b::PAUSE) || p.hit(b::CROUCH) {
                self.leave_pause();
                self.controller.reset();
                self.pad = Default::default();
            }
            return;
        }
        if self.online.takeover {
            self.pad = Default::default();
            if p.hit(b::JUMP) {
                self.online_key(KeyCode::Enter, event_loop);
            }
            if p.hit(b::RELOAD) {
                self.online_key(KeyCode::KeyC, event_loop);
            }
            // The race lobby: the d-pad and the bumpers step through the animals.
            if p.hit(b::LEFT) || p.hit(b::PREVIOUS) {
                self.online_key(KeyCode::ArrowLeft, event_loop);
            }
            if p.hit(b::RIGHT) || p.hit(b::NEXT) {
                self.online_key(KeyCode::ArrowRight, event_loop);
            }
            if p.hit(b::CROUCH) {
                self.online_key(KeyCode::Escape, event_loop);
            }
            return;
        }
        if p.hit(b::PAUSE) {
            self.enter_pause();
            return;
        }
        if p.hit(b::UP) {
            self.open_maps();
            return;
        }
        if !self.grabbed {
            self.pad = Default::default();
            if p.hit(b::JUMP) {
                self.set_grab(true);
                self.controller.reset();
                self.pad = Default::default();
            }
            return;
        }
        // On a shared screen the first player plays on the keyboard and mouse; gamepads belong to the others (`poll_guest_pads`).
        if self.local_player_count() > 1 && self.device == red_engine2::splitscreen::Device::KeyboardMouse {
            self.pad = Default::default();
            return;
        }
        self.apply_pad_gameplay(dt);
    }

    /// What the gamepad in `self.pad` does to the game for the player swapped in: look, jump, use, switch, reload.
    pub(crate) fn apply_pad_gameplay(&mut self, dt: f32) {
        let p = self.pad;
        let zoom = red_engine2::firearms::zoom_sensitivity(self.camera.fov_deg, self.scene.player.fov_deg);
        self.camera.look(p.look.x * 2.8 * dt * zoom, p.look.y * 2.2 * dt * zoom);
        if p.hit(b::JUMP) {
            self.jump_queued = true;
        }
        if p.hit(b::FIRE) {
            if self.scene.player.mode.is_peaceful() {
                self.interact();
            } else {
                self.attack_queued = true;
                self.net_pulse[1] = NET_PULSE_TICKS;
            }
        }
        if p.hit(b::INTERACT) {
            self.interact();
        }
        if p.hit(b::NEXT) {
            self.on_scroll(1.0);
        }
        if p.hit(b::PREVIOUS) {
            self.on_scroll(-1.0);
        }
        if p.hit(b::VIEW) {
            self.toggle_view_mode();
        }
        if p.hit(b::RELOAD) {
            if self.net.is_some() {
                self.net_pulse[3] = NET_PULSE_TICKS;
            } else if self.weapon.is_firearm() {
                self.ammo.reload();
            }
        }
    }
}
