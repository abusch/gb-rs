mod cpu;
mod gameboy;
mod bus;
mod cartridge;

use anyhow::Result;
use log::info;

use crate::gameboy::GameBoy;

fn main() -> Result<()> {
    env_logger::builder()
        .parse_filters("gb_rs=debug")
        .init();

    let cartridge = cartridge::Cartridge::load("assets/Tetris (World).gb")?;
    if cartridge.is_cgb() {
        info!("CGB flag is set");
    }
    info!("Title is {}", cartridge.title());
    info!("Licensee code is {}", cartridge.licensee_code());
    info!("Cartridge type is {}", cartridge.cartridge_type());
    info!("ROM size is ${:02x}", cartridge.get_rom_size());
    info!("RAM size is ${:02x}", cartridge.get_ram_size());
    
    let mut gb = GameBoy::new(cartridge);

    for _ in 0..30 {
        gb.dump_cpu();
        gb.step();
    }

    Ok(())
}
