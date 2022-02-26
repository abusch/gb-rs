use anyhow::Result;
use emulator::Emulator;

mod debugger;
mod emulator;

fn main() -> Result<()> {
    // initialise logger
    env_logger::builder().parse_filters("gb_rs=debug").init();

    let mut emulator = Emulator::new()?;
    emulator.run();

    Ok(())
}
