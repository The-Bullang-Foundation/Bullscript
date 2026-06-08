use crate::modes;

pub fn run() {
	let update_handle = std::thread::spawn(|| {
		let remote = modes::remote_head(modes::REPO, "main")?;
		let installed = modes::installed_hash("bullarch", modes::REPO, "main")?;
		if installed == remote {
			None
		} else {
			Some(format!(
				"\nA new version of bullarchy is available. Run `bullarchy update` to install."
			))
		}
	});

	if let Ok(Some(msg)) = update_handle.join() {
    println!("{}", msg);
	}

    let mut rl = rustyline::DefaultEditor::new()
        .expect("failed to initialise line editor");

    loop {
        let line = match rl.readline("command -> ") {
            Ok(l)                                            => l,
            Err(rustyline::error::ReadlineError::Eof)        => { println!("Goodbye."); break; }
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(e) => { eprintln!("Read error: {}", e); break; }
        };
        let line = line.trim();
        if line.is_empty() { continue; }
        let _ = rl.add_history_entry(line);

        // Split into at most 4 parts so output_file path is kept whole
        let parts: Vec<&str> = line.splitn(4, ' ').collect();

        match parts[0] {
            "help"  => modes::help::run(),
            "build" => modes::build::run(),
            "test"  => modes::test::run(),

            "run" => {
                if parts.len() < 2 {
                    eprintln!("  Usage: run <file.bu>");
                } else {
                    modes::run::run(parts[1]);
                }
            }

            "arrow" => {
                if parts.len() < 4 {
                    eprintln!("  Usage: arrow <first> <second> <output_file>");
                } else {
                    modes::arrow::run(parts[1], parts[2], parts[3]);
                }
            }

            "exit" => { println!("Goodbye."); break; }

            other => eprintln!(
                "  Unknown command: '{}'. Type 'help' for available commands.",
                other
            ),
        }
    }
}