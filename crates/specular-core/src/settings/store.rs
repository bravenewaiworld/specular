//! JSON files on the SD card (settings, their schemas, and Specular's own records in `cache/` and
//! `saves/`), written so a power cut can't destroy them. FAT32 has no journal, so a file is never
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
    /// Not a JSON object. The file was moved aside to `<file>.bad` so the user can look at it.
    Corrupt { moved_to: PathBuf },
}

fn tmp(path: &Path) -> PathBuf { path.with_extension("json.tmp") }
fn bad(path: &Path) -> PathBuf { path.with_extension("json.bad") }

/// A file's object; None if there is no file; InvalidData if it isn't a JSON object. A file with
/// nothing in it counts as an empty object (someone may have emptied it by hand).
fn parse(path: &Path) -> io::Result<Option<Map<String, Value>>> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if text.trim().is_empty() { return Ok(Some(Map::new())); }
    match serde_json::from_str(&text) {
        Ok(Value::Object(m)) => Ok(Some(m)),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "not a JSON object")),
    }
}

/// Reads one settings file. A missing file is an empty layer. If the file is missing or not
/// valid JSON, a valid `<file>.tmp` left by a save that was cut off is used instead.
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
            Err(LoadError::Corrupt { moved_to })
        }
        Err(e) => Err(LoadError::Io(e)),
    }
}

/// Writes one settings file safely (see the module doc). An empty layer removes the file.
pub fn store_layer(path: &Path, map: &Map<String, Value>) -> io::Result<()> {
    if map.is_empty() {
        // Also a stale .tmp, which `load_layer` would otherwise bring back; first, so a power cut
        // between the two can't leave it alone.
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

/// Writes a generated file (a settings schema) safely, and read-only, as a hint that edits to it
/// are ignored. Left alone if it already holds `content`, to spare the card (even if its
/// read-only mark was cleared). Returns whether it was written.
pub fn store_generated(path: &Path, content: &Value) -> io::Result<bool> {
    let text = json_text(content);
    if fs::read(path).is_ok_and(|old| old == text.as_bytes()) { return Ok(false); }
    replace(path, &text, true).map(|()| true)
}

fn json_text(v: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(v).expect("JSON values always serialize") + "\n"
}

/// Replaces `path` with `text` through `<file>.tmp` (see the module doc).
fn replace(path: &Path, text: &str, read_only: bool) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = tmp(path);
    // One left by a cut-off `store_generated` is already read-only, so it couldn't be opened.
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
