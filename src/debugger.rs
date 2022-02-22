#![allow(dead_code)]
use gb_rs::{gameboy::GameBoy, FrameSink};
use rustyline::{Editor, error::ReadlineError};

#[derive(Debug)]
pub struct Debugger {
    editor: Editor<()>,
}

impl Debugger {
    pub fn new() -> Self {
        Self {
            editor: Editor::<()>::new(),
        }
    }
    pub fn debug(&mut self, gb: &mut GameBoy, sink: &mut dyn FrameSink) -> bool {
        let readline = self.editor.readline("gb-rs> ");
        match readline {
            Ok(line) => {
                self.editor.add_history_entry(line.as_str());
                match line.as_str() {
                    "next" => {
                        gb.step(sink);
                        gb.dump_cpu();
                        false
                    }
                    "continue" => {
                        gb.resume();
                        false
                    }
                    "dump_cpu" => {
                        gb.dump_cpu();
                        false
                    }
                    s if s.starts_with("dump_mem") => {
                        if let Some(addr_str) = s.split_whitespace().nth(1) {
                            if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                                println!("Dumping memory at {:x}", addr);
                                gb.dump_mem(addr);
                            }
                        }
                        false
                    }
                    s if s.starts_with("br") => {
                        if let Some(addr_str) = s.split_whitespace().nth(1) {
                            if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                                println!("Setting breakpoint at {:x}", addr);
                                gb.set_breakpoint(addr);
                            }
                        }
                        false
                    }
                    "quit" => {
                        true
                    }
                    _ => {
                        eprintln!("Unknown command {}", line);
                        false
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                true
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                true
            }
            Err(err) => {
                println!("Error: {:?}", err);
                true
            }
        }
    }
}
