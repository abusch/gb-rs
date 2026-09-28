use std::borrow::Cow;

use ansi_term::Colour;
use anyhow::Result;
use rustyline::{
    Config, Editor, Helper,
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::Highlighter,
    hint::Hinter,
    history::MemHistory,
    validate::Validator,
};

#[derive(Debug)]
pub struct Debugger {
    editor: Editor<DebuggerHelper, MemHistory>,
}

impl Debugger {
    pub fn new() -> Result<Self> {
        let helper = DebuggerHelper::default();
        let mut editor = Editor::<DebuggerHelper, MemHistory>::with_history(
            Config::default(),
            MemHistory::new(),
        )?;
        editor.set_helper(Some(helper));
        Ok(Self { editor })
    }

    pub fn debug(&mut self) -> Command {
        let readline = self.editor.readline("gb-rs> ");
        match readline {
            Ok(line) => {
                self.editor
                    .add_history_entry(line.as_str())
                    .expect("Failed to add history entry");
                if line.trim().is_empty() {
                    Command::Nop
                } else {
                    parse_command(&line).unwrap_or_else(|| {
                        println!("Invalid command: {line}");
                        Command::Nop
                    })
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

/// Parse a command line. Addresses, values and the `next` count are in hex, without a `0x` prefix.
fn parse_command(line: &str) -> Option<Command> {
    let hex16 = |s: &str| u16::from_str_radix(s, 16).ok();
    let mut words = line.split_whitespace();
    let command = match (words.next()?, words.next()) {
        ("next", count) => Command::Next(count.map_or(Some(1), hex16)?),
        ("continue", None) => Command::Continue,
        ("cpu", None) => Command::DumpCpu,
        ("oam", None) => Command::DumpOam,
        ("palettes", None) => Command::DumpPalettes,
        ("mem", Some(addr)) => Command::DumpMem(hex16(addr)?),
        ("dis", Some(addr)) => Command::Disassemble(hex16(addr)?),
        ("br", Some(addr)) => Command::Break(hex16(addr)?),
        ("sprite", Some(id)) => Command::Sprite(id.parse().ok()?),
        ("poke", Some(addr)) => {
            Command::Poke(hex16(addr)?, u8::from_str_radix(words.next()?, 16).ok()?)
        }
        ("quit", None) => Command::Quit,
        _ => return None,
    };
    // No trailing arguments
    words.next().is_none().then_some(command)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Next(u16),
    Continue,
    DumpMem(u16),
    Disassemble(u16),
    DumpCpu,
    DumpOam,
    Sprite(u8),
    DumpPalettes,
    Break(u16),
    Poke(u16, u8),
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
        let _ = (pos, ctx);
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
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        Cow::Owned(format!("{}", Colour::Green.dimmed().paint(prompt)))
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(format!("{}", Colour::White.dimmed().paint(hint)))
    }
}

impl Validator for DebuggerHelper {}

impl Default for DebuggerHelper {
    fn default() -> DebuggerHelper {
        DebuggerHelper {
            commands: vec![
                "mem", "cpu", "oam", "sprite", "palettes", "br", "next", "continue", "quit", "dis",
                "poke",
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command() {
        assert_eq!(parse_command("next"), Some(Command::Next(1)));
        assert_eq!(parse_command("next 10"), Some(Command::Next(0x10)));
        assert_eq!(
            parse_command("  mem  c000 "),
            Some(Command::DumpMem(0xC000))
        );
        assert_eq!(parse_command("sprite 12"), Some(Command::Sprite(12)));
        assert_eq!(
            parse_command("poke ff40 91"),
            Some(Command::Poke(0xFF40, 0x91))
        );
        assert_eq!(parse_command("quit"), Some(Command::Quit));

        assert_eq!(parse_command("mem"), None);
        assert_eq!(parse_command("br 0x100"), None);
        assert_eq!(parse_command("poke ff40"), None);
        assert_eq!(parse_command("cpu now"), None);
        assert_eq!(parse_command("jump 100"), None);
    }
}
