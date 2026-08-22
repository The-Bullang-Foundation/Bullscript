//! A registry: a name -> stored file map under `~/.bullscript`.
//!
//! The bag, the data store and the bin store are the same thing holding
//! different files. Each keeps a JSON list of names in `~/.bullscript`, its
//! own copy of every registered file in a directory beside it, and offers
//! the same operations: add, remove, list, export to a zip, import from one.
//! This module is that shape, once. A store is a `Registry` value plus the
//! handful of things that are actually particular to it — what the bag
//! checks before it accepts a script, how the bin store copies a program.
//!
//! Names are BullScript identifiers. That is what stops `bag::add x.busc
//! ../../etc/thing` from writing outside the store, and it also rules out a
//! name like `my entry` that could never be typed back as `bag::my entry`.
//!
//! A list file that exists but cannot be read is never treated as an empty
//! store: the next save would write that emptiness back and destroy every
//! entry. The error says so and asks the user to look at the file.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What distinguishes one store from another: where it lives and how it
/// talks about itself. Everything else is shared.
pub struct Registry {
    /// The namespace users type: "bag", "data", "bin". Also names the list
    /// file (`bag.json`) and the default archive (`bullscript-bag.zip`).
    pub prefix: &'static str,
    /// The directory under `~/.bullscript` holding the store's own copies.
    pub files_dir: &'static str,
    /// The extension given to stored copies and expected inside archives,
    /// without the dot. None for files stored as they are (programs).
    pub extension: Option<&'static str>,
    /// A single entry, for messages: "bag entry", "data entry", "program".
    pub noun: &'static str,
    /// The whole store, for messages: "the bag", "your data", "your programs".
    pub whole: &'static str,
}

pub type Entries = BTreeMap<String, String>;

impl Registry {
    // ── Locations ─────────────────────────────────────────────────────────

    /// `~/.bullscript`, shared by every store.
    pub fn root() -> Result<PathBuf, String> {
        // HOME on Unix; USERPROFILE is the Windows equivalent and HOME is
        // normally unset there. Falling back to a relative path would
        // silently put the stores in whatever directory the user happened to
        // launch from.
        let home = std::env::var("HOME").ok()
            .filter(|h| !h.is_empty())
            .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()));
        match home {
            Some(h) => Ok(PathBuf::from(h).join(".bullscript")),
            None => Err(
                "cannot locate your home directory: neither HOME nor USERPROFILE is set, \
                 so BullScript does not know where to keep your files".to_string()
            ),
        }
    }

    fn list_path(&self) -> Result<PathBuf, String> {
        Ok(Self::root()?.join(format!("{}.json", self.prefix)))
    }

    /// Where this store keeps its own copy of every registered file.
    pub fn dir(&self) -> Result<PathBuf, String> {
        Ok(Self::root()?.join(self.files_dir))
    }

    /// Where the copy for `name` lives inside the store.
    pub fn path_for(&self, name: &str) -> Result<PathBuf, String> {
        let file = match self.extension {
            Some(ext) => format!("{}.{}", name, ext),
            None      => name.to_string(),
        };
        Ok(self.dir()?.join(file))
    }

    // ── Names ─────────────────────────────────────────────────────────────

    /// Entry names must be BullScript identifiers.
    pub fn validate_name(&self, name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err(format!("a {} name cannot be empty", self.noun));
        }
        let mut chars = name.chars();
        let first = chars.next().unwrap();
        if !(first.is_ascii_alphabetic() || first == '_') {
            return Err(format!(
                "'{}' is not a valid {} name: it must start with a letter or underscore",
                name, self.noun
            ));
        }
        if let Some(bad) = chars.find(|c| !(c.is_ascii_alphanumeric() || *c == '_')) {
            return Err(format!(
                "'{}' is not a valid {} name: '{}' is not allowed — use letters, digits \
                 and underscores only", name, self.noun, bad
            ));
        }
        if name == "true" || name == "false" {
            return Err(format!(
                "'{}' is a reserved word and cannot be a {} name", name, self.noun
            ));
        }
        Ok(())
    }

    // ── The list file ─────────────────────────────────────────────────────

    pub fn load(&self) -> Result<Entries, String> {
        let path = self.list_path()?;
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            // No file yet simply means an empty store.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Entries::new()),
            Err(e) => return Err(format!("could not read {}: {}", path.display(), e)),
        };
        if content.trim().is_empty() {
            return Ok(Entries::new());
        }
        serde_json::from_str(&content).map_err(|_| format!(
            "your {} list at {} is damaged and cannot be read.\n  \
             BullScript will not overwrite it, so nothing has been lost yet.\n  \
             Open the file to repair it, or delete it to start with an empty store.",
            self.noun, path.display()
        ))
    }

    fn save(&self, map: &Entries) -> Result<(), String> {
        let dir = Self::root()?;
        fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create {}: {}", dir.display(), e))?;
        let path = self.list_path()?;
        let content = serde_json::to_string_pretty(map)
            .map_err(|e| format!("could not encode the {} list: {}", self.noun, e))?;
        fs::write(&path, content)
            .map_err(|e| format!("could not write {}: {}", path.display(), e))
    }

    // ── Operations ────────────────────────────────────────────────────────

    /// Record that `name` is stored at `dest`. Returns true if an entry of
    /// that name already existed and was replaced.
    pub fn register(&self, name: &str, dest: &Path) -> Result<bool, String> {
        let mut map = self.load()?;
        let replaced = map.insert(name.to_string(), dest.display().to_string()).is_some();
        self.save(&map)?;
        Ok(replaced)
    }

    /// Write `content` as the store's copy for `name` and register it.
    pub fn store_text(&self, name: &str, content: &str) -> Result<bool, String> {
        self.validate_name(name)?;
        let dest = self.path_for(name)?;
        let dir = dest.parent().expect("path_for always has a parent");
        fs::create_dir_all(dir)
            .map_err(|e| format!("could not create {}: {}", dir.display(), e))?;
        fs::write(&dest, content)
            .map_err(|e| format!("could not write {}: {}", dest.display(), e))?;
        self.register(name, &dest)
    }

    /// Remove `name`. Returns false if it wasn't present.
    pub fn remove(&self, name: &str) -> Result<bool, String> {
        let mut map = self.load()?;
        let Some(stored) = map.remove(name) else { return Ok(false) };
        // Only delete the file if it lives in our own directory.
        if let Ok(dir) = self.dir() {
            let p = PathBuf::from(&stored);
            if p.starts_with(&dir) {
                let _ = fs::remove_file(&p);
            }
        }
        self.save(&map)?;
        Ok(true)
    }

    /// Every `(name, stored path)`, sorted by name.
    pub fn list(&self) -> Result<Vec<(String, String)>, String> {
        Ok(self.load()?.into_iter().collect())
    }

    /// The stored path for `name`, if registered.
    pub fn resolve(&self, name: &str) -> Result<Option<PathBuf>, String> {
        Ok(self.load()?.get(name).map(PathBuf::from))
    }

    // ── Sharing ───────────────────────────────────────────────────────────
    //
    // `export` writes every file in the store into one zip; `import` reads
    // such a zip back into another store. Between them a store is something
    // you can hand to someone.
    //
    // Only the file name of each archive entry is used, never its path.
    // That follows from what import means — "the files in the folder", flat,
    // into a flat store — and it means an entry cannot name a destination at
    // all, so nothing lands outside the store's directory.

    /// Write every entry into a zip at `path`.
    ///
    /// Returns how many were written and where they went.
    pub fn export(&self, path: &str) -> Result<(usize, PathBuf), String> {
        let ext = self.extension.expect("export is only offered by stores with an extension");
        let entries = self.load()?;
        if entries.is_empty() {
            return Err(format!("{} is empty — there is nothing to export", self.whole));
        }

        let dest = self.archive_destination(path);
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
            // silently dropped: an export that quietly loses files is worse
            // than one that fails.
            let content = fs::read_to_string(stored).map_err(|e| format!(
                "could not read the {} '{}' at {}: {}\n  \
                 The entry exists but its file does not. Remove it with \
                 `{}::remove {}` or re-add it.",
                self.noun, name, stored, e, self.prefix, name
            ))?;
            zip.start_file(format!("{}.{}", name, ext), options)
                .map_err(|e| format!("could not add '{}' to the archive: {}", name, e))?;
            io::Write::write_all(&mut zip, content.as_bytes())
                .map_err(|e| format!("could not write '{}' into the archive: {}", name, e))?;
            written += 1;
        }

        zip.finish()
            .map_err(|e| format!("could not finish writing {}: {}", dest.display(), e))?;
        Ok((written, dest))
    }

    /// Read every file with the store's extension from the zip at `path`.
    ///
    /// `accept` sees each file's content and decides whether it belongs in
    /// the store; a refused file is skipped, not an error. Returns
    /// `(added, replaced, skipped)`, skipped being the file names that were
    /// refused or could not be entry names.
    pub fn import(
        &self,
        path:   &str,
        accept: impl Fn(&str) -> bool,
    ) -> Result<(usize, usize, Vec<String>), String> {
        let ext = self.extension.expect("import is only offered by stores with an extension");
        let suffix = format!(".{}", ext);

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

            let raw = entry.name().to_string();
            let file_name = Path::new(&raw)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            // Anything with another extension is not this store's business.
            let Some(stem) = file_name.strip_suffix(&suffix) else { continue };

            // Not a trust check — an entry that cannot be named cannot be
            // used, and one the store cannot read could never be read from.
            if self.validate_name(stem).is_err() {
                skipped.push(file_name);
                continue;
            }
            let mut content = String::new();
            io::Read::read_to_string(&mut entry, &mut content)
                .map_err(|e| format!("could not read '{}' from the archive: {}", file_name, e))?;
            if !accept(&content) {
                skipped.push(file_name);
                continue;
            }

            if self.store_text(stem, &content)? {
                replaced += 1;
            } else {
                added += 1;
            }
        }

        if added == 0 && replaced == 0 && skipped.is_empty() {
            return Err(format!("'{}' contains no {} files", path, suffix));
        }
        Ok((added, replaced, skipped))
    }

    /// Where an export should write, given what the user typed.
    ///
    /// A directory gets a default name inside it; anything else is taken as
    /// the file to write, with `.zip` appended when it is missing so
    /// `bag::export mybag` does the obvious thing.
    fn archive_destination(&self, path: &str) -> PathBuf {
        let p = PathBuf::from(path);
        if p.is_dir() {
            return p.join(format!("bullscript-{}.zip", self.prefix));
        }
        match p.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("zip") => p,
            _ => PathBuf::from(format!("{}.zip", path)),
        }
    }
}
