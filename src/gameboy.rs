use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::bus::Bus;

pub struct GameBoy {
    cpu: Cpu,
    bus: Bus,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge) -> Self {
        Self {
            cpu: Cpu::default(),
            bus: Bus::new(8 * 1024, cartridge),
        }
    }

    pub fn step(&mut self) {
        let cycles = self.cpu.step(&mut self.bus);
        self.bus.cycle(cycles);
    }

    pub fn dump_cpu(&self) {
        self.cpu.dump_cpu();
    }

    pub fn is_halted(&self) -> bool {
        // TODO implement
        false
    }

    pub fn is_paused(&self) -> bool {
        self.cpu.paused()
    }
}
