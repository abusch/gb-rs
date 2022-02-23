use bitvec::{view::BitView, order::Lsb0};

#[derive(Debug, Default)]
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

impl Joypad {
    pub fn read(&self) -> u8 {
        let mut byte = 0xFFu8;
        let bits = byte.view_bits_mut::<Lsb0>();
        if self.action_selected {
            bits.set(0, !self.a_pressed);
            bits.set(1, !self.b_pressed);
            bits.set(2, !self.select_pressed);
            bits.set(3, !self.start_pressed);
        } else if self.direction_selected {
            bits.set(0, !self.right_pressed);
            bits.set(1, !self.left_pressed);
            bits.set(2, !self.up_pressed);
            bits.set(3, !self.down_pressed);
        }

        byte
    }

    pub fn write(&mut self, b: u8) {
        let bits = b.view_bits::<Lsb0>();
        self.direction_selected = !bits[4];
        self.action_selected = !bits[5];
    }

    pub fn set_button(&mut self, button: Button, is_pressed: bool) -> bool {
        let orig_state = self.read();

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

        self.read() != orig_state
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
