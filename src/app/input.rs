//! Keyboard and pointer state for a frame, collected from `winit` window events: what is held, what was pressed this
//! frame, where the pointer is, and wheel movement. It forgets everything held when the window loses focus (no release
//! events arrive while unfocused, so a held key would otherwise stick). No bindings: a game decides what keys mean.

use std::collections::HashSet;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::keyboard::PhysicalKey;
/// The key and button names [`InputState`] is queried with, re-exported so a game needs no `winit` dependency.
pub use winit::{event::MouseButton, keyboard::KeyCode};

/// Input for the current frame. Feed it [`InputState::on_event`], read it in the game's update, then call
/// [`InputState::end_frame`].
#[derive(Debug, Default, Clone)]
pub struct InputState {
    held: HashSet<KeyCode>,
    pressed: HashSet<KeyCode>,
    buttons: HashSet<MouseButton>,
    clicked: Vec<(MouseButton, (f32, f32))>,
    pointer: Option<(f32, f32)>,
    wheel: f32,
    focused: bool,
}

impl InputState {
    /// A fresh, focused input state.
    pub fn new() -> Self {
        InputState { focused: true, ..Default::default() }
    }

    /// Updates the state from one window event (other events are ignored).
    pub fn on_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            if !event.repeat && self.held.insert(code) {
                                self.pressed.insert(code);
                            }
                        }
                        ElementState::Released => {
                            self.held.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.pointer = Some((position.x as f32, position.y as f32)),
            WindowEvent::CursorLeft { .. } => self.pointer = None,
            WindowEvent::MouseInput { state, button, .. } => match state {
                ElementState::Pressed => {
                    self.buttons.insert(*button);
                    if let Some(p) = self.pointer {
                        self.clicked.push((*button, p));
                    }
                }
                ElementState::Released => {
                    self.buttons.remove(button);
                }
            },
            WindowEvent::MouseWheel { delta, .. } => {
                self.wheel += match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = *focused;
                if !focused {
                    self.clear();
                }
            }
            _ => {}
        }
    }

    /// Clears the per-frame parts (presses, clicks, wheel). Held keys and buttons stay held.
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.clicked.clear();
        self.wheel = 0.0;
    }

    /// Forgets everything held or pressed (focus loss, a menu taking over).
    pub fn clear(&mut self) {
        self.held.clear();
        self.pressed.clear();
        self.buttons.clear();
        self.clicked.clear();
        self.wheel = 0.0;
    }

    /// Whether `key` is down.
    pub fn held(&self, key: KeyCode) -> bool {
        self.held.contains(&key)
    }

    /// Whether `key` went down this frame (not a key-repeat).
    pub fn pressed(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    /// Whether a mouse button is down.
    pub fn button(&self, button: MouseButton) -> bool {
        self.buttons.contains(&button)
    }

    /// Where `button` was pressed this frame, in window pixels (row 0 at the top).
    pub fn clicked(&self, button: MouseButton) -> Option<(f32, f32)> {
        self.clicked.iter().rev().find(|(b, _)| *b == button).map(|(_, p)| *p)
    }

    /// The pointer position in window pixels, while it is over the window.
    pub fn pointer(&self) -> Option<(f32, f32)> {
        self.pointer
    }

    /// Wheel lines scrolled this frame (positive = away from the user).
    pub fn wheel(&self) -> f32 {
        self.wheel
    }

    /// Whether the window has keyboard focus.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// `+1` / `-1` / `0` from two sets of keys (e.g. `axis(&[KeyW, ArrowUp], &[KeyS, ArrowDown])`).
    pub fn axis(&self, positive: &[KeyCode], negative: &[KeyCode]) -> i8 {
        positive.iter().any(|k| self.held(*k)) as i8 - negative.iter().any(|k| self.held(*k)) as i8
    }

    /// WASD / arrow keys as `(forward, strafe)` in -1..1: the axes `sim::player::PlayerInput` takes.
    pub fn wasd(&self) -> (i8, i8) {
        (
            self.axis(&[KeyCode::KeyW, KeyCode::ArrowUp], &[KeyCode::KeyS, KeyCode::ArrowDown]),
            self.axis(&[KeyCode::KeyD, KeyCode::ArrowRight], &[KeyCode::KeyA, KeyCode::ArrowLeft]),
        )
    }

    /// Test and scripting hook: press (`true`) or release a key as if the window had sent it.
    pub fn set_key(&mut self, key: KeyCode, down: bool) {
        if down {
            if self.held.insert(key) {
                self.pressed.insert(key);
            }
        } else {
            self.held.remove(&key);
        }
    }

    /// Test and scripting hook: a click of `button` at `(x, y)` this frame.
    pub fn click(&mut self, button: MouseButton, x: f32, y: f32) {
        self.pointer = Some((x, y));
        self.clicked.push((button, (x, y)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_last_one_frame_holds_last_until_release_and_focus_loss_clears() {
        let mut i = InputState::new();
        i.set_key(KeyCode::KeyW, true);
        i.set_key(KeyCode::KeyD, true);
        assert!(i.pressed(KeyCode::KeyW));
        assert_eq!(i.wasd(), (1, 1));
        i.end_frame();
        assert!(!i.pressed(KeyCode::KeyW) && i.held(KeyCode::KeyW));
        i.set_key(KeyCode::KeyD, false);
        assert_eq!(i.wasd(), (1, 0));
        i.on_event(&WindowEvent::Focused(false));
        assert_eq!(i.wasd(), (0, 0));
        assert!(!i.focused());
    }

    #[test]
    fn clicks_carry_their_position_for_one_frame() {
        let mut i = InputState::new();
        i.click(MouseButton::Left, 10.0, 20.0);
        assert_eq!(i.clicked(MouseButton::Left), Some((10.0, 20.0)));
        assert_eq!(i.clicked(MouseButton::Right), None);
        i.end_frame();
        assert_eq!(i.clicked(MouseButton::Left), None);
        assert_eq!(i.pointer(), Some((10.0, 20.0)));
    }
}
