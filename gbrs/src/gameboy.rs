use crate::bus::{BootRom, Bus};
use crate::cartridge::{Cartridge, RTC_SAVE_SIZE};
use crate::cpu::Cpu;
use crate::disasm::Disassembler;
use crate::joypad::Button;
use crate::{AudioSink, FrameSink, Rgb555};

pub struct GameBoy {
    cpu: Cpu,
    bus: Bus,
}

impl GameBoy {
    /// Power on a Game Boy with the given cartridge inserted.
    ///
    /// With a boot ROM, emulation starts by running it. Without one, it starts directly at the
    /// cartridge's entry point, in the state the boot ROM would have left the hardware in.
    pub fn new(
        cartridge: Cartridge,
        boot_rom: Option<BootRom>,
        breakpoint: Option<u16>,
        enable_soft_break: bool,
        sample_rate: u32,
    ) -> Self {
        let skip_boot = boot_rom.is_none();
        let mut gb = Self {
            cpu: Cpu::with_breakpoint(breakpoint, enable_soft_break),
            bus: Bus::new(8 * 1024, cartridge, sample_rate, boot_rom),
        };
        if skip_boot {
            gb.cpu.skip_boot(gb.bus.header_checksum());
            gb.bus.skip_boot();
        }
        gb
    }

    pub fn step(&mut self, frame_sink: &mut dyn FrameSink, audio_sink: &mut dyn AudioSink) -> u64 {
        let cycles = self.cpu.step(&mut self.bus);
        debug_assert!(
            cycles.is_multiple_of(4),
            "{cycles} cycles is not a whole number of M-cycles"
        );
        for _ in 0..cycles / 4 {
            self.bus.cycle(4, frame_sink, audio_sink);
            self.cpu.handle_interrupt(&mut self.bus);
        }

        cycles as u64
    }

    pub fn dump_cpu(&self) {
        self.cpu.dump_cpu();
    }

    pub fn dump_mem(&self, addr: u16) {
        for offset in 0..4 {
            let addr = addr + offset * 16;
            print!("{:04x}: ", addr);
            for a in addr..addr + 16 {
                print!("{:02x} ", self.bus.read_byte(a));
            }
            println!();
        }
    }

    pub fn disassemble(&self, addr: u16) {
        let bytes = (addr..addr + 100)
            .map(|a| self.bus.read_byte(a))
            .collect::<Vec<_>>();
        let instrs = Disassembler::new(&bytes).run();
        let mut pc = addr;
        for inst in instrs {
            println!("{pc:04X}\t{inst}");
            pc += inst.bytes;
        }
    }

    pub fn dump_oam(&self) {
        self.bus.gfx.dump_oam();
    }

    pub fn dump_sprite(&self, id: u8) {
        self.bus.gfx.dump_sprite(id);
    }

    pub fn dump_palettes(&self) {
        self.bus.gfx.dump_palettes();
    }

    pub fn is_halted(&self) -> bool {
        self.cpu.halted()
    }

    pub fn is_paused(&self) -> bool {
        self.cpu.is_paused()
    }

    pub fn pause(&mut self) {
        self.cpu.set_pause(true);
        self.bus.gfx.disable();
    }

    pub fn resume(&mut self) {
        self.bus.gfx.enable();
        self.cpu.set_pause(false);
    }

    pub fn set_breakpoint(&mut self, addr: u16) {
        self.cpu.set_breakpoint(addr);
    }

    pub fn set_button_pressed(&mut self, button: Button, is_pressed: bool) {
        self.bus.set_button_pressed(button, is_pressed);
    }

    /// Power off the Game Boy and pull out its cartridge, as it would be on a fresh power-on.
    /// External RAM keeps both its contents and its heap allocation.
    pub fn eject(self) -> Cartridge {
        let mut cartridge = self.bus.cartridge;
        cartridge.reset_mapper();
        cartridge
    }

    /// Set the colours used for the 4 DMG shades, from lightest to darkest.
    pub fn set_dmg_palette(&mut self, palette: [Rgb555; 4]) {
        self.bus.gfx.set_dmg_palette(palette);
    }

    pub fn save_ram(&self) -> Option<&[u8]> {
        self.bus.cartridge.save_ram()
    }

    pub fn save_ram_mut(&mut self) -> Option<&mut [u8]> {
        self.bus.cartridge.save_ram_mut()
    }

    pub fn has_rtc(&self) -> bool {
        self.bus.cartridge.has_rtc()
    }

    /// See [`Cartridge::save_rtc`].
    pub fn save_rtc(&self, unix_time: u64) -> Option<[u8; RTC_SAVE_SIZE]> {
        self.bus.cartridge.save_rtc(unix_time)
    }

    /// See [`Cartridge::load_rtc`].
    pub fn load_rtc(&mut self, data: &[u8], unix_time: u64) -> anyhow::Result<()> {
        self.bus.cartridge.load_rtc(data, unix_time)
    }

    pub fn poke(&mut self, addr: u16, value: u8) {
        self.bus.write_byte(addr, value);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    struct NullSink;

    impl FrameSink for NullSink {
        fn push_frame(&mut self, _frame: &[Rgb555]) {}
    }

    impl AudioSink for NullSink {
        fn push_sample(&mut self, _sample: (f32, f32)) -> bool {
            false
        }
    }

    /// A minimal ROM-only cartridge that passes the boot ROM's logo and header checks.
    fn test_cartridge(boot_rom: &[u8], title: &[u8]) -> Cartridge {
        let mut rom = vec![0; 0x8000];
        // The boot ROM holds its own copy of the logo to compare the cartridge's against.
        rom[0x0104..0x0134].copy_from_slice(&boot_rom[0xA8..0xD8]);
        rom[0x0134..0x0134 + title.len()].copy_from_slice(title);
        rom[0x014D] = rom[0x0134..0x014D]
            .iter()
            .fold(0u8, |x, b| x.wrapping_sub(*b).wrapping_sub(1));
        Cartridge::load_bytes(rom).unwrap()
    }

    /// The state reached by skipping the boot ROM should match the one after actually running
    /// it. This needs a boot ROM, so it's skipped if there isn't one.
    #[test]
    fn skip_boot_matches_boot_rom() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/dmg_boot.bin");
        let Ok(boot_rom) = std::fs::read(&path) else {
            eprintln!("No boot ROM at {}, skipping", path.display());
            return;
        };

        // Some replacement boot ROMs hardcode the flags as if both nibbles of the header checksum
        // were non-zero, so stick to such a checksum to be able to compare against them too.
        let cart = || test_cartridge(&boot_rom, b"TEST");
        let mut booted = GameBoy::new(
            cart(),
            Some(BootRom::load_bytes(boot_rom.clone()).unwrap()),
            None,
            false,
            48_000,
        );
        let mut skipped = GameBoy::new(cart(), None, None, false, 48_000);

        let mut cycles = 0;
        while booted.cpu.snapshot().0[5] != 0x0100 {
            cycles += booted.step(&mut NullSink, &mut NullSink);
            assert!(cycles < 100_000_000, "boot ROM never reached 0x0100");
        }

        assert_eq!(booted.cpu.snapshot(), skipped.cpu.snapshot());

        // LY, STAT and DIV depend on exactly how long the boot takes, so skip them.
        let io = (0xFF00..=0xFF4B).chain([0xFFFF]);
        for addr in io.filter(|a| ![0xFF04, 0xFF41, 0xFF44].contains(a)) {
            assert_eq!(
                booted.bus.read_byte(addr),
                skipped.bus.read_byte(addr),
                "register {addr:04X}"
            );
        }

        // Turn the LCD off so the PPU doesn't lock the CPU out of VRAM.
        booted.bus.gfx.disable();
        skipped.bus.gfx.disable();
        for addr in 0x8000..=0x9FFF {
            assert_eq!(
                booted.bus.read_byte(addr),
                skipped.bus.read_byte(addr),
                "VRAM {addr:04X}"
            );
        }
    }
}
