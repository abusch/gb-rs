use std::{io::stdin, sync::mpsc::channel};

use anyhow::Result;
use log::{info, debug};

use gb_rs::{cartridge::Cartridge, gameboy::GameBoy, FrameSink, SCREEN_HEIGHT, SCREEN_WIDTH};
use minifb::Window;

fn main() -> Result<()> {
    // initialise logger
    env_logger::builder().parse_filters("gb_rs=debug").init();
    // ctrl-c handler
    let (tx, rx) = channel();
    ctrlc::set_handler(move || tx.send(()).expect("Could not send signal on channel."))
        .expect("Error setting Ctrl-C handler");

    let cartridge = Cartridge::load("assets/Tetris (World).gb")?;
    if cartridge.is_cgb() {
        info!("CGB flag is set");
    }
    info!("Title is {}", cartridge.title());
    info!("Licensee code is {}", cartridge.licensee_code());
    info!("Cartridge type is {}", cartridge.cartridge_type());
    info!("ROM size is ${:02x}", cartridge.get_rom_size());
    info!("RAM size is ${:02x}", cartridge.get_ram_size());

    let mut gb = GameBoy::new(cartridge);

    let mut sink = MinifbFrameSink::new()?;

    gb.dump_cpu();
    let mut buf = String::new();
    while !gb.is_halted() {
        if rx.try_recv().is_ok() {
            info!("Got ctrl-c. Exiting...");
            break;
        }
        // debug!("Updating minifb events...");
        // sink.window.update();
        // debug!("done");
        if gb.is_paused() {
            stdin().read_line(&mut buf).unwrap();
            gb.step(&mut sink);
            gb.dump_cpu();
        } else {
            gb.step(&mut sink);
        }
    }
    gb.dump_cpu();

    Ok(())
}

pub struct MinifbFrameSink {
    window: Window,
    buf: [u32; SCREEN_WIDTH * SCREEN_HEIGHT],
}

impl MinifbFrameSink {
    pub fn new() -> Result<Self> {
        let window = Window::new(
            "gb-rs",
            160,
            144,
            minifb::WindowOptions {
                resize: false,
                scale: minifb::Scale::X2,
                ..Default::default()
            },
        )?;
        Ok(Self {
            window,
            buf: [0u32; SCREEN_WIDTH * SCREEN_HEIGHT],
        })
    }
}

impl FrameSink for MinifbFrameSink {
    fn push_frame(&mut self, frame: &[(u8, u8, u8)]) {
        // debug!("Framed pushed");
        self.buf
            .iter_mut()
            .zip(frame)
            .for_each(|(buf_p, lcd_p)| {
                let (r, g, b) = *lcd_p;
                *buf_p = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
            });

        // debug!("Updating minifb buffer");
        self.window.update_with_buffer(&self.buf, SCREEN_WIDTH, SCREEN_HEIGHT).expect("Failed to update window buffer");
        // debug!("done minifb buffer");
    }
}
