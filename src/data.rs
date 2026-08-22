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

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::registry::Registry;

/// The data store is a registry of `.json` documents.
pub const REGISTRY: Registry = Registry {
    prefix:    "data",
    files_dir: "data",
    extension: Some("json"),
    noun:      "data entry",
    whole:     "your data store",
};

pub fn validate_name(name: &str) -> Result<(), String> {
    REGISTRY.validate_name(name)
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

    let path = REGISTRY.resolve(name)?.ok_or_else(|| format!(
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
    let path = REGISTRY.resolve(name)?.ok_or_else(|| format!(
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

/// List the entries in the store.
pub fn list() -> Result<Vec<(String, String)>, String> {
    invalidate_cache();
    REGISTRY.list()
}

// ── Sharing ───────────────────────────────────────────────────────────────

/// Write every entry into a zip at `path`.
pub fn export(path: &str) -> Result<(usize, PathBuf), String> {
    REGISTRY.export(path)
}

/// Read every `.json` file in the zip at `path` into the store.
///
/// Returns (added, replaced, skipped) — skipped being files whose name could
/// not be an entry name, or whose content is not a JSON object.
pub fn import(path: &str) -> Result<(usize, usize, Vec<String>), String> {
    let result = REGISTRY.import(path, |src| {
        serde_json::from_str::<serde_json::Value>(src).map(|v| v.is_object()).unwrap_or(false)
    });
    invalidate_cache();
    result
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
