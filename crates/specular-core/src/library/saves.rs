//! Saves follow their game when its ROM is renamed or moved. `saves/` mirrors `roms/` by name
//! (see `Game::save_path`), so a renamed ROM would lose its saves; to find them again, Specular
//! keeps a record, `saves/.specular-saves.json`, of the size and checksum of each ROM it has
//! played. After a scan, the saves of a ROM that is gone move to the one game without saves whose
//! ROM has the same size and checksum. With no such game, or several, they stay where they are.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::json;

use super::Game;
use crate::settings::store;

fn record_path(saves: &Path) -> PathBuf { saves.join(".specular-saves.json") }

/// A file's size and modification time (seconds since 1970).
fn stamp(path: &Path) -> io::Result<(u64, u64)> {
    let m = std::fs::metadata(path)?;
    let t = m.modified()?.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Ok((m.len(), t))
}

/// Notes the ROM of `game` as played: its size and checksum (computed only if the record lacks
/// them or the file changed). Reads the whole file the first time: call it off the UI thread.
pub fn note_played(roms: &Path, saves: &Path, game: &Game) -> io::Result<()> {
    let rom = roms.join(&game.path);
    let (size, modified) = stamp(&rom)?;
    let key = game.path.to_string_lossy();
    let mut records = store::load_layer(&record_path(saves)).unwrap_or_default();
    let known = records.get(key.as_ref()).is_some_and(|r| r["size"] == size && r["modified"] == modified);
    if known { return Ok(()); }
    let crc = crc32fast::hash(&std::fs::read(&rom)?);
    records.insert(key.into_owned(), json!({ "size": size, "modified": modified, "crc32": crc }));
    store::store_layer(&record_path(saves), &records)
}

/// The save files of the game whose ROM is at `rom` (under `roms/`): those in its folder under
/// `saves` named like its ROM, with any extension ("pokehns.srm", "pokehns.state1").
fn save_files(saves: &Path, rom: &Path) -> Vec<PathBuf> {
    let base = saves.join(rom).with_extension("");
    let (Some(dir), Some(stem)) = (base.parent(), base.file_name()) else { return vec![] };
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    entries.flatten().map(|e| e.path()).filter(|p| p.file_stem() == Some(stem) && p.is_file()).collect()
}

/// Moves the saves of ROMs that are gone to the game whose ROM is the same file under a new
/// name (see the module doc). Returns the moves made, as (old ROM path, new ROM path).
pub fn follow_moved_roms(roms: &Path, saves: &Path, games: &[Game]) -> Vec<(PathBuf, PathBuf)> {
    let mut records = store::load_layer(&record_path(saves)).unwrap_or_default();
    let present: HashMap<String, &Game> = games.iter().map(|g| (g.path.to_string_lossy().into_owned(), g)).collect();
    let gone: Vec<(String, u64, u64)> = records.iter()
        .filter(|(rom, _)| !present.contains_key(*rom))
        .filter_map(|(rom, r)| Some((rom.clone(), r["size"].as_u64()?, r["crc32"].as_u64()?)))
        .filter(|(rom, _, _)| !save_files(saves, Path::new(rom)).is_empty())
        .collect();
    let mut moves = Vec::new();
    for (old, size, crc) in gone {
        // Games without saves of the same size, then the same checksum (read only for these).
        let same: Vec<&Game> = games.iter()
            .filter(|g| save_files(saves, &g.path).is_empty())
            .filter(|g| stamp(&roms.join(&g.path)).is_ok_and(|(s, _)| s == size))
            .filter(|g| std::fs::read(roms.join(&g.path)).is_ok_and(|d| crc32fast::hash(&d) as u64 == crc))
            .collect();
        let [game] = same[..] else { continue };
        let (old_rom, new_base) = (PathBuf::from(&old), saves.join(&game.path).with_extension(""));
        let mut moved = true;
        for file in save_files(saves, &old_rom) {
            let mut to = new_base.clone().into_os_string();
            if let Some(ext) = file.extension() { to.push("."); to.push(ext); }
            let to = PathBuf::from(to);
            let ok = to.parent().is_none_or(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::rename(&file, &to).is_ok();
            if !ok { eprintln!("saves: moving {} to {} failed", file.display(), to.display()); moved = false; }
        }
        if moved {
            if let Some(r) = records.remove(&old) { records.insert(game.path.to_string_lossy().into_owned(), r); }
            moves.push((old_rom, game.path.clone()));
        }
    }
    if !moves.is_empty() && let Err(e) = store::store_layer(&record_path(saves), &records) { eprintln!("saves: {e}"); }
    moves
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The record's entry for a ROM.
    fn record(saves: &Path, rom: &str) -> Option<serde_json::Value> {
        store::load_layer(&record_path(saves)).ok()?.get(rom).cloned()
    }

    fn game(path: &str) -> Game { Game { system: "gba", path: path.into(), title: String::new(), sort_key: String::new() } }

    fn write(path: &Path, data: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
    }

    #[test]
    fn saves_follow_a_renamed_rom_but_not_to_an_ambiguous_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (roms, saves) = (tmp.path().join("roms"), tmp.path().join("saves"));
        write(&roms.join("gba/Old Name.gba"), b"rom one");
        write(&saves.join("gba/Old Name.srm"), b"battery");
        write(&saves.join("gba/Old Name.state1"), b"state");
        note_played(&roms, &saves, &game("gba/Old Name.gba")).unwrap();
        assert_eq!(record(&saves, "gba/Old Name.gba").unwrap()["crc32"], crc32fast::hash(b"rom one"));
        // Renamed and moved into a folder; another game with different contents, and one with saves.
        std::fs::remove_file(roms.join("gba/Old Name.gba")).unwrap();
        write(&roms.join("gba/RPG/New Name.gba"), b"rom one");
        write(&roms.join("gba/Other.gba"), b"rom two");
        let games = [game("gba/RPG/New Name.gba"), game("gba/Other.gba")];
        assert_eq!(follow_moved_roms(&roms, &saves, &games), vec![(PathBuf::from("gba/Old Name.gba"), PathBuf::from("gba/RPG/New Name.gba"))]);
        assert_eq!(std::fs::read(saves.join("gba/RPG/New Name.srm")).unwrap(), b"battery");
        assert!(saves.join("gba/RPG/New Name.state1").exists() && !saves.join("gba/Old Name.srm").exists());
        assert!(record(&saves, "gba/RPG/New Name.gba").is_some());
        // Gone again, now with two identical copies: nothing moves.
        std::fs::remove_file(roms.join("gba/RPG/New Name.gba")).unwrap();
        write(&roms.join("a/Copy 1.gba"), b"rom one");
        write(&roms.join("a/Copy 2.gba"), b"rom one");
        assert_eq!(follow_moved_roms(&roms, &saves, &[game("a/Copy 1.gba"), game("a/Copy 2.gba")]), vec![]);
        assert!(saves.join("gba/RPG/New Name.srm").exists());
    }

    #[test]
    fn a_game_s_save_files_are_those_named_like_its_rom() {
        let tmp = tempfile::tempdir().unwrap();
        for f in ["gba/pokehns.srm", "gba/pokehns.state1", "gba/pokehns.fastlevels.srm"] { write(&tmp.path().join(f), b""); }
        let mut files = save_files(tmp.path(), Path::new("gba/pokehns.gba"));
        files.sort();
        assert_eq!(files, [tmp.path().join("gba/pokehns.srm"), tmp.path().join("gba/pokehns.state1")]);
    }
}
