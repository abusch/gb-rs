use std::path::Path;

use gbrs::{
    AudioSink, BootRom, FrameSink, Rgb555, SCREEN_HEIGHT, SCREEN_WIDTH, cartridge::Cartridge,
    gameboy::GameBoy, joypad::Button,
};
use libretro::{
    ContentContract, ControllerDescription, ControllerDevice, ControllerInfo, Core, CoreMemory,
    Environment, GameInfo, InputPort, JoypadButton, MemoryRegion, PixelFormat, Runtime, SystemInfo,
    fixed_system_av_info,
};

const AUDIO_SAMPLE_RATE: f64 = 48000.0;

/// Optional boot ROM, looked up in the frontend's system directory.
const BOOT_ROM_FILE: &str = "dmg_boot.bin";

/// 4.194304 MHz CPU clock.
const CPU_HZ: u64 = 4_194_304;
/// 154 scanlines of 456 dots each.
const CYCLES_PER_FRAME: u64 = 154 * 456;
/// ~59.73 Hz: the DMG does not run at exactly 60 fps.
const FPS: f64 = CPU_HZ as f64 / CYCLES_PER_FRAME as f64;

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
        }
    }
}

impl GbrsCore {
    fn power_on(&mut self, cartridge: Cartridge) {
        self.emulator = Some(GameBoy::new(
            cartridge,
            self.boot_rom.clone(),
            None,
            false,
            AUDIO_SAMPLE_RATE as u32,
        ));
        self.frame = RetroFrameSink::default();
        self.audio.samples.clear();
        self.cycle_carry = 0;
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
    }

    fn set_controller_port_device(&mut self, port: InputPort, _device: ControllerDevice) {
        self.port = port;
    }

    fn load_game(&mut self, game: Option<GameInfo<'_>>, runtime: &mut Runtime<'_>) -> bool {
        // The default libretro format is 0RGB1555; ask for 32-bit so we can pass colours through.
        if !runtime
            .environment()
            .set_pixel_format(PixelFormat::Xrgb8888)
        {
            return false;
        }

        let Some(data) = game.and_then(|game| game.data) else {
            return false;
        };
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
                true
            }
            Err(_) => false,
        }
    }

    fn unload_game(&mut self) {
        self.emulator = None;
    }

    fn reset(&mut self) {
        // Reuse the cartridge rather than reloading it: battery-backed RAM must survive a reset,
        // and the frontend may still hold the pointer we handed out for `SaveRam`.
        if let Some(gb) = self.emulator.take() {
            self.power_on(gb.eject());
        }
    }

    fn run(&mut self, runtime: &mut Runtime<'_>) {
        runtime.poll_input();

        let Some(gb) = &mut self.emulator else {
            return;
        };

        let buttons = runtime.joypad_buttons(self.port);
        for (retro_button, gb_button) in BUTTON_MAP {
            gb.set_button_pressed(gb_button, buttons.contains(retro_button));
        }

        // Run exactly one frame's worth of cycles, carrying any overshoot into the next frame so
        // the long-term rate stays locked to `FPS`. We count cycles rather than waiting for a
        // pushed frame because the PPU doesn't push frames while the LCD is off.
        let mut cycles = self.cycle_carry;
        while cycles < CYCLES_PER_FRAME {
            cycles += gb.step(&mut self.frame, &mut self.audio);
        }
        self.cycle_carry = cycles - CYCLES_PER_FRAME;

        // Always submit the last complete frame, even if the LCD was off this frame.
        let _ = runtime.video_refresh_frame_with_audio(
            &self.frame.buf[..],
            SCREEN_WIDTH as u32,
            SCREEN_HEIGHT as u32,
            SCREEN_WIDTH * std::mem::size_of::<u32>(),
            &self.audio.samples,
        );
        self.audio.samples.clear();
    }

    fn memory_region(&mut self, region: MemoryRegion) -> Option<CoreMemory<'_>> {
        match region {
            // Exposing cart RAM lets the frontend handle `.srm` load/save for us.
            MemoryRegion::SaveRam => self
                .emulator
                .as_mut()?
                .save_ram_mut()
                .map(CoreMemory::read_write),
            _ => None,
        }
    }
}

/// Keeps the most recent frame in XRGB8888 format.
struct RetroFrameSink {
    buf: Box<[u32]>,
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
            let (r, g, b) = color.to_rgb888();
            *dst = u32::from_be_bytes([0, r, g, b]);
        }
    }
}

/// Accumulates one frame's worth of stereo samples, converted to i16.
#[derive(Default)]
struct RetroAudioSink {
    samples: Vec<[i16; 2]>,
}

impl AudioSink for RetroAudioSink {
    fn push_sample(&mut self, (left, right): (f32, f32)) -> bool {
        let to_i16 = |s: f32| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        self.samples.push([to_i16(left), to_i16(right)]);
        false
    }
}

libretro::export_core!(GbrsCore::default());
