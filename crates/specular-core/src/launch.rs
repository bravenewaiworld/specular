//! The launch record, `cache/launch.json`: written as a game starts and removed when it ends
//! cleanly, so one still there at start-up means the last game didn't end cleanly: its core (or
//! Specular itself) crashed or hung, or the power was cut. Written with the settings'
//! power-cut-safe store.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::library::Game;
use crate::settings::store;

/// What the record says about the game running.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LaunchRecord {
    /// The core's name, e.g. "mGBA".
    pub core: String,
    /// Its library, e.g. `.../cores/mgba_libretro.so`.
    pub library: PathBuf,
    pub system: String,
    /// The game's path under `roms/`.
    pub game: PathBuf,
    /// When it started, in seconds since 1970 by the device's clock (which may not have been set).
    pub started: u64,
    /// Why Specular ended the game itself, if it did (its core stopped responding).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Where the record is kept.
pub fn path(cache: &Path) -> PathBuf { cache.join("launch.json") }

/// Where the game last started is recorded (`last_game`). Unlike the launch record it stays, so
/// `play-last` works after a restart.
pub fn last_game_path(cache: &Path) -> PathBuf { cache.join("last-game.json") }

/// What `last_game_path` holds for `game` (stored with `store::store_layer`).
pub fn last_game_record(game: &Game) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    map.insert("game".into(), game.path.to_string_lossy().into());
    map
}

/// The path under roms/ of the game last started, as recorded in `cache` (None if not recorded,
/// or unreadable: logged).
pub fn last_game(cache: &Path) -> Option<PathBuf> {
    let path = last_game_path(cache);
    let map = store::load_layer(&path).map_err(|e| eprintln!("game: the last game's record {}: {e:?}", path.display())).ok()?;
    map.get("game")?.as_str().map(PathBuf::from)
}

impl LaunchRecord {
    /// A record of `game` starting now with the core `core` from `library`.
    pub fn new(core: &str, library: &Path, game: &Game) -> Self {
        let started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Self { core: core.into(), library: library.into(), system: game.system.into(), game: game.path.clone(), started, reason: None }
    }

    /// Writes the record to `path`, replacing any there.
    pub fn write(&self, path: &Path) -> io::Result<()> { store::store_record(path, self) }

    /// Removes the record at `path` (the store removes the file, and any `.tmp` left by a write
    /// cut short, when given nothing to store).
    pub fn remove(path: &Path) -> io::Result<()> { store::store_layer(path, &Default::default()) }

    /// Removes the record at `path` if it is this one; returns whether it was. For a game ended
    /// without saving (its card gone or read-only, so the record couldn't be removed then), once
    /// its card can be written again: a record for any other game is left for `left_behind`.
    pub fn remove_if_this(&self, path: &Path) -> io::Result<bool> {
        let Ok(map) = store::load_layer(path) else { return Ok(false) };
        if serde_json::from_value::<Self>(serde_json::Value::Object(map)).ok().as_ref() != Some(self) { return Ok(false); }
        Self::remove(path).map(|()| true)
    }

    /// At start-up: the record left at `path` by a game that didn't end, if there is one. It is
    /// logged in full and removed. A file that isn't JSON is logged and moved aside
    /// (`store::load_layer`); JSON that isn't a record is logged and removed. Both give None.
    pub fn left_behind(path: &Path) -> Option<Self> {
        let map = store::load_layer(path).map_err(|e| eprintln!("game: the launch record {}: {e:?}", path.display())).ok()?;
        if map.is_empty() { return None; }
        let text = serde_json::to_string(&map).unwrap_or_default();
        eprintln!("game: the last game didn't end; its launch record: {text}");
        if let Err(e) = Self::remove(path) { eprintln!("game: removing {}: {e}", path.display()); }
        serde_json::from_value(serde_json::Value::Object(map)).map_err(|e| eprintln!("game: the launch record {}: {e}", path.display())).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_game_is_recorded_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(last_game(dir.path()), None);
        let game = Game { system: "gba", path: "gba/Golden Sun.zip".into(), title: "Golden Sun".into(), sort_key: String::new() };
        store::store_layer(&last_game_path(dir.path()), &last_game_record(&game)).unwrap();
        assert_eq!(last_game(dir.path()), Some("gba/Golden Sun.zip".into()));
    }

    #[test]
    fn a_record_left_behind_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(dir.path());
        let game = Game { system: "gba", path: "gba/Golden Sun.zip".into(), title: "Golden Sun".into(), sort_key: String::new() };
        let mut r = LaunchRecord::new("mGBA", Path::new("/cores/mgba_libretro.so"), &game);
        r.write(&p).unwrap();
        r.reason = Some("it stopped responding".into());
        r.write(&p).unwrap();
        assert_eq!(LaunchRecord::left_behind(&p), Some(r.clone()));
        assert_eq!(LaunchRecord::left_behind(&p), None);
        r.write(&p).unwrap();
        LaunchRecord::remove(&p).unwrap();
        assert_eq!(LaunchRecord::left_behind(&p), None);
    }

    #[test]
    fn only_the_same_record_is_removed_by_remove_if_this() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(dir.path());
        let game = |title: &str| Game { system: "gba", path: format!("gba/{title}.gba").into(), title: title.into(), sort_key: String::new() };
        let ours = LaunchRecord::new("mGBA", Path::new("/cores/mgba_libretro.so"), &game("Golden Sun"));
        assert!(!ours.remove_if_this(&p).unwrap()); // none there
        LaunchRecord::new("mGBA", Path::new("/cores/mgba_libretro.so"), &game("Mother 3")).write(&p).unwrap();
        assert!(!ours.remove_if_this(&p).unwrap());
        assert!(p.exists());
        ours.write(&p).unwrap();
        assert!(ours.remove_if_this(&p).unwrap());
        assert!(!p.exists());
    }
}
