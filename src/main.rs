mod bag;
mod help;
mod lang;
mod record;
mod repl;
mod script;

const USAGE: &str = "\
Usage:
  bullscript                              Start the interactive prompt
  bullscript <file.busc> [arguments...]   Run a script
  bullscript --help                       Show this message
  bullscript --version                    Show the version";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        None => repl::run(),

        Some("--help") | Some("-h") => println!("{}", USAGE),

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
