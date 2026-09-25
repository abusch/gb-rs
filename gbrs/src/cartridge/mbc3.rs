use log::trace;

use super::rtc::Rtc;

/// MBC3 mapper: up to 2MiB of ROM and 32KiB of RAM, optionally with a real-time clock.
///
/// See <https://gbdev.io/pandocs/MBC3.html>.
pub(super) struct Mbc3 {
    /// Number of 16KiB ROM banks, rounded up to a power of 2 so it can be used as a mask.
    rom_banks: usize,
    /// Number of 8KiB RAM banks.
    ram_banks: usize,
    /// Whether RAM and the RTC registers can be accessed.
    ram_enabled: bool,
    /// ROM bank mapped at 4000-7FFF.
    rom_bank: u8,
    /// What's mapped at A000-BFFF: a RAM bank for 0x00-0x07, an RTC register for 0x08-0x0C.
    ram_bank: u8,
    /// Last value written to the latch register: the RTC gets latched when going from 0 to 1.
    latch: u8,
    pub(super) rtc: Option<Rtc>,
}

impl Mbc3 {
    pub(super) fn new(rom_len: usize, ram_banks: usize, has_rtc: bool) -> Self {
        Self {
            rom_banks: rom_len.div_ceil(0x4000).next_power_of_two(),
            ram_banks,
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
            latch: 0xFF,
            rtc: has_rtc.then(Rtc::default),
        }
    }

    pub(super) fn reset(&mut self) {
        self.ram_enabled = false;
        self.rom_bank = 1;
        self.ram_bank = 0;
        self.latch = 0xFF;
    }

    /// Write to one of the mapper's registers, which sit over the ROM area (0000-7FFF).
    pub(super) fn write_register(&mut self, addr: u16, b: u8) {
        match addr {
            0x0000..=0x1FFF => {
                self.ram_enabled = b & 0x0F == 0x0A;
                trace!("External RAM enabled: {}", self.ram_enabled);
            }
            0x2000..=0x3FFF => {
                // Bank 0 is always mapped at 0000-3FFF, so selecting it maps bank 1 instead.
                self.rom_bank = (b & 0x7F).max(1);
                trace!("Selected ROM bank {}", self.rom_bank);
            }
            0x4000..=0x5FFF => {
                self.ram_bank = b;
                trace!("Selected RAM bank / RTC register {:02X}", b);
            }
            _ => {
                if self.latch == 0x00
                    && b == 0x01
                    && let Some(rtc) = &mut self.rtc
                {
                    rtc.latch();
                }
                self.latch = b;
            }
        }
    }

    pub(super) fn read_rom(&self, rom: &[u8], addr: u16) -> u8 {
        let offset = if addr < 0x4000 {
            addr as usize
        } else {
            (self.rom_bank as usize & (self.rom_banks - 1)) * 0x4000 + (addr - 0x4000) as usize
        };
        // ROM dumps whose size isn't a power of 2 leave some banks unbacked
        rom.get(offset).copied().unwrap_or(0xFF)
    }

    pub(super) fn read_ram(&self, ram: &[u8], addr: u16) -> u8 {
        if !self.ram_enabled {
            return 0xFF;
        }
        match self.ram_bank {
            0x00..=0x07 if self.ram_banks > 0 => ram[self.ram_offset(addr)],
            0x08..=0x0C => self
                .rtc
                .as_ref()
                .map_or(0xFF, |rtc| rtc.read(self.ram_bank - 0x08)),
            _ => 0xFF,
        }
    }

    pub(super) fn write_ram(&mut self, ram: &mut [u8], addr: u16, b: u8) {
        if !self.ram_enabled {
            return;
        }
        match self.ram_bank {
            0x00..=0x07 if self.ram_banks > 0 => ram[self.ram_offset(addr)] = b,
            0x08..=0x0C => {
                if let Some(rtc) = &mut self.rtc {
                    rtc.write(self.ram_bank - 0x08, b);
                }
            }
            _ => (),
        }
    }

    fn ram_offset(&self, addr: u16) -> usize {
        (self.ram_bank as usize & (self.ram_banks - 1)) * 0x2000 + addr as usize
    }
}
