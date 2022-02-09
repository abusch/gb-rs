use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::interconnect::Interconnect;

const BOOT_ROM: &[u8] = include_bytes!("../assets/dmg_boot.bin");

pub struct GameBoy {
    cpu: Cpu,
    itx: Interconnect,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge) -> Self {
        Self {
            cpu: Cpu::default(),
            itx: Interconnect::new(8 * 1024, cartridge),
        }
    }
}
