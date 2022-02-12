use std::{io::stdin, sync::mpsc::channel};

use anyhow::Result;
use log::info;

use gb_rs::{
    gameboy::GameBoy,
    cartridge::Cartridge,
};

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

    gb.dump_cpu();
    let mut buf = String::new();
    while !gb.is_halted() {
        if rx.try_recv().is_ok() {
            info!("Got ctrl-c. Exiting...");
            break;
        }
        if gb.is_paused() {
            stdin().read_line(&mut buf).unwrap();
            gb.step();
            gb.dump_cpu();
        } else {
            gb.step();
        }
    }
    gb.dump_cpu();

    Ok(())
}
