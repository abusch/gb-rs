#[derive(Default)]
pub struct Cpu {
    regs: Registers,

    sp: u16,
    pc: u16,
}

impl Cpu {
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
