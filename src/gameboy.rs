use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::disasm::Disassembler;
use crate::joypad::Button;
use crate::{AudioSink, FrameSink};

pub struct GameBoy {
    cpu: Cpu,
    bus: Bus,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge, breakpoint: Option<u16>, enable_soft_break: bool) -> Self {
        Self {
            cpu: Cpu::with_breakpoint(breakpoint, enable_soft_break),
            bus: Bus::new(8 * 1024, cartridge),
        }
    }

    pub fn step(&mut self, frame_sink: &mut dyn FrameSink, audio_sink: &mut dyn AudioSink) -> u64 {
        let cycles = self.cpu.step(&mut self.bus);
        for _ in 0..cycles {
            self.bus.cycle(1, frame_sink, audio_sink);
            self.cpu.handle_interrupt(&mut self.bus);
        }

        cycles as u64
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

    pub fn disassemble(&self, addr: u16) {
        let bytes = (addr..addr + 100)
            .map(|a| self.bus.read_byte(a))
            .collect::<Vec<_>>();
        let instrs = Disassembler::new(&bytes).run();
        let mut pc = addr;
        for inst in instrs {
            println!("{pc:04X}\t{inst}");
            pc += inst.bytes;
        }
    }

    pub fn dump_oam(&self) {
        self.bus.gfx.dump_oam();
    }

    pub fn dump_sprite(&self, id: u8) {
        self.bus.gfx.dump_sprite(id);
    }

    pub fn dump_palettes(&self) {
        self.bus.gfx.dump_palettes();
    }

    pub fn is_halted(&self) -> bool {
        self.cpu.halted()
    }

    pub fn is_paused(&self) -> bool {
        self.cpu.is_paused()
    }

    pub fn pause(&mut self) {
        self.cpu.set_pause(true);
        self.bus.gfx.disable();
    }

    pub fn resume(&mut self) {
        self.bus.gfx.enable();
        self.cpu.set_pause(false);
    }

    pub fn set_breakpoint(&mut self, addr: u16) {
        self.cpu.set_breakpoint(addr);
    }

    pub fn set_button_pressed(&mut self, button: Button, is_pressed: bool) {
        self.bus.set_button_pressed(button, is_pressed);
    }

    pub fn save(&self) {
        self.bus.cartridge.save();
    }
}
