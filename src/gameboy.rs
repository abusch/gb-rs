use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::FrameSink;

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

    pub fn step(&mut self, frame_sink: &mut dyn FrameSink) {
        let cycles = self.cpu.step(&mut self.bus);
        self.bus.cycle(cycles, frame_sink);
    }

    pub fn dump_cpu(&self) {
        self.cpu.dump_cpu();
    }

    pub fn dump_mem(&self, addr: u16) {
        for offset in 0..4 {
            let addr = addr + offset * 16;
            print!("{:04x}: ", addr);
            for a in addr..addr + 16 {
                print!("{:02x} ", self.bus.read_byte(a));
            }
            println!();
        }
    }

    pub fn is_halted(&self) -> bool {
        // TODO implement
        false
    }

    pub fn is_paused(&self) -> bool {
        self.cpu.is_paused()
    }

    pub fn pause(&mut self) {
        self.cpu.set_pause(true);
    }

    pub fn resume(&mut self) {
        self.cpu.set_pause(false);
    }
}
