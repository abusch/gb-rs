mod register;

use log::debug;

use crate::bus::Bus;
use self::register::{Registers, Reg, RegPair};

pub struct Cpu {
    regs: Registers,

    sp: u16,
    pc: u16,

    // for debugging
    breakpoint: u16,
    paused: bool,
}

impl Default for Cpu {
    fn default() -> Self {
        Self {
            regs: Default::default(),
            sp: Default::default(),
            pc: Default::default(),
            breakpoint: 0x0070,
            paused: Default::default(),
        }
    }
}

impl Cpu {
    /// Fetch and execute the next instructions.
    ///
    /// Return the number of clock cycles used.
    pub fn step(&mut self, bus: &mut Bus) -> u8 {
        // for debugging
        if self.pc == self.breakpoint {
            self.paused = true;
        }

        let orig_pc = self.pc;
        let op = self.fetch(bus);
        // Number of clock cycles used by the instruction (T-states)

        match op {
            // INC BC
            0x03 => self.inc_rr(RegPair::BC),
            // INC B
            0x04 => self.inc_r(Reg::B),
            // DEC B
            0x05 => self.dec_r(Reg::B),
            // LD B,d8
            0x06 => self.ld_r_d8(bus, Reg::B),
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
            // LD D,d8
            0x16 => self.ld_r_d8(bus, Reg::D),
            // RLA
            0x17 => self.rla(),
            // JR r8
            0x18 => self.jr_if_r8(bus, true),
            // LD A,(DE)
            0x1a => {
                let de = bus.read_byte(*self.regs.de);
                self.regs.set(Reg::A, de);
                8
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
                // NZ
                let flag = !self.regs.flag_z().is_set();
                self.jr_if_r8(bus, flag)
            }
            // LD HL,d16
            0x21 => self.ld_rr_d16(bus, RegPair::HL),
            // LD (HL-),A
            0x22 => {
                bus.write_byte(*self.regs.hl, self.regs.get(Reg::A));
                *self.regs.hl = self.regs.hl.wrapping_add(1);
                8
            }
            // INC HL
            0x23 => self.inc_rr(RegPair::HL),
            // INC H
            0x24 => self.inc_r(Reg::H),
            // DEC H
            0x25 => self.dec_r(Reg::H),
            // JR Z,r8
            0x28 => {
                // NZ
                let flag = self.regs.flag_z().is_set();
                self.jr_if_r8(bus, flag)
            }
            // DEC HL
            0x2b => self.dec_rr(RegPair::HL),
            // INC L
            0x2c => self.inc_r(Reg::L),
            // DEC L
            0x2d => self.dec_r(Reg::L),
            // LD L,d8
            0x2e => self.ld_r_d8(bus, Reg::L),
            // LD SP,d16
            0x31 => {
                self.sp = self.fetch_word(bus);
                12
            }
            // LD (HL-),A
            0x32 => {
                bus.write_byte(self.regs.get_pair(RegPair::HL), self.regs.get(Reg::A));
                *self.regs.hl = self.regs.hl.wrapping_sub(1);
                8
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
                let addr = *self.regs.hl;
                bus.write_byte(addr, self.regs.get(Reg::A));
                8
            }
            // LD A,A..L
            0x78 => self.ld_r_r(Reg::A, Reg::B),
            0x79 => self.ld_r_r(Reg::A, Reg::C),
            0x7a => self.ld_r_r(Reg::A, Reg::D),
            0x7b => self.ld_r_r(Reg::A, Reg::E),
            0x7c => self.ld_r_r(Reg::A, Reg::H),
            0x7d => self.ld_r_r(Reg::A, Reg::L),
            0x7f => self.ld_r_r(Reg::A, Reg::A),
            // ADD A,(HL)
            0x86 => self.add_hl(bus),
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
                self.pc = self.pop_word(bus);
                debug!("Returning from subroutine to 0x{:04x}", self.pc);
                16
            }
            // CB prefix
            0xcb => self.step_cb(bus),
            // CALL a16
            0xcd => {
                let addr = self.fetch_word(bus);
                self.push_word(bus, self.pc);
                self.pc = addr;

                debug!("Calling subroutine at 0x{:04x}", addr);
                24
            }
            // LDH (a8),A
            0xe0 => {
                let a8 = self.fetch(bus);
                let addr = 0xFF00 + a8 as u16;
                bus.write_byte(addr, self.regs.get(Reg::A));
                12
            }
            // LD (C),A
            0xe2 => {
                let addr = 0xFF00 + self.regs.get(Reg::C) as u16;
                bus.write_byte(addr, self.regs.get(Reg::A));
                8
            }
            // LD (a16),A
            0xea => {
                let addr = self.fetch_word(bus);
                bus.write_byte(addr, self.regs.get(Reg::A));
                16
            }
            // LDH A,(a8)
            0xf0 => {
                let a8 = self.fetch(bus);
                let addr = 0xFF00 + a8 as u16;
                self.regs.set(Reg::A, bus.read_byte(addr));
                12
            }
            // CP d8
            0xfe => self.cp_d8(bus),

            _ => {
                self.dump_cpu();
                unimplemented!("op=0x{:02x}, orig_pc=0x{:04x}", op, orig_pc);
            }
        }
    }

    /// CB-prefixed instruction
    fn step_cb(&mut self, bus: &mut Bus) -> u8 {
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
            "PC=0x{:04x}, SP=0x{:04x},\n\tregs={:?}",
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
    fn ld_r_d8(&mut self, bus: &mut Bus, reg: Reg) -> u8 {
        let d8 = self.fetch(bus);
        self.regs.set(reg, d8);
        8
    }

    /// LD r,r
    fn ld_r_r(&mut self, rt: Reg, rs: Reg) -> u8 {
        self.regs.set(rt, self.regs.get(rs));
        4
    }

    /// LD rr,d16
    fn ld_rr_d16(&mut self, bus: &mut Bus, reg: RegPair) -> u8 {
        let d16 = self.fetch_word(bus);
        self.regs.set_pair(reg, d16);
        12
    }

    /// A <- A ^ r
    fn xor_r(&mut self, reg: Reg) -> u8 {
        let r = self.regs.get(reg);
        let new_a = self.regs.get(Reg::A) ^ r;
        self.regs.set(Reg::A, new_a);
        if new_a == 0 {
            self.regs.flag_z().set();
        }
        4
    }

    /// DEC r
    fn dec_r(&mut self, reg: Reg) -> u8 {
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
        4
    }

    /// INC r
    fn inc_r(&mut self, reg: Reg) -> u8 {
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
        4
    }

    /// DEC rr
    fn dec_rr(&mut self, reg: RegPair) -> u8 {
        self.regs
            .set_pair(reg, self.regs.get_pair(reg).wrapping_sub(1));
        8
    }

    /// INC rr
    fn inc_rr(&mut self, reg: RegPair) -> u8 {
        self.regs
            .set_pair(reg, self.regs.get_pair(reg).wrapping_add(1));
        8
    }

    /// Test bit n of register r
    fn bit_n_r(&mut self, n: u8, reg: Reg) -> u8 {
        let r = self.regs.get(reg);
        if r & (1 << n) == 0 {
            self.regs.flag_z().set();
        } else {
            self.regs.flag_z().clear();
        }
        self.regs.flag_n().clear();
        self.regs.flag_h().set();
        8
    }

    /// Conditional relative jump
    fn jr_if_r8(&mut self, bus: &mut Bus, flag: bool) -> u8 {
        let r8 = self.fetch(bus) as i8;
        if flag {
            self.pc = self.pc.wrapping_add(r8 as i16 as u16);
            12
        } else {
            8
        }
    }

    /// PUSH rr
    fn push_rr(&mut self, bus: &mut Bus, rr: RegPair) -> u8 {
        self.push_word(bus, self.regs.get_pair(rr));
        16
    }

    /// POP rr
    fn pop_rr(&mut self, bus: &mut Bus, rr: RegPair) -> u8 {
        let word = self.pop_word(bus);
        self.regs.set_pair(rr, word);
        12
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
    fn rl_r(&mut self, reg: Reg) -> u8 {
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

        8
    }

    /// RLA
    fn rla(&mut self) -> u8 {
        // Same as RL A but Z is always cleared
        self.rl_r(Reg::A);
        self.regs.flag_z().clear();
        4
    }

    fn add_hl(&mut self, bus: &mut Bus) -> u8 {
        let hl = bus.read_byte(*self.regs.hl);
        let (sum, carry) = self.regs.get(Reg::A).overflowing_add(hl);
        self.regs.set(Reg::A, sum);
        self.regs.flag_z().set_value(sum == 0);
        self.regs.flag_n().clear();
        self.regs.flag_c().set_value(carry);
        // TODO how to set H?
        8
    }

    fn sub_r(&mut self, reg: Reg) -> u8 {
        let d8 = self.regs.get(reg);
        let (sub, carry) = self.regs.get(Reg::A).overflowing_sub(d8);
        self.regs.set(Reg::A, sub);
        self.regs.flag_z().set_value(sub == 0);
        self.regs.flag_n().set();
        self.regs.flag_c().set_value(carry);
        // TODO how to set H?
        4
    }

    fn cp_hl(&mut self, bus: &mut Bus) -> u8 {
        let d8 = bus.read_byte(self.regs.get_pair(RegPair::HL));
        self.cp(d8);
        8
    }

    fn cp_d8(&mut self, bus: &mut Bus) -> u8 {
        let d8 = self.fetch(bus);
        self.cp(d8);
        8
    }

    fn cp(&mut self, value: u8) {
        let (sub, carry) = self.regs.get(Reg::A).overflowing_sub(value);
        self.regs.flag_z().set_value(sub == 0);
        self.regs.flag_n().set();
        self.regs.flag_c().set_value(carry);
        // TODO how to set H?
    }

    /// Get the cpu's paused.
    pub fn paused(&self) -> bool {
        self.paused
    }
}
