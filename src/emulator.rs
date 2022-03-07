use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use log::info;
use minifb::{Key, Window};

use gb_rs::{
    cartridge::Cartridge, gameboy::GameBoy, joypad::Button, FrameSink, SCREEN_HEIGHT, SCREEN_WIDTH,
};

use crate::debugger::{Command, Debugger};

// 4.194304MHZ -> 4194304 cycles per seconds
// const CPU_CYCLE_PER_SEC: u64 = 4194304;
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
        let file = std::env::args().nth(1).context("Unable to find ROM file")?;

        let cartridge = Cartridge::load(file)?;
        info!("Title is {}", cartridge.title());
        info!("Licensee code is {}", cartridge.licensee_code());
        info!("Cartridge type is {}", cartridge.cartridge_type());
        info!("ROM size is ${:02x}", cartridge.get_rom_size());
        info!("RAM size is ${:02x}", cartridge.get_ram_size());
        info!("CGB flag: {}", cartridge.cgb_flag());
        info!("SGB flag: {}", cartridge.sgb_flag());
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

    pub fn run(&mut self) {
        let mut sink = MinifbFrameSink::default();
        // Draw an empty frame to  show the window
        sink.draw_current_frame(&mut self.window);

        let mut debugger = Debugger::new();

        let mut start_time_ns = Instant::now();
        let mut emulated_cycles = 0;
        while self.window.is_open() && !self.window.is_key_down(Key::Escape) {
            // if ctrl_c.try_recv().is_ok() {
            if self.window.is_key_pressed(Key::D, minifb::KeyRepeat::No) {
                info!("Starting debugger...");
                self.gb.pause();
            }
            if sink.new_frame {
                sink.draw_current_frame(&mut self.window);
                self.read_input_keys();
            } else {
                self.window.update();
            }

            let target_time_ns = start_time_ns.elapsed();
            let target_cycles = target_time_ns.as_nanos() as u64 / CPU_CYCLE_TIME_NS;

            if self.gb.is_paused() {
                match debugger.debug() {
                    Command::Next(n) => {
                        for _ in 0..n {
                            emulated_cycles += self.gb.step(&mut sink);
                        }
                        self.gb.dump_cpu();
                    }
                    Command::Continue => {
                        // Reset start time
                        start_time_ns = Instant::now()
                            - Duration::from_nanos(emulated_cycles * CPU_CYCLE_TIME_NS);
                        self.gb.resume();
                    }
                    Command::DumpMem(addr) => self.gb.dump_mem(addr),
                    Command::DumpCpu => self.gb.dump_cpu(),
                    Command::DumpOam => self.gb.dump_oam(),
                    Command::DumpPalettes => self.gb.dump_palettes(),
                    Command::Break(addr) => self.gb.set_breakpoint(addr),
                    Command::Sprite(id) => self.gb.dump_sprite(id),
                    Command::Quit => break,
                    Command::Nop => (),
                }
            } else {
                while emulated_cycles < target_cycles && !self.gb.is_paused() {
                    emulated_cycles += self.gb.step(&mut sink);
                }
            }
        }
    }

    fn read_input_keys(&mut self) {
        self.gb
            .set_button_pressed(Button::Start, self.window.is_key_down(Key::Enter));
        self.gb
            .set_button_pressed(Button::Select, self.window.is_key_down(Key::Space));
        self.gb
            .set_button_pressed(Button::A, self.window.is_key_down(Key::A));
        self.gb
            .set_button_pressed(Button::B, self.window.is_key_down(Key::B));
        self.gb
            .set_button_pressed(Button::Up, self.window.is_key_down(Key::Up));
        self.gb
            .set_button_pressed(Button::Down, self.window.is_key_down(Key::Down));
        self.gb
            .set_button_pressed(Button::Left, self.window.is_key_down(Key::Left));
        self.gb
            .set_button_pressed(Button::Right, self.window.is_key_down(Key::Right));
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
