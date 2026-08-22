//! The bag: a name -> stored `.busc` script registry.
//!
//! Only `.busc` files are accepted — no `.bu`, no arbitrary pre-built
//! binaries. Every entry is, by construction, something written inside
//! BullScript's own narrow four-type pool, so there is no path where a random
//! executable becomes a bag-listed callable with no declared prototype.
//!
//! `bag::add` does the work up front: it reads the file, parses it, type
//! checks it, and copies the source into the bag's own `scripts/` directory.
//! The bag therefore owns its copy rather than pointing at wherever the file
//! happened to live, so moving or deleting the original cannot silently break
//! an entry. The cost is that editing the original no longer updates the
//! entry — re-run `bag::add` for that.
//!
//! Parsed programs are cached in memory for the life of the process. Every
//! `bag::` directive (`add`, `remove`, `list`) refreshes that cache, so the
//! bag is always current without a restart and without re-reading on every
//! call.
//!
//! `builtin::*` entries are never stored here — see `lang::builtins`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::lang::ast::Program;
use crate::lang::check::Signature;
use crate::registry::Registry;

/// The bag is a registry of `.busc` scripts.
pub const REGISTRY: Registry = Registry {
    prefix:    "bag",
    files_dir: "scripts",
    extension: Some("busc"),
    noun:      "bag entry",
    whole:     "the bag",
};

pub fn validate_name(name: &str) -> Result<(), String> {
    REGISTRY.validate_name(name)
}

// ── Parsed-program cache ──────────────────────────────────────────────────

static CACHE: Mutex<Option<HashMap<String, Program>>> = Mutex::new(None);

/// Drop the parsed-program cache. Called by every `bag::` directive so the
/// next call sees the current state of the bag.
pub fn invalidate_cache() {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

/// The parsed program for a bag entry, loading and caching it on first use.
pub fn program(name: &str) -> Result<Program, String> {
    {
        let guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cache) = guard.as_ref() {
            if let Some(p) = cache.get(name) {
                return Ok(p.clone());
            }
        }
    }

    let path = REGISTRY.resolve(name)?.ok_or_else(|| format!("no bag entry named '{}'", name))?;
    let src = fs::read_to_string(&path).map_err(|e| format!(
        "bag entry '{}' could not be read from {}: {}", name, path.display(), e
    ))?;
    let pipes = crate::lang::parse_source(&src)
        .map_err(|e| format!("bag entry '{}' failed to parse: {}", name, e))?;
    if pipes.is_empty() {
        return Err(format!("bag entry '{}' is empty", name));
    }

    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    guard.get_or_insert_with(HashMap::new).insert(name.to_string(), pipes.clone());
    Ok(pipes)
}

/// The declared signature of a bag entry: first pipe's input types, last
/// pipe's binding type. Used by the static pass to check `bag::` calls.
pub fn signature(name: &str) -> Result<Signature, String> {
    let pipes = program(name)?;
    crate::lang::check::check_program(&pipes, &HashMap::new(), &|inner| signature(inner))
        .map_err(|e| format!("bag entry '{}' does not type check: {}", name, e))
}

// ── Public operations ─────────────────────────────────────────────────────

/// Register `path` under `name`.
///
/// The script is parsed and type checked before anything is written, then
/// copied into the bag's own scripts directory. Returns true if an existing
/// entry of the same name was replaced.
pub fn add(path: &str, name: &str) -> Result<bool, String> {
    validate_name(name)?;

    if !path.ends_with(".busc") {
        return Err(format!(
            "'{}' is not a .busc file — the bag only stores BullScript scripts", path
        ));
    }
    if !Path::new(path).exists() {
        return Err(format!("'{}' does not exist", path));
    }

    let src = fs::read_to_string(path)
        .map_err(|e| format!("could not read '{}': {}", path, e))?;

    let pipes = crate::lang::parse_source(&src).map_err(|e| format!(
        "error: invalid BullScript syntax in the script you tried to save\n  {}\n  {}",
        path, e
    ))?;
    if pipes.is_empty() {
        return Err(format!("'{}' contains no pipes — there is nothing to save", path));
    }
    crate::lang::check::check_program(&pipes, &HashMap::new(), &|inner| signature(inner))
        .map_err(|e| format!(
            "error: invalid BullScript syntax in the script you tried to save\n  {}\n  {}",
            path, e
        ))?;

    store(name, &src)
}

/// Write `content` into the bag's scripts directory under `name` and register
/// it. Used by `bag::add` and by `record::end`, which already holds the text.
pub fn store(name: &str, content: &str) -> Result<bool, String> {
    let replaced = REGISTRY.store_text(name, content)?;
    invalidate_cache();
    Ok(replaced)
}

/// Remove a single entry by name. Returns false if it wasn't present.
pub fn remove(name: &str) -> Result<bool, String> {
    let existed = REGISTRY.remove(name)?;
    invalidate_cache();
    Ok(existed)
}

/// List user-added entries only — builtins never appear here.
pub fn list() -> Result<Vec<(String, String)>, String> {
    invalidate_cache();
    REGISTRY.list()
}

// ── Sharing a bag ─────────────────────────────────────────────────────────
//
// An imported archive is trusted as far as coherence goes: its scripts are
// copied in as they are, with no type check. That is a deliberate choice —
// the archive is something the user went and fetched, and vouching for it is
// theirs to do, exactly as it is for any file passed to `bag::add`. A script
// that does not even parse is skipped, though, the same way the data store
// skips a file that is not JSON: an entry that could never run is not an
// entry.

/// Write every bag entry into a zip at `path`.
pub fn export(path: &str) -> Result<(usize, PathBuf), String> {
    REGISTRY.export(path)
}

/// Read every `.busc` file in the zip at `path` into the bag.
///
/// Returns (added, replaced, skipped) — skipped being files whose name could
/// not be an entry name or whose content does not parse.
pub fn import(path: &str) -> Result<(usize, usize, Vec<String>), String> {
    let result = REGISTRY.import(path, |src| {
        crate::lang::parse_source(src).map(|p| !p.is_empty()).unwrap_or(false)
    });
    invalidate_cache();
    result
}
