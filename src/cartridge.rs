use std::path::Path;

use anyhow::{Context, Result};
use log::info;

pub struct Cartridge {
    pub(crate) data: Box<[u8]>,
}

impl Cartridge {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read(path).context("Failed to open rom file")?;
        info!("Loaded {} bytes from rom file", content.len());

        Ok(Self {
            data: content.into_boxed_slice(),
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

    pub fn get_rom_size(&self) -> u8 {
        self.data[0x0148]
    }

    pub fn get_ram_size(&self) -> u8 {
        self.data[0x0149]
    }
}
