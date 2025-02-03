use std::collections::VecDeque;

mod apu;
mod bus;
pub mod cartridge;
mod cpu;
pub mod disasm;
pub mod gameboy;
mod gfx;
mod interrupt;
pub mod joypad;
mod timer;

pub const SCREEN_WIDTH: usize = 160;
pub const SCREEN_HEIGHT: usize = 144;

pub trait FrameSink {
    fn push_frame(&mut self, frame: &[(u8, u8, u8)]);
}

pub trait AudioSink {
    fn push_sample(&mut self, sample: (f32, f32)) -> bool;
    fn push_samples(&mut self, samples: &mut VecDeque<f32>);
}
