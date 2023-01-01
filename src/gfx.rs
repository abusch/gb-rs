use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use bitvec::prelude::*;
use log::trace;

use crate::{interrupt::InterruptFlag, FrameSink, SCREEN_HEIGHT, SCREEN_WIDTH};

const VRAM_START: u16 = 0x8000;
const OAM_START: u16 = 0xFE00;

const VRAM_TILE_DATA_BLOCK_0_ADDR: u16 = 0x8000;
// const VRAM_TILE_DATA_BLOCK_1_ADDR: u16 = 0x8800;
const VRAM_TILE_DATA_BLOCK_2_ADDR: u16 = 0x9000;

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
    lcd: Box<[(u8, u8, u8)]>,

    /// Number of clock cycles since we began rendering the current frame
    dots: usize,
    running_mode: Mode,
    line_drawing_state: LineDrawingState,

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
    bg_and_window_enable: bool,

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

    // STAT interrupt sources
    stat_lyc_eq_ly_itr_source: bool,
    stat_oam_itr_source: bool,
    stat_vblank_itr_source: bool,
    stat_hblank_itr_source: bool,

    // STAT interrupt values
    stat_lyc_eq_ly_active: bool,
    stat_oam_active: bool,
    stat_vblank_active: bool,
    stat_hblank_active: bool,

    /// BG Palette
    bgp: Palette,
    /// OBJ Palette 0
    obp0: Palette,
    /// OBJ Palette 1
    obp1: Palette,
}

impl Gfx {
    pub fn new() -> Self {
        Self {
            vram: vec![0; 8 * 1024].into_boxed_slice(),
            oam_ram: vec![0; 0xA0].into_boxed_slice(),
            lcd: vec![(0, 0, 0); SCREEN_WIDTH * SCREEN_HEIGHT].into_boxed_slice(),
            dots: 0,
            running_mode: Mode::Mode2,
            line_drawing_state: LineDrawingState::Idle,
            // TODO should it be exploded into individual flags?
            lcd_and_ppu_enabled: false,
            window_tile_map_area: false,
            window_enable: false,
            bg_and_window_tile_data_area: false,
            bg_tile_map_area: false,
            obj_size: false,
            obj_enabled: false,
            bg_and_window_enable: false,
            scy: 0,
            scx: 0,
            bgp: Palette([Color::White; 4]),
            obp0: Palette([Color::White; 4]),
            obp1: Palette([Color::White; 4]),
            ly: 0,
            lyc: 0,
            wy: 0,
            wx: 0,
            stat_lyc_eq_ly_itr_source: false,
            stat_oam_itr_source: false,
            stat_vblank_itr_source: false,
            stat_hblank_itr_source: false,
            stat_lyc_eq_ly_active: false,
            stat_oam_active: false,
            stat_vblank_active: false,
            stat_hblank_active: false,
        }
    }

    /// Read access to the VRAM.
    ///
    /// Note: when the PPU is active (mode 3), this area is locked to the CPU so reads will return
    /// 0xFF in that case.
    pub fn read_vram(&self, addr: u16) -> u8 {
        if self.running_mode != Mode::Mode3 || !self.lcd_and_ppu_enabled {
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
        if self.running_mode != Mode::Mode3 || !self.lcd_and_ppu_enabled {
            self.vram[(addr - VRAM_START) as usize] = b;
        }
    }

    pub fn read_oam(&self, addr: u16) -> u8 {
        if !self.lcd_and_ppu_enabled
            || (self.running_mode != Mode::Mode2 && self.running_mode != Mode::Mode3)
        {
            self.oam_ram[(addr - OAM_START) as usize]
        } else {
            0xff
        }
    }

    pub fn write_oam(&mut self, addr: u16, b: u8) {
        if !self.lcd_and_ppu_enabled
            || (self.running_mode != Mode::Mode2 && self.running_mode != Mode::Mode3)
        {
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
            bits.set(0, self.bg_and_window_enable);

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
            // CGB-only registers, so just ignore for now
            // warn!("unimplemented register 0x{:04x}", addr);
            0xFF
        }
    }

    pub fn write_reg(&mut self, addr: u16, b: u8) {
        if addr == LCDC_REG {
            let orig_lcd_state = self.lcd_and_ppu_enabled;
            let bits = b.view_bits::<Lsb0>();
            self.lcd_and_ppu_enabled = bits[7];
            self.window_tile_map_area = bits[6];
            self.window_enable = bits[5];
            self.bg_and_window_tile_data_area = bits[4];
            self.bg_tile_map_area = bits[3];
            self.obj_size = bits[2];
            self.obj_enabled = bits[1];
            self.bg_and_window_enable = bits[0];
            trace!("LCDC reg = 0b{:b}", b);
            if orig_lcd_state && !self.lcd_and_ppu_enabled {
                trace!("LCD turned OFF!");
            } else if !orig_lcd_state && self.lcd_and_ppu_enabled {
                trace!("LCD turned ON!");
            }
        } else if addr == STAT_REG {
            self.set_stat(b);
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
            trace!("Setting WY={}", self.wy);
        } else if addr == WX_REG {
            // FF4B WX
            self.wx = b;
            trace!("Setting WX={}", self.wx);
        } else if addr == BGP_REG {
            // FF47 - BGP (BG Palette Data)
            set_palette_data(&mut self.bgp, b);
        } else if addr == OBP0_REG {
            set_palette_data(&mut self.obp0, b);
        } else if addr == OBP1_REG {
            set_palette_data(&mut self.obp1, b);
        } else {
            // CGB-only registers, so just ignore for now
            // warn!("unimplemented register 0x{:04x}", addr);
        }
    }

    /// Return the value of the STAT register (FF41)
    fn stat(&self) -> u8 {
        let mut byte = 0u8;
        let bits = byte.view_bits_mut::<Lsb0>();

        // interrupt sources
        bits.set(6, self.stat_lyc_eq_ly_itr_source);
        bits.set(5, self.stat_oam_itr_source);
        bits.set(4, self.stat_vblank_itr_source);
        bits.set(3, self.stat_hblank_itr_source);

        bits.set(2, self.ly == self.lyc);

        let mode = self.running_mode as u8;
        let mode_bits = mode.view_bits::<Lsb0>();
        bits.set(1, mode_bits[1]);
        bits.set(0, mode_bits[0]);

        bits.load()
    }

    fn set_stat(&mut self, stat: u8) {
        let bits = stat.view_bits::<Lsb0>();
        self.stat_lyc_eq_ly_itr_source = bits[6];
        self.stat_oam_itr_source = bits[5];
        self.stat_vblank_itr_source = bits[4];
        self.stat_hblank_itr_source = bits[3];
    }

    pub(crate) fn dots(&mut self, cycles: u8, frame_sink: &mut dyn FrameSink) -> InterruptFlag {
        let mut interrupt = InterruptFlag::empty();
        for _ in 0..cycles {
            interrupt |= self.dot(frame_sink);
        }

        interrupt
    }

    /// Run the graphics subsystem for one clock cycle (or _dot_)
    fn dot(&mut self, frame_sink: &mut dyn FrameSink) -> InterruptFlag {
        let mut interrupts = InterruptFlag::empty();
        let stat_line = self.stat_line();

        self.dots += 1;
        // Each scanline takes 456 dots
        let mut scanline = (self.dots / 456) as u8;
        let line_dot = (self.dots % 456) as u16;

        // A whole frame (drawing + VSync) is 153 scanlines
        if scanline > 153 {
            self.dots = line_dot as usize;
            scanline = 0;
        }
        self.ly = scanline;
        self.stat_lyc_eq_ly_active = self.ly == self.lyc;

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

        self.stat_hblank_active = self.running_mode == Mode::Mode0;
        self.stat_vblank_active = self.running_mode == Mode::Mode1;
        self.stat_oam_active = self.running_mode == Mode::Mode2;
        match self.running_mode {
            Mode::Mode0 => {
                if self.line_drawing_state == LineDrawingState::Drawing {
                    self.line_drawing_state = LineDrawingState::Idle;
                }
            }
            Mode::Mode1 => {
                if self.line_drawing_state == LineDrawingState::Idle {
                    if self.lcd_and_ppu_enabled {
                        frame_sink.push_frame(&self.lcd);
                    }
                    interrupts |= InterruptFlag::VBLANK;
                    self.line_drawing_state = LineDrawingState::FramePushed;
                }
            }
            Mode::Mode2 => {
                if self.line_drawing_state == LineDrawingState::Idle
                    || self.line_drawing_state == LineDrawingState::FramePushed
                {
                    // OAM scan
                    self.line_drawing_state = LineDrawingState::OamScan;
                }
            }
            Mode::Mode3 => {
                if self.line_drawing_state == LineDrawingState::OamScan {
                    self.line_drawing_state = LineDrawingState::Drawing;
                    self.draw_scan_line();
                }
            }
        }

        let new_stat_line = self.stat_line();
        // A STAT interrupt will be triggered by a rising edge (transition from low to high) on the
        // STAT interrupt line.
        if !stat_line && new_stat_line {
            trace!("Rising edge of the STAT itr line detected: requesting STAT interrupt");
            interrupts |= InterruptFlag::STAT;
        }

        // Only raise interrupts requests if the LCD is on
        if self.lcd_and_ppu_enabled {
            interrupts
        } else {
            InterruptFlag::empty()
        }
    }

    fn draw_scan_line(&mut self) {
        let bg_tilemap_area = if self.bg_tile_map_area {
            0x9C00
        } else {
            0x9800
        };
        let win_tilemap_area = if self.window_tile_map_area {
            0x9C00
        } else {
            0x9800
        };
        // Get the first 10 sprites that match the current line
        let sprites = self.get_sprites_for_scanline(self.ly);

        // Render a line of pixels
        for x in 0..SCREEN_WIDTH as u8 {
            // Coordinates in "LCD space" (i.e 160x144)
            let (lcd_x, lcd_y) = (x, self.ly);
            // Coordinates in "Background area" space (i.e 256x256)
            let (bg_x, bg_y, tilemap_area) = if self.bg_and_window_enable
                && self.window_enable
                && lcd_x + 7 >= self.wx
                && lcd_y >= self.wy
            {
                // We're in the window
                (lcd_x + 7 - self.wx, lcd_y - self.wy, win_tilemap_area)
            } else {
                // we're in the background

                (lcd_x + self.scx, lcd_y + self.scy, bg_tilemap_area)
            };
            // Coordinates in "tilemap space" (i.e. 32x32)
            let (tilemap_x, tilemap_y) = (bg_x / 8, bg_y / 8);

            let tile_id =
                self.read_vram_internal(tilemap_area + (tilemap_y as u16 * 32 + tilemap_x as u16));

            // Now that we've got the tileid, look up the tile data in the appropriate location.

            // Coordinates in "tile space" (i.e. which pixel of an 8x8 tile to draw)
            let (tile_col, tile_row) = (bg_x % 8, bg_y % 8);

            let mut tile_offset: u16 = if self.bg_and_window_tile_data_area {
                let base = VRAM_TILE_DATA_BLOCK_0_ADDR;
                // treat tile id as unsigned
                base + 16 * tile_id as u16
            } else {
                let base = VRAM_TILE_DATA_BLOCK_2_ADDR;
                // treat tile id as *signed*, so sign-extend it to 16 bits
                let signed_id = tile_id as i8 as i16;
                let offset = (16 * signed_id) as u16;

                base.wrapping_add(offset)
                // (base + 16 * signed_id as i16) as u16
            };
            tile_offset += 2 * tile_row as u16;

            let lo_byte = self.read_vram_internal(tile_offset);
            let hi_byte = self.read_vram_internal(tile_offset + 1);

            let mut color_byte = 0u8;
            if self.bg_and_window_enable {
                let color_bits = color_byte.view_bits_mut::<Lsb0>();
                // Use Msb0 order here as pixel 0 is the leftmost bit (bit 7).
                color_bits.set(1, hi_byte.view_bits::<Msb0>()[tile_col as usize]);
                color_bits.set(0, lo_byte.view_bits::<Msb0>()[tile_col as usize]);
            }
            // Background / window pixel
            // let color = if self.bg_and_window_enable {
            //     self.bgp[color_byte as usize]
            // } else {
            //     Color::White
            // };

            // Potential sprite pixel
            let sprite_pixel_and_bg_has_priority = sprites.iter().find_map(|s| {
                self.get_sprite_pixel(s, lcd_x, lcd_y)
                    .map(|p| (p, s.bg_has_priority()))
            });

            let final_color = match (self.obj_enabled, sprite_pixel_and_bg_has_priority) {
                (true, Some((p, true))) => {
                    if color_byte != 0 {
                        self.bgp[color_byte as usize]
                    } else {
                        p
                    }
                }
                (true, Some((p, false))) => p,
                (true, None) => self.bgp[color_byte as usize],
                (false, _) => self.bgp[color_byte as usize],
            };

            self.write_pixel(x, self.ly, final_color);
        }
    }

    fn get_block0_tile_data(&self, tile_id: u8, tile_row: u8) -> (u8, u8) {
        let base = VRAM_TILE_DATA_BLOCK_0_ADDR;
        // treat tile id as unsigned
        let mut tile_offset = base + 16 * tile_id as u16;
        tile_offset += 2 * tile_row as u16;
        let hi_byte = self.read_vram_internal(tile_offset);
        let lo_byte = self.read_vram_internal(tile_offset + 1);

        (lo_byte, hi_byte)
    }

    fn get_sprites_for_scanline(&self, y: u8) -> Vec<Sprite> {
        let sprites = self
            .oam_ram
            .chunks(4)
            .map(Sprite::new)
            .filter(|sprite| sprite.matches_scanline(y, self.obj_size))
            .take(10)
            .collect::<Vec<_>>();

        // Order the sprites by smallest `x` as they have hight priority
        // (&mut sprites[..]).sort_by(|s1, s2| s1.x.cmp(&s2.x));
        sprites
    }

    fn get_sprite_pixel(&self, sprite: &Sprite, x: u8, y: u8) -> Option<Color> {
        sprite
            .get_tile_coordinates(x, y, self.obj_size)
            .and_then(|(tile_x, tile_y)| self.get_sprite_color(sprite, tile_x, tile_y))
    }

    fn get_sprite_color(&self, sprite: &Sprite, tile_x: u8, tile_y: u8) -> Option<Color> {
        let (lo_byte, hi_byte) = if self.obj_size {
            // upper tile
            let tile_idx_1 = sprite.tile_index & 0xFE;
            // lower tile
            let tile_idx_2 = sprite.tile_index | 0x01;

            if tile_y < 8 {
                self.get_block0_tile_data(tile_idx_1, tile_y)
            } else {
                self.get_block0_tile_data(tile_idx_2, tile_y - 8)
            }
        } else {
            self.get_block0_tile_data(sprite.tile_index, tile_y)
        };

        let mut color_byte = 0u8;
        let color_bits = color_byte.view_bits_mut::<Lsb0>();
        // Use Msb0 order here as pixel 0 is the leftmost bit (bit 7).
        color_bits.set(1, lo_byte.view_bits::<Msb0>()[tile_x as usize]);
        color_bits.set(0, hi_byte.view_bits::<Msb0>()[tile_x as usize]);

        // Color index 0 is transparent for sprites
        if color_byte == 0 {
            None
        } else if sprite.obp1_palette() {
            Some(self.obp1[color_byte as usize])
        } else {
            Some(self.obp0[color_byte as usize])
        }
    }

    fn write_pixel(&mut self, x: u8, y: u8, color: Color) {
        self.lcd[y as usize * SCREEN_WIDTH + x as usize] = color.as_rgba();
    }

    pub fn dump_oam(&self) {
        println!("OAM:");
        self.oam_ram
            .chunks(4)
            .map(Sprite::new)
            .enumerate()
            .for_each(|(i, s)| println!("  {:02}: {:?}", i, s));
    }

    pub fn dump_sprite(&self, id: u8) {
        if id >= 40 {
            return;
        }

        let offset = id as usize * 4;
        let data = &self.oam_ram[offset..offset + 4];
        print!("Sprite data: ");
        data.iter().for_each(|b| print!("{:02x} ", b));
        println!("\n");

        let sprite = Sprite::new(data);

        let height = if self.obj_size { 16 } else { 8 };
        for y in 0..height {
            for x in 0..8 {
                let pixel = self.get_sprite_color(&sprite, x, y).unwrap_or(Color::White);
                let (r, g, b) = pixel.as_rgba();
                print!("{}", ansi_term::Color::RGB(r, g, b).paint("██"));
            }
            println!();
        }
    }

    pub fn dump_palettes(&self) {
        println!("BGP:  {}", self.bgp.to_debug_str());
        println!("OBP0: {}", self.obp0.to_debug_str());
        println!("OBP1: {}", self.obp1.to_debug_str());
    }

    /// Disable the LCD.
    ///
    /// This is meant to be called by the debugger to allow access to VRAM.
    pub(crate) fn disable(&mut self) {
        self.lcd_and_ppu_enabled = false;
    }

    /// Enable the LCD.
    ///
    /// This is meant to be called by the debugger before resuming normal running mode.
    pub(crate) fn enable(&mut self) {
        self.lcd_and_ppu_enabled = true;
    }

    /// The various STAT interrupt sources (modes 0-2 and LYC=LY) have their state (inactive=low
    /// and active=high) logically ORed into a shared “STAT interrupt line” if their respective
    /// enable bit is turned on.
    fn stat_line(&self) -> bool {
        let mut stat_line = false;
        if self.stat_oam_itr_source {
            stat_line |= self.stat_oam_active;
        }
        if self.stat_lyc_eq_ly_itr_source {
            stat_line |= self.stat_lyc_eq_ly_active;
        }
        if self.stat_hblank_itr_source {
            stat_line |= self.stat_hblank_active;
        }
        if self.stat_vblank_itr_source {
            stat_line |= self.stat_vblank_active;
        }
        stat_line
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

fn set_palette_data(palette: &mut Palette, b: u8) {
    trace!("Writing BG Palette with {:b}", b);
    let bits = b.view_bits::<Msb0>();
    let color0 = bits[6..=7].load::<u8>();
    let color1 = bits[4..=5].load::<u8>();
    let color2 = bits[2..=3].load::<u8>();
    let color3 = bits[0..=1].load::<u8>();

    palette[0] = Color::from(color0);
    palette[1] = Color::from(color1);
    palette[2] = Color::from(color2);
    palette[3] = Color::from(color3);
    trace!("BG Palette is now {:?}", palette);
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

    fn as_rgba(&self) -> (u8, u8, u8) {
        match self {
            Color::White => (0xe0, 0xf8, 0xd0),     // #e0f8d0
            Color::LightGray => (0x88, 0xc0, 0x70), // #88c070
            Color::DarkGray => (0x30, 0x68, 0x50),  // #306850
            Color::Black => (0x08, 0x18, 0x20),     // #081820
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
enum LineDrawingState {
    Idle,
    OamScan,
    Drawing,
    FramePushed,
}

struct Sprite {
    x: u8,
    y: u8,
    tile_index: u8,
    attrs: u8,
}

impl Sprite {
    pub fn new(data: &[u8]) -> Self {
        assert!(data.len() == 4);
        Self {
            y: data[0],
            x: data[1],
            tile_index: data[2],
            attrs: data[3],
        }
    }

    pub fn matches_scanline(&self, y: u8, double_size: bool) -> bool {
        let effective_y = y + 16;
        let top_y = self.y;
        let bottom_y = if double_size {
            top_y.wrapping_add(15)
        } else {
            top_y.wrapping_add(7)
        };

        (effective_y >= top_y) && (effective_y <= bottom_y)
    }

    /// Convert the given coordinates (in LCD space) into tile-space coordinates.
    pub fn get_tile_coordinates(&self, x: u8, y: u8, double_size: bool) -> Option<(u8, u8)> {
        let y_size = if double_size { 16 } else { 8 };
        let effective_x = x + 8;
        let effective_y = y + 16;
        let left_x = self.x;
        let right_x = self.x + 7;
        if (effective_x >= left_x) && (effective_x <= right_x) {
            let mut tile_x = effective_x - self.x;
            let mut tile_y = effective_y - self.y;

            if self.is_x_flip() {
                tile_x = 7 - tile_x;
            }
            if self.is_y_flip() {
                tile_y = y_size - 1 - tile_y;
            }

            Some((tile_x, tile_y))
        } else {
            None
        }
    }

    pub fn obp1_palette(&self) -> bool {
        self.attrs.view_bits::<Lsb0>()[4]
    }

    pub fn bg_has_priority(&self) -> bool {
        self.attrs.view_bits::<Lsb0>()[7]
    }

    pub fn is_y_flip(&self) -> bool {
        self.attrs.view_bits::<Lsb0>()[6]
    }

    pub fn is_x_flip(&self) -> bool {
        self.attrs.view_bits::<Lsb0>()[5]
    }
}

impl Debug for Sprite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sprite")
            .field("pos", &format_args!("({:3},{:3})", self.x, self.y))
            .field("tile_idx", &format_args!("{:02x}", self.tile_index))
            .field("attrs", &format_args!("{:08b}", self.attrs))
            .finish()
    }
}

#[derive(Debug)]
struct Palette([Color; 4]);

impl Palette {
    fn to_debug_str(&self) -> String {
        let mut s = String::new();
        for c in self.0 {
            let (r, g, b) = c.as_rgba();
            s.push_str(&format!("{}", ansi_term::Color::RGB(r, g, b).paint("██")));
        }
        s
    }
}

impl Deref for Palette {
    type Target = [Color; 4];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Palette {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
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
        let palette = [
            Color::White,
            Color::LightGray,
            Color::DarkGray,
            Color::Black,
        ];

        assert_eq!(0b11100100, get_palette_as_byte(&palette));
    }
}
