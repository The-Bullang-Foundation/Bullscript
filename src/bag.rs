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

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::lang::ast::Program;
use crate::lang::check::Signature;

// ── Locations ─────────────────────────────────────────────────────────────

fn bag_dir() -> Result<PathBuf, String> {
    // HOME on Unix; USERPROFILE is the Windows equivalent and HOME is
    // normally unset there. Falling back to a relative path would silently
    // put the bag in whatever directory the user happened to launch from.
    let home = std::env::var("HOME").ok()
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()));

    match home {
        Some(h) => Ok(PathBuf::from(h).join(".bullscript")),
        None => Err(
            "cannot locate your home directory: neither HOME nor USERPROFILE is set, \
             so BullScript does not know where to keep the bag".to_string()
        ),
    }
}

fn bag_json_path() -> Result<PathBuf, String> {
    Ok(bag_dir()?.join("bag.json"))
}

/// Where the bag keeps its own copy of every registered script.
pub fn scripts_dir() -> Result<PathBuf, String> {
    Ok(bag_dir()?.join("scripts"))
}

// ── Name validation ───────────────────────────────────────────────────────

/// Bag entry names must be BullScript identifiers.
///
/// This is what stops `bag::add script.busc ../../etc/thing` from writing
/// outside the bag directory, and it also rules out names like `my entry`
/// that could never be typed back as `bag::my entry`.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a bag entry name cannot be empty".to_string());
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(format!(
            "'{}' is not a valid entry name: it must start with a letter or underscore", name
        ));
    }
    if let Some(bad) = chars.find(|c| !(c.is_ascii_alphanumeric() || *c == '_')) {
        return Err(format!(
            "'{}' is not a valid entry name: '{}' is not allowed — use letters, digits \
             and underscores only", name, bad
        ));
    }
    if name == "true" || name == "false" {
        return Err(format!("'{}' is a reserved word and cannot be an entry name", name));
    }
    Ok(())
}

// ── Registry file ─────────────────────────────────────────────────────────

fn load() -> Result<BTreeMap<String, String>, String> {
    let path = bag_json_path()?;
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        // No file yet simply means an empty bag.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("could not read {}: {}", path.display(), e)),
    };

    if content.trim().is_empty() {
        return Ok(BTreeMap::new());
    }

    // A file that exists but cannot be read is never treated as an empty bag:
    // the next save would write that emptiness back and destroy every entry.
    serde_json::from_str(&content).map_err(|_| format!(
        "your bag list at {} is damaged and cannot be read.\n  \
         BullScript will not overwrite it, so nothing has been lost yet.\n  \
         Open the file to repair it, or delete it to start with an empty bag.",
        path.display()
    ))
}

fn save(map: &BTreeMap<String, String>) -> Result<(), String> {
    let dir = bag_dir()?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {}", dir.display(), e))?;
    let path = bag_json_path()?;
    let content = serde_json::to_string_pretty(map)
        .map_err(|e| format!("could not encode the bag list: {}", e))?;
    fs::write(&path, content)
        .map_err(|e| format!("could not write {}: {}", path.display(), e))
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

    let path = resolve(name)?.ok_or_else(|| format!("no bag entry named '{}'", name))?;
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
    validate_name(name)?;

    let dir = scripts_dir()?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {}", dir.display(), e))?;
    let dest = dir.join(format!("{}.busc", name));
    fs::write(&dest, content)
        .map_err(|e| format!("could not write {}: {}", dest.display(), e))?;

    let mut map = load()?;
    let replaced = map.insert(name.to_string(), dest.display().to_string()).is_some();
    save(&map)?;
    invalidate_cache();
    Ok(replaced)
}

/// Remove a single entry by name. Returns false if it wasn't present.
pub fn remove(name: &str) -> Result<bool, String> {
    let mut map = load()?;
    let existed = match map.remove(name) {
        Some(stored) => {
            // Only delete the file if it lives in our own scripts directory.
            if let Ok(dir) = scripts_dir() {
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
    invalidate_cache();
    Ok(existed)
}

/// List user-added entries only — builtins never appear here.
pub fn list() -> Result<Vec<(String, String)>, String> {
    invalidate_cache();
    Ok(load()?.into_iter().collect())
}

/// Resolve a bag entry name to the stored script path.
pub fn resolve(name: &str) -> Result<Option<PathBuf>, String> {
    Ok(load()?.get(name).map(PathBuf::from))
}

// ── Sharing a bag ─────────────────────────────────────────────────────────
//
// `bag::export` writes every script in the bag into one zip; `bag::import`
// reads such a zip back into another bag. Between them a bag is something you
// can hand to someone.
//
// An imported archive is trusted: its scripts are copied in as they are, with
// no parse or type check. That is a deliberate choice — the archive is
// something the user went and fetched, and vouching for its coherence is
// theirs to do, exactly as it is for any file passed to `bag::add`. An
// incoherent entry fails when it is called, with the same error it would give
// had it been added directly.
//
// Only the file name of each entry is used, never the path recorded in the
// archive. That follows from what import means — "the scripts in the folder",
// flat, into a flat bag — and it means an entry cannot name a destination at
// all, so nothing lands outside the scripts directory.

/// The name given to an archive when the export path names a directory.
const DEFAULT_ARCHIVE: &str = "bullscript-bag.zip";

/// Write every bag entry into a zip at `path`.
///
/// Returns the number of scripts written and where they went.
pub fn export(path: &str) -> Result<(usize, PathBuf), String> {
    let entries = load()?;
    if entries.is_empty() {
        return Err("the bag is empty — there is nothing to export".to_string());
    }

    let dest = archive_destination(path);
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {}", parent.display(), e))?;
        }
    }

    let file = fs::File::create(&dest)
        .map_err(|e| format!("could not create {}: {}", dest.display(), e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut written = 0usize;
    for (name, stored) in &entries {
        // An entry whose file has gone missing is reported rather than
        // silently dropped: an export that quietly loses scripts is worse
        // than one that fails.
        let content = fs::read_to_string(stored).map_err(|e| format!(
            "could not read the script for '{}' at {}: {}\n  \
             The bag entry exists but its file does not. Remove it with \
             `bag::remove {}` or re-add it.",
            name, stored, e, name
        ))?;

        zip.start_file(format!("{}.busc", name), options)
            .map_err(|e| format!("could not add '{}' to the archive: {}", name, e))?;
        io::Write::write_all(&mut zip, content.as_bytes())
            .map_err(|e| format!("could not write '{}' into the archive: {}", name, e))?;
        written += 1;
    }

    zip.finish()
        .map_err(|e| format!("could not finish writing {}: {}", dest.display(), e))?;
    Ok((written, dest))
}

/// Read every `.busc` file in the zip at `path` into the bag.
///
/// Returns (added, replaced, skipped) — skipped being entries whose name
/// could not be a bag entry name.
pub fn import(path: &str) -> Result<(usize, usize, Vec<String>), String> {
    let file = fs::File::open(path)
        .map_err(|e| format!("could not open '{}': {}", path, e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("'{}' is not a readable zip archive: {}", path, e))?;

    let mut added    = 0usize;
    let mut replaced = 0usize;
    let mut skipped  = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)
            .map_err(|e| format!("could not read entry {} of '{}': {}", i, path, e))?;
        if entry.is_dir() {
            continue;
        }

        // The archive's own path is not consulted — only the file name.
        let raw = entry.name().to_string();
        let file_name = Path::new(&raw)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let Some(stem) = file_name.strip_suffix(".busc") else {
            // Anything that is not a script is not the bag's business.
            continue;
        };

        // The name still has to be callable as `bag::<name>`, so it must be an
        // identifier. This is not a trust check — an entry that cannot be
        // named cannot be used.
        if validate_name(stem).is_err() {
            skipped.push(file_name);
            continue;
        }

        let mut content = String::new();
        io::Read::read_to_string(&mut entry, &mut content)
            .map_err(|e| format!("could not read '{}' from the archive: {}", file_name, e))?;

        if store(stem, &content)? {
            replaced += 1;
        } else {
            added += 1;
        }
    }

    if added == 0 && replaced == 0 && skipped.is_empty() {
        return Err(format!("'{}' contains no .busc scripts", path));
    }
    Ok((added, replaced, skipped))
}

/// Where an export should write, given what the user typed.
///
/// A directory gets a default name inside it; anything else is taken as the
/// file to write, with `.zip` appended when it is missing so `bag::export
/// mybag` does the obvious thing.
fn archive_destination(path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    if p.is_dir() {
        return p.join(DEFAULT_ARCHIVE);
    }
    match p.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("zip") => p,
        _ => PathBuf::from(format!("{}.zip", path)),
    }
}
