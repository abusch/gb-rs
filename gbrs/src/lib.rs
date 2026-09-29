use serde::{Deserialize, Serialize};

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

pub use bus::{BOOT_ROM_SIZE, BootRom};
pub use gfx::{DEFAULT_DMG_PALETTE, DMG_PALETTES, DmgPalette};

pub const SCREEN_WIDTH: usize = 160;
pub const SCREEN_HEIGHT: usize = 144;

/// The clock everything runs off, in Hz. [`gameboy::GameBoy::step`] counts in its cycles.
pub const CPU_HZ: u64 = 4_194_304;
/// Clock cycles per frame: 154 lines of 456 dots each, so ~59.73 frames per second.
pub const CYCLES_PER_FRAME: u64 = 154 * 456;

/// A 15-bit colour in the Game Boy Color's native layout: `0bxBBBBBGGGGGRRRRR`.
///
/// This is what CGB palette RAM holds, so it can represent every colour either model can show.
///
/// In save states, it's always 2 bytes rather than a varint, so that the size of the framebuffer
/// doesn't depend on what's on screen.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb555(#[serde(with = "postcard::fixint::le")] pub u16);

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

    /// Convert to `RRRRRGGGGGGBBBBB`, replicating green's high bit into its extra low bit so that
    /// full intensity stays full.
    pub const fn to_rgb565(self) -> u16 {
        let r = self.0 & 0x1F;
        let g = (self.0 >> 5) & 0x1F;
        let b = (self.0 >> 10) & 0x1F;
        (r << 11) | (g << 6) | ((g >> 4) << 5) | b
    }
}

pub trait FrameSink {
    fn push_frame(&mut self, frame: &[Rgb555]);
}

pub trait AudioSink {
    fn push_sample(&mut self, sample: (f32, f32));
}

/// Discards the frames.
impl FrameSink for () {
    fn push_frame(&mut self, _frame: &[Rgb555]) {}
}

/// Discards the samples.
impl AudioSink for () {
    fn push_sample(&mut self, _sample: (f32, f32)) {}
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
            assert_eq!(
                Rgb555::from_rgb888(r, g, b),
                Rgb555(c | (c << 5) | (c << 10))
            );
        }
    }

    #[test]
    fn test_rgb555_to_rgb565() {
        assert_eq!(Rgb555(0x7FFF).to_rgb565(), 0xFFFF);
        assert_eq!(Rgb555(0x0000).to_rgb565(), 0x0000);
        assert_eq!(Rgb555(0x001F).to_rgb565(), 0xF800);
        assert_eq!(Rgb555(0x03E0).to_rgb565(), 0x07E0);
        assert_eq!(Rgb555(0x7C00).to_rgb565(), 0x001F);
        // Unused top bit is ignored.
        assert_eq!(Rgb555(0x8000).to_rgb565(), 0x0000);
    }
}
