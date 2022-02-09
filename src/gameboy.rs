use crate::cpu::Cpu;
use crate::interconnect::Interconnect;

pub struct GameBoy {
    cpu: Cpu,
    itx: Interconnect,
}

impl GameBoy {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::default(),
            itx: Interconnect::new(8 * 1024),
        }
    }
}
