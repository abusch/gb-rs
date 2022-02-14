use std::ops::RangeInclusive;

// use bitvec::prelude as bv;
use log::{debug, warn};

use crate::{cartridge::Cartridge, gfx::Gfx, FrameSink};

const BOOT_ROM_DATA: &[u8] = include_bytes!("../assets/dmg_boot.bin");

// Memory Map
const BOOT_ROM: RangeInclusive<u16> = 0x0000..=0x00FF;
const CART_BANK_00: RangeInclusive<u16> = 0x0000..=0x3FFF;
const CART_BANK_MAPPED: RangeInclusive<u16> = 0x4000..=0x7FFF;
const VRAM: RangeInclusive<u16> = 0x8000..=0x9FFF;
const EXT_RAM: RangeInclusive<u16> = 0xA000..=0xBFFF;
const WRAM: RangeInclusive<u16> = 0xC000..=0xDFFF;
const ECHO_RAM: RangeInclusive<u16> = 0xE000..=0xFDFF;
const OAM: RangeInclusive<u16> = 0xFE00..=0xFE9F;
const INVALID_AREA: RangeInclusive<u16> = 0xFEA0..=0xFEFF;
const IO_REGISTERS: RangeInclusive<u16> = 0xFF00..=0xFF7F;
const HRAM: RangeInclusive<u16> = 0xFF80..=0xFFFE;

//
// IO registers ranges (TODO CGB registers)
//
// Joypad controller
const IO_RANGE_JPD: RangeInclusive<u16> = 0xFF00..=0xFF00;
// Communication
const IO_RANGE_COM: RangeInclusive<u16> = 0xFF01..=0xFF02;
// Divider and Timer
const IO_RANGE_TIM: RangeInclusive<u16> = 0xFF04..=0xFF07;
// Sound (APU)
const IO_RANGE_APU: RangeInclusive<u16> = 0xFF10..=0xFF26;
// Waveform RAM
const IO_RANGE_WAV: RangeInclusive<u16> = 0xFF30..=0xFF3F;
// LCD
const IO_RANGE_LCD: RangeInclusive<u16> = 0xFF40..=0xFF4B;
// Disable Boot ROM
const IO_RANGE_DBR: RangeInclusive<u16> = 0xFF50..=0xFF50;

const CYCLES_PER_SECOND: u64 = 4194304; // 4.194304 MHz

pub struct Bus {
    ram: Box<[u8]>,
    hram: Box<[u8]>,
    gfx: Gfx,
    cartridge: Cartridge,
    // P1/JOYP Joypad contoller
    joypad: u8,
    has_booted: bool,
}

impl Bus {
    pub fn new(ram_size: usize, cartridge: Cartridge) -> Self {
        let ram = vec![0; ram_size];

        Self {
            ram: ram.into_boxed_slice(),
            hram: vec![0; 0x80].into_boxed_slice(),
            gfx: Gfx::new(),
            cartridge,
            joypad: 0,
            has_booted: false,
        }
    }

    /// Run the different peripherals for the given number of clock cycles
    pub fn cycle(&mut self, cycles: u8, frame_sync: &mut dyn FrameSink) {
        self.gfx.dots(cycles, frame_sync);
    }

    pub fn read_byte(&self, addr: u16) -> u8 {
        if BOOT_ROM.contains(&addr) && !self.has_booted {
            // read from boot rom
            BOOT_ROM_DATA[addr as usize]
        } else if CART_BANK_00.contains(&addr) {
            self.cartridge.data[addr as usize]
        } else if CART_BANK_MAPPED.contains(&addr) {
            // unimplemented!("switchable banks 0x{:04x}", addr);
            warn!("unimplemented switchable banks 0x{:04x}", addr);
            0xFF
        } else if VRAM.contains(&addr) {
            self.gfx.read_vram(addr)
        } else if EXT_RAM.contains(&addr) {
            // unimplemented!("External RAM 0x{:04x}", addr);
            warn!("External RAM 0x{:04x}", addr);
            0xFF
        } else if WRAM.contains(&addr) {
            self.ram[(addr - WRAM.start()) as usize]
        } else if ECHO_RAM.contains(&addr) {
            // ECHO RAM: mirror of C000-DDFF
            warn!("Accessing ECHO RAM!");
            self.ram[(addr - 0x2000) as usize]
        } else if OAM.contains(&addr) {
            debug!("Reading Sprite attribute table (OAM): 0x{:04x}", addr);
            self.gfx.read_oam(addr)
        } else if INVALID_AREA.contains(&addr) {
            panic!("Invalid access to address 0x{:04x}", addr);
        } else if IO_REGISTERS.contains(&addr) {
            self.read_io(addr)
        } else if HRAM.contains(&addr) {
            self.hram[(addr - HRAM.start()) as usize]
        } else if addr == 0xFFFF {
            // unimplemented!("Interrupt Enable Register: 0x{:04x}", addr);
            warn!("Interrupt Enable Register: 0x{:04x}", addr);
            0xFF
        } else {
            unreachable!("How did we get here?");
        }
    }

    pub fn read_word(&self, addr: u16) -> u16 {
        // memory access is little-endian (i.e lsb comes first)
        let lsb = self.read_byte(addr);
        let msb = self.read_byte(addr + 1);

        u16::from_le_bytes([lsb, msb])
    }

    pub fn write_byte(&mut self, addr: u16, b: u8) {
        if BOOT_ROM.contains(&addr) && !self.has_booted {
            panic!("Tried to write into boot ROM during the boot sequence!");
        } else if CART_BANK_00.contains(&addr) {
            self.cartridge.data[addr as usize] = b;
        } else if CART_BANK_MAPPED.contains(&addr) {
            // unimplemented!("switchable banks 0x{:04x}", addr);
            warn!("unimplemented switchable banks 0x{:04x}", addr);
        } else if VRAM.contains(&addr) {
            self.gfx.write_vram(addr, b);
        } else if WRAM.contains(&addr) {
            self.ram[(addr - WRAM.start()) as usize] = b;
        } else if ECHO_RAM.contains(&addr) {
            // ECHO RAM: mirror of C000-DDFF
            // FIXME should we panic here instead?
            self.ram[(addr - 0x2000) as usize] = b;
        } else if OAM.contains(&addr) {
            debug!("Writing Sprite attribute table (OAM): 0x{:04x}", addr);
            self.gfx.write_oam(addr, b);
        } else if INVALID_AREA.contains(&addr) {
            panic!("Invalid access to address 0x{:04x}", addr);
        } else if IO_REGISTERS.contains(&addr) {
            self.write_io(addr, b);
        } else if HRAM.contains(&addr) {
            self.hram[(addr - HRAM.start()) as usize] = b;
        } else if addr == 0xFFFF {
            // unimplemented!("Interrupt Enable Register: 0x{:04x}", addr);
            warn!("Unimplemented Interrupt Enable Register: 0x{:04x}", addr);
        } else {
            unreachable!("How did we get here?");
        }
    }

    pub(crate) fn write_word(&mut self, addr: u16, word: u16) {
        // memory access is little-endian, so write the lsb first...
        let [lsb, msb] = word.to_le_bytes();
        self.write_byte(addr, lsb);
        // then the msb
        self.write_byte(addr + 1, msb);
    }

    /// Read access to IO registers
    fn read_io(&self, addr: u16) -> u8 {
        if IO_RANGE_JPD.contains(&addr) {
            // Joypad controller register
            debug!(
                "Read Joypad controller register 0x{:04x} (NOT IMPLEMENTED)",
                addr
            );
            self.joypad
        } else if IO_RANGE_COM.contains(&addr) {
            // Communication controller
            debug!(
                "Read communication controller register 0x{:04x} (NOT IMPLEMENTED)",
                addr
            );
            0
        } else if IO_RANGE_TIM.contains(&addr) {
            // Divider and timer
            debug!(
                "Read divider and timer register 0x{:04x} (NOT IMPLEMENTED)",
                addr
            );
            0
        } else if IO_RANGE_APU.contains(&addr) {
            // Sound
            debug!("Read sound register 0x{:04x} (NOT IMPLEMENTED)", addr);
            0
        } else if IO_RANGE_WAV.contains(&addr) {
            // Waveform ram
            debug!("Read waveform RAM 0x{:04x} (NOT IMPLEMENTED)", addr);
            0
        } else if IO_RANGE_LCD.contains(&addr) {
            // LCD
            // debug!("Read LCD controller 0x{:04x}", addr);
            self.gfx.read_reg(addr)
        } else if IO_RANGE_DBR.contains(&addr) {
            // Disable boot rom
            debug!("Read disable boot rom 0x{:04x} (NOT IMPLEMENTED)", addr);
            0
        } else {
            debug!("Read unknown I/O Register 0x{:04x} (NOT IMPLEMENTED)", addr);
            0
        }
    }

    /// Write access to IO registers.
    fn write_io(&mut self, addr: u16, b: u8) {
        if IO_RANGE_JPD.contains(&addr) {
            // Joypad controller register
            debug!(
                "Write Joypad controller register 0x{:04x}<-0x{:02X}",
                addr, b
            );
            // only bits 4 and 5 can be written to
            self.joypad |= b & 0b00110000;
        } else if IO_RANGE_COM.contains(&addr) {
            // Communication controller
            debug!(
                "Write communication controller register 0x{:04x}<-0x{:02X} (NOT IMPLEMENTED)",
                addr, b
            );
        } else if IO_RANGE_TIM.contains(&addr) {
            // Divider and timer
            debug!(
                "Write divider and timer register 0x{:04x}<-0x{:02X} (NOT IMPLEMENTED)",
                addr, b
            );
        } else if IO_RANGE_APU.contains(&addr) {
            // Sound
            debug!(
                "Write sound register 0x{:04x}<-0x{:02X} (NOT IMPLEMENTED)",
                addr, b
            );
        } else if IO_RANGE_WAV.contains(&addr) {
            // Waveform ram
            debug!(
                "Write waveform RAM 0x{:04x}<-0x{:02X} (NOT IMPLEMENTED)",
                addr, b
            );
        } else if IO_RANGE_LCD.contains(&addr) {
            // LCD
            debug!(
                "Write LCD controller 0x{:04x}<-0x{:02X}",
                addr, b
            );
            self.gfx.write_reg(addr, b);
        } else if IO_RANGE_DBR.contains(&addr) {
            if b == 0x01 {
                self.has_booted = true;
                // Disable boot rom
                debug!("Boot sequence complete. Disabling boot ROM.");
            }
        } else {
            // unimplemented!("I/O Registers: 0x{:04x}", addr);
            debug!(
                "Write I/O Register 0x{:04x}<-0x{:02X} (NOT IMPLEMENTED)",
                addr, b
            );
        }
    }
}
