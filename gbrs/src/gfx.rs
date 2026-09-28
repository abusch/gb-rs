use std::fmt::{Debug, Write};

use log::trace;
use serde::{Deserialize, Serialize};

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

// LCDC bits
const LCDC_ENABLE: u8 = 1 << 7;
const LCDC_WINDOW_TILE_MAP: u8 = 1 << 6;
const LCDC_WINDOW_ENABLE: u8 = 1 << 5;
/// Where the BG and window tiles are: block 0 with unsigned ids if set, block 2 with signed ones
/// otherwise.
const LCDC_TILE_DATA: u8 = 1 << 4;
const LCDC_BG_TILE_MAP: u8 = 1 << 3;
/// 8x16 sprites if set, 8x8 otherwise.
const LCDC_OBJ_SIZE: u8 = 1 << 2;
const LCDC_OBJ_ENABLE: u8 = 1 << 1;
const LCDC_BG_WINDOW_ENABLE: u8 = 1 << 0;

// STAT interrupt sources, as laid out in the STAT register
const STAT_HBLANK: u8 = 1 << 3;
const STAT_VBLANK: u8 = 1 << 4;
const STAT_OAM: u8 = 1 << 5;
const STAT_LYC: u8 = 1 << 6;
const STAT_SOURCES: u8 = STAT_HBLANK | STAT_VBLANK | STAT_OAM | STAT_LYC;

const DOTS_PER_LINE: u16 = 456;
/// 144 visible scanlines followed by 10 of VBlank
const LINES_PER_FRAME: u8 = 154;
/// LY is incremented this many dots before the OAM scan starts: for those dots, OAM is already
/// locked, but STAT (and its interrupt conditions) still say mode 0, and the LY=LYC comparison
/// isn't redone for the new LY yet. The STAT register shows the LY=LYC flag as clear then, but the
/// STAT interrupt line still sees the previous line's comparison, so there's no gap in it. Mooneye's
/// `lcdon_timing` measures the former, and Cool Hand needs the latter: with the LYC and mode 2
/// sources enabled, the line after LYC mustn't get a mode 2 interrupt.
const LINE_START_DOTS: u16 = 4;
/// Mode 2 (OAM scan) runs for 80 dots, then mode 3 (drawing).
const MODE3_START_DOT: u16 = LINE_START_DOTS + 80;
/// Mode 0 (HBlank) runs from here until the end of the line.
const MODE0_START_DOT: u16 = MODE3_START_DOT + 172;
/// At the end of the OAM scan, a little before STAT reports mode 3, VRAM reads get locked. OAM
/// writes get unlocked until drawing starts, like they are before the scan (mooneye's
/// `lcdon_timing` and `lcdon_write_timing` measure these).
const OAM_SCAN_END_DOT: u16 = MODE3_START_DOT - 4;

/// Colours used for the 4 DMG shades (white to black) unless a frontend picks its own.
pub const DEFAULT_DMG_PALETTE: [Rgb555; 4] = [
    Rgb555::from_rgb888(0xe0, 0xf8, 0xd0),
    Rgb555::from_rgb888(0x88, 0xc0, 0x70),
    Rgb555::from_rgb888(0x30, 0x68, 0x50),
    Rgb555::from_rgb888(0x08, 0x18, 0x20),
];

#[derive(Debug, Serialize, Deserialize)]
pub struct Gfx {
    vram: Box<[u8]>,
    oam_ram: Box<[u8]>,

    /// Represents the LCD itself, i.e. where pixels are actually written.
    ///
    /// Each pixel is a 15-bit colour, so the same buffer can hold CGB output later on.
    lcd: Box<[Rgb555]>,
    /// Colours the 4 DMG shades are rendered with.
    /// A frontend setting, so not part of save states.
    #[serde(skip)]
    dmg_palette: [Rgb555; 4],

    /// Number of clock cycles since we began rendering the current scanline
    line_dot: u16,
    running_mode: Mode,

    /// LCDC (LCD Control), made of the `LCDC_*` bits
    lcdc: u8,

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
    /// Level of the STAT interrupt line, whose rising edges request the interrupt.
    stat_line_high: bool,
    /// The LY=LYC comparison, as the STAT interrupt line sees it. It's only updated while the PPU
    /// runs, so it holds its value while the LCD is off.
    lyc_equal: bool,

    /// BG Palette: the shade of each colour index, 2 bits each (see `shade`)
    bgp: u8,
    /// OBJ Palette 0
    obp0: u8,
    /// OBJ Palette 1
    obp1: u8,

    // Window internal line counter
    window_internal_line_counter: u8,

    /// Gives the debugger access to VRAM and OAM whatever the PPU is doing. Not part of save
    /// states.
    #[serde(skip)]
    debugger_access: bool,
}

impl Gfx {
    pub fn new() -> Self {
        Self {
            vram: vec![0; 8 * 1024].into_boxed_slice(),
            oam_ram: vec![0; 0xA0].into_boxed_slice(),
            lcd: vec![Rgb555::default(); SCREEN_WIDTH * SCREEN_HEIGHT].into_boxed_slice(),
            dmg_palette: DEFAULT_DMG_PALETTE,
            line_dot: 0,
            // What STAT reports while the LCD is off
            running_mode: Mode::Mode0,
            lcdc: 0,
            scy: 0,
            scx: 0,
            bgp: 0,
            obp0: 0,
            obp1: 0,
            ly: 0,
            lyc: 0,
            wy: 0,
            wx: 0,
            stat_sources: 0,
            stat_line_high: false,
            lyc_equal: false,
            window_internal_line_counter: 0,
            debugger_access: false,
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
        let locked = self.running_mode == Mode::Mode3
            || (self.running_mode == Mode::Mode2 && self.line_dot >= OAM_SCAN_END_DOT);
        if !locked || self.cpu_access_unlocked() {
            self.read_vram_internal(addr)
        } else {
            0xff
        }
    }

    /// Whether the CPU can access VRAM and OAM however busy the PPU is.
    fn cpu_access_unlocked(&self) -> bool {
        self.debugger_access || !self.lcdc(LCDC_ENABLE)
    }

    /// Whether the given `LCDC_*` bit is set.
    fn lcdc(&self, bit: u8) -> bool {
        self.lcdc & bit != 0
    }

    /// Whether the PPU is in the middle of its OAM scan, where it locks OAM writes.
    fn scanning_oam(&self) -> bool {
        self.running_mode == Mode::Mode2
            && (LINE_START_DOTS..OAM_SCAN_END_DOT).contains(&self.line_dot)
    }

    /// Read access to the VRAM from within the PPU
    fn read_vram_internal(&self, addr: u16) -> u8 {
        self.vram[(addr - VRAM_START) as usize]
    }

    /// VRAM writes are only locked while the PPU draws a line (mode 3).
    pub fn write_vram(&mut self, addr: u16, b: u8) {
        if self.running_mode != Mode::Mode3 || self.cpu_access_unlocked() {
            self.vram[(addr - VRAM_START) as usize] = b;
        }
    }

    /// OAM reads are locked while the PPU uses OAM (modes 2 and 3).
    pub fn read_oam(&self, addr: u16) -> u8 {
        if !matches!(self.running_mode, Mode::Mode2 | Mode::Mode3) || self.cpu_access_unlocked() {
            self.oam_ram[(addr - OAM_START) as usize]
        } else {
            0xff
        }
    }

    /// OAM writes are locked during the OAM scan proper, and while drawing.
    pub fn write_oam(&mut self, addr: u16, b: u8) {
        let locked = self.scanning_oam() || self.running_mode == Mode::Mode3;
        if !locked || self.cpu_access_unlocked() {
            self.oam_ram[(addr - OAM_START) as usize] = b;
        }
    }

    /// Write a byte of OAM for the OAM DMA, which isn't locked out like the CPU.
    pub(crate) fn write_oam_dma(&mut self, index: u8, b: u8) {
        self.oam_ram[index as usize] = b;
    }

    pub fn read_reg(&self, addr: u16) -> u8 {
        match addr {
            LCDC_REG => self.lcdc,
            STAT_REG => self.stat(),
            SCY_REG => self.scy,
            SCX_REG => self.scx,
            LY_REG => self.ly,
            LYC_REG => self.lyc,
            WY_REG => self.wy,
            WX_REG => self.wx,
            BGP_REG => self.bgp,
            OBP0_REG => self.obp0,
            OBP1_REG => self.obp1,
            // CGB-only registers, so just ignore for now
            _ => 0xFF,
        }
    }

    pub fn write_reg(&mut self, addr: u16, b: u8) {
        match addr {
            LCDC_REG => {
                let was_enabled = self.lcdc(LCDC_ENABLE);
                self.lcdc = b;
                trace!("LCDC reg = 0b{:b}", b);
                if was_enabled != self.lcdc(LCDC_ENABLE) {
                    trace!("LCD turned {}", if was_enabled { "OFF" } else { "ON" });
                    // The PPU stops while the LCD is off, with LY at 0 and STAT in mode 0. Turning
                    // it back on starts a new frame, whose first line skips the OAM scan (it stays
                    // in mode 0 until drawing starts) and starts where the scan would, which makes
                    // it 4 dots shorter.
                    self.ly = 0;
                    self.line_dot = LINE_START_DOTS;
                    self.running_mode = Mode::Mode0;
                    self.window_internal_line_counter = 0;
                    // The LY=LYC comparison stops with the PPU, and restarts straight away with it.
                    if !was_enabled {
                        self.compare_lyc();
                    }
                }
            }
            STAT_REG => self.stat_sources = b & STAT_SOURCES,
            SCY_REG => self.scy = b,
            SCX_REG => self.scx = b,
            // LY is read-only
            LY_REG => (),
            LYC_REG => self.lyc = b,
            WY_REG => self.wy = b,
            WX_REG => self.wx = b,
            BGP_REG => self.bgp = b,
            OBP0_REG => self.obp0 = b,
            OBP1_REG => self.obp1 = b,
            // CGB-only registers, so just ignore for now
            _ => (),
        }
    }

    /// Return the value of the STAT register (FF41)
    fn stat(&self) -> u8 {
        // bit 7 is always 1
        let lyc_eq_ly = if self.lyc_equal && self.line_dot >= LINE_START_DOTS {
            0b100
        } else {
            0
        };
        0x80 | self.stat_sources | lyc_eq_ly | self.stat_mode() as u8
    }

    pub(crate) fn dots(&mut self, cycles: u8, frame_sink: &mut dyn FrameSink) -> InterruptFlag {
        let mut interrupt = InterruptFlag::empty();
        if !self.lcdc(LCDC_ENABLE) {
            return interrupt;
        }
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

    /// The next value of `line_dot` at which `dot` changes mode, updates the LY=LYC comparison or
    /// starts a new line.
    fn next_mode_change_dot(&self) -> u16 {
        if self.line_dot < LINE_START_DOTS {
            LINE_START_DOTS
        } else if self.ly >= SCREEN_HEIGHT as u8 || self.line_dot >= MODE0_START_DOT {
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
        let mut vblank_started = false;

        // The mode only ever changes at a handful of fixed dots, so only do work at those.
        self.line_dot += 1;
        if self.line_dot == DOTS_PER_LINE {
            self.line_dot = 0;
            self.ly = if self.ly == LINES_PER_FRAME - 1 {
                0
            } else {
                self.ly + 1
            };
            if self.ly < SCREEN_HEIGHT as u8 {
                // OAM scan
                self.running_mode = Mode::Mode2;
            }
        } else if self.ly == SCREEN_HEIGHT as u8 && self.line_dot == LINE_START_DOTS {
            // VBlank starts where the OAM scan would, like the other modes
            self.running_mode = Mode::Mode1;
            frame_sink.push_frame(&self.lcd);
            interrupts |= InterruptFlag::VBLANK;
            vblank_started = true;
            // Reset the window internal line counter
            self.window_internal_line_counter = 0;
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

        if self.line_dot >= LINE_START_DOTS {
            self.compare_lyc();
        }

        // A STAT interrupt is requested on a rising edge of the STAT interrupt line. On the DMG, the
        // mode 2 source also triggers one when VBlank starts, along with the VBlank source.
        let stat_line = self.stat_line();
        let oam_at_vblank = vblank_started && self.stat_sources & STAT_OAM != 0;
        if (stat_line || oam_at_vblank) && !self.stat_line_high {
            interrupts |= InterruptFlag::STAT;
        }
        self.stat_line_high = stat_line;

        interrupts
    }

    fn compare_lyc(&mut self) {
        self.lyc_equal = self.ly == self.lyc;
    }

    /// The mode as STAT reports it, which lags behind at the start of a line.
    fn stat_mode(&self) -> Mode {
        if self.running_mode == Mode::Mode2 && self.line_dot < LINE_START_DOTS {
            Mode::Mode0
        } else {
            self.running_mode
        }
    }

    /// The STAT interrupt conditions that currently hold, as `STAT_*` bits.
    fn stat_conditions(&self) -> u8 {
        let lyc_eq_ly = if self.lyc_equal { STAT_LYC } else { 0 };
        self.stat_mode().stat_condition() | lyc_eq_ly
    }

    // Only runs once per line: keep it out of `dots`, which runs every M-cycle and would otherwise
    // pay for this function's register and stack usage on every call.
    #[inline(never)]
    fn draw_scan_line(&mut self) {
        let mut drawn_from_window = false;
        let bg_tilemap_area = if self.lcdc(LCDC_BG_TILE_MAP) {
            0x9C00
        } else {
            0x9800
        };
        let win_tilemap_area = if self.lcdc(LCDC_WINDOW_TILE_MAP) {
            0x9C00
        } else {
            0x9800
        };
        let sprite_pixels = if self.lcdc(LCDC_OBJ_ENABLE) {
            self.sprite_pixels_for_scanline(self.ly)
        } else {
            [None; SCREEN_WIDTH]
        };

        // Render a line of pixels
        for x in 0..SCREEN_WIDTH as u8 {
            // Coordinates in "LCD space" (i.e 160x144)
            let (lcd_x, lcd_y) = (x, self.ly);
            // Coordinates in "Background area" space (i.e 256x256)
            let bg_and_window_enable = self.lcdc(LCDC_BG_WINDOW_ENABLE);
            let (bg_x, bg_y, tilemap_area) = if bg_and_window_enable
                && self.lcdc(LCDC_WINDOW_ENABLE)
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

            let color_byte = if bg_and_window_enable {
                self.bg_pixel(tilemap_area, bg_x, bg_y)
            } else {
                0
            };

            let final_color = match sprite_pixels[x as usize] {
                Some((p, bg_has_priority)) if !(bg_has_priority && color_byte != 0) => p,
                _ => shade(self.bgp, color_byte),
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

        let tile_offset: u16 = if self.lcdc(LCDC_TILE_DATA) {
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
        let (tile_index, tile_y) = if self.lcdc(LCDC_OBJ_SIZE) {
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
    fn sprite_pixels_for_scanline(&self, y: u8) -> [Option<(u8, bool)>; SCREEN_WIDTH] {
        let mut sprites = [Sprite::default(); 40];
        let mut count = 0;
        for data in self.oam_ram.as_chunks::<4>().0 {
            let sprite = Sprite::new(data);
            if sprite.matches_scanline(y, self.lcdc(LCDC_OBJ_SIZE)) {
                sprites[count] = sprite;
                count += 1;
            }
        }
        // Order the sprites by smallest `x` as they have higher priority, and only draw the first
        // 10.
        let sprites = &mut sprites[..count];
        sprites.sort_by_key(|s| s.x);

        let y_size = self.sprite_height();
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
                    pixels[lcd_x] = Some((shade(palette, color), sprite.bg_has_priority()));
                }
            }
        }
        pixels
    }

    fn sprite_height(&self) -> u8 {
        if self.lcdc(LCDC_OBJ_SIZE) { 16 } else { 8 }
    }

    fn sprite_palette(&self, sprite: &Sprite) -> u8 {
        if sprite.obp1_palette() {
            self.obp1
        } else {
            self.obp0
        }
    }

    /// The shade of a sprite's pixel, or `None` where it's transparent.
    fn sprite_shade(&self, sprite: &Sprite, tile_x: u8, tile_y: u8) -> Option<u8> {
        // Color index 0 is transparent for sprites
        match tile_pixel(self.sprite_tile_row(sprite, tile_y), tile_x) {
            0 => None,
            color => Some(shade(self.sprite_palette(sprite), color)),
        }
    }

    fn write_pixel(&mut self, x: u8, y: u8, shade: u8) {
        self.lcd[y as usize * SCREEN_WIDTH + x as usize] = self.dmg_palette[shade as usize];
    }

    /// Carry over what save states leave out from the `Gfx` this one replaces.
    pub(crate) fn restore_unsaved(&mut self, previous: &Self) {
        self.dmg_palette = previous.dmg_palette;
        self.debugger_access = previous.debugger_access;
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

        for y in 0..self.sprite_height() {
            for x in 0..8 {
                let shade = self.sprite_shade(&sprite, x, y).unwrap_or(0);
                print!("{}", self.shade_block(shade));
            }
            println!();
        }
    }

    pub fn dump_palettes(&self) {
        for (name, palette) in [
            ("BGP: ", self.bgp),
            ("OBP0:", self.obp0),
            ("OBP1:", self.obp1),
        ] {
            let mut s = String::new();
            for color in 0..4 {
                write!(s, "{}", self.shade_block(shade(palette, color))).unwrap();
            }
            println!("{name} {s}");
        }
    }

    /// A block of the given shade's colour, to print to the terminal.
    fn shade_block(&self, shade: u8) -> impl std::fmt::Display {
        let (r, g, b) = self.dmg_palette[shade as usize].to_rgb888();
        ansi_term::Color::RGB(r, g, b).paint("██")
    }

    /// Let the debugger access VRAM and OAM even while the PPU is using them.
    pub(crate) fn set_debugger_access(&mut self, enabled: bool) {
        self.debugger_access = enabled;
    }

    /// The various STAT interrupt sources (modes 0-2 and LYC=LY) have their state (inactive=low
    /// and active=high) logically ORed into a shared “STAT interrupt line” if their respective
    /// enable bit is turned on.
    #[inline(always)]
    fn stat_line(&self) -> bool {
        self.stat_sources & self.stat_conditions() != 0
    }
}

/// Colour index (0-3) of pixel `x` (0 being the leftmost) of a tile row, given as its two
/// bitplanes (low, high).
fn tile_pixel((lo, hi): (u8, u8), x: u8) -> u8 {
    let bit = 7 - x;
    (((hi >> bit) & 1) << 1) | ((lo >> bit) & 1)
}

/// The shade (0-3, lightest to darkest) a palette register maps a colour index to.
fn shade(palette: u8, color: u8) -> u8 {
    (palette >> (2 * color)) & 0b11
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        self.attrs & 0x10 != 0
    }

    pub fn bg_has_priority(&self) -> bool {
        self.attrs & 0x80 != 0
    }

    pub fn is_y_flip(&self) -> bool {
        self.attrs & 0x40 != 0
    }

    pub fn is_x_flip(&self) -> bool {
        self.attrs & 0x20 != 0
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shade() {
        let palette = 0b11_10_01_00;
        assert_eq!(
            [0, 1, 2, 3].map(|color| shade(palette, color)),
            [0, 1, 2, 3]
        );
        assert_eq!(shade(0b00_00_11_00, 1), 3);
    }
}
