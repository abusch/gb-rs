use std::{time::Instant, sync::mpsc::Receiver};

use anyhow::Result;
use log::info;
use minifb::{Window, Key};

use gb_rs::{cartridge::Cartridge, gameboy::GameBoy, SCREEN_WIDTH, SCREEN_HEIGHT, FrameSink};
use rustyline::{Editor, error::ReadlineError};

// 4.194304MHZ -> 4194304 cycles per seconds
const CPU_CYCLE_PER_SEC: u64 = 4194304;
// 1/4194304 seconds per cycle -> 238 nanoseconds per cycle
const CPU_CYCLE_TIME_NS: u64 = 238;

/// The object that pulls everything together and drives the emulation engine while interfacing
/// with actual input/outputs.
pub struct Emulator {
    window: Window,
    gb: GameBoy,
}

impl Emulator {
    pub fn new() -> Result<Self> {
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
        let gb = GameBoy::new(cartridge);
        let window = Window::new(
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
        Ok(Self { window, gb })
    }

    pub fn run(&mut self, ctrl_c: Receiver<()>) {
        let mut sink = MinifbFrameSink::default();
        // Draw an empty frame to  show the window
        sink.draw_current_frame(&mut self.window);

        let mut rl = Editor::<()>::new();

        let start_time_ns = Instant::now();
        let mut emulated_cycles = 0;
        while self.window.is_open() && !self.window.is_key_down(Key::Escape) {
            if ctrl_c.try_recv().is_ok() {
                info!("Got ctrl-c");
                self.gb.pause();
            }
            if sink.new_frame {
                sink.draw_current_frame(&mut self.window);
            } else {
                self.window.update();
            }

            let target_time_ns = start_time_ns.elapsed();
            let target_cycles = target_time_ns.as_nanos() as u64 / CPU_CYCLE_TIME_NS;

            if self.gb.is_paused() {
                let readline = rl.readline("gb-rs> ");
                match readline {
                    Ok(line) => {
                        rl.add_history_entry(line.as_str());
                        match line.as_str() {
                            "next" => {
                                self.gb.step(&mut sink);
                                self.gb.dump_cpu();
                            }
                            "continue" => {
                                self.gb.resume();
                            }
                            "dump_cpu" => {
                                self.gb.dump_cpu();
                            }
                            s if s.starts_with("dump_mem") => {
                                if let Some(addr_str) = s.split_whitespace().nth(1) {
                                    if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                                        println!("Dumping memory at {:x}", addr);
                                        self.gb.dump_mem(addr);
                                    }
                                }
                            }
                            s if s.starts_with("br") => {
                                if let Some(addr_str) = s.split_whitespace().nth(1) {
                                    if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                                        println!("Setting breakpoint at {:x}", addr);
                                        self.gb.set_breakpoint(addr);
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
                while emulated_cycles < target_cycles {
                    emulated_cycles += self.gb.step(&mut sink);
                }
            }
        }
    }
}

struct MinifbFrameSink {
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
