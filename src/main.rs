use std::any::Any;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Sample, SampleRate, Stream, StreamConfig};
use emulator::Emulator;
use gb_rs::{SCREEN_HEIGHT, SCREEN_WIDTH};
use log::{debug, error, info, trace, warn};
use pixels::{Pixels, SurfaceTexture};
use ringbuf::{Consumer, HeapRb};
use winit::event::WindowEvent;
use winit::keyboard::KeyCode;
use winit::{dpi::LogicalSize, event::Event, event_loop::EventLoop, window::WindowBuilder};
use winit_input_helper::WinitInputHelper;

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
    env_logger::builder().parse_filters("gb_rs=debug").init();

    let cli = Cli::parse();

    let event_loop = EventLoop::new()?;
    let mut input = WinitInputHelper::new();
    let window = {
        let size = LogicalSize::new(SCREEN_WIDTH as f64, SCREEN_HEIGHT as f64);
        WindowBuilder::new()
            .with_title("gb-rs")
            .with_inner_size(size)
            .with_min_inner_size(size)
            .build(&event_loop)
            .unwrap()
    };

    let mut pixels = {
        let window_size = window.inner_size();
        let surface_texture = SurfaceTexture::new(window_size.width, window_size.height, &window);
        Pixels::new(SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32, surface_texture)?
    };

    // Buffer can hold 0.5s of samples (assuming 2 channels)
    let ringbuf = HeapRb::new(8102);
    let (producer, consumer) = ringbuf.split();
    let mut emulator = Emulator::new(&cli.rom, producer, cli.breakpoint, cli.enable_soft_break)?;
    let _guard: Box<dyn Any> = if cli.quiet {
        init_no_audio(consumer);
        Box::new(())
    } else {
        let stream = init_audio(consumer)?;
        Box::new(stream)
    };

    event_loop.run(|event, event_loop| {
        if let Event::WindowEvent {
            event: WindowEvent::RedrawRequested,
            ..
        } = event
        {
            emulator.render(pixels.frame_mut());
            if let Err(e) = pixels.render() {
                error!("Error while rendering frame: {}", e);
                event_loop.exit();
                return;
            }
        }

        if input.update(&event) {
            // Close events
            if input.key_pressed(KeyCode::Escape) {
                event_loop.exit();
                emulator.finish();
                return;
            }

            if let Some(size) = input.window_resized() {
                if let Err(e) = pixels.resize_surface(size.width, size.height) {
                    error!("Error while rendering frame: {e}");
                    event_loop.exit();
                    return;
                }
            }

            if input.key_pressed(KeyCode::KeyD) {
                emulator.start_debugger();
            }

            if input.key_pressed(KeyCode::KeyS) {
                if let Err(e) = emulator.screenshot() {
                    warn!("Failed to save screenshot: {}", e);
                }
            }

            emulator.handle_input(&input);
            if emulator.update() {
                event_loop.exit();
                emulator.finish();
                return;
            }
            window.request_redraw();
        }
    })?;

    Ok(())
}

fn init_audio(mut consumer: Consumer<i16, Arc<HeapRb<i16>>>) -> Result<Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("error while querying config")?;
    debug!("Audio device: {:?}", device.name());
    let config = StreamConfig {
        channels: 2,
        sample_rate: SampleRate(44100),
        buffer_size: BufferSize::Fixed(2048),
    };
    let err_fn = |err| {
        error!("Error writing to audio stream: {}", err);
    };
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut _fell_behind = false;
                trace!("Writing {} audio samples", data.len());
                for sample in data {
                    *sample = match consumer.pop() {
                        Some(s) => Sample::from(&s),
                        None => {
                            _fell_behind = true;
                            0.0
                        }
                    }
                }
                // if fell_behind {
                //     debug!("Buffer underrun!");
                // }
            },
            err_fn,
        )
        .context("Failed to build output stream")?;
    stream.play().context("Failed to start stream")?;
    info!("Audio stream started!");

    Ok(stream)
}

fn init_no_audio(mut consumer: Consumer<i16, Arc<HeapRb<i16>>>) {
    thread::spawn(move || {
        loop {
            // empty the ring buffer
            while consumer.pop().is_some() {
                // nop
            }
            thread::sleep(Duration::from_millis(5));
        }
    });
}
