mod register;

use log::{info, trace, warn};
use serde::{Deserialize, Serialize};

use self::register::{FLAG_C, FLAG_H, FLAG_N, FLAG_Z, Reg, RegPair, Registers};
use crate::{bus::CpuBus, interrupt::InterruptFlag};

/// Index of the `(HL)` operand among the 8-bit operands of `Cpu::read_r8` and `Cpu::write_r8`.
const OPERAND_HL: u8 = 6;

/// The register pairs `PUSH` and `POP` encode in bits 4-5 of their opcode.
const RP2: [RegPair; 4] = [RegPair::BC, RegPair::DE, RegPair::HL, RegPair::AF];

/// Clock cycles taken by an instruction on an 8-bit operand: `hl` if it's `(HL)`, whose memory
/// accesses take longer, `reg` otherwise.
const fn operand_cycles(operand: u8, reg: u8, hl: u8) -> u8 {
    if operand == OPERAND_HL { hl } else { reg }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Cpu {
    regs: Registers,

    sp: u16,
    pc: u16,
    halted: bool,

    /// IME - Interrupt Master Enable Flag. Interrupts are disabled at power-on, and the boot ROM
    /// doesn't enable them either.
    ime: bool,
    /// Instructions left to run until EI takes effect: it only sets IME at the end of the
    /// instruction after it.
    ime_delay: u8,

    // for debugging, so not part of save states
    #[serde(skip)]
    breakpoint: Option<u16>,
    #[serde(skip)]
    paused: bool,
    // Pause cpu if LD B,B is encountered
    #[serde(skip)]
    enable_soft_break: bool,

    // Flag for the HALT bug
    halt_bug: bool,
    /// Set by an invalid opcode: the CPU stops for good, and ignores interrupts.
    locked_up: bool,
}

impl Cpu {
    /// Set the registers to the values the DMG boot ROM leaves them with when it jumps to the
    /// cartridge's entry point.
    pub fn skip_boot(&mut self, header_checksum: u8) {
        self.regs.set_pair(RegPair::AF, 0x0100);
        // The flags are left over from verifying the header: the last step adds the checksum to
        // a value that makes the total wrap to exactly 0.
        self.regs.set_flags(
            true,
            false,
            header_checksum & 0x0F != 0,
            header_checksum != 0,
        );
        self.regs.set_pair(RegPair::BC, 0x0013);
        self.regs.set_pair(RegPair::DE, 0x00D8);
        self.regs.set_pair(RegPair::HL, 0x014D);
        self.sp = 0xFFFE;
        self.pc = 0x0100;
    }

    /// AF, BC, DE, HL, SP, PC and IME.
    pub(crate) fn snapshot(&self) -> ([u16; 6], bool) {
        let r = &self.regs;
        ([*r.af, *r.bc, *r.de, *r.hl, self.sp, self.pc], self.ime)
    }

    pub fn handle_interrupt(&mut self, bus: &mut CpuBus<'_>) {
        if self.locked_up {
            return;
        }
        let pending_interrupts = !bus.pending_interrupts().is_empty();

        // if there are pending interrupts, we need to wake the cpu (even if IME=0)
        if pending_interrupts && self.halted {
            self.halted = false;
        }

        if self.ime && pending_interrupts {
            self.dispatch_interrupt(bus);
        }
    }

    /// Fetch and execute the next instruction.
    ///
    /// Return the number of clock cycles used
    pub fn step(&mut self, bus: &mut CpuBus<'_>) -> u8 {
        let cycles = self.execute(bus);
        if self.ime_delay > 0 {
            self.ime_delay -= 1;
            if self.ime_delay == 0 {
                self.ime = true;
            }
        }
        cycles
    }

    /// Run one instruction, and return how many clock cycles it takes. Its memory accesses run
    /// the rest of the hardware as they happen, and `GameBoy::step` runs whatever cycles are left
    /// after the last one.
    ///
    /// Most opcodes are laid out as `xxyyyzzz`, where `yyy` and `zzz` select an operation, a
    /// register or a condition, so they're decoded in groups.
    fn execute(&mut self, bus: &mut CpuBus<'_>) -> u8 {
        // for debugging
        if self.breakpoint == Some(self.pc) {
            self.paused = true;
        }
        if self.halted || self.locked_up {
            return 4;
        }
        let orig_pc = self.pc;
        let op = self.fetch(bus);
        let y = (op >> 3) & 0b111;
        let z = op & 0b111;
        // Bits 4-5, for the opcodes that encode a register pair
        let p = y >> 1;

        match op {
            // NOP
            0x00 => 4,
            // LD (a16),SP
            0x08 => {
                let addr = self.fetch_word(bus);
                let [lsb, msb] = self.sp.to_le_bytes();
                bus.write_byte(addr, lsb);
                bus.write_byte(addr.wrapping_add(1), msb);
                20
            }
            // STOP 0
            0x10 => {
                self.halted = true;
                trace!("STOP @{:04x}", orig_pc);
                4
            }
            // JR r8
            0x18 => self.jr_if(bus, true),
            // JR cc,r8
            0x20 | 0x28 | 0x30 | 0x38 => {
                let condition = self.condition(y);
                self.jr_if(bus, condition)
            }
            // LD rr,d16
            0x01 | 0x11 | 0x21 | 0x31 => {
                let d16 = self.fetch_word(bus);
                self.set_rp(p, d16);
                12
            }
            // ADD HL,rr
            0x09 | 0x19 | 0x29 | 0x39 => {
                let hl = *self.regs.hl;
                let rr = self.rp(p);
                let (sum, carry) = hl.overflowing_add(rr);
                *self.regs.hl = sum;
                let half_carry = (hl & 0x0FFF) + (rr & 0x0FFF) > 0x0FFF;
                self.regs.set_flag(FLAG_N, false);
                self.regs.set_flag(FLAG_H, half_carry);
                self.regs.set_flag(FLAG_C, carry);
                8
            }
            // LD (BC),A / LD (DE),A / LD (HL+),A / LD (HL-),A
            0x02 | 0x12 | 0x22 | 0x32 => {
                let addr = self.indirect_addr(p);
                bus.write_byte(addr, self.regs.get(Reg::A));
                8
            }
            // LD A,(BC) / LD A,(DE) / LD A,(HL+) / LD A,(HL-)
            0x0A | 0x1A | 0x2A | 0x3A => {
                let addr = self.indirect_addr(p);
                self.regs.set(Reg::A, bus.read_byte(addr));
                8
            }
            // INC rr
            0x03 | 0x13 | 0x23 | 0x33 => {
                self.set_rp(p, self.rp(p).wrapping_add(1));
                8
            }
            // DEC rr
            0x0B | 0x1B | 0x2B | 0x3B => {
                self.set_rp(p, self.rp(p).wrapping_sub(1));
                8
            }
            // INC r
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
                let v = self.read_r8(bus, y);
                let result = self.inc(v);
                self.write_r8(bus, y, result);
                operand_cycles(y, 4, 12)
            }
            // DEC r
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
                let v = self.read_r8(bus, y);
                let result = self.dec(v);
                self.write_r8(bus, y, result);
                operand_cycles(y, 4, 12)
            }
            // LD r,d8
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => {
                let d8 = self.fetch(bus);
                self.write_r8(bus, y, d8);
                operand_cycles(y, 8, 12)
            }
            // RLCA / RRCA / RLA / RRA: like the CB-prefixed versions on A, but Z is always cleared
            0x07 | 0x0F | 0x17 | 0x1F => {
                let result = self.rotate_shift(y, self.regs.get(Reg::A));
                self.regs.set(Reg::A, result);
                self.regs.set_flag(FLAG_Z, false);
                4
            }
            // DAA
            0x27 => self.daa(),
            // CPL
            0x2F => {
                self.regs.set(Reg::A, !self.regs.get(Reg::A));
                self.regs.set_flag(FLAG_N, true);
                self.regs.set_flag(FLAG_H, true);
                4
            }
            // SCF
            0x37 => {
                self.regs.set_flag(FLAG_N, false);
                self.regs.set_flag(FLAG_H, false);
                self.regs.set_flag(FLAG_C, true);
                4
            }
            // CCF
            0x3F => {
                self.regs.set_flag(FLAG_N, false);
                self.regs.set_flag(FLAG_H, false);
                self.regs.set_flag(FLAG_C, !self.regs.flag(FLAG_C));
                4
            }
            // HALT
            0x76 => {
                if !self.ime && bus.interrupt_pending() {
                    // HALT bug: don't pause the CPU, but the next byte gets read twice
                    self.halt_bug = true;
                } else {
                    // Pause the CPU until the next interrupt
                    self.halted = true;
                }
                4
            }
            // LD r,r'
            0x40..=0x7F => {
                if self.enable_soft_break && op == 0x40 {
                    info!("Software breakpoint triggered");
                    self.paused = true;
                }
                let v = self.read_r8(bus, z);
                self.write_r8(bus, y, v);
                if y == OPERAND_HL || z == OPERAND_HL {
                    8
                } else {
                    4
                }
            }
            // ADD/ADC/SUB/SBC/AND/XOR/OR/CP A,r
            0x80..=0xBF => {
                let v = self.read_r8(bus, z);
                self.alu(y, v);
                operand_cycles(z, 4, 8)
            }
            // ADD/ADC/SUB/SBC/AND/XOR/OR/CP A,d8
            0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                let d8 = self.fetch(bus);
                self.alu(y, d8);
                8
            }
            // RET cc, which takes an internal M-cycle to check the condition first
            0xC0 | 0xC8 | 0xD0 | 0xD8 => {
                bus.tick();
                if self.condition(y) {
                    self.pc = self.pop_word(bus);
                    20
                } else {
                    8
                }
            }
            // RET
            0xC9 => {
                self.pc = self.pop_word(bus);
                16
            }
            // RETI
            0xD9 => {
                self.pc = self.pop_word(bus);
                self.ime = true;
                trace!("Returning from interrupt handler to 0x{:04x}", self.pc);
                16
            }
            // JP cc,a16
            0xC2 | 0xCA | 0xD2 | 0xDA => {
                let condition = self.condition(y);
                self.jp_if(bus, condition)
            }
            // JP a16
            0xC3 => self.jp_if(bus, true),
            // JP HL
            0xE9 => {
                self.pc = *self.regs.hl;
                4
            }
            // CALL cc,a16
            0xC4 | 0xCC | 0xD4 | 0xDC => {
                let condition = self.condition(y);
                self.call_if(bus, condition)
            }
            // CALL a16
            0xCD => self.call_if(bus, true),
            // RST
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                self.call(bus, u16::from(y) * 8);
                16
            }
            // POP rr
            0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                let word = self.pop_word(bus);
                self.regs.set_pair(RP2[p as usize], word);
                12
            }
            // PUSH rr
            0xC5 | 0xD5 | 0xE5 | 0xF5 => {
                self.push_word(bus, self.regs.get_pair(RP2[p as usize]));
                16
            }
            // CB prefix
            0xCB => self.execute_cb(bus),
            // LDH (a8),A
            0xE0 => {
                let addr = 0xFF00 | u16::from(self.fetch(bus));
                bus.write_byte(addr, self.regs.get(Reg::A));
                12
            }
            // LDH A,(a8)
            0xF0 => {
                let addr = 0xFF00 | u16::from(self.fetch(bus));
                self.regs.set(Reg::A, bus.read_byte(addr));
                12
            }
            // LD (C),A
            0xE2 => {
                let addr = 0xFF00 | u16::from(self.regs.get(Reg::C));
                bus.write_byte(addr, self.regs.get(Reg::A));
                8
            }
            // LD A,(C)
            0xF2 => {
                let addr = 0xFF00 | u16::from(self.regs.get(Reg::C));
                self.regs.set(Reg::A, bus.read_byte(addr));
                8
            }
            // LD (a16),A
            0xEA => {
                let addr = self.fetch_word(bus);
                bus.write_byte(addr, self.regs.get(Reg::A));
                16
            }
            // LD A,(a16)
            0xFA => {
                let addr = self.fetch_word(bus);
                self.regs.set(Reg::A, bus.read_byte(addr));
                16
            }
            // ADD SP,r8
            0xE8 => {
                self.sp = self.sp_plus_r8(bus);
                16
            }
            // LD HL,SP+r8
            0xF8 => {
                *self.regs.hl = self.sp_plus_r8(bus);
                12
            }
            // LD SP,HL
            0xF9 => {
                self.sp = *self.regs.hl;
                8
            }
            // DI
            0xF3 => {
                trace!("Disabling interrupts");
                self.ime = false;
                self.ime_delay = 0;
                4
            }
            // EI
            0xFB => {
                trace!("Enabling interrupts");
                // A second EI right after the first doesn't delay it any further
                if self.ime_delay == 0 {
                    self.ime_delay = 2;
                }
                4
            }
            // The remaining opcodes don't exist, and lock the CPU up until it's powered off.
            _ => {
                warn!("Invalid opcode 0x{op:02x} at 0x{orig_pc:04x}: locking up");
                self.locked_up = true;
                4
            }
        }
    }

    /// Run a CB-prefixed instruction: a rotation or shift, BIT, RES or SET.
    fn execute_cb(&mut self, bus: &mut CpuBus<'_>) -> u8 {
        let op = self.fetch(bus);
        // The operation, or the bit number for BIT, RES and SET
        let y = (op >> 3) & 0b111;
        let z = op & 0b111;
        let v = self.read_r8(bus, z);
        match op >> 6 {
            0 => {
                let result = self.rotate_shift(y, v);
                self.write_r8(bus, z, result);
                operand_cycles(z, 8, 16)
            }
            // BIT
            1 => {
                self.regs.set_flag(FLAG_Z, v & (1 << y) == 0);
                self.regs.set_flag(FLAG_N, false);
                self.regs.set_flag(FLAG_H, true);
                operand_cycles(z, 8, 12)
            }
            // RES
            2 => {
                self.write_r8(bus, z, v & !(1 << y));
                operand_cycles(z, 8, 16)
            }
            // SET
            _ => {
                self.write_r8(bus, z, v | (1 << y));
                operand_cycles(z, 8, 16)
            }
        }
    }

    // TODO probably should implement Debug instead...
    pub fn dump_cpu(&self) {
        println!(
            "PC=${:04X}, SP=${:04X}, regs={:?}, IME={}",
            self.pc, self.sp, self.regs, self.ime
        );
    }

    fn fetch(&mut self, bus: &mut CpuBus<'_>) -> u8 {
        let byte = bus.read_byte(self.pc);
        if self.halt_bug {
            // Don't increment PC so the same byte is read again
            // See https://gbdev.io/pandocs/halt.html#halt-bug
            self.halt_bug = false;
        } else {
            self.pc += 1;
        }
        byte
    }

    fn fetch_word(&mut self, bus: &mut CpuBus<'_>) -> u16 {
        let lsb = self.fetch(bus);
        let msb = self.fetch(bus);
        u16::from_le_bytes([lsb, msb])
    }

    /// Read one of the 8-bit operands opcodes encode in 3 bits: B, C, D, E, H, L, (HL) or A.
    fn read_r8(&mut self, bus: &mut CpuBus<'_>, operand: u8) -> u8 {
        match operand {
            0 => self.regs.get(Reg::B),
            1 => self.regs.get(Reg::C),
            2 => self.regs.get(Reg::D),
            3 => self.regs.get(Reg::E),
            4 => self.regs.get(Reg::H),
            5 => self.regs.get(Reg::L),
            OPERAND_HL => bus.read_byte(*self.regs.hl),
            _ => self.regs.get(Reg::A),
        }
    }

    /// Write one of the 8-bit operands, like `read_r8`.
    fn write_r8(&mut self, bus: &mut CpuBus<'_>, operand: u8, v: u8) {
        match operand {
            0 => self.regs.set(Reg::B, v),
            1 => self.regs.set(Reg::C, v),
            2 => self.regs.set(Reg::D, v),
            3 => self.regs.set(Reg::E, v),
            4 => self.regs.set(Reg::H, v),
            5 => self.regs.set(Reg::L, v),
            OPERAND_HL => bus.write_byte(*self.regs.hl, v),
            _ => self.regs.set(Reg::A, v),
        }
    }

    /// One of the register pairs opcodes encode in bits 4-5: BC, DE, HL or SP.
    fn rp(&self, p: u8) -> u16 {
        match p {
            0 => *self.regs.bc,
            1 => *self.regs.de,
            2 => *self.regs.hl,
            _ => self.sp,
        }
    }

    fn set_rp(&mut self, p: u8, v: u16) {
        match p {
            0 => *self.regs.bc = v,
            1 => *self.regs.de = v,
            2 => *self.regs.hl = v,
            _ => self.sp = v,
        }
    }

    /// The address for `LD (rr),A` and `LD A,(rr)`: BC, DE, HL then incremented, or HL then
    /// decremented.
    fn indirect_addr(&mut self, p: u8) -> u16 {
        match p {
            0 => *self.regs.bc,
            1 => *self.regs.de,
            2 => {
                let hl = *self.regs.hl;
                *self.regs.hl = hl.wrapping_add(1);
                hl
            }
            _ => {
                let hl = *self.regs.hl;
                *self.regs.hl = hl.wrapping_sub(1);
                hl
            }
        }
    }

    /// One of the conditions opcodes encode in bits 3-4: NZ, Z, NC or C.
    fn condition(&self, cc: u8) -> bool {
        match cc & 0b11 {
            0 => !self.regs.flag(FLAG_Z),
            1 => self.regs.flag(FLAG_Z),
            2 => !self.regs.flag(FLAG_C),
            _ => self.regs.flag(FLAG_C),
        }
    }

    /// Run an 8-bit arithmetic or logic operation on A and `v`: ADD, ADC, SUB, SBC, AND, XOR, OR
    /// or CP, in the order opcodes encode them.
    fn alu(&mut self, operation: u8, v: u8) {
        let a = self.regs.get(Reg::A);
        let carry = u8::from(self.regs.flag(FLAG_C));
        let (result, n, h, c) = match operation {
            // ADD, ADC
            0 | 1 => {
                let carry = if operation == 1 { carry } else { 0 };
                let (sum, c1) = a.overflowing_add(v);
                let (sum, c2) = sum.overflowing_add(carry);
                (sum, false, (a & 0x0F) + (v & 0x0F) + carry > 0x0F, c1 | c2)
            }
            // SUB, SBC, CP
            2 | 3 | 7 => {
                let carry = if operation == 3 { carry } else { 0 };
                let (diff, c1) = a.overflowing_sub(v);
                let (diff, c2) = diff.overflowing_sub(carry);
                (diff, true, (a & 0x0F) < (v & 0x0F) + carry, c1 | c2)
            }
            4 => (a & v, false, true, false),
            5 => (a ^ v, false, false, false),
            _ => (a | v, false, false, false),
        };
        self.regs.set_flags(result == 0, n, h, c);
        // CP only compares
        if operation != 7 {
            self.regs.set(Reg::A, result);
        }
    }

    /// Run a rotation or shift: RLC, RRC, RL, RR, SLA, SRA, SWAP or SRL, in the order CB-prefixed
    /// opcodes encode them.
    fn rotate_shift(&mut self, operation: u8, v: u8) -> u8 {
        let carry = u8::from(self.regs.flag(FLAG_C));
        let (result, c) = match operation {
            0 => (v.rotate_left(1), v & 0x80 != 0),
            1 => (v.rotate_right(1), v & 0x01 != 0),
            // Through the carry
            2 => (v << 1 | carry, v & 0x80 != 0),
            3 => (v >> 1 | carry << 7, v & 0x01 != 0),
            4 => (v << 1, v & 0x80 != 0),
            // Keeps bit 7
            5 => (((v as i8) >> 1) as u8, v & 0x01 != 0),
            6 => (v.rotate_right(4), false),
            _ => (v >> 1, v & 0x01 != 0),
        };
        self.regs.set_flags(result == 0, false, false, c);
        result
    }

    /// INC for 8-bit values, which leaves C alone.
    fn inc(&mut self, v: u8) -> u8 {
        let result = v.wrapping_add(1);
        self.regs.set_flag(FLAG_Z, result == 0);
        self.regs.set_flag(FLAG_N, false);
        self.regs.set_flag(FLAG_H, v & 0x0F == 0x0F);
        result
    }

    /// DEC for 8-bit values, which leaves C alone.
    fn dec(&mut self, v: u8) -> u8 {
        let result = v.wrapping_sub(1);
        self.regs.set_flag(FLAG_Z, result == 0);
        self.regs.set_flag(FLAG_N, true);
        self.regs.set_flag(FLAG_H, v & 0x0F == 0);
        result
    }

    /// SP plus a signed immediate, for `ADD SP,r8` and `LD HL,SP+r8`. The flags come from adding
    /// the low bytes.
    fn sp_plus_r8(&mut self, bus: &mut CpuBus<'_>) -> u16 {
        // sign extend r8 to 16 bits
        let r8 = self.fetch(bus) as i8 as u16;
        let sp = self.sp;
        self.regs.set_flags(
            false,
            false,
            (sp & 0x000F) + (r8 & 0x000F) > 0x000F,
            (sp & 0x00FF) + (r8 & 0x00FF) > 0x00FF,
        );
        sp.wrapping_add(r8)
    }

    fn daa(&mut self) -> u8 {
        let mut a = self.regs.get(Reg::A);
        let mut adjust = if self.regs.flag(FLAG_C) { 0x60 } else { 0x00 };
        if self.regs.flag(FLAG_H) {
            adjust |= 0x06;
        }
        if !self.regs.flag(FLAG_N) {
            if a & 0x0f > 0x09 {
                adjust |= 0x06;
            }
            if a > 0x99 {
                adjust |= 0x60;
            }
            a = a.wrapping_add(adjust);
        } else {
            a = a.wrapping_sub(adjust);
        }
        self.regs.set_flag(FLAG_Z, a == 0);
        self.regs.set_flag(FLAG_H, false);
        self.regs.set_flag(FLAG_C, adjust >= 0x60);
        self.regs.set(Reg::A, a);
        4
    }

    /// Conditional relative jump
    fn jr_if(&mut self, bus: &mut CpuBus<'_>, condition: bool) -> u8 {
        let r8 = self.fetch(bus) as i8;
        if condition {
            self.pc = self.pc.wrapping_add(r8 as u16);
            12
        } else {
            8
        }
    }

    /// Conditional absolute jump
    fn jp_if(&mut self, bus: &mut CpuBus<'_>, condition: bool) -> u8 {
        let a16 = self.fetch_word(bus);
        if condition {
            self.pc = a16;
            16
        } else {
            12
        }
    }

    /// Conditional CALL
    fn call_if(&mut self, bus: &mut CpuBus<'_>, condition: bool) -> u8 {
        let addr = self.fetch_word(bus);
        if condition {
            self.call(bus, addr);
            24
        } else {
            12
        }
    }

    fn call(&mut self, bus: &mut CpuBus<'_>, addr: u16) {
        trace!("Calling subroutine at 0x{:04x}", addr);
        self.push_word(bus, self.pc);
        self.pc = addr;
    }

    /// Jump to the handler of the highest priority pending interrupt, which takes 5 M-cycles.
    ///
    /// Which interrupt that is only gets decided after the high byte of PC has been pushed, which
    /// can overwrite IE (at 0xFFFF): if no interrupt is left pending then, execution continues at
    /// 0x0000.
    fn dispatch_interrupt(&mut self, bus: &mut CpuBus<'_>) {
        self.ime = false;
        if self.halt_bug {
            // HALT right after EI, with an interrupt pending: HALT still ran with IME=0, so it
            // triggered the halt bug, but the handler returns to the HALT, which runs again.
            self.halt_bug = false;
            self.pc = self.pc.wrapping_sub(1);
        }
        let [lsb, msb] = self.pc.to_le_bytes();
        bus.tick();
        bus.tick();
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, msb);

        // The lowest bit has the highest priority, and the handlers are 8 bytes apart from 0x0040.
        let pending = bus.pending_interrupts().bits();
        self.pc = if pending == 0 {
            0x0000
        } else {
            let bit = pending.trailing_zeros();
            let flag = InterruptFlag::from_bits_retain(1 << bit);
            trace!("Handling {flag:?} interrupt");
            bus.ack_interrupt(flag);
            0x0040 + 8 * bit as u16
        };

        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, lsb);
        bus.tick();
    }

    /// Push a word on the stack: an internal M-cycle to decrement SP, then the high byte and the
    /// low byte.
    fn push_word(&mut self, bus: &mut CpuBus<'_>, word: u16) {
        let [lsb, msb] = word.to_le_bytes();
        bus.tick();
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, msb);
        self.sp = self.sp.wrapping_sub(1);
        bus.write_byte(self.sp, lsb);
    }

    /// Pop a word from the stack, low byte first.
    fn pop_word(&mut self, bus: &mut CpuBus<'_>) -> u16 {
        let lsb = bus.read_byte(self.sp);
        self.sp = self.sp.wrapping_add(1);
        let msb = bus.read_byte(self.sp);
        self.sp = self.sp.wrapping_add(1);
        u16::from_le_bytes([lsb, msb])
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_pause(&mut self, pause: bool) {
        self.paused = pause;
    }

    /// Carry over what save states leave out (the debugger's state) from the `Cpu` this one
    /// replaces.
    pub(crate) fn restore_unsaved(&mut self, previous: &Self) {
        self.breakpoint = previous.breakpoint;
        self.paused = previous.paused;
        self.enable_soft_break = previous.enable_soft_break;
    }

    /// Set the cpu's breakpoint.
    pub fn set_breakpoint(&mut self, breakpoint: u16) {
        self.breakpoint = Some(breakpoint);
    }

    /// Make `LD B,B` pause the CPU, like a breakpoint.
    pub fn set_soft_break(&mut self, enabled: bool) {
        self.enable_soft_break = enabled;
    }

    /// Whether the CPU is halted, and stepping it would do nothing but wait for an interrupt: no
    /// EI about to take effect, and no breakpoint to hit.
    pub(crate) fn idle_while_halted(&self) -> bool {
        self.halted && !self.locked_up && self.ime_delay == 0 && self.breakpoint != Some(self.pc)
    }

    /// Get the cpu's halted.
    pub fn halted(&self) -> bool {
        self.halted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bus::Bus, cartridge::Cartridge};

    /// Number of cycles taken by the first instruction of `program`.
    fn cycles(program: &[u8]) -> u8 {
        let mut rom = vec![0; 0x8000];
        rom[..program.len()].copy_from_slice(program);
        let cartridge = Cartridge::load_bytes(rom).unwrap();
        let mut bus = Bus::new(cartridge, 48_000, None);
        Cpu::default().step(&mut CpuBus::new(&mut bus, &mut (), &mut ()))
    }

    #[test]
    fn instruction_timings() {
        // All flags are clear, so NZ and NC hold, and Z and C don't.
        for (program, expected, name) in [
            (&[0x00][..], 4, "NOP"),
            (&[0x80], 4, "ADD A,B"),
            (&[0x88], 4, "ADC A,B"),
            (&[0x98], 4, "SBC A,B"),
            (&[0x86], 8, "ADD A,(HL)"),
            (&[0xBE], 8, "CP (HL)"),
            (&[0xC6, 0x01], 8, "ADD A,d8"),
            (&[0xCE, 0x01], 8, "ADC A,d8"),
            (&[0xDE, 0x01], 8, "SBC A,d8"),
            (&[0x2F], 4, "CPL"),
            (&[0x27], 4, "DAA"),
            (&[0x07], 4, "RLCA"),
            (&[0x41], 4, "LD B,C"),
            (&[0x46], 8, "LD B,(HL)"),
            (&[0x70], 8, "LD (HL),B"),
            (&[0x06, 0x01], 8, "LD B,d8"),
            (&[0x36, 0x01], 12, "LD (HL),d8"),
            (&[0x0A], 8, "LD A,(BC)"),
            (&[0x22], 8, "LD (HL+),A"),
            (&[0x3A], 8, "LD A,(HL-)"),
            (&[0x04], 4, "INC B"),
            (&[0x35], 12, "DEC (HL)"),
            (&[0x03], 8, "INC BC"),
            (&[0x3B], 8, "DEC SP"),
            (&[0x09], 8, "ADD HL,BC"),
            (&[0x01, 0x34, 0x12], 12, "LD BC,d16"),
            (&[0x08, 0x00, 0xC0], 20, "LD (a16),SP"),
            (&[0xE0, 0x80], 12, "LDH (a8),A"),
            (&[0xF0, 0x80], 12, "LDH A,(a8)"),
            (&[0xE2], 8, "LD (C),A"),
            (&[0xEA, 0x00, 0xC0], 16, "LD (a16),A"),
            (&[0xFA, 0x00, 0xC0], 16, "LD A,(a16)"),
            (&[0xE8, 0x01], 16, "ADD SP,r8"),
            (&[0xF8, 0x01], 12, "LD HL,SP+r8"),
            (&[0xF9], 8, "LD SP,HL"),
            (&[0xC5], 16, "PUSH BC"),
            (&[0xF1], 12, "POP AF"),
            (&[0x18, 0x00], 12, "JR r8"),
            (&[0x20, 0x00], 12, "JR NZ,r8 (taken)"),
            (&[0x28, 0x00], 8, "JR Z,r8 (not taken)"),
            (&[0xC3, 0x00, 0x00], 16, "JP a16"),
            (&[0xD2, 0x00, 0x00], 16, "JP NC,a16 (taken)"),
            (&[0xDA, 0x00, 0x00], 12, "JP C,a16 (not taken)"),
            (&[0xE9], 4, "JP HL"),
            (&[0xCD, 0x00, 0x00], 24, "CALL a16"),
            (&[0xC4, 0x00, 0x00], 24, "CALL NZ,a16 (taken)"),
            (&[0xCC, 0x00, 0x00], 12, "CALL Z,a16 (not taken)"),
            (&[0xC9], 16, "RET"),
            (&[0xD9], 16, "RETI"),
            (&[0xD0], 20, "RET NC (taken)"),
            (&[0xD8], 8, "RET C (not taken)"),
            (&[0xFF], 16, "RST 38H"),
            (&[0xF3], 4, "DI"),
            (&[0xFB], 4, "EI"),
            (&[0xCB, 0x00], 8, "RLC B"),
            (&[0xCB, 0x06], 16, "RLC (HL)"),
            (&[0xCB, 0x36], 16, "SWAP (HL)"),
            (&[0xCB, 0x40], 8, "BIT 0,B"),
            (&[0xCB, 0x46], 12, "BIT 0,(HL)"),
            (&[0xCB, 0x86], 16, "RES 0,(HL)"),
            (&[0xCB, 0xFF], 8, "SET 7,A"),
            (&[0xCB, 0xFE], 16, "SET 7,(HL)"),
        ] {
            assert_eq!(cycles(program), expected, "{name}");
        }
    }

    /// Run a rotation or shift on `value` with the given carry, and return the result and the
    /// Z and C flags.
    fn rotate_shift(operation: u8, value: u8, carry: bool) -> (u8, bool, bool) {
        let mut cpu = Cpu::default();
        cpu.regs.set_flag(FLAG_C, carry);
        let result = cpu.rotate_shift(operation, value);
        (result, cpu.regs.flag(FLAG_Z), cpu.regs.flag(FLAG_C))
    }

    #[test]
    fn test_rl() {
        let rl = |value, carry| rotate_shift(2, value, carry);
        assert_eq!(rl(0b01010101, false), (0b10101010, false, false));
        assert_eq!(rl(0b01010101, true), (0b10101011, false, false));
        assert_eq!(rl(0b10101010, false), (0b01010100, false, true));
        assert_eq!(rl(0b10101010, true), (0b01010101, false, true));
        // Make sure Z flag gets set
        assert_eq!(rl(0b10000000, false), (0, true, true));
    }

    #[test]
    fn test_rr() {
        let rr = |value, carry| rotate_shift(3, value, carry);
        assert_eq!(rr(0b01010101, false), (0b00101010, false, true));
        assert_eq!(rr(0b01010101, true), (0b10101010, false, true));
        assert_eq!(rr(0b10101010, false), (0b01010101, false, false));
        assert_eq!(rr(0b10101010, true), (0b11010101, false, false));
        // Make sure Z flag gets set
        assert_eq!(rr(0b00000001, false), (0, true, true));
    }

    #[test]
    fn test_sra_and_swap() {
        assert_eq!(
            rotate_shift(5, 0b1000_0001, false),
            (0b1100_0000, false, true)
        );
        assert_eq!(rotate_shift(6, 0xF1, true), (0x1F, false, false));
    }

    /// The Z, N, H and C flags.
    fn flags(cpu: &Cpu) -> (bool, bool, bool, bool) {
        let r = &cpu.regs;
        (
            r.flag(FLAG_Z),
            r.flag(FLAG_N),
            r.flag(FLAG_H),
            r.flag(FLAG_C),
        )
    }

    #[test]
    fn test_inc() {
        let mut cpu = Cpu::default();
        assert_eq!(cpu.inc(0x00), 0x01);
        assert_eq!(flags(&cpu), (false, false, false, false));
        // H is set
        assert_eq!(cpu.inc(0x0F), 0x10);
        assert_eq!(flags(&cpu), (false, false, true, false));
        // Z is set, and C is left alone
        assert_eq!(cpu.inc(0xFF), 0x00);
        assert_eq!(flags(&cpu), (true, false, true, false));
    }

    #[test]
    fn test_dec() {
        let mut cpu = Cpu::default();
        assert_eq!(cpu.dec(0x02), 0x01);
        assert_eq!(flags(&cpu), (false, true, false, false));
        // H is set
        assert_eq!(cpu.dec(0x10), 0x0F);
        assert_eq!(flags(&cpu), (false, true, true, false));
        // Z is set, and C is left alone
        assert_eq!(cpu.dec(0x01), 0x00);
        assert_eq!(flags(&cpu), (true, true, false, false));
    }

    #[test]
    fn test_alu() {
        let alu = |operation, a, v, carry| {
            let mut cpu = Cpu::default();
            cpu.regs.set(Reg::A, a);
            cpu.regs.set_flag(FLAG_C, carry);
            cpu.alu(operation, v);
            (cpu.regs.get(Reg::A), flags(&cpu))
        };
        // ADD, ADC
        assert_eq!(
            alu(0, 0x0F, 0x01, true),
            (0x10, (false, false, true, false))
        );
        assert_eq!(alu(1, 0xFF, 0x00, true), (0x00, (true, false, true, true)));
        // SUB, SBC
        assert_eq!(alu(2, 0x10, 0x01, true), (0x0F, (false, true, true, false)));
        assert_eq!(alu(3, 0x00, 0x00, true), (0xFF, (false, true, true, true)));
        // AND, XOR, OR
        assert_eq!(alu(4, 0xF0, 0x0F, true), (0x00, (true, false, true, false)));
        assert_eq!(
            alu(5, 0xFF, 0x0F, true),
            (0xF0, (false, false, false, false))
        );
        assert_eq!(
            alu(6, 0xF0, 0x0F, true),
            (0xFF, (false, false, false, false))
        );
        // CP leaves A alone
        assert_eq!(
            alu(7, 0x42, 0x42, false),
            (0x42, (true, true, false, false))
        );
    }

    #[test]
    fn test_skip_boot_flags() {
        let af = |header_checksum| {
            let mut cpu = Cpu::default();
            cpu.skip_boot(header_checksum);
            cpu.regs.get_pair(RegPair::AF)
        };

        assert_eq!(af(0x00), 0x0180);
        assert_eq!(af(0xE7), 0x01B0);
        // No half carry when the low nibbles are both 0
        assert_eq!(af(0x30), 0x0190);
    }
}
