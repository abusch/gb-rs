//! Game Boy buttons from the keyboard and gamepads.

use std::ops::BitOr;

use gbrs::joypad::Button;
use gilrs::{Axis, EventType, Gilrs};
use log::{info, warn};

pub const ALL_BUTTONS: [Button; 8] = [
    Button::Start,
    Button::Select,
    Button::A,
    Button::B,
    Button::Up,
    Button::Down,
    Button::Left,
    Button::Right,
];

/// Gamepad buttons for each Game Boy button. A and B go by position rather than label, as on the
/// Game Boy: A is the right face button, B the bottom one.
const GAMEPAD_BUTTONS: [(gilrs::Button, Button); 8] = [
    (gilrs::Button::East, Button::A),
    (gilrs::Button::South, Button::B),
    (gilrs::Button::Start, Button::Start),
    (gilrs::Button::Select, Button::Select),
    (gilrs::Button::DPadUp, Button::Up),
    (gilrs::Button::DPadDown, Button::Down),
    (gilrs::Button::DPadLeft, Button::Left),
    (gilrs::Button::DPadRight, Button::Right),
];

/// How far the left stick has to be pushed to press a direction.
const STICK_THRESHOLD: f32 = 0.5;

/// A set of Game Boy buttons.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Buttons(u8);

impl Buttons {
    pub fn contains(self, button: Button) -> bool {
        self.0 & (1 << button as u8) != 0
    }

    pub fn set(&mut self, button: Button, pressed: bool) {
        if pressed {
            self.0 |= 1 << button as u8;
        } else {
            self.0 &= !(1 << button as u8);
        }
    }
}

impl BitOr for Buttons {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// The connected gamepads, if the platform supports them.
pub struct Gamepads {
    gilrs: Option<Gilrs>,
}

impl Gamepads {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(gilrs) => {
                for (_, gamepad) in gilrs.gamepads() {
                    info!("Gamepad found: {}", gamepad.name());
                }
                Some(gilrs)
            }
            Err(e) => {
                warn!("Gamepads are not available: {e}");
                None
            }
        };
        Self { gilrs }
    }

    /// Process pending gamepad events, and return the buttons held on any connected gamepad.
    ///
    /// This looks at the current state of each gamepad rather than at individual events, so a
    /// gamepad that gets disconnected releases its buttons, and several of them can be used at
    /// once.
    pub fn poll(&mut self) -> Buttons {
        let Some(gilrs) = &mut self.gilrs else {
            return Buttons::default();
        };
        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::Connected => {
                    info!("Gamepad connected: {}", gilrs.gamepad(event.id).name());
                }
                EventType::Disconnected => {
                    info!("Gamepad disconnected: {}", gilrs.gamepad(event.id).name());
                }
                _ => (),
            }
        }

        let mut held = Buttons::default();
        for (_, gamepad) in gilrs.gamepads() {
            for (gamepad_button, button) in GAMEPAD_BUTTONS {
                if gamepad.is_pressed(gamepad_button) {
                    held.set(button, true);
                }
            }
            // Some gamepads report their D-pad as axes. Positive Y is up.
            for (x_axis, y_axis) in [
                (Axis::LeftStickX, Axis::LeftStickY),
                (Axis::DPadX, Axis::DPadY),
            ] {
                let (x, y) = (gamepad.value(x_axis), gamepad.value(y_axis));
                if x <= -STICK_THRESHOLD {
                    held.set(Button::Left, true);
                }
                if x >= STICK_THRESHOLD {
                    held.set(Button::Right, true);
                }
                if y >= STICK_THRESHOLD {
                    held.set(Button::Up, true);
                }
                if y <= -STICK_THRESHOLD {
                    held.set(Button::Down, true);
                }
            }
        }
        held
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buttons() {
        let mut buttons = Buttons::default();
        buttons.set(Button::A, true);
        buttons.set(Button::Right, true);
        for button in ALL_BUTTONS {
            assert_eq!(
                buttons.contains(button),
                matches!(button, Button::A | Button::Right)
            );
        }
        buttons.set(Button::A, false);
        assert_eq!(buttons, {
            let mut right = Buttons::default();
            right.set(Button::Right, true);
            right
        });

        let mut start = Buttons::default();
        start.set(Button::Start, true);
        assert!((buttons | start).contains(Button::Start));
        assert!((buttons | start).contains(Button::Right));
    }
}
