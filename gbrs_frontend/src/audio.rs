//! Audio output: the emulator pushes samples into a ring buffer, which cpal drains on its own
//! thread.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use cpal::{
    BufferSize, Device, Stream, StreamConfig, SupportedStreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use gbrs::AudioSink;
use log::{debug, error, info, trace};
use ringbuf::{
    SharedRb,
    producer::Producer,
    storage::Heap,
    traits::{Consumer, Observer},
    wrap::caching::Caching,
};

pub type ProducerF32 = Caching<Arc<SharedRb<Heap<f32>>>, true, false>;

#[derive(Debug, Default)]
pub struct AudioStats {
    pub underrun_count: AtomicU64,
    pub producer_drop_count: AtomicU64,
}

/// Feeds the emulator's samples to the ring buffer, interleaved.
pub struct CpalAudioSink {
    buffer: ProducerF32,
    stats: Arc<AudioStats>,
}

impl CpalAudioSink {
    pub fn new(buffer: ProducerF32, stats: Arc<AudioStats>) -> Self {
        Self { buffer, stats }
    }

    /// Number of f32 values in the ring buffer.
    pub fn fill_level(&self) -> usize {
        self.buffer.occupied_len()
    }

    pub fn stats(&self) -> &AudioStats {
        &self.stats
    }
}

impl AudioSink for CpalAudioSink {
    fn push_sample(&mut self, (left, right): (f32, f32)) {
        if self.buffer.try_push(left).is_err() || self.buffer.try_push(right).is_err() {
            self.stats
                .producer_drop_count
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Pick the default output device, and a config with the given sample rate or else its default
/// one.
pub fn negotiate_audio(forced_rate: Option<u32>) -> Result<(Device, SupportedStreamConfig)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("no default output device available")?;
    debug!("Audio device: {:?}", device.description());
    let supported_cfg = match forced_rate {
        None => device
            .default_output_config()
            .context("failed to query default output config")?,
        Some(rate) => device
            .supported_output_configs()
            .context("failed to enumerate output configs")?
            .find_map(|range| range.try_with_sample_rate(rate))
            .with_context(|| format!("device does not support sample rate {rate} Hz"))?,
    };
    Ok((device, supported_cfg))
}

/// Start playing the samples from `consumer`. Playback stops when the returned stream is dropped.
pub fn init_audio(
    device: Device,
    supported_cfg: SupportedStreamConfig,
    mut consumer: impl Consumer<Item = f32> + Send + 'static,
    stats: Arc<AudioStats>,
) -> Result<Stream> {
    let config = StreamConfig {
        channels: 2,
        sample_rate: supported_cfg.sample_rate(),
        buffer_size: BufferSize::Fixed(2048),
    };
    let err_fn = |err| {
        error!("Error writing to audio stream: {}", err);
    };
    let mut fade = Fade::default();
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                trace!("Writing {} audio samples", data.len());
                let mut underrun_frames: u64 = 0;
                for frame in data.as_chunks_mut::<2>().0 {
                    let popped = if consumer.occupied_len() >= 2 {
                        Some((consumer.try_pop().unwrap(), consumer.try_pop().unwrap()))
                    } else {
                        underrun_frames += 1;
                        None
                    };
                    let (l, r) = fade.process_frame(popped);
                    *frame = [l, r];
                }
                if underrun_frames > 0 {
                    stats
                        .underrun_count
                        .fetch_add(underrun_frames, Ordering::Relaxed);
                }
            },
            err_fn,
            None,
        )
        .context("Failed to build output stream")?;
    stream.play().context("Failed to start stream")?;
    info!("Audio stream started!");

    Ok(stream)
}

/// Without audio output, keep emptying the ring buffer so that the emulator never stalls on it.
pub fn init_no_audio(mut consumer: impl Consumer<Item = f32> + Send + 'static) {
    thread::spawn(move || {
        loop {
            consumer.clear();
            thread::sleep(Duration::from_millis(5));
        }
    });
}

/// Ramp length (in output frames) for fade-out / fade-in on underrun boundaries.
/// 132 frames is ~3 ms at 44.1 kHz, ~2.75 ms at 48 kHz — short enough for either rate.
const RAMP_FRAMES: u32 = 132;

/// Hides underruns: when the ring buffer runs dry, the last sample fades out rather than cutting
/// to silence, and fades back in when samples come back, so there are no pops.
struct Fade {
    last: (f32, f32),
    /// Position on the ramp, from 0 (silent) to `RAMP_FRAMES` (full volume).
    level: u32,
}

impl Default for Fade {
    fn default() -> Self {
        Self {
            last: (0.0, 0.0),
            level: RAMP_FRAMES,
        }
    }
}

impl Fade {
    fn process_frame(&mut self, popped: Option<(f32, f32)>) -> (f32, f32) {
        match popped {
            Some(sample) => {
                self.last = sample;
                self.level = (self.level + 1).min(RAMP_FRAMES);
            }
            None => self.level = self.level.saturating_sub(1),
        }
        let gain = self.level as f32 / RAMP_FRAMES as f32;
        (self.last.0 * gain, self.last.1 * gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A constant-amplitude sample: easy to reason about envelope shape.
    const A: (f32, f32) = (0.5, 0.5);

    #[test]
    fn passthrough_without_underruns() {
        let mut fade = Fade::default();
        for _ in 0..5 {
            assert_eq!(fade.process_frame(Some(A)), A);
        }
    }

    #[test]
    fn underrun_fades_to_silence() {
        let mut fade = Fade::default();
        fade.process_frame(Some(A));

        // The envelope goes down monotonically, and reaches 0 after `RAMP_FRAMES` frames.
        let mut prev = A.0;
        for _ in 0..RAMP_FRAMES {
            let (l, _) = fade.process_frame(None);
            assert!(l <= prev, "expected monotone fade-out: {l} > {prev}");
            prev = l;
        }
        assert_eq!(prev, 0.0);
        // Subsequent underruns are pure silence.
        for _ in 0..10 {
            assert_eq!(fade.process_frame(None), (0.0, 0.0));
        }
    }

    #[test]
    fn recovery_from_silence_fades_in() {
        let mut fade = Fade::default();
        fade.process_frame(Some(A));
        for _ in 0..RAMP_FRAMES + 5 {
            fade.process_frame(None);
        }

        // The envelope rises monotonically back to full volume.
        let mut prev = 0.0;
        for _ in 0..RAMP_FRAMES {
            let (l, _) = fade.process_frame(Some(A));
            assert!(l >= prev, "expected monotone fade-in: {l} < {prev}");
            prev = l;
        }
        assert_eq!(prev, A.0);
        assert_eq!(fade.process_frame(Some(A)), A);
    }

    #[test]
    fn underrun_during_fade_in_reverses_envelope() {
        let mut fade = Fade::default();
        fade.process_frame(Some(A));
        for _ in 0..RAMP_FRAMES {
            fade.process_frame(None);
        }
        for _ in 0..RAMP_FRAMES / 2 {
            fade.process_frame(Some(A));
        }
        let (peak, _) = fade.process_frame(Some(A));
        let (next, _) = fade.process_frame(None);
        assert!(
            next < peak,
            "fade-out should start below peak: {next} >= {peak}"
        );
    }

    #[test]
    fn no_pop_when_starting_in_silence() {
        let mut fade = Fade::default();
        for _ in 0..RAMP_FRAMES * 2 {
            assert_eq!(fade.process_frame(None), (0.0, 0.0));
        }
    }
}
