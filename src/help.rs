use crate::lang::builtins;

pub fn run() {
    println!("{}", HEAD);
    println!("Builtins:\n");
    for name in builtins::NAMES {
        if let Some((params, ret)) = builtins::prototype(name) {
            let ps: Vec<String> = params.iter().map(|t| t.to_string()).collect();
            println!("  builtin::{:<9} ({}) -> {}", name, ps.join(", "), ret);
        }
    }
    println!("{}", TAIL);
}

const HEAD: &str = r#"BullScript is a small pipe-only language. Type pipes directly at the
prompt — there's no dispatcher, no subcommands for running code.

  ( <input>: <type>, ... ) : <callee-or-expr> -> { <name>: <type> } ;

  Example:
    (a: i64, b: i64) : builtin::add      -> {sum: i64};
    (sum: i64)       : sum * 2           -> {doubled: i64};
    (1: i64, "done\n": String) : builtin::out -> {ok: bool};

Every input and every created binding always carries an explicit type.
Use {} to discard a pipe's result instead of binding it.

Types: i64, f64, bool, String. Literals include true and false.

Every line is fully type checked before any of it runs, so a mistake
never leaves half its work behind.
"#;

const TAIL: &str = r#"
File descriptors: 1 is stdout, 2 is stderr, and builtin::open returns
3 and upward. builtin::out writes exactly what you give it — add '\n'
yourself when you want a line break.

Directives (typed bare, not prefixed with anything):

  help
      Print this message.

  bag::add <path> <name>
      Parse, check and store a .busc file in the bag under <name>.
      The bag keeps its own copy, so editing the original afterwards
      does not change the entry — run bag::add again for that.

  bag::remove <name>
      Remove a single entry from the bag.

  bag::list
      List your bag entries (builtins never appear here).

  clear
      Clear the screen, as in a shell. Your bindings, your bag and your
      history are untouched.

  bag::export <path>
      Write every script in your bag into one .zip at <path>. If <path>
      is a directory the archive is named bullscript-bag.zip inside it;
      a missing .zip extension is added. Hand the file to someone and
      they can bag::import it.

  bag::import <path>
      Read every .busc file in the .zip at <path> into your bag, named
      after each file. An entry whose name already exists is replaced.

      Imported scripts are taken as they are: they are not parsed or
      type checked on the way in, the same as any file you point
      bag::add at. Import an archive you are willing to run — an
      incoherent script fails when you call it, not before.

  record::start
      Start capturing every pipe you type, until record::end. Only
      lines that ran successfully are captured.

  record::end
      Stop recording, preview what was captured, and optionally save it
      as a new bag entry.

  exit
      Quit BullScript. (Ctrl+D also works, and discards an in-progress
      recording with a warning.)

Calling a bag entry from a pipe: bag::<name>
  e.g.  (4: i64) : bag::double -> {r: i64};

The first pipe of a script declares its parameters — every slot in that
list takes an argument, whether it holds a name or a literal. The last
pipe's binding is the script's return value.

Running a script from the shell:
  bullscript path/to/script.busc [arguments...]"#;
