//! Files on the SD card (settings, their schemas, the guide files, and Specular's own records in
//! `cache/` and `saves/`), written so a power cut can't destroy them. FAT32 has no journal, so a file is never
//! rewritten in place: the new content goes to `<file>.tmp`, is flushed, and replaces the file with
//! a rename.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Why a settings file couldn't be used.
#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    /// Not a JSON object, for `reason` (serde's error, with its line and column). The file was
    /// moved aside to `<file>.bad` so the user can look at it.
    Corrupt { moved_to: PathBuf, reason: String },
}

fn tmp(path: &Path) -> PathBuf { with_suffix(path, ".tmp") }
fn bad(path: &Path) -> PathBuf { with_suffix(path, ".bad") }
fn with_suffix(path: &Path, suffix: &str) -> PathBuf { let mut p = path.as_os_str().to_owned(); p.push(suffix); p.into() }

/// A file's object; None if there is no file; InvalidData, saying why, if it isn't a JSON object.
/// A file with nothing in it counts as an empty object (someone may have emptied it by hand).
fn parse(path: &Path) -> io::Result<Option<Map<String, Value>>> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if text.trim().is_empty() { return Ok(Some(Map::new())); }
    match serde_json::from_str(&text) {
        Ok(Value::Object(m)) => Ok(Some(m)),
        Ok(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "valid JSON, but not { … }")),
        Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e)),
    }
}

/// Reads one settings file. If the file is missing or not a JSON object, a valid `<file>.tmp`
/// left by a save that was cut off is used instead; failing that, a missing file is an empty
/// layer, and one that isn't a JSON object is moved aside (`LoadError::Corrupt`).
pub fn load_layer(path: &Path) -> Result<Map<String, Value>, LoadError> {
    match parse(path) {
        Ok(Some(m)) => Ok(m),
        Ok(None) => match parse(&tmp(path)) {
            Ok(Some(m)) => Ok(m),
            _ => Ok(Map::new()),
        },
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            if let Ok(Some(m)) = parse(&tmp(path)) { return Ok(m); }
            let moved_to = bad(path);
            fs::rename(path, &moved_to).map_err(LoadError::Io)?;
            Err(LoadError::Corrupt { moved_to, reason: e.to_string() })
        }
        Err(e) => Err(LoadError::Io(e)),
    }
}

/// `set_aside`'s reason for a `.bad` file that is a JSON object again.
pub const FIXED_SINCE: &str = "it has been corrected since, but not renamed back";

/// Every `<file>.json.bad` under `config` (`load_layer`'s, of files that weren't JSON objects), as
/// paths relative to it, sorted, each with why its settings aren't used, worded to follow "its
/// settings aren't used: ". Each is read again to find out: one fixed since, but not renamed
/// back, gives `FIXED_SINCE`. Looks no deeper than `library::MAX_DEPTH` folders.
pub fn set_aside(config: &Path) -> Vec<(PathBuf, String)> {
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(PathBuf, String)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let (path, Ok(kind)) = (e.path(), e.file_type()) else { continue };
            if kind.is_dir() && depth < crate::library::MAX_DEPTH { walk(root, &path, depth + 1, out); continue; }
            if !kind.is_file() || !e.file_name().to_string_lossy().ends_with(".json.bad") { continue; }
            let why = match parse(&path) {
                Err(e) if e.kind() == io::ErrorKind::InvalidData => format!("it isn't a JSON object ({e})"),
                Err(e) => format!("it can't be read ({e})"),
                Ok(_) => FIXED_SINCE.into(),
            };
            out.push((path.strip_prefix(root).unwrap_or(&path).to_path_buf(), why));
        }
    }
    let mut found = Vec::new();
    walk(config, config, 0, &mut found);
    found.sort();
    found
}

/// Writes one settings file safely (see the module doc). An empty layer removes the file.
pub fn store_layer(path: &Path, map: &Map<String, Value>) -> io::Result<()> {
    if map.is_empty() {
        // The `.tmp` a cut-off save may have left goes too, or `load_layer` would bring it back
        // once the file is gone. It goes first, so a power cut between the two removals can't
        // leave the `.tmp` without the file.
        for p in [tmp(path), path.to_path_buf()] {
            if let Err(e) = fs::remove_file(p) && e.kind() != io::ErrorKind::NotFound { return Err(e); }
        }
        return Ok(());
    }
    replace(path, &json_text(map), false)
}

/// Writes one of Specular's records (a struct, as a JSON object) with `store_layer`. Panics if
/// `record` doesn't serialize to a JSON object.
pub fn store_record(path: &Path, record: &impl serde::Serialize) -> io::Result<()> {
    let Ok(Value::Object(map)) = serde_json::to_value(record) else { panic!("a record is a JSON object") };
    store_layer(path, &map)
}

/// Writes a generated file (a settings schema, a guide) as `store_layer` does, and read-only, as a hint that
/// edits to it are lost. Left alone if it already holds `text`, to spare the card (even if its
/// read-only mark was cleared). Returns whether it was written.
pub fn store_generated(path: &Path, text: &str) -> io::Result<bool> {
    if fs::read(path).is_ok_and(|old| old == text.as_bytes()) { return Ok(false); }
    replace(path, text, true).map(|()| true)
}

/// `v` as Specular writes JSON files: indented, with a newline at the end.
pub fn json_text(v: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(v).expect("JSON values always serialize") + "\n"
}

/// Replaces `path` with `text` through `<file>.tmp` (see the module doc).
fn replace(path: &Path, text: &str, read_only: bool) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = tmp(path);
    // One left by a cut-off `store_generated` is already read-only, so `File::create` couldn't
    // open it for writing.
    let _ = fs::remove_file(&tmp);
    let mut f = File::create(&tmp)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    if read_only {
        // Only a hint, so a card that refuses it (exFAT can) doesn't stop the write.
        let marked = f.metadata().and_then(|m| { let mut p = m.permissions(); p.set_readonly(true); f.set_permissions(p) });
        if let Err(e) = marked { eprintln!("settings: can't mark {} read-only: {e}", path.display()); }
    }
    // Renaming over a read-only file takes write access to the folder only (POSIX; checked on
    // the PC, not yet on a FAT32 card).
    fs::rename(&tmp, path)?;
    sync_dir(dir)
}

/// Makes the changes to a folder's entries (files created, renamed or removed) durable.
pub fn sync_dir(dir: &Path) -> io::Result<()> { File::open(dir)?.sync_all() }
