//! What the keyboard, the mouse and the gamepad mean in a match.
//!
//! | action | keyboard and mouse | gamepad (Xbox names, laid out like CS:GO's) |
//! |---|---|---|
//! | move / look | W A S D, mouse | left stick, right stick |
//! | fire | left mouse | right trigger |
//! | aim down sights / scope (grenade: underhand) | right mouse | left trigger |
//! | jump | Space | A |
//! | crouch (hold) | Ctrl or C | B |
//! | reload | R | X |
//! | use / pick up | E | Y |
//! | drop weapon | G | Y while crouching (B held) |
//! | weapon slots | 1 primary, 2 secondary, 3 knife, 4 grenades | D-pad up, right, left, down |
//! | previous / next weapon | mouse wheel | left / right bumper |
//! | last weapon | Q | left stick click |
//! | scoreboard | hold Tab | Back |
//! | pause | Esc | Start |
//!

use red_engine2::controller::{button as pad, Sample};
use red_engine2::sim::player::PlayerInput;
use std::collections::HashSet;
use winit::keyboard::KeyCode;

/// Ticks an action button stays down on the input sent to the server (the server acts on the press; redundancy covers a lost packet).
const PULSE_TICKS: u8 = 2;

/// Raw device state and queued presses.
#[derive(Default)]
pub struct Controls {
    /// Keys held.
    pub keys: HashSet<KeyCode>,
    /// Left mouse button held.
    pub fire: bool,
    /// Right mouse button held.
    pub aim: bool,
    /// Right mouse button went down since the last frame (scopes toggle their zoom level on it).
    pub aim_pressed: bool,
    /// Mouse motion since the last frame, pixels.
    pub mouse: (f32, f32),
    /// Wheel lines since the last frame (up positive).
    pub wheel: f32,
    /// The gamepad this frame.
    pub pad: Sample,
    jump: bool,
    reload: u8,
    interact: u8,
    drop: u8,
    select: (u8, u8),
}

impl Controls {
    /// Forgets every held control (a menu took over, focus was lost).
    pub fn release_all(&mut self) {
        *self = Controls::default();
    }

    /// A key went down (not a repeat): queues the one-shot actions.
    pub fn key_down(&mut self, code: KeyCode) {
        if self.keys.contains(&code) {
            return;
        }
        self.keys.insert(code);
        match code {
            KeyCode::Space => self.jump = true,
            KeyCode::KeyR => self.reload = PULSE_TICKS,
            KeyCode::KeyE => self.interact = PULSE_TICKS,
            KeyCode::KeyG => self.drop = PULSE_TICKS,
            KeyCode::Digit1 => self.select = (1, PULSE_TICKS),
            KeyCode::Digit2 => self.select = (2, PULSE_TICKS),
            KeyCode::Digit3 => self.select = (3, PULSE_TICKS),
            KeyCode::Digit4 => self.select = (4, PULSE_TICKS),
            KeyCode::KeyQ => self.select = (5, PULSE_TICKS),
            _ => {}
        }
    }

    /// A key went up.
    pub fn key_up(&mut self, code: KeyCode) {
        self.keys.remove(&code);
    }

    /// Applies the gamepad's presses (once per frame, after polling it).
    pub fn apply_pad(&mut self) {
        let p = self.pad;
        if p.hit(pad::JUMP) {
            self.jump = true;
        }
        if p.hit(pad::NEXT) {
            self.select = (6, PULSE_TICKS);
        }
        if p.hit(pad::PREVIOUS) {
            self.select = (7, PULSE_TICKS);
        }
        if p.hit(pad::UP) {
            self.select = (1, PULSE_TICKS);
        }
        if p.hit(pad::RIGHT) {
            self.select = (2, PULSE_TICKS);
        }
        if p.hit(pad::LEFT) {
            self.select = (3, PULSE_TICKS);
        }
        if p.hit(pad::DOWN) {
            self.select = (4, PULSE_TICKS);
        }
        if p.hit(pad::SPRINT) {
            self.select = (5, PULSE_TICKS);
        }
        if p.hit(pad::INTERACT) {
            self.reload = PULSE_TICKS;
        }
        if p.hit(pad::RELOAD) {
            if p.down(pad::CROUCH) {
                self.drop = PULSE_TICKS;
            } else {
                self.interact = PULSE_TICKS;
            }
        }
    }

    /// Queues a wheel notch as a weapon change.
    pub fn apply_wheel(&mut self) {
        if self.wheel >= 1.0 {
            self.select = (7, PULSE_TICKS);
            self.wheel = 0.0;
        } else if self.wheel <= -1.0 {
            self.select = (6, PULSE_TICKS);
            self.wheel = 0.0;
        }
    }

    fn held(&self, a: KeyCode, b: KeyCode) -> bool {
        self.keys.contains(&a) || self.keys.contains(&b)
    }

    /// Whether the scoreboard is asked for.
    pub fn scoreboard(&self) -> bool {
        self.keys.contains(&KeyCode::Tab) || self.pad.down(pad::VIEW)
    }

    /// Whether the crouch control is held.
    pub fn crouching(&self) -> bool {
        self.held(KeyCode::ControlLeft, KeyCode::KeyC) || self.keys.contains(&KeyCode::ControlRight) || self.pad.down(pad::CROUCH)
    }

    /// Whether the trigger is held (mouse or right trigger).
    pub fn firing(&self) -> bool {
        self.fire || self.pad.down(pad::FIRE)
    }

    /// Whether the aim control is held (right mouse or left trigger).
    pub fn aiming(&self) -> bool {
        self.aim || self.pad.down(pad::AIM)
    }

    /// Whether the aim control went down this frame.
    pub fn aim_edge(&self) -> bool {
        self.aim_pressed || self.pad.hit(pad::AIM)
    }

    /// The look stick, `(yaw, pitch)` from -1 to 1 (right and up positive).
    pub fn look_stick(&self) -> (f32, f32) {
        (self.pad.look.x, self.pad.look.y)
    }

    /// One fixed tick of input for the server. `yaw` and `pitch` are where the view points, recoil included.
    pub fn tick(&mut self, yaw: f32, pitch: f32, aim: bool) -> PlayerInput {
        let key = |pos: [KeyCode; 2], neg: [KeyCode; 2]| self.held(pos[0], pos[1]) as i8 - self.held(neg[0], neg[1]) as i8;
        let fwd = key([KeyCode::KeyW, KeyCode::ArrowUp], [KeyCode::KeyS, KeyCode::ArrowDown]);
        let strafe = key([KeyCode::KeyD, KeyCode::ArrowRight], [KeyCode::KeyA, KeyCode::ArrowLeft]);
        let stick = self.pad.movement;
        let analog = fwd == 0 && strafe == 0 && stick.length_squared() > 0.0;
        let mut input = PlayerInput {
            forward: if analog { (stick.y * 127.0).round() as i8 } else { fwd },
            strafe: if analog { (stick.x * 127.0).round() as i8 } else { strafe },
            analog,
            jump: std::mem::take(&mut self.jump),
            crouch: self.crouching(),
            yaw,
            pitch,
            attack: self.firing(),
            aim,
            ..Default::default()
        };
        let mut tick_down = |v: &mut u8| {
            let on = *v > 0;
            *v = v.saturating_sub(1);
            on
        };
        input.reload = tick_down(&mut self.reload);
        input.interact = tick_down(&mut self.interact);
        input.drop = tick_down(&mut self.drop);
        if self.select.1 > 0 {
            input.select = self.select.0;
            self.select.1 -= 1;
        }
        input
    }

    /// Clears the per-frame accumulators after the frame used them.
    pub fn end_frame(&mut self) {
        self.mouse = (0.0, 0.0);
        self.aim_pressed = false;
    }
}
