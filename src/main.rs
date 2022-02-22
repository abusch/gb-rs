use std::sync::mpsc::channel;

use anyhow::Result;
use log::info;
use minifb::Window;
use rustyline::{error::ReadlineError, Editor};

use gb_rs::{cartridge::Cartridge, gameboy::GameBoy, FrameSink, SCREEN_HEIGHT, SCREEN_WIDTH};

mod debugger;

fn main() -> Result<()> {
    // initialise logger
    env_logger::builder().parse_filters("gb_rs=debug").init();
    // ctrl-c handler
    let (tx, rx) = channel();
    ctrlc::set_handler(move || tx.send(()).expect("Could not send signal on channel."))
        .expect("Error setting Ctrl-C handler");

    // let cartridge = Cartridge::load("assets/Tetris (World).gb")?;
    let file = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/Tetris (World).gb".into());
    let cartridge = Cartridge::load(file)?;
    if cartridge.is_cgb() {
        info!("CGB flag is set");
    }
    info!("Title is {}", cartridge.title());
    info!("Licensee code is {}", cartridge.licensee_code());
    info!("Cartridge type is {}", cartridge.cartridge_type());
    info!("ROM size is ${:02x}", cartridge.get_rom_size());
    info!("RAM size is ${:02x}", cartridge.get_ram_size());

    let mut gb = GameBoy::new(cartridge);

    let mut window = Window::new(
        "gb-rs",
        160,
        144,
        minifb::WindowOptions {
            resize: false,
            topmost: true,
            scale: minifb::Scale::X2,
            ..Default::default()
        },
    )?;
    let mut sink = MinifbFrameSink::default();
    // Draw an empty frame to  show the window
    sink.draw_current_frame(&mut window);

    let mut rl = Editor::<()>::new();

    loop {
        if rx.try_recv().is_ok() {
            info!("Got ctrl-c");
            gb.pause();
        }
        if sink.new_frame {
            sink.draw_current_frame(&mut window);
        // } else {
        //     window.update();
        }

        if gb.is_paused() {
            let readline = rl.readline("gb-rs> ");
            match readline {
                Ok(line) => {
                    rl.add_history_entry(line.as_str());
                    match line.as_str() {
                        "next" => {
                            gb.step(&mut sink);
                            gb.dump_cpu();
                        }
                        "continue" => {
                            gb.resume();
                        }
                        "dump_cpu" => {
                            gb.dump_cpu();
                        }
                        s if s.starts_with("dump_mem") => {
                            if let Some(addr_str) = s.split_whitespace().nth(1) {
                                if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                                    println!("Dumping memory at {:x}", addr);
                                    gb.dump_mem(addr);
                                }
                            }
                        }
                        s if s.starts_with("br") => {
                            if let Some(addr_str) = s.split_whitespace().nth(1) {
                                if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                                    println!("Setting breakpoint at {:x}", addr);
                                    gb.set_breakpoint(addr);
                                }
                            }
                        }
                        "quit" => {
                            break;
                        }
                        _ => {
                            eprintln!("Unknown command {}", line);
                        }
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!("CTRL-C");
                    break;
                }
                Err(ReadlineError::Eof) => {
                    println!("CTRL-D");
                    break;
                }
                Err(err) => {
                    println!("Error: {:?}", err);
                    break;
                }
            }
        } else {
            gb.step(&mut sink);
        }
    }

    Ok(())
}

pub struct MinifbFrameSink {
    buf: [u32; SCREEN_WIDTH * SCREEN_HEIGHT],
    new_frame: bool,
}

impl MinifbFrameSink {
    pub fn new() -> Self {
        Self {
            buf: [0u32; SCREEN_WIDTH * SCREEN_HEIGHT],
            new_frame: true,
        }
    }

    fn draw_current_frame(&mut self, window: &mut Window) {
        // debug!("Updating minifb buffer");
        window
            .update_with_buffer(&self.buf, SCREEN_WIDTH, SCREEN_HEIGHT)
            .expect("Failed to update window buffer");
        self.new_frame = false;
        // debug!("done minifb buffer");
    }
}

impl Default for MinifbFrameSink {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSink for MinifbFrameSink {
    fn push_frame(&mut self, frame: &[(u8, u8, u8)]) {
        // debug!("Framed pushed");
        self.buf.iter_mut().zip(frame).for_each(|(buf_p, lcd_p)| {
            let (r, g, b) = *lcd_p;
            *buf_p = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
        });
        self.new_frame = true;
    }
}
