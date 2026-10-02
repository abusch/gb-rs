use std::any::Any;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use audio::{AudioStats, CpalAudioSink, init_audio, init_no_audio, negotiate_audio};
use clap::Parser;
use config::{Config, Shader};
use emulator::Emulator;
use gbrs::{BootRom, SCREEN_HEIGHT, SCREEN_WIDTH};
use lcd::LcdRenderer;
use log::{error, info};
use overlay::OverlayRenderer;
use pixels::{Pixels, SurfaceTexture};
use ringbuf::{HeapRb, traits::Split};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::EventLoop,
    keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    window::{Fullscreen, Window, WindowAttributes},
};

mod audio;
mod config;
mod debugger;
mod emulator;
mod input;
mod lcd;
mod overlay;

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
    /// Force a specific audio sample rate (Hz). Must be supported by the output device.
    /// If omitted, the device's default rate is used.
    #[arg(long)]
    audio_rate: Option<u32>,
    /// Path to a DMG boot ROM to run before the game.
    ///
    /// If omitted, the game starts straight away, as if the boot ROM had just run.
    #[arg(long)]
    boot_rom: Option<PathBuf>,
    /// Path to the config file.
    ///
    /// Defaults to `gbrs/config.toml` in the user's configuration directory
    /// (`$XDG_CONFIG_HOME`, or `~/.config`).
    #[arg(long)]
    config: Option<PathBuf>,
    /// Path to the ROM file
    rom: PathBuf,
}

fn parse_addr(s: &str) -> Result<u16, ParseIntError> {
    u16::from_str_radix(s, 16)
}

fn main() -> Result<()> {
    // initialise logger
    env_logger::builder()
        .parse_filters("gbrs=debug,gbrs::apu=info")
        .init();

    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;
    let boot_rom = cli
        .boot_rom
        .map(|path| {
            let content = std::fs::read(&path).context("Failed to read boot rom file")?;
            BootRom::load_bytes(content)
        })
        .transpose()?;

    // Pick the output device and its preferred config so the APU can decimate directly to
    // the device sample rate. For `--quiet`, fall back to a sensible constant since no
    // stream is opened.
    let audio_target = if cli.quiet {
        None
    } else {
        Some(negotiate_audio(cli.audio_rate)?)
    };
    let device_rate = audio_target
        .as_ref()
        .map(|(_, cfg)| cfg.sample_rate())
        .unwrap_or(48_000);
    info!("audio: device rate = {device_rate} Hz");

    // Buffer holds ~0.5 s of stereo at the device rate (= device_rate * 2 channels / 2).
    let ringbuf_capacity = device_rate as usize;
    let ringbuf = HeapRb::<f32>::new(ringbuf_capacity);
    let (producer, consumer) = ringbuf.split();
    let audio_stats = Arc::new(AudioStats::default());
    let mut emulator = Emulator::new(
        &cli.rom,
        &config,
        boot_rom,
        CpalAudioSink::new(producer, Arc::clone(&audio_stats)),
        cli.breakpoint,
        cli.enable_soft_break,
        device_rate,
    )?;

    // Pre-buffer ~46 ms of audio (2 cpal callbacks worth) before starting the cpal stream,
    // so the first callback finds samples ready instead of underrunning.
    if !cli.quiet {
        let warmup_f32 = (device_rate as usize) * 46 / 1000 * 2;
        const WARMUP_TIMEOUT: Duration = Duration::from_millis(500);
        emulator.warm_up_audio(warmup_f32, WARMUP_TIMEOUT);
        emulator.reset_clock();
    }

    let _guard: Box<dyn Any> = match audio_target {
        None => {
            init_no_audio(consumer);
            Box::new(())
        }
        Some((device, supported_cfg)) => {
            let stream = init_audio(device, supported_cfg, consumer, Arc::clone(&audio_stats))?;
            Box::new(stream)
        }
    };

    let mut app = App::new(emulator);
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut app)?;

    Ok(())
}

struct App {
    pixels: Option<Pixels<'static>>,
    lcd: Option<LcdRenderer>,
    overlay: Option<OverlayRenderer>,
    window: Option<Arc<Window>>,
    emulator: Emulator,
}

impl App {
    pub fn new(emulator: Emulator) -> Self {
        Self {
            pixels: None,
            lcd: None,
            overlay: None,
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

            self.lcd = Some(LcdRenderer::new(&pixels));
            self.overlay = Some(OverlayRenderer::new(&pixels));
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
                event_loop.exit();
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
                    return;
                }
                if k.physical_key == PhysicalKey::Code(KeyCode::KeyF)
                    && k.state.is_pressed()
                    && !k.repeat
                {
                    if let Some(window) = &self.window {
                        let fullscreen = window.fullscreen().is_none();
                        window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
                    }
                    return;
                }
                self.emulator.handle_input(k);
            }
            WindowEvent::RedrawRequested => {
                if let (Some(pixels), Some(lcd), Some(overlay), Some(window)) = (
                    self.pixels.as_mut(),
                    self.lcd.as_ref(),
                    self.overlay.as_ref(),
                    self.window.as_mut(),
                ) {
                    // Run the emiulator
                    if self.emulator.update() {
                        event_loop.exit();
                        return;
                    }
                    // Render a frame
                    self.emulator.render(pixels.frame_mut());
                    let shader = self.emulator.shader();
                    let background = self.emulator.background();
                    let message = self.emulator.message();
                    let result = pixels.render_with(|encoder, render_target, context| {
                        match shader {
                            Shader::None => context.scaling_renderer.render(encoder, render_target),
                            Shader::Lcd => lcd.render(encoder, render_target, context, background),
                        }
                        if let Some((text, opacity)) = message {
                            overlay.render(encoder, render_target, context, text, opacity);
                        }
                        Ok(())
                    });
                    if let Err(e) = result {
                        error!("Error while rendering frame: {}", e);
                        event_loop.exit();
                        return;
                    }
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    /// Save the game however the emulator is quit: this runs after `exit()`, and when the OS
    /// quits the application (e.g. Cmd-Q on macOS).
    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        self.emulator.finish();
    }
}
