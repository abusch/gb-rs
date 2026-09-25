use log::trace;

/// MBC1 mapper: up to 2MiB of ROM and 32KiB of RAM. Also used as a fallback for the cartridge
/// types that aren't supported yet.
///
/// See <https://gbdev.io/pandocs/MBC1.html>.
pub(super) struct Mbc1 {
    /// Number of 16KiB ROM banks, rounded up to a power of 2 so it can be used as a mask.
    rom_banks: usize,
    /// Number of 8KiB RAM banks.
    ram_banks: usize,
    /// Whether RAM can be accessed.
    ram_enabled: bool,
    /// BANK1: the low 5 bits of the ROM bank mapped at 4000-7FFF.
    bank1: u8,
    /// BANK2: 2 more bits, used as bits 5-6 of the ROM bank number, and in mode 1 as the RAM bank
    /// and to map a higher bank at 0000-3FFF too.
    bank2: u8,
    mode_1: bool,
    /// Where BANK2 goes in the ROM bank number, and which bits of BANK1 are used. An MBC1M
    /// multicart wires BANK2 as bits 4-5 instead of 5-6, so each game gets 16 banks and bit 4 of
    /// BANK1 goes unused.
    bank2_shift: u8,
    bank1_mask: u8,
    /// Whether writing 0 to BANK1 selects bank 1, as on a real MBC1. It doesn't for ROM-only
    /// carts and the types falling back to this mapper, to keep their behaviour unchanged.
    bank_0_selects_1: bool,
}

impl Mbc1 {
    pub(super) fn new(
        rom_len: usize,
        ram_banks: usize,
        bank_0_selects_1: bool,
        multicart: bool,
    ) -> Self {
        Self {
            rom_banks: rom_len.div_ceil(0x4000).next_power_of_two(),
            ram_banks,
            ram_enabled: false,
            bank1: 1,
            bank2: 0,
            mode_1: false,
            bank2_shift: if multicart { 4 } else { 5 },
            bank1_mask: if multicart { 0x0F } else { 0x1F },
            bank_0_selects_1,
        }
    }

    pub(super) fn reset(&mut self) {
        self.ram_enabled = false;
        self.bank1 = 1;
        self.bank2 = 0;
        self.mode_1 = false;
    }

    /// Write to one of the mapper's registers, which sit over the ROM area (0000-7FFF).
    pub(super) fn write_register(&mut self, addr: u16, b: u8) {
        match addr {
            0x0000..=0x1FFF => {
                self.ram_enabled = b & 0x0F == 0x0A;
                trace!("External RAM enabled: {}", self.ram_enabled);
            }
            0x2000..=0x3FFF => {
                // Only the 5 bits that exist are checked for 0, so e.g. 0x20 selects bank 1 too.
                self.bank1 = b & 0x1F;
                if self.bank1 == 0 && self.bank_0_selects_1 {
                    self.bank1 = 1;
                }
                trace!("BANK1: {:02x}", self.bank1);
            }
            0x4000..=0x5FFF => {
                self.bank2 = b & 0x03;
                trace!("BANK2: {:02x}", self.bank2);
            }
            _ => {
                self.mode_1 = b & 0x01 != 0;
                trace!("Banking mode 1: {}", self.mode_1);
            }
        }
    }

    pub(super) fn read_rom(&self, rom: &[u8], addr: u16) -> u8 {
        let bank2 = self.bank2 << self.bank2_shift;
        let (bank, offset) = if addr < 0x4000 {
            (if self.mode_1 { bank2 } else { 0 }, addr)
        } else {
            (bank2 | (self.bank1 & self.bank1_mask), addr - 0x4000)
        };
        // Bank numbers wrap around to the size of the ROM, since the higher bits aren't wired.
        let offset = (bank as usize & (self.rom_banks - 1)) * 0x4000 + offset as usize;
        // ROM dumps whose size isn't a power of 2 leave some banks unbacked
        rom.get(offset).copied().unwrap_or(0xFF)
    }

    pub(super) fn read_ram(&self, ram: &[u8], addr: u16) -> u8 {
        match self.ram_offset(addr) {
            Some(offset) => ram[offset],
            None => 0xFF,
        }
    }

    pub(super) fn write_ram(&self, ram: &mut [u8], addr: u16, b: u8) {
        if let Some(offset) = self.ram_offset(addr) {
            ram[offset] = b;
        }
    }

    /// Where an access to A000-BFFF lands in RAM, if anywhere.
    fn ram_offset(&self, addr: u16) -> Option<usize> {
        if !self.ram_enabled || self.ram_banks == 0 {
            return None;
        }
        let bank = if self.mode_1 { self.bank2 as usize } else { 0 };
        Some((bank & (self.ram_banks - 1)) * 0x2000 + addr as usize)
    }
}
