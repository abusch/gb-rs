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
            // INC BC
            0x03 => self.inc_rr(RegPair::BC),
            // LD B,d8
            0x06 => self.ld_r_d8(bus, Reg::B),
            // INC B
            0x04 => self.inc_r(Reg::B),
            // DEC B
            0x05 => self.dec_r(Reg::B),
            // DEC BC
            0x0b => self.dec_rr(RegPair::BC),
            // INC C
            0x0c => self.inc_r(Reg::C),
            // DEC C
            0x0d => self.dec_r(Reg::C),
            // LD C,d8
            0x0e => self.ld_r_d8(bus, Reg::C),
            // LD DE,d16
            0x11 => self.ld_rr_d16(bus, RegPair::DE),
            // INC DE
            0x13 => self.inc_rr(RegPair::DE),
            // INC D
            0x14 => self.inc_r(Reg::D),
            // DEC D
            0x15 => self.dec_r(Reg::D),
            // RLA
            0x17 => self.rla(),
            // LD A,(DE)
            0x1a => {
                let de = bus.read_byte(self.regs.de());
                self.regs.set_a(de);
            }
            // DEC DE
            0x1b => self.dec_rr(RegPair::DE),
            // INC E
            0x1c => self.inc_r(Reg::E),
            // DEC E
            0x1d => self.dec_r(Reg::E),
            // LD E,d8
            0x1e => self.ld_r_d8(bus, Reg::C),
            // JR NZ,r8
            0x20 => {
                let dd = self.fetch(bus) as i8;
                // NZ
                let flag = !self.regs.flag_z().is_set();
                self.jr_if(flag, dd);
            }
            // LD HL,d16
            0x21 => self.ld_rr_d16(bus, RegPair::HL),
            // LD (HL-),A
            0x22 => {
                bus.write_byte(self.regs.hl(), self.regs.a());
                self.regs.set_hl(self.regs.hl().wrapping_add(1));
            }
            // INC HL
            0x23 => self.inc_rr(RegPair::HL),
            // INC H
            0x24 => self.inc_r(Reg::H),
            // DEC H
            0x25 => self.dec_r(Reg::H),
            // JR Z,r8
            0x28 => {
                let dd = self.fetch(bus) as i8;
                // NZ
                let flag = self.regs.flag_z().is_set();
                self.jr_if(flag, dd);
            }
            // DEC HL
            0x2b => self.dec_rr(RegPair::HL),
            // INC L
            0x2c => self.inc_r(Reg::L),
            // DEC L
            0x2d => self.dec_r(Reg::L),
            // LD SP,d16
            0x31 => self.sp = self.fetch_word(bus),
            // LD (HL-),A
            0x32 => {
                bus.write_byte(self.regs.hl(), self.regs.a());
                self.regs.set_hl(self.regs.hl().wrapping_sub(1));
            }
            // // INC SP
            // 0x33 => self.inc_rr(RegPair::SP),
            // INC A
            0x3c => self.inc_r(Reg::A),
            // DEC A
            0x3d => self.dec_r(Reg::A),
            // LD A,d8
            0x3e => self.ld_r_d8(bus, Reg::A),
            // LD B,A..L
            0x40 => self.ld_r_r(Reg::B, Reg::B),
            0x41 => self.ld_r_r(Reg::B, Reg::C),
            0x42 => self.ld_r_r(Reg::B, Reg::D),
            0x43 => self.ld_r_r(Reg::B, Reg::E),
            0x44 => self.ld_r_r(Reg::B, Reg::H),
            0x45 => self.ld_r_r(Reg::B, Reg::L),
            0x47 => self.ld_r_r(Reg::B, Reg::A),
            // LD C,A..L
            0x48 => self.ld_r_r(Reg::C, Reg::B),
            0x49 => self.ld_r_r(Reg::C, Reg::C),
            0x4a => self.ld_r_r(Reg::C, Reg::D),
            0x4b => self.ld_r_r(Reg::C, Reg::E),
            0x4c => self.ld_r_r(Reg::C, Reg::H),
            0x4d => self.ld_r_r(Reg::C, Reg::L),
            0x4f => self.ld_r_r(Reg::C, Reg::A),
            // LD D,A..L
            0x50 => self.ld_r_r(Reg::D, Reg::B),
            0x51 => self.ld_r_r(Reg::D, Reg::C),
            0x52 => self.ld_r_r(Reg::D, Reg::D),
            0x53 => self.ld_r_r(Reg::D, Reg::E),
            0x54 => self.ld_r_r(Reg::D, Reg::H),
            0x55 => self.ld_r_r(Reg::D, Reg::L),
            0x57 => self.ld_r_r(Reg::D, Reg::A),
            // LD E,A..L
            0x58 => self.ld_r_r(Reg::E, Reg::B),
            0x59 => self.ld_r_r(Reg::E, Reg::C),
            0x5a => self.ld_r_r(Reg::E, Reg::D),
            0x5b => self.ld_r_r(Reg::E, Reg::E),
            0x5c => self.ld_r_r(Reg::E, Reg::H),
            0x5d => self.ld_r_r(Reg::E, Reg::L),
            0x5f => self.ld_r_r(Reg::E, Reg::A),
            // LD H,A..L
            0x60 => self.ld_r_r(Reg::H, Reg::B),
            0x61 => self.ld_r_r(Reg::H, Reg::C),
            0x62 => self.ld_r_r(Reg::H, Reg::D),
            0x63 => self.ld_r_r(Reg::H, Reg::E),
            0x64 => self.ld_r_r(Reg::H, Reg::H),
            0x65 => self.ld_r_r(Reg::H, Reg::L),
            0x67 => self.ld_r_r(Reg::H, Reg::A),
            // LD L,A..L
            0x68 => self.ld_r_r(Reg::L, Reg::B),
            0x69 => self.ld_r_r(Reg::L, Reg::C),
            0x6a => self.ld_r_r(Reg::L, Reg::D),
            0x6b => self.ld_r_r(Reg::L, Reg::E),
            0x6c => self.ld_r_r(Reg::L, Reg::H),
            0x6d => self.ld_r_r(Reg::L, Reg::L),
            0x6f => self.ld_r_r(Reg::L, Reg::A),
            // LD (HL),A
            0x77 => {
                let addr = self.regs.hl();
                bus.write_byte(addr, self.regs.a());
            }
            // LD A,A..L
            0x78 => self.ld_r_r(Reg::A, Reg::B),
            0x79 => self.ld_r_r(Reg::A, Reg::C),
            0x7a => self.ld_r_r(Reg::A, Reg::D),
            0x7b => self.ld_r_r(Reg::A, Reg::E),
            0x7c => self.ld_r_r(Reg::A, Reg::H),
            0x7d => self.ld_r_r(Reg::A, Reg::L),
            0x7f => self.ld_r_r(Reg::A, Reg::A),
            // SUB B
            0x90 => self.sub_r(Reg::B),
            // XOR A
            0xaf => self.xor_r(Reg::A),
            // CP (HL)
            0xbe => self.cp_hl(bus),
            // POP BC
            0xc1 => self.pop_rr(bus, RegPair::BC),
            // PUSH BC
            0xc5 => self.push_rr(bus, RegPair::BC),
            // RET
            0xc9 => {
                self.pc = bus.read_word(self.sp);
                self.sp = self.sp.wrapping_add(2);
                debug!("Returning from subroutine");
            }
            // CB prefix
            0xcb => self.step_cb(bus),
            // CALL a16
            0xcd => {
                let addr = self.fetch_word(bus);
                self.push_word(bus, self.pc);
                self.pc = addr;

                debug!("Calling subroutine at 0x{:04x}", addr);
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
            // LD (a16),A
            0xea => {
                let addr = self.fetch_word(bus);
                bus.write_byte(addr, self.regs.a());
            }
            // LDH A,(a8)
            0xf0 => {
                let a8 = self.fetch(bus);
                let addr = 0xFF00 + a8 as u16;
                self.regs.set(Reg::A, bus.read_byte(addr));
            }
            // CP d8
            0xfe => self.cp_d8(bus),

            _ => unimplemented!("op=0x{:02x}, PC=0x{:04x}", op, orig_pc),
        }
    }

    /// CB-prefixed instruction
    fn step_cb(&mut self, bus: &mut Bus) {
        let orig_pc = self.pc;
        let cb_op = self.fetch(bus);
        match cb_op {
            // RL C
            0x11 => self.rl_r(Reg::C),
            0x7c => self.bit_n_r(7, Reg::H),
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

    /// LD r,d8
    fn ld_r_d8(&mut self, bus: &mut Bus, reg: Reg) {
        let d8 = self.fetch(bus);
        self.regs.set(reg, d8);
    }

    /// LD r,r
    fn ld_r_r(&mut self, rt: Reg, rs: Reg) {
        self.regs.set(rt, self.regs.get(rs));
    }

    /// LD rr,d16
    fn ld_rr_d16(&mut self, bus: &mut Bus, reg: RegPair) {
        let d16 = self.fetch_word(bus);
        self.regs.set_pair(reg, d16);
    }

    /// A <- A ^ r
    fn xor_r(&mut self, reg: Reg) {
        let r = self.regs.get(reg);
        let new_a = self.regs.get(Reg::A) ^ r;
        self.regs.set(Reg::A, new_a);
        if new_a == 0 {
            self.regs.flag_z().set();
        }
    }

    /// DEC r
    fn dec_r(&mut self, reg: Reg) {
        let mut r = self.regs.get(reg);
        r = r.wrapping_sub(1);
        self.regs.set(reg, r);
        if r == 0 {
            self.regs.flag_z().set();
        } else {
            self.regs.flag_z().clear();
        }
        self.regs.flag_n().set();
        if r > 0x0F {
            self.regs.flag_h().set();
        } else {
            self.regs.flag_h().clear();
        }
    }

    /// INC r
    fn inc_r(&mut self, reg: Reg) {
        let mut r = self.regs.get(reg);
        r = r.wrapping_add(1);
        self.regs.set(reg, r);
        if r == 0 {
            self.regs.flag_z().set();
        } else {
            self.regs.flag_z().clear();
        }
        self.regs.flag_n().clear();
        if r > 0x0F {
            self.regs.flag_h().set();
        } else {
            self.regs.flag_h().clear();
        }
    }

    /// DEC rr
    fn dec_rr(&mut self, reg: RegPair) {
        self.regs.set_pair(reg, self.regs.get_pair(reg).wrapping_sub(1));
    }

    /// INC rr
    fn inc_rr(&mut self, reg: RegPair) {
        self.regs.set_pair(reg, self.regs.get_pair(reg).wrapping_add(1));
    }

    /// Test bit n of register r
    fn bit_n_r(&mut self, n: u8, reg: Reg) {
        let r = self.regs.get(reg);
        if r & (1 << n) == 0 {
            self.regs.flag_z().set();
        } else {
            self.regs.flag_z().clear();
        }
        self.regs.flag_n().clear();
        self.regs.flag_h().set();
    }

    /// Conditional relative jump
    fn jr_if(&mut self, flag: bool, dd: i8) {
        if flag {
            self.pc = self.pc.wrapping_add(dd as i16 as u16);
        }
    }

    /// PUSH rr
    fn push_rr(&mut self, bus: &mut Bus, rr: RegPair) {
        self.push_word(bus, self.regs.get_pair(rr));
    }

    /// POP rr
    fn pop_rr(&mut self, bus: &mut Bus, rr: RegPair) {
        let word = self.pop_word(bus);
        self.regs.set_pair(rr, word);
    }

    /// PUSH a16
    fn push_word(&mut self, bus: &mut Bus, word: u16) {
        self.sp = self.sp.wrapping_sub(2);
        bus.write_word(self.sp, word);
    }

    /// POP a16
    fn pop_word(&mut self, bus: &mut Bus) -> u16 {
        let word = bus.read_word(self.sp);
        self.sp = self.sp.wrapping_add(2);
        word
    }

    /// RL r ;rotate left through carry
    fn rl_r(&mut self, reg: Reg) {
        // C <- [7 <- 0] <- C
        let mut r = self.regs.get(reg);
        let c = self.regs.flag_c().is_set();
        r = r.rotate_left(1);
        // What used to be the 7th bit (and is now the 0th bit) should be the new carry flag
        let new_c = r & 0x1;
        // What was the carry should now be the 0th bit
        if c {
            // set the bit
            r |= 0x01;
        } else {
            // clear the bit
            r &= 0xFE;
        }
        self.regs.set(reg, r);
        if r == 0 {
            self.regs.flag_z().set();
        } else {
            self.regs.flag_z().clear();
        }
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        if new_c == 0 {
            self.regs.flag_c().clear();
        } else {
            self.regs.flag_c().set();
        }
    }

    /// RLA
    fn rla(&mut self) {
        // Same as RL A but Z is always cleared
        self.rl_r(Reg::A);
        self.regs.flag_z().clear();
    }

    fn sub_r(&mut self, reg: Reg) {
        let d8 = self.regs.get(reg);
        let (sub, carry) = self.regs.get(Reg::A).overflowing_sub(d8);
        self.regs.set(Reg::A, sub);
        self.regs.flag_z().set_value(sub == 0);
        self.regs.flag_n().set();
        self.regs.flag_c().set_value(carry);
        // TODO how to set H?
    }

    fn cp_hl(&mut self, bus: &mut Bus) {
        let d8 = bus.read_byte(self.regs.get_pair(RegPair::HL));
        self.cp(d8);
    }

    fn cp_d8(&mut self, bus: &mut Bus) {
        let d8 = self.fetch(bus);
        self.cp(d8);
    }

    fn cp(&mut self, value: u8) {
        let (sub, carry) = self.regs.get(Reg::A).overflowing_sub(value);
        self.regs.flag_z().set_value(sub == 0);
        self.regs.flag_n().set();
        self.regs.flag_c().set_value(carry);
        // TODO how to set H?
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

    fn get(&self, name: Reg) -> u8 {
        match name {
            Reg::A => self.a(),
            Reg::B => self.b(),
            Reg::C => self.c(),
            Reg::D => self.d(),
            Reg::E => self.e(),
            Reg::H => self.h(),
            Reg::L => self.l(),
        }
    }

    fn set(&mut self, name: Reg, value: u8) {
        match name {
            Reg::A => self.set_a(value),
            Reg::B => self.set_b(value),
            Reg::C => self.set_c(value),
            Reg::D => self.set_d(value),
            Reg::E => self.set_e(value),
            Reg::H => self.set_h(value),
            Reg::L => self.set_l(value),
        }
    }

    fn get_pair(&self, pair: RegPair) -> u16 {
        match pair {
            RegPair::AF => self.af,
            RegPair::BC => self.bc,
            RegPair::DE => self.de,
            RegPair::HL => self.hl,
        }
    }

    fn set_pair(&mut self, pair: RegPair, value: u16) {
        match pair {
            RegPair::AF => self.af = value,
            RegPair::BC => self.bc = value,
            RegPair::DE => self.de = value,
            RegPair::HL => self.hl = value,
        }
    }

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
    fn new(regs: &mut Registers, flag: u16) -> Flags {
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

    fn set_value(&mut self, value: bool) {
        if value {
            self.set();
        } else {
            self.clear();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reg {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegPair {
    AF,
    BC,
    DE,
    HL,
}
