//! Native gamepad polling and renderer-independent stick shaping, button edges and neutral rearming.
use glam::Vec2;

/// Logical buttons, using Xbox names; equivalent PlayStation/Nintendo positions are mapped by gilrs.
pub mod button {
    pub const JUMP: u16 = 1;
    pub const CROUCH: u16 = 2;
    pub const INTERACT: u16 = 4;
    pub const RELOAD: u16 = 8;
    pub const PREVIOUS: u16 = 16;
    pub const NEXT: u16 = 32;
    pub const VIEW: u16 = 64;
    pub const PAUSE: u16 = 128;
    pub const SPRINT: u16 = 256;
    pub const FIRE: u16 = 512;
    pub const AIM: u16 = 1024;
    pub const LEFT: u16 = 2048;
    pub const RIGHT: u16 = 4096;
    pub const UP: u16 = 8192;
    pub const DOWN: u16 = 16384;
}

/// One gamepad sample. Movement Y is forward; look Y is up.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    pub movement: Vec2,
    pub look: Vec2,
    pub held: u16,
    pub pressed: u16,
}
impl Sample {
    /// Is this logical button held?
    pub fn down(self, button: u16) -> bool {
        self.held & button != 0
    }
    /// Did this logical button become pressed this poll?
    pub fn hit(self, button: u16) -> bool {
        self.pressed & button != 0
    }
    fn neutral(self) -> bool {
        self.held == 0 && self.movement == Vec2::ZERO && self.look == Vec2::ZERO
    }
}

/// Circular dead zone with continuous full-range remapping; rejects invalid native values.
pub fn stick(raw: Vec2, dead_zone: f32) -> Vec2 {
    if !raw.is_finite() {
        return Vec2::ZERO;
    }
    let length = raw.length();
    let dead = dead_zone.clamp(0.0, 0.95);
    if length <= dead {
        Vec2::ZERO
    } else {
        raw / length * ((length.min(1.0) - dead) / (1.0 - dead))
    }
}

/// Blocks held input after focus loss, disconnect or menu transitions until all controls are released.
#[derive(Default)]
pub struct Gate {
    armed: bool,
    previous: u16,
}
impl Gate {
    /// Require a neutral sample before accepting input again.
    pub fn reset(&mut self) {
        self.armed = false;
        self.previous = 0;
    }
    /// Generate rising edges while respecting focus and neutral rearming.
    pub fn sample(&mut self, mut raw: Sample, enabled: bool) -> Sample {
        if !enabled {
            self.reset();
            return Sample::default();
        }
        if !self.armed {
            self.armed = raw.neutral();
            return Sample::default();
        }
        raw.pressed = raw.held & !self.previous;
        self.previous = raw.held;
        raw
    }
}

/// Native backend. A missing backend/device leaves keyboard and mouse fully usable.
#[cfg(feature = "gfx")]
pub struct Controller {
    native: Option<gilrs::Gilrs>,
    active: Option<gilrs::GamepadId>,
    gate: Gate,
}
#[cfg(feature = "gfx")]
impl Default for Controller {
    fn default() -> Self {
        let native = gilrs::Gilrs::new().map_err(|e| eprintln!("Controller input unavailable: {e}")).ok();
        Self { native, active: None, gate: Gate::default() }
    }
}
#[cfg(feature = "gfx")]
impl Controller {
    /// Require release before resuming play after a menu transition.
    pub fn reset(&mut self) {
        self.gate.reset();
    }
    /// Poll the first connected controller, retaining ownership until it disconnects.
    pub fn poll(&mut self, focused: bool) -> Sample {
        use gilrs::{Axis, Button, EventType};
        let Some(native) = self.native.as_mut() else {
            return Sample::default();
        };
        while let Some(event) = native.next_event() {
            if matches!(event.event, EventType::Disconnected) && self.active == Some(event.id) {
                self.active = None;
                self.gate.reset();
            }
        }
        if self.active.is_none() {
            self.active = native.gamepads().find(|(_, p)| p.is_connected()).map(|(id, _)| id);
            self.gate.reset();
        }
        let Some(id) = self.active else {
            return Sample::default();
        };
        let pad = native.gamepad(id);
        let mut raw = Sample {
            movement: stick(Vec2::new(pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY)), 0.18),
            look: stick(Vec2::new(pad.value(Axis::RightStickX), pad.value(Axis::RightStickY)), 0.15),
            ..Default::default()
        };
        for (physical, logical) in [
            (Button::South, button::JUMP),
            (Button::East, button::CROUCH),
            (Button::West, button::INTERACT),
            (Button::North, button::RELOAD),
            (Button::LeftTrigger, button::PREVIOUS),
            (Button::RightTrigger, button::NEXT),
            (Button::Select, button::VIEW),
            (Button::Start, button::PAUSE),
            (Button::LeftThumb, button::SPRINT),
            (Button::DPadLeft, button::LEFT),
            (Button::DPadRight, button::RIGHT),
            (Button::DPadUp, button::UP),
            (Button::DPadDown, button::DOWN),
        ] {
            if pad.is_pressed(physical) {
                raw.held |= logical;
            }
        }
        for (physical, logical) in [(Button::LeftTrigger2, button::AIM), (Button::RightTrigger2, button::FIRE)] {
            if pad.button_data(physical).is_some_and(|b| b.value() > 0.25) {
                raw.held |= logical;
            }
        }
        self.gate.sample(raw, focused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn circular_dead_zone_preserves_direction_and_clamps_magnitude() {
        assert_eq!(stick(Vec2::new(0.1, 0.1), 0.18), Vec2::ZERO);
        assert_eq!(stick(Vec2::new(f32::NAN, 1.0), 0.18), Vec2::ZERO);
        assert!((stick(Vec2::new(0.59, 0.0), 0.18).x - 0.5).abs() < 0.001);
        assert!((stick(Vec2::ONE, 0.18).length() - 1.0).abs() < 0.001);
    }
    #[test]
    fn focus_and_disconnect_require_release_and_buttons_only_edge_once() {
        let mut gate = Gate::default();
        let held = Sample { held: button::FIRE, ..Default::default() };
        assert_eq!(gate.sample(held, true).held, 0);
        gate.sample(Sample::default(), true);
        assert!(gate.sample(held, true).hit(button::FIRE));
        assert!(!gate.sample(held, true).hit(button::FIRE));
        assert_eq!(gate.sample(held, false).held, 0);
        assert_eq!(gate.sample(held, true).held, 0);
        gate.sample(Sample::default(), true);
        assert!(gate.sample(held, true).hit(button::FIRE));
        gate.reset();
        assert_eq!(gate.sample(held, true).held, 0);
    }
}
