use bitvec::prelude::*;
use log::debug;

const SCREEN_WIDTH: usize = 160;
const SCREEN_HEIGHT: usize = 144;

const VRAM_START: u16 = 0x8000;
const OAM_START: u16 = 0xFE00;

const LCDC_REG: u16 = 0xFF40;
const STAT_REG: u16 = 0xFF41;
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
    vram: Box<[u8]>,
    oam_ram: Box<[u8]>,

    /// Represents the LCD itself, i.e. where pixels are actually written.
    ///
    /// Each pixel is in RGBA format.
    lcd: Box<[u8]>,

    /// Number of clock cycles since we began rendering the current frame
    dots: usize,
    running_mode: Mode,
    pixel_bg_fetcher_step: PixelFetcherStep,
    line_drawing_state: LineDrawingState,

    /// LCDC (LCD Control)
    lcdc: u8,
    // LCDC individual flags:
    /// LCDC.7
    lcd_and_ppu_enabled: bool,
    /// LCDC.6
    window_tile_map_area: bool,
    /// LCDC.5
    window_enable: bool,
    /// LCDC.4
    bg_and_window_tile_data_area: bool,
    /// LCDC.3
    bg_tile_map_area: bool,
    /// LCDC.2
    obj_size: bool,
    /// LCDC.1
    obj_enabled: bool,
    /// LCDC.0
    bg_and_window_enable_priority: bool,

    /// SCY (Scroll Y)
    scy: u8,
    /// SCX (Scroll X)
    scx: u8,

    /// LY (LCD Y Coordinate) == line currently being drawn
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
            vram: vec![0; 8 * 1024].into_boxed_slice(),
            oam_ram: vec![0; 0xA0].into_boxed_slice(),
            lcd: vec![0; SCREEN_WIDTH * SCREEN_HEIGHT * 4].into_boxed_slice(),
            dots: 0,
            running_mode: Mode::Mode2,
            pixel_bg_fetcher_step: PixelFetcherStep::GetTile,
            line_drawing_state: LineDrawingState::Idle,
            // TODO should it be exploded into individual flags?
            lcdc: 0,
            lcd_and_ppu_enabled: false,
            window_tile_map_area: false,
            window_enable: false,
            bg_and_window_tile_data_area: false,
            bg_tile_map_area: false,
            obj_size: false,
            obj_enabled: false,
            bg_and_window_enable_priority: false,
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

    /// Read access to the VRAM.
    ///
    /// Note: when the PPU is active (mode 3), this area is locked to the CPU so reads will return
    /// 0xFF in that case.
    pub fn read_vram(&self, addr: u16) -> u8 {
        if self.running_mode != Mode::Mode3 {
            self.read_vram_internal(addr)
        } else {
            0xff
        }
    }

    /// Read access to the VRAM from within the PPU
    fn read_vram_internal(&self, addr: u16) -> u8 {
        self.vram[(addr - VRAM_START) as usize]
    }

    pub fn write_vram(&mut self, addr: u16, b: u8) {
        if self.running_mode != Mode::Mode3 {
            self.vram[(addr - VRAM_START) as usize] = b;
        }
    }

    pub fn read_oam(&self, addr: u16) -> u8 {
        if self.running_mode != Mode::Mode2 && self.running_mode != Mode::Mode3 {
            self.oam_ram[(addr - OAM_START) as usize]
        } else {
            0xff
        }
    }

    pub fn write_oam(&mut self, addr: u16, b: u8) {
        if self.running_mode != Mode::Mode2 && self.running_mode != Mode::Mode3 {
            self.oam_ram[(addr - OAM_START) as usize] = b;
        }
    }

    pub fn read_reg(&self, addr: u16) -> u8 {
        if addr == LCDC_REG {
            let mut lcdc = 0u8;
            let bits = lcdc.view_bits_mut::<Lsb0>();
            bits.set(7, self.lcd_and_ppu_enabled);
            bits.set(6, self.window_tile_map_area);
            bits.set(5, self.window_enable);
            bits.set(4, self.bg_and_window_tile_data_area);
            bits.set(3, self.bg_tile_map_area);
            bits.set(2, self.obj_size);
            bits.set(1, self.obj_enabled);
            bits.set(0, self.bg_and_window_enable_priority);

            lcdc
        } else if addr == STAT_REG {
            // FF41 STAT
            self.stat()
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

    pub fn write_reg(&mut self, addr: u16, b: u8) {
        if addr == LCDC_REG {
            self.lcdc = b;
            let bits = self.lcdc.view_bits::<Lsb0>();
            self.lcd_and_ppu_enabled = bits[7];
            self.window_tile_map_area = bits[6];
            self.window_enable = bits[5];
            self.bg_and_window_tile_data_area = bits[4];
            self.bg_tile_map_area = bits[3];
            self.obj_size = bits[2];
            self.obj_enabled = bits[1];
            self.bg_and_window_enable_priority = bits[0];
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

    /// Return the value of the STAT register (FF41)
    fn stat(&self) -> u8 {
        // TODO interrupt sources
        let mut byte = 0u8;
        let bits = byte.view_bits_mut::<Lsb0>();
        bits.set(2, self.ly == self.lyc);

        let mode = self.running_mode as u8;
        let mode_bits = mode.view_bits::<Lsb0>();
        bits.set(1, mode_bits[1]);
        bits.set(0, mode_bits[0]);

        bits.load()
    }

    pub(crate) fn dots(&mut self, cycles: u8) {
        for _ in 0..cycles {
            self.dot();
        }
    }

    /// Run the graphics subsystem for one clock cycle (or _dot_)
    fn dot(&mut self) {
        self.dots += 1;
        let mut scanline = (self.dots / 456) as u8;
        let line_dot = (self.dots % 456) as u16;

        if scanline > 153 {
            self.dots = line_dot as usize;
            scanline = 0;
        }
        self.ly = scanline;

        if scanline > 143 {
            self.running_mode = Mode::Mode1;
        } else {
            self.running_mode = match line_dot {
                0..=79 => Mode::Mode2,
                80..=251 => Mode::Mode3,
                252..=455 => Mode::Mode0,
                // unreachable as we pattern match on the result of a modulo 456 operation
                _ => unreachable!("This shouldn't happen!"),
            }
        }

        match self.running_mode {
            Mode::Mode0 => {
                if self.line_drawing_state == LineDrawingState::Drawing {
                    self.line_drawing_state = LineDrawingState::Idle;
                }
            }
            Mode::Mode1 => (),
            Mode::Mode2 => {
                if self.line_drawing_state == LineDrawingState::Idle {
                    // OAM scan
                    self.line_drawing_state = LineDrawingState::OamScan;
                    // TODO do the actual scan
                }
            }
            Mode::Mode3 => {
                if self.line_drawing_state == LineDrawingState::OamScan {
                    self.line_drawing_state = LineDrawingState::Drawing;
                    self.draw_scan_line();
                }
            }
        }
    }

    fn draw_scan_line(&mut self) {
        // TODO handle window
        let tilemap_area = if self.bg_tile_map_area {
            0x9C00
        } else {
            0x9800
        };
        // (tile_x, tile_y) are the "tile coordinates", i.e which column/row in the 32x32 tile map.
        let tile_y = (self.scy + self.ly) / 8;
        // Render a line of pixels
        for x in 0..SCREEN_WIDTH as u8 {
            let tile_x = ((self.scx + x) / 8) & 0x1F;
            let tile_id = self.read_vram_internal(tilemap_area + (tile_y as u16 * 32 + tile_x as u16));
            // if tile_id != 0 {
            //     debug!(
            //         "pixel ({}, {}): tileid = {}",
            //         self.scy + self.ly,
            //         x,
            //         tile_id
            //         );
            // }

            // Now that we've got the tileid, look up the tile data in the appropriate location.

            let tile_offset: u16 = if self.bg_and_window_tile_data_area {
                let base = 0x8000;
                base + tile_id as u16
            } else {
                let base = 0x8800u16 as i16;
                let signed_id = tile_id as i8;
                (base + signed_id as i16) as u16
            };
            let hi_byte = self.read_vram_internal(tile_offset);
            let lo_byte = self.read_vram_internal(tile_offset + 1);

            // which line of the tile are we drawing?
            let tile_line = (self.scy + self.ly) % 8;
            let mut color_byte = 0u8;
            let color_bits = color_byte.view_bits_mut::<Lsb0>();
            color_bits.set(1, hi_byte.view_bits::<Lsb0>()[tile_line as usize]);
            color_bits.set(0, lo_byte.view_bits::<Lsb0>()[tile_line as usize]);
            let color = self.bgp[color_byte as usize];
            self.write_pixel(x, self.ly, color);
        }
    }

    fn write_pixel(&mut self, x: u8, y: u8, color: Color) {
        let offset = (y as usize * SCREEN_WIDTH + x as usize) * 4;
        self.lcd[offset..=offset+4].copy_from_slice(&color.as_rgba());
    }
}

fn get_palette_as_byte(palette: &[Color; 4]) -> u8 {
    let mut byte = 0u8;
    let bits = byte.view_bits_mut::<Lsb0>();

    bits.chunks_mut(2)
        .zip(palette.iter())
        .for_each(|(chunk, color)| {
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

    fn as_rgba(&self) -> [u8; 4] {
        match self {
            Color::White => [0xe0, 0xf8, 0xd0, 0xff],
            Color::LightGray => [0x88, 0xc0, 0x70, 0xff],
            Color::DarkGray => [0x30, 0x68, 0x50, 0xff],
            Color::Black => [0x08, 0x18, 0x20, 0xff],
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Mode {
    /// HSync
    Mode0 = 0,
    /// VSync
    Mode1 = 1,
    /// OAM scan
    Mode2 = 2,
    /// Drawing pixels
    Mode3 = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PixelFetcherStep {
    GetTile,
    GetTileDataLow,
    GetTileDataHigh,
    Sleep,
    Push,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineDrawingState {
    Idle,
    OamScan,
    Drawing,
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
        let palette = [
            Color::White,
            Color::LightGray,
            Color::DarkGray,
            Color::Black,
        ];

        assert_eq!(0b11100100, get_palette_as_byte(&palette));
    }
}
