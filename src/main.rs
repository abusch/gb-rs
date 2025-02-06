use std::any::Any;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BufferSize, Sample, SampleRate, Stream, StreamConfig,
};
use emulator::Emulator;
use gb_rs::{SCREEN_HEIGHT, SCREEN_WIDTH};
use log::{debug, error, info, trace};
use pixels::{Pixels, SurfaceTexture};
use ringbuf::{
    traits::{Consumer, Split},
    HeapRb,
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
    let emulator = Emulator::new(&cli.rom, producer, cli.breakpoint, cli.enable_soft_break)?;
    let _guard: Box<dyn Any> = if cli.quiet {
        init_no_audio(consumer);
        Box::new(())
    } else {
        let stream = init_audio(consumer)?;
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
                if let Some(ref mut pixels) = self.pixels {
                    if let Err(e) = pixels.resize_surface(size.width, size.height) {
                        error!("Error while rendering frame: {e}");
                        event_loop.exit();
                    }
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

fn init_audio(mut consumer: impl Consumer<Item = f32> + Send + 'static) -> Result<Stream> {
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
                trace!("Writing {} audio samples", data.len());
                for sample in data {
                    *sample = match consumer.try_pop() {
                        Some(s) => s.to_sample::<f32>(),
                        None => 0.0,
                    }
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
