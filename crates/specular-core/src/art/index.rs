//! What is known about each ROM's art, kept in `cache/identify/<system>.json` so no file is
//! identified twice, and sources aren't asked again soon for an image they don't have.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::{IDENTIFY_VERSION, Identity, Method};
use crate::settings::store;

/// What is known about one ROM file, valid while its size and modification time are unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub size: u64,
    /// Seconds since 1970.
    pub modified: u64,
    /// None: checked, and the game table doesn't know it.
    pub identity: Option<Identity>,
    /// When every art source last said it has no image (seconds since 1970), so it isn't asked
    /// again for a while.
    pub missing_since: Option<u64>,
}

/// One system's entries, by ROM path under `roms/`.
pub struct ArtIndex {
    path: PathBuf,
    version: String,
    games: Map<String, Value>,
    dirty: bool,
}

impl ArtIndex {
    /// Loads the index at `path`. It starts empty if the file is missing or unreadable, or was
    /// made with a game table other than `table` (`GameDb::version`) or another
    /// `IDENTIFY_VERSION`.
    pub fn load(path: PathBuf, table: &str) -> Self {
        let version = format!("identify {IDENTIFY_VERSION}, {table}");
        let file = store::load_layer(&path).unwrap_or_default();
        let games = match (file.get("version").and_then(Value::as_str), file.get("games")) {
            (Some(v), Some(Value::Object(g))) if v == version => g.clone(),
            _ => Map::new(),
        };
        Self { path, version, games, dirty: false }
    }

    /// The entry for `rom`, if it was made for a file of this size and modification time.
    pub fn get(&self, rom: &Path, size: u64, modified: u64) -> Option<Entry> {
        let e = self.games.get(rom.to_str()?)?;
        let n = |k: &str| e.get(k).and_then(Value::as_u64);
        if (n("size"), n("modified")) != (Some(size), Some(modified)) { return None; }
        let identity = e.get("name").and_then(Value::as_str).map(|name| Identity {
            name: name.to_string(),
            by: Method::ALL.into_iter().find(|m| Some(m.name()) == e.get("by").and_then(Value::as_str)).unwrap_or(Method::Name),
        });
        Some(Entry { size, modified, identity, missing_since: n("missing_since") })
    }

    /// Records `e` for `rom`, to be written by the next `save`.
    pub fn set(&mut self, rom: &Path, e: &Entry) {
        let Some(key) = rom.to_str() else { return };
        let mut v = json!({ "size": e.size, "modified": e.modified });
        if let Some(id) = &e.identity { v["name"] = id.name.clone().into(); v["by"] = id.by.name().into(); }
        if let Some(t) = e.missing_since { v["missing_since"] = t.into(); }
        self.games.insert(key.to_string(), v);
        self.dirty = true;
    }

    /// Writes the index if it changed since it was loaded or last saved.
    pub fn save(&mut self) -> io::Result<()> {
        if !self.dirty { return Ok(()); }
        let mut file = Map::new();
        file.insert("version".into(), self.version.clone().into());
        file.insert("games".into(), Value::Object(self.games.clone()));
        store::store_layer(&self.path, &file)?;
        self.dirty = false;
        Ok(())
    }
}
