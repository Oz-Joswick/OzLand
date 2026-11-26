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
    left_held: bool,
    left_released: bool,
    buttons_held: HashSet<MouseButton>,
    buttons_pressed: HashSet<MouseButton>,
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
            if state == ElementState::Released && self.left_held {
                self.left_released = true;
            }
            self.left_held = state == ElementState::Pressed;
        }
        if state == ElementState::Pressed {
            self.buttons_pressed.insert(button);
            self.buttons_held.insert(button);
        } else {
            self.buttons_held.remove(&button);
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
        self.left_held = false;
        self.left_released = false;
        self.buttons_held.clear();
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

    pub fn left_clicked(&self) -> bool {
        self.left_clicked
    }

    pub fn left_released(&self) -> bool {
        self.left_released
    }

    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }

    /// Clears the per-frame state (presses, clicks, mouse movement).
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.buttons_pressed.clear();
        self.mouse_delta = Vec2::ZERO;
        self.left_clicked = false;
        self.left_released = false;
    }

    /// -1..1 along an axis from a pair of key groups.
    pub fn axis(&self, negative: &[KeyCode], positive: &[KeyCode]) -> f32 {
        self.any_held(positive) as i32 as f32 - self.any_held(negative) as i32 as f32
    }
}

/// What a skill is bound to: a key or a mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bind {
    Key(KeyCode),
    Mouse(MouseButton),
}

/// Keys a skill can be bound to, with how they're shown. Movement, jump,
/// sprint, crouch, Esc, Tab, Enter, V, the weapon keys 1-7 and the clone
/// keys M, X and R are kept for themselves.
const BINDABLE: [(KeyCode, &str); 36] = [
    (KeyCode::KeyB, "B"),
    (KeyCode::KeyC, "C"),
    (KeyCode::KeyE, "E"),
    (KeyCode::KeyF, "F"),
    (KeyCode::KeyG, "G"),
    (KeyCode::KeyH, "H"),
    (KeyCode::KeyI, "I"),
    (KeyCode::KeyJ, "J"),
    (KeyCode::KeyK, "K"),
    (KeyCode::KeyL, "L"),
    (KeyCode::KeyN, "N"),
    (KeyCode::KeyO, "O"),
    (KeyCode::KeyP, "P"),
    (KeyCode::KeyQ, "Q"),
    (KeyCode::KeyT, "T"),
    (KeyCode::KeyU, "U"),
    (KeyCode::KeyY, "Y"),
    (KeyCode::KeyZ, "Z"),
    (KeyCode::Digit8, "8"),
    (KeyCode::Digit9, "9"),
    (KeyCode::Digit0, "0"),
    (KeyCode::Backquote, "`"),
    (KeyCode::CapsLock, "Caps"),
    (KeyCode::Backslash, "\\"),
    (KeyCode::BracketLeft, "["),
    (KeyCode::BracketRight, "]"),
    (KeyCode::Semicolon, ";"),
    (KeyCode::Quote, "'"),
    (KeyCode::Comma, ","),
    (KeyCode::Period, "."),
    (KeyCode::Slash, "/"),
    (KeyCode::Minus, "-"),
    (KeyCode::Equal, "="),
    (KeyCode::F1, "F1"),
    (KeyCode::F2, "F2"),
    (KeyCode::F3, "F3"),
];

const MOUSE: [(MouseButton, &str, &str); 4] = [
    (MouseButton::Right, "MouseRight", "RMB"),
    (MouseButton::Middle, "MouseMiddle", "MMB"),
    (MouseButton::Back, "MouseBack", "Mouse4"),
    (MouseButton::Forward, "MouseForward", "Mouse5"),
];

impl Bind {
    /// Can a skill use it?
    pub fn allowed(self) -> bool {
        match self {
            Bind::Key(k) => BINDABLE.iter().any(|(c, _)| *c == k),
            Bind::Mouse(b) => MOUSE.iter().any(|(m, _, _)| *m == b),
        }
    }

    /// How it's shown: "F", "RMB".
    pub fn label(self) -> String {
        match self {
            Bind::Key(k) => BINDABLE.iter().find(|(c, _)| *c == k).map_or_else(|| format!("{k:?}"), |(_, l)| (*l).to_owned()),
            Bind::Mouse(b) => MOUSE.iter().find(|(m, _, _)| *m == b).map_or_else(|| format!("{b:?}"), |(_, _, l)| (*l).to_owned()),
        }
    }

    /// The name saved in the settings file ("KeyF", "MouseRight").
    pub fn id(self) -> String {
        match self {
            Bind::Key(k) => format!("{k:?}"),
            Bind::Mouse(b) => MOUSE.iter().find(|(m, _, _)| *m == b).map_or_else(String::new, |(_, id, _)| (*id).to_owned()),
        }
    }

    /// From a saved name or a label shown ("KeyF" or "F", "RMB").
    pub fn parse(s: &str) -> Option<Bind> {
        BINDABLE
            .iter()
            .find(|(c, l)| format!("{c:?}") == s || *l == s)
            .map(|(c, _)| Bind::Key(*c))
            .or_else(|| MOUSE.iter().find(|(_, id, l)| *id == s || *l == s).map(|(m, _, _)| Bind::Mouse(*m)))
    }
}

impl Input {
    pub fn bind_pressed(&self, bind: Bind) -> bool {
        match bind {
            Bind::Key(k) => self.pressed.contains(&k),
            Bind::Mouse(b) => self.buttons_pressed.contains(&b),
        }
    }

    pub fn bind_held(&self, bind: Bind) -> bool {
        match bind {
            Bind::Key(k) => self.held.contains(&k),
            Bind::Mouse(b) => self.buttons_held.contains(&b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_round_trip_and_reserve_movement() {
        for b in [Bind::Key(KeyCode::KeyF), Bind::Mouse(MouseButton::Right), Bind::Key(KeyCode::Digit8)] {
            assert_eq!(Bind::parse(&b.id()), Some(b));
            assert_eq!(Bind::parse(&b.label()), Some(b));
            assert!(b.allowed());
        }
        for reserved in [KeyCode::KeyW, KeyCode::Space, KeyCode::Escape, KeyCode::Tab, KeyCode::Digit1, KeyCode::KeyV] {
            assert!(!Bind::Key(reserved).allowed());
        }
        let mut input = Input::default();
        input.on_mouse_button(MouseButton::Middle, ElementState::Pressed);
        assert!(input.bind_pressed(Bind::Mouse(MouseButton::Middle)) && input.bind_held(Bind::Mouse(MouseButton::Middle)));
        input.end_frame();
        assert!(!input.bind_pressed(Bind::Mouse(MouseButton::Middle)) && input.bind_held(Bind::Mouse(MouseButton::Middle)));
    }
}
