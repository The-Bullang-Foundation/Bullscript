mod bag;
// `bin` is a Rust keyword-adjacent directory name in cargo layouts, so the
// module file is bin_store.rs while the namespace users type is `bin::`.
#[path = "bin_store.rs"]
mod bin_store;
mod complete;
mod data;
mod help;
mod lang;
mod lsp;
mod record;
mod repl;
mod script;

const USAGE: &str = "\
Usage:
  bullscript                              Start the interactive prompt
  bullscript <file.busc> [arguments...]   Run a script
  bullscript lsp                          Run the language server on stdio
  bullscript --help                       Show this message
  bullscript --version                    Show the version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        None => repl::run(),

        Some("--help") | Some("-h") => println!("{}", USAGE),
        // The language server: syntax and type diagnostics for .busc files,
        // from the same lexer, parser and checker the interpreter uses.
        Some("lsp") => lsp::run(),

        Some("--version") | Some("-V") => {
            println!("bullscript {}", env!("CARGO_PKG_VERSION"));
        }

        Some(flag) if flag.starts_with('-') => {
            eprintln!("Unknown option '{}'.\n\n{}", flag, USAGE);
            std::process::exit(1);
        }

        Some(path) => script::run(path, &args[1..]),
    }
}
