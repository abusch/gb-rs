use bitvec::prelude::*;
use log::debug;

#[derive(Debug)]
pub struct Gfx {
    pub vram: Box<[u8]>,
    /// LCDC (LCD Control)
    lcdc: u8,

    /// BG Palette
    bgp: [Color; 4],
    /// OBJ Palette 0
    obp0: [Color; 4],
    /// OBJ Palette 1
    obp1: [Color; 4],
}

impl Gfx {
    pub fn new() -> Self {
        Self {
            lcdc: 0,
            vram: vec![0; 8 * 1024].into_boxed_slice(),
            bgp: [Color::White; 4],
            obp0: [Color::White; 4],
            obp1: [Color::White; 4],
        }
    }

    pub fn write(&mut self, addr: u16, b: u8) {
        if addr == 0xFF47 {
            // FF47 - BGP (BG Palette Data)
            set_palette_data(&mut self.bgp, b);
        } else if addr == 0xFF48 {
            set_palette_data(&mut self.obp0, b);
        } else if addr == 0xFF49 {
            set_palette_data(&mut self.obp1, b);
        } else {
            unimplemented!();
        }
    }
}

fn set_palette_data(palette: &mut [Color; 4], b: u8) {
    debug!("Writing BG Palette with {:b}", b);
    let bits = b.view_bits::<Lsb0>();
    let color1 = bits[0..=1].load::<u8>();
    let color2 = bits[2..=3].load::<u8>();
    let color3 = bits[4..=5].load::<u8>();
    let color4 = bits[6..=7].load::<u8>();

    palette[0] = Color::from(color1);
    palette[1] = Color::from(color2);
    palette[2] = Color::from(color3);
    palette[3] = Color::from(color4);
    debug!("BG Palette is now {:?}", palette);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Color {
    White = 0,
    LightGray = 1,
    DarkGray = 2,
    Black = 3,
}

impl From<u8> for Color {
    fn from(b: u8) -> Self {
        match b {
            0 => Self::White,
            1 => Self::LightGray,
            2 => Self::DarkGray,
            3 => Self::Black,
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitvec() {
        let b: u8 = 0b00000110;
        let bits = b.view_bits::<Lsb0>();

        // 0b10
        assert_eq!(bits[0..=1].load::<u8>(), 2);
        // 0b110
        assert_eq!(bits[0..=2].load::<u8>(), 6);
        // 0b11
        assert_eq!(bits[1..=2].load::<u8>(), 3);
        // 0b011
        assert_eq!(bits[1..=3].load::<u8>(), 3);
    }
}
