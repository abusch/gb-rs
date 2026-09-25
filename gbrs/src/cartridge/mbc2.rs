use log::trace;

/// Size of the MBC2's built-in RAM, in half-bytes (stored one per byte).
pub(super) const MBC2_RAM_SIZE: usize = 512;

/// MBC2 mapper: up to 256KiB of ROM, and 512 half-bytes of RAM built into the chip.
///
/// See <https://gbdev.io/pandocs/MBC2.html>.
pub(super) struct Mbc2 {
    /// Number of 16KiB ROM banks, rounded up to a power of 2 so it can be used as a mask.
    rom_banks: usize,
    /// Whether RAM can be accessed.
    ram_enabled: bool,
    /// 4-bit ROM bank mapped at 4000-7FFF.
    rom_bank: u8,
}

impl Mbc2 {
    pub(super) fn new(rom_len: usize) -> Self {
        Self {
            rom_banks: rom_len.div_ceil(0x4000).next_power_of_two(),
            ram_enabled: false,
            rom_bank: 1,
        }
    }

    pub(super) fn reset(&mut self) {
        self.ram_enabled = false;
        self.rom_bank = 1;
    }

    /// Write to one of the mapper's registers, which sit over the ROM area (0000-7FFF).
    pub(super) fn write_register(&mut self, addr: u16, b: u8) {
        // Both registers are in 0000-3FFF, told apart by bit 8 of the address.
        match addr {
            0x0000..=0x3FFF if addr & 0x0100 == 0 => {
                self.ram_enabled = b & 0x0F == 0x0A;
                trace!("External RAM enabled: {}", self.ram_enabled);
            }
            0x0000..=0x3FFF => {
                self.rom_bank = (b & 0x0F).max(1);
                trace!("Selected ROM bank {}", self.rom_bank);
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
        if !self.ram_enabled {
            return 0xFF;
        }
        // Only the low 4 bits exist: the others are open bus, which reads as 1s.
        0xF0 | ram[Self::ram_offset(addr)]
    }

    pub(super) fn write_ram(&self, ram: &mut [u8], addr: u16, b: u8) {
        if self.ram_enabled {
            ram[Self::ram_offset(addr)] = b & 0x0F;
        }
    }

    /// The RAM only decodes 9 address bits, so it's mirrored all over A000-BFFF.
    fn ram_offset(addr: u16) -> usize {
        addr as usize % MBC2_RAM_SIZE
    }
}
