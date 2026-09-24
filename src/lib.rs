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

pub use gfx::DEFAULT_DMG_PALETTE;

pub const SCREEN_WIDTH: usize = 160;
pub const SCREEN_HEIGHT: usize = 144;

/// A 15-bit colour in the Game Boy Color's native layout: `0bxBBBBBGGGGGRRRRR`.
///
/// This is what CGB palette RAM holds, so it can represent every colour either model can show.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Rgb555(pub u16);

impl Rgb555 {
    /// Build a colour from 8-bit channels, dropping the 3 low bits of each.
    pub const fn from_rgb888(r: u8, g: u8, b: u8) -> Self {
        Self(((b as u16 >> 3) << 10) | ((g as u16 >> 3) << 5) | (r as u16 >> 3))
    }

    /// Expand to 8-bit channels, replicating the high bits into the low ones so that full
    /// intensity maps to 0xFF.
    pub const fn to_rgb888(self) -> (u8, u8, u8) {
        const fn expand(c: u16) -> u8 {
            let c = (c & 0x1F) as u8;
            (c << 3) | (c >> 2)
        }
        (expand(self.0), expand(self.0 >> 5), expand(self.0 >> 10))
    }
}

pub trait FrameSink {
    fn push_frame(&mut self, frame: &[Rgb555]);
}

pub trait AudioSink {
    fn push_sample(&mut self, sample: (f32, f32)) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb555_layout() {
        // Red lives in the low bits, blue in the high bits, like CGB palette RAM.
        assert_eq!(Rgb555::from_rgb888(0xFF, 0, 0), Rgb555(0x001F));
        assert_eq!(Rgb555::from_rgb888(0, 0xFF, 0), Rgb555(0x03E0));
        assert_eq!(Rgb555::from_rgb888(0, 0, 0xFF), Rgb555(0x7C00));
    }

    #[test]
    fn test_rgb555_to_rgb888() {
        assert_eq!(Rgb555(0x7FFF).to_rgb888(), (0xFF, 0xFF, 0xFF));
        assert_eq!(Rgb555(0x0000).to_rgb888(), (0x00, 0x00, 0x00));
        assert_eq!(Rgb555(0x001F).to_rgb888(), (0xFF, 0x00, 0x00));
        // Unused top bit is ignored.
        assert_eq!(Rgb555(0x8000).to_rgb888(), (0x00, 0x00, 0x00));
        // Every 5-bit value survives a round trip.
        for c in 0..32u16 {
            let (r, g, b) = Rgb555(c | (c << 5) | (c << 10)).to_rgb888();
            assert_eq!(Rgb555::from_rgb888(r, g, b), Rgb555(c | (c << 5) | (c << 10)));
        }
    }
}
