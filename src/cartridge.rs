use std::path::Path;

use anyhow::{Context, Result};
use log::{debug, info};

pub struct Cartridge {
    data: Box<[u8]>,
    ram: Box<[u8]>,
    selected_rom_bank: u8,
    selected_ram_bank: u8,
}

impl Cartridge {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read(path).context("Failed to open rom file")?;
        info!("Loaded {} bytes from rom file", content.len());

        Ok(Self {
            data: content.into_boxed_slice(),
            // Allocate the most RAM a cart can have
            ram: vec![0; 64 * 1024].into_boxed_slice(),
            selected_rom_bank: 0x01,
            selected_ram_bank: 0x00,
        })
    }

    pub fn is_cgb(&self) -> bool {
        self.data[0x143] >> 7 != 0
    }

    pub fn title(&self) -> String {
        let bytes = &self.data[0x0134..=0x0143];
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());

        String::from_utf8_lossy(&bytes[..end]).to_string()
    }

    pub fn licensee_code(&self) -> String {
        String::from_utf8_lossy(&self.data[0x0144..=0x0145]).to_string()
    }

    pub fn cartridge_type(&self) -> &'static str {
        match self.data[0x0147] {
            0x00 => "ROM ONLY",
            0x01 => "MBC1",
            0x02 => "MBC1+RAM",
            0x03 => "MBC1+RAM+BATTERY",
            0x05 => "MBC2",
            0x06 => "MBC2+BATTERY",
            0x08 => "ROM+RAM 1",
            0x09 => "ROM+RAM+BATTERY 1",
            0x0B => "MMM01",
            0x0C => "MMM01+RAM",
            0x0D => "MMM01+RAM+BATTERY",
            0x0F => "MBC3+TIMER+BATTERY",
            0x10 => "MBC3+TIMER+RAM+BATTERY 2",
            0x11 => "MBC3",
            0x12 => "MBC3+RAM 2",
            0x13 => "MBC3+RAM+BATTERY 2",
            0x19 => "MBC5",
            0x1A => "MBC5+RAM",
            0x1B => "MBC5+RAM+BATTERY",
            0x1C => "MBC5+RUMBLE",
            0x1D => "MBC5+RUMBLE+RAM",
            0x1E => "MBC5+RUMBLE+RAM+BATTERY",
            0x20 => "MBC6",
            0x22 => "MBC7+SENSOR+RUMBLE+RAM+BATTERY",
            0xFC => "POCKET CAMERA",
            0xFD => "BANDAI TAMA5",
            0xFE => "HuC3",
            0xFF => "HuC1+RAM+BATTERY",
            b => panic!("Unknown cartridge type {:x}", b),
        }
    }

    pub fn has_mbc1(&self) -> bool {
        matches!(self.data[0x0147], 0x01..=0x03)
    }

    pub fn get_rom_size(&self) -> u8 {
        self.data[0x0148]
    }

    pub fn get_ram_size(&self) -> u8 {
        self.data[0x0149]
    }

    pub fn select_rom_bank(&mut self, bank: u8) {
        if bank == 0 {
            self.selected_rom_bank = 0x01;
        } else {
            self.selected_rom_bank = bank & 0x1f;
        }
        // assert!(bank <= self.get_num_rom_banks());
        debug!("Selected ROM bank {}", self.selected_rom_bank);
    }

    pub fn select_ram_bank(&mut self, bank: u8) {
        self.selected_ram_bank = bank & 0x03;
        debug!("Selected RAM bank {}", self.selected_ram_bank);
    }

    /// Read a byte from the selected bank of this cartridge's ROM.
    ///
    /// The given address should be relative to the selected bank, i.e. in the range 0000-3FFF.
    pub fn read_rom(&self, addr: u16) -> u8 {
        // assert!(addr < 0x4000);
        let mapped_addr = if addr < 0x4000 {
            addr
        } else {
            0x4000 * self.selected_rom_bank as u16 + (addr - 0x4000)
        };
        self.data[mapped_addr as usize]
    }

    /// Write a byte into the selected bank of this cartridge's ROM
    ///
    /// The given address should be relative to the selected bank, i.e. in the range 0000-3FFF.
    pub fn write_rom(&mut self, addr: u16, b: u8) {
        assert!(addr < 0x4000);
        let addr = 0x4000 * self.selected_rom_bank as u16 + addr;
        self.ram[addr as usize] = b;
    }

    /// Read a byte from the selected bank of this cartridge's external RAM.
    ///
    /// The given address should be relative to the selected bank, i.e. in the range 0000-1FFF.
    pub fn read_ram(&self, addr: u16) -> u8 {
        assert!(addr < 0x2000, "addr=0x{:04x}", addr);
        let addr = 0x2000 * self.selected_ram_bank as u16 + addr;
        self.ram[addr as usize]
    }

    /// Write a byte into the selected bank of this cartridge's external RAM
    ///
    /// The given address should be relative to the selected bank, i.e. in the range 0000-1FFF.
    pub fn write_ram(&mut self, addr: u16, b: u8) {
        assert!(addr < 0x2000);
        let addr = 0x2000 * self.selected_ram_bank as u16 + addr;
        self.ram[addr as usize] = b;
    }

    #[allow(dead_code)]
    fn get_num_rom_banks(&self) -> u16 {
        match self.get_rom_size() {
            0x00 => 2,
            0x01 => 4,
            0x02 => 8,
            0x03 => 16,
            0x04 => 32,
            0x05 => 64,
            0x06 => 128,
            0x07 => 256,
            0x08 => 512,
            s => panic!("Invalid ROM size {}", s),
        }
    }
}
