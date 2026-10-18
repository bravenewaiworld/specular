//! The game library: which systems exist, and which games are on the card.
//!
//! Games are found by scanning `roms/` recursively and matching each file to a system by its
//! extension. A zip is matched by the file inside it. Files that only make up another game (the
//! tracks a .cue sheet lists, the discs an .m3u playlist lists) aren't listed on their own.
//! Symbolic links are not followed.

mod names;
pub mod saves;
mod search;
pub(crate) mod zip;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub use names::{display_title, index_letter, sort_key};
pub use search::search;

/// An emulated system and the files that hold its games.
#[derive(Clone, Debug)]
pub struct SystemDecl {
    /// Short, stable id, used in file paths and settings (e.g. "gba").
    pub id: &'static str,
    pub name: &'static str,
    /// Lower-case extensions of its game files, without the dot.
    pub extensions: &'static [&'static str],
    /// Its name in the libretro databases and thumbnail server (e.g. "Nintendo - Game Boy Advance").
    pub libretro_name: &'static str,
    /// Its native picture size in pixels (width, height), which sizes a game's place on screen
    /// before its first frame arrives.
    pub screen: (u32, u32),
    /// Its games' checksums and names (see `art::GameDb`); empty if there is no table.
    pub game_db: &'static str,
    /// Where its ROM header holds a game code that the table lists as a serial: (offset, length).
    /// Identifies ROMs no checksum matches, such as hacks, which keep their base game's code.
    pub header_code: Option<(u64, usize)>,
    /// The libretro core that plays its games; None while there is none (its games can't be
    /// played yet).
    pub core: Option<&'static CoreDecl>,
    /// The size of a header some dumps put before the ROM (such as the 512 bytes that old SNES
    /// copier devices added), which the table's checksums leave out: skipped when hashing a file
    /// whose size is that much over a multiple of 1 KiB.
    pub dump_header: u64,
}

/// A libretro core, and how Specular runs it.
#[derive(Debug)]
pub struct CoreDecl {
    /// Its library's name without ".so", e.g. "mgba_libretro".
    pub library: &'static str,
    /// The core options it runs with; the rest keep the core's defaults.
    pub options: &'static [(&'static str, &'static str)],
    /// Give it only the ROM's path, although it asks for the contents (see
    /// `LaunchSpec::rom_by_path` in specular-runtime).
    pub rom_by_path: bool,
}

/// mGBA, for GB, GBC and GBA alike. "Remove Known" skips the busy-wait loops of known games,
/// which saves battery without changing how they run (TrimUI's own setting too). It gets only the
/// ROM's path, and our build (tools/core-patches/mgba) maps that file instead of copying it. Given
/// the contents, mGBA copies them, and copies a GBA ROM again on the first write to its cartridge
/// area (as Pokémon games' clock does): 64 MB of memory for a 32 MB ROM.
const MGBA: &CoreDecl = &CoreDecl { library: "mgba_libretro", options: &[("mgba_idle_optimization", "Remove Known")], rom_by_path: true };

/// The systems Specular knows out of the box. Disc images for PS1 are listed by their .cue sheet,
/// .chd, .pbp or .m3u; a .bin is only a track of a .cue.
pub fn builtin_systems() -> Vec<SystemDecl> {
    vec![
        SystemDecl { id: "gb", name: "Game Boy", extensions: &["gb"], libretro_name: "Nintendo - Game Boy",
            screen: (160, 144), game_db: include_str!("../../../../data/db/gb.tsv"), core: Some(MGBA), header_code: None, dump_header: 0 },
        SystemDecl { id: "gbc", name: "Game Boy Color", extensions: &["gbc"], libretro_name: "Nintendo - Game Boy Color",
            screen: (160, 144), game_db: include_str!("../../../../data/db/gbc.tsv"), core: Some(MGBA), header_code: None, dump_header: 0 },
        SystemDecl { id: "gba", name: "Game Boy Advance", extensions: &["gba"], libretro_name: "Nintendo - Game Boy Advance",
            screen: (240, 160), game_db: include_str!("../../../../data/db/gba.tsv"), core: Some(MGBA), header_code: Some((0xac, 4)), dump_header: 0 },
        SystemDecl { id: "nes", name: "NES", extensions: &["nes", "fds"], libretro_name: "Nintendo - Nintendo Entertainment System",
            screen: (256, 240), game_db: include_str!("../../../../data/db/nes.tsv"), core: None, header_code: None, dump_header: 0 },
        SystemDecl { id: "snes", name: "Super Nintendo", extensions: &["sfc", "smc"], libretro_name: "Nintendo - Super Nintendo Entertainment System",
            screen: (256, 224), game_db: include_str!("../../../../data/db/snes.tsv"), core: None, header_code: None, dump_header: 512 },
        SystemDecl { id: "md", name: "Mega Drive", extensions: &["md", "gen", "smd"], libretro_name: "Sega - Mega Drive - Genesis",
            screen: (320, 224), game_db: include_str!("../../../../data/db/md.tsv"), core: None, header_code: None, dump_header: 0 },
        SystemDecl { id: "ps1", name: "PlayStation", extensions: &["cue", "chd", "pbp", "m3u"], libretro_name: "Sony - PlayStation",
            screen: (320, 240), game_db: "", core: None, header_code: None, dump_header: 0 },
    ]
}

/// A game on the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    pub system: &'static str,
    /// Relative to `roms/`, e.g. "gba/RPG/Golden Sun (USA, Europe).gba".
    pub path: PathBuf,
    /// The name shown, from the file name (tags such as "(USA, Europe)" removed).
    pub title: String,
    pub sort_key: String,
}

impl Game {
    /// The file name without its extension: the name of its saves and per-game settings.
    pub fn stem(&self) -> &str { self.path.file_stem().and_then(|s| s.to_str()).unwrap_or_default() }

    /// Its per-game settings layer, keyed by its path so same-named games in different folders
    /// keep separate settings.
    pub fn settings_layer(&self) -> crate::settings::Layer {
        crate::settings::Layer::Game { system: self.system.to_string(), game: self.path.with_extension("").to_string_lossy().into_owned() }
    }

    /// Where a battery save of this game goes: `saves/` mirrors `roms/`, with the ROM's name and
    /// the save's own extension (e.g. "srm").
    pub fn save_path(&self, saves: &Path, extension: &str) -> PathBuf {
        saves.join(self.path.with_extension(extension))
    }

    /// Where a save state of this game goes: in the `.states` folder beside its ROM (shared by
    /// the folder's games), with the ROM's name and the slot's extension (e.g. "state1").
    pub fn state_path(&self, roms: &Path, extension: &str) -> PathBuf {
        saves::in_states_folder(&roms.join(self.path.with_extension(extension)))
    }
}

/// Folders never scanned: hidden ones (including macOS's) and Windows' volume information.
pub fn skip_dir(name: &str) -> bool { name.starts_with('.') || name.eq_ignore_ascii_case("System Volume Information") }

/// Files never listed: hidden ones, including the "._name" files macOS leaves on FAT cards.
fn skip_file(name: &str) -> bool { name.starts_with('.') }

/// How deep folders are followed, so a huge tree can't stall the scan.
pub const MAX_DEPTH: usize = 8;

/// Finds every game under `roms`, sorted by system order then title. Unreadable folders and
/// files are skipped; the scan never fails as a whole.
pub fn scan(roms: &Path, systems: &[SystemDecl]) -> Vec<Game> {
    let mut files = Vec::new();
    walk(roms, roms, 0, &mut files);
    let parts = parts_of_other_games(roms, &files);
    let mut games: Vec<Game> = files.iter()
        .filter(|f| !parts.contains(&lower(f)))
        .filter_map(|f| {
            let system = system_of(&roms.join(f), systems)?;
            let stem = f.file_stem()?.to_str()?;
            Some(Game { system, path: f.clone(), title: display_title(stem), sort_key: sort_key(stem) })
        })
        .collect();
    let order = |id: &str| systems.iter().position(|s| s.id == id);
    games.sort_by(|a, b| (order(a.system), &a.sort_key, &a.path).cmp(&(order(b.system), &b.sort_key, &b.path)));
    games
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            if depth < MAX_DEPTH && !skip_dir(name) { walk(root, &e.path(), depth + 1, out); }
        } else if kind.is_file() && !skip_file(name)
            && let Ok(rel) = e.path().strip_prefix(root) { out.push(rel.to_path_buf()); }
    }
}

pub(crate) fn extension(p: &Path) -> Option<String> { p.extension()?.to_str().map(str::to_ascii_lowercase) }

/// The system a file belongs to: by its extension, or for a zip, by the first file inside it
/// with a known extension.
fn system_of(path: &Path, systems: &[SystemDecl]) -> Option<&'static str> {
    let by_ext = |ext: &str| systems.iter().find(|s| s.extensions.contains(&ext)).map(|s| s.id);
    let ext = extension(path)?;
    if ext == "zip" {
        return zip::entries(path).ok()?.iter().find_map(|e| by_ext(&extension(Path::new(&e.name))?));
    }
    by_ext(&ext)
}

/// A path in lower case: FAT file names are case-insensitive, so a .cue or .m3u may name a file
/// in a different case.
fn lower(p: &Path) -> PathBuf { PathBuf::from(p.to_string_lossy().to_lowercase()) }

/// Files that are part of another game (in lower case): those a .cue or .m3u in the same folder
/// refers to.
fn parts_of_other_games(roms: &Path, files: &[PathBuf]) -> HashSet<PathBuf> {
    let mut parts = HashSet::new();
    for f in files {
        let ext = extension(f);
        if !matches!(ext.as_deref(), Some("cue" | "m3u")) { continue; }
        let Ok(text) = std::fs::read_to_string(roms.join(f)) else { continue };
        let dir = f.parent().unwrap_or(Path::new(""));
        for line in text.lines().map(str::trim) {
            let name = if ext.as_deref() == Some("cue") {
                // FILE "Track 01.bin" BINARY
                line.strip_prefix("FILE ").and_then(|r| r.split('"').nth(1))
            } else {
                Some(line).filter(|l| !l.is_empty() && !l.starts_with('#'))
            };
            if let Some(n) = name { parts.insert(lower(&dir.join(n.trim_start_matches("./")))); }
        }
    }
    parts
}

#[cfg(test)]
pub(crate) mod tests;
