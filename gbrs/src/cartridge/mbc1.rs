use log::{debug, trace, warn};

/// MBC1 mapper. Also used as a fallback for the cartridge types that aren't supported yet.
pub(super) struct Mbc1 {
    selected_rom_bank: u8,
    secondary_bank_register: u8,
    banking_mode_1: bool,
    /// Whether the cart has 1MiB of ROM or more, so the secondary register selects ROM banks.
    large_rom: bool,
    /// Whether the cart has 32KiB of RAM or more, so the secondary register selects RAM banks.
    large_ram: bool,
    /// Whether writing 0 to the ROM bank register selects bank 1, as on a real MBC1. Other
    /// cartridge types falling back to this mapper (e.g. MBC5) can map bank 0 there.
    bank_0_selects_1: bool,
}

impl Mbc1 {
    pub(super) fn new(rom_size: u8, ram_size: u8, bank_0_selects_1: bool) -> Self {
        Self {
            selected_rom_bank: 0x01,
            secondary_bank_register: 0x00,
            banking_mode_1: false,
            large_rom: rom_size >= 0x05,
            large_ram: ram_size >= 0x03,
            bank_0_selects_1,
        }
    }

    pub(super) fn reset(&mut self) {
        self.selected_rom_bank = 0x01;
        self.secondary_bank_register = 0x00;
        self.banking_mode_1 = false;
    }

    /// Write to one of the mapper's registers, which sit over the ROM area (0000-7FFF).
    pub(super) fn write_register(&mut self, addr: u16, b: u8) {
        match addr {
            0x0000..=0x1FFF => {
                if b & 0x0A == 0x0A {
                    trace!("Enabling external RAM");
                } else {
                    trace!("Disabling external RAM");
                }
            }
            0x2000..=0x3FFF => self.select_rom_bank(b),
            0x4000..=0x5FFF => {
                self.secondary_bank_register = b & 0x03;
                trace!(
                    "Secondary bank register: {:02x}",
                    self.secondary_bank_register
                );
            }
            _ => self.select_banking_mode(b),
        }
    }

    fn select_rom_bank(&mut self, bank: u8) {
        if bank == 0 && self.bank_0_selects_1 {
            self.selected_rom_bank = 0x01;
        } else {
            self.selected_rom_bank = bank & 0x1f;
        }
        trace!("Selected ROM bank {}", self.selected_rom_bank);
    }

    fn select_banking_mode(&mut self, b: u8) {
        if b == 0 {
            self.banking_mode_1 = false;
            debug!("Banking mode select 0");
        } else if b == 1 {
            self.banking_mode_1 = true;
            debug!("Banking mode select 1");
        } else {
            warn!("Banking mode select set to unknown value: {:02x}", b);
        }
    }

    pub(super) fn read_rom(&self, rom: &[u8], addr: u16) -> u8 {
        let mapped_addr = if addr < 0x4000 {
            if self.banking_mode_1 && self.large_rom {
                (self.secondary_bank_register << 5) as u32 * 0x4000 + addr as u32
            } else {
                addr as u32
            }
        } else {
            let bank_num = if self.large_rom {
                (self.secondary_bank_register << 5) + self.selected_rom_bank
            } else {
                self.selected_rom_bank
            };
            0x4000 * (bank_num as u32) + (addr as u32 - 0x4000)
        };
        rom[mapped_addr as usize]
    }

    pub(super) fn read_ram(&self, ram: &[u8], addr: u16) -> u8 {
        let mapped_addr = if self.banking_mode_1 && self.large_ram {
            0x2000 * self.secondary_bank_register as u16 + addr
        } else {
            addr
        };
        ram[mapped_addr as usize]
    }

    pub(super) fn write_ram(&self, ram: &mut [u8], addr: u16, b: u8) {
        let addr = 0x2000 * self.secondary_bank_register as u16 + addr;
        ram[addr as usize] = b;
    }
}
