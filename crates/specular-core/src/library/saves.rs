//! Saves follow their game when its ROM is renamed or moved. A game's save files are named after
//! its ROM: battery saves in the same folders under `saves/` as the ROM is under `roms/` (see
//! `Game::save_path`), save states in `.states/` beside the ROM (see `Game::state_path`); so a
//! renamed ROM would leave its saves behind. To find them again, Specular
//! records the size and checksum of each ROM it has played, in `saves/.specular-saves.json`.
//! After a scan, the saves of a recorded ROM that is no longer on the card move to the one game
//! without battery saves whose ROM has the same size and checksum. With no such game, or several, they
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

/// The name, extension aside, of the battery saves of the game whose ROM is at `rom` (under
/// `roms/`).
fn battery_name(saves: &Path, rom: &Path) -> PathBuf { saves.join(rom).with_extension("") }

/// The name, extension aside, of the save states of the game whose ROM is at `rom` (under
/// `roms/`).
fn state_name(roms: &Path, rom: &Path) -> PathBuf { in_states_folder(&roms.join(rom).with_extension("")) }

/// The path `file` would have in a `.states` folder in its own folder, where save states are kept.
pub(super) fn in_states_folder(file: &Path) -> PathBuf {
    file.with_file_name(".states").join(file.file_name().unwrap_or_default())
}

/// The names, extension aside, of the saves of the game whose ROM is at `rom` (under `roms`): its
/// battery saves', then its save states'.
fn save_names(roms: &Path, saves: &Path, rom: &Path) -> [PathBuf; 2] { [battery_name(saves, rom), state_name(roms, rom)] }

/// The files named `name` with any extension ("pokehns" → "pokehns.srm", "pokehns.state1").
fn files_named(name: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (name.parent(), name.file_name()) else { return vec![] };
    let Ok(entries) = fs::read_dir(dir) else { return vec![] };
    entries.flatten().map(|e| e.path()).filter(|p| p.file_stem() == Some(stem) && p.is_file()).collect()
}

/// Moves the game's `state1` save state from its folder under `saves/`, where earlier builds of
/// Specular kept it, to `.states/` beside its ROM; unless `.states/` already has one, which wins
/// (the old one then stays where it is).
pub fn move_old_state(roms: &Path, saves: &Path, game: &Game) -> io::Result<()> {
    let (old, new) = (game.save_path(saves, "state1"), game.state_path(roms, "state1"));
    if !old.is_file() || new.exists() { return Ok(()); }
    fs::create_dir_all(new.parent().unwrap_or(roms))?;
    fs::rename(old, new)
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
        .filter(|(rom, _, _)| save_names(roms, saves, Path::new(rom)).iter().any(|n| !files_named(n).is_empty()))
        .collect();
    let mut moves = Vec::new();
    for (old, size, crc) in gone {
        // Cheapest test first; the checksum reads the whole ROM.
        let same: Vec<&Game> = games.iter()
            .filter(|g| fs::metadata(roms.join(&g.path)).is_ok_and(|m| m.len() == size))
            // A game with states still qualifies: when a ROM's whole folder is renamed, its
            // `.states/` goes along, leaving only the battery saves to move. (`move_saves` refuses
            // to replace a file, should both have states.)
            .filter(|g| files_named(&battery_name(saves, &g.path)).is_empty())
            .filter(|g| checksum(&roms.join(&g.path)).is_ok_and(|c| c as u64 == crc))
            .collect();
        let [game] = same[..] else { continue };
        if !move_saves(roms, saves, Path::new(&old), &game.path) { continue; }
        if let Some(r) = records.remove(&old) { records.insert(game.path.to_string_lossy().into_owned(), r); }
        moves.push((old.into(), game.path.clone()));
    }
    if !moves.is_empty() && let Err(e) = store::store_layer(&record_path(saves), &records) { eprintln!("saves: {e}"); }
    moves
}

/// Renames the save files of the ROM `from` to go with the ROM `to` (both under `roms/`): all of
/// them, or none, putting back any already moved if one can't be, or would replace a file.
/// Returns whether they moved.
fn move_saves(roms: &Path, saves: &Path, from: &Path, to: &Path) -> bool {
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in save_names(roms, saves, from).into_iter().zip(save_names(roms, saves, to)) {
        for file in files_named(&from) {
            let mut dest = to.clone().into_os_string();
            if let Some(ext) = file.extension() { dest.push("."); dest.push(ext); }
            let dest = PathBuf::from(dest);
            let r = if dest.exists() { Err(io::Error::from(io::ErrorKind::AlreadyExists)) }
                    else { dest.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| fs::rename(&file, &dest)) };
            if let Err(e) = r {
                eprintln!("saves: moving {} to {}: {e}; leaving its game's saves where they were", file.display(), dest.display());
                for (file, dest) in moved.iter().rev() { let _ = fs::rename(dest, file); }
                return false;
            }
            moved.push((file, dest));
        }
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
        write(&roms.join("gba/.states/Old Name.state1"), b"state");
        note_played(&roms, &saves, &game("gba/Old Name.gba")).unwrap();
        assert_eq!(record(&saves, "gba/Old Name.gba").unwrap()["crc32"], crc32fast::hash(b"rom one"));
        // Renamed and moved into a folder; another game with different contents, and one with saves.
        fs::remove_file(roms.join("gba/Old Name.gba")).unwrap();
        write(&roms.join("gba/RPG/New v1.1.gba"), b"rom one");
        write(&roms.join("gba/Other.gba"), b"rom two");
        let games = [game("gba/RPG/New v1.1.gba"), game("gba/Other.gba")];
        assert_eq!(follow_moved_roms(&roms, &saves, &games), vec![(PathBuf::from("gba/Old Name.gba"), PathBuf::from("gba/RPG/New v1.1.gba"))]);
        assert_eq!(fs::read(saves.join("gba/RPG/New v1.1.srm")).unwrap(), b"battery");
        assert!(roms.join("gba/RPG/.states/New v1.1.state1").exists() && !saves.join("gba/Old Name.srm").exists());
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
        write(&roms.join("gba/.states/Old.state1"), b"state");
        note_played(&roms, &saves, &game("gba/Old.gba")).unwrap();
        write(&roms.join("gba/New.gba"), b"rom");
        // Not listed (say its folder couldn't be read), but still on the card.
        assert_eq!(follow_moved_roms(&roms, &saves, &[game("gba/New.gba")]), vec![]);
        // Gone, but one of its saves would replace a state the new name already has.
        fs::remove_file(roms.join("gba/Old.gba")).unwrap();
        write(&roms.join("gba/.states/New.state1"), b"newer state");
        assert_eq!(follow_moved_roms(&roms, &saves, &[game("gba/New.gba")]), vec![]);
        assert!(saves.join("gba/Old.srm").exists() && !saves.join("gba/New.srm").exists());
        assert_eq!(fs::read(roms.join("gba/.states/New.state1")).unwrap(), b"newer state");
    }

    #[test]
    fn a_renamed_folder_s_battery_saves_follow_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (roms, saves) = (tmp.path().join("roms"), tmp.path().join("saves"));
        write(&roms.join("gba/RPG/Game.gba"), b"rom");
        write(&saves.join("gba/RPG/Game.srm"), b"battery");
        write(&roms.join("gba/RPG/.states/Game.state1"), b"state");
        note_played(&roms, &saves, &game("gba/RPG/Game.gba")).unwrap();
        fs::rename(roms.join("gba/RPG"), roms.join("gba/Role-playing")).unwrap(); // its states go along
        assert_eq!(follow_moved_roms(&roms, &saves, &[game("gba/Role-playing/Game.gba")]).len(), 1);
        assert!(saves.join("gba/Role-playing/Game.srm").exists() && roms.join("gba/Role-playing/.states/Game.state1").exists());
    }

    #[test]
    fn a_game_s_save_files_are_those_named_like_its_rom() {
        let tmp = tempfile::tempdir().unwrap();
        for f in ["gba/pokehns.srm", "gba/pokehns.state1", "gba/pokehns.fastlevels.srm"] { write(&tmp.path().join(f), b""); }
        let mut files = files_named(&battery_name(tmp.path(), Path::new("gba/pokehns.gba")));
        files.sort();
        assert_eq!(files, [tmp.path().join("gba/pokehns.srm"), tmp.path().join("gba/pokehns.state1")]);
        assert_eq!(game("gba/pokehns.v2.gba").state_path(Path::new("/roms"), "autosave"), Path::new("/roms/gba/.states/pokehns.v2.autosave"));
    }

    #[test]
    fn an_old_state_moves_beside_its_rom_unless_one_is_there() {
        let tmp = tempfile::tempdir().unwrap();
        let (roms, saves) = (tmp.path().join("roms"), tmp.path().join("saves"));
        let (a, b) = (game("gba/A.gba"), game("gba/B.gba"));
        write(&saves.join("gba/A.state1"), b"old");
        write(&saves.join("gba/B.state1"), b"old");
        write(&roms.join("gba/.states/B.state1"), b"new");
        move_old_state(&roms, &saves, &a).unwrap();
        move_old_state(&roms, &saves, &b).unwrap();
        assert_eq!(fs::read(roms.join("gba/.states/A.state1")).unwrap(), b"old");
        assert_eq!(fs::read(roms.join("gba/.states/B.state1")).unwrap(), b"new");
        assert!(!saves.join("gba/A.state1").exists() && saves.join("gba/B.state1").exists());
    }
}
