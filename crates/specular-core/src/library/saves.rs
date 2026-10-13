//! Saves follow their game when its ROM is renamed or moved. A game's save files are named after
//! its ROM, in the same folders under `saves/` as the ROM is under `roms/` (see
//! `Game::save_path`), so a renamed ROM would leave its saves behind. To find them again, Specular
//! records the size and checksum of each ROM it has played, in `saves/.specular-saves.json`.
//! After a scan, the saves of a recorded ROM that is no longer on the card move to the one game
//! without saves whose ROM has the same size and checksum. With no such game, or several, they
//! stay where they are.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde_json::{Map, Value, json};

use super::Game;
use crate::settings::store::{self, LoadError};

/// Held while the record is read and rewritten: each launch notes its game on its own thread, and
/// a launch may start before the one before it has finished.
static RECORD: Mutex<()> = Mutex::new(());

fn record_path(saves: &Path) -> PathBuf { saves.join(".specular-saves.json") }

/// The record, by ROM path under `roms/`. A corrupt one has been moved aside, so it starts
/// afresh; one that can't be read is an error, as writing over it would lose every entry.
fn load(saves: &Path) -> io::Result<Map<String, Value>> {
    match store::load_layer(&record_path(saves)) {
        Ok(m) => Ok(m),
        Err(LoadError::Corrupt { .. }) => Ok(Map::new()),
        Err(LoadError::Io(e)) => Err(e),
    }
}

/// A file's size and modification time (seconds since 1970).
fn stamp(path: &Path) -> io::Result<(u64, u64)> {
    let m = fs::metadata(path)?;
    let t = m.modified()?.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Ok((m.len(), t))
}

/// A file's CRC-32, read a piece at a time (a disc image can be hundreds of megabytes).
fn checksum(path: &Path) -> io::Result<u32> {
    let (mut file, mut crc) = (io::BufReader::with_capacity(1 << 16, File::open(path)?), crc32fast::Hasher::new());
    loop {
        let piece = file.fill_buf()?;
        if piece.is_empty() { return Ok(crc.finalize()); }
        crc.update(piece);
        let n = piece.len();
        file.consume(n);
    }
}

/// Notes the ROM of `game` as played: its size and checksum (worked out again only if the file
/// has changed since). Reads the whole file the first time: call it off the UI thread.
pub fn note_played(roms: &Path, saves: &Path, game: &Game) -> io::Result<()> {
    let rom = roms.join(&game.path);
    let (size, modified) = stamp(&rom)?;
    let key = game.path.to_string_lossy().into_owned();
    let _held = RECORD.lock().unwrap_or_else(|e| e.into_inner());
    let mut records = load(saves)?;
    if records.get(&key).is_some_and(|r| r["size"] == size && r["modified"] == modified) { return Ok(()); }
    records.insert(key, json!({ "size": size, "modified": modified, "crc32": checksum(&rom)? }));
    store::store_layer(&record_path(saves), &records)
}

/// The save files of the game whose ROM is at `rom` (under `roms/`): those in its folder under
/// `saves` named like its ROM, with any extension ("pokehns.srm", "pokehns.state1").
fn save_files(saves: &Path, rom: &Path) -> Vec<PathBuf> {
    let base = saves.join(rom).with_extension("");
    let (Some(dir), Some(stem)) = (base.parent(), base.file_name()) else { return vec![] };
    let Ok(entries) = fs::read_dir(dir) else { return vec![] };
    entries.flatten().map(|e| e.path()).filter(|p| p.file_stem() == Some(stem) && p.is_file()).collect()
}

/// Moves the saves of ROMs that are gone to the game whose ROM is the same file under a new
/// name (see the module doc). Returns the moves made, as (old ROM path, new ROM path).
pub fn follow_moved_roms(roms: &Path, saves: &Path, games: &[Game]) -> Vec<(PathBuf, PathBuf)> {
    let _held = RECORD.lock().unwrap_or_else(|e| e.into_inner());
    let mut records = match load(saves) {
        Ok(r) => r,
        Err(e) => { eprintln!("saves: {}: {e}", record_path(saves).display()); return vec![]; }
    };
    // Saves named like a listed game's ROM are that game's, even if a recorded ROM had the same
    // name (say "Game.gba", since zipped up as "Game.zip").
    let in_use: HashSet<PathBuf> = games.iter().map(|g| g.path.with_extension("")).collect();
    let gone: Vec<(String, u64, u64)> = records.iter()
        // Gone from the card, not just from the list (the scan may have skipped a folder).
        .filter(|(rom, _)| !in_use.contains(&Path::new(rom).with_extension("")) && !roms.join(rom).exists())
        .filter_map(|(rom, r)| Some((rom.clone(), r["size"].as_u64()?, r["crc32"].as_u64()?)))
        .filter(|(rom, _, _)| !save_files(saves, Path::new(rom)).is_empty())
        .collect();
    let mut moves = Vec::new();
    for (old, size, crc) in gone {
        // Cheapest test first; the checksum reads the whole ROM.
        let same: Vec<&Game> = games.iter()
            .filter(|g| fs::metadata(roms.join(&g.path)).is_ok_and(|m| m.len() == size))
            .filter(|g| save_files(saves, &g.path).is_empty())
            .filter(|g| checksum(&roms.join(&g.path)).is_ok_and(|c| c as u64 == crc))
            .collect();
        let [game] = same[..] else { continue };
        if !move_saves(saves, Path::new(&old), &game.path) { continue; }
        if let Some(r) = records.remove(&old) { records.insert(game.path.to_string_lossy().into_owned(), r); }
        moves.push((old.into(), game.path.clone()));
    }
    if !moves.is_empty() && let Err(e) = store::store_layer(&record_path(saves), &records) { eprintln!("saves: {e}"); }
    moves
}

/// Renames the save files of the ROM `from` to go with the ROM `to` (both under `roms/`): all of
/// them, or none, putting back any already moved if one can't be. Returns whether they moved.
fn move_saves(saves: &Path, from: &Path, to: &Path) -> bool {
    let base = saves.join(to).with_extension("");
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for file in save_files(saves, from) {
        let mut dest = base.clone().into_os_string();
        if let Some(ext) = file.extension() { dest.push("."); dest.push(ext); }
        let dest = PathBuf::from(dest);
        if let Err(e) = dest.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| fs::rename(&file, &dest)) {
            eprintln!("saves: moving {} to {}: {e}; leaving its game's saves where they were", file.display(), dest.display());
            for (file, dest) in moved.iter().rev() { let _ = fs::rename(dest, file); }
            return false;
        }
        moved.push((file, dest));
    }
    true
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
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
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
        fs::remove_file(roms.join("gba/Old Name.gba")).unwrap();
        write(&roms.join("gba/RPG/New v1.1.gba"), b"rom one");
        write(&roms.join("gba/Other.gba"), b"rom two");
        let games = [game("gba/RPG/New v1.1.gba"), game("gba/Other.gba")];
        assert_eq!(follow_moved_roms(&roms, &saves, &games), vec![(PathBuf::from("gba/Old Name.gba"), PathBuf::from("gba/RPG/New v1.1.gba"))]);
        assert_eq!(fs::read(saves.join("gba/RPG/New v1.1.srm")).unwrap(), b"battery");
        assert!(saves.join("gba/RPG/New v1.1.state1").exists() && !saves.join("gba/Old Name.srm").exists());
        assert!(record(&saves, "gba/RPG/New v1.1.gba").is_some());
        // Gone again, now with two identical copies: nothing moves.
        fs::remove_file(roms.join("gba/RPG/New v1.1.gba")).unwrap();
        write(&roms.join("a/Copy 1.gba"), b"rom one");
        write(&roms.join("a/Copy 2.gba"), b"rom one");
        assert_eq!(follow_moved_roms(&roms, &saves, &[game("a/Copy 1.gba"), game("a/Copy 2.gba")]), vec![]);
        assert!(saves.join("gba/RPG/New v1.1.srm").exists());
    }

    #[test]
    fn saves_stay_if_the_rom_is_still_there_or_one_of_them_cant_move() {
        let tmp = tempfile::tempdir().unwrap();
        let (roms, saves) = (tmp.path().join("roms"), tmp.path().join("saves"));
        write(&roms.join("gba/Old.gba"), b"rom");
        write(&saves.join("gba/Old.srm"), b"battery");
        write(&saves.join("gba/Old.state1"), b"state");
        note_played(&roms, &saves, &game("gba/Old.gba")).unwrap();
        write(&roms.join("gba/New.gba"), b"rom");
        // Not listed (say its folder couldn't be read), but still on the card.
        assert_eq!(follow_moved_roms(&roms, &saves, &[game("gba/New.gba")]), vec![]);
        // Gone, but one of its saves can't take the new name (a folder has it).
        fs::remove_file(roms.join("gba/Old.gba")).unwrap();
        fs::create_dir_all(saves.join("gba/New.state1/x")).unwrap();
        assert_eq!(follow_moved_roms(&roms, &saves, &[game("gba/New.gba")]), vec![]);
        assert!(saves.join("gba/Old.srm").exists() && saves.join("gba/Old.state1").exists() && !saves.join("gba/New.srm").exists());
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
