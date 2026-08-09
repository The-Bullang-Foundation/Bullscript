//! `record::start` / `record::end` — capture pipe-lines typed at the prompt,
//! then save them into the bag as a new entry.
//!
//! Only lines that actually ran successfully are captured: recording a line
//! that failed to parse or errored at runtime would bake a broken pipe into
//! the saved script.

use crate::bag;

pub struct Recorder {
    lines: Option<Vec<String>>,
}

impl Recorder {
    pub fn new() -> Self {
        Recorder { lines: None }
    }

    /// `record::start` — no nesting: rejected if already recording.
    pub fn start(&mut self) {
        if self.lines.is_some() {
            eprintln!("  Already recording — 'record::end' first (no nested recordings).");
            return;
        }
        self.lines = Some(Vec::new());
        println!("  Recording started. Type pipes as usual; 'record::end' to finish.");
    }

    /// Capture one raw pipe-line. Call this only after the line has run
    /// successfully.
    pub fn capture(&mut self, line: &str) {
        if let Some(lines) = &mut self.lines {
            lines.push(line.to_string());
        }
    }

    /// Abrupt exit while recording (Ctrl+D, etc.) — auto-discard with a warning.
    pub fn discard_on_exit(&mut self) {
        if self.lines.take().is_some() {
            eprintln!("  Warning: recording in progress was discarded on exit.");
        }
    }

    /// `record::end` — preview, ask to save, write into the bag on yes.
    ///
    /// `ask` reads a line from the same editor the prompt uses, so the
    /// terminal is never left in an inconsistent state.
    pub fn end(&mut self, ask: &mut dyn FnMut(&str) -> Option<String>) {
        let Some(lines) = self.lines.take() else {
            eprintln!("  Not currently recording.");
            return;
        };
        if lines.is_empty() {
            println!("  Nothing was recorded. Discarded.");
            return;
        }

        let content = lines.join("\n") + "\n";
        println!("\n  Preview:\n");
        for line in content.lines() {
            println!("  {}", line);
        }
        println!();

        let Some(answer) = ask("  Save? (Y/n) -> ") else {
            println!("  Discarded.");
            return;
        };
        if answer.trim().eq_ignore_ascii_case("n") {
            println!("  Discarded.");
            return;
        }

        loop {
            let Some(name) = ask("  Name for this bag entry -> ") else {
                println!("  Discarded.");
                return;
            };
            let name = name.trim();
            if name.is_empty() {
                println!("  No name given. Discarded.");
                return;
            }
            if let Err(e) = bag::validate_name(name) {
                eprintln!("  {}", e);
                continue;
            }
            match bag::store(name, &content) {
                Ok(true)  => println!("  Saved '{}' to the bag (replaced an existing entry).", name),
                Ok(false) => println!("  Saved '{}' to the bag.", name),
                Err(e)    => eprintln!("  Error saving '{}': {}", name, e),
            }
            return;
        }
    }
}
