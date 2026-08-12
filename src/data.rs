//! The data store: a name -> stored `.json` file registry.
//!
//! The same shape as the bag, for the same reasons, differing only in what it
//! holds. Where the bag stores `.busc` scripts you call, this stores `.json`
//! documents you read fields out of — a prompts file with an `audit` and a
//! `prod` section, a table of settings, anything keyed.
//!
//! `data::add` does the work up front: it reads the file, parses it as JSON,
//! and copies it into the store's own `data/` directory. The store therefore
//! owns its copy rather than pointing at wherever the file happened to live,
//! so moving or deleting the original cannot silently break an entry. The cost
//! is the same as the bag's: editing the original no longer updates the entry
//! — re-run `data::add` for that.
//!
//! Parsed documents are cached in memory for the life of the process. Every
//! `data::` directive refreshes that cache, so the store is current without a
//! restart and without re-reading on every field access.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ── Locations ─────────────────────────────────────────────────────────────

fn store_dir() -> Result<PathBuf, String> {
    // HOME on Unix; USERPROFILE is the Windows equivalent and HOME is
    // normally unset there. Falling back to a relative path would silently
    // put the store in whatever directory the user happened to launch from.
    let home = std::env::var("HOME").ok()
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()));

    match home {
        Some(h) => Ok(PathBuf::from(h).join(".bullscript")),
        None => Err(
            "cannot locate your home directory: neither HOME nor USERPROFILE is set, \
             so BullScript does not know where to keep your data".to_string()
        ),
    }
}

fn data_json_path() -> Result<PathBuf, String> {
    Ok(store_dir()?.join("data.json"))
}

/// Where the store keeps its own copy of every registered document.
pub fn files_dir() -> Result<PathBuf, String> {
    Ok(store_dir()?.join("data"))
}

// ── Name validation ───────────────────────────────────────────────────────

/// Data entry names must be BullScript identifiers.
///
/// This is what stops `data::add file.json ../../etc/thing` from writing
/// outside the data directory, and it also rules out names like `my entry`
/// that could never be typed back as `data::my entry.field`.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a data entry name cannot be empty".to_string());
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
    let path = data_json_path()?;
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        // No file yet simply means an empty store.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("could not read {}: {}", path.display(), e)),
    };

    if content.trim().is_empty() {
        return Ok(BTreeMap::new());
    }

    // A file that exists but cannot be read is never treated as an empty
    // store: the next save would write that emptiness back and destroy every
    // entry.
    serde_json::from_str(&content).map_err(|_| format!(
        "your data list at {} is damaged and cannot be read.\n  \
         BullScript will not overwrite it, so nothing has been lost yet.\n  \
         Open the file to repair it, or delete it to start with an empty store.",
        path.display()
    ))
}

fn save(map: &BTreeMap<String, String>) -> Result<(), String> {
    let dir = store_dir()?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {}", dir.display(), e))?;
    let path = data_json_path()?;
    let content = serde_json::to_string_pretty(map)
        .map_err(|e| format!("could not encode the data list: {}", e))?;
    fs::write(&path, content)
        .map_err(|e| format!("could not write {}: {}", path.display(), e))
}

// ── Parsed-document cache ─────────────────────────────────────────────────

static CACHE: Mutex<Option<HashMap<String, serde_json::Value>>> = Mutex::new(None);

/// Drop the parsed-document cache. Called by every `data::` directive so the
/// next access sees the current state of the store.
pub fn invalidate_cache() {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = None;
}

/// The parsed document stored under `name`.
pub fn document(name: &str) -> Result<serde_json::Value, String> {
    {
        let guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cache) = guard.as_ref() {
            if let Some(doc) = cache.get(name) {
                return Ok(doc.clone());
            }
        }
    }

    let path = resolve(name)?.ok_or_else(|| format!(
        "'{}' is not in your data store — run `data::list` to see what is", name
    ))?;
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("could not read the data for '{}' at {}: {}", name, path.display(), e))?;
    let doc: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("the data for '{}' at {} is not valid JSON: {}", name, path.display(), e))?;

    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    guard.get_or_insert_with(HashMap::new).insert(name.to_string(), doc.clone());
    Ok(doc)
}

/// Write `doc` back to the file stored under `name`, and refresh the cache.
///
/// Called once per assignment rather than at the end of the program: a write
/// takes effect when it runs, the same as `builtin::out`. A program that fails
/// halfway therefore leaves the writes that already happened in place.
pub fn write_document(name: &str, doc: &serde_json::Value) -> Result<(), String> {
    let path = resolve(name)?.ok_or_else(|| format!(
        "'{}' is not in your data store", name
    ))?;
    let content = serde_json::to_string_pretty(doc)
        .map_err(|e| format!("could not encode the data for '{}': {}", name, e))?;
    fs::write(&path, content)
        .map_err(|e| format!("could not write {}: {}", path.display(), e))?;

    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    guard.get_or_insert_with(HashMap::new).insert(name.to_string(), doc.clone());
    Ok(())
}

// ── Registry operations ───────────────────────────────────────────────────

pub fn add(path: &str, name: &str) -> Result<bool, String> {
    validate_name(name)?;

    if !path.ends_with(".json") {
        return Err(format!(
            "'{}' is not a .json file — the data store only holds JSON documents", path
        ));
    }
    if !Path::new(path).exists() {
        return Err(format!("'{}' does not exist", path));
    }

    let src = fs::read_to_string(path)
        .map_err(|e| format!("could not read '{}': {}", path, e))?;

    // Parsed up front, exactly as `bag::add` parses and checks a script: an
    // entry that cannot be read is caught when it is added, not when some
    // pipe first reaches for a field.
    let doc: serde_json::Value = serde_json::from_str(&src).map_err(|e| format!(
        "error: '{}' is not valid JSON\n  {}", path, e
    ))?;
    if !doc.is_object() {
        return Err(format!(
            "'{}' is a JSON {}, not an object — a data entry must have named fields \
             to read, like {{\"audit\": \"...\"}}",
            path, json_kind(&doc)
        ));
    }

    store(name, &src)
}

/// Write `content` into the store's data directory under `name` and register
/// it.
pub fn store(name: &str, content: &str) -> Result<bool, String> {
    validate_name(name)?;

    let dir = files_dir()?;
    fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {}", dir.display(), e))?;
    let dest = dir.join(format!("{}.json", name));
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
            // Only delete the file if it lives in our own data directory.
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
    invalidate_cache();
    Ok(existed)
}

/// List the entries in the store.
pub fn list() -> Result<Vec<(String, String)>, String> {
    invalidate_cache();
    Ok(load()?.into_iter().collect())
}

/// Resolve a data entry name to the stored file path.
pub fn resolve(name: &str) -> Result<Option<PathBuf>, String> {
    Ok(load()?.get(name).map(PathBuf::from))
}

/// What a JSON value is, for an error message.
pub fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null      => "null",
        serde_json::Value::Bool(_)   => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_)  => "array",
        serde_json::Value::Object(_) => "object",
    }
}

// ── Sharing a data store ──────────────────────────────────────────────────
//
// The same pair as the bag's, for the same reason: a store you can hand to
// someone. `data::export` writes every document into one zip; `data::import`
// reads such a zip into another store.
//
// An imported archive is trusted in the same sense — its documents are copied
// in as they are. They are still parsed on the way in, because `data::add`
// parses too and an entry that is not JSON could never be read from.

/// The name given to an archive when the export path names a directory.
const DEFAULT_ARCHIVE: &str = "bullscript-data.zip";

/// Write every entry into a zip at `path`.
pub fn export(path: &str) -> Result<(usize, PathBuf), String> {
    let entries = load()?;
    if entries.is_empty() {
        return Err("your data store is empty — there is nothing to export".to_string());
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
        // silently dropped: an export that quietly loses documents is worse
        // than one that fails.
        let content = fs::read_to_string(stored).map_err(|e| format!(
            "could not read the data for '{}' at {}: {}\n  \
             The entry exists but its file does not. Remove it with \
             `data::remove {}` or re-add it.",
            name, stored, e, name
        ))?;

        zip.start_file(format!("{}.json", name), options)
            .map_err(|e| format!("could not add '{}' to the archive: {}", name, e))?;
        io::Write::write_all(&mut zip, content.as_bytes())
            .map_err(|e| format!("could not write '{}' into the archive: {}", name, e))?;
        written += 1;
    }

    zip.finish()
        .map_err(|e| format!("could not finish writing {}: {}", dest.display(), e))?;
    Ok((written, dest))
}

/// Read every `.json` file in the zip at `path` into the store.
///
/// Returns (added, replaced, skipped) — skipped being entries whose name
/// could not be an entry name, or whose content is not a JSON object.
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

        let Some(stem) = file_name.strip_suffix(".json") else {
            continue;
        };

        // The name still has to be usable as `data::<name>.field`, so it must
        // be an identifier. This is not a trust check — an entry that cannot
        // be named cannot be read from.
        if validate_name(stem).is_err() {
            skipped.push(file_name);
            continue;
        }

        let mut content = String::new();
        io::Read::read_to_string(&mut entry, &mut content)
            .map_err(|e| format!("could not read '{}' from the archive: {}", file_name, e))?;

        // Not a trust check either: a document that is not a JSON object has
        // no named fields, so nothing could ever read from it.
        match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(v) if v.is_object() => {}
            _ => {
                skipped.push(file_name);
                continue;
            }
        }

        if store(stem, &content)? {
            replaced += 1;
        } else {
            added += 1;
        }
    }

    if added == 0 && replaced == 0 && skipped.is_empty() {
        return Err(format!("'{}' contains no .json documents", path));
    }
    Ok((added, replaced, skipped))
}

/// Where an export should write, given what the user typed.
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

// ── Field access ──────────────────────────────────────────────────────────
//
// JSON has one number type and BullScript has two, so an integral number is
// i64 and a fractional one is f64. That is the only place the two type systems
// disagree about a *scalar*; objects, arrays and null are not BullScript values
// at all, and a path landing on one is an error naming what is actually there.

use crate::lang::ast::{DataRef, PathSeg};
use crate::lang::types::BsType;

/// Resolve one path segment against an object, given the variables in scope.
///
/// `keys` supplies the value of a `[key]` segment. A `Field` segment does not
/// need it, which is why the whole path can be walked at check time as long as
/// every segment is written out.
fn step<'a>(
    cur:  &'a serde_json::Value,
    seg:  &PathSeg,
    keys: &dyn Fn(&str) -> Option<String>,
    r:    &DataRef,
    done: &[PathSeg],
) -> Result<&'a serde_json::Value, String> {
    let obj = cur.as_object().ok_or_else(|| {
        let so_far: String = done.iter().map(|p| p.to_string()).collect();
        format!(
            "'data::{}{}' is {}, so it has no fields",
            r.entry, so_far, json_kind(cur)
        )
    })?;

    let name = match seg {
        PathSeg::Field(n) => n.clone(),
        PathSeg::Key(v) => keys(v).ok_or_else(|| format!(
            "the value of '{}' is not known here", v
        ))?,
    };

    obj.get(&name).ok_or_else(|| {
        let mut names: Vec<&str> = obj.keys().map(String::as_str).collect();
        names.sort_unstable();
        match seg {
            PathSeg::Field(_) => format!(
                "'{}' has no field '{}'{}",
                r, name,
                if names.is_empty() { String::new() }
                else { format!(" — it has {}", names.join(", ")) }
            ),
            // A key came from a variable, so the value is worth quoting: the
            // mistake is in the data being passed, not in the source.
            PathSeg::Key(v) => format!(
                "'{}' holds \"{}\", which is not a field of 'data::{}'{}",
                v, name, r.entry,
                if names.is_empty() { String::new() }
                else { format!(" — it has {}", names.join(", ")) }
            ),
        }
    })
}

/// Walk a fully written-out path. Used where no variables are available.
fn walk<'a>(doc: &'a serde_json::Value, r: &DataRef) -> Result<&'a serde_json::Value, String> {
    let mut cur = doc;
    for (i, seg) in r.path.iter().enumerate() {
        cur = step(cur, seg, &|_| None, r, &r.path[..i])?;
    }
    Ok(cur)
}

/// The object a path reaches just before its final segment.
fn parent_of<'a>(doc: &'a serde_json::Value, r: &DataRef) -> Result<&'a serde_json::Value, String> {
    let mut cur = doc;
    for (i, seg) in r.path[..r.path.len() - 1].iter().enumerate() {
        cur = step(cur, seg, &|_| None, r, &r.path[..i])?;
    }
    Ok(cur)
}

/// The BullScript type of the field a `DataRef` names.
///
/// A written-out path names one field, so its type is that field's. A path
/// ending in `[key]` could name any field of the object it selects from, so it
/// has one type only if **every** field of that object has that type — which
/// is what makes a dynamic key statically checkable rather than a hole in the
/// checking. For a document whose fields are all Strings, `data::norm[lang]`
/// is provably a String whatever `lang` turns out to hold.
///
/// A `[key]` earlier in the path is not supported for the same reason in
/// reverse: the objects it could select are not required to agree in shape, so
/// there is nothing to check the rest of the path against.
pub fn field_type(r: &DataRef) -> Result<BsType, String> {
    let doc = document(&r.entry)?;

    if let Some(i) = r.path.iter().position(|s| matches!(s, PathSeg::Key(_))) {
        if i != r.path.len() - 1 {
            return Err(format!(
                "'{}': a [key] may only be the last step of a path, because what it \
                 selects is not known until the pipe runs",
                r
            ));
        }
        let parent = parent_of(&doc, r)?;
        let obj = parent.as_object().ok_or_else(|| format!(
            "'{}' selects from {}, which has no fields", r, json_kind(parent)
        ))?;
        if obj.is_empty() {
            return Err(format!("'data::{}' has no fields to select from", r.entry));
        }

        // Every field must agree, or the annotation could not be right for
        // every key the variable might hold.
        let mut agreed: Option<BsType> = None;
        for (name, v) in obj {
            let ty = value_type(v).ok_or_else(|| format!(
                "'{}' could select field '{}', which is {} — not one of BullScript's \
                 four types",
                r, name, json_kind(v)
            ))?;
            match agreed {
                None => agreed = Some(ty),
                Some(prev) if prev == ty => {}
                Some(prev) => return Err(format!(
                    "'{}' could select any field of 'data::{}', but they do not all \
                     have the same type — '{}' is {} where another is {}. A [key] \
                     needs every field to agree, so that its type is known before \
                     the pipe runs.",
                    r, r.entry, name, ty, prev
                )),
            }
        }
        return Ok(agreed.expect("the object was checked non-empty"));
    }

    let v = walk(&doc, r)?;
    value_type(v).ok_or_else(|| format!(
        "'{}' is {}, which is not one of BullScript's four types \
         (i64, f64, bool, String)",
        r, json_kind(v)
    ))
}

/// The BullScript type of a JSON scalar, or None for object, array and null.
pub fn value_type(v: &serde_json::Value) -> Option<BsType> {
    match v {
        serde_json::Value::Bool(_)   => Some(BsType::Bool),
        serde_json::Value::String(_) => Some(BsType::String),
        // An integral number is i64; anything with a fraction or exponent is
        // f64. JSON does not distinguish them, so the value decides.
        serde_json::Value::Number(n) if n.is_i64() => Some(BsType::I64),
        serde_json::Value::Number(_) => Some(BsType::F64),
        _ => None,
    }
}

/// Resolve a path's `[key]` segments against a runtime environment.
///
/// A key must hold a String: a field name is a String, and BullScript will not
/// quietly turn a number into one.
fn keys_from<'a>(env: &'a crate::lang::interp::Env) -> impl Fn(&str) -> Option<String> + 'a {
    move |name: &str| match env.get(name) {
        Some(crate::lang::types::Value::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Walk a path with variables available, for use at run time.
fn walk_with<'a>(
    doc: &'a serde_json::Value,
    r:   &DataRef,
    env: &crate::lang::interp::Env,
) -> Result<&'a serde_json::Value, String> {
    let keys = keys_from(env);
    let mut cur = doc;
    for (i, seg) in r.path.iter().enumerate() {
        cur = step(cur, seg, &keys, r, &r.path[..i])?;
    }
    Ok(cur)
}

/// The name a path's final segment resolves to.
fn final_name(r: &DataRef, env: &crate::lang::interp::Env) -> Result<String, String> {
    match r.path.last().expect("a DataRef always has at least one segment") {
        PathSeg::Field(n) => Ok(n.clone()),
        PathSeg::Key(v) => keys_from(env)(v).ok_or_else(|| format!(
            "'{}' does not hold a String, so it cannot name a field", v
        )),
    }
}

/// The value of the field a `DataRef` names.
pub fn read_field(
    r:   &DataRef,
    env: &crate::lang::interp::Env,
) -> Result<crate::lang::types::Value, String> {
    use crate::lang::types::Value;
    let doc = document(&r.entry)?;
    let v = walk_with(&doc, r, env)?;
    match v {
        serde_json::Value::Bool(b)   => Ok(Value::Bool(*b)),
        serde_json::Value::String(s) => Ok(Value::Str(s.clone())),
        serde_json::Value::Number(n) if n.is_i64() => Ok(Value::I64(n.as_i64().unwrap())),
        serde_json::Value::Number(n) => Ok(Value::F64(n.as_f64().unwrap_or(0.0))),
        other => Err(format!(
            "'{}' is {}, which is not one of BullScript's four types", r, json_kind(other)
        )),
    }
}

/// Write `value` into the field a `DataRef` names, and persist the document.
///
/// The field must already exist and keep its type — both are established by
/// the type checker before anything runs, so a failure here means the document
/// changed underneath a running program.
pub fn write_field(
    r:     &DataRef,
    value: &crate::lang::types::Value,
    env:   &crate::lang::interp::Env,
) -> Result<(), String> {
    use crate::lang::types::Value;

    let mut doc = document(&r.entry)?;

    // Resolve every name first, while the document is still borrowed
    // immutably — then walk again to get the mutable slot.
    let mut names: Vec<String> = Vec::with_capacity(r.path.len());
    {
        let keys = keys_from(env);
        let mut cur = &doc;
        for (i, seg) in r.path.iter().enumerate() {
            let name = match seg {
                PathSeg::Field(n) => n.clone(),
                PathSeg::Key(v) => keys(v).ok_or_else(|| format!(
                    "'{}' does not hold a String, so it cannot name a field", v
                ))?,
            };
            // Checks the step exists and gives the same error as a read would.
            cur = step(cur, seg, &keys, r, &r.path[..i])?;
            names.push(name);
        }
    }
    let _ = final_name(r, env)?;

    let mut cur = &mut doc;
    for part in &names[..names.len() - 1] {
        cur = cur.get_mut(part).ok_or_else(|| format!("'{}' has no field '{}'", r, part))?;
    }
    let last = names.last().expect("a DataRef always has at least one segment");
    let slot = cur.get_mut(last).ok_or_else(|| format!("'{}' has no field '{}'", r, last))?;

    *slot = match value {
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Str(s)  => serde_json::Value::String(s.clone()),
        Value::I64(n)  => serde_json::Value::Number((*n).into()),
        Value::F64(x)  => serde_json::Number::from_f64(*x)
            .map(serde_json::Value::Number)
            .ok_or_else(|| format!(
                "cannot write {} into '{}': JSON has no representation for it", x, r
            ))?,
    };

    write_document(&r.entry, &doc)
}
