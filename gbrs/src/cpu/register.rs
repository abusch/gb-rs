use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Registers {
    pub(super) af: Register,
    pub(super) bc: Register,
    pub(super) de: Register,
    pub(super) hl: Register,
}

/// Zero flag
pub(super) const FLAG_Z: u16 = 0x80;
/// Subtract flag
pub(super) const FLAG_N: u16 = 0x40;
/// Half Carry flag
pub(super) const FLAG_H: u16 = 0x20;
/// Carry flag
pub(super) const FLAG_C: u16 = 0x10;

impl Registers {
    pub(super) fn get(&self, name: Reg) -> u8 {
        match name {
            Reg::A => self.af.hi(),
            Reg::B => self.bc.hi(),
            Reg::C => self.bc.lo(),
            Reg::D => self.de.hi(),
            Reg::E => self.de.lo(),
            Reg::H => self.hl.hi(),
            Reg::L => self.hl.lo(),
        }
    }

    pub(super) fn set(&mut self, name: Reg, value: u8) {
        match name {
            Reg::A => self.af.set_hi(value),
            Reg::B => self.bc.set_hi(value),
            Reg::C => self.bc.set_lo(value),
            Reg::D => self.de.set_hi(value),
            Reg::E => self.de.set_lo(value),
            Reg::H => self.hl.set_hi(value),
            Reg::L => self.hl.set_lo(value),
        }
    }

    pub(super) fn get_pair(&self, pair: RegPair) -> u16 {
        match pair {
            RegPair::AF => *self.af,
            RegPair::BC => *self.bc,
            RegPair::DE => *self.de,
            RegPair::HL => *self.hl,
        }
    }

    pub(super) fn set_pair(&mut self, pair: RegPair, value: u16) {
        match pair {
            RegPair::AF => *self.af = value & 0xFFF0, // ignore low bits of the flag
            RegPair::BC => *self.bc = value,
            RegPair::DE => *self.de = value,
            RegPair::HL => *self.hl = value,
        }
    }

    /// Whether the given `FLAG_*` is set.
    pub(super) fn flag(&self, flag: u16) -> bool {
        *self.af & flag != 0
    }

    pub(super) fn set_flag(&mut self, flag: u16, value: bool) {
        if value {
            *self.af |= flag;
        } else {
            *self.af &= !flag;
        }
    }

    /// Set all 4 flags at once.
    pub(super) fn set_flags(&mut self, z: bool, n: bool, h: bool, c: bool) {
        self.set_flag(FLAG_Z, z);
        self.set_flag(FLAG_N, n);
        self.set_flag(FLAG_H, h);
        self.set_flag(FLAG_C, c);
    }
}

impl Debug for Registers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registers")
            .field("af", &format_args!("${:04X}", *self.af))
            .field("bc", &format_args!("${:04X}", *self.bc))
            .field("de", &format_args!("${:04X}", *self.de))
            .field("hl", &format_args!("${:04X}", *self.hl))
            .field(
                "flags",
                &format_args!(
                    "{}{}{}{}",
                    if self.flag(FLAG_Z) { "Z" } else { "-" },
                    if self.flag(FLAG_N) { "N" } else { "-" },
                    if self.flag(FLAG_H) { "H" } else { "-" },
                    if self.flag(FLAG_C) { "C" } else { "-" },
                ),
            )
            .finish()
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Register(u16);

impl Register {
    fn hi(&self) -> u8 {
        (self.0 >> 8) as u8
    }

    fn lo(&self) -> u8 {
        (self.0 & 0x00FF) as u8
    }

    fn set_hi(&mut self, b: u8) {
        self.0 = ((b as u16) << 8) | (self.0 & 0xFF);
    }

    fn set_lo(&mut self, b: u8) {
        self.0 = (self.0 & 0xFF00) | (b as u16);
    }
}

impl Deref for Register {
    type Target = u16;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Register {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reg {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegPair {
    AF,
    BC,
    DE,
    HL,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flags() {
        let mut regs = Registers::default();

        assert!(!regs.flag(FLAG_Z));
        regs.set_flag(FLAG_Z, true);
        assert!(regs.flag(FLAG_Z));
        assert_eq!(*regs.af, 0x0080);
        regs.set_flag(FLAG_Z, false);
        assert!(!regs.flag(FLAG_Z));

        regs.set_flags(false, true, false, true);
        assert_eq!(*regs.af, 0x0050);
    }
}
