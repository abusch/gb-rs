pub struct Interconnect {
    ram: Box<[u8]>,
}

impl Interconnect {
    pub fn new(ram_size: usize) -> Self {
        let ram = vec![0; ram_size];

        Self {
            ram: ram.into_boxed_slice(),
        }
    }

    pub fn read_byte(&self, addr: u16) -> u8 {
        self.ram[addr as usize]
    }

    pub fn read_word(&self, addr: u16) -> u16 {
        // FIXME is this little- or big-endian?
        let lsb = self.read_byte(addr);
        let msb = self.read_byte(addr + 1);

        ((msb as u16) << 8) | (lsb as u16)
    }

    pub fn write_byte(&mut self, addr: u16, b: u8) {
        self.ram[addr as usize] = b;
    }
}
