use log::debug;

use crate::bus::Bus;

#[derive(Default)]
pub struct Cpu {
    regs: Registers,

    sp: u16,
    pc: u16,
}

impl Cpu {
    pub fn step(&mut self, bus: &mut Bus) {
        let orig_pc = self.pc;
        let op = self.fetch(bus);

        match op {
            0x31 => self.sp = self.fetch_word(bus),
            _ => unimplemented!("op=0x{:02x}, PC=0x{:04x}", op, orig_pc),
        }
    }

    // TODO probably should implement Debug instead...
    pub fn dump_cpu(&self) {
        debug!("PC=0x{:04x}, SP=0x{:04x}, regs=TODO", self.pc, self.sp);
    }

    fn fetch(&mut self, bus: &mut Bus) -> u8 {
        let byte = bus.read_byte(self.pc);
        self.pc += 1;
        byte
    }

    fn fetch_word(&mut self, bus: &mut Bus) -> u16 {
        let byte1 = self.fetch(bus);
        let byte2 = self.fetch(bus);

        (byte1 as u16) << 8 | (byte2 as u16)
    }

}

#[derive(Default)]
struct Registers {
    af: u16,
    bc: u16,
    de: u16,
    hl: u16,
}

impl Registers {
    //
    // AF
    //
    fn a(&self) -> u8 {
        (self.af >> 8) as u8
    }

    fn f(&self) -> u8 {
        (self.af & 0x00ff) as u8
    }

    fn af(&self) -> u16 {
        self.af
    }

    fn set_a(&mut self, a: u8) {
        self.af = (self.af & 0x00ff) | ((a as u16) << 8);
    }

    fn set_f(&mut self, f: u8) {
        self.af = (self.af & 0x00ff) | ((f as u16) << 8);
    }
    //
    // BC
    //
    fn b(&self) -> u8 {
        (self.bc >> 8) as u8
    }

    fn c(&self) -> u8 {
        (self.bc & 0x00ff) as u8
    }

    fn bc(&self) -> u16 {
        self.bc
    }

    fn set_b(&mut self, b: u8) {
        self.bc = (self.bc & 0x00ff) | ((b as u16) << 8);
    }

    fn set_c(&mut self, c: u8) {
        self.bc = (self.bc & 0x00ff) | ((c as u16) << 8);
    }
    //
    // DE
    //
    fn d(&self) -> u8 {
        (self.de >> 8) as u8
    }

    fn e(&self) -> u8 {
        (self.de & 0x00ff) as u8
    }

    fn de(&self) -> u16 {
        self.de
    }

    fn set_d(&mut self, d: u8) {
        self.de = (self.de & 0x00ff) | ((d as u16) << 8);
    }

    fn set_e(&mut self, e: u8) {
        self.de = (self.de & 0x00ff) | ((e as u16) << 8);
    }
    //
    // HL
    //
    fn h(&self) -> u8 {
        (self.hl >> 8) as u8
    }

    fn l(&self) -> u8 {
        (self.hl & 0x00ff) as u8
    }

    fn hl(&self) -> u16 {
        self.hl
    }

    fn set_h(&mut self, h: u8) {
        self.hl = (self.hl & 0x00ff) | ((h as u16) << 8);
    }

    fn set_l(&mut self, l: u8) {
        self.hl = (self.hl & 0x00ff) | ((l as u16) << 8);
    }
}
