use std::{
    fs::File,
    io::BufWriter,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use log::{debug, info};

use gb_rs::{
    cartridge::Cartridge, gameboy::GameBoy, joypad::Button, AudioSink, FrameSink, SCREEN_HEIGHT,
    SCREEN_WIDTH,
};
use ringbuf::Producer;
use winit::event::VirtualKeyCode;
use winit_input_helper::WinitInputHelper;

use crate::debugger::{Command, Debugger};

// 4.194304MHZ -> 4194304 cycles per seconds
// const CPU_CYCLE_PER_SEC: u64 = 4194304;
// 1/4194304 seconds per cycle -> 238 nanoseconds per cycle
const CPU_CYCLE_TIME_NS: u64 = 238;

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
    pub fn new(producer: Producer<f32>) -> Result<Self> {
        let file = std::env::args().nth(1).context("Unable to find ROM file")?;

        let cartridge = Cartridge::load(file)?;
        info!("Title is {}", cartridge.title());
        info!("Licensee code is {}", cartridge.licensee_code());
        info!("Cartridge type is {}", cartridge.cartridge_type());
        info!("ROM size is ${:02x}", cartridge.get_rom_size());
        info!("RAM size is ${:02x}", cartridge.get_ram_size());
        info!("CGB flag: {}", cartridge.cgb_flag());
        info!("SGB flag: {}", cartridge.sgb_flag());
        let gb = GameBoy::new(cartridge);

        Ok(Self {
            gb,
            start_time_ns: Instant::now(),
            emulated_cycles: 0,
            debugger: Debugger::new(),
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

    pub fn handle_input(&mut self, input: &WinitInputHelper) {
        self.gb
            .set_button_pressed(Button::Start, input.key_held(VirtualKeyCode::Return));
        self.gb
            .set_button_pressed(Button::Select, input.key_held(VirtualKeyCode::Space));
        self.gb
            .set_button_pressed(Button::A, input.key_held(VirtualKeyCode::A));
        self.gb
            .set_button_pressed(Button::B, input.key_held(VirtualKeyCode::B));
        self.gb
            .set_button_pressed(Button::Up, input.key_held(VirtualKeyCode::Up));
        self.gb
            .set_button_pressed(Button::Down, input.key_held(VirtualKeyCode::Down));
        self.gb
            .set_button_pressed(Button::Left, input.key_held(VirtualKeyCode::Left));
        self.gb
            .set_button_pressed(Button::Right, input.key_held(VirtualKeyCode::Right));
    }
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
    buffer: Producer<f32>,
}

impl CpalAudioSink {
    fn new(buffer: Producer<f32>) -> Self {
        Self { buffer }
    }
}

impl AudioSink for CpalAudioSink {
    fn push_sample(&mut self, sample: (f32, f32)) {
        if self.buffer.push(sample.0).is_err() || self.buffer.push(sample.1).is_err() {
            debug!("Buffer overrun!");
        }
    }
}
