use std::any::Any;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use cpal::{
    BufferSize, Sample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use emulator::{AudioStats, Emulator};
use gb_rs::{SCREEN_HEIGHT, SCREEN_WIDTH};
use log::{debug, error, info, trace};
use pixels::{Pixels, SurfaceTexture};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Split},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::EventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowAttributes},
};

mod debugger;
mod emulator;

#[derive(Parser)]
#[command(about, version, author)]
pub struct Cli {
    /// Disable sound output
    #[arg(short, long)]
    quiet: bool,
    /// Set a breakpoint at the given address
    #[arg(short, long, value_parser = parse_addr)]
    breakpoint: Option<u16>,
    /// Enable software breakpoint
    ///
    /// If enabled, the `LD B,B` instruction triggers a breakpoint. Execution is paused and the
    /// debugger is started. This is useful for some test ROMS.
    #[arg(long)]
    enable_soft_break: bool,
    /// Path to the ROM file
    rom: PathBuf,
}

fn parse_addr(s: &str) -> Result<u16, ParseIntError> {
    u16::from_str_radix(s, 16)
}

fn main() -> Result<()> {
    // initialise logger
    env_logger::builder()
        .parse_filters("gb_rs=debug,gb_rs::apu=info")
        .init();

    let cli = Cli::parse();

    // Buffer can hold 0.5s of samples (assuming 2 channels)
    let ringbuf = HeapRb::<f32>::new(8102);
    let (producer, consumer) = ringbuf.split();
    let audio_stats = Arc::new(AudioStats::default());
    let mut emulator = Emulator::new(
        &cli.rom,
        producer,
        Arc::clone(&audio_stats),
        cli.breakpoint,
        cli.enable_soft_break,
    )?;

    // Pre-buffer ~30 ms of audio (1323 stereo frames = 2646 f32) before starting the
    // cpal stream, so the first callback finds samples ready instead of underrunning.
    if !cli.quiet {
        const WARMUP_F32: usize = 1323 * 2;
        const WARMUP_TIMEOUT: Duration = Duration::from_millis(200);
        emulator.warm_up_audio(WARMUP_F32, WARMUP_TIMEOUT);
        emulator.reset_clock();
    }

    let _guard: Box<dyn Any> = if cli.quiet {
        init_no_audio(consumer);
        Box::new(())
    } else {
        let stream = init_audio(consumer, Arc::clone(&audio_stats))?;
        Box::new(stream)
    };

    let mut app = App::new(emulator);
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;

    Ok(())
}

struct App {
    pixels: Option<Pixels<'static>>,
    window: Option<Arc<Window>>,
    emulator: Emulator,
}

impl App {
    pub fn new(emulator: Emulator) -> Self {
        Self {
            pixels: None,
            window: None,
            emulator,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.pixels.is_none() {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
            let size = LogicalSize::new(SCREEN_WIDTH as f64, SCREEN_HEIGHT as f64);
            let window = Arc::new(
                event_loop
                    .create_window(
                        WindowAttributes::default()
                            .with_title("gb-rs")
                            .with_inner_size(size)
                            .with_min_inner_size(size),
                    )
                    .expect("Failed to create window"),
            );
            let window_size = window.inner_size();
            let surface_texture =
                SurfaceTexture::new(window_size.width, window_size.height, window.clone());
            let pixels = Pixels::new(SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32, surface_texture)
                .expect("Failed to create Pixels");

            // kickoff rendering
            window.request_redraw();

            self.pixels = Some(pixels);
            self.window = Some(window);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exiting();
            }
            WindowEvent::Resized(size) => {
                if let Some(ref mut pixels) = self.pixels
                    && let Err(e) = pixels.resize_surface(size.width, size.height)
                {
                    error!("Error while rendering frame: {e}");
                    event_loop.exit();
                }
            }
            WindowEvent::KeyboardInput { event: k, .. } => {
                if k.logical_key == Key::Named(NamedKey::Escape) {
                    event_loop.exit();
                    self.emulator.finish();
                    return;
                }
                self.emulator.handle_input(k);
            }
            WindowEvent::RedrawRequested => {
                if let (Some(pixels), Some(window)) = (self.pixels.as_mut(), self.window.as_mut()) {
                    // Run the emiulator
                    if self.emulator.update() {
                        event_loop.exit();
                        self.emulator.finish();
                        return;
                    }
                    // Render a frame
                    self.emulator.render(pixels.frame_mut());
                    if let Err(e) = pixels.render() {
                        error!("Error while rendering frame: {}", e);
                        event_loop.exit();
                        self.emulator.finish();
                        return;
                    }
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn init_audio(
    mut consumer: impl Consumer<Item = f32> + Send + 'static,
    stats: Arc<AudioStats>,
) -> Result<Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("error while querying config")?;
    debug!("Audio device: {:?}", device.description());
    let config = StreamConfig {
        channels: 2,
        sample_rate: 44100,
        buffer_size: BufferSize::Fixed(2048),
    };
    let err_fn = |err| {
        error!("Error writing to audio stream: {}", err);
    };
    let mut playback = PlaybackState::new();
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                trace!("Writing {} audio samples", data.len());
                let mut underrun_frames: u64 = 0;
                for frame in data.chunks_exact_mut(2) {
                    let popped = if consumer.occupied_len() >= 2 {
                        let l = consumer.try_pop().unwrap().to_sample::<f32>();
                        let r = consumer.try_pop().unwrap().to_sample::<f32>();
                        Some((l, r))
                    } else {
                        underrun_frames += 1;
                        None
                    };
                    let (l, r) = playback.process_frame(popped);
                    frame[0] = l;
                    frame[1] = r;
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

/// ~3 ms at 44.1 kHz; ramp length for fade-out and fade-in on underrun boundaries.
const RAMP_FRAMES: u32 = 132;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayPhase {
    Normal,
    FadingOut,
    Silent,
    FadingIn,
}

struct PlaybackState {
    phase: PlayPhase,
    last_l: f32,
    last_r: f32,
    ramp_pos: u32,
}

impl PlaybackState {
    fn new() -> Self {
        Self {
            phase: PlayPhase::Normal,
            last_l: 0.0,
            last_r: 0.0,
            ramp_pos: 0,
        }
    }

    fn process_frame(&mut self, popped: Option<(f32, f32)>) -> (f32, f32) {
        match (self.phase, popped) {
            (PlayPhase::Normal, Some((l, r))) => {
                self.last_l = l;
                self.last_r = r;
                (l, r)
            }
            (PlayPhase::Normal, None) => {
                self.phase = PlayPhase::FadingOut;
                self.ramp_pos = 0;
                self.fade_out_step()
            }
            (PlayPhase::FadingOut, None) => self.fade_out_step(),
            (PlayPhase::FadingOut, Some((l, r))) => {
                // Underrun ended mid fade-out. Switch to fade-in at the matching envelope
                // value to keep the output continuous.
                self.last_l = l;
                self.last_r = r;
                self.phase = PlayPhase::FadingIn;
                self.ramp_pos = (RAMP_FRAMES + 1).saturating_sub(self.ramp_pos);
                self.fade_in_step()
            }
            (PlayPhase::Silent, None) => (0.0, 0.0),
            (PlayPhase::Silent, Some((l, r))) => {
                self.last_l = l;
                self.last_r = r;
                self.phase = PlayPhase::FadingIn;
                self.ramp_pos = 0;
                self.fade_in_step()
            }
            (PlayPhase::FadingIn, Some((l, r))) => {
                self.last_l = l;
                self.last_r = r;
                self.fade_in_step()
            }
            (PlayPhase::FadingIn, None) => {
                // Underrun resumed during fade-in. Ramp back down from the current envelope.
                self.phase = PlayPhase::FadingOut;
                self.ramp_pos = (RAMP_FRAMES + 1).saturating_sub(self.ramp_pos);
                self.fade_out_step()
            }
        }
    }

    fn fade_out_step(&mut self) -> (f32, f32) {
        let env = 1.0 - (self.ramp_pos as f32 / RAMP_FRAMES as f32);
        self.ramp_pos += 1;
        if self.ramp_pos >= RAMP_FRAMES {
            self.phase = PlayPhase::Silent;
            self.ramp_pos = 0;
        }
        (self.last_l * env, self.last_r * env)
    }

    fn fade_in_step(&mut self) -> (f32, f32) {
        let env = self.ramp_pos as f32 / RAMP_FRAMES as f32;
        self.ramp_pos += 1;
        if self.ramp_pos >= RAMP_FRAMES {
            self.phase = PlayPhase::Normal;
            self.ramp_pos = 0;
        }
        (self.last_l * env, self.last_r * env)
    }
}

fn init_no_audio(mut consumer: impl Consumer<Item = f32> + Send + 'static) {
    thread::spawn(move || {
        loop {
            // empty the ring buffer
            while consumer.try_pop().is_some() {
                // nop
            }
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A constant-amplitude sample: easy to reason about envelope shape.
    const A: (f32, f32) = (0.5, 0.5);

    fn run(state: &mut PlaybackState, frames: &[Option<(f32, f32)>]) -> Vec<(f32, f32)> {
        frames.iter().map(|f| state.process_frame(*f)).collect()
    }

    #[test]
    fn passthrough_in_normal_state() {
        let mut state = PlaybackState::new();
        let out = run(&mut state, &[Some(A); 5]);
        assert!(out.iter().all(|&s| s == A));
        assert_eq!(state.phase, PlayPhase::Normal);
    }

    #[test]
    fn underrun_triggers_monotone_fade_to_silence() {
        let mut state = PlaybackState::new();
        // Prime with one good sample so last_l/last_r are set.
        state.process_frame(Some(A));

        // Run RAMP_FRAMES underruns; expect monotone non-increasing envelope down to ~0.
        let mut prev = A.0 + 1.0; // sentinel above 1.0
        for _ in 0..RAMP_FRAMES {
            let (l, _) = state.process_frame(None);
            assert!(l <= prev, "expected monotone fade-out: {l} > {prev}");
            assert!((0.0..=A.0).contains(&l));
            prev = l;
        }
        assert_eq!(state.phase, PlayPhase::Silent);

        // Subsequent underruns are pure silence.
        for _ in 0..10 {
            assert_eq!(state.process_frame(None), (0.0, 0.0));
        }
    }

    #[test]
    fn recovery_from_silence_fades_in_from_zero() {
        let mut state = PlaybackState::new();
        state.process_frame(Some(A));
        // Drain to Silent.
        for _ in 0..(RAMP_FRAMES + 5) {
            state.process_frame(None);
        }
        assert_eq!(state.phase, PlayPhase::Silent);

        // Now feed RAMP_FRAMES samples; envelope should monotonically rise from 0 to A.
        let mut prev = -1.0;
        for _ in 0..RAMP_FRAMES {
            let (l, _) = state.process_frame(Some(A));
            assert!(l >= prev, "expected monotone fade-in: {l} < {prev}");
            assert!((0.0..=A.0).contains(&l));
            prev = l;
        }
        assert_eq!(state.phase, PlayPhase::Normal);

        // After fade-in, full passthrough.
        let (l, r) = state.process_frame(Some(A));
        assert_eq!((l, r), A);
    }

    #[test]
    fn underrun_during_fade_in_reverses_envelope() {
        let mut state = PlaybackState::new();
        state.process_frame(Some(A));
        for _ in 0..(RAMP_FRAMES + 1) {
            state.process_frame(None);
        }
        // We're now Silent; ramp partway up.
        for _ in 0..(RAMP_FRAMES / 2) {
            state.process_frame(Some(A));
        }
        assert_eq!(state.phase, PlayPhase::FadingIn);
        let (peak, _) = state.process_frame(Some(A));

        // Now hit underrun - phase should switch to FadingOut and envelope decrease.
        let (next, _) = state.process_frame(None);
        assert_eq!(state.phase, PlayPhase::FadingOut);
        assert!(
            next <= peak,
            "fade-out should start below peak: {next} > {peak}"
        );
    }

    #[test]
    fn no_pop_when_starting_in_silence() {
        // First-ever underruns (state == Normal, last is zero) should produce all zeros,
        // not a discontinuity.
        let mut state = PlaybackState::new();
        for _ in 0..(RAMP_FRAMES * 2) {
            assert_eq!(state.process_frame(None), (0.0, 0.0));
        }
    }
}
