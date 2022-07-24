use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Sample, SampleRate, Stream, StreamConfig};
use emulator::Emulator;
use gb_rs::{SCREEN_HEIGHT, SCREEN_WIDTH};
use log::{debug, error, info, trace, warn};
use pixels::{Pixels, SurfaceTexture};
use ringbuf::{Consumer, RingBuffer};
use winit::{
    dpi::LogicalSize,
    event::{Event, VirtualKeyCode},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use winit_input_helper::WinitInputHelper;

mod debugger;
mod emulator;

fn main() -> Result<()> {
    // initialise logger
    env_logger::builder().parse_filters("gb_rs=debug").init();

    let event_loop = EventLoop::new();
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
    let ringbuf = RingBuffer::new(8102);
    let (producer, consumer) = ringbuf.split();
    let mut emulator = Emulator::new(producer)?;
    let _stream = init_audio(consumer)?;

    event_loop.run(move |event, _, control_flow| {
        if let Event::RedrawRequested(_) = event {
            emulator.render(pixels.get_frame());
            if let Err(e) = pixels.render() {
                error!("Error while rendering frame: {}", e);
                *control_flow = ControlFlow::Exit;
                return;
            }
        }

        if input.update(&event) {
            // Close events
            if input.key_pressed(VirtualKeyCode::Escape) {
                *control_flow = ControlFlow::Exit;
                emulator.finish();
                return;
            }

            if let Some(size) = input.window_resized() {
                pixels.resize_surface(size.width, size.height);
            }

            if input.key_pressed(VirtualKeyCode::D) {
                emulator.start_debugger();
            }

            if input.key_pressed(VirtualKeyCode::S) {
                if let Err(e) = emulator.screenshot() {
                    warn!("Failed to save screenshot: {}", e);
                }
            }

            emulator.handle_input(&input);
            if emulator.update() {
                *control_flow = ControlFlow::Exit;
                emulator.finish();
                return;
            }
            window.request_redraw();
        }
    });
}

fn init_audio(mut consumer: Consumer<i16>) -> Result<Stream> {
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
                let mut fell_behind = false;
                trace!("Writing {} audio samples", data.len());
                for sample in data {
                    *sample = match consumer.pop() {
                        Some(s) => Sample::from(&s),
                        None => {
                            fell_behind = true;
                            0.0
                        }
                    }
                }
                if fell_behind {
                    debug!("Buffer underrun!");
                }
            },
            err_fn,
        )
        .context("Failed to build output stream")?;
    stream.play().context("Failed to start stream")?;
    info!("Audio stream started!");

    Ok(stream)
}
