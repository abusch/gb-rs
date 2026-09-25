use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use log::{info, warn};

use gbrs::{
    AudioSink, BootRom, FrameSink, Rgb555, SCREEN_HEIGHT, SCREEN_WIDTH, cartridge::Cartridge,
    gameboy::GameBoy, joypad::Button,
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

/// Read a ROM image from `path`, which is either a raw ROM or a ZIP archive containing one.
fn read_rom(path: &Path) -> Result<Vec<u8>> {
    let mut file = BufReader::new(File::open(path).context("Failed to open rom file")?);
    let mut content = Vec::new();
    if path.extension().is_some_and(|ext| ext == "zip") {
        let mut zip = zip::ZipArchive::new(file).context("Failed to open zip archive")?;
        let file_name = zip
            .file_names()
            .find(|&name| name.ends_with(".gb"))
            .context("No ROM found in ZIP file")?
            .to_owned();
        let mut rom = zip
            .by_name(&file_name)
            .context("Failed to read ROM from ZIP file")?;
        rom.read_to_end(&mut content)
            .context("Failed to read rom file")?;
    } else {
        file.read_to_end(&mut content)
            .context("Failed to read rom file")?;
    };
    info!("Loaded {} bytes from rom file", content.len());
    Ok(content)
}

/// Current time in seconds since the Unix epoch, which the RTC's save data is stamped with.
fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Restore the cartridge's battery-backed RAM, and RTC if it has one, from `save_file`.
///
/// Like BGB and VBA-M, the RTC state is stored after the RAM, so their save files can be used.
fn load_save_file(gb: &mut GameBoy, save_file: &Path) -> Result<()> {
    let ram_len = gb.save_ram().map_or(0, <[u8]>::len);
    if ram_len == 0 && !gb.has_rtc() {
        return Ok(());
    }
    if !save_file.exists() {
        info!("No RAM file found.");
        return Ok(());
    }
    let content = fs::read(save_file).context("Failed to load RAM file")?;
    let (ram, rtc) = content.split_at(ram_len.min(content.len()));
    if ram.len() != ram_len || (!rtc.is_empty() && !gb.has_rtc()) {
        warn!(
            "RAM file {} has size {}, expected {}. Ignoring...",
            save_file.display(),
            content.len(),
            ram_len
        );
        return Ok(());
    }
    info!("Loading RAM file {}...", save_file.display());
    if let Some(save_ram) = gb.save_ram_mut() {
        save_ram.copy_from_slice(ram);
    }
    // Save files from emulators without RTC support just leave the clock at 0.
    if !rtc.is_empty()
        && let Err(e) = gb.load_rtc(rtc, unix_time())
    {
        warn!("Ignoring RTC state in {}: {e}", save_file.display());
    }
    Ok(())
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
    /// Where the cartridge's battery-backed RAM is persisted.
    save_file: PathBuf,
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
        boot_rom: Option<BootRom>,
        producer: ProducerF32,
        audio_stats: Arc<AudioStats>,
        breakpoint: Option<u16>,
        enable_soft_break: bool,
        sample_rate: u32,
    ) -> Result<Self> {
        let rom = rom.as_ref();
        let cartridge =
            Cartridge::load_bytes(read_rom(rom)?).context("Failed to load cartridge from bytes")?;
        info!("Title is {}", cartridge.title());
        info!("Licensee code is {}", cartridge.licensee_code());
        info!("Cartridge type is {}", cartridge.cartridge_type());
        info!("ROM size is ${:02x}", cartridge.get_rom_size());
        info!("RAM size is ${:02x}", cartridge.get_ram_size());
        info!("CGB flag: {}", cartridge.cgb_flag());
        info!("SGB flag: {}", cartridge.sgb_flag());
        let mut gb = GameBoy::new(
            cartridge,
            boot_rom,
            breakpoint,
            enable_soft_break,
            sample_rate,
        );
        let save_file = rom.with_extension("sav");
        load_save_file(&mut gb, &save_file)?;

        let now = Instant::now();
        Ok(Self {
            gb,
            save_file,
            start_time_ns: now,
            emulated_cycles: 0,
            debugger: Debugger::new()?,
            sink: MostRecentFrameSink::default(),
            audio_sink: CpalAudioSink::new(producer, Arc::clone(&audio_stats)),
            audio_stats,
            last_stats_log: now,
        })
    }

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
                Command::Poke(addr, val) => self.gb.poke(addr, val),
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
        let mut content = self.gb.save_ram().unwrap_or_default().to_vec();
        if let Some(rtc) = self.gb.save_rtc(unix_time()) {
            content.extend_from_slice(&rtc);
        }
        if !content.is_empty()
            && let Err(e) = fs::write(&self.save_file, content)
        {
            warn!(
                "Failed to save RAM file {}: {}",
                self.save_file.display(),
                e
            );
        }
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
            KeyCode::KeyD => self.start_debugger(),
            _ => (),
        }
    }
}

/// Frame sink that only keeps the most recent frame
struct MostRecentFrameSink {
    buf: [Rgb555; SCREEN_WIDTH * SCREEN_HEIGHT],
    new_frame: bool,
}

impl MostRecentFrameSink {
    pub fn new() -> Self {
        Self {
            buf: [Rgb555::default(); SCREEN_WIDTH * SCREEN_HEIGHT],
            new_frame: true,
        }
    }

    fn draw_current_frame(&mut self, frame: &mut [u8]) {
        self.buf
            .iter()
            .zip(frame.chunks_mut(4))
            .for_each(|(color, p)| {
                let (r, g, b) = color.to_rgb888();
                p.copy_from_slice(&[r, g, b, 255]);
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
    fn push_frame(&mut self, frame: &[Rgb555]) {
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
