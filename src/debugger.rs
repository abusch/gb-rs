#![allow(dead_code)]
use gb_rs::{gameboy::GameBoy, FrameSink};
use rustyline::{
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::Highlighter,
    hint::Hinter,
    validate::Validator,
    Editor, Helper,
};

#[derive(Debug)]
pub struct Debugger {
    editor: Editor<DebuggerHelper>,
}

impl Debugger {
    pub fn new() -> Self {
        let helper = DebuggerHelper::default();
        let mut editor = Editor::<DebuggerHelper>::new();
        editor.set_helper(Some(helper));
        Self { editor }
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
                    "quit" => true,
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

struct DebuggerHelper {
    commands: Vec<&'static str>,
}

impl Helper for DebuggerHelper {}

impl Completer for DebuggerHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let _ = (line, pos, ctx);
        let candidates = self
            .commands
            .iter()
            .filter(|c| c.starts_with(line))
            .map(|c| Pair {
                display: c.to_string(),
                replacement: c.to_string(),
            })
            .collect::<Vec<_>>();

        Ok((0, candidates))
    }
}

impl Hinter for DebuggerHelper {
    type Hint = String;
}

impl Highlighter for DebuggerHelper {}

impl Validator for DebuggerHelper {}

impl Default for DebuggerHelper {
    fn default() -> DebuggerHelper {
        DebuggerHelper {
            commands: vec!["dump_mem", "dump_cpu", "br", "next", "continue", "quit"],
        }
    }
}
