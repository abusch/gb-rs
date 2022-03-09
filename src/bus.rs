use std::ops::RangeInclusive;

use log::{debug, info, trace};

use crate::{
    cartridge::Cartridge, gfx::Gfx, interrupt::InterruptFlag, joypad::Joypad, timer::Timer,
    FrameSink,
};

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
/// Joypad controller
const IO_RANGE_JPD: RangeInclusive<u16> = 0xFF00..=0xFF00;
/// Communication
const IO_RANGE_COM: RangeInclusive<u16> = 0xFF01..=0xFF02;
/// Divider and Timer
const IO_RANGE_TIM: RangeInclusive<u16> = 0xFF04..=0xFF07;
/// IF - Interrupt Flag
const IO_RANGE_INT: RangeInclusive<u16> = 0xFF0F..=0xFF0F;
/// Sound (APU)
const IO_RANGE_APU: RangeInclusive<u16> = 0xFF10..=0xFF26;
/// Waveform RAM
const IO_RANGE_WAV: RangeInclusive<u16> = 0xFF30..=0xFF3F;
/// LCD
const IO_RANGE_LCD: RangeInclusive<u16> = 0xFF40..=0xFF4F;
/// Disable Boot ROM
const IO_RANGE_DBR: RangeInclusive<u16> = 0xFF50..=0xFF50;

pub struct Bus {
    ram: Box<[u8]>,
    hram: Box<[u8]>,
    pub(crate) gfx: Gfx,
    pub(crate) cartridge: Cartridge,
    /// P1/JOYP Joypad contoller
    joypad: Joypad,
    input_has_changed: bool,

    has_booted: bool,

    /// IE - Interrupt Enable register
    interrupt_enable: InterruptFlag,
    /// IF - Interrupt Flag register
    interrupt_flag: InterruptFlag,

    timer: Timer,
}

impl Bus {
    pub fn new(ram_size: usize, cartridge: Cartridge) -> Self {
        let ram = vec![0; ram_size];

        Self {
            ram: ram.into_boxed_slice(),
            hram: vec![0; 0x80].into_boxed_slice(),
            gfx: Gfx::new(),
            cartridge,
            joypad: Joypad::default(),
            input_has_changed: false,
            has_booted: false,
            interrupt_enable: InterruptFlag::empty(),
            interrupt_flag: InterruptFlag::empty(),
            timer: Timer::new(),
        }
    }

    /// Run the different peripherals for the given number of clock cycles
    pub fn cycle(&mut self, cycles: u8, frame_sync: &mut dyn FrameSink) {
        self.interrupt_flag |= self.gfx.dots(cycles, frame_sync);
        if self.timer.cycle(cycles) {
            self.interrupt_flag |= InterruptFlag::TIMER;
        }
        if self.input_has_changed {
            self.interrupt_flag |= InterruptFlag::JOYPAD;
            self.input_has_changed = false;
        }
    }

    pub fn read_byte(&self, addr: u16) -> u8 {
        if BOOT_ROM.contains(&addr) && !self.has_booted {
            // read from boot rom
            BOOT_ROM_DATA[addr as usize]
        } else if CART_BANK_00.contains(&addr) || CART_BANK_MAPPED.contains(&addr) {
            self.cartridge.read_rom(addr)
        } else if VRAM.contains(&addr) {
            self.gfx.read_vram(addr)
        } else if EXT_RAM.contains(&addr) {
            trace!("External RAM 0x{:04x}", addr);
            self.cartridge.read_ram(addr - EXT_RAM.start())
        } else if WRAM.contains(&addr) {
            self.ram[(addr - WRAM.start()) as usize]
        } else if ECHO_RAM.contains(&addr) {
            // ECHO RAM: mirror of C000-DDFF
            trace!("Accessing ECHO RAM!");
            self.read_byte(addr - 0x2000)
        } else if OAM.contains(&addr) {
            // debug!("Reading Sprite attribute table (OAM): 0x{:04x}", addr);
            self.gfx.read_oam(addr)
        } else if INVALID_AREA.contains(&addr) {
            trace!("Invalid access to address 0x{:04x}", addr);
            0x00
        } else if IO_REGISTERS.contains(&addr) {
            self.read_io(addr)
        } else if HRAM.contains(&addr) {
            self.hram[(addr - HRAM.start()) as usize]
        } else if addr == 0xFFFF {
            trace!("Reading IE register: {:?}", self.interrupt_enable);
            self.interrupt_enable.bits()
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
            if (0x0000..=0x1FFF).contains(&addr) {
                if b & 0x0A == 0x0A {
                    trace!("Enabling external RAM");
                } else {
                    trace!("Disabling external RAM");
                }
            } else if (0x2000..0x3FFF).contains(&addr) {
                self.cartridge.select_rom_bank(b);
            // } else {
            //     // ROM Bank Number register
            //     // self.cartridge.select_rom_bank(b);
            //     debug!("Not implemented: select 9th bit of ROM bank number");
            }
        } else if CART_BANK_MAPPED.contains(&addr) {
            if (0x4000..=0x5FFF).contains(&addr) {
                trace!("Selecting external RAM bank {:02X}", b);
                self.cartridge.set_secondary_bank_register(b);
            } else {
                // Select banking mode
                self.cartridge.select_banking_mode(b);
            }
        } else if VRAM.contains(&addr) {
            self.gfx.write_vram(addr, b);
        } else if EXT_RAM.contains(&addr) {
            self.cartridge.write_ram(addr - EXT_RAM.start(), b);
        } else if WRAM.contains(&addr) {
            self.ram[(addr - WRAM.start()) as usize] = b;
        } else if ECHO_RAM.contains(&addr) {
            // ECHO RAM: mirror of C000-DDFF
            self.write_byte(addr - 0x2000, b);
        } else if OAM.contains(&addr) {
            // debug!("Writing Sprite attribute table (OAM): 0x{:04x}", addr);
            self.gfx.write_oam(addr, b);
        } else if INVALID_AREA.contains(&addr) {
            // Ignore writes to this area as some games reset it to 0 for some reason
            // warn!("Invalid access to address 0x{:04x}", addr);
        } else if IO_REGISTERS.contains(&addr) {
            self.write_io(addr, b);
        } else if HRAM.contains(&addr) {
            self.hram[(addr - HRAM.start()) as usize] = b;
        } else if addr == 0xFFFF {
            trace!("Setting Interrupt Enable Register with 0b{:08b}", b);
            self.interrupt_enable = InterruptFlag::from_bits_truncate(b);
        } else {
            unreachable!("How did we get here? addr=0x{:04x}", addr);
        }
    }

    pub(crate) fn write_word(&mut self, addr: u16, word: u16) {
        // memory access is little-endian, so write the lsb first...
        let [lsb, msb] = word.to_le_bytes();
        self.write_byte(addr, lsb);
        // then the msb
        self.write_byte(addr + 1, msb);
    }

    pub fn interrupt_enable(&self) -> InterruptFlag {
        self.interrupt_enable
    }

    pub fn interrupt_flag(&self) -> InterruptFlag {
        self.interrupt_flag
    }

    pub fn ack_interrupt(&mut self, flag: InterruptFlag) {
        self.interrupt_flag.toggle(flag);
        trace!(
            "Acknowledging interrupt: {:?}. Pending: {:?}",
            flag,
            self.interrupt_flag
        );
    }

    /// Read access to IO registers
    fn read_io(&self, addr: u16) -> u8 {
        if IO_RANGE_JPD.contains(&addr) {
            // Joypad controller register
            trace!("Read Joypad controller register 0x{:04x}", addr);
            self.joypad.read()
        } else if IO_RANGE_COM.contains(&addr) {
            // Communication controller
            // debug!(
            //     "Read communication controller register 0x{:04x} (NOT IMPLEMENTED)",
            //     addr
            // );
            0
        } else if IO_RANGE_TIM.contains(&addr) {
            match addr {
                0xff04 => self.timer.div_timer(),
                0xff05 => self.timer.tima(),
                0xff06 => self.timer.tma(),
                0xff07 => self.timer.tac(),
                _ => {
                    // Divider and timer
                    debug!(
                        "Read divider and timer register 0x{:04x} (NOT IMPLEMENTED)",
                        addr
                    );
                    0
                }
            }
        } else if IO_RANGE_INT.contains(&addr) {
            // IF - interrupt flag
            self.interrupt_flag.bits()
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
            self.joypad.write(b);
            trace!(
                "Write Joypad controller register 0x{:04x}<-0x{:02X}. Register is now {:08b}",
                addr,
                b,
                self.joypad.read()
            );
        } else if IO_RANGE_COM.contains(&addr) {
            // Communication controller
            // debug!(
            //     "Write communication controller register 0x{:04x}<-0x{:02X} (NOT IMPLEMENTED)",
            //     addr, b
            // );
        } else if IO_RANGE_TIM.contains(&addr) {
            match addr {
                0xff04 => self.timer.reset_div_timer(),
                0xff05 => self.timer.set_tima(b),
                0xff06 => self.timer.set_tma(b),
                0xff07 => self.timer.set_tac(b),
                _ => unreachable!(),
            }
        } else if IO_RANGE_INT.contains(&addr) {
            // IF - interrupt flag
            trace!("Setting IF with {:08b}", b);
            self.interrupt_flag = InterruptFlag::from_bits_truncate(b);
        } else if IO_RANGE_APU.contains(&addr) {
            // Sound
            trace!(
                "Write sound register 0x{:04x}<-0x{:02X} (NOT IMPLEMENTED)",
                addr,
                b
            );
        } else if IO_RANGE_WAV.contains(&addr) {
            // Waveform ram
            trace!(
                "Write waveform RAM 0x{:04x}<-0x{:02X} (NOT IMPLEMENTED)",
                addr,
                b
            );
        } else if IO_RANGE_LCD.contains(&addr) {
            // LCD
            // debug!("Write LCD controller 0x{:04x}<-0x{:02X}", addr, b);
            if addr == 0xff46 {
                // DMA transfer
                let base_addr = (b as u16) * 0x100;
                // debug!("Starting DMA transfer from 0x{:04x} to OAM", base_addr);
                for i in 0..=0x9Fu16 {
                    self.gfx
                        .write_oam(OAM.start() + i, self.read_byte(base_addr + i));
                }
            } else {
                self.gfx.write_reg(addr, b);
            }
        } else if IO_RANGE_DBR.contains(&addr) {
            if b == 0x01 {
                self.has_booted = true;
                // Disable boot rom
                info!("Boot sequence complete. Disabling boot ROM.");
            }
        } else if (0xff68..=0xff69).contains(&addr) {
            // CGB-only registers, just ignore for now
        } else {
            trace!(
                "Write I/O Register 0x{:04x}<-0x{:02X} (NOT IMPLEMENTED)",
                addr, b
            );
        }
    }

    pub(crate) fn set_button_pressed(&mut self, button: crate::joypad::Button, is_pressed: bool) {
        self.input_has_changed = self.joypad.set_button(button, is_pressed);
    }
}
