use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::bus::{BootRom, Bus, CpuBus};
use crate::cartridge::{Cartridge, RTC_SAVE_SIZE};
use crate::cpu::Cpu;
use crate::disasm::Disassembler;
use crate::joypad::Button;
use crate::{AudioSink, FrameSink, Rgb555};

/// Identifies gb-rs save states.
const SAVE_STATE_MAGIC: [u8; 4] = *b"GBRS";
/// Bump this whenever the shape of the emulator's state changes (e.g. a field is added to one of
/// the components), since save states are just the serialised structs.
const SAVE_STATE_VERSION: u16 = 10;

/// The Game Boy's RAM, for frontends that read (or write) it directly, e.g. for RetroAchievements.
///
/// These are the buffers the emulator uses, as they are: VRAM and OAM can be accessed whatever the
/// PPU is doing. They stay where they are for as long as the [`GameBoy`] exists, even when a save
/// state is loaded or it's reset, so frontends can keep pointers to them.
pub struct Memory<'a> {
    /// 0x8000-0x9FFF
    pub vram: &'a mut [u8],
    /// The cartridge's external RAM, if it has any: all its banks, the first of which can be
    /// mapped at 0xA000-0xBFFF. MBC2's is 512 bytes holding a half-byte each.
    pub cart_ram: &'a mut [u8],
    /// 0xC000-0xDFFF
    pub wram: &'a mut [u8],
    /// 0xFE00-0xFE9F
    pub oam: &'a mut [u8],
    /// 0xFF80-0xFFFE
    pub hram: &'a mut [u8],
}

/// Comes first in save states, to reject ones that can't be loaded.
#[derive(Serialize, Deserialize)]
struct SaveStateHeader {
    magic: [u8; 4],
    version: u16,
    /// See [`Cartridge::checksums`].
    rom_checksums: [u8; 3],
}

/// The CPU's registers, e.g. for test ROMs that report their results in them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuRegisters {
    pub af: u16,
    pub bc: u16,
    pub de: u16,
    pub hl: u16,
    pub sp: u16,
    pub pc: u16,
}

#[derive(Serialize, Deserialize)]
pub struct GameBoy {
    cpu: Cpu,
    bus: Bus,
}

impl GameBoy {
    /// Power on a Game Boy with the given cartridge inserted.
    ///
    /// With a boot ROM, emulation starts by running it. Without one, it starts directly at the
    /// cartridge's entry point, in the state the boot ROM would have left the hardware in.
    pub fn new(cartridge: Cartridge, boot_rom: Option<BootRom>, sample_rate: u32) -> Self {
        let skip_boot = boot_rom.is_none();
        let mut gb = Self {
            cpu: Cpu::default(),
            bus: Bus::new(cartridge, sample_rate, boot_rom),
        };
        if skip_boot {
            gb.cpu.skip_boot(gb.bus.header_checksum());
            gb.bus.skip_boot();
        }
        gb
    }

    /// Run one CPU instruction (or one M-cycle while halted), then the interrupt dispatch if one
    /// is due. Return the number of clock cycles that took.
    pub fn step(&mut self, frame_sink: &mut dyn FrameSink, audio_sink: &mut dyn AudioSink) -> u64 {
        let mut bus = CpuBus::new(&mut self.bus, frame_sink, audio_sink);
        let cycles = self.cpu.step(&mut bus);
        // The rest of the hardware runs during each of the instruction's memory accesses. What's
        // left is the internal M-cycles at the end of the instruction, which don't access memory.
        debug_assert!(
            cycles.is_multiple_of(4) && bus.cycles() <= cycles,
            "{cycles} cycles for an instruction that took {}",
            bus.cycles()
        );
        while bus.cycles() < cycles {
            bus.tick();
        }
        self.cpu.handle_interrupt(&mut bus);

        bus.cycles() as u64
    }

    /// Run for at least `cycles` clock cycles, stopping early if the CPU gets paused (e.g. by a
    /// breakpoint). Return the number of cycles run.
    ///
    /// This is the same as calling `step` until then, but faster: while the CPU is halted, the rest
    /// of the hardware runs up to its next event in one go, rather than an M-cycle at a time.
    pub fn run(
        &mut self,
        cycles: u64,
        frame_sink: &mut dyn FrameSink,
        audio_sink: &mut dyn AudioSink,
    ) -> u64 {
        let mut ran = 0;
        while ran < cycles && !self.cpu.is_paused() {
            let skipped = self.skip_halted(cycles - ran, frame_sink, audio_sink);
            ran += if skipped > 0 {
                skipped
            } else {
                self.step(frame_sink, audio_sink)
            };
        }
        ran
    }

    /// While the CPU is halted with nothing to wake it up, run the rest of the hardware for as
    /// many M-cycles as it can take in one go (see `Bus::skip_idle`), and at most `max_cycles`
    /// rounded up to an M-cycle, then handle the interrupt if one came up in the last one. That's
    /// what stepping the halted CPU for those M-cycles would do. Return the cycles run, or 0 if the
    /// CPU has to be stepped.
    fn skip_halted(
        &mut self,
        max_cycles: u64,
        frame_sink: &mut dyn FrameSink,
        audio_sink: &mut dyn AudioSink,
    ) -> u64 {
        if !self.cpu.idle_while_halted() {
            return 0;
        }
        let max_cycles = max_cycles.next_multiple_of(4).min(u64::from(u16::MAX)) as u16;
        let skipped = self.bus.skip_idle(max_cycles, frame_sink, audio_sink);
        if skipped == 0 {
            return 0;
        }
        let mut bus = CpuBus::new(&mut self.bus, frame_sink, audio_sink);
        self.cpu.handle_interrupt(&mut bus);
        u64::from(skipped) + bus.cycles() as u64
    }

    pub fn dump_cpu(&self) {
        self.cpu.dump_cpu();
    }

    pub fn registers(&self) -> CpuRegisters {
        let ([af, bc, de, hl, sp, pc], _) = self.cpu.snapshot();
        CpuRegisters {
            af,
            bc,
            de,
            hl,
            sp,
            pc,
        }
    }

    /// Read a byte as the CPU would, without side effects.
    pub fn peek(&self, addr: u16) -> u8 {
        self.bus.read_byte(addr)
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
        self.bus.gfx.set_debugger_access(true);
    }

    pub fn resume(&mut self) {
        self.bus.gfx.set_debugger_access(false);
        self.cpu.set_pause(false);
    }

    pub fn set_breakpoint(&mut self, addr: u16) {
        self.cpu.set_breakpoint(addr);
    }

    /// Make the `LD B,B` instruction pause emulation like a breakpoint, as some test ROMs expect.
    pub fn set_soft_break(&mut self, enabled: bool) {
        self.cpu.set_soft_break(enabled);
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

    /// Turn the Game Boy off and on again with the same cartridge, running `boot_rom` if given.
    ///
    /// This is like [`GameBoy::eject`] followed by [`GameBoy::new`], except that the memory stays
    /// where it is (see [`Memory`]), and so do the settings that aren't part of save states (such as
    /// the DMG palette).
    pub fn reset(&mut self, boot_rom: Option<BootRom>) {
        let mut cartridge = std::mem::replace(&mut self.bus.cartridge, Cartridge::empty());
        cartridge.reset_mapper();
        let mut gb = Self::new(cartridge, boot_rom, self.bus.sample_rate());
        gb.bus.restore_peripherals(&mut self.bus);
        gb.cpu.restore_unsaved(&self.cpu);
        *self = gb;
    }

    /// The Game Boy's RAM, see [`Memory`].
    pub fn memory_mut(&mut self) -> Memory<'_> {
        self.bus.memory_mut()
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

    /// Snapshot the state of the whole Game Boy, except for the ROM and the frontend's settings.
    pub fn save_state(&self) -> Vec<u8> {
        let header = SaveStateHeader {
            magic: SAVE_STATE_MAGIC,
            version: SAVE_STATE_VERSION,
            rom_checksums: self.bus.cartridge.checksums(),
        };
        let data = postcard::to_stdvec(&header).expect("Save state header should serialise");
        postcard::to_extend(self, data).expect("Game Boy state should serialise")
    }

    /// Restore a snapshot from [`GameBoy::save_state`]. Trailing bytes are ignored, since some
    /// frontends pad save states. The memory stays where it is (see [`Memory`]).
    ///
    /// On error, the Game Boy is left as it was.
    pub fn load_state(&mut self, data: &[u8]) -> Result<()> {
        let (header, data) =
            postcard::take_from_bytes::<SaveStateHeader>(data).context("Not a gb-rs save state")?;
        ensure!(header.magic == SAVE_STATE_MAGIC, "Not a gb-rs save state");
        ensure!(
            header.version == SAVE_STATE_VERSION,
            "Save state has version {}, but only version {SAVE_STATE_VERSION} is supported",
            header.version
        );
        ensure!(
            header.rom_checksums == self.bus.cartridge.checksums(),
            "Save state is for a different game"
        );
        let (mut state, _) =
            postcard::take_from_bytes::<GameBoy>(data).context("Corrupted save state")?;
        state.bus.restore_unsaved(&mut self.bus)?;
        state.cpu.restore_unsaved(&self.cpu);
        *self = state;
        Ok(())
    }

    pub fn poke(&mut self, addr: u16, value: u8) {
        self.bus.write_byte(addr, value);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::CYCLES_PER_FRAME;

    /// Hashes the video and audio output.
    #[derive(Default)]
    struct HashSink(std::hash::DefaultHasher);

    impl FrameSink for HashSink {
        fn push_frame(&mut self, frame: &[Rgb555]) {
            for pixel in frame {
                std::hash::Hasher::write_u16(&mut self.0, pixel.0);
            }
        }
    }

    impl AudioSink for HashSink {
        fn push_sample(&mut self, (left, right): (f32, f32)) {
            std::hash::Hasher::write_u32(&mut self.0, left.to_bits());
            std::hash::Hasher::write_u32(&mut self.0, right.to_bits());
        }
    }

    /// An MBC1+RAM cart that keeps scrolling the screen, so that its output changes every frame.
    fn scrolling_cartridge(checksum: u8) -> Cartridge {
        let mut rom = vec![0; 0x8000];
        // loop: INC A; LDH (SCX),A; JR loop
        rom[0x0100..0x0105].copy_from_slice(&[0x3C, 0xE0, 0x43, 0x18, 0xFB]);
        rom[0x0147] = 0x03;
        rom[0x0149] = 0x02;
        rom[0x014D] = checksum;
        Cartridge::load_bytes(rom).unwrap()
    }

    /// Run for `frames` frames, returning a hash of the output.
    fn run_frames(gb: &mut GameBoy, frames: u64) -> u64 {
        let mut sink = HashSink::default();
        let mut audio = HashSink::default();
        let mut cycles = 0;
        while cycles < frames * CYCLES_PER_FRAME {
            cycles += gb.step(&mut sink, &mut audio);
        }
        std::hash::Hasher::finish(&sink.0) ^ std::hash::Hasher::finish(&audio.0)
    }

    #[test]
    fn test_save_state_replays() {
        // A custom palette isn't part of the state, so the output only matches if it's kept.
        let palette = [Rgb555(1), Rgb555(2), Rgb555(3), Rgb555(4)];
        let mut gb = GameBoy::new(scrolling_cartridge(0), None, 48_000);
        gb.set_dmg_palette(palette);
        run_frames(&mut gb, 30);
        let state = gb.save_state();
        let expected = run_frames(&mut gb, 60);

        gb.load_state(&state).unwrap();
        assert_eq!(run_frames(&mut gb, 60), expected);

        // Also after a restart, and with padding at the end
        let mut fresh = GameBoy::new(scrolling_cartridge(0), None, 48_000);
        fresh.set_dmg_palette(palette);
        let mut padded = state.clone();
        padded.resize(state.len() + 100, 0);
        fresh.load_state(&padded).unwrap();
        assert_eq!(run_frames(&mut fresh, 60), expected);

        // Saving a loaded state gives it back as it was
        fresh.load_state(&state).unwrap();
        assert_eq!(fresh.save_state(), state);
    }

    #[test]
    fn test_invalid_save_states_are_rejected() {
        let mut gb = GameBoy::new(scrolling_cartridge(0), None, 48_000);
        run_frames(&mut gb, 10);
        let state = gb.save_state();
        run_frames(&mut gb, 10);
        let before = gb.save_state();

        let mut other_game = GameBoy::new(scrolling_cartridge(1), None, 48_000);
        let mut bad_magic = state.clone();
        bad_magic[0] = b'X';
        for (data, error) in [
            (other_game.save_state(), "different game"),
            (state[..state.len() / 2].to_vec(), "Corrupted"),
            (bad_magic, "Not a gb-rs save state"),
            (vec![], "Not a gb-rs save state"),
        ] {
            let result = gb.load_state(&data);
            assert!(
                result
                    .as_ref()
                    .is_err_and(|e| e.to_string().contains(error)),
                "expected {error:?}, got {result:?}"
            );
            // The Game Boy is left as it was.
            assert!(gb.save_state() == before);
        }
        // and the other way around
        assert!(other_game.load_state(&state).is_err());
    }

    fn memory_pointers(gb: &mut GameBoy) -> [*const u8; 5] {
        let memory = gb.memory_mut();
        [
            memory.vram,
            memory.cart_ram,
            memory.wram,
            memory.oam,
            memory.hram,
        ]
        .map(|buffer| buffer.as_ptr())
    }

    #[test]
    fn test_memory() {
        let mut gb = GameBoy::new(scrolling_cartridge(0), None, 48_000);
        // Enable the cartridge's RAM
        gb.poke(0x0000, 0x0A);
        for addr in [0xA123, 0xC123, 0xD123, 0xFF90] {
            gb.poke(addr, addr as u8);
        }
        let memory = gb.memory_mut();
        assert_eq!(
            (memory.vram.len(), memory.cart_ram.len(), memory.oam.len()),
            (0x2000, 0x2000, 0xA0)
        );
        assert_eq!(memory.cart_ram[0x123], 0x23);
        assert_eq!(memory.wram[0x0123], 0x23);
        assert_eq!(memory.wram[0x1123], 0x23);
        assert_eq!(memory.hram[0x10], 0x90);
        assert_eq!(memory.hram.len(), 0x7F);
    }

    /// Frontends may keep pointers to the memory, so it mustn't move.
    #[test]
    fn test_memory_stays_put() {
        let mut gb = GameBoy::new(scrolling_cartridge(0), None, 48_000);
        let pointers = memory_pointers(&mut gb);
        gb.memory_mut().wram[0] = 0x42;
        let state = gb.save_state();
        run_frames(&mut gb, 1);
        gb.memory_mut().wram[0] = 0;

        gb.load_state(&state).unwrap();
        assert_eq!(memory_pointers(&mut gb), pointers);
        assert_eq!(gb.memory_mut().wram[0], 0x42);

        gb.reset(None);
        assert_eq!(memory_pointers(&mut gb), pointers);
    }

    #[test]
    fn test_reset() {
        let palette = [Rgb555(1), Rgb555(2), Rgb555(3), Rgb555(4)];
        let mut gb = GameBoy::new(scrolling_cartridge(0), None, 48_000);
        gb.set_dmg_palette(palette);
        gb.poke(0x0000, 0x0A);
        gb.poke(0xA000, 0x42);
        run_frames(&mut gb, 10);
        gb.reset(None);

        // It's as if the Game Boy had just been turned on, with the cartridge's RAM as it was, and
        // the same palette.
        let mut cartridge = scrolling_cartridge(0);
        cartridge.save_ram_mut().unwrap()[0] = 0x42;
        let mut fresh = GameBoy::new(cartridge, None, 48_000);
        fresh.set_dmg_palette(palette);
        assert!(gb.save_state() == fresh.save_state());
        assert_eq!(run_frames(&mut gb, 10), run_frames(&mut fresh, 10));
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
            48_000,
        );
        let mut skipped = GameBoy::new(cart(), None, 48_000);

        let mut cycles = 0;
        while booted.cpu.snapshot().0[5] != 0x0100 {
            cycles += booted.step(&mut (), &mut ());
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

        // So that the PPU doesn't lock the CPU out of VRAM.
        booted.bus.gfx.set_debugger_access(true);
        skipped.bus.gfx.set_debugger_access(true);
        for addr in 0x8000..=0x9FFF {
            assert_eq!(
                booted.bus.read_byte(addr),
                skipped.bus.read_byte(addr),
                "VRAM {addr:04X}"
            );
        }
    }
}
