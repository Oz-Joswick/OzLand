//! Keyboard and mouse state collected from winit events, read once per frame.

use std::collections::HashSet;

use glam::Vec2;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

#[derive(Default)]
pub struct Input {
    held: HashSet<KeyCode>,
    pressed: HashSet<KeyCode>,
    mouse_delta: Vec2,
    left_clicked: bool,
    right_held: bool,
    left_held: bool,
}

impl Input {
    pub fn on_key(&mut self, code: KeyCode, state: ElementState, repeat: bool) {
        match state {
            ElementState::Pressed => {
                if !repeat {
                    self.pressed.insert(code);
                }
                self.held.insert(code);
            }
            ElementState::Released => {
                self.held.remove(&code);
            }
        }
    }

    pub fn on_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        if button == MouseButton::Left && state == ElementState::Pressed {
            self.left_clicked = true;
        }
        if button == MouseButton::Left {
            self.left_held = state == ElementState::Pressed;
        }
        if button == MouseButton::Right {
            self.right_held = state == ElementState::Pressed;
        }
    }

    /// Raw (unaccelerated) mouse movement, in device units.
    pub fn on_mouse_motion(&mut self, dx: f64, dy: f64) {
        self.mouse_delta += Vec2::new(dx as f32, dy as f32);
    }

    /// Forget held keys, e.g. when the window loses focus and would never
    /// see their release events.
    pub fn release_all(&mut self) {
        self.held.clear();
        self.right_held = false;
        self.left_held = false;
        self.mouse_delta = Vec2::ZERO;
    }

    pub fn any_held(&self, keys: &[KeyCode]) -> bool {
        keys.iter().any(|k| self.held.contains(k))
    }

    pub fn pressed(&self, key: KeyCode) -> bool {
        self.pressed.contains(&key)
    }

    pub fn left_held(&self) -> bool {
        self.left_held
    }

    pub fn right_held(&self) -> bool {
        self.right_held
    }

    pub fn left_clicked(&self) -> bool {
        self.left_clicked
    }

    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }

    /// Clears the per-frame state (presses, clicks, mouse movement).
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.mouse_delta = Vec2::ZERO;
        self.left_clicked = false;
    }

    /// -1..1 along an axis from a pair of key groups.
    pub fn axis(&self, negative: &[KeyCode], positive: &[KeyCode]) -> f32 {
        self.any_held(positive) as i32 as f32 - self.any_held(negative) as i32 as f32
    }
}
