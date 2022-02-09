use crate::cartridge::Cartridge;

const BOOT_ROM: &[u8] = include_bytes!("../assets/dmg_boot.bin");

pub struct Interconnect {
    ram: Box<[u8]>,
    cartridge: Cartridge,
}

impl Interconnect {
    pub fn new(ram_size: usize, cartridge: Cartridge) -> Self {
        let ram = vec![0; ram_size];

        Self {
            ram: ram.into_boxed_slice(),
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
            unimplemented!("VRAM 0x{:04x}", addr);
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
            unimplemented!("I/O Registers: 0x{:04x}", addr);
        } else if addr <= 0xFFFE {
            unimplemented!("High RAM: 0x{:04x}", addr);
        } else if addr == 0xFFFF {
            unimplemented!("Interrupt Enable Register: 0x{:04x}", addr);
        } else {
            unreachable!("How did we get here?");
        }
    }

    pub fn read_word(&self, addr: u16) -> u16 {
        // FIXME is this little- or big-endian?
        let lsb = self.read_byte(addr);
        let msb = self.read_byte(addr + 1);

        ((msb as u16) << 8) | (lsb as u16)
    }

    pub fn write_byte(&mut self, addr: u16, b: u8) {
        self.ram[addr as usize] = b;
    }
}
