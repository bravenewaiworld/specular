//! Settings files on the SD card: plain JSON objects, written so a power cut can't destroy them.
//! FAT32 has no journal, so a file is never rewritten in place: the new content goes to
//! `<file>.tmp`, is flushed, and replaces the file with a rename.

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
        // Also a stale .tmp, which `load_layer` would otherwise bring back.
        for p in [path.to_path_buf(), tmp(path)] {
            if let Err(e) = fs::remove_file(p) && e.kind() != io::ErrorKind::NotFound { return Err(e); }
        }
        return Ok(());
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = tmp(path);
    let mut f = File::create(&tmp)?;
    f.write_all(serde_json::to_string_pretty(map).expect("JSON values always serialize").as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    fs::rename(&tmp, path)?;
    // Make the rename itself durable.
    File::open(dir)?.sync_all()
}
