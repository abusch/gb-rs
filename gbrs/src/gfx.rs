use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use bitvec::prelude::*;
use log::trace;

use crate::{FrameSink, Rgb555, SCREEN_HEIGHT, SCREEN_WIDTH, interrupt::InterruptFlag};

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

// STAT interrupt sources, as laid out in the STAT register
const STAT_HBLANK: u8 = 1 << 3;
const STAT_VBLANK: u8 = 1 << 4;
const STAT_OAM: u8 = 1 << 5;
const STAT_LYC: u8 = 1 << 6;
const STAT_SOURCES: u8 = STAT_HBLANK | STAT_VBLANK | STAT_OAM | STAT_LYC;

const DOTS_PER_LINE: u16 = 456;
/// 144 visible scanlines followed by 10 of VBlank
const LINES_PER_FRAME: u8 = 154;
/// Mode 2 (OAM scan) runs for the first 80 dots of a visible line, then mode 3 (drawing).
const MODE3_START_DOT: u16 = 80;
/// Mode 0 (HBlank) runs from here until the end of the line.
const MODE0_START_DOT: u16 = 252;

/// Colours used for the 4 DMG shades (white to black) unless a frontend picks its own.
pub const DEFAULT_DMG_PALETTE: [Rgb555; 4] = [
    Rgb555::from_rgb888(0xe0, 0xf8, 0xd0),
    Rgb555::from_rgb888(0x88, 0xc0, 0x70),
    Rgb555::from_rgb888(0x30, 0x68, 0x50),
    Rgb555::from_rgb888(0x08, 0x18, 0x20),
];

#[derive(Debug)]
pub struct Gfx {
    vram: Box<[u8]>,
    oam_ram: Box<[u8]>,

    /// Represents the LCD itself, i.e. where pixels are actually written.
    ///
    /// Each pixel is a 15-bit colour, so the same buffer can hold CGB output later on.
    lcd: Box<[Rgb555]>,
    /// Colours the 4 DMG shades are rendered with.
    dmg_palette: [Rgb555; 4],

    /// Number of clock cycles since we began rendering the current scanline
    line_dot: u16,
    running_mode: Mode,

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

    /// STAT interrupt sources that are enabled, as `STAT_*` bits
    stat_sources: u8,
    /// STAT interrupt conditions that currently hold, as `STAT_*` bits
    stat_conditions: u8,

    /// BG Palette
    bgp: Palette,
    /// OBJ Palette 0
    obp0: Palette,
    /// OBJ Palette 1
    obp1: Palette,

    // Window internal line counter
    window_internal_line_counter: u8,
}

impl Gfx {
    pub fn new() -> Self {
        Self {
            vram: vec![0; 8 * 1024].into_boxed_slice(),
            oam_ram: vec![0; 0xA0].into_boxed_slice(),
            lcd: vec![Rgb555::default(); SCREEN_WIDTH * SCREEN_HEIGHT].into_boxed_slice(),
            dmg_palette: DEFAULT_DMG_PALETTE,
            line_dot: 0,
            running_mode: Mode::Mode2,
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
            stat_sources: 0,
            stat_conditions: 0,
            window_internal_line_counter: 0,
        }
    }

    /// Leave VRAM and the LCD registers the way the DMG boot ROM does: the cartridge's logo
    /// followed by a ® in the middle of the background, with the LCD on.
    pub(crate) fn skip_boot(&mut self, logo: &[u8; 48]) {
        // Each nibble of the logo becomes an 8-pixel row by doubling every bit, and each row is
        // repeated to double the height too. Only the low bitplane is set (colour 1).
        let mut addr = 0x8010;
        for nibble in logo.iter().flat_map(|b| [b >> 4, b & 0x0F]) {
            let row = (0..4)
                .rev()
                .fold(0u8, |row, bit| (row << 2) | (((nibble >> bit) & 1) * 0b11));
            self.write_vram(addr, row);
            self.write_vram(addr + 2, row);
            addr += 4;
        }
        // The ® tile directly follows the 24 logo tiles, i.e. it's tile 0x19.
        for row in [0x3C, 0x42, 0xB9, 0xA5, 0xB9, 0xA5, 0x42, 0x3C] {
            self.write_vram(addr, row);
            addr += 2;
        }

        // The logo is 12x2 tiles, with the ® to the right of its top row.
        for i in 0..12 {
            self.write_vram(0x9904 + i as u16, 0x01 + i);
            self.write_vram(0x9924 + i as u16, 0x0D + i);
        }
        self.write_vram(0x9910, 0x19);

        self.write_reg(BGP_REG, 0xFC);
        self.write_reg(LCDC_REG, 0x91);
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
            // FF44 LY is read-only
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
        // bit 7 is always 1
        let lyc_eq_ly = if self.ly == self.lyc { 0b100 } else { 0 };
        0x80 | self.stat_sources | lyc_eq_ly | self.running_mode as u8
    }

    fn set_stat(&mut self, stat: u8) {
        self.stat_sources = stat & STAT_SOURCES;
    }

    pub(crate) fn dots(&mut self, cycles: u8, frame_sink: &mut dyn FrameSink) -> InterruptFlag {
        let mut interrupt = InterruptFlag::empty();
        let mut remaining = cycles as u16;
        while remaining > 0 {
            // The first dot picks up any register writes made since the last call.
            interrupt |= self.dot(frame_sink);
            remaining -= 1;
            // After that, dots before the next mode change only advance the dot counter: LY, LYC
            // and the mode all stay put, so the STAT line can't rise either.
            let skip = (self.next_mode_change_dot() - self.line_dot - 1).min(remaining);
            self.line_dot += skip;
            remaining -= skip;
        }

        interrupt
    }

    /// The next value of `line_dot` at which `dot` changes mode or starts a new line.
    fn next_mode_change_dot(&self) -> u16 {
        if self.ly >= SCREEN_HEIGHT as u8 || self.line_dot >= MODE0_START_DOT {
            DOTS_PER_LINE
        } else if self.line_dot >= MODE3_START_DOT {
            MODE0_START_DOT
        } else {
            MODE3_START_DOT
        }
    }

    /// Run the graphics subsystem for one clock cycle (or _dot_)
    #[inline(always)]
    fn dot(&mut self, frame_sink: &mut dyn FrameSink) -> InterruptFlag {
        let mut interrupts = InterruptFlag::empty();
        let stat_line = self.stat_line();

        // The mode only ever changes at a handful of fixed dots, so only do work at those.
        self.line_dot += 1;
        if self.line_dot == DOTS_PER_LINE {
            self.line_dot = 0;
            self.ly = if self.ly == LINES_PER_FRAME - 1 {
                0
            } else {
                self.ly + 1
            };
            if self.ly == SCREEN_HEIGHT as u8 {
                // VBlank
                self.running_mode = Mode::Mode1;
                if self.lcd_and_ppu_enabled {
                    frame_sink.push_frame(&self.lcd);
                }
                interrupts |= InterruptFlag::VBLANK;
                // Reset the window internal line counter
                self.window_internal_line_counter = 0;
            } else if self.ly < SCREEN_HEIGHT as u8 {
                // OAM scan
                self.running_mode = Mode::Mode2;
            }
        } else if self.ly < SCREEN_HEIGHT as u8 {
            if self.line_dot == MODE3_START_DOT {
                // Drawing
                self.running_mode = Mode::Mode3;
                self.draw_scan_line();
            } else if self.line_dot == MODE0_START_DOT {
                // HBlank
                self.running_mode = Mode::Mode0;
            }
        }

        let lyc_eq_ly = if self.ly == self.lyc { STAT_LYC } else { 0 };
        self.stat_conditions = self.running_mode.stat_condition() | lyc_eq_ly;

        let new_stat_line = self.stat_line();
        // A STAT interrupt will be triggered by a rising edge (transition from low to high) on the
        // STAT interrupt line.
        if !stat_line && new_stat_line {
            // trace!("Rising edge of the STAT itr line detected: requesting STAT interrupt");
            interrupts |= InterruptFlag::STAT;
        }

        // Only raise interrupts requests if the LCD is on
        if self.lcd_and_ppu_enabled {
            interrupts
        } else {
            InterruptFlag::empty()
        }
    }

    // Only runs once per line: keep it out of `dots`, which runs every M-cycle and would otherwise
    // pay for this function's register and stack usage on every call.
    #[inline(never)]
    fn draw_scan_line(&mut self) {
        let mut drawn_from_window = false;
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
        let sprite_pixels = if self.obj_enabled {
            self.sprite_pixels_for_scanline(self.ly)
        } else {
            [None; SCREEN_WIDTH]
        };

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
                drawn_from_window = true;
                (
                    lcd_x + 7 - self.wx,
                    self.window_internal_line_counter,
                    win_tilemap_area,
                )
            } else {
                // we're in the background
                // BG is a 256x256 torus, so scroll offsets wrap around u8.
                (
                    lcd_x.wrapping_add(self.scx),
                    lcd_y.wrapping_add(self.scy),
                    bg_tilemap_area,
                )
            };

            let color_byte = if self.bg_and_window_enable {
                self.bg_pixel(tilemap_area, bg_x, bg_y)
            } else {
                0
            };

            let final_color = match sprite_pixels[x as usize] {
                Some((p, bg_has_priority)) if !(bg_has_priority && color_byte != 0) => p,
                _ => self.bgp[color_byte as usize],
            };

            self.write_pixel(x, self.ly, final_color);
        }

        if drawn_from_window {
            self.window_internal_line_counter += 1;
        }
    }

    /// Colour index (0-3) of the background/window pixel at the given coordinates in the 256x256
    /// area covered by the tilemap at `tilemap_area`.
    fn bg_pixel(&self, tilemap_area: u16, bg_x: u8, bg_y: u8) -> u8 {
        // Coordinates in "tilemap space" (i.e. 32x32)
        let (tilemap_x, tilemap_y) = (bg_x / 8, bg_y / 8);
        let tile_id =
            self.read_vram_internal(tilemap_area + (tilemap_y as u16 * 32 + tilemap_x as u16));

        // Now that we've got the tileid, look up the tile data in the appropriate location.

        // Coordinates in "tile space" (i.e. which pixel of an 8x8 tile to draw)
        let (tile_col, tile_row) = (bg_x % 8, bg_y % 8);

        let tile_offset: u16 = if self.bg_and_window_tile_data_area {
            let base = VRAM_TILE_DATA_BLOCK_0_ADDR;
            // treat tile id as unsigned
            base + 16 * tile_id as u16
        } else {
            let base = VRAM_TILE_DATA_BLOCK_2_ADDR;
            // treat tile id as *signed*, so sign-extend it to 16 bits
            let signed_id = tile_id as i8 as i16;
            let offset = (16 * signed_id) as u16;

            base.wrapping_add(offset)
        };
        let row = self.tile_row_at(tile_offset + 2 * tile_row as u16);
        tile_pixel(row, tile_col)
    }

    /// The two bitplanes (low, high) of the tile row at `addr`.
    fn tile_row_at(&self, addr: u16) -> (u8, u8) {
        (
            self.read_vram_internal(addr),
            self.read_vram_internal(addr + 1),
        )
    }

    /// The two bitplanes (low, high) of the given row of a sprite's tile(s).
    fn sprite_tile_row(&self, sprite: &Sprite, tile_y: u8) -> (u8, u8) {
        let (tile_index, tile_y) = if self.obj_size {
            // 8x16 sprites use an upper tile and a lower tile
            if tile_y < 8 {
                (sprite.tile_index & 0xFE, tile_y)
            } else {
                (sprite.tile_index | 0x01, tile_y - 8)
            }
        } else {
            (sprite.tile_index, tile_y)
        };
        // Sprites always use block 0 with an unsigned tile id
        self.tile_row_at(VRAM_TILE_DATA_BLOCK_0_ADDR + 16 * tile_index as u16 + 2 * tile_y as u16)
    }

    /// The sprite pixel (if any) to draw at each x of line `y`, along with whether the background
    /// has priority over it.
    fn sprite_pixels_for_scanline(&self, y: u8) -> [Option<(Color, bool)>; SCREEN_WIDTH] {
        let mut sprites = [Sprite::default(); 40];
        let mut count = 0;
        for data in self.oam_ram.as_chunks::<4>().0 {
            let sprite = Sprite::new(data);
            if sprite.matches_scanline(y, self.obj_size) {
                sprites[count] = sprite;
                count += 1;
            }
        }
        // Order the sprites by smallest `x` as they have higher priority, and only draw the first
        // 10.
        let sprites = &mut sprites[..count];
        sprites.sort_by_key(|s| s.x);

        let y_size = if self.obj_size { 16 } else { 8 };
        let mut pixels = [None; SCREEN_WIDTH];
        for sprite in sprites.iter().take(10) {
            let mut tile_y = y + 16 - sprite.y;
            if sprite.is_y_flip() {
                tile_y = y_size - 1 - tile_y;
            }
            let row = self.sprite_tile_row(sprite, tile_y);
            let palette = self.sprite_palette(sprite);

            // Sprite x is offset by 8, so that sprites can be partially off the left edge.
            for tile_x in 0..8u8 {
                let lcd_x = sprite.x as usize + tile_x as usize;
                let Some(lcd_x) = lcd_x.checked_sub(8).filter(|&x| x < SCREEN_WIDTH) else {
                    continue;
                };
                // A higher priority sprite already has an opaque pixel here
                if pixels[lcd_x].is_some() {
                    continue;
                }
                let x = if sprite.is_x_flip() {
                    7 - tile_x
                } else {
                    tile_x
                };
                // Color index 0 is transparent for sprites
                let color = tile_pixel(row, x);
                if color != 0 {
                    pixels[lcd_x] = Some((palette[color as usize], sprite.bg_has_priority()));
                }
            }
        }
        pixels
    }

    fn sprite_palette(&self, sprite: &Sprite) -> &Palette {
        if sprite.obp1_palette() {
            &self.obp1
        } else {
            &self.obp0
        }
    }

    fn get_sprite_color(&self, sprite: &Sprite, tile_x: u8, tile_y: u8) -> Option<Color> {
        // Color index 0 is transparent for sprites
        match tile_pixel(self.sprite_tile_row(sprite, tile_y), tile_x) {
            0 => None,
            color => Some(self.sprite_palette(sprite)[color as usize]),
        }
    }

    fn write_pixel(&mut self, x: u8, y: u8, color: Color) {
        self.lcd[y as usize * SCREEN_WIDTH + x as usize] = self.dmg_color(color);
    }

    fn dmg_color(&self, color: Color) -> Rgb555 {
        self.dmg_palette[color.as_u8() as usize]
    }

    pub(crate) fn set_dmg_palette(&mut self, palette: [Rgb555; 4]) {
        self.dmg_palette = palette;
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
                let (r, g, b) = self.dmg_color(pixel).to_rgb888();
                print!("{}", ansi_term::Color::RGB(r, g, b).paint("██"));
            }
            println!();
        }
    }

    pub fn dump_palettes(&self) {
        println!("BGP:  {}", self.bgp.to_debug_str(&self.dmg_palette));
        println!("OBP0: {}", self.obp0.to_debug_str(&self.dmg_palette));
        println!("OBP1: {}", self.obp1.to_debug_str(&self.dmg_palette));
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
    #[inline(always)]
    fn stat_line(&self) -> bool {
        self.stat_sources & self.stat_conditions != 0
    }
}

/// Colour index (0-3) of pixel `x` (0 being the leftmost) of a tile row, given as its two
/// bitplanes (low, high).
fn tile_pixel((lo, hi): (u8, u8), x: u8) -> u8 {
    let bit = 7 - x;
    (((hi >> bit) & 1) << 1) | ((lo >> bit) & 1)
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

impl Mode {
    /// The STAT interrupt condition that holds while in this mode, as a `STAT_*` bit.
    fn stat_condition(self) -> u8 {
        match self {
            Mode::Mode0 => STAT_HBLANK,
            Mode::Mode1 => STAT_VBLANK,
            Mode::Mode2 => STAT_OAM,
            Mode::Mode3 => 0,
        }
    }
}

#[derive(Clone, Copy, Default)]
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
    fn to_debug_str(&self, dmg_palette: &[Rgb555; 4]) -> String {
        let mut s = String::new();
        for c in self.0 {
            let (r, g, b) = dmg_palette[c.as_u8() as usize].to_rgb888();
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
