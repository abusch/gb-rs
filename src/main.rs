use anyhow::Result;
use emulator::Emulator;
use gb_rs::{SCREEN_HEIGHT, SCREEN_WIDTH};
use log::error;
use pixels::{Pixels, SurfaceTexture};
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

    let mut emulator = Emulator::new()?;

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
