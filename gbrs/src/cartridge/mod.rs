mod mbc1;
mod mbc2;
mod mbc3;
mod mbc5;
mod rtc;

use anyhow::Result;
use log::warn;

use mbc1::Mbc1;
use mbc2::{MBC2_RAM_SIZE, Mbc2};
use mbc3::Mbc3;
use mbc5::Mbc5;
pub use rtc::RTC_SAVE_SIZE;

/// The memory bank controller, which maps the cartridge's ROM and RAM into the address space.
enum Mbc {
    Mbc1(Mbc1),
    Mbc2(Mbc2),
    Mbc3(Mbc3),
    Mbc5(Mbc5),
}

pub struct Cartridge {
    data: Box<[u8]>,
    ram: Box<[u8]>,
    mbc: Mbc,
}

impl Cartridge {
    /// Create a cartridge from a raw ROM image, with its external RAM zeroed. Battery-backed RAM
    /// can be restored afterwards through [`Cartridge::save_ram_mut`].
    pub fn load_bytes(content: Vec<u8>) -> Result<Self> {
        // The header ends at 0x014F; everything below indexes into it unconditionally.
        anyhow::ensure!(
            content.len() >= 0x150,
            "ROM is too small ({} bytes) to contain a cartridge header",
            content.len()
        );
        let mut cartridge = Self {
            data: content.into_boxed_slice(),
            ram: Box::default(),
            mbc: Mbc::Mbc1(Mbc1::new(0, 0, false, false)),
        };
        let rom_len = cartridge.data.len();
        let ram_banks = cartridge.get_num_ram_banks().unwrap_or(0) as usize;
        let mut ram_size = ram_banks * 0x2000;
        cartridge.mbc = match cartridge.data[0x0147] {
            0x05 | 0x06 => {
                // The RAM is built into the MBC, so the header doesn't declare it.
                ram_size = MBC2_RAM_SIZE;
                Mbc::Mbc2(Mbc2::new(rom_len))
            }
            0x0F..=0x13 => Mbc::Mbc3(Mbc3::new(rom_len, ram_banks, cartridge.has_rtc())),
            t @ 0x19..=0x1E => Mbc::Mbc5(Mbc5::new(rom_len, ram_banks, t >= 0x1C)),
            t @ 0x00..=0x03 => Mbc::Mbc1(Mbc1::new(
                rom_len,
                ram_banks,
                t != 0x00,
                t != 0x00 && cartridge.is_multicart(),
            )),
            _ => {
                warn!(
                    "Unsupported cartridge type {}, falling back to MBC1",
                    cartridge.cartridge_type()
                );
                Mbc::Mbc1(Mbc1::new(rom_len, ram_banks, false, false))
            }
        };
        cartridge.ram = vec![0; ram_size].into_boxed_slice();
        Ok(cartridge)
    }

    pub fn cgb_flag(&self) -> bool {
        self.data[0x143] >> 7 != 0
    }

    pub fn sgb_flag(&self) -> bool {
        self.data[0x146] == 0x03
    }

    pub fn title(&self) -> String {
        let bytes = &self.data[0x0134..=0x0143];
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());

        String::from_utf8_lossy(&bytes[..end]).to_string()
    }

    pub fn licensee_code(&self) -> String {
        let code = self.data[0x014B];
        if code == 33 {
            // Uses New Licensee code instead
            String::from_utf8_lossy(&self.data[0x0144..=0x0145]).to_string()
        } else {
            // Old licensee code
            format!("{:02x} (OLD)", code)
        }
    }

    pub fn cartridge_type(&self) -> &'static str {
        match self.data[0x0147] {
            0x00 => "ROM ONLY",
            0x01 => "MBC1",
            0x02 => "MBC1+RAM",
            0x03 => "MBC1+RAM+BATTERY",
            0x05 => "MBC2",
            0x06 => "MBC2+BATTERY",
            0x08 => "ROM+RAM 1",
            0x09 => "ROM+RAM+BATTERY 1",
            0x0B => "MMM01",
            0x0C => "MMM01+RAM",
            0x0D => "MMM01+RAM+BATTERY",
            0x0F => "MBC3+TIMER+BATTERY",
            0x10 => "MBC3+TIMER+RAM+BATTERY 2",
            0x11 => "MBC3",
            0x12 => "MBC3+RAM 2",
            0x13 => "MBC3+RAM+BATTERY 2",
            0x19 => "MBC5",
            0x1A => "MBC5+RAM",
            0x1B => "MBC5+RAM+BATTERY",
            0x1C => "MBC5+RUMBLE",
            0x1D => "MBC5+RUMBLE+RAM",
            0x1E => "MBC5+RUMBLE+RAM+BATTERY",
            0x20 => "MBC6",
            0x22 => "MBC7+SENSOR+RUMBLE+RAM+BATTERY",
            0xFC => "POCKET CAMERA",
            0xFD => "BANDAI TAMA5",
            0xFE => "HuC3",
            0xFF => "HuC1+RAM+BATTERY",
            b => panic!("Unknown cartridge type {:x}", b),
        }
    }

    pub fn has_ram(&self) -> bool {
        matches!(
            self.data[0x147],
            0x02 | 0x03
                | 0x08
                | 0x09
                | 0x0c
                | 0x0d
                | 0x10
                | 0x12
                | 0x13
                | 0x1A
                | 0x1B
                | 0x1D
                | 0x1E
                | 0x22
                | 0xFF
        )
    }

    /// Whether this is an MBC1M multicart. Their headers don't say so, but they're 1MiB carts
    /// holding several games, so the second game's header (with its Nintendo logo) is found at
    /// bank 0x10.
    fn is_multicart(&self) -> bool {
        const LOGO: std::ops::Range<usize> = 0x0104..0x0134;
        const GAME_2: usize = 0x10 * 0x4000;
        self.data.len() == 1024 * 1024
            && self.data[GAME_2 + LOGO.start..GAME_2 + LOGO.end] == self.data[LOGO]
    }

    /// Whether the cartridge has an MBC3 real-time clock.
    pub fn has_rtc(&self) -> bool {
        matches!(self.data[0x0147], 0x0F | 0x10)
    }

    pub fn get_rom_size(&self) -> u8 {
        self.data[0x0148]
    }

    pub fn get_ram_size(&self) -> u8 {
        self.data[0x0149]
    }

    /// Read a byte from the ROM area (0000-7FFF), through the memory bank controller.
    pub fn read_rom(&self, addr: u16) -> u8 {
        match &self.mbc {
            Mbc::Mbc1(mbc) => mbc.read_rom(&self.data, addr),
            Mbc::Mbc2(mbc) => mbc.read_rom(&self.data, addr),
            Mbc::Mbc3(mbc) => mbc.read_rom(&self.data, addr),
            Mbc::Mbc5(mbc) => mbc.read_rom(&self.data, addr),
        }
    }

    /// Write a byte to the ROM area (0000-7FFF), which holds the memory bank controller's
    /// registers.
    pub fn write_rom(&mut self, addr: u16, b: u8) {
        match &mut self.mbc {
            Mbc::Mbc1(mbc) => mbc.write_register(addr, b),
            Mbc::Mbc2(mbc) => mbc.write_register(addr, b),
            Mbc::Mbc3(mbc) => mbc.write_register(addr, b),
            Mbc::Mbc5(mbc) => mbc.write_register(addr, b),
        }
    }

    /// Read a byte from the selected bank of this cartridge's external RAM.
    ///
    /// The given address should be relative to the selected bank, i.e. in the range 0000-1FFF.
    pub fn read_ram(&self, addr: u16) -> u8 {
        assert!(addr < 0x2000, "addr=0x{:04x}", addr);
        match &self.mbc {
            Mbc::Mbc1(mbc) => mbc.read_ram(&self.ram, addr),
            Mbc::Mbc2(mbc) => mbc.read_ram(&self.ram, addr),
            Mbc::Mbc3(mbc) => mbc.read_ram(&self.ram, addr),
            Mbc::Mbc5(mbc) => mbc.read_ram(&self.ram, addr),
        }
    }

    /// Write a byte into the selected bank of this cartridge's external RAM
    ///
    /// The given address should be relative to the selected bank, i.e. in the range 0000-1FFF.
    pub fn write_ram(&mut self, addr: u16, b: u8) {
        assert!(addr < 0x2000);
        match &mut self.mbc {
            Mbc::Mbc1(mbc) => mbc.write_ram(&mut self.ram, addr, b),
            Mbc::Mbc2(mbc) => mbc.write_ram(&mut self.ram, addr, b),
            Mbc::Mbc3(mbc) => mbc.write_ram(&mut self.ram, addr, b),
            Mbc::Mbc5(mbc) => mbc.write_ram(&mut self.ram, addr, b),
        }
    }

    /// Run the cartridge's own hardware (i.e. the RTC) for the given number of clock cycles.
    pub(crate) fn step(&mut self, cycles: u8) {
        if let Mbc::Mbc3(Mbc3 { rtc: Some(rtc), .. }) = &mut self.mbc {
            rtc.step(cycles);
        }
    }

    /// Reset the memory bank controller to its power-on state, leaving RAM and the RTC untouched.
    pub fn reset_mapper(&mut self) {
        match &mut self.mbc {
            Mbc::Mbc1(mbc) => mbc.reset(),
            Mbc::Mbc2(mbc) => mbc.reset(),
            Mbc::Mbc3(mbc) => mbc.reset(),
            Mbc::Mbc5(mbc) => mbc.reset(),
        }
    }

    /// The state of the real-time clock, stamped with the given time in seconds since the Unix
    /// epoch, or `None` if the cartridge has no RTC.
    ///
    /// This is in the format BGB and VBA-M append to save files.
    pub fn save_rtc(&self, unix_time: u64) -> Option<[u8; RTC_SAVE_SIZE]> {
        match &self.mbc {
            Mbc::Mbc3(Mbc3 { rtc: Some(rtc), .. }) => Some(rtc.save(unix_time)),
            _ => None,
        }
    }

    /// Restore the real-time clock from the output of [`Cartridge::save_rtc`], advancing it by
    /// the time elapsed since then (`unix_time` being the current time).
    pub fn load_rtc(&mut self, data: &[u8], unix_time: u64) -> Result<()> {
        match &mut self.mbc {
            Mbc::Mbc3(Mbc3 { rtc: Some(rtc), .. }) => rtc.load(data, unix_time),
            _ => anyhow::bail!("Cartridge has no RTC"),
        }
    }

    /// The battery-backed external RAM, sized to what the cartridge has, or `None` if it has no
    /// RAM. For MBC2, that's 512 bytes each holding a half-byte.
    pub fn save_ram(&self) -> Option<&[u8]> {
        (!self.ram.is_empty()).then_some(&self.ram[..])
    }

    /// Mutable version of [`Cartridge::save_ram`].
    pub fn save_ram_mut(&mut self) -> Option<&mut [u8]> {
        (!self.ram.is_empty()).then_some(&mut self.ram[..])
    }

    #[allow(dead_code)]
    fn get_num_rom_banks(&self) -> u16 {
        match self.get_rom_size() {
            0x00 => 2,
            0x01 => 4,
            0x02 => 8,
            0x03 => 16,
            0x04 => 32,
            0x05 => 64,
            0x06 => 128,
            0x07 => 256,
            0x08 => 512,
            s => panic!("Invalid ROM size {}", s),
        }
    }

    fn get_num_ram_banks(&self) -> Option<u16> {
        if self.has_ram() {
            match self.get_ram_size() {
                0x02 => Some(1),
                0x03 => Some(4),
                0x04 => Some(16),
                0x05 => Some(8),
                _ => None,
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cart of the given type and ROM/RAM size codes, with each ROM bank filled with its
    /// number.
    fn cartridge(cartridge_type: u8, rom_size: u8, ram_size: u8) -> Cartridge {
        let banks = 2u16 << rom_size;
        let mut rom: Vec<u8> = (0..banks).flat_map(|bank| [bank as u8; 0x4000]).collect();
        rom[0x0147] = cartridge_type;
        rom[0x0148] = rom_size;
        rom[0x0149] = ram_size;
        Cartridge::load_bytes(rom).unwrap()
    }

    /// A 128KiB MBC3+TIMER+RAM+BATTERY cart with 32KiB of RAM.
    fn mbc3_cartridge() -> Cartridge {
        cartridge(0x10, 0x02, 0x03)
    }

    #[test]
    fn test_mbc1_rom_banking() {
        // 2MiB
        let mut cart = cartridge(0x01, 0x06, 0x00);
        assert_eq!(cart.read_rom(0x4000), 1);
        // Only the low 5 bits are checked for bank 0
        cart.write_rom(0x2000, 0x20);
        assert_eq!(cart.read_rom(0x4000), 1);
        cart.write_rom(0x2000, 0x05);
        cart.write_rom(0x4000, 0x02);
        assert_eq!(cart.read_rom(0x7FFF), 0x45);
        // BANK2 only applies to 0000-3FFF in mode 1
        assert_eq!(cart.read_rom(0x1000), 0x00);
        cart.write_rom(0x6000, 0x01);
        assert_eq!(cart.read_rom(0x1000), 0x40);
        assert_eq!(cart.read_rom(0x4000), 0x45);

        // Banks beyond the end of the ROM wrap around: 256KiB has 16 banks
        let mut cart = cartridge(0x01, 0x03, 0x00);
        cart.write_rom(0x2000, 0x15);
        cart.write_rom(0x4000, 0x03);
        assert_eq!(cart.read_rom(0x4000), 0x05);
        cart.write_rom(0x6000, 0x01);
        assert_eq!(cart.read_rom(0x0000), 0x00);
    }

    #[test]
    fn test_mbc1_multicart() {
        // 1MiB, with a second game's header in bank 0x10
        let multicart = |second_logo: bool| {
            let mut rom: Vec<u8> = (0..64u8).flat_map(|bank| [bank; 0x4000]).collect();
            rom[0x0147] = 0x01;
            rom[0x0148] = 0x05;
            rom[0x0104..0x0134].fill(0xCE);
            if second_logo {
                rom[0x40104..0x40134].fill(0xCE);
            }
            Cartridge::load_bytes(rom).unwrap()
        };

        let mut cart = multicart(true);
        // BANK2 provides bits 4-5 of the bank number, and bit 4 of BANK1 is ignored
        cart.write_rom(0x4000, 0x01);
        cart.write_rom(0x2000, 0x12);
        assert_eq!(cart.read_rom(0x4000), 0x12);
        cart.write_rom(0x6000, 0x01);
        assert_eq!(cart.read_rom(0x1000), 0x10);
        // Bit 4 still counts for the bank 0 check, so this maps each game's first bank.
        cart.write_rom(0x2000, 0x10);
        assert_eq!(cart.read_rom(0x4000), 0x10);
        cart.write_rom(0x4000, 0x00);
        assert_eq!(cart.read_rom(0x4000), 0x00);

        // Without the second header, it's a regular MBC1 cart.
        let mut cart = multicart(false);
        cart.write_rom(0x4000, 0x01);
        cart.write_rom(0x2000, 0x12);
        assert_eq!(cart.read_rom(0x4000), 0x32);
    }

    #[test]
    fn test_mbc1_ram_banking() {
        // 512KiB of ROM, 32KiB of RAM
        let mut cart = cartridge(0x03, 0x04, 0x03);
        // RAM is disabled on power-on
        cart.write_ram(0x0000, 0x42);
        assert_eq!(cart.read_ram(0x0000), 0xFF);
        cart.write_rom(0x0000, 0x0A);
        assert_eq!(cart.read_ram(0x0000), 0x00);

        // In mode 0, BANK2 doesn't apply to RAM, for writes as well as reads.
        cart.write_rom(0x4000, 0x02);
        cart.write_ram(0x0000, 0xAA);
        assert_eq!(cart.read_ram(0x0000), 0xAA);
        assert_eq!(cart.save_ram().unwrap()[0], 0xAA);

        cart.write_rom(0x6000, 0x01);
        assert_eq!(cart.read_ram(0x0000), 0x00);
        cart.write_ram(0x0000, 0xBB);
        assert_eq!(cart.save_ram().unwrap()[2 * 0x2000], 0xBB);

        cart.write_rom(0x0000, 0x00);
        assert_eq!(cart.read_ram(0x0000), 0xFF);
    }

    #[test]
    fn test_mbc2() {
        // 256KiB, the most MBC2 can address
        let mut cart = cartridge(0x06, 0x03, 0x00);
        assert_eq!(cart.save_ram().unwrap().len(), 512);

        // Bit 8 of the address picks the register: set for the ROM bank...
        cart.write_rom(0x0100, 0x05);
        assert_eq!(cart.read_rom(0x4000), 5);
        cart.write_rom(0x3FFF, 0xF3);
        assert_eq!(cart.read_rom(0x4000), 3);
        cart.write_rom(0x2100, 0x00);
        assert_eq!(cart.read_rom(0x4000), 1);
        // ...clear for RAM enable.
        assert_eq!(cart.read_ram(0x0000), 0xFF);
        cart.write_rom(0x3EFF, 0x0A);
        assert_eq!(cart.read_rom(0x4000), 1);
        assert_eq!(cart.read_ram(0x0000), 0xF0);

        // Only the low 4 bits are stored, and the 512 half-bytes are mirrored over A000-BFFF.
        cart.write_ram(0x01FF, 0xAB);
        assert_eq!(cart.read_ram(0x01FF), 0xFB);
        assert_eq!(cart.read_ram(0x1FFF), 0xFB);
        assert_eq!(cart.save_ram().unwrap()[0x1FF], 0x0B);

        cart.write_rom(0x0000, 0x00);
        assert_eq!(cart.read_ram(0x01FF), 0xFF);
        // 4000-7FFF does nothing
        cart.write_rom(0x4100, 0x02);
        assert_eq!(cart.read_rom(0x4000), 1);
    }

    #[test]
    fn test_mbc5_rom_banking() {
        // 8MiB, so all 9 bits of the bank number are used. Banks start with their number.
        let mut rom = vec![0; 512 * 0x4000];
        for bank in 0..512 {
            rom[bank * 0x4000..][..2].copy_from_slice(&(bank as u16).to_le_bytes());
        }
        rom[0x0147] = 0x19;
        rom[0x0148] = 0x08;
        let mut cart = Cartridge::load_bytes(rom).unwrap();
        let bank =
            |cart: &Cartridge| u16::from_le_bytes([cart.read_rom(0x4000), cart.read_rom(0x4001)]);

        assert_eq!(bank(&cart), 1);
        cart.write_rom(0x2000, 0x34);
        cart.write_rom(0x3000, 0x01);
        assert_eq!(bank(&cart), 0x134);
        // Each register only sets its own bits
        cart.write_rom(0x2FFF, 0xFF);
        assert_eq!(bank(&cart), 0x1FF);
        cart.write_rom(0x3FFF, 0xFE);
        assert_eq!(bank(&cart), 0x0FF);
        // Bank 0 can be mapped at 4000-7FFF
        cart.write_rom(0x2000, 0x00);
        assert_eq!(bank(&cart), 0);
        // 6000-7FFF does nothing
        cart.write_rom(0x6000, 0x01);
        assert_eq!(cart.read_rom(0x0000), 0);
    }

    #[test]
    fn test_mbc5_ram_banking() {
        // 1MiB of ROM, 128KiB of RAM
        let mut cart = cartridge(0x1B, 0x05, 0x04);
        cart.write_ram(0x0000, 0x42);
        assert_eq!(cart.read_ram(0x0000), 0xFF);

        cart.write_rom(0x0000, 0x0A);
        for bank in 0..16 {
            cart.write_rom(0x4000, bank);
            cart.write_ram(0x1FFF, 0x10 + bank);
        }
        for bank in 0..16 {
            cart.write_rom(0x4000, bank);
            assert_eq!(cart.read_ram(0x1FFF), 0x10 + bank);
        }
        assert_eq!(cart.save_ram().unwrap()[15 * 0x2000 + 0x1FFF], 0x1F);
        // ROM banks beyond the end wrap around
        cart.write_rom(0x2000, 0x45);
        assert_eq!(cart.read_rom(0x4000), 0x05);

        // On rumble carts, bit 3 drives the motor rather than selecting RAM.
        let mut cart = cartridge(0x1E, 0x05, 0x04);
        cart.write_rom(0x0000, 0x0A);
        cart.write_rom(0x4000, 0x0B);
        cart.write_ram(0x0000, 0x42);
        assert_eq!(cart.save_ram().unwrap()[3 * 0x2000], 0x42);
    }

    #[test]
    fn test_ram_sizes() {
        // No RAM: reads are open bus
        let mut cart = cartridge(0x01, 0x00, 0x00);
        cart.write_rom(0x0000, 0x0A);
        cart.write_ram(0x0000, 0x42);
        assert_eq!(cart.read_ram(0x0000), 0xFF);
        assert!(cart.save_ram().is_none());

        // 128KiB, which is more than MBC1 can address
        let cart = cartridge(0x03, 0x00, 0x04);
        assert_eq!(cart.save_ram().unwrap().len(), 128 * 1024);
    }

    #[test]
    fn test_mbc3_rom_banking() {
        let mut cart = mbc3_cartridge();
        assert_eq!(cart.read_rom(0x4000), 1);
        cart.write_rom(0x2000, 5);
        assert_eq!(cart.read_rom(0x4000), 5);
        assert_eq!(cart.read_rom(0x3FFF), 0);
        // Bank 0 can't be mapped at 4000-7FFF
        cart.write_rom(0x3FFF, 0);
        assert_eq!(cart.read_rom(0x7FFF), 1);
        // Banks beyond the end of the ROM wrap around
        cart.write_rom(0x2000, 13);
        assert_eq!(cart.read_rom(0x4000), 5);
    }

    #[test]
    fn test_mbc3_ram_banking() {
        let mut cart = mbc3_cartridge();
        // RAM is disabled on power-on
        cart.write_ram(0x0000, 0x42);
        assert_eq!(cart.read_ram(0x0000), 0xFF);

        cart.write_rom(0x0000, 0x0A);
        for bank in 0..4 {
            cart.write_rom(0x4000, bank);
            cart.write_ram(0x1FFF, 0x10 + bank);
        }
        for bank in 0..4 {
            cart.write_rom(0x4000, bank);
            assert_eq!(cart.read_ram(0x1FFF), 0x10 + bank);
        }
        assert_eq!(cart.save_ram().unwrap()[3 * 0x2000 + 0x1FFF], 0x13);

        cart.write_rom(0x0000, 0x00);
        assert_eq!(cart.read_ram(0x1FFF), 0xFF);
    }

    #[test]
    fn test_mbc3_rtc_registers() {
        let mut cart = mbc3_cartridge();
        cart.write_rom(0x0000, 0x0A);
        // Minutes
        cart.write_rom(0x4000, 0x09);
        cart.write_ram(0x0000, 0xFF);
        assert_eq!(cart.read_ram(0x0000), 0x3F);

        // Reads return the latched value, until the next 0 then 1 write to 6000-7FFF.
        cart.write_rom(0x4000, 0x08);
        for _ in 0..4_194_304 / 4 {
            cart.step(4);
        }
        assert_eq!(cart.read_ram(0x0000), 0);
        cart.write_rom(0x6000, 0x01);
        assert_eq!(cart.read_ram(0x0000), 0);
        cart.write_rom(0x6000, 0x00);
        cart.write_rom(0x6000, 0x01);
        assert_eq!(cart.read_ram(0x1234), 1);

        // Unmapped register numbers
        cart.write_rom(0x4000, 0x0D);
        assert_eq!(cart.read_ram(0x0000), 0xFF);

        let saved = cart.save_rtc(0).unwrap();
        let mut restored = mbc3_cartridge();
        restored.load_rtc(&saved, 0).unwrap();
        assert_eq!(restored.save_rtc(0), Some(saved));
    }
}
