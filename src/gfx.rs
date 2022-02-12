use bitvec::prelude::*;
use log::debug;

const LCDC_REG: u16 = 0xFF40;
const SCY_REG: u16 = 0xFF42;
const SCX_REG: u16 = 0xFF43;
const LY_REG: u16 = 0xFF44;
const LYC_REG: u16 = 0xFF45;
const BGP_REG: u16 = 0xFF47;
const OBP0_REG: u16 = 0xFF48;
const OBP1_REG: u16 = 0xFF49;
const WY_REG: u16 = 0xFF4A;
const WX_REG: u16 = 0xFF4B;

#[derive(Debug)]
pub struct Gfx {
    pub vram: Box<[u8]>,
    /// LCDC (LCD Control)
    lcdc: u8,

    /// SCY (Scroll Y)
    scy: u8,
    /// SCX (Scroll X)
    scx: u8,

    /// LY (LCD Y Coordinate)
    ly: u8,
    /// LYC (LY Compare)
    lyc: u8,

    /// WY (Window Y Position)
    wy: u8,
    /// WX (Window X Position + 7)
    wx: u8,

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
            scy: 0,
            scx: 0,
            bgp: [Color::White; 4],
            obp0: [Color::White; 4],
            obp1: [Color::White; 4],
            ly: 0,
            lyc: 0,
            wy: 0,
            wx: 0,
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        if addr == LCDC_REG {
            self.lcdc
        } else if addr == SCY_REG {
            // FF42 SCY
            self.scy
        } else if addr == SCX_REG {
            // FF43 SCX
            self.scx
        } else if addr == LY_REG {
            // FF44 LY
            self.ly
        } else if addr == LYC_REG {
            // FF45 LYC
            self.lyc
        } else if addr == WY_REG {
            // FF4A WY
            self.wy
        } else if addr == WX_REG {
            // FF4B WX
            self.wx
        } else if addr == BGP_REG {
            // FF47 - BGP (BG Palette Data)
            get_palette_as_byte(&self.bgp)
        } else if addr == OBP0_REG {
            get_palette_as_byte(&self.obp0)
        } else if addr == OBP1_REG {
            get_palette_as_byte(&self.obp1)
        } else {
            unimplemented!();
        }
    }
    pub fn write(&mut self, addr: u16, b: u8) {
        if addr == LCDC_REG {
            self.lcdc = b;
        } else if addr == SCY_REG {
            // FF42 SCY
            self.scy = b;
        } else if addr == SCX_REG {
            // FF43 SCX
            self.scx = b;
        } else if addr == LY_REG {
            // FF44 LY
            self.ly = b;
        } else if addr == LYC_REG {
            // FF45 LYC
            self.lyc = b;
        } else if addr == WY_REG {
            // FF4A WY
            self.wy = b;
        } else if addr == WX_REG {
            // FF4B WX
            self.wx = b;
        } else if addr == BGP_REG {
            // FF47 - BGP (BG Palette Data)
            set_palette_data(&mut self.bgp, b);
        } else if addr == OBP0_REG {
            set_palette_data(&mut self.obp0, b);
        } else if addr == OBP1_REG {
            set_palette_data(&mut self.obp1, b);
        } else {
            unimplemented!();
        }
    }
}

fn get_palette_as_byte(palette: &[Color; 4]) -> u8 {
    let mut byte = 0u8;
    let bits = byte.view_bits_mut::<Lsb0>();

    bits.chunks_mut(2).zip(palette.iter()).for_each(|(chunk, color)| {
        let color_byte = color.as_u8();
        let color_bits = color_byte.view_bits::<Lsb0>();
        chunk.set(0, color_bits[0]);
        chunk.set(1, color_bits[1]);
    });

    bits.load::<u8>()
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

impl Color {
    fn as_u8(&self) -> u8 {
        match self {
            Color::White => 0,
            Color::LightGray => 1,
            Color::DarkGray => 2,
            Color::Black => 3,
        }
    }
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

    #[test]
    fn test_get_palette_data() {
        let palette = [Color::White, Color::LightGray, Color::DarkGray, Color::Black];

        assert_eq!(0b11100100, get_palette_as_byte(&palette));
    }
}
