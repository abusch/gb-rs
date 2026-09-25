use log::trace;

/// MBC5 mapper: up to 8MiB of ROM and 128KiB of RAM, optionally with a rumble motor.
///
/// See <https://gbdev.io/pandocs/MBC5.html>.
pub(super) struct Mbc5 {
    /// Number of 16KiB ROM banks, rounded up to a power of 2 so it can be used as a mask.
    rom_banks: usize,
    /// Number of 8KiB RAM banks.
    ram_banks: usize,
    /// Whether the cart has a rumble motor, which takes over bit 3 of the RAM bank register.
    has_rumble: bool,
    /// Whether RAM can be accessed.
    ram_enabled: bool,
    /// 9-bit ROM bank mapped at 4000-7FFF. Unlike on MBC1 and MBC3, this can be bank 0.
    rom_bank: u16,
    /// RAM bank mapped at A000-BFFF.
    ram_bank: u8,
}

impl Mbc5 {
    pub(super) fn new(rom_len: usize, ram_banks: usize, has_rumble: bool) -> Self {
        Self {
            rom_banks: rom_len.div_ceil(0x4000).next_power_of_two(),
            ram_banks,
            has_rumble,
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
        }
    }

    pub(super) fn reset(&mut self) {
        self.ram_enabled = false;
        self.rom_bank = 1;
        self.ram_bank = 0;
    }

    /// Write to one of the mapper's registers, which sit over the ROM area (0000-7FFF).
    pub(super) fn write_register(&mut self, addr: u16, b: u8) {
        match addr {
            0x0000..=0x1FFF => {
                self.ram_enabled = b & 0x0F == 0x0A;
                trace!("External RAM enabled: {}", self.ram_enabled);
            }
            0x2000..=0x2FFF => {
                self.rom_bank = (self.rom_bank & 0x100) | u16::from(b);
                trace!("Selected ROM bank {}", self.rom_bank);
            }
            0x3000..=0x3FFF => {
                self.rom_bank = (self.rom_bank & 0xFF) | (u16::from(b & 0x01) << 8);
                trace!("Selected ROM bank {}", self.rom_bank);
            }
            0x4000..=0x5FFF => {
                self.ram_bank = if self.has_rumble { b & 0x07 } else { b & 0x0F };
                trace!("Selected RAM bank {}", self.ram_bank);
            }
            _ => (),
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
        Some((self.ram_bank as usize & (self.ram_banks - 1)) * 0x2000 + addr as usize)
    }
}
