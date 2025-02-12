mod register;

use bitvec::{order::Lsb0, view::BitView};
use log::{info, trace, warn};

use self::register::{Reg, RegPair, Registers};
use crate::{bus::Bus, interrupt::InterruptFlag};

const ITR_VBLANK: u16 = 0x0040;
const ITR_STAT: u16 = 0x0048;
const ITR_TIMER: u16 = 0x0050;
const ITR_SERIAL: u16 = 0x0058;
const ITR_JOYP: u16 = 0x0060;

pub struct Cpu {
    regs: Registers,

    sp: u16,
    pc: u16,
    halted: bool,

    /// IME - Interrupt Master Enable Flag
    ime: bool,

    // for debugging
    breakpoint: u16,
    paused: bool,
    // Pause cpu if LD B,B is encountered
    enable_soft_break: bool,

    // Flag for the HALT bug
    halt_bug: bool,
}

impl Default for Cpu {
    fn default() -> Self {
        Self {
            regs: Registers::default(),
            sp: Default::default(),
            pc: Default::default(),
            halted: false,
            ime: true, // is this correct?
            // breakpoint: 0x0100,
            breakpoint: 0xffff,
            paused: Default::default(),
            enable_soft_break: false,
            halt_bug: false,
        }
    }
}

impl Cpu {
    pub fn with_breakpoint(breakpoint: Option<u16>, enable_soft_break: bool) -> Self {
        Self {
            breakpoint: breakpoint.unwrap_or(0xffff),
            enable_soft_break,
            ..Self::default()
        }
    }

    pub fn handle_interrupt(&mut self, bus: &mut Bus) {
        let interrupt_flag = bus.interrupt_flag();
        let interrupt_enable = bus.interrupt_enable();
        let pending_interrupts = bus.interrupt_pending();

        // if there are pending interrupts, we need to wake the cpu (even if IME=0)
        if pending_interrupts && self.halted {
            self.halted = false;
        }

        if !self.ime || !pending_interrupts {
            // If interrupts are disabled, or no pending interrupts, just return
            // debug!("interrupts are disabled, ignoring");
            return;
        }
        // debug!("Handling interrupts: pending: {:?} / enabled: {:?}", interrupt_flag, interrupt_enable);

        let should_handle = |f: InterruptFlag| -> bool {
            interrupt_flag.contains(f) && interrupt_enable.contains(f)
        };

        // These need to be ordered by priority:
        if should_handle(InterruptFlag::VBLANK) {
            trace!("Handling VBLANK interrupt");
            self.call_interrupt(bus, InterruptFlag::VBLANK);
        } else if should_handle(InterruptFlag::STAT) {
            trace!("Handling STAT interrupt");
            self.call_interrupt(bus, InterruptFlag::STAT);
        } else if should_handle(InterruptFlag::TIMER) {
            trace!("Handling TIMER interrupt");
            self.call_interrupt(bus, InterruptFlag::TIMER);
        } else if should_handle(InterruptFlag::SERIAL) {
            trace!("Handling SERIAL interrupt");
            self.call_interrupt(bus, InterruptFlag::SERIAL);
        } else if should_handle(InterruptFlag::JOYPAD) {
            trace!("Handling JOYPAD interrupt");
            self.call_interrupt(bus, InterruptFlag::JOYPAD);
        }
    }

    fn get_itr_vector(&self, flag: InterruptFlag) -> u16 {
        match flag {
            InterruptFlag::VBLANK => ITR_VBLANK,
            InterruptFlag::STAT => ITR_STAT,
            InterruptFlag::TIMER => ITR_TIMER,
            InterruptFlag::SERIAL => ITR_SERIAL,
            InterruptFlag::JOYPAD => ITR_JOYP,
            _ => unimplemented!(),
        }
    }

    /// Fetch and execute the next instructions.
    ///
    /// Return the number of clock cycles used
    pub fn step(&mut self, bus: &mut Bus) -> u8 {
        // for debugging
        if self.pc == self.breakpoint {
            self.paused = true;
        }
        if self.halted {
            return 4;
        }

        let orig_pc = self.pc;
        let op = self.fetch(bus);

        let cycles = match op {
            // NOP
            0x00 => 4,
            // LD BC,d16
            0x01 => self.ld_rr_d16(bus, RegPair::BC),
            // LD (BC),A
            0x02 => self.ld_addr_r(bus, RegPair::BC, Reg::A),
            // INC BC
            0x03 => self.inc_rr(RegPair::BC),
            // INC B
            0x04 => self.inc_r(Reg::B),
            // DEC B
            0x05 => self.dec_r(Reg::B),
            // LD B,d8
            0x06 => self.ld_r_d8(bus, Reg::B),
            // RLCA
            0x07 => self.rlca(),
            // LD (a16),SP
            0x08 => {
                let addr = self.fetch_word(bus);
                bus.write_word(addr, self.sp);
                20
            }
            // ADD HL,BC
            0x09 => self.add_rr_rr(RegPair::HL, *self.regs.bc),
            // LD A,(BC)
            0x0a => self.ld_r_addr(bus, Reg::A, RegPair::BC),
            // DEC BC
            0x0b => self.dec_rr(RegPair::BC),
            // INC C
            0x0c => self.inc_r(Reg::C),
            // DEC C
            0x0d => self.dec_r(Reg::C),
            // LD C,d8
            0x0e => self.ld_r_d8(bus, Reg::C),
            // RRCA
            0x0f => self.rrca(),
            // STOP 0
            0x10 => {
                self.halted = true;
                trace!("STOP @{:04x}, halted={}", orig_pc, self.halted);
                4
            }
            // LD DE,d16
            0x11 => self.ld_rr_d16(bus, RegPair::DE),
            // LD (DE),A
            0x12 => self.ld_addr_r(bus, RegPair::DE, Reg::A),
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
            // ADD HL,DE
            0x19 => self.add_rr_rr(RegPair::HL, *self.regs.de),
            // LD A,(DE)
            0x1a => self.ld_r_addr(bus, Reg::A, RegPair::DE),
            // DEC DE
            0x1b => self.dec_rr(RegPair::DE),
            // INC E
            0x1c => self.inc_r(Reg::E),
            // DEC E
            0x1d => self.dec_r(Reg::E),
            // LD E,d8
            0x1e => self.ld_r_d8(bus, Reg::E),
            // RRA
            0x1f => self.rra(),
            // JR NZ,r8
            0x20 => {
                // NZ
                let flag = !self.regs.flag_z().is_set();
                self.jr_if_r8(bus, flag)
            }
            // LD HL,d16
            0x21 => self.ld_rr_d16(bus, RegPair::HL),
            // LD (HL+),A
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
            // LD H,d8
            0x26 => self.ld_r_d8(bus, Reg::H),
            // DAA
            0x27 => self.daa(),
            // JR Z,r8
            0x28 => {
                // Z
                let flag = self.regs.flag_z().is_set();
                self.jr_if_r8(bus, flag)
            }
            // ADD HL,HL
            0x29 => self.add_rr_rr(RegPair::HL, *self.regs.hl),
            // LD A,(HL+)
            0x2a => {
                self.ld_r_addr(bus, Reg::A, RegPair::HL);
                *self.regs.hl = self.regs.hl.wrapping_add(1);
                8
            }
            // DEC HL
            0x2b => self.dec_rr(RegPair::HL),
            // INC L
            0x2c => self.inc_r(Reg::L),
            // DEC L
            0x2d => self.dec_r(Reg::L),
            // LD L,d8
            0x2e => self.ld_r_d8(bus, Reg::L),
            // CPL
            0x2f => self.cpl(),
            // JR NC,r8
            0x30 => {
                // NC
                let flag = !self.regs.flag_c().is_set();
                self.jr_if_r8(bus, flag)
            }
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
            // INC SP
            0x33 => self.inc_sp(),
            // INC (HL)
            0x34 => self.inc_hl(bus),
            // DEC (HL)
            0x35 => self.dec_hl(bus),
            // LD (HL), d8
            0x36 => self.ld_hl_d8(bus),
            // SCF
            0x37 => {
                self.regs.flag_n().clear();
                self.regs.flag_h().clear();
                self.regs.flag_c().set();
                4
            }
            // JR C,r8
            0x38 => {
                // C
                let flag = self.regs.flag_c().is_set();
                self.jr_if_r8(bus, flag)
            }
            // ADD HL,SP
            0x39 => self.add_rr_rr(RegPair::HL, self.sp),
            // // INC SP
            // 0x33 => self.inc_rr(RegPair::SP),
            // LD A,(HL-)
            0x3a => {
                self.ld_r_addr(bus, Reg::A, RegPair::HL);
                *self.regs.hl = self.regs.hl.wrapping_sub(1);
                8
            }
            // DEC SP
            0x3b => {
                self.sp = self.sp.wrapping_sub(1);
                8
            }
            // INC A
            0x3c => self.inc_r(Reg::A),
            // DEC A
            0x3d => self.dec_r(Reg::A),
            // LD A,d8
            0x3e => self.ld_r_d8(bus, Reg::A),
            // CCF
            0x3f => {
                self.regs.flag_n().clear();
                self.regs.flag_h().clear();
                let c = self.regs.flag_c().is_set();
                self.regs.flag_c().set_value(!c);
                4
            }
            // LD B,A..L
            0x40 => self.ld_r_r(Reg::B, Reg::B),
            0x41 => self.ld_r_r(Reg::B, Reg::C),
            0x42 => self.ld_r_r(Reg::B, Reg::D),
            0x43 => self.ld_r_r(Reg::B, Reg::E),
            0x44 => self.ld_r_r(Reg::B, Reg::H),
            0x45 => self.ld_r_r(Reg::B, Reg::L),
            0x46 => self.ld_r_addr(bus, Reg::B, RegPair::HL),
            0x47 => self.ld_r_r(Reg::B, Reg::A),
            // LD C,A..L
            0x48 => self.ld_r_r(Reg::C, Reg::B),
            0x49 => self.ld_r_r(Reg::C, Reg::C),
            0x4a => self.ld_r_r(Reg::C, Reg::D),
            0x4b => self.ld_r_r(Reg::C, Reg::E),
            0x4c => self.ld_r_r(Reg::C, Reg::H),
            0x4d => self.ld_r_r(Reg::C, Reg::L),
            0x4e => self.ld_r_addr(bus, Reg::C, RegPair::HL),
            0x4f => self.ld_r_r(Reg::C, Reg::A),
            // LD D,A..L
            0x50 => self.ld_r_r(Reg::D, Reg::B),
            0x51 => self.ld_r_r(Reg::D, Reg::C),
            0x52 => self.ld_r_r(Reg::D, Reg::D),
            0x53 => self.ld_r_r(Reg::D, Reg::E),
            0x54 => self.ld_r_r(Reg::D, Reg::H),
            0x55 => self.ld_r_r(Reg::D, Reg::L),
            0x56 => self.ld_r_addr(bus, Reg::D, RegPair::HL),
            0x57 => self.ld_r_r(Reg::D, Reg::A),
            // LD E,A..L
            0x58 => self.ld_r_r(Reg::E, Reg::B),
            0x59 => self.ld_r_r(Reg::E, Reg::C),
            0x5a => self.ld_r_r(Reg::E, Reg::D),
            0x5b => self.ld_r_r(Reg::E, Reg::E),
            0x5c => self.ld_r_r(Reg::E, Reg::H),
            0x5d => self.ld_r_r(Reg::E, Reg::L),
            0x5e => self.ld_r_addr(bus, Reg::E, RegPair::HL),
            0x5f => self.ld_r_r(Reg::E, Reg::A),
            // LD H,A..L
            0x60 => self.ld_r_r(Reg::H, Reg::B),
            0x61 => self.ld_r_r(Reg::H, Reg::C),
            0x62 => self.ld_r_r(Reg::H, Reg::D),
            0x63 => self.ld_r_r(Reg::H, Reg::E),
            0x64 => self.ld_r_r(Reg::H, Reg::H),
            0x65 => self.ld_r_r(Reg::H, Reg::L),
            0x66 => self.ld_r_addr(bus, Reg::H, RegPair::HL),
            0x67 => self.ld_r_r(Reg::H, Reg::A),
            // LD L,A..L
            0x68 => self.ld_r_r(Reg::L, Reg::B),
            0x69 => self.ld_r_r(Reg::L, Reg::C),
            0x6a => self.ld_r_r(Reg::L, Reg::D),
            0x6b => self.ld_r_r(Reg::L, Reg::E),
            0x6c => self.ld_r_r(Reg::L, Reg::H),
            0x6d => self.ld_r_r(Reg::L, Reg::L),
            0x6e => self.ld_r_addr(bus, Reg::L, RegPair::HL),
            0x6f => self.ld_r_r(Reg::L, Reg::A),
            // LD (HL),B
            0x70 => self.ld_addr_r(bus, RegPair::HL, Reg::B),
            // LD (HL),C
            0x71 => self.ld_addr_r(bus, RegPair::HL, Reg::C),
            // LD (HL),D
            0x72 => self.ld_addr_r(bus, RegPair::HL, Reg::D),
            // LD (HL),E
            0x73 => self.ld_addr_r(bus, RegPair::HL, Reg::E),
            // LD (HL),H
            0x74 => self.ld_addr_r(bus, RegPair::HL, Reg::H),
            // LD (HL),L
            0x75 => self.ld_addr_r(bus, RegPair::HL, Reg::L),
            // HALT
            0x76 => {
                if self.ime {
                    // "Normal" case: pause the CPU until next interrupt
                    self.halted = true;
                } else if bus.interrupt_pending() {
                    // HALT bug: don't pause the CPU, but triggers the halt bug, which causes
                    // the next byte to be read twice
                    self.halt_bug = true;
                } else {
                    self.halted = true;
                }
                4
            }
            // LD (HL),A
            0x77 => self.ld_addr_r(bus, RegPair::HL, Reg::A),

            // LD A,A..L
            0x78 => self.ld_r_r(Reg::A, Reg::B),
            0x79 => self.ld_r_r(Reg::A, Reg::C),
            0x7a => self.ld_r_r(Reg::A, Reg::D),
            0x7b => self.ld_r_r(Reg::A, Reg::E),
            0x7c => self.ld_r_r(Reg::A, Reg::H),
            0x7d => self.ld_r_r(Reg::A, Reg::L),
            0x7e => self.ld_r_addr(bus, Reg::A, RegPair::HL),
            0x7f => self.ld_r_r(Reg::A, Reg::A),
            // ADD A,B
            0x80 => self.add_r(Reg::B),
            // ADD A,C
            0x81 => self.add_r(Reg::C),
            // ADD A,D
            0x82 => self.add_r(Reg::D),
            // ADD A,E
            0x83 => self.add_r(Reg::E),
            // ADD A,H
            0x84 => self.add_r(Reg::H),
            // ADD A,L
            0x85 => self.add_r(Reg::L),
            // ADD A,(HL)
            0x86 => self.add_hl_addr(bus),
            // ADD A,A
            0x87 => self.add_r(Reg::A),
            // ADC A,B
            0x88 => self.adc_r(Reg::B),
            // ADC A,C
            0x89 => self.adc_r(Reg::C),
            // ADC A,D
            0x8a => self.adc_r(Reg::D),
            // ADC A,E
            0x8b => self.adc_r(Reg::E),
            // ADC A,H
            0x8c => self.adc_r(Reg::H),
            // ADC A,L
            0x8d => self.adc_r(Reg::L),
            // ADC (HL)
            0x8e => self.adc_hl(bus),
            // ADC A,A
            0x8f => self.adc_r(Reg::A),
            // SUB B
            0x90 => self.sub_r(Reg::B),
            // SUB C
            0x91 => self.sub_r(Reg::C),
            // SUB D
            0x92 => self.sub_r(Reg::D),
            // SUB E
            0x93 => self.sub_r(Reg::E),
            // SUB H
            0x94 => self.sub_r(Reg::H),
            // SUB L
            0x95 => self.sub_r(Reg::L),
            // SUB (HL)
            0x96 => self.sub_hl_addr(bus),
            // SUB A
            0x97 => self.sub_r(Reg::A),
            // SBC B
            0x98 => self.sbc_r(Reg::B),
            // SBC C
            0x99 => self.sbc_r(Reg::C),
            // SBC D
            0x9a => self.sbc_r(Reg::D),
            // SBC E
            0x9b => self.sbc_r(Reg::E),
            // SBC H
            0x9c => self.sbc_r(Reg::H),
            // SBC L
            0x9d => self.sbc_r(Reg::L),
            // SBC (HL)
            0x9e => self.sbc_hl(bus),
            // SBC A
            0x9f => self.sbc_r(Reg::A),
            // AND B
            0xa0 => self.and_r(Reg::B),
            // AND C
            0xa1 => self.and_r(Reg::C),
            // AND D
            0xa2 => self.and_r(Reg::D),
            // AND E
            0xa3 => self.and_r(Reg::E),
            // AND H
            0xa4 => self.and_r(Reg::H),
            // AND L
            0xa5 => self.and_r(Reg::L),
            // AND (HL)
            0xa6 => self.and_hl(bus),
            // AND A
            0xa7 => self.and_r(Reg::A),
            // XOR B
            0xa8 => self.xor_r(Reg::B),
            // XOR C
            0xa9 => self.xor_r(Reg::C),
            // XOR D
            0xaa => self.xor_r(Reg::D),
            // XOR E
            0xab => self.xor_r(Reg::E),
            // XOR H
            0xac => self.xor_r(Reg::H),
            // XOR L
            0xad => self.xor_r(Reg::L),
            // XOR (HL)
            0xae => self.xor_hl(bus),
            // XOR A
            0xaf => self.xor_r(Reg::A),
            // OR B
            0xb0 => self.or_r(Reg::B),
            // OR C
            0xb1 => self.or_r(Reg::C),
            // OR D
            0xb2 => self.or_r(Reg::D),
            // OR E
            0xb3 => self.or_r(Reg::E),
            // OR H
            0xb4 => self.or_r(Reg::H),
            // OR L
            0xb5 => self.or_r(Reg::L),
            // OR (HL)
            0xb6 => self.or_hl(bus),
            // OR A
            0xb7 => self.or_r(Reg::A),
            // CP B
            0xb8 => self.cp_r(Reg::B),
            // CP C
            0xb9 => self.cp_r(Reg::C),
            // CP D
            0xba => self.cp_r(Reg::D),
            // CP E
            0xbb => self.cp_r(Reg::E),
            // CP H
            0xbc => self.cp_r(Reg::H),
            // CP L
            0xbd => self.cp_r(Reg::L),
            // CP (HL)
            0xbe => self.cp_hl(bus),
            // CP A
            0xbf => self.cp_r(Reg::A),
            // RET NZ
            0xc0 => {
                let nz = !self.regs.flag_z().is_set();
                self.ret_if(bus, nz)
            }
            // POP BC
            0xc1 => self.pop_rr(bus, RegPair::BC),
            // JP NZ,a16
            0xc2 => {
                let nz = !self.regs.flag_z().is_set();
                self.jp_if_a16(bus, nz)
            }
            // JP a16
            0xc3 => self.jp_if_a16(bus, true),
            // CALL NZ a16
            0xc4 => {
                let nz = !self.regs.flag_z().is_set();
                self.call_if_a16(bus, nz)
            }
            // PUSH BC
            0xc5 => self.push_rr(bus, RegPair::BC),
            // ADD d8
            0xc6 => self.add_d8(bus),
            // RST 0x00
            0xc7 => self.rst(bus, 0x00),
            // RET Z
            0xc8 => {
                let z = self.regs.flag_z().is_set();
                self.ret_if(bus, z)
            }
            // RET
            0xc9 => {
                self.ret_if(bus, true);
                // debug!("Returning from subroutine to 0x{:04x}", self.pc);
                16
            }
            // JP Z,a16
            0xca => {
                let z = self.regs.flag_z().is_set();
                self.jp_if_a16(bus, z)
            }
            // CB prefix
            0xcb => self.step_cb(bus),
            // CALL Z a16
            0xcc => {
                let z = self.regs.flag_z().is_set();
                self.call_if_a16(bus, z)
            }
            // CALL a16
            0xcd => self.call_if_a16(bus, true),
            // ADC A,d8
            0xce => self.adc_d8(bus),
            // RST 0x08
            0xcf => self.rst(bus, 0x08),
            // RET NC
            0xd0 => {
                let nc = !self.regs.flag_c().is_set();
                self.ret_if(bus, nc)
            }
            // POP DE
            0xd1 => self.pop_rr(bus, RegPair::DE),
            // JP NC,a16
            0xd2 => {
                let nc = !self.regs.flag_c().is_set();
                self.jp_if_a16(bus, nc)
            }
            // CALL NC a16
            0xd4 => {
                let nc = !self.regs.flag_c().is_set();
                self.call_if_a16(bus, nc)
            }
            // PUSH DE
            0xd5 => self.push_rr(bus, RegPair::DE),
            // SUB d8
            0xd6 => self.sub_d8(bus),
            // RST 0x10
            0xd7 => self.rst(bus, 0x10),
            // RET C
            0xd8 => {
                let c = self.regs.flag_c().is_set();
                self.ret_if(bus, c)
            }
            // RETI
            0xd9 => {
                self.ret_if(bus, true);
                // Re-enable interrupts
                self.ime = true;
                trace!("Returning from interrupt handler to 0x{:04x}", self.pc);
                16
            }
            // JP C,a16
            0xda => {
                let c = self.regs.flag_c().is_set();
                self.jp_if_a16(bus, c)
            }
            // CALL C a16
            0xdc => {
                let c = self.regs.flag_c().is_set();
                self.call_if_a16(bus, c)
            }
            // SBC A,d8
            0xde => self.sbc_d8(bus),
            // RST 0x18
            0xdf => self.rst(bus, 0x18),
            // LDH (a8),A
            0xe0 => {
                let a8 = self.fetch(bus);
                let addr = 0xFF00 + a8 as u16;
                bus.write_byte(addr, self.regs.get(Reg::A));
                12
            }
            // POP HL
            0xe1 => self.pop_rr(bus, RegPair::HL),
            // LD (C),A
            0xe2 => {
                let addr = 0xFF00 + self.regs.get(Reg::C) as u16;
                bus.write_byte(addr, self.regs.get(Reg::A));
                8
            }
            // PUSH HL
            0xe5 => self.push_rr(bus, RegPair::HL),
            // AND d8
            0xe6 => self.and_d8(bus),
            // RST 0x20
            0xe7 => self.rst(bus, 0x20),
            // ADD SP,r8
            0xe8 => self.add_sp_r8(bus),
            // JP HL
            0xe9 => self.jp_hl(),
            // LD (a16),A
            0xea => self.ld_a16_r(bus, Reg::A),
            // XOR d8
            0xee => self.xor_d8(bus),
            // RST 0x28
            0xef => self.rst(bus, 0x28),
            // LDH A,(a8)
            0xf0 => {
                let a8 = self.fetch(bus);
                let addr = 0xFF00 + a8 as u16;
                self.regs.set(Reg::A, bus.read_byte(addr));
                12
            }
            // POP AF
            0xf1 => self.pop_rr(bus, RegPair::AF),
            // LD A,(C)
            0xf2 => {
                let addr = 0xFF00 + self.regs.get(Reg::C) as u16;
                self.regs.set(Reg::A, bus.read_byte(addr));
                8
            }
            // DI
            0xf3 => {
                trace!("Disabling interrupts");
                self.ime = false;
                4
            }
            // PUSH AF
            0xf5 => self.push_rr(bus, RegPair::AF),
            // OR d8
            0xf6 => self.or_d8(bus),
            // RST 0x30
            0xf7 => self.rst(bus, 0x30),
            // LD HL,SP+r8
            0xf8 => self.ld_hl_sp_r8(bus),
            // LD SP,HL
            0xf9 => {
                self.sp = *self.regs.hl;
                8
            }
            // LD A,(a16)
            0xfa => self.ld_r_a16(bus, Reg::A),
            // EI
            0xfb => {
                trace!("Enabling interrupts");
                // TODO the effect needs to be delayed by one instruction...
                self.ime = true;
                4
            }
            // CP d8
            0xfe => self.cp_d8(bus),
            // RST 0x38
            0xff => self.rst(bus, 0x38),

            _ => {
                // self.dump_cpu();
                // unimplemented!("op=0x{:02x}, orig_pc=0x{:04x}", op, orig_pc);
                warn!("Unimplemented op=0x{:02x}, orig_pc=0x{:04x}", op, orig_pc);
                0
            }
        };
        cycles
    }

    /// CB-prefixed instruction
    fn step_cb(&mut self, bus: &mut Bus) -> u8 {
        let cb_op = self.fetch(bus);
        match cb_op {
            // RLC B
            0x00 => self.rlc_r(Reg::B),
            // RLC C
            0x01 => self.rlc_r(Reg::C),
            // RLC D
            0x02 => self.rlc_r(Reg::D),
            // RLC E
            0x03 => self.rlc_r(Reg::E),
            // RLC H
            0x04 => self.rlc_r(Reg::H),
            // RLC L
            0x05 => self.rlc_r(Reg::L),
            // RLC (HL)
            0x06 => self.rlc_hl(bus),
            // RLC A
            0x07 => self.rlc_r(Reg::A),
            // RRC B
            0x08 => self.rrc_r(Reg::B),
            // RRC C
            0x09 => self.rrc_r(Reg::C),
            // RRC D
            0x0a => self.rrc_r(Reg::D),
            // RRC E
            0x0b => self.rrc_r(Reg::E),
            // RRC H
            0x0c => self.rrc_r(Reg::H),
            // RRC L
            0x0d => self.rrc_r(Reg::L),
            // RRC (HL)
            0x0e => self.rrc_hl(bus),
            // RRC A
            0x0f => self.rrc_r(Reg::A),
            // RL B
            0x10 => self.rl_r(Reg::B),
            // RL C
            0x11 => self.rl_r(Reg::C),
            // RL D
            0x12 => self.rl_r(Reg::D),
            // RL E
            0x13 => self.rl_r(Reg::E),
            // RL H
            0x14 => self.rl_r(Reg::H),
            // RL L
            0x15 => self.rl_r(Reg::L),
            // RL (HL)
            0x16 => self.rl_hl(bus),
            // RL A
            0x17 => self.rl_r(Reg::A),
            // RR B
            0x18 => self.rr_r(Reg::B),
            // RR C
            0x19 => self.rr_r(Reg::C),
            // RR D
            0x1a => self.rr_r(Reg::D),
            // RR E
            0x1b => self.rr_r(Reg::E),
            // RR H
            0x1c => self.rr_r(Reg::H),
            // RR L
            0x1d => self.rr_r(Reg::L),
            // RR (HL)
            0x1e => self.rr_hl(bus),
            // RR A
            0x1f => self.rr_r(Reg::A),
            // SLA B
            0x20 => self.sla_r(Reg::B),
            // SLA C
            0x21 => self.sla_r(Reg::C),
            // SLA D
            0x22 => self.sla_r(Reg::D),
            // SLA E
            0x23 => self.sla_r(Reg::E),
            // SLA H
            0x24 => self.sla_r(Reg::H),
            // SLA L
            0x25 => self.sla_r(Reg::L),
            // SLA (HL)
            0x26 => self.sla_hl(bus),
            // SLA A
            0x27 => self.sla_r(Reg::A),
            // SRA B
            0x28 => self.sra_r(Reg::B),
            // SRA C
            0x29 => self.sra_r(Reg::C),
            // SRA D
            0x2a => self.sra_r(Reg::D),
            // SRA E
            0x2b => self.sra_r(Reg::E),
            // SRA H
            0x2c => self.sra_r(Reg::H),
            // SRA L
            0x2d => self.sra_r(Reg::L),
            // SRA (HL)
            0x2e => self.sra_hl(bus),
            // SRA A
            0x2f => self.sra_r(Reg::A),
            // SWAP B
            0x30 => self.swap_r(Reg::B),
            // SWAP C
            0x31 => self.swap_r(Reg::C),
            // SWAP D
            0x32 => self.swap_r(Reg::D),
            // SWAP E
            0x33 => self.swap_r(Reg::E),
            // SWAP H
            0x34 => self.swap_r(Reg::H),
            // SWAP L
            0x35 => self.swap_r(Reg::L),
            // SWAP (HL)
            0x36 => self.swap_hl(bus),
            // SWAP A
            0x37 => self.swap_r(Reg::A),
            // SRL B
            0x38 => self.srl_r(Reg::B),
            // SRL C
            0x39 => self.srl_r(Reg::C),
            // SRL D
            0x3a => self.srl_r(Reg::D),
            // SRL E
            0x3b => self.srl_r(Reg::E),
            // SRL H
            0x3c => self.srl_r(Reg::H),
            // SRL L
            0x3d => self.srl_r(Reg::L),
            // SRL (HL)
            0x3e => self.srl_hl(bus),
            // SRL A
            0x3f => self.srl_r(Reg::A),
            // BIT 0,r
            0x40 => self.bit_n_r(0, Reg::B),
            0x41 => self.bit_n_r(0, Reg::C),
            0x42 => self.bit_n_r(0, Reg::D),
            0x43 => self.bit_n_r(0, Reg::E),
            0x44 => self.bit_n_r(0, Reg::H),
            0x45 => self.bit_n_r(0, Reg::L),
            0x46 => self.bit_n_hl(0, bus),
            0x47 => self.bit_n_r(0, Reg::A),
            // BIT 1,r
            0x48 => self.bit_n_r(1, Reg::B),
            0x49 => self.bit_n_r(1, Reg::C),
            0x4a => self.bit_n_r(1, Reg::D),
            0x4b => self.bit_n_r(1, Reg::E),
            0x4c => self.bit_n_r(1, Reg::H),
            0x4d => self.bit_n_r(1, Reg::L),
            0x4e => self.bit_n_hl(1, bus),
            0x4f => self.bit_n_r(1, Reg::A),
            // BIT 2,r
            0x50 => self.bit_n_r(2, Reg::B),
            0x51 => self.bit_n_r(2, Reg::C),
            0x52 => self.bit_n_r(2, Reg::D),
            0x53 => self.bit_n_r(2, Reg::E),
            0x54 => self.bit_n_r(2, Reg::H),
            0x55 => self.bit_n_r(2, Reg::L),
            0x56 => self.bit_n_hl(2, bus),
            0x57 => self.bit_n_r(2, Reg::A),
            // BIT 3,r
            0x58 => self.bit_n_r(3, Reg::B),
            0x59 => self.bit_n_r(3, Reg::C),
            0x5a => self.bit_n_r(3, Reg::D),
            0x5b => self.bit_n_r(3, Reg::E),
            0x5c => self.bit_n_r(3, Reg::H),
            0x5d => self.bit_n_r(3, Reg::L),
            0x5e => self.bit_n_hl(3, bus),
            0x5f => self.bit_n_r(3, Reg::A),
            // BIT 4,r
            0x60 => self.bit_n_r(4, Reg::B),
            0x61 => self.bit_n_r(4, Reg::C),
            0x62 => self.bit_n_r(4, Reg::D),
            0x63 => self.bit_n_r(4, Reg::E),
            0x64 => self.bit_n_r(4, Reg::H),
            0x65 => self.bit_n_r(4, Reg::L),
            0x66 => self.bit_n_hl(4, bus),
            0x67 => self.bit_n_r(4, Reg::A),
            // BIT 5,r
            0x68 => self.bit_n_r(5, Reg::B),
            0x69 => self.bit_n_r(5, Reg::C),
            0x6a => self.bit_n_r(5, Reg::D),
            0x6b => self.bit_n_r(5, Reg::E),
            0x6c => self.bit_n_r(5, Reg::H),
            0x6d => self.bit_n_r(5, Reg::L),
            0x6e => self.bit_n_hl(5, bus),
            0x6f => self.bit_n_r(5, Reg::A),
            // BIT 6,r
            0x70 => self.bit_n_r(6, Reg::B),
            0x71 => self.bit_n_r(6, Reg::C),
            0x72 => self.bit_n_r(6, Reg::D),
            0x73 => self.bit_n_r(6, Reg::E),
            0x74 => self.bit_n_r(6, Reg::H),
            0x75 => self.bit_n_r(6, Reg::L),
            0x76 => self.bit_n_hl(6, bus),
            0x77 => self.bit_n_r(6, Reg::A),
            // BIT 7,r
            0x78 => self.bit_n_r(7, Reg::B),
            0x79 => self.bit_n_r(7, Reg::C),
            0x7a => self.bit_n_r(7, Reg::D),
            0x7b => self.bit_n_r(7, Reg::E),
            0x7c => self.bit_n_r(7, Reg::H),
            0x7d => self.bit_n_r(7, Reg::L),
            0x7e => self.bit_n_hl(7, bus),
            0x7f => self.bit_n_r(7, Reg::A),
            // RES 0,r
            0x80 => self.res_n_r(0, Reg::B),
            0x81 => self.res_n_r(0, Reg::C),
            0x82 => self.res_n_r(0, Reg::D),
            0x83 => self.res_n_r(0, Reg::E),
            0x84 => self.res_n_r(0, Reg::H),
            0x85 => self.res_n_r(0, Reg::L),
            0x86 => self.res_hl(0, bus),
            0x87 => self.res_n_r(0, Reg::A),
            // RES 1,r
            0x88 => self.res_n_r(1, Reg::B),
            0x89 => self.res_n_r(1, Reg::C),
            0x8a => self.res_n_r(1, Reg::D),
            0x8b => self.res_n_r(1, Reg::E),
            0x8c => self.res_n_r(1, Reg::H),
            0x8d => self.res_n_r(1, Reg::L),
            0x8e => self.res_hl(1, bus),
            0x8f => self.res_n_r(1, Reg::A),
            // RES 2,r
            0x90 => self.res_n_r(2, Reg::B),
            0x91 => self.res_n_r(2, Reg::C),
            0x92 => self.res_n_r(2, Reg::D),
            0x93 => self.res_n_r(2, Reg::E),
            0x94 => self.res_n_r(2, Reg::H),
            0x95 => self.res_n_r(2, Reg::L),
            0x96 => self.res_hl(2, bus),
            0x97 => self.res_n_r(2, Reg::A),
            // RES 3,r
            0x98 => self.res_n_r(3, Reg::B),
            0x99 => self.res_n_r(3, Reg::C),
            0x9a => self.res_n_r(3, Reg::D),
            0x9b => self.res_n_r(3, Reg::E),
            0x9c => self.res_n_r(3, Reg::H),
            0x9d => self.res_n_r(3, Reg::L),
            0x9e => self.res_hl(3, bus),
            0x9f => self.res_n_r(3, Reg::A),
            // RES 4,r
            0xa0 => self.res_n_r(4, Reg::B),
            0xa1 => self.res_n_r(4, Reg::C),
            0xa2 => self.res_n_r(4, Reg::D),
            0xa3 => self.res_n_r(4, Reg::E),
            0xa4 => self.res_n_r(4, Reg::H),
            0xa5 => self.res_n_r(4, Reg::L),
            0xa6 => self.res_hl(4, bus),
            0xa7 => self.res_n_r(4, Reg::A),
            // RES 5,r
            0xa8 => self.res_n_r(5, Reg::B),
            0xa9 => self.res_n_r(5, Reg::C),
            0xaa => self.res_n_r(5, Reg::D),
            0xab => self.res_n_r(5, Reg::E),
            0xac => self.res_n_r(5, Reg::H),
            0xad => self.res_n_r(5, Reg::L),
            0xae => self.res_hl(5, bus),
            0xaf => self.res_n_r(5, Reg::A),
            // RES 6,r
            0xb0 => self.res_n_r(6, Reg::B),
            0xb1 => self.res_n_r(6, Reg::C),
            0xb2 => self.res_n_r(6, Reg::D),
            0xb3 => self.res_n_r(6, Reg::E),
            0xb4 => self.res_n_r(6, Reg::H),
            0xb5 => self.res_n_r(6, Reg::L),
            0xb6 => self.res_hl(6, bus),
            0xb7 => self.res_n_r(6, Reg::A),
            // RES 7,r
            0xb8 => self.res_n_r(7, Reg::B),
            0xb9 => self.res_n_r(7, Reg::C),
            0xba => self.res_n_r(7, Reg::D),
            0xbb => self.res_n_r(7, Reg::E),
            0xbc => self.res_n_r(7, Reg::H),
            0xbd => self.res_n_r(7, Reg::L),
            0xbe => self.res_hl(7, bus),
            0xbf => self.res_n_r(7, Reg::A),

            // SET 0,r
            0xc0 => self.set_n_r(0, Reg::B),
            0xc1 => self.set_n_r(0, Reg::C),
            0xc2 => self.set_n_r(0, Reg::D),
            0xc3 => self.set_n_r(0, Reg::E),
            0xc4 => self.set_n_r(0, Reg::H),
            0xc5 => self.set_n_r(0, Reg::L),
            0xc6 => self.set_hl(0, bus),
            0xc7 => self.set_n_r(0, Reg::A),
            // SET 1,r
            0xc8 => self.set_n_r(1, Reg::B),
            0xc9 => self.set_n_r(1, Reg::C),
            0xca => self.set_n_r(1, Reg::D),
            0xcb => self.set_n_r(1, Reg::E),
            0xcc => self.set_n_r(1, Reg::H),
            0xcd => self.set_n_r(1, Reg::L),
            0xce => self.set_hl(1, bus),
            0xcf => self.set_n_r(1, Reg::A),
            // SET 2,r
            0xd0 => self.set_n_r(2, Reg::B),
            0xd1 => self.set_n_r(2, Reg::C),
            0xd2 => self.set_n_r(2, Reg::D),
            0xd3 => self.set_n_r(2, Reg::E),
            0xd4 => self.set_n_r(2, Reg::H),
            0xd5 => self.set_n_r(2, Reg::L),
            0xd6 => self.set_hl(2, bus),
            0xd7 => self.set_n_r(2, Reg::A),
            // SET 3,r
            0xd8 => self.set_n_r(3, Reg::B),
            0xd9 => self.set_n_r(3, Reg::C),
            0xda => self.set_n_r(3, Reg::D),
            0xdb => self.set_n_r(3, Reg::E),
            0xdc => self.set_n_r(3, Reg::H),
            0xdd => self.set_n_r(3, Reg::L),
            0xde => self.set_hl(3, bus),
            0xdf => self.set_n_r(3, Reg::A),
            // SET 4,r
            0xe0 => self.set_n_r(4, Reg::B),
            0xe1 => self.set_n_r(4, Reg::C),
            0xe2 => self.set_n_r(4, Reg::D),
            0xe3 => self.set_n_r(4, Reg::E),
            0xe4 => self.set_n_r(4, Reg::H),
            0xe5 => self.set_n_r(4, Reg::L),
            0xe6 => self.set_hl(4, bus),
            0xe7 => self.set_n_r(4, Reg::A),
            // SET 5,r
            0xe8 => self.set_n_r(5, Reg::B),
            0xe9 => self.set_n_r(5, Reg::C),
            0xea => self.set_n_r(5, Reg::D),
            0xeb => self.set_n_r(5, Reg::E),
            0xec => self.set_n_r(5, Reg::H),
            0xed => self.set_n_r(5, Reg::L),
            0xee => self.set_hl(5, bus),
            0xef => self.set_n_r(5, Reg::A),
            // SET 6,r
            0xf0 => self.set_n_r(6, Reg::B),
            0xf1 => self.set_n_r(6, Reg::C),
            0xf2 => self.set_n_r(6, Reg::D),
            0xf3 => self.set_n_r(6, Reg::E),
            0xf4 => self.set_n_r(6, Reg::H),
            0xf5 => self.set_n_r(6, Reg::L),
            0xf6 => self.set_hl(6, bus),
            0xf7 => self.set_n_r(6, Reg::A),
            // SET 7,r
            0xf8 => self.set_n_r(7, Reg::B),
            0xf9 => self.set_n_r(7, Reg::C),
            0xfa => self.set_n_r(7, Reg::D),
            0xfb => self.set_n_r(7, Reg::E),
            0xfc => self.set_n_r(7, Reg::H),
            0xfd => self.set_n_r(7, Reg::L),
            0xfe => self.set_hl(7, bus),
            0xff => self.set_n_r(7, Reg::A),
        }
    }

    // TODO probably should implement Debug instead...
    pub fn dump_cpu(&self) {
        println!(
            "PC=${:04X}, SP=${:04X}, regs={:?}, IME={}",
            self.pc, self.sp, self.regs, self.ime
        );
    }

    fn fetch(&mut self, bus: &mut Bus) -> u8 {
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

    /// LD (HL),d8
    fn ld_hl_d8(&mut self, bus: &mut Bus) -> u8 {
        let d8 = self.fetch(bus);
        bus.write_byte(*self.regs.hl, d8);
        12
    }

    /// LD r,r
    fn ld_r_r(&mut self, rt: Reg, rs: Reg) -> u8 {
        if self.enable_soft_break && rt == Reg::B && rs == Reg::B {
            info!("Software breakpoint triggered");
            self.paused = true;
        }
        self.regs.set(rt, self.regs.get(rs));
        4
    }

    /// LD rr,d16
    fn ld_rr_d16(&mut self, bus: &mut Bus, reg: RegPair) -> u8 {
        let d16 = self.fetch_word(bus);
        self.regs.set_pair(reg, d16);
        12
    }

    fn ld_r_addr(&mut self, bus: &mut Bus, r: Reg, rr: RegPair) -> u8 {
        let addr = self.regs.get_pair(rr);
        self.regs.set(r, bus.read_byte(addr));
        8
    }

    fn ld_addr_r(&mut self, bus: &mut Bus, rr: RegPair, r: Reg) -> u8 {
        let addr = self.regs.get_pair(rr);
        bus.write_byte(addr, self.regs.get(r));
        8
    }

    fn ld_a16_r(&mut self, bus: &mut Bus, r: Reg) -> u8 {
        let addr = self.fetch_word(bus);
        bus.write_byte(addr, self.regs.get(r));
        16
    }

    fn ld_r_a16(&mut self, bus: &mut Bus, r: Reg) -> u8 {
        let addr = self.fetch_word(bus);
        let byte = bus.read_byte(addr);
        self.regs.set(r, byte);
        16
    }

    /// LD HL,SP+r8
    ///
    /// This is basically the same as `ADD SP,r8` except that the result is stored in `HL` and `SP`
    /// is not modified.
    fn ld_hl_sp_r8(&mut self, bus: &mut Bus) -> u8 {
        // save SP
        let sp = self.sp;
        self.add_sp_r8(bus);
        self.regs.set_pair(RegPair::HL, self.sp);
        // restore SP
        self.sp = sp;

        12
    }

    fn add_sp_r8(&mut self, bus: &mut Bus) -> u8 {
        // sign extend r8 to 16 bits
        let r8 = self.fetch(bus) as i8 as i16 as u16;
        let sp = self.sp;
        let sum = sp.wrapping_add(r8);
        self.sp = sum;
        self.regs.flag_z().clear();
        self.regs.flag_n().clear();
        self.regs
            .flag_h()
            .set_value((sp & 0x000f) + (r8 & 0x000f) > 0x000f);
        self.regs
            .flag_c()
            .set_value((sp & 0x00ff) + (r8 & 0x00ff) > 0x00ff);

        16
    }

    fn xor_r(&mut self, reg: Reg) -> u8 {
        let r = self.regs.get(reg);
        self.xor(r)
    }

    fn xor_d8(&mut self, bus: &mut Bus) -> u8 {
        let v = self.fetch(bus);
        self.xor(v);
        8
    }

    fn xor_hl(&mut self, bus: &mut Bus) -> u8 {
        let v = bus.read_byte(*self.regs.hl);
        self.xor(v);
        8
    }

    /// A <- A ^ v
    fn xor(&mut self, v: u8) -> u8 {
        let new_a = self.regs.get(Reg::A) ^ v;
        self.regs.set(Reg::A, new_a);
        self.regs.flag_z().set_value(new_a == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        self.regs.flag_c().clear();
        4
    }

    /// AND r
    fn and_r(&mut self, r: Reg) -> u8 {
        self.and(self.regs.get(r))
    }

    /// AND d8
    fn and_d8(&mut self, bus: &mut Bus) -> u8 {
        let d8 = self.fetch(bus);
        self.and(d8);
        8
    }

    // AND (HL)
    fn and_hl(&mut self, bus: &mut Bus) -> u8 {
        let hl = bus.read_byte(*self.regs.hl);
        self.and(hl);
        8
    }

    /// AND <value>
    fn and(&mut self, v: u8) -> u8 {
        let res = self.regs.get(Reg::A) & v;
        self.regs.flag_z().set_value(res == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().set();
        self.regs.flag_c().clear();
        self.regs.set(Reg::A, res);
        4
    }

    /// OR r
    fn or_r(&mut self, r: Reg) -> u8 {
        self.or(self.regs.get(r))
    }

    /// OR (HL)
    fn or_hl(&mut self, bus: &mut Bus) -> u8 {
        let v = bus.read_byte(*self.regs.hl);
        self.or(v);
        8
    }

    fn or_d8(&mut self, bus: &mut Bus) -> u8 {
        let v = self.fetch(bus);
        self.or(v);
        8
    }

    fn or(&mut self, v: u8) -> u8 {
        let res = self.regs.get(Reg::A) | v;
        self.regs.flag_z().set_value(res == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        self.regs.flag_c().clear();
        self.regs.set(Reg::A, res);
        4
    }

    // SRL r (Shift Right Logically)
    fn srl_r(&mut self, reg: Reg) -> u8 {
        // 0 -> [7 -> 0] -> C
        let r = self.regs.get(reg);
        let shifted = self.srl_value_and_set_flags(r);
        self.regs.set(reg, shifted);

        8
    }

    fn srl_hl(&mut self, bus: &mut Bus) -> u8 {
        let hl = bus.read_byte(*self.regs.hl);
        bus.write_byte(*self.regs.hl, self.srl_value_and_set_flags(hl));
        16
    }

    fn srl_value_and_set_flags(&mut self, mut value: u8) -> u8 {
        let c = (value & 0x01) != 0;
        value >>= 1;
        self.regs.flag_z().set_value(value == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        self.regs.flag_c().set_value(c);
        value
    }

    // SRA r (Shift Right Arithmetically)
    fn sra_hl(&mut self, bus: &mut Bus) -> u8 {
        let r = bus.read_byte(*self.regs.hl);
        let new_r = self.sra(r);
        bus.write_byte(*self.regs.hl, new_r);
        16
    }

    fn sra_r(&mut self, reg: Reg) -> u8 {
        let r = self.regs.get(reg);
        let new_r = self.sra(r);
        self.regs.set(reg, new_r);
        8
    }
    fn sra(&mut self, mut r: u8) -> u8 {
        // [7] -> [7 -> 0] -> C
        let c = (r & 0x01) != 0;
        r = ((r as i8) >> 1) as u8;
        self.regs.flag_z().set_value(r == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        self.regs.flag_c().set_value(c);
        r
    }

    // SLA r (Shift Left Arithmetically)
    fn sla_hl(&mut self, bus: &mut Bus) -> u8 {
        let r = bus.read_byte(*self.regs.hl);
        let new_r = self.sla(r);
        bus.write_byte(*self.regs.hl, new_r);
        16
    }

    fn sla_r(&mut self, reg: Reg) -> u8 {
        let r = self.regs.get(reg);
        let new_r = self.sla(r);
        self.regs.set(reg, new_r);
        8
    }

    fn sla(&mut self, mut r: u8) -> u8 {
        // C <- [7 <- 0] <- 0
        let c = (r & 0x80) != 0;
        r <<= 1;
        self.regs.flag_z().set_value(r == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        self.regs.flag_c().set_value(c);
        r
    }

    /// DEC (HL)
    fn dec_hl(&mut self, bus: &mut Bus) -> u8 {
        let new_r = self.dec_value_and_set_flags(bus.read_byte(*self.regs.hl));
        bus.write_byte(*self.regs.hl, new_r);

        12
    }

    /// DEC r
    fn dec_r(&mut self, reg: Reg) -> u8 {
        let new_r = self.dec_value_and_set_flags(self.regs.get(reg));
        self.regs.set(reg, new_r);
        4
    }

    fn dec_value_and_set_flags(&mut self, v: u8) -> u8 {
        let new_v = v.wrapping_sub(1);
        self.regs.flag_z().set_value(new_v == 0);
        self.regs.flag_n().set();
        self.regs
            .flag_h()
            .set_value(((v & 0xf).wrapping_sub(1)) & 0x10 == 0x10);

        new_v
    }

    /// INC (HL)
    fn inc_hl(&mut self, bus: &mut Bus) -> u8 {
        let new_r = self.inc_value_and_set_flags(bus.read_byte(*self.regs.hl));
        bus.write_byte(*self.regs.hl, new_r);
        12
    }

    /// INC r
    fn inc_r(&mut self, reg: Reg) -> u8 {
        let new_r = self.inc_value_and_set_flags(self.regs.get(reg));
        self.regs.set(reg, new_r);

        4
    }

    fn inc_value_and_set_flags(&mut self, v: u8) -> u8 {
        let new_r = v.wrapping_add(1);
        self.regs.flag_z().set_value(new_r == 0);
        self.regs.flag_n().clear();
        self.regs
            .flag_h()
            .set_value(((v & 0x0f) + 1) & 0x10 == 0x10);

        new_r
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

    /// INC rr
    fn inc_sp(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        8
    }

    /// Test bit n of register r
    fn bit_n_r(&mut self, n: u8, reg: Reg) -> u8 {
        self.bit_n_value(n, self.regs.get(reg))
    }

    /// Test bit n of register r
    fn bit_n_hl(&mut self, n: u8, bus: &mut Bus) -> u8 {
        self.bit_n_value(n, bus.read_byte(*self.regs.hl));
        16
    }

    fn bit_n_value(&mut self, n: u8, value: u8) -> u8 {
        if value & (1 << n) == 0 {
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

    /// conditional absolute jump
    fn jp_if_a16(&mut self, bus: &mut Bus, flag: bool) -> u8 {
        let a16 = self.fetch_word(bus);
        if flag {
            self.pc = a16;
            16
        } else {
            12
        }
    }

    fn jp_hl(&mut self) -> u8 {
        self.pc = *self.regs.hl;
        4
    }

    /// conditional CALL
    fn call_if_a16(&mut self, bus: &mut Bus, flag: bool) -> u8 {
        let addr = self.fetch_word(bus);
        if flag {
            self.call(bus, addr);
            24
        } else {
            12
        }
    }

    fn call_interrupt(&mut self, bus: &mut Bus, itr_flag: InterruptFlag) {
        let addr = self.get_itr_vector(itr_flag);
        trace!("Calling ITR 0x{:02X}", addr);
        // disable interrupts
        self.ime = false;
        bus.ack_interrupt(itr_flag);
        self.push_word(bus, self.pc);
        self.pc = addr;
    }

    fn call(&mut self, bus: &mut Bus, addr: u16) {
        trace!("Calling subroutine at 0x{:04x}", addr);
        self.push_word(bus, self.pc);
        self.pc = addr;
    }

    fn ret_if(&mut self, bus: &mut Bus, flag: bool) -> u8 {
        if flag {
            self.pc = self.pop_word(bus);
            20
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
        let r = self.regs.get(reg);
        let new_r = self.rl(r);
        self.regs.set(reg, new_r);
        8
    }

    /// RL r ;rotate left through carry
    fn rl_hl(&mut self, bus: &mut Bus) -> u8 {
        let v = bus.read_byte(*self.regs.hl);
        let res = self.rl(v);
        bus.write_byte(*self.regs.hl, res);
        16
    }

    fn rl(&mut self, v: u8) -> u8 {
        // C <- [7 <- 0] <- C
        let c = self.regs.flag_c().is_set() as u8;
        // What used to be the 7th bit (and is now the 0th bit) should be the new carry flag
        let new_c = (v & 0x80) != 0;
        let mut res = v.rotate_left(1);
        // What was the carry should now be the 0th bit
        if c != 0 {
            // set the bit
            res |= 0x01;
        } else {
            // clear the bit
            res &= 0xFE;
        }
        self.regs.flag_z().set_value(res == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        self.regs.flag_c().set_value(new_c);

        res
    }

    /// RR r ;rotate right through carry
    fn rr_r(&mut self, reg: Reg) -> u8 {
        let r = self.regs.get(reg);
        let new_r = self.rr(r);
        self.regs.set(reg, new_r);

        8
    }

    fn rr_hl(&mut self, bus: &mut Bus) -> u8 {
        let r = bus.read_byte(*self.regs.hl);
        let new_r = self.rr(r);
        bus.write_byte(*self.regs.hl, new_r);

        16
    }

    fn rr(&mut self, mut r: u8) -> u8 {
        // C -> [7 -> 0] -> C
        let c = self.regs.flag_c().is_set();

        // Save the 0th bit which should be the new carry flag
        let new_c = r & 0x01;
        // r = r.rotate_right(1);
        r >>= 1;
        // What was the carry should now be the 7th bit
        if c {
            // set the bit
            r |= 0x80;
        } else {
            // clear the bit
            r &= 0x7F;
        }
        self.regs.flag_z().set_value(r == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        self.regs.flag_c().set_value(new_c != 0);

        r
    }

    /// RLA
    fn rla(&mut self) -> u8 {
        // Same as RL A but Z is always cleared
        self.rl_r(Reg::A);
        self.regs.flag_z().clear();
        4
    }

    /// RRA
    fn rra(&mut self) -> u8 {
        // Same as RR A but Z is always cleared
        self.rr_r(Reg::A);
        self.regs.flag_z().clear();
        4
    }

    /// RLC r -- Rotate Left
    fn rlc_r(&mut self, reg: Reg) -> u8 {
        let r = self.regs.get(reg);
        let rotated = self.rlc(r);
        self.regs.set(reg, rotated);
        8
    }

    fn rlc_hl(&mut self, bus: &mut Bus) -> u8 {
        let r = bus.read_byte(*self.regs.hl);
        let rotated = self.rlc(r);
        bus.write_byte(*self.regs.hl, rotated);
        16
    }

    fn rlc(&mut self, r: u8) -> u8 {
        let c = r & 0x80 != 0;
        let rotated = r.rotate_left(1);
        self.regs.flag_z().set_value(rotated == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        self.regs.flag_c().set_value(c);

        rotated
    }

    /// RLCA -- Rotate register A left (same as RLC A but z is always cleared)
    fn rlca(&mut self) -> u8 {
        self.rlc_r(Reg::A);
        self.regs.flag_z().clear();
        4
    }

    /// RRC r -- Rotate Right
    fn rrc_r(&mut self, reg: Reg) -> u8 {
        let r = self.regs.get(reg);
        let rotated = self.rrc(r);
        self.regs.set(reg, rotated);
        8
    }

    fn rrc_hl(&mut self, bus: &mut Bus) -> u8 {
        let r = bus.read_byte(*self.regs.hl);
        let rotated = self.rrc(r);
        bus.write_byte(*self.regs.hl, rotated);
        16
    }

    fn rrc(&mut self, r: u8) -> u8 {
        let c = r & 0x01 != 0;
        let rotated = r.rotate_right(1);
        self.regs.flag_z().set_value(rotated == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        self.regs.flag_c().set_value(c);

        rotated
    }

    /// RRCA -- Rotate register A Right (same as RRC A but z is always cleared)
    fn rrca(&mut self) -> u8 {
        self.rrc_r(Reg::A);
        self.regs.flag_z().clear();
        4
    }

    /// ADD (HL)
    fn add_hl_addr(&mut self, bus: &mut Bus) -> u8 {
        let hl = bus.read_byte(*self.regs.hl);
        self.add(hl);
        8
    }

    /// ADD r
    fn add_r(&mut self, reg: Reg) -> u8 {
        self.add(self.regs.get(reg))
    }

    /// ADD d8
    fn add_d8(&mut self, bus: &mut Bus) -> u8 {
        let d8 = self.fetch(bus);
        self.add(d8);
        4
    }

    fn add(&mut self, value: u8) -> u8 {
        self.adc(value, false)
    }

    /// ADD rr,rr
    fn add_rr_rr(&mut self, rt: RegPair, v: u16) -> u8 {
        let reg = self.regs.get_pair(rt);
        let (sum, carry) = reg.overflowing_add(v);
        self.regs.set_pair(rt, sum);
        self.regs.flag_c().set_value(carry);
        self.regs.flag_n().clear();
        self.regs
            .flag_h()
            .set_value((reg & 0x0fff) + (v & 0x0fff) > 0x0fff);
        8
    }

    /// ADD (HL)
    fn adc_hl(&mut self, bus: &mut Bus) -> u8 {
        let hl = bus.read_byte(*self.regs.hl);
        self.adc(hl, true);
        8
    }

    /// ADD r
    fn adc_r(&mut self, reg: Reg) -> u8 {
        self.adc(self.regs.get(reg), true)
    }

    /// ADD d8
    fn adc_d8(&mut self, bus: &mut Bus) -> u8 {
        let d8 = self.fetch(bus);
        self.adc(d8, true);
        4
    }

    fn adc(&mut self, value: u8, with_carry: bool) -> u8 {
        let c = (with_carry && self.regs.flag_c().is_set()) as u8;

        let reg_a = self.regs.get(Reg::A);
        let (sum1, carry1) = reg_a.overflowing_add(value);
        let (sum2, carry2) = sum1.overflowing_add(c);
        self.regs.set(Reg::A, sum2);
        self.regs.flag_z().set_value(sum2 == 0);
        self.regs.flag_n().clear();
        self.regs.flag_c().set_value(carry1 | carry2);
        self.regs
            .flag_h()
            .set_value((reg_a & 0x0f) + (value & 0x0f) + c > 0x0f);
        8
    }

    /// SBC (HL)
    fn sbc_hl(&mut self, bus: &mut Bus) -> u8 {
        let hl = bus.read_byte(*self.regs.hl);
        self.sbc(hl, true);
        8
    }

    /// SBC r
    fn sbc_r(&mut self, reg: Reg) -> u8 {
        self.sbc(self.regs.get(reg), true)
    }

    /// SBC d8
    fn sbc_d8(&mut self, bus: &mut Bus) -> u8 {
        let d8 = self.fetch(bus);
        self.sbc(d8, true);
        4
    }

    /// SUB (HL)
    fn sub_hl_addr(&mut self, bus: &mut Bus) -> u8 {
        let hl = bus.read_byte(*self.regs.hl);
        self.sub(hl);
        8
    }

    /// SUB d8
    fn sub_d8(&mut self, bus: &mut Bus) -> u8 {
        let d8 = self.fetch(bus);
        self.sub(d8);
        8
    }

    fn sub_r(&mut self, reg: Reg) -> u8 {
        self.sub(self.regs.get(reg))
    }

    fn sub(&mut self, v: u8) -> u8 {
        self.sbc(v, false);
        4
    }

    fn sbc(&mut self, value: u8, with_carry: bool) -> u8 {
        let c = (with_carry && self.regs.flag_c().is_set()) as u8;
        let reg_a = self.regs.get(Reg::A);

        let (sub1, carry1) = reg_a.overflowing_sub(value);
        let (sub2, carry2) = sub1.overflowing_sub(c);

        self.regs.set(Reg::A, sub2);
        self.regs.flag_z().set_value(sub2 == 0);
        self.regs.flag_n().set();
        self.regs.flag_c().set_value(carry1 | carry2);
        self.regs
            .flag_h()
            .set_value((reg_a & 0x0f) < (value & 0x0f) + c);
        8
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

    fn cp_r(&mut self, reg: Reg) -> u8 {
        let r = self.regs.get(reg);
        self.cp(r);
        4
    }

    fn cp(&mut self, value: u8) {
        let reg_a = self.regs.get(Reg::A);
        let (sub, carry) = reg_a.overflowing_sub(value);
        self.regs.flag_z().set_value(sub == 0);
        self.regs.flag_n().set();
        self.regs.flag_c().set_value(carry);
        self.regs
            .flag_h()
            .set_value((reg_a & 0x0f) < (value & 0x0f));
    }

    fn rst(&mut self, bus: &mut Bus, vec: u8) -> u8 {
        self.call(bus, vec as u16);
        16
    }

    /// CPL -- Complement Accumulator
    fn cpl(&mut self) -> u8 {
        self.regs.set(Reg::A, !self.regs.get(Reg::A));
        self.regs.flag_n().set();
        self.regs.flag_h().set();
        8
    }

    fn swap_hl(&mut self, bus: &mut Bus) -> u8 {
        let r = bus.read_byte(*self.regs.hl);
        let new_r = self.swap(r);
        bus.write_byte(*self.regs.hl, new_r);

        16
    }
    fn swap_r(&mut self, reg: Reg) -> u8 {
        let r = self.regs.get(reg);
        let new_r = self.swap(r);
        self.regs.set(reg, new_r);

        8
    }

    fn swap(&mut self, v: u8) -> u8 {
        let r = v.rotate_right(4);
        self.regs.flag_z().set_value(r == 0);
        self.regs.flag_n().clear();
        self.regs.flag_h().clear();
        self.regs.flag_c().clear();
        r
    }

    fn res_n_r(&mut self, n: u8, reg: Reg) -> u8 {
        let mut r = self.regs.get(reg);
        r.view_bits_mut::<Lsb0>().set(n as usize, false);
        self.regs.set(reg, r);

        8
    }

    fn res_hl(&mut self, n: u8, bus: &mut Bus) -> u8 {
        let mut hl = bus.read_byte(*self.regs.hl);
        hl.view_bits_mut::<Lsb0>().set(n as usize, false);
        bus.write_byte(*self.regs.hl, hl);

        16
    }

    fn set_n_r(&mut self, n: u8, reg: Reg) -> u8 {
        let mut r = self.regs.get(reg);
        r.view_bits_mut::<Lsb0>().set(n as usize, true);
        self.regs.set(reg, r);

        8
    }

    fn set_hl(&mut self, n: u8, bus: &mut Bus) -> u8 {
        let mut hl = bus.read_byte(*self.regs.hl);
        hl.view_bits_mut::<Lsb0>().set(n as usize, true);
        bus.write_byte(*self.regs.hl, hl);

        16
    }

    fn daa(&mut self) -> u8 {
        let mut a = self.regs.get(Reg::A);
        let mut adjust = if self.regs.flag_c().is_set() {
            0x60
        } else {
            0x00
        };
        if self.regs.flag_h().is_set() {
            adjust |= 0x06;
        };
        if !self.regs.flag_n().is_set() {
            if a & 0x0f > 0x09 {
                adjust |= 0x06;
            };
            if a > 0x99 {
                adjust |= 0x60;
            };
            a = a.wrapping_add(adjust);
        } else {
            a = a.wrapping_sub(adjust);
        }
        self.regs.flag_c().set_value(adjust >= 0x60);
        self.regs.flag_h().set_value(false);
        self.regs.flag_z().set_value(a == 0x00);
        self.regs.set(Reg::A, a);

        4
    }

    // fn daa(&mut self) -> u8 {
    //     let reg_a = self.regs.get(Reg::A);
    //     let hi = (reg_a & 0xf0) >> 4;
    //     let lo = reg_a & 0x0f;

    //     if self.regs.flag_n().is_set() {
    //         // Last operation was subtraction
    //         match (self.regs.flag_c().is_set(), self.regs.flag_h().is_set()) {
    //             (false, false) => (),
    //             (false, true) => {
    //                 if hi <= 8 && lo >= 6 {
    //                     self.add(0xfa);
    //                 }
    //             }
    //             (true, false) => {
    //                 if hi >= 7 && lo <= 9 {
    //                     self.add(0xa0);
    //                 }
    //             }
    //             (true, true) => {
    //                 if hi >= 6 && lo >= 6 {
    //                     self.add(0x9a);
    //                 }
    //             }
    //         }
    //     } else {
    //         // Last operation was an addition
    //         match (self.regs.flag_c().is_set(), self.regs.flag_h().is_set()) {
    //             (false, false) => {
    //                 if hi <= 8 && lo >= 0x0a {
    //                     self.add(0x06);
    //                 } else if hi >= 0x0a && lo <= 9 {
    //                     self.add(0x60);
    //                 } else if hi >= 0x09 && lo >= 0x0a {
    //                     self.add(0x66);
    //                 }
    //             }
    //             (false, true) => {
    //                 if hi <= 9 && lo <= 3 {
    //                     self.add(0x06);
    //                 } else if hi >= 0x0a && lo <= 3 {
    //                     self.add(0x66);
    //                 }
    //             }
    //             (true, false) => {
    //                 if hi <= 2 && lo <= 9 {
    //                     self.add(0x60);
    //                 } else if hi <= 2 && lo >= 0x0a {
    //                     self.add(0x66);
    //                 }
    //             }
    //             (true, true) => {
    //                 if hi <= 3 && lo <= 3 {
    //                     self.add(0x66);
    //                 }
    //             }
    //         }
    //     }

    //     4
    // }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_pause(&mut self, pause: bool) {
        self.paused = pause;
    }

    /// Set the cpu's breakpoint.
    pub fn set_breakpoint(&mut self, breakpoint: u16) {
        self.breakpoint = breakpoint;
    }

    /// Get the cpu's halted.
    pub fn halted(&self) -> bool {
        self.halted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rl() {
        let mut cpu = Cpu::default();

        let mut test = |value, carry, res, new_carry, zero| {
            cpu.regs.set(Reg::A, value);
            cpu.regs.flag_c().set_value(carry);
            cpu.rl_r(Reg::A);

            assert!(cpu.regs.get(Reg::A) == res);
            assert!(cpu.regs.flag_c().is_set() == new_carry);
            assert!(cpu.regs.flag_z().is_set() == zero);
        };

        test(0b01010101, false, 0b10101010, false, false);
        test(0b01010101, true, 0b10101011, false, false);
        test(0b10101010, false, 0b01010100, true, false);
        test(0b10101010, true, 0b01010101, true, false);

        // Make sure Z flag gets set
        test(0b10000000, false, 0b00000000, true, true);
    }

    #[test]
    fn test_rr() {
        let mut cpu = Cpu::default();

        let mut test = |value, carry, res, new_carry, zero| {
            cpu.regs.set(Reg::A, value);
            cpu.regs.flag_c().set_value(carry);
            cpu.rr_r(Reg::A);

            assert!(cpu.regs.get(Reg::A) == res);
            assert!(cpu.regs.flag_c().is_set() == new_carry);
            assert!(cpu.regs.flag_z().is_set() == zero);
        };

        test(0b01010101, false, 0b00101010, true, false);
        test(0b01010101, true, 0b10101010, true, false);
        test(0b10101010, false, 0b01010101, false, false);
        test(0b10101010, true, 0b11010101, false, false);

        // Make sure Z flag gets set
        test(0b00000001, false, 0b00000000, true, true);
    }

    #[test]
    fn test_inc() {
        let mut cpu = Cpu::default();
        let mut test_and_check_flags = |value, res, z, n, h, c| {
            // reset all flags
            cpu.regs.set_pair(RegPair::AF, 0x0000);
            assert_eq!(res, cpu.inc_value_and_set_flags(value));
            assert_eq!(z, cpu.regs.flag_z().is_set());
            assert_eq!(n, cpu.regs.flag_n().is_set());
            assert_eq!(h, cpu.regs.flag_h().is_set());
            assert_eq!(c, cpu.regs.flag_c().is_set());
        };

        test_and_check_flags(0x00, 0x01, false, false, false, false);
        // H is set
        test_and_check_flags(0x0F, 0x10, false, false, true, false);
        // Z is set, and C is ignored
        test_and_check_flags(0xFF, 0x00, true, false, true, false);
    }

    #[test]
    fn test_dec() {
        let mut cpu = Cpu::default();
        let mut test_and_check_flags = |value, res, z, n, h, c| {
            // reset all flags
            cpu.regs.set_pair(RegPair::AF, 0x0000);
            assert_eq!(res, cpu.dec_value_and_set_flags(value));
            assert_eq!(z, cpu.regs.flag_z().is_set());
            assert_eq!(n, cpu.regs.flag_n().is_set());
            assert_eq!(h, cpu.regs.flag_h().is_set());
            assert_eq!(c, cpu.regs.flag_c().is_set());
        };

        test_and_check_flags(0x02, 0x01, false, true, false, false);
        // H is set
        test_and_check_flags(0x10, 0x0F, false, true, true, false);
        // Z is set, and C is ignored
        test_and_check_flags(0x01, 0x00, true, true, false, false);
    }
}
