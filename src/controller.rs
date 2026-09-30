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
    /// The analog triggers, `0.0` (released) to `1.0` (fully pulled): `x` the left, `y` the right. A pad whose triggers are only buttons reads `0` or `1`.
    pub triggers: Vec2,
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
        self.held == 0 && self.movement == Vec2::ZERO && self.look == Vec2::ZERO && self.triggers == Vec2::ZERO
    }
}

/// What a kart driver asks for, from a gamepad sample (Great Outdoors): the pure mapping, so it is tested without a pad. Xbox names; the equivalent
/// PlayStation and Nintendo buttons are mapped by gilrs.
///
/// | control | does |
/// |---|---|
/// | left stick X, or the D-pad | steer |
/// | right trigger (analog), or A | accelerate |
/// | left trigger (analog), or B | brake, then reverse |
/// | either shoulder button | hop, and drift while held |
/// | X | use the held item |
/// | Y | the driver's ability (the Beaver builds) |
/// | Start | pause (handled by the menu layer) |
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KartControls {
    /// Steering, `-1.0` left to `1.0` right.
    pub steer: f32,
    /// Throttle, `-1.0` (full brake or reverse) to `1.0` (full throttle).
    pub throttle: f32,
    /// Hop and drift button held.
    pub hop: bool,
    /// Item button held (the kart uses the item on the press).
    pub item: bool,
    /// Ability button held (the ability fires on the press).
    pub ability: bool,
}

impl KartControls {
    /// Whether the pad is asking for anything: when it is not, the keyboard's digital keys are used instead.
    pub fn active(&self) -> bool {
        self.steer != 0.0 || self.throttle != 0.0
    }
}

/// The kart controls a gamepad sample means. Analog triggers and the stick are used as they are; a pad without analog triggers falls back to A and B.
pub fn kart_controls(pad: &Sample) -> KartControls {
    let dpad = pad.down(button::RIGHT) as i8 as f32 - pad.down(button::LEFT) as i8 as f32;
    let steer = if pad.movement.x != 0.0 { pad.movement.x } else { dpad }.clamp(-1.0, 1.0);
    let accelerate = pad.triggers.y.max(pad.down(button::JUMP) as u8 as f32);
    let brake = pad.triggers.x.max(pad.down(button::CROUCH) as u8 as f32);
    KartControls {
        steer,
        throttle: (accelerate - brake).clamp(-1.0, 1.0),
        hop: pad.down(button::NEXT) || pad.down(button::PREVIOUS),
        item: pad.down(button::INTERACT),
        ability: pad.down(button::RELOAD),
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
        let trigger = |b: Button| pad.button_data(b).map_or(0.0, |d| d.value()).clamp(0.0, 1.0);
        // A small dead zone so a resting trigger that reads 0.02 does not creep the kart forward.
        let shaped = |v: f32| if v < 0.05 { 0.0 } else { (v - 0.05) / 0.95 };
        let mut raw = Sample {
            movement: stick(Vec2::new(pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY)), 0.18),
            look: stick(Vec2::new(pad.value(Axis::RightStickX), pad.value(Axis::RightStickY)), 0.15),
            triggers: Vec2::new(shaped(trigger(Button::LeftTrigger2)), shaped(trigger(Button::RightTrigger2))),
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

    #[test]
    fn a_pad_with_analog_triggers_drives_a_kart_by_the_pull() {
        let half = Sample { triggers: Vec2::new(0.0, 0.5), movement: Vec2::new(-0.6, 0.9), ..Default::default() };
        let c = kart_controls(&half);
        assert_eq!((c.steer, c.throttle), (-0.6, 0.5), "the stick steers, the right trigger is a half throttle; stick Y is ignored");
        let brake = kart_controls(&Sample { triggers: Vec2::new(1.0, 0.0), ..Default::default() });
        assert_eq!(brake.throttle, -1.0, "the left trigger brakes and reverses");
        let both = kart_controls(&Sample { triggers: Vec2::new(0.4, 1.0), ..Default::default() });
        assert!((both.throttle - 0.6).abs() < 1e-6, "both pulled: the difference");
        assert!(!kart_controls(&Sample::default()).active(), "a resting pad asks for nothing, so the keyboard keeps working");
        assert!(kart_controls(&half).active());
    }

    #[test]
    fn a_pad_without_analog_triggers_uses_the_face_buttons() {
        let go = kart_controls(&Sample { held: button::JUMP, ..Default::default() });
        assert_eq!(go.throttle, 1.0, "A accelerates");
        let stop = kart_controls(&Sample { held: button::CROUCH, ..Default::default() });
        assert_eq!(stop.throttle, -1.0, "B brakes");
        assert_eq!(kart_controls(&Sample { held: button::JUMP | button::CROUCH, ..Default::default() }).throttle, 0.0);
        let dpad = kart_controls(&Sample { held: button::RIGHT, ..Default::default() });
        assert_eq!(dpad.steer, 1.0, "the D-pad steers");
        let stick_wins = kart_controls(&Sample { held: button::RIGHT, movement: Vec2::new(-0.3, 0.0), ..Default::default() });
        assert_eq!(stick_wins.steer, -0.3, "the analog stick beats the D-pad");
    }

    #[test]
    fn hop_item_and_ability_are_the_shoulders_x_and_y_and_held_buttons_stay_held() {
        for shoulder in [button::NEXT, button::PREVIOUS] {
            assert!(kart_controls(&Sample { held: shoulder, ..Default::default() }).hop);
        }
        let x = kart_controls(&Sample { held: button::INTERACT, ..Default::default() });
        let y = kart_controls(&Sample { held: button::RELOAD, ..Default::default() });
        assert!(x.item && !x.ability && y.ability && !y.item);
        assert!(!kart_controls(&Sample::default()).hop);
        // The mapping reads held buttons, not the per-poll edge, so a press between two fixed steps is never lost: the kart does its own edge detection.
        let edge_missed = Sample { held: button::INTERACT, pressed: 0, ..Default::default() };
        assert!(kart_controls(&edge_missed).item);
    }

    #[test]
    fn triggers_are_held_back_by_the_gate_like_everything_else() {
        let mut gate = Gate::default();
        let pulled = Sample { triggers: Vec2::new(0.0, 1.0), ..Default::default() };
        assert_eq!(gate.sample(pulled, true).triggers, Vec2::ZERO, "a trigger pulled while the gate is unarmed is ignored until release");
        gate.sample(Sample::default(), true);
        assert_eq!(gate.sample(pulled, true).triggers, Vec2::new(0.0, 1.0));
        assert_eq!(gate.sample(pulled, false).triggers, Vec2::ZERO, "and a lost focus zeroes them");
    }
}
