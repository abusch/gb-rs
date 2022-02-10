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
            // INC C
            0x0c => {
                // TODO extract this much better
                let mut c = self.regs.c();
                c += 1;
                self.regs.set_c(c);
                if c == 0 {
                    self.regs.flag_z().set();
                } else {
                    self.regs.flag_z().clear();
                }
                self.regs.flag_n().clear();
                if c > 0x0F {
                    self.regs.flag_h().set();
                } else {
                    self.regs.flag_h().clear();
                }
            }
            // LD C,d8
            0x0e => {
                let d = self.fetch(bus);
                self.regs.set_c(d);
            }
            // LD DE,d16
            0x11 => {
                let dd = self.fetch_word(bus);
                self.regs.set_de(dd);
            }
            // LD A,(DE)
            0x1a => {
                let de = bus.read_byte(self.regs.de());
                self.regs.set_a(de);
            }
            // JR NZ,r8
            0x20 => {
                let dd = self.fetch(bus) as i8;
                // NZ
                let flag = !self.regs.flag_z().is_set();
                self.jr_if(flag, dd);
            }
            // LD HL,d16
            0x21 => {
                let dd = self.fetch_word(bus);
                self.regs.set_hl(dd);
            }
            // LD SP,d16
            0x31 => self.sp = self.fetch_word(bus),
            // LD (HL-),A
            0x32 => {
                bus.write_byte(self.regs.hl(), self.regs.a());
                self.regs.set_hl(self.regs.hl() - 1);
            }
            // LD A,d8
            0x3e => {
                let d = self.fetch(bus);
                self.regs.set_a(d);
            }
            // LD (HL),A
            0x77 => {
                let addr = self.regs.hl();
                bus.write_byte(addr, self.regs.a());
            }
            // XOR A
            0xaf => self.xor(self.regs.a()),
            // CB prefix
            0xcb => self.step_cb(bus),
            // CALL a16
            0xcd => {
                unimplemented!("CALL a16");
            }
            // LDH (a8),A
            0xe0 => {
                let a8 = self.fetch(bus);
                let addr = 0xFF00 + a8 as u16;
                bus.write_byte(addr, self.regs.a());
            }
            // LD (C),A
            0xe2 => {
                let addr = 0xFF00 + self.regs.c() as u16;
                bus.write_byte(addr, self.regs.a());
            }
            _ => unimplemented!("op=0x{:02x}, PC=0x{:04x}", op, orig_pc),
        }
    }

    /// CB-prefixed instruction
    fn step_cb(&mut self, bus: &mut Bus) {
        let orig_pc = self.pc;
        let cb_op = self.fetch(bus);
        match cb_op {
            0x7c => self.bit_n_r(7, self.regs.h()),
            _ => unimplemented!("CB prefix op=0x{:02x}, PC=0x{:04x}", cb_op, orig_pc),
        }
    }

    // TODO probably should implement Debug instead...
    pub fn dump_cpu(&self) {
        debug!(
            "PC=0x{:04x}, SP=0x{:04x}, regs={:?}",
            self.pc, self.sp, self.regs
        );
    }

    fn fetch(&mut self, bus: &mut Bus) -> u8 {
        let byte = bus.read_byte(self.pc);
        self.pc += 1;
        byte
    }

    fn fetch_word(&mut self, bus: &mut Bus) -> u16 {
        let lsb = self.fetch(bus);
        let msb = self.fetch(bus);

        (msb as u16) << 8 | (lsb as u16)
    }

    /// A <- A ^ r
    pub fn xor(&mut self, r: u8) {
        self.regs.set_a(self.regs.a() ^ r);
        if self.regs.a() == 0 {
            self.regs.flag_z().set();
        }
    }

    /// Test bit n of register r
    fn bit_n_r(&mut self, n: u8, r: u8) {
        if r & (1 << n) == 0 {
            self.regs.flag_z().set();
        } else {
            self.regs.flag_z().clear();
        }
        self.regs.flag_n().clear();
        self.regs.flag_h().set();
    }

    /// Conditional relative jump
    pub fn jr_if(&mut self, flag: bool, dd: i8) {
        if flag {
            self.pc = self.pc.wrapping_add(dd as i16 as u16);
        }
    }
}

// TODO don't think this is a great design... maybe we need a `Register` struct for a single
// register.
#[derive(Default, Debug)]
struct Registers {
    af: u16,
    bc: u16,
    de: u16,
    hl: u16,
}

impl Registers {
    /// Zero flag
    const FLAG_Z: u16 = 0x80;
    /// Subtract flag
    const FLAG_N: u16 = 0x80;
    /// Half Carry flag
    const FLAG_H: u16 = 0x80;
    /// Carry flag
    const FLAG_C: u16 = 0x80;

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
    
    fn set_de(&mut self, de: u16) {
        self.de = de;
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

    fn set_hl(&mut self, hl: u16) {
        self.hl = hl;
    }

    // Flags
    fn flag_z(&mut self) -> Flags<'_> {
        Flags::new(self, Self::FLAG_Z)
    }

    // N Flag
    fn flag_n(&mut self) -> Flags<'_> {
        Flags::new(self, Self::FLAG_N)
    }

    // H Flag
    fn flag_h(&mut self) -> Flags<'_> {
        Flags::new(self, Self::FLAG_H)
    }

    // C Flag
    fn flag_c(&mut self) -> Flags<'_> {
        Flags::new(self, Self::FLAG_C)
    }
}

struct Flags<'regs> {
    regs: &'regs mut Registers,
    flag: u16,
}

impl<'regs> Flags<'regs> {
    fn new<'a>(regs: &'a mut Registers, flag: u16) -> Flags<'a> {
        Flags { regs, flag }
    }

    fn is_set(&self) -> bool {
        self.regs.af & self.flag != 0
    }

    fn set(&mut self) {
        self.regs.af |= self.flag;
    }

    fn clear(&mut self) {
        self.regs.af &= !self.flag;
    }
}
