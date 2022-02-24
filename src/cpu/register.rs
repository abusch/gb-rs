use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

// TODO don't think this is a great design... maybe we need a `Register` struct for a single
// register.
#[derive(Default)]
pub(super) struct Registers {
    pub(super) af: Register,
    pub(super) bc: Register,
    pub(super) de: Register,
    pub(super) hl: Register,
}

impl Registers {
    /// Zero flag
    const FLAG_Z: u16 = 0x80;
    /// Subtract flag
    const FLAG_N: u16 = 0x40;
    /// Half Carry flag
    const FLAG_H: u16 = 0x20;
    /// Carry flag
    const FLAG_C: u16 = 0x10;

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

    // Flags
    pub(super) fn flag_z(&mut self) -> Flags<'_> {
        Flags::new(self, Self::FLAG_Z)
    }

    // N Flag
    pub(super) fn flag_n(&mut self) -> Flags<'_> {
        Flags::new(self, Self::FLAG_N)
    }

    // H Flag
    pub(super) fn flag_h(&mut self) -> Flags<'_> {
        Flags::new(self, Self::FLAG_H)
    }

    // C Flag
    pub(super) fn flag_c(&mut self) -> Flags<'_> {
        Flags::new(self, Self::FLAG_C)
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
                    if (*self.af & Self::FLAG_Z) != 0 {
                        "Z"
                    } else {
                        "-"
                    },
                    if (*self.af & Self::FLAG_N) != 0 {
                        "N"
                    } else {
                        "-"
                    },
                    if (*self.af & Self::FLAG_H) != 0 {
                        "H"
                    } else {
                        "-"
                    },
                    if (*self.af & Self::FLAG_C) != 0 {
                        "C"
                    } else {
                        "-"
                    },
                ),
            )
            .finish()
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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

pub(super) struct Flags<'regs> {
    regs: &'regs mut Registers,
    flag: u16,
}

impl<'regs> Flags<'regs> {
    fn new(regs: &mut Registers, flag: u16) -> Flags {
        Flags { regs, flag }
    }

    pub(super) fn is_set(&self) -> bool {
        *self.regs.af & self.flag != 0
    }

    pub(super) fn set(&mut self) {
        *self.regs.af |= self.flag;
    }

    pub(super) fn clear(&mut self) {
        *self.regs.af &= !self.flag;
    }

    pub(super) fn set_value(&mut self, value: bool) {
        if value {
            self.set();
        } else {
            self.clear();
        }
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

        assert!(!regs.flag_z().is_set());
        regs.flag_z().set();
        assert!(regs.flag_z().is_set());
        regs.flag_z().clear();
        assert!(!regs.flag_z().is_set());

        regs.flag_z().set_value(true);
        assert!(regs.flag_z().is_set());
        regs.flag_z().set_value(false);
        assert!(!regs.flag_z().is_set());
    }
}
