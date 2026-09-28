use bitvec::{order::Lsb0, view::BitView};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Joypad {
    action_selected: bool,
    direction_selected: bool,
    select_pressed: bool,
    start_pressed: bool,
    a_pressed: bool,
    b_pressed: bool,
    up_pressed: bool,
    down_pressed: bool,
    left_pressed: bool,
    right_pressed: bool,
}

impl Default for Joypad {
    fn default() -> Self {
        Self {
            action_selected: true,
            direction_selected: true,
            select_pressed: Default::default(),
            start_pressed: Default::default(),
            a_pressed: Default::default(),
            b_pressed: Default::default(),
            up_pressed: Default::default(),
            down_pressed: Default::default(),
            left_pressed: Default::default(),
            right_pressed: Default::default(),
        }
    }
}

impl Joypad {
    /// The P1/JOYP register. Bits 0-3 are the input lines, low when a button in a selected group
    /// is pressed. With both groups selected, a line is low if either of its buttons is pressed.
    pub fn read(&self) -> u8 {
        let mut byte = 0xFFu8;
        let bits = byte.view_bits_mut::<Lsb0>();
        bits.set(4, !self.direction_selected);
        bits.set(5, !self.action_selected);
        byte & !self.pressed_lines()
    }

    /// The input lines pulled low by pressed buttons in the selected groups, as a bitmask.
    fn pressed_lines(&self) -> u8 {
        let mut lines = 0;
        if self.action_selected {
            lines |= self.a_pressed as u8
                | (self.b_pressed as u8) << 1
                | (self.select_pressed as u8) << 2
                | (self.start_pressed as u8) << 3;
        }
        if self.direction_selected {
            lines |= self.right_pressed as u8
                | (self.left_pressed as u8) << 1
                | (self.up_pressed as u8) << 2
                | (self.down_pressed as u8) << 3;
        }
        lines
    }

    /// Write the P1/JOYP register, which selects the groups of buttons to read. Return whether an
    /// input line went low, which requests the joypad interrupt: selecting a group while one of
    /// its buttons is held does.
    pub fn write(&mut self, b: u8) -> bool {
        let before = self.pressed_lines();
        let bits = b.view_bits::<Lsb0>();
        self.direction_selected = !bits[4];
        self.action_selected = !bits[5];
        self.pressed_lines() & !before != 0
    }

    /// Press or release a button. Return whether an input line went low, which requests the
    /// joypad interrupt: only pressing a button in a selected group does.
    pub fn set_button(&mut self, button: Button, is_pressed: bool) -> bool {
        let before = self.pressed_lines();

        match button {
            Button::Start => self.start_pressed = is_pressed,
            Button::Select => self.select_pressed = is_pressed,
            Button::A => self.a_pressed = is_pressed,
            Button::B => self.b_pressed = is_pressed,
            Button::Up => self.up_pressed = is_pressed,
            Button::Down => self.down_pressed = is_pressed,
            Button::Left => self.left_pressed = is_pressed,
            Button::Right => self.right_pressed = is_pressed,
        }

        self.pressed_lines() & !before != 0
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum Button {
    Start = 0,
    Select,
    A,
    B,
    Up,
    Down,
    Left,
    Right,
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
