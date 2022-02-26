use std::borrow::Cow;

use ansi_term::Colour;
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

    pub fn debug(&mut self) -> Command {
        let readline = self.editor.readline("gb-rs> ");
        match readline {
            Ok(line) => {
                self.editor.add_history_entry(line.as_str());
                match line.as_str() {
                    "next" => {
                        Command::Next
                        // gb.step(sink);
                        // gb.dump_cpu();
                        // false
                    }
                    "continue" => {
                        Command::Continue
                        // gb.resume();
                        // false
                    }
                    "cpu" => {
                        Command::DumpCpu
                        // gb.dump_cpu();
                        // false
                    }
                    "oam" => Command::DumpOam,
                    s if s.starts_with("mem") => {
                        if let Some(addr_str) = s.split_whitespace().nth(1) {
                            if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                                // println!("Dumping memory at {:x}", addr);
                                // gb.dump_mem(addr);
                                return Command::DumpMem(addr);
                            }
                        }
                        Command::Nop
                    }
                    s if s.starts_with("br") => {
                        if let Some(addr_str) = s.split_whitespace().nth(1) {
                            if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                                // println!("Setting breakpoint at {:x}", addr);
                                // gb.set_breakpoint(addr);
                                return Command::Break(addr);
                            }
                        }
                        Command::Nop
                    }
                    s if s.starts_with("sprite ") => {
                        if let Some(id_str) = s.split_whitespace().nth(1) {
                            if let Ok(id) = id_str.parse::<u8>() {
                                return Command::Sprite(id);
                            }
                        }
                        Command::Nop
                    }
                    "quit" => Command::Quit,
                    _ => {
                        // eprintln!("Unknown command {}", line);
                        // false
                        Command::Nop
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                Command::Nop
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                Command::Quit
            }
            Err(err) => {
                println!("Error: {:?}", err);
                Command::Nop
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Next,
    Continue,
    DumpMem(u16),
    DumpCpu,
    DumpOam,
    Sprite(u8),
    Break(u16),
    Quit,
    Nop,
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

    fn hint(&self, line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<Self::Hint> {
        if line == "br " {
            Some("<hex address>".to_string())
        } else if line == "sprite " {
            Some("<sprite number>".to_string())
        } else {
            None
        }
    }
}

impl Highlighter for DebuggerHelper {
    fn highlight<'l>(&self, line: &'l str, pos: usize) -> std::borrow::Cow<'l, str> {
        let _ = pos;
        std::borrow::Cow::Borrowed(line)
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> std::borrow::Cow<'b, str> {
        Cow::Owned(format!("{}", Colour::Green.dimmed().paint(prompt)))
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> std::borrow::Cow<'h, str> {
        Cow::Owned(format!("{}", Colour::White.dimmed().paint(hint)))
        // std::borrow::Cow::Borrowed(hint)
    }

    fn highlight_candidate<'c>(
        &self,
        candidate: &'c str, // FIXME should be Completer::Candidate
        completion: rustyline::CompletionType,
    ) -> std::borrow::Cow<'c, str> {
        let _ = completion;
        std::borrow::Cow::Borrowed(candidate)
    }

    fn highlight_char(&self, line: &str, pos: usize) -> bool {
        let _ = (line, pos);
        false
    }
}

impl Validator for DebuggerHelper {}

impl Default for DebuggerHelper {
    fn default() -> DebuggerHelper {
        DebuggerHelper {
            commands: vec!["mem", "cpu", "oam", "sprite", "br", "next", "continue", "quit"],
        }
    }
}
