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

Tab completes whatever is at the cursor — directives, paths, builtin,
bag, bin and data names, fields, types and your bindings. A dimmed hint
shows the rest of a word when only one fits; the right arrow accepts it.

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

  data::add <path.json> <n>
      Parse and store a .json file under <n>. The store keeps its own copy,
      so editing the original afterwards does not change the entry — run
      data::add again for that.

  data::remove <n>
      Remove a single document from the store.

  data::list
      List your data entries.

  data::export <path>
      Write every document into one .zip at <path>.

  data::import <path>
      Read every .json in a .zip into your store.

      Read a field with data::<name>.<field>, as an input or a binding:
        (1: i64, data::prompt.audit: String) : builtin::out -> {};
        ("new text": String) : builtin::trim -> {data::prompt.audit: String};
      A field keeps the type it has in the document, and must already exist.

  bin::add <path> <n>
      Copy the program at <path> into your programs under <n>. Build it
      first, with whatever toolchain it needs; any file that runs will do,
      a compiled binary or a script with a shebang. Call it from a pipe
      with bin::<n>: it takes any number of String arguments and gives
      back its exit code.

        ("--check": String) : bin::mytool -> {code: i64};

      The program inherits your terminal, so it prints where you can see it.
      There is no bin::export — a compiled program only runs on the machine
      it was built for.

  bin::remove <n>
      Remove a single program.

  bin::list
      List your programs.

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

      A file that does not parse is skipped. The rest are taken as they
      are, without a type check: import an archive you are willing to
      run — an incoherent script fails when you call it, not before.

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

The first pipe of a script declares its parameters. Called as a bag
entry, every slot in that list takes an argument, whether it holds a
name or a literal. The last pipe's binding is the script's return value.

Running a script from the shell:
  bullscript path/to/script.busc [arguments...]
Here only the named slots take an argument; a literal keeps its value."#;
