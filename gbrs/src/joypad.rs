use serde::{Deserialize, Serialize};

/// P1 bit that is set while the direction buttons are *not* selected.
const DIRECTIONS_DESELECTED: u8 = 1 << 4;
/// P1 bit that is set while the action buttons are *not* selected.
const ACTIONS_DESELECTED: u8 = 1 << 5;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Joypad {
    /// The group selection bits of P1 (`*_DESELECTED`). Both groups are selected at power-on.
    select: u8,
    /// The buttons held, one bit each (see `Button`).
    pressed: u8,
}

impl Joypad {
    /// The P1/JOYP register. Bits 0-3 are the input lines, low when a button in a selected group
    /// is pressed. With both groups selected, a line is low if either of its buttons is pressed.
    pub fn read(&self) -> u8 {
        0xC0 | self.select | (!self.pressed_lines() & 0x0F)
    }

    /// The input lines pulled low by pressed buttons in the selected groups, as a bitmask.
    fn pressed_lines(&self) -> u8 {
        let mut lines = 0;
        if self.select & ACTIONS_DESELECTED == 0 {
            lines |= self.pressed & 0x0F;
        }
        if self.select & DIRECTIONS_DESELECTED == 0 {
            lines |= self.pressed >> 4;
        }
        lines
    }

    /// Write the P1/JOYP register, which selects the groups of buttons to read. Return whether an
    /// input line went low, which requests the joypad interrupt: selecting a group while one of
    /// its buttons is held does.
    pub fn write(&mut self, b: u8) -> bool {
        let before = self.pressed_lines();
        self.select = b & (ACTIONS_DESELECTED | DIRECTIONS_DESELECTED);
        self.pressed_lines() & !before != 0
    }

    /// Press or release a button. Return whether an input line went low, which requests the
    /// joypad interrupt: only pressing a button in a selected group does.
    pub fn set_button(&mut self, button: Button, is_pressed: bool) -> bool {
        let before = self.pressed_lines();
        let bit = 1 << button as u8;
        if is_pressed {
            self.pressed |= bit;
        } else {
            self.pressed &= !bit;
        }
        self.pressed_lines() & !before != 0
    }
}

/// A Game Boy button. Each group's buttons are in the order of the input lines they pull low: the
/// action buttons on lines 0-3, then the directions.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Button {
    A = 0,
    B,
    Select,
    Start,
    Right,
    Left,
    Up,
    Down,
}

impl Button {
    pub const ALL: [Button; 8] = [
        Button::A,
        Button::B,
        Button::Select,
        Button::Start,
        Button::Right,
        Button::Left,
        Button::Up,
        Button::Down,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    const SELECT_ACTIONS: u8 = 0x10;
    const SELECT_DIRECTIONS: u8 = 0x20;
    const SELECT_BOTH: u8 = 0x00;
    const SELECT_NONE: u8 = 0x30;

    #[test]
    fn test_read() {
        let mut joypad = Joypad::default();
        joypad.set_button(Button::Start, true);
        joypad.set_button(Button::Left, true);
        joypad.write(SELECT_ACTIONS);
        assert_eq!(joypad.read(), 0xD7);
        joypad.write(SELECT_DIRECTIONS);
        assert_eq!(joypad.read(), 0xED);
        // With both groups selected, lines are low for buttons pressed in either.
        joypad.write(SELECT_BOTH);
        assert_eq!(joypad.read(), 0xC5);
        joypad.write(SELECT_NONE);
        assert_eq!(joypad.read(), 0xFF);
    }

    #[test]
    fn test_interrupt_on_falling_lines() {
        let mut joypad = Joypad::default();
        joypad.write(SELECT_ACTIONS);
        // Only presses in a selected group pull a line low
        assert!(joypad.set_button(Button::A, true));
        assert!(!joypad.set_button(Button::Up, true));
        assert!(!joypad.set_button(Button::A, false));

        // Selecting a group while one of its buttons is held does too
        joypad.write(SELECT_NONE);
        joypad.set_button(Button::Start, true);
        assert!(joypad.write(SELECT_ACTIONS));
        assert!(!joypad.write(SELECT_ACTIONS));
        // Up is still held
        assert!(joypad.write(SELECT_BOTH));
        assert!(!joypad.write(SELECT_NONE));
    }
}
