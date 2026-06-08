pub mod modes;
mod bullscript_ui;

use clap::{Parser as ClapParser, Subcommand};

#[derive(ClapParser)]
#[command(
    name    = "bullscript",
    version = env!("CARGO_PKG_VERSION"),
    about   = "Bullscript — the language registry.\n\n\
               Defines the .bu language: grammar, parser, AST, type system, and standard library.\n\
               For transpiling, formatting, scaffolding, and LSP support, use bullarchy."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
	Update,
}

const BANNER: &str = r#"
 ____        _ _               _       _
|  _ \      | | |             (_)     | |
| |_) |_   _| | |___  ___ _ __ _ _ __ | |_
|  _ <| | | | | / __|/ __| '__| | '_ \| __|
| |_) | |_| | | \__ \ (__| |  | | |_) | |_
|____/ \__,_|_|_|___/\___|_|  |_| .__/ \__|
                                | |
                                |_|

Bullscript 1.0.0 — interactive Bullang tool
Type 'help' for available commands.
"#;

fn main() {
	let cli = Cli::parse();

	match cli.command {
		Some(Command::Update) => modes::cmd_update(),
		None => {
		    println!("{}", BANNER);
			bullscript_ui::run();
		}
	}

}
