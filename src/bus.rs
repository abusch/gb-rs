use log::debug;

use crate::cartridge::Cartridge;

const BOOT_ROM: &[u8] = include_bytes!("../assets/dmg_boot.bin");

pub struct Bus {
    ram: Box<[u8]>,
    vram: Box<[u8]>,
    hram: Box<[u8]>,
    cartridge: Cartridge,
}

impl Bus {
    pub fn new(ram_size: usize, cartridge: Cartridge) -> Self {
        let ram = vec![0; ram_size];

        Self {
            ram: ram.into_boxed_slice(),
            vram: vec![0; 8 * 1024].into_boxed_slice(),
            hram: vec![0; 0x80].into_boxed_slice(),
            cartridge,
        }
    }

    pub fn read_byte(&self, addr: u16) -> u8 {
        if addr <= 0x0100 {
            // read from boot rom
            // TODO only during boot sequence... once control is handed over to the cartridge, we
            // should read from the cartridge memory instead
            BOOT_ROM[addr as usize]
        } else if addr <= 0x3FFF {
            self.cartridge.data[addr as usize]
        } else if addr <= 0x7FFF {
            unimplemented!("switchable banks 0x{:04x}", addr);
        } else if addr <= 0x9FFF {
            self.vram[(addr - 0x8000) as usize]
        } else if addr <= 0xDFFF {
            self.ram[(addr - 0xC000) as usize]
        } else if addr <= 0xFDFF {
            // ECHO RAM: mirror of C000-DDFF
            self.ram[(addr - 0xE000) as usize]
        } else if addr <= 0xFE9F {
            unimplemented!("Sprite attribute table (OAM): 0x{:04x}", addr);
        } else if addr <= 0xFEFF {
            panic!("Invalid access to address 0x{:04x}", addr);
        } else if addr <= 0xFF7F {
            // unimplemented!("I/O Registers: 0x{:04x}", addr);
            debug!("Accessed I/O Register 0x{:04x} (NOT IMPLEMENTED)", addr);
            0
        } else if addr <= 0xFFFE {
            self.hram[(addr - 0xFF80) as usize]
        } else if addr == 0xFFFF {
            unimplemented!("Interrupt Enable Register: 0x{:04x}", addr);
        } else {
            unreachable!("How did we get here?");
        }
    }

    pub fn read_word(&self, addr: u16) -> u16 {
        // memory access is big-endian
        let lsb = self.read_byte(addr);
        let msb = self.read_byte(addr + 1);

        ((msb as u16) << 8) | (lsb as u16)
    }

    pub fn write_byte(&mut self, addr: u16, b: u8) {
        if addr <= 0x3FFF {
            self.cartridge.data[addr as usize] = b;
        } else if addr <= 0x7FFF {
            unimplemented!("switchable banks 0x{:04x}", addr);
        } else if addr <= 0x9FFF {
            self.vram[(addr - 0x8000) as usize] = b;
        } else if addr <= 0xDFFF {
            self.ram[(addr - 0xC000) as usize] = b;
        } else if addr <= 0xFDFF {
            // ECHO RAM: mirror of C000-DDFF
            self.ram[(addr - 0xE000) as usize] = b;
        } else if addr <= 0xFE9F {
            unimplemented!("Sprite attribute table (OAM): 0x{:04x}", addr);
        } else if addr <= 0xFEFF {
            panic!("Invalid access to address 0x{:04x}", addr);
        } else if addr <= 0xFF7F {
            // unimplemented!("I/O Registers: 0x{:04x}", addr);
            debug!("Accessed I/O Register 0x{:04x} (NOT IMPLEMENTED)", addr);
        } else if addr <= 0xFFFE {
            self.hram[(addr - 0xFF80) as usize] = b;
        } else if addr == 0xFFFF {
            unimplemented!("Interrupt Enable Register: 0x{:04x}", addr);
        } else {
            unreachable!("How did we get here?");
        }
    }

    pub(crate) fn write_word(&mut self, addr: u16, word: u16) {
        // memory access is big-endian, so write the lsb first...
        self.write_byte(addr, (word & 0x00FF) as u8);
        // then the msb
        self.write_byte(addr + 1, (word >> 8) as u8);
    }
}
