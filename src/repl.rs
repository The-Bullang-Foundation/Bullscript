//! The interactive prompt.
//!
//! Every line typed is a program in its own right: it is parsed, type checked
//! against the bindings already in scope, and only then run. Bindings persist
//! across lines, so a value bound by one line is available to the next until
//! the session ends.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Context, Editor, Helper};

use crate::bag;
use crate::bin_store;
use crate::complete::{self, Completion};
use crate::data;
use crate::help;
use crate::lang;
use crate::lang::interp::{self, Env};
use crate::lang::types::BsType;
use crate::record::Recorder;

const PROMPT: &str = "bullscript -> ";

/// The line editor, with completion and hints attached.
type Line = Editor<BsHelper, DefaultHistory>;

// ── Completion and hints ──────────────────────────────────────────────────

/// The bindings in scope, shared with the line editor.
///
/// rustyline owns its helper, and the environment is owned by the loop, so
/// the helper cannot borrow it. A snapshot of `(name, type)` pairs is taken
/// after every line instead; it is what completion needs and nothing more.
type Bindings = Arc<Mutex<Vec<(String, String)>>>;

/// What rustyline asks for at the prompt: completion on Tab, a hint while
/// typing, and how to draw the hint. All three come from `complete::complete`.
struct BsHelper {
    bindings: Bindings,
    files:    FilenameCompleter,
}

impl BsHelper {
    fn new(bindings: Bindings) -> Self {
        BsHelper { bindings, files: FilenameCompleter::new() }
    }

    fn lookup(&self, line: &str, pos: usize) -> Completion {
        let snapshot = self.bindings.lock().unwrap_or_else(|e| e.into_inner());
        complete::complete(&line[..pos], &snapshot, true)
    }
}

impl Completer for BsHelper {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        match self.lookup(line, pos) {
            Completion::Path => self.files.complete_path(line, pos),
            Completion::Words { start, candidates } => Ok((
                start,
                candidates.into_iter().map(|c| {
                    // Shown in the list with its signature or type, inserted
                    // without it.
                    let display = match &c.detail {
                        Some(d) => format!("{}  {}", c.label, d),
                        None    => c.label.clone(),
                    };
                    Pair { display, replacement: c.label }
                }).collect(),
            )),
            Completion::None => Ok((pos, Vec::new())),
        }
    }
}

impl Hinter for BsHelper {
    type Hint = String;

    /// The greyed-out rest of the word, as fish does. Only shown while the
    /// cursor is at the end of the line and only when the candidates agree:
    /// a single match gives its tail, several give their common prefix if it
    /// is longer than what has been typed, and anything less stays quiet —
    /// a hint that is wrong more often than right is noise.
    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<String> {
        if pos < line.len() || line.is_empty() {
            return None;
        }
        let Completion::Words { start, candidates } = self.lookup(line, pos) else {
            return None;
        };
        let typed = &line[start..];
        if typed.is_empty() {
            return None;
        }
        let common = candidates.iter().skip(1).fold(candidates[0].label.as_str(), |acc, c| {
            let n = acc.bytes().zip(c.label.bytes()).take_while(|(a, b)| a == b).count();
            &acc[..n]
        });
        let rest = &common[typed.len()..];
        if rest.is_empty() { None } else { Some(rest.to_string()) }
    }
}

impl Highlighter for BsHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        // Dim, reset after. Windows Terminal and every modern Unix terminal
        // read these; an older console shows the hint undimmed, which is
        // still readable.
        Cow::Owned(format!("\x1b[2m{}\x1b[0m", hint))
    }
}

impl Validator for BsHelper {}
impl Helper for BsHelper {}

/// Refresh the editor's view of the bindings after a line has run.
fn snapshot_bindings(env: &Env, bindings: &Bindings) {
    let mut snap: Vec<(String, String)> = env.iter()
        .map(|(n, v)| (n.clone(), v.ty().to_string()))
        .collect();
    snap.sort();
    *bindings.lock().unwrap_or_else(|e| e.into_inner()) = snap;
}

/// Clear the screen and put the cursor at the top left.
///
/// Written directly rather than shelling out to `clear` or `cls`: those are
/// two different commands on two platforms, either may be absent, and running
/// a process to emit six bytes is a poor trade. The sequence is ANSI —
/// `2J` erases the display, `H` homes the cursor — which Windows Terminal and
/// every modern Unix terminal understand.
fn clear_screen() {
    use std::io::Write;
    print!("\x1b[2J\x1b[H");
    let _ = std::io::stdout().flush();
}

pub fn run() {
    let mut rl: Line = match Editor::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Could not start the line editor: {}", e);
            std::process::exit(1);
        }
    };
    let bindings: Bindings = Arc::new(Mutex::new(Vec::new()));
    rl.set_helper(Some(BsHelper::new(Arc::clone(&bindings))));

    let history = history_path();
    if let Some(ref p) = history {
        let _ = rl.load_history(p);
    }

    let mut env = Env::new();
    let mut recorder = Recorder::new();

    loop {
        let line = match rl.readline(PROMPT) {
            Ok(l) => l,
            Err(ReadlineError::Eof) => break,
            Err(ReadlineError::Interrupted) => {
                // Ctrl+C abandons the line being typed, not the session.
                continue;
            }
            Err(e) => {
                eprintln!("Read error: {}", e);
                break;
            }
        };

        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(line);

        if handle_line(line, &mut env, &mut recorder, &mut rl) {
            break;
        }
        snapshot_bindings(&env, &bindings);

        // `builtin::out` appends nothing, so a pipe can leave the cursor
        // part-way along a line. rustyline draws its next prompt with a
        // carriage return followed by erase-to-end-of-line, which would wipe
        // that line — short output vanished outright, and output long enough
        // to wrap lost its last row, which looked like truncation. Finishing
        // the line here is what a shell does with a command whose output has
        // no final newline.
        if !crate::lang::builtins::at_line_start() {
            println!();
            crate::lang::builtins::mark_line_start();
        }
    }

    quit(&mut recorder, &mut rl, history.as_ref());
}

/// Handle one line. Returns true when the session should end.
fn handle_line(
    line:     &str,
    env:      &mut Env,
    recorder: &mut Recorder,
    rl:       &mut Line,
) -> bool {
    match line {
        "help" => { help::run(); return false; }
        "exit" => return true,

        // Clears the screen, as `clear` does in a shell. It does *not* clear
        // your bindings — this is the same distinction a terminal makes, where
        // clearing the screen leaves the shell's variables alone. `bag::list`
        // and the history are likewise untouched.
        "clear" => { clear_screen(); return false; }
        "record::start" => { recorder.start(); return false; }
        "record::end" => {
            let mut ask = |prompt: &str| rl.readline(prompt).ok();
            recorder.end(&mut ask);
            return false;
        }
        _ => {}
    }

    // Store directives are matched before the pipe parser so that a bare or
    // malformed directive gets its usage message rather than a parse error
    // about an unexpected identifier.
    let mut words = line.split_whitespace();
    let first = words.next().unwrap_or("");
    if let Some((prefix, op)) = first.split_once("::") {
        if let Some(store) = STORES.iter().find(|s| s.prefix == prefix) {
            let args: Vec<&str> = words.collect();
            if store.directive(op, &args) {
                return false;
            }
        }
    }

    // Anything else is a pipe (or several).
    if run_line(line, env) {
        // Only a line that parsed, checked and ran is worth recording.
        recorder.capture(line);
    }
    false
}

// ── Store directives ──────────────────────────────────────────────────────
//
// The bag, the data store and the bin store accept the same directives —
// add, remove, list, and for the two whose files travel, export and import.
// Each store is one row here; the handling is written once.

/// What the prompt needs to know about a store to run its directives.
struct Store {
    /// The namespace users type: `bag`, `data`, `bin`.
    prefix: &'static str,
    /// One thing it holds, for counts: "script", "document", "program".
    item: &'static str,
    /// The store itself, for messages: "the bag", "your data".
    whole: &'static str,
    /// What `add` expects as its first argument, for the usage line.
    add_path: &'static str,
    /// Printed when `list` has nothing to show.
    empty: &'static str,
    add:    fn(&str, &str) -> Result<bool, String>,
    remove: fn(&str) -> Result<bool, String>,
    list:   fn() -> Result<Vec<(String, String)>, String>,
    /// None for a store whose files do not travel.
    export: Option<fn(&str) -> Result<(usize, PathBuf), String>>,
    import: Option<fn(&str) -> Result<(usize, usize, Vec<String>), String>>,
}

const STORES: &[Store] = &[
    Store {
        prefix:   "bag",
        item:     "script",
        whole:    "the bag",
        add_path: "<path/to/script.busc>",
        empty:    "(bag is empty)",
        add:      bag::add,
        remove:   bag::remove,
        list:     bag::list,
        export:   Some(bag::export),
        import:   Some(bag::import),
    },
    Store {
        prefix:   "data",
        item:     "document",
        whole:    "your data",
        add_path: "<path/to/file.json>",
        empty:    "(your data is empty)",
        add:      data::add,
        remove:   data::remove,
        list:     data::list,
        export:   Some(data::export),
        import:   Some(data::import),
    },
    // A compiled program is tied to one operating system and one
    // architecture, so there is no export or import: an archive of binaries
    // handed to someone else would largely not run.
    Store {
        prefix:   "bin",
        item:     "program",
        whole:    "your programs",
        add_path: "<path/to/program>",
        empty:    "(you have no programs)",
        add:      bin_store::add,
        remove:   bin_store::remove,
        list:     bin_store::list,
        export:   None,
        import:   None,
    },
];

impl Store {
    /// Run `op` with `args`. Returns false if `op` is not a directive of this
    /// store, so the line falls through to the pipe parser.
    fn directive(&self, op: &str, args: &[&str]) -> bool {
        match op {
            "add" => {
                if args.len() != 2 {
                    eprintln!("  Usage: {}::add {} <name>", self.prefix, self.add_path);
                    return true;
                }
                match (self.add)(args[0], args[1]) {
                    Ok(true)  => println!("  Added '{}' to {} (replaced an existing entry).", args[1], self.whole),
                    Ok(false) => println!("  Added '{}' to {}.", args[1], self.whole),
                    Err(e)    => eprintln!("  {}", e),
                }
            }
            "remove" => {
                if args.len() != 1 {
                    eprintln!("  Usage: {}::remove <name>", self.prefix);
                    return true;
                }
                match (self.remove)(args[0]) {
                    Ok(true)  => println!("  Removed '{}' from {}.", args[0], self.whole),
                    Ok(false) => eprintln!("  '{}' is not in {}.", args[0], self.whole),
                    Err(e)    => eprintln!("  {}", e),
                }
            }
            "list" => {
                if !args.is_empty() {
                    eprintln!("  Usage: {}::list", self.prefix);
                    return true;
                }
                match (self.list)() {
                    Ok(entries) if entries.is_empty() => println!("  {}", self.empty),
                    Ok(entries) => {
                        for (name, path) in entries {
                            println!("  {} -> {}", name, path);
                        }
                    }
                    Err(e) => eprintln!("  {}", e),
                }
            }
            "export" => {
                let Some(export) = self.export else { return false };
                if args.len() != 1 {
                    eprintln!("  Usage: {}::export <path/to/archive.zip>", self.prefix);
                    return true;
                }
                match export(args[0]) {
                    Ok((n, dest)) => println!("  Exported {} {}(s) to {}.", n, self.item, dest.display()),
                    Err(e)        => eprintln!("  {}", e),
                }
            }
            "import" => {
                let Some(import) = self.import else { return false };
                if args.len() != 1 {
                    eprintln!("  Usage: {}::import <path/to/archive.zip>", self.prefix);
                    return true;
                }
                match import(args[0]) {
                    Ok((added, replaced, skipped)) => {
                        println!("  Imported {} {}(s), replaced {}.", added, self.item, replaced);
                        for name in &skipped {
                            eprintln!(
                                "  Skipped '{}': its name cannot be an entry name, or its \
                                 content is not a {}.", name, self.item
                            );
                        }
                    }
                    Err(e) => eprintln!("  {}", e),
                }
            }
            _ => return false,
        }
        true
    }
}

/// Parse, check and run one line. Returns true on success.
fn run_line(line: &str, env: &mut Env) -> bool {
    let program = match lang::parse_source(line) {
        Ok(p) => p,
        Err(e) => { eprintln!("  parse error — {}", e); return false; }
    };
    if program.is_empty() {
        return false;
    }

    // Type check against what is already in scope.
    let scope: HashMap<String, BsType> = env.iter()
        .map(|(k, v)| (k.clone(), v.ty()))
        .collect();
    if let Err(e) = lang::check_with_bag(&program, &scope) {
        eprintln!("  {}", e);
        return false;
    }

    match interp::run_program(&program, env) {
        Ok(_)  => true,
        Err(e) => { eprintln!("  {}", e); false }
    }
}

fn quit(recorder: &mut Recorder, rl: &mut Line, history: Option<&PathBuf>) {
    recorder.discard_on_exit();
    lang::builtins::close_all_fds();
    if let Some(p) = history {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = rl.save_history(p);
    }
    println!("Goodbye.");
}

fn history_path() -> Option<PathBuf> {
    crate::registry::Registry::root().ok().map(|d| d.join("history"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The directives the prompt accepts and the ones completion offers are
    /// two lists; this is what keeps them the same list.
    #[test]
    fn directives_match_completion() {
        let fixed = ["help", "clear", "exit", "record::start", "record::end"];
        let mut handled: Vec<String> = fixed.iter().map(|s| s.to_string()).collect();
        for store in STORES {
            for op in ["add", "remove", "list"] {
                handled.push(format!("{}::{}", store.prefix, op));
            }
            if store.export.is_some() { handled.push(format!("{}::export", store.prefix)); }
            if store.import.is_some() { handled.push(format!("{}::import", store.prefix)); }
        }
        let mut offered: Vec<String> = complete::DIRECTIVES.iter().map(|s| s.to_string()).collect();
        handled.sort();
        offered.sort();
        assert_eq!(handled, offered);
    }
}
