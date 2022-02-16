#![allow(dead_code)]
use rustyline::Editor;

#[derive(Debug)]
struct Debugger {
    editor: Editor<()>
}
