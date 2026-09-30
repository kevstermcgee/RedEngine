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
            if p.hit(b::UP) {
                self.pause_hover = Some(PauseAction::Resume);
                self.repaint_pause();
            }
            if p.hit(b::DOWN) {
                self.pause_hover = Some(if self.pause_hover == Some(PauseAction::Resume) { PauseAction::Fullscreen } else { PauseAction::Quit });
                self.repaint_pause();
            }
            if p.hit(b::JUMP) && self.pause_hover == Some(PauseAction::Fullscreen) {
                self.toggle_fullscreen();
            } else if p.hit(b::JUMP) && self.pause_hover == Some(PauseAction::Quit) {
                event_loop.exit();
            } else if p.hit(b::PAUSE) || p.hit(b::CROUCH) || p.hit(b::JUMP) {
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
