use std::{
    fs::File,
    io::BufWriter,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use log::info;

use gb_rs::{
    AudioSink, FrameSink, SCREEN_HEIGHT, SCREEN_WIDTH, cartridge::Cartridge, gameboy::GameBoy,
    joypad::Button,
};
use ringbuf::{
    SharedRb, producer::Producer, storage::Heap, traits::Observer, wrap::caching::Caching,
};
use winit::{
    event::KeyEvent,
    keyboard::{KeyCode, PhysicalKey},
};

use crate::debugger::{Command, Debugger};

// 4.194304 MHz CPU clock. We do not store a precomputed ns-per-cycle constant: at 238 ns
// it rounds the period down by 0.18%, which makes the emulator run ~78 samples/s faster
// than real time and steadily fills the audio ring buffer. Convert via full multiply/divide
// against `NS_PER_SEC` instead.
const CPU_HZ: u64 = 4_194_304;
const NS_PER_SEC: u64 = 1_000_000_000;

fn cycles_to_ns(cycles: u64) -> u64 {
    (cycles as u128 * NS_PER_SEC as u128 / CPU_HZ as u128) as u64
}
pub type ProducerF32 = Caching<Arc<SharedRb<Heap<f32>>>, true, false>;

const STATS_LOG_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub struct AudioStats {
    pub underrun_count: AtomicU64,
    pub producer_drop_count: AtomicU64,
    pub last_fill_level: AtomicU32,
}

/// The object that pulls everything together and drives the emulation engine while interfacing
/// with actual input/outputs.
pub struct Emulator {
    gb: GameBoy,
    start_time_ns: Instant,
    emulated_cycles: u64,
    debugger: Debugger,
    sink: MostRecentFrameSink,
    audio_sink: CpalAudioSink,
    audio_stats: Arc<AudioStats>,
    last_stats_log: Instant,
}

impl Emulator {
    pub fn new(
        rom: impl AsRef<Path>,
        producer: ProducerF32,
        audio_stats: Arc<AudioStats>,
        breakpoint: Option<u16>,
        enable_soft_break: bool,
        sample_rate: u32,
    ) -> Result<Self> {
        let cartridge = Cartridge::load(rom)?;
        info!("Title is {}", cartridge.title());
        info!("Licensee code is {}", cartridge.licensee_code());
        info!("Cartridge type is {}", cartridge.cartridge_type());
        info!("ROM size is ${:02x}", cartridge.get_rom_size());
        info!("RAM size is ${:02x}", cartridge.get_ram_size());
        info!("CGB flag: {}", cartridge.cgb_flag());
        info!("SGB flag: {}", cartridge.sgb_flag());
        let gb = GameBoy::new(cartridge, breakpoint, enable_soft_break, sample_rate);

        let now = Instant::now();
        Ok(Self {
            gb,
            start_time_ns: now,
            emulated_cycles: 0,
            debugger: Debugger::new()?,
            sink: MostRecentFrameSink::default(),
            audio_sink: CpalAudioSink::new(producer, Arc::clone(&audio_stats)),
            audio_stats,
            last_stats_log: now,
        })
    }

    #[allow(dead_code)]
    pub fn start_debugger(&mut self) {
        self.gb.pause();
    }

    pub fn render(&mut self, buf: &mut [u8]) {
        self.sink.draw_current_frame(buf);
    }

    pub fn update(&mut self) -> bool {
        self.log_audio_stats();

        let elapsed_ns = self.start_time_ns.elapsed().as_nanos() as u64;
        let target_cycles = elapsed_ns.saturating_mul(CPU_HZ) / NS_PER_SEC;

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
                    self.start_time_ns =
                        Instant::now() - Duration::from_nanos(cycles_to_ns(self.emulated_cycles));
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

    /// Re-anchor the wall-clock so `update()` doesn't try to "catch up" after a pause
    /// (warm-up, debugger, etc.).
    pub fn reset_clock(&mut self) {
        let now = Instant::now();
        self.start_time_ns = now - Duration::from_nanos(cycles_to_ns(self.emulated_cycles));
        self.last_stats_log = now;
    }

    /// Step the emulator (ignoring wall-clock pacing) until the audio ring buffer holds at
    /// least `target_fill` f32 samples, or `timeout` elapses. Used at startup to avoid the
    /// initial cpal underrun. Caller should `reset_clock()` afterwards.
    pub fn warm_up_audio(&mut self, target_fill: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.audio_sink.fill_level() < target_fill {
            if Instant::now() >= deadline {
                break;
            }
            self.emulated_cycles += self.gb.step(&mut self.sink, &mut self.audio_sink);
        }
    }

    fn log_audio_stats(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_stats_log) < STATS_LOG_INTERVAL {
            return;
        }
        self.last_stats_log = now;

        let fill = self.audio_sink.fill_level() as u32;
        self.audio_stats
            .last_fill_level
            .store(fill, Ordering::Relaxed);

        let underruns = self.audio_stats.underrun_count.swap(0, Ordering::Relaxed);
        let drops = self
            .audio_stats
            .producer_drop_count
            .swap(0, Ordering::Relaxed);

        if underruns > 0 || drops > 0 {
            info!("audio stats: underruns/s={underruns} producer_drops/s={drops} fill={fill}");
        }
    }

    #[allow(dead_code)]
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
    stats: Arc<AudioStats>,
}

impl CpalAudioSink {
    fn new(buffer: ProducerF32, stats: Arc<AudioStats>) -> Self {
        Self {
            buffer,
            master_volume: 1.0,
            stats,
        }
    }

    fn fill_level(&self) -> usize {
        self.buffer.occupied_len()
    }
}

impl AudioSink for CpalAudioSink {
    fn push_sample(&mut self, sample: (f32, f32)) -> bool {
        if self.buffer.try_push(sample.0 * self.master_volume).is_err()
            || self.buffer.try_push(sample.1 * self.master_volume).is_err()
        {
            self.stats
                .producer_drop_count
                .fetch_add(1, Ordering::Relaxed);
            return true;
        }

        false
    }
}
