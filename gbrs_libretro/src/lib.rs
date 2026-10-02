use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use gbrs::{
    AudioSink, BootRom, CPU_HZ, CYCLES_PER_FRAME, DMG_PALETTES, FrameSink, Rgb555, SCREEN_HEIGHT,
    SCREEN_WIDTH,
    cartridge::{Cartridge, RTC_SAVE_SIZE},
    gameboy::GameBoy,
    joypad::Button,
};
use libretro::{
    ContentContract, ControllerDescription, ControllerDevice, ControllerInfo, Core, CoreMemory,
    CoreOptionDefinition, CoreOptionValue, CoreOptions, Environment, GameInfo, InputPort,
    JoypadButton, MemoryDescriptorFlag, MemoryMapDescriptor, MemoryRegion, PixelFormat, Runtime,
    SystemInfo, fixed_system_av_info,
};

const AUDIO_SAMPLE_RATE: f64 = 48000.0;

/// Optional boot ROM, looked up in the frontend's system directory.
const BOOT_ROM_FILE: &str = "dmg_boot.bin";

/// Core option picking one of `DMG_PALETTES` by name.
const PALETTE_OPTION: &str = "gbrs_palette";

/// ~59.73 Hz: the DMG does not run at exactly 60 fps.
const FPS: f64 = CPU_HZ as f64 / CYCLES_PER_FRAME as f64;

/// Room left in save states for their size to vary. The emulator's state is almost fixed-size, but
/// small integers are encoded in fewer bytes, and the size libretro asks for can't grow while a
/// game is loaded. In practice, it varies by about 10 bytes.
const SAVE_STATE_SLACK: usize = 1024;

/// Where RetroAchievements expects the cartridge RAM banks after the first one (see the Game Boy's
/// memory regions in rcheevos' `consoleinfo.c`), since only one is mapped at 0xA000.
const EXTRA_CART_RAM_BANKS: usize = 0x16000;
const CART_RAM_BANK_SIZE: usize = 0x2000;

const BUTTON_MAP: [(JoypadButton, Button); 8] = [
    (JoypadButton::Up, Button::Up),
    (JoypadButton::Down, Button::Down),
    (JoypadButton::Left, Button::Left),
    (JoypadButton::Right, Button::Right),
    (JoypadButton::A, Button::A),
    (JoypadButton::B, Button::B),
    (JoypadButton::Select, Button::Select),
    (JoypadButton::Start, Button::Start),
];

struct GbrsCore {
    content_contract: ContentContract,
    emulator: Option<GameBoy>,
    boot_rom: Option<BootRom>,
    port: InputPort,
    frame: RetroFrameSink,
    audio: RetroAudioSink,
    /// Cycles the last instruction of the previous frame overshot `CYCLES_PER_FRAME` by.
    cycle_carry: u64,
    rtc: RtcRegion,
    /// What `serialize_size` returns, fixed when the game gets loaded.
    save_state_size: usize,
    /// Colours for the DMG shades, from the palette core option.
    palette: [Rgb555; 4],
}

impl Default for GbrsCore {
    fn default() -> Self {
        Self {
            content_contract: ContentContract::new("gb|dmg").with_support_no_game(false),
            emulator: None,
            boot_rom: None,
            port: InputPort::default(),
            frame: RetroFrameSink::default(),
            audio: RetroAudioSink::default(),
            cycle_carry: 0,
            rtc: RtcRegion::default(),
            save_state_size: 0,
            palette: DMG_PALETTES[0].colors,
        }
    }
}

impl GbrsCore {
    fn power_on(&mut self, cartridge: Cartridge) {
        let mut gb = GameBoy::new(cartridge, self.boot_rom.clone(), AUDIO_SAMPLE_RATE as u32);
        gb.set_dmg_palette(self.palette);
        self.emulator = Some(gb);
        self.restart_output();
    }

    /// Start over from a Game Boy that was just turned on.
    fn restart_output(&mut self) {
        self.frame = RetroFrameSink::default();
        self.audio.samples.clear();
        self.cycle_carry = 0;
        if let Some(gb) = &mut self.emulator {
            self.rtc.sync(gb);
        }
    }

    /// Apply the core options' current values.
    fn read_options(&mut self, env: &mut Environment<'_>) {
        // An unknown palette (e.g. from a newer version) leaves the current one.
        if let Some(palette) = env
            .get_variable(PALETTE_OPTION)
            .and_then(|name| DMG_PALETTES.iter().find(|p| p.name == name))
        {
            self.palette = palette.colors;
            if let Some(gb) = &mut self.emulator {
                gb.set_dmg_palette(self.palette);
            }
        }
    }

    /// Fix what `serialize_size` returns for the game that was just loaded.
    fn set_save_state_size(&mut self) {
        // `cycle_carry` goes first in save states.
        self.save_state_size = self.emulator.as_ref().map_or(0, |gb| {
            size_of::<u64>() + gb.save_state().len() + SAVE_STATE_SLACK
        });
    }
}

impl Core for GbrsCore {
    fn system_info(&self) -> SystemInfo {
        let mut sys_info = SystemInfo::new("GBRS Core", env!("CARGO_PKG_VERSION"));
        self.content_contract.apply_to_system_info(&mut sys_info);
        sys_info
    }

    fn av_info(&self) -> libretro::SystemAvInfo {
        fixed_system_av_info(
            SCREEN_WIDTH as u32,
            SCREEN_HEIGHT as u32,
            FPS,
            AUDIO_SAMPLE_RATE,
        )
    }

    fn on_set_environment(&mut self, env: &mut Environment<'_>) {
        let _ = self.content_contract.register_environment(env);
        let controllers = [ControllerInfo::new([ControllerDescription::new(
            "GameBoy pad",
            ControllerDevice::Joypad,
        )])];
        let _ = env.set_controller_info(&controllers);
        let palette = CoreOptionDefinition::new(PALETTE_OPTION, "Palette", DMG_PALETTES[0].name)
            .with_info("Colours of the four shades of the Game Boy's screen.")
            .with_values(
                DMG_PALETTES
                    .iter()
                    .map(|p| CoreOptionValue::new(p.name).with_label(p.label)),
            );
        let _ = env.set_core_options(&CoreOptions::new([palette]));
    }

    fn set_controller_port_device(&mut self, port: InputPort, _device: ControllerDevice) {
        self.port = port;
    }

    fn load_game(&mut self, game: Option<GameInfo<'_>>, runtime: &mut Runtime<'_>) -> bool {
        // RGB565 rather than XRGB8888: frames are half the size, which matters on handhelds, and
        // 16 bits are enough for the Game Boy's 15-bit colours.
        if !runtime.environment().set_pixel_format(PixelFormat::Rgb565) {
            return false;
        }

        let Some(data) = game.and_then(|game| game.data) else {
            return false;
        };
        self.read_options(&mut runtime.environment());
        // Without a (valid) boot ROM, the game just starts straight away.
        self.boot_rom = runtime
            .environment()
            .system_directory()
            .and_then(|dir| std::fs::read(Path::new(&dir).join(BOOT_ROM_FILE)).ok())
            .and_then(|content| BootRom::load_bytes(content).ok());
        // No save file: the frontend loads/saves `.srm` through `memory_region(SaveRam)`.
        match Cartridge::load_bytes(data.to_vec()) {
            Ok(cartridge) => {
                self.power_on(cartridge);
                self.set_save_state_size();
                if let Some(gb) = &mut self.emulator {
                    set_memory_maps(gb, &mut runtime.environment());
                }
                runtime.environment().set_support_achievements(true);
                true
            }
            Err(_) => false,
        }
    }

    fn unload_game(&mut self) {
        self.emulator = None;
        self.save_state_size = 0;
    }

    fn reset(&mut self) {
        // Reset in place rather than reloading the cartridge: battery-backed RAM must survive a
        // reset, and the frontend holds pointers to the memory (`SaveRam`, the memory maps).
        if let Some(gb) = &mut self.emulator {
            gb.reset(self.boot_rom.clone());
            self.restart_output();
        }
    }

    fn run(&mut self, runtime: &mut Runtime<'_>) {
        runtime.poll_input();
        if runtime.environment().variables_updated() {
            self.read_options(&mut runtime.environment());
        }

        let Some(gb) = &mut self.emulator else {
            return;
        };

        self.rtc.sync(gb);

        let buttons = runtime.joypad_buttons(self.port);
        for (retro_button, gb_button) in BUTTON_MAP {
            gb.set_button_pressed(gb_button, buttons.contains(retro_button));
        }

        // Run exactly one frame's worth of cycles, carrying any overshoot into the next frame so
        // the long-term rate stays locked to `FPS`. We count cycles rather than waiting for a
        // pushed frame because the PPU doesn't push frames while the LCD is off.
        let cycles = self.cycle_carry
            + gb.run(
                CYCLES_PER_FRAME - self.cycle_carry,
                &mut self.frame,
                &mut self.audio,
            );
        self.cycle_carry = cycles.saturating_sub(CYCLES_PER_FRAME);

        // Always submit the last complete frame, even if the LCD was off this frame.
        let _ = runtime.video_refresh_frame_with_audio(
            &self.frame.buf[..],
            SCREEN_WIDTH as u32,
            SCREEN_HEIGHT as u32,
            SCREEN_WIDTH * std::mem::size_of::<u16>(),
            &self.audio.samples,
        );
        self.audio.samples.clear();
    }

    fn serialize_size(&self) -> usize {
        self.save_state_size
    }

    fn serialize(&self, data: &mut [u8]) -> bool {
        let Some(gb) = &self.emulator else {
            return false;
        };
        // The frame's overshoot has to be restored too, so that replaying from a state (e.g. for
        // rewind or run-ahead) runs the exact same cycles.
        let state = gb.save_state();
        let Some((carry, rest)) = data.split_first_chunk_mut::<8>() else {
            return false;
        };
        let Some((state_data, padding)) = rest.split_at_mut_checked(state.len()) else {
            return false;
        };
        *carry = self.cycle_carry.to_le_bytes();
        state_data.copy_from_slice(&state);
        padding.fill(0);
        true
    }

    fn unserialize(&mut self, data: &[u8]) -> bool {
        let Some(gb) = &mut self.emulator else {
            return false;
        };
        let Some((carry, state)) = data.split_first_chunk::<8>() else {
            return false;
        };
        if gb.load_state(state).is_err() {
            return false;
        }
        self.cycle_carry = u64::from_le_bytes(*carry);
        true
    }

    fn memory_region(&mut self, region: MemoryRegion) -> Option<CoreMemory<'_>> {
        match region {
            // Exposing cart RAM lets the frontend handle `.srm` load/save for us.
            MemoryRegion::SaveRam => self
                .emulator
                .as_mut()?
                .save_ram_mut()
                .map(CoreMemory::read_write),
            // Same for the `.rtc` file, which the frontend keeps separately.
            MemoryRegion::Rtc if self.emulator.as_ref()?.has_rtc() => {
                Some(CoreMemory::read_write(&mut self.rtc.buf[..]))
            }
            _ => None,
        }
    }
}

/// Tell the frontend where the Game Boy's memory is, at the addresses RetroAchievements uses. It reads
/// it through these pointers for as long as the game is loaded, which `GameBoy` allows, since its
/// memory stays put even when loading a save state or resetting.
///
/// The ROM isn't mapped, as only its first bank stays in place, nor the I/O registers, which
/// aren't kept in memory.
fn set_memory_maps(gb: &mut GameBoy, env: &mut Environment<'_>) -> bool {
    let memory = gb.memory_mut();
    let first_bank_size = memory.cart_ram.len().min(CART_RAM_BANK_SIZE);
    let (cart_ram, extra_cart_ram) = memory.cart_ram.split_at_mut(first_bank_size);
    let mut descriptors = vec![
        MemoryMapDescriptor::from_slice(None, 0x8000, memory.vram)
            .with_flags(MemoryDescriptorFlag::VideoRam.into()),
        MemoryMapDescriptor::from_slice(None, 0xC000, memory.wram)
            .with_flags(MemoryDescriptorFlag::SystemRam.into()),
        MemoryMapDescriptor::from_slice(None, 0xFE00, memory.oam)
            .with_flags(MemoryDescriptorFlag::VideoRam.into()),
        MemoryMapDescriptor::from_slice(None, 0xFF80, memory.hram)
            .with_flags(MemoryDescriptorFlag::SystemRam.into()),
    ];
    for (start, ram) in [(0xA000, cart_ram), (EXTRA_CART_RAM_BANKS, extra_cart_ram)] {
        if !ram.is_empty() {
            descriptors.push(
                MemoryMapDescriptor::from_slice(None, start, ram)
                    .with_flags(MemoryDescriptorFlag::SaveRam.into()),
            );
        }
    }
    env.set_memory_maps(&descriptors)
}

/// The cartridge's real-time clock, as exposed to the frontend through `MemoryRegion::Rtc`.
///
/// The frontend reads this buffer to save it, and writes a saved clock into it after loading the
/// game, whenever it likes. So it's kept up to date every frame, and when it holds something else
/// than what was last written to it, the frontend has restored a save.
struct RtcRegion {
    /// Boxed so it stays put while the frontend holds a pointer to it.
    buf: Box<[u8; RTC_SAVE_SIZE]>,
    /// What was last written to `buf`.
    written: [u8; RTC_SAVE_SIZE],
}

impl Default for RtcRegion {
    fn default() -> Self {
        Self {
            buf: Box::new([0; RTC_SAVE_SIZE]),
            written: [0; RTC_SAVE_SIZE],
        }
    }
}

impl RtcRegion {
    fn sync(&mut self, gb: &mut GameBoy) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if *self.buf != self.written {
            // If the save is invalid, keep the clock as it is: it gets written over just below.
            let _ = gb.load_rtc(&self.buf[..], now);
        }
        if let Some(rtc) = gb.save_rtc(now) {
            *self.buf = rtc;
            self.written = rtc;
        }
    }
}

/// Keeps the most recent frame in RGB565 format.
struct RetroFrameSink {
    buf: Box<[u16]>,
}

impl Default for RetroFrameSink {
    fn default() -> Self {
        Self {
            buf: vec![0; SCREEN_WIDTH * SCREEN_HEIGHT].into_boxed_slice(),
        }
    }
}

impl FrameSink for RetroFrameSink {
    fn push_frame(&mut self, frame: &[Rgb555]) {
        for (dst, color) in self.buf.iter_mut().zip(frame) {
            *dst = color.to_rgb565();
        }
    }
}

/// Accumulates one frame's worth of stereo samples, converted to i16.
#[derive(Default)]
struct RetroAudioSink {
    samples: Vec<[i16; 2]>,
}

impl AudioSink for RetroAudioSink {
    fn push_sample(&mut self, (left, right): (f32, f32)) {
        let to_i16 = |s: f32| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        self.samples.push([to_i16(left), to_i16(right)]);
    }
}

libretro::export_core!(GbrsCore::default());

#[cfg(test)]
mod tests {
    use super::*;

    fn game_boy(cartridge_type: u8) -> GameBoy {
        let mut rom = vec![0; 0x8000];
        rom[0x0147] = cartridge_type;
        GameBoy::new(Cartridge::load_bytes(rom).unwrap(), None, 48_000)
    }

    fn unix_time() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn test_serialize() {
        let mut core = GbrsCore::default();
        core.power_on(game_boy(0x03).eject());
        core.set_save_state_size();
        core.cycle_carry = 12;

        let mut data = vec![0xAA; core.serialize_size()];
        assert!(core.serialize(&mut data));
        let state = core.emulator.as_ref().unwrap().save_state();
        assert_eq!(data[..8], 12u64.to_le_bytes());
        assert_eq!(data[8..8 + state.len()], state);
        assert!(data[8 + state.len()..].iter().all(|b| *b == 0));

        core.cycle_carry = 0;
        assert!(core.unserialize(&data));
        assert_eq!(core.cycle_carry, 12);
        assert_eq!(core.emulator.as_ref().unwrap().save_state(), state);

        // Buffers that are too small, and data that isn't a save state, are rejected.
        assert!(!core.serialize(&mut data[..state.len()]));
        assert!(!core.serialize(&mut data[..4]));
        assert!(!core.unserialize(&data[..4]));
        assert!(!core.unserialize(&[0; 100]));
        assert_eq!(core.cycle_carry, 12);
    }

    #[test]
    fn test_rtc_region() {
        let mut rtc = RtcRegion::default();
        // Without an RTC, the buffer is left alone
        rtc.sync(&mut game_boy(0x00));
        assert_eq!(*rtc.buf, [0; RTC_SAVE_SIZE]);

        // MBC3+TIMER+BATTERY
        let mut gb = game_boy(0x0F);
        rtc.sync(&mut gb);
        assert_eq!(rtc.buf[..40], gb.save_rtc(0).unwrap()[..40]);

        // The frontend restores a clock saved at 05:00:00 an hour ago
        let mut saved = [0; RTC_SAVE_SIZE];
        saved[8] = 5;
        saved[40..].copy_from_slice(&(unix_time() - 3600).to_le_bytes());
        *rtc.buf = saved;
        rtc.sync(&mut gb);
        let state = gb.save_rtc(0).unwrap();
        assert_eq!(state[8], 6, "hours in {state:?}");
        assert_eq!(state[4], 0, "minutes in {state:?}");
        // and it's written back for the next save
        assert_eq!(rtc.buf[..40], state[..40]);
        assert_ne!(*rtc.buf, saved);
    }
}
