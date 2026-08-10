//! Zed extension for BullScript.
//!
//! Zed will not recognise a new language without an extension, so `.busc`
//! needs one just as `.bu` does. It tells Zed to run `bullscript lsp`, which
//! reports syntax and type errors from the same lexer, parser and checker the
//! interpreter uses.

use zed_extension_api::{self as zed, LanguageServerId, Result};

struct BullscriptExtension;

impl zed::Extension for BullscriptExtension {
    fn new() -> Self {
        BullscriptExtension
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        // The worktree's PATH is the user's shell environment rather than
        // Zed's, which matters for a cargo-installed binary that a GUI
        // launched from a dock would not otherwise see.
        let path = worktree.which("bullscript").ok_or_else(|| {
            "bullscript was not found on PATH.\n\
             Install it with:\n  \
             cargo install --git https://github.com/The-Bullang-Foundation/Bullscript.git\n\
             then restart Zed."
                .to_string()
        })?;

        Ok(zed::Command {
            command: path,
            args: vec!["lsp".to_string()],
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(BullscriptExtension);
