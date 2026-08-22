//! The bin store: a name -> stored executable registry.
//!
//! The third of the three stores, and the smallest. The bag holds scripts you
//! call, `data` holds documents you read fields out of, and this holds
//! programs you run.
//!
//! `bin::add <path> <name>` copies an already-built program at `path` into
//! the store under `name`, exactly as `bag::add` does for a script. Building
//! the program is your job, with whatever toolchain it needs; the store only
//! keeps the result. From then on the program is reachable by that name from
//! any pipe:
//!
//! ```text
//! ("--check": String) : bin::mytool -> {code: i64};
//! ```
//!
//! It has only `add`, `list` and `remove`. There is no export or import,
//! deliberately: a compiled program is tied to one operating system and one
//! architecture, so an archive of binaries handed to someone else would
//! largely not run. Scripts and JSON travel; executables do not.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::registry::Registry;

/// The bin store is a registry of programs, stored as they are: a program
/// keeps no extension, so `bin::mytool` is the file `bin/mytool`.
pub const REGISTRY: Registry = Registry {
    prefix:    "bin",
    files_dir: "bin",
    extension: None,
    noun:      "program",
    whole:     "your programs",
};

pub fn validate_name(name: &str) -> Result<(), String> {
    REGISTRY.validate_name(name)
}

// ── Registry operations ───────────────────────────────────────────────────

/// Store the program at `path` under `name`.
///
/// `path` must be an existing file — the built program, not the project it
/// came from. Any file will do: a compiled binary, or a script with a
/// shebang line. It is copied, so the original can be rebuilt or deleted
/// afterwards without affecting the stored copy.
pub fn add(path: &str, name: &str) -> Result<bool, String> {
    validate_name(name)?;

    let given = Path::new(path);
    if !given.exists() {
        return Err(format!("'{}' does not exist", path));
    }
    if !given.is_file() {
        return Err(format!(
            "'{}' is not a file — give the path to the built program itself, \
             not the directory it is in", path
        ));
    }

    store(given, name)
}

/// Copy an already-built program into the store under `name`.
pub fn store(artifact: &Path, name: &str) -> Result<bool, String> {
    validate_name(name)?;

    let dest = REGISTRY.path_for(name)?;
    let dir = dest.parent().expect("path_for always has a parent");
    fs::create_dir_all(dir)
        .map_err(|e| format!("could not create {}: {}", dir.display(), e))?;

    // Removed first rather than overwritten: replacing a file that is running
    // fails on Windows and rewrites the running image on some Unixes.
    let _ = fs::remove_file(&dest);
    fs::copy(artifact, &dest).map_err(|e| format!(
        "could not copy {} to {}: {}", artifact.display(), dest.display(), e
    ))?;

    // A copy does not necessarily carry the executable bit, and a program that
    // cannot be executed is not worth storing.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest)
            .map_err(|e| format!("could not read {}: {}", dest.display(), e))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest, perms)
            .map_err(|e| format!("could not make {} executable: {}", dest.display(), e))?;
    }

    REGISTRY.register(name, &dest)
}

/// Remove a single program by name. Returns false if it wasn't present.
pub fn remove(name: &str) -> Result<bool, String> {
    REGISTRY.remove(name)
}

/// List the programs in the store.
pub fn list() -> Result<Vec<(String, String)>, String> {
    REGISTRY.list()
}

/// The stored path for `name`, or an error naming what is available.
///
/// Called at check time as well as run time, so a missing program is caught
/// before a script starts rather than part-way through.
pub fn require(name: &str) -> Result<PathBuf, String> {
    match REGISTRY.resolve(name)? {
        Some(p) if p.is_file() => Ok(p),
        Some(p) => Err(format!(
            "'{}' is registered but its file is missing from {}.\n  \
             Add it again with `bin::add`, or remove it with `bin::remove {}`.",
            name, p.display(), name
        )),
        None => {
            let names: Vec<String> = REGISTRY.load()?.into_keys().collect();
            Err(if names.is_empty() {
                format!("'{}' is not in your programs — your bin store is empty", name)
            } else {
                format!(
                    "'{}' is not in your programs — you have {}",
                    name, names.join(", ")
                )
            })
        }
    }
}

/// Run a stored program with `args`, and return its exit code.
///
/// The terminal is inherited, so the program prints where the user can see it
/// and can read input. The arguments are passed as separate argv entries and
/// never through a shell, so a value containing a space or a quote cannot
/// change what runs.
///
/// A program killed by a signal has no exit code of its own; the shell
/// convention of 128 + the signal number is used, which is what `$?` would
/// report for the same command.
pub fn run(name: &str, args: &[String]) -> Result<i64, String> {
    let path = require(name)?;

    let status = Command::new(&path)
        .args(args)
        .status()
        .map_err(|e| format!("could not run '{}' ({}): {}", name, path.display(), e))?;
    crate::lang::builtins::mark_terminal_dirty();

    match status.code() {
        Some(c) => Ok(c as i64),
        None => {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(sig) = status.signal() {
                    return Ok(128 + sig as i64);
                }
            }
            Ok(-1)
        }
    }
}
