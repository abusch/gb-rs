//! Run a ROM without any frontend, as fast as possible, for benchmarking and profiling.
//!
//! Usage: `cargo run --release --example headless -- <ROM> [FRAMES] [PNG]`
//!
//! Prints how fast the emulation ran compared to real time, plus hashes of the video and audio
//! output so that optimisations can be checked for changes in behaviour. With a PNG path, also
//! saves the last frame there, e.g. to look at what a change in the hashes is about.

use std::{fs::File, hash::Hasher, io::BufWriter, path::Path, time::Instant};

use anyhow::{Context, Result};
use gbrs::{
    AudioSink, CPU_HZ, CYCLES_PER_FRAME, FrameSink, Rgb555, SCREEN_HEIGHT, SCREEN_WIDTH,
    cartridge::Cartridge, gameboy::GameBoy,
};

const SAMPLE_RATE: u32 = 48_000;

#[derive(Default)]
struct HashingSink {
    frames: u64,
    samples: u64,
    video: std::hash::DefaultHasher,
    audio: std::hash::DefaultHasher,
    /// A copy of the last frame, if it's wanted (copying it costs a little speed).
    last_frame: Option<Vec<Rgb555>>,
}

impl FrameSink for HashingSink {
    fn push_frame(&mut self, frame: &[Rgb555]) {
        self.frames += 1;
        for pixel in frame {
            self.video.write_u16(pixel.0);
        }
        if let Some(last_frame) = &mut self.last_frame {
            last_frame.clear();
            last_frame.extend_from_slice(frame);
        }
    }
}

impl AudioSink for HashingSink {
    fn push_sample(&mut self, (left, right): (f32, f32)) {
        self.samples += 1;
        self.audio.write_u32(left.to_bits());
        self.audio.write_u32(right.to_bits());
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let rom = args
        .next()
        .context("Usage: headless <ROM> [FRAMES] [PNG]")?;
    let frames: u64 = args
        .next()
        .map(|s| s.parse())
        .transpose()
        .context("Invalid frame count")?
        .unwrap_or(3600);
    let png = args.next();

    let content = std::fs::read(&rom).context("Failed to read ROM")?;
    let cartridge = Cartridge::load_bytes(content)?;
    let mut gb = GameBoy::new(cartridge, None, SAMPLE_RATE);

    let mut frame_sink = HashingSink {
        last_frame: png.is_some().then(Vec::new),
        ..Default::default()
    };
    let mut audio_sink = HashingSink::default();
    let target_cycles = frames * CYCLES_PER_FRAME;
    let mut cycles = 0;
    let start = Instant::now();
    while cycles < target_cycles {
        cycles += gb.step(&mut frame_sink, &mut audio_sink);
    }
    let elapsed = start.elapsed();

    let emulated = cycles as f64 / CPU_HZ as f64;
    println!(
        "{frames} frames in {:.3}s: {:.1} fps, {:.2}x real time",
        elapsed.as_secs_f64(),
        frames as f64 / elapsed.as_secs_f64(),
        emulated / elapsed.as_secs_f64()
    );
    println!(
        "video: {} frames, hash {:016x}",
        frame_sink.frames,
        frame_sink.video.finish()
    );
    println!(
        "audio: {} samples, hash {:016x}",
        audio_sink.samples,
        audio_sink.audio.finish()
    );
    if let (Some(png), Some(frame)) = (png, &frame_sink.last_frame) {
        save_png(Path::new(&png), frame)?;
    }
    Ok(())
}

fn save_png(path: &Path, frame: &[Rgb555]) -> Result<()> {
    let file =
        File::create(path).with_context(|| format!("Failed to create {}", path.display()))?;
    let mut encoder = png::Encoder::new(
        BufWriter::new(file),
        SCREEN_WIDTH as u32,
        SCREEN_HEIGHT as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = frame
        .iter()
        .flat_map(|pixel| {
            let (r, g, b) = pixel.to_rgb888();
            [r, g, b]
        })
        .collect();
    encoder.write_header()?.write_image_data(&data)?;
    Ok(())
}
