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

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Locations ─────────────────────────────────────────────────────────────

fn store_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").ok()
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()));

    match home {
        Some(h) => Ok(PathBuf::from(h).join(".bullscript")),
        None => Err(
            "cannot locate your home directory: neither HOME nor USERPROFILE is set, \
             so BullScript does not know where to keep your programs".to_string()
        ),
    }
}

fn bin_json_path() -> Result<PathBuf, String> {
    Ok(store_dir()?.join("bin.json"))
}

/// Where the store keeps its own copy of every registered program.
pub fn files_dir() -> Result<PathBuf, String> {
    Ok(store_dir()?.join("bin"))
}

// ── Name validation ───────────────────────────────────────────────────────

/// Program names must be BullScript identifiers.
///
/// This is what stops `bin::add ./tool ../../bin/sh` from writing outside
/// the store, and it also rules out names that could never be typed back as
/// `bin::my program`.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a program name cannot be empty".to_string());
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(format!(
            "'{}' is not a valid program name: it must start with a letter or underscore", name
        ));
    }
    if let Some(bad) = chars.find(|c| !(c.is_ascii_alphanumeric() || *c == '_')) {
        return Err(format!(
            "'{}' is not a valid program name: '{}' is not allowed — use letters, digits \
             and underscores only", name, bad
        ));
    }
    if name == "true" || name == "false" {
        return Err(format!("'{}' is a reserved word and cannot be a program name", name));
    }
    Ok(())
}

// ── Registry file ─────────────────────────────────────────────────────────

fn load() -> Result<BTreeMap<String, String>, String> {
    let path = bin_json_path()?;
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("could not read {}: {}", path.display(), e)),
    };

    if content.trim().is_empty() {
        return Ok(BTreeMap::new());
    }

    // A file that exists but cannot be read is never treated as an empty
    // store: the next save would write that emptiness back.
    serde_json::from_str(&content).map_err(|_| format!(
        "your program list at {} is damaged and cannot be read.\n  \
         BullScript will not overwrite it, so nothing has been lost yet.\n  \
         Open the file to repair it, or delete it to start with an empty store.",
        path.display()
    ))
}

fn save(map: &BTreeMap<String, String>) -> Result<(), String> {
    let dir = store_dir()?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {}", dir.display(), e))?;
    let path = bin_json_path()?;
    let content = serde_json::to_string_pretty(map)
        .map_err(|e| format!("could not encode the program list: {}", e))?;
    fs::write(&path, content)
        .map_err(|e| format!("could not write {}: {}", path.display(), e))
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

    let dir = files_dir()?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {}", dir.display(), e))?;
    let dest = dir.join(name);

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

    let mut map = load()?;
    let replaced = map.insert(name.to_string(), dest.display().to_string()).is_some();
    save(&map)?;
    Ok(replaced)
}

/// Remove a single program by name. Returns false if it wasn't present.
pub fn remove(name: &str) -> Result<bool, String> {
    let mut map = load()?;
    let existed = match map.remove(name) {
        Some(stored) => {
            // Only delete the file if it lives in our own bin directory.
            if let Ok(dir) = files_dir() {
                let p = PathBuf::from(&stored);
                if p.starts_with(&dir) {
                    let _ = fs::remove_file(&p);
                }
            }
            true
        }
        None => false,
    };
    if existed {
        save(&map)?;
    }
    Ok(existed)
}

/// List the programs in the store.
pub fn list() -> Result<Vec<(String, String)>, String> {
    Ok(load()?.into_iter().collect())
}

/// Resolve a program name to the stored path.
pub fn resolve(name: &str) -> Result<Option<PathBuf>, String> {
    Ok(load()?.get(name).map(PathBuf::from))
}

/// The stored path for `name`, or an error naming what is available.
///
/// Called at check time as well as run time, so a missing program is caught
/// before a script starts rather than part-way through.
pub fn require(name: &str) -> Result<PathBuf, String> {
    match resolve(name)? {
        Some(p) if p.is_file() => Ok(p),
        Some(p) => Err(format!(
            "'{}' is registered but its file is missing from {}.\n  \
             Add it again with `bin::add`, or remove it with `bin::remove {}`.",
            name, p.display(), name
        )),
        None => {
            let names: Vec<String> = load()?.into_keys().collect();
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
