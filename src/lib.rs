mod bus;
pub mod cartridge;
mod cpu;
pub mod gameboy;
mod gfx;
mod timer;

pub const SCREEN_WIDTH: usize = 160;
pub const SCREEN_HEIGHT: usize = 144;

pub trait FrameSink {
    fn push_frame(&mut self, frame: &[(u8, u8, u8)]);
}
