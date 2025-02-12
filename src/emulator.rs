use std::{
    collections::VecDeque,
    fs::File,
    io::BufWriter,
    path::Path,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use log::{debug, info};

use gb_rs::{
    cartridge::Cartridge, gameboy::GameBoy, joypad::Button, AudioSink, FrameSink, SCREEN_HEIGHT,
    SCREEN_WIDTH,
};
use ringbuf::{producer::Producer, storage::Heap, wrap::caching::Caching, SharedRb};
use winit::{
    event::KeyEvent,
    keyboard::{KeyCode, PhysicalKey},
};

use crate::debugger::{Command, Debugger};

// 4.194304MHZ -> 4194304 cycles per seconds
// const CPU_CYCLE_PER_SEC: u64 = 4194304;
// 1/4194304 seconds per cycle -> 238 nanoseconds per cycle
const CPU_CYCLE_TIME_NS: u64 = 238;
pub type ProducerF32 = Caching<Arc<SharedRb<Heap<f32>>>, true, false>;

/// The object that pulls everything together and drives the emulation engine while interfacing
/// with actual input/outputs.
pub struct Emulator {
    gb: GameBoy,
    start_time_ns: Instant,
    emulated_cycles: u64,
    debugger: Debugger,
    sink: MostRecentFrameSink,
    audio_sink: CpalAudioSink,
}

impl Emulator {
    pub fn new(
        rom: impl AsRef<Path>,
        producer: ProducerF32,
        breakpoint: Option<u16>,
        enable_soft_break: bool,
    ) -> Result<Self> {
        let cartridge = Cartridge::load(rom)?;
        info!("Title is {}", cartridge.title());
        info!("Licensee code is {}", cartridge.licensee_code());
        info!("Cartridge type is {}", cartridge.cartridge_type());
        info!("ROM size is ${:02x}", cartridge.get_rom_size());
        info!("RAM size is ${:02x}", cartridge.get_ram_size());
        info!("CGB flag: {}", cartridge.cgb_flag());
        info!("SGB flag: {}", cartridge.sgb_flag());
        let gb = GameBoy::new(cartridge, breakpoint, enable_soft_break);

        Ok(Self {
            gb,
            start_time_ns: Instant::now(),
            emulated_cycles: 0,
            debugger: Debugger::new()?,
            sink: MostRecentFrameSink::default(),
            audio_sink: CpalAudioSink::new(producer),
        })
    }

    pub fn start_debugger(&mut self) {
        self.gb.pause();
    }

    pub fn render(&mut self, buf: &mut [u8]) {
        self.sink.draw_current_frame(buf);
    }

    pub fn update(&mut self) -> bool {
        let target_time_ns = self.start_time_ns.elapsed();
        let target_cycles = target_time_ns.as_nanos() as u64 / CPU_CYCLE_TIME_NS;

        if self.gb.is_paused() {
            match self.debugger.debug() {
                Command::Next(n) => {
                    for _ in 0..n {
                        self.emulated_cycles += self.gb.step(&mut self.sink, &mut self.audio_sink);
                    }
                    self.gb.dump_cpu();
                }
                Command::Continue => {
                    // Reset start time
                    self.start_time_ns = Instant::now()
                        - Duration::from_nanos(self.emulated_cycles * CPU_CYCLE_TIME_NS);
                    self.gb.resume();
                }
                Command::DumpMem(addr) => self.gb.dump_mem(addr),
                Command::Disassemble(addr) => self.gb.disassemble(addr),
                Command::DumpCpu => self.gb.dump_cpu(),
                Command::DumpOam => self.gb.dump_oam(),
                Command::DumpPalettes => self.gb.dump_palettes(),
                Command::Break(addr) => self.gb.set_breakpoint(addr),
                Command::Sprite(id) => self.gb.dump_sprite(id),
                Command::Quit => return true,
                Command::Nop => (),
            }
        } else {
            while self.emulated_cycles < target_cycles && !self.gb.is_paused() {
                self.emulated_cycles += self.gb.step(&mut self.sink, &mut self.audio_sink);
            }
        }

        false
    }

    pub fn finish(&mut self) {
        self.gb.save();
    }

    pub fn screenshot(&mut self) -> Result<()> {
        let filename = format!(
            "gb-rs-screenshot_{}.png",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()
        );
        let path = Path::new(&filename);
        let file = File::create(path)?;
        let mut w = BufWriter::new(file);

        let mut encoder = png::Encoder::new(&mut w, SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;

        let mut data = [0u8; SCREEN_WIDTH * SCREEN_HEIGHT * 4];
        self.sink.draw_current_frame(&mut data);
        writer.write_image_data(&data)?;
        println!("Saved screenshot to {}", filename);
        Ok(())
    }

    pub fn handle_input(&mut self, key: KeyEvent) {
        // Ignore repeats
        if key.repeat {
            return;
        }
        // Ignore keys that don't have a key code
        let PhysicalKey::Code(keycode) = key.physical_key else {
            return;
        };

        match keycode {
            KeyCode::Enter => self
                .gb
                .set_button_pressed(Button::Start, key.state.is_pressed()),
            KeyCode::Space => self
                .gb
                .set_button_pressed(Button::Select, key.state.is_pressed()),
            KeyCode::KeyA => self
                .gb
                .set_button_pressed(Button::A, key.state.is_pressed()),
            KeyCode::KeyB => self
                .gb
                .set_button_pressed(Button::B, key.state.is_pressed()),
            KeyCode::ArrowUp => self
                .gb
                .set_button_pressed(Button::Up, key.state.is_pressed()),
            KeyCode::ArrowDown => self
                .gb
                .set_button_pressed(Button::Down, key.state.is_pressed()),
            KeyCode::ArrowLeft => self
                .gb
                .set_button_pressed(Button::Left, key.state.is_pressed()),
            KeyCode::ArrowRight => self
                .gb
                .set_button_pressed(Button::Right, key.state.is_pressed()),
            _ => (),
        }
    }
    // pub fn handle_input(&mut self, input: &WinitInputHelper) {
    //     self.gb
    //         .set_button_pressed(Button::Start, input.key_held(KeyCode::Enter));
    //     self.gb
    //         .set_button_pressed(Button::Select, input.key_held(KeyCode::Space));
    //     self.gb
    //         .set_button_pressed(Button::A, input.key_held(KeyCode::KeyA));
    //     self.gb
    //         .set_button_pressed(Button::B, input.key_held(KeyCode::KeyB));
    //     self.gb
    //         .set_button_pressed(Button::Up, input.key_held(KeyCode::ArrowUp));
    //     self.gb
    //         .set_button_pressed(Button::Down, input.key_held(KeyCode::ArrowDown));
    //     self.gb
    //         .set_button_pressed(Button::Left, input.key_held(KeyCode::ArrowLeft));
    //     self.gb
    //         .set_button_pressed(Button::Right, input.key_held(KeyCode::ArrowRight));
    // }
}

/// Frame sink that only keeps the most recent frame
struct MostRecentFrameSink {
    buf: [(u8, u8, u8); SCREEN_WIDTH * SCREEN_HEIGHT],
    new_frame: bool,
}

impl MostRecentFrameSink {
    pub fn new() -> Self {
        Self {
            buf: [(0, 0, 0); SCREEN_WIDTH * SCREEN_HEIGHT],
            new_frame: true,
        }
    }

    fn draw_current_frame(&mut self, frame: &mut [u8]) {
        self.buf
            .iter()
            .zip(frame.chunks_mut(4))
            .for_each(|((r, g, b), p)| {
                p[0] = *r;
                p[1] = *g;
                p[2] = *b;
                p[3] = 255;
            });
        self.new_frame = false;
    }
}

impl Default for MostRecentFrameSink {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSink for MostRecentFrameSink {
    fn push_frame(&mut self, frame: &[(u8, u8, u8)]) {
        self.buf.copy_from_slice(frame);
        self.new_frame = true;
    }
}

struct CpalAudioSink {
    buffer: ProducerF32,
    master_volume: f32,
}

impl CpalAudioSink {
    fn new(buffer: ProducerF32) -> Self {
        Self {
            buffer,
            master_volume: 1.0,
        }
    }
}

impl AudioSink for CpalAudioSink {
    fn push_sample(&mut self, sample: (f32, f32)) -> bool {
        if self.buffer.try_push(sample.0 * self.master_volume).is_err()
            || self.buffer.try_push(sample.1 * self.master_volume).is_err()
        {
            debug!("Buffer overrun!");
            return true;
        }

        false
    }

    fn push_samples(&mut self, samples: &mut VecDeque<f32>) {
        let mut iter = samples.iter().map(|v| *v * self.master_volume);
        let n = self.buffer.push_iter(&mut iter);
        samples.drain(0..n);
    }
}
