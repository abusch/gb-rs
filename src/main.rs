use std::sync::mpsc::channel;

use anyhow::Result;
use emulator::Emulator;

mod debugger;
mod emulator;

fn main() -> Result<()> {
    // initialise logger
    env_logger::builder().parse_filters("gb_rs=debug").init();
    // ctrl-c handler
    let (tx, rx) = channel();
    ctrlc::set_handler(move || tx.send(()).expect("Could not send signal on channel."))
        .expect("Error setting Ctrl-C handler");

    let mut emulator = Emulator::new()?;

    emulator.run(rx);

    Ok(())
}

