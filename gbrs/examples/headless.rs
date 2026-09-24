//! Run a ROM without any frontend, as fast as possible, for benchmarking and profiling.
//!
//! Usage: `cargo run --release --example headless -- <ROM> [FRAMES]`
//!
//! Prints how fast the emulation ran compared to real time, plus hashes of the video and audio
//! output so that optimisations can be checked for changes in behaviour.

use std::{hash::Hasher, time::Instant};

use anyhow::{Context, Result};
use gbrs::{AudioSink, FrameSink, Rgb555, cartridge::Cartridge, gameboy::GameBoy};

/// Dots per frame: 154 scanlines of 456 dots each.
const CYCLES_PER_FRAME: u64 = 154 * 456;
const CPU_HZ: f64 = 4_194_304.0;
const SAMPLE_RATE: u32 = 48_000;

#[derive(Default)]
struct HashingSink {
    frames: u64,
    samples: u64,
    video: std::hash::DefaultHasher,
    audio: std::hash::DefaultHasher,
}

impl FrameSink for HashingSink {
    fn push_frame(&mut self, frame: &[Rgb555]) {
        self.frames += 1;
        for pixel in frame {
            self.video.write_u16(pixel.0);
        }
    }
}

impl AudioSink for HashingSink {
    fn push_sample(&mut self, (left, right): (f32, f32)) -> bool {
        self.samples += 1;
        self.audio.write_u32(left.to_bits());
        self.audio.write_u32(right.to_bits());
        true
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let rom = args.next().context("Usage: headless <ROM> [FRAMES]")?;
    let frames: u64 = args
        .next()
        .map(|s| s.parse())
        .transpose()
        .context("Invalid frame count")?
        .unwrap_or(3600);

    let content = std::fs::read(&rom).context("Failed to read ROM")?;
    let cartridge = Cartridge::load_bytes(content)?;
    let mut gb = GameBoy::new(cartridge, None, None, false, SAMPLE_RATE);

    let mut frame_sink = HashingSink::default();
    let mut audio_sink = HashingSink::default();
    let target_cycles = frames * CYCLES_PER_FRAME;
    let mut cycles = 0;
    let start = Instant::now();
    while cycles < target_cycles {
        cycles += gb.step(&mut frame_sink, &mut audio_sink);
    }
    let elapsed = start.elapsed();

    let emulated = cycles as f64 / CPU_HZ;
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
    Ok(())
}
