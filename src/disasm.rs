use std::io::Cursor;

use anyhow::Result;
use bitvec::prelude::*;
use byteorder::LittleEndian;
use byteorder::ReadBytesExt;

pub struct Instr {
    /// String representation of the decoded instruction
    repr: String,
    /// Number of bytes that this instruction takes
    pub bytes: u16,
}

impl Instr {
    pub fn new(repr: String, bytes: u16) -> Self {
        Self { repr, bytes }
    }
}

impl std::fmt::Display for Instr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.repr)
    }
}

pub struct Disassembler<'a> {
    instructions: Cursor<&'a [u8]>,
    decoded: Vec<Instr>,
}

impl<'a> Disassembler<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            instructions: Cursor::new(bytes),
            decoded: Vec::new(),
        }
    }

    pub fn run(mut self) -> Vec<Instr> {
        // ignore error: it just meant we didn't have enough bytes to decode the last instruction
        let _ = self.run_inner();
        self.decoded
    }

    fn run_inner(&mut self) -> Result<()> {
        while let Ok(op) = self.read_byte() {
            // unprefixed
            let bits = op.view_bits::<Lsb0>();
            let x = bits[6..=7].load::<u8>();
            let y = bits[3..=5].load::<u8>();
            let z = bits[0..=2].load::<u8>();
            let p = bits[4..=5].load::<u8>();
            let q = bits[3];

            match x {
                0 => match z {
                    0 => match y {
                        0 => self.push("NOP".to_string(), 1),
                        1 => {
                            let nn = self.read_word()?;
                            self.push(format!("LD (${nn:04X}),SP"), 3);
                        }
                        2 => {
                            // STOP is encoded as 2 bytes for some reason
                            let _ = self.read_byte();
                            self.push("STOP".to_string(), 2);
                        }
                        3 => {
                            let d = self.read_byte()? as i8;
                            self.push(format!("JR {d}"), 2);
                        }
                        4..=7 => {
                            let cond = cc(y - 4);
                            let d = self.read_byte()? as i8;
                            self.push(format!("JR {cond},{d}"), 2);
                        }
                        _ => unreachable!(),
                    },
                    1 => {
                        let rp = rp(p);
                        if q {
                            self.push(format!("ADD HL,{rp}"), 1);
                        } else {
                            let nn = self.read_word()?;
                            self.push(format!("LD {rp},${nn:04X}"), 3);
                        }
                    }
                    2 => {
                        let reg = ind_load_reg(p);
                        if q {
                            self.push(format!("LD A,({reg})"), 1);
                        } else {
                            self.push(format!("LD ({reg}),A"), 1);
                        }
                    }
                    3 => {
                        let reg = rp(p);
                        if q {
                            self.push(format!("DEC {reg}"), 1);
                        } else {
                            self.push(format!("INC {reg}"), 1);
                        }
                    }
                    4 => {
                        let r = r(y);
                        self.push(format!("INC {r}"), 1);
                    }
                    5 => {
                        let r = r(y);
                        self.push(format!("DEC {r}"), 1);
                    }
                    6 => {
                        let r = r(y);
                        let n = self.read_byte()?;
                        self.push(format!("LD {r},${n:02X}"), 2);
                    }
                    7 => {
                        let op = match y {
                            0 => "RLCA",
                            1 => "RRCA",
                            2 => "RLA",
                            3 => "RRA",
                            4 => "DAA",
                            5 => "CPL",
                            6 => "SCF",
                            7 => "CCF",
                            _ => unreachable!(),
                        };
                        self.push(op.to_string(), 1);
                    }
                    _ => unreachable!(),
                },
                1 => {
                    if z == 6 && y == 6 {
                        self.push("HALT".to_string(), 1);
                    } else {
                        let r1 = r(y);
                        let r2 = r(z);
                        self.push(format!("LD {r1},{r2}"), 1);
                    }
                }
                2 => {
                    let alu = alu(y);
                    let r = r(z);
                    self.push(format!("{alu}{r}"), 1);
                }
                3 => match z {
                    0 => match y {
                        0..=3 => {
                            let cc = cc(y);
                            self.push(format!("RET {cc}"), 1);
                        }
                        4 => {
                            let n = self.read_byte()?;
                            let addr = 0xFF00 + n as u16;
                            self.push(format!("LD (${addr:04X}),A"), 2);
                        }
                        5 => {
                            let d = self.read_byte()? as i8;
                            self.push(format!("ADD SP,{d}"), 2);
                        }
                        6 => {
                            let n = self.read_byte()?;
                            let addr = 0xFF00 + n as u16;
                            self.push(format!("LD A,(${addr:04X})"), 2);
                        }
                        7 => {
                            let d = self.read_byte()? as i8;
                            self.push(format!("LD HL,SP + {d}"), 2);
                        }
                        _ => unreachable!(),
                    },
                    1 => {
                        if q {
                            let op = match p {
                                0 => "RET",
                                1 => "RETI",
                                2 => "JP HL",
                                3 => "LD SP,HL",
                                _ => unreachable!(),
                            };
                            self.push(op.to_string(), 1);
                        } else {
                            let r = rp2(p);
                            self.push(format!("POP {r}"), 1);
                        }
                    }
                    2 => match y {
                        0..=3 => {
                            let cc = cc(y);
                            let nn = self.read_word()?;
                            self.push(format!("JP {cc},${nn:04X}"), 3);
                        }
                        4 => self.push("LD ($FF00+C),A".to_string(), 1),
                        5 => {
                            let nn = self.read_word()?;
                            self.push(format!("LD (${nn:04X}),A"), 3);
                        }
                        6 => self.push("LD A,($FF00+C)".to_string(), 1),
                        7 => {
                            let nn = self.read_word()?;
                            self.push(format!("LD A,(${nn:04X})"), 3);
                        }
                        _ => unreachable!(),
                    },
                    3 => {
                        match y {
                            0 => {
                                let nn = self.read_word()?;
                                self.push(format!("JP ${nn:04X}"), 3);
                            }
                            1 => {
                                // CB prefix
                                let sub_op = self.read_byte()?;
                                self.push(disasm_cb_instr(sub_op), 2);
                            }
                            2..=5 => {
                                self.push(format!("<unknown> {op}"), 1);
                            }
                            6 => self.push("DI".to_string(), 1),
                            7 => self.push("EI".to_string(), 1),
                            _ => unreachable!(),
                        }
                    }
                    4 => {
                        let cc = cc(y);
                        let nn = self.read_word()?;
                        self.push(format!("CALL {cc},${nn:04X}"), 3);
                    }
                    5 => {
                        if q {
                            if p == 0 {
                                let nn = self.read_word()?;
                                self.push(format!("CALL ${nn:04X}"), 3);
                            } else {
                                self.push(format!("<unknown> {op}"), 1);
                            }
                        } else {
                            let r = rp2(p);
                            self.push(format!("PUSH {r}"), 1);
                        }
                    }
                    6 => {
                        let alu = alu(y);
                        let n = self.read_byte()?;
                        self.push(format!("{alu}${n:02X}"), 2);
                    }
                    7 => {
                        let n = y * 8;
                        self.push(format!("RST {n}"), 1);
                    }
                    _ => unreachable!(), // z
                },
                _ => unreachable!(), // x
            }
        }
        Ok(())
    }

    fn push(&mut self, repr: impl Into<String>, bytes: u16) {
        self.decoded.push(Instr::new(repr.into(), bytes));
    }

    fn read_byte(&mut self) -> Result<u8> {
        Ok(self.instructions.read_u8()?)
    }

    fn read_word(&mut self) -> Result<u16> {
        Ok(self.instructions.read_u16::<LittleEndian>()?)
    }
}

fn disasm_cb_instr(op: u8) -> String {
    let bits = op.view_bits::<Lsb0>();
    let x = bits[6..=7].load::<u8>();
    let y = bits[3..=5].load::<u8>();
    let z = bits[0..=2].load::<u8>();

    let r = r(z);

    match x {
        0 => {
            let rot = match y {
                0 => "RLC",
                1 => "RRC",
                2 => "RL",
                3 => "RR",
                4 => "SLA",
                5 => "SRA",
                6 => "SWAP",
                7 => "SRL",
                _ => unreachable!(),
            };
            format!("{rot} {r}")
        }
        1 => format!("BIT {y},{r}"),
        2 => format!("RES {y},{r}"),
        3 => format!("SET {y},{r}"),
        _ => unreachable!(),
    }
}

fn r(n: u8) -> &'static str {
    match n {
        0 => "B",
        1 => "C",
        2 => "D",
        3 => "E",
        4 => "H",
        5 => "L",
        6 => "(HL)",
        7 => "A",
        _ => unreachable!(),
    }
}

fn cc(n: u8) -> &'static str {
    match n {
        0 => "NZ",
        1 => "Z",
        2 => "NC",
        3 => "C",
        _ => unreachable!(),
    }
}

fn rp(n: u8) -> &'static str {
    match n {
        0 => "BC",
        1 => "DE",
        2 => "HL",
        3 => "SP",
        _ => unreachable!(),
    }
}

fn rp2(n: u8) -> &'static str {
    match n {
        0 => "BC",
        1 => "DE",
        2 => "HL",
        3 => "AF",
        _ => unreachable!(),
    }
}

fn ind_load_reg(n: u8) -> &'static str {
    match n {
        0 => "BC",
        1 => "DE",
        2 => "HL+",
        3 => "HL-",
        _ => unreachable!(),
    }
}

fn alu(n: u8) -> &'static str {
    match n {
        0 => "ADD A,",
        1 => "ADC A,",
        2 => "SUB ",
        3 => "SBC A,",
        4 => "AND ",
        5 => "XOR ",
        6 => "OR ",
        7 => "CP ",
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boot_rom() {
        let rom = std::fs::read("assets/dmg_boot.bin").unwrap();
        let ops = Disassembler::new(&rom).run();

        let output = ops
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        println!("{}", output);
        panic!("");
    }
}
