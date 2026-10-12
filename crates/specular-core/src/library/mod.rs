//! The game library: which systems exist, and which games are on the card.
//!
//! Games are found by scanning `roms/` recursively and matching each file to a system by its
//! extension. A zip is matched by the file inside it. Files that only make up another game (the
//! tracks a .cue sheet lists, the discs an .m3u playlist lists) aren't listed on their own.

mod names;
mod zip;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub use names::{display_title, sort_key};

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
}

/// The systems Specular knows out of the box. Disc images for PS1 are listed by their .cue sheet,
/// .chd, .pbp or .m3u; a .bin is only a track of a .cue.
pub fn builtin_systems() -> Vec<SystemDecl> {
    vec![
        SystemDecl { id: "gb", name: "Game Boy", extensions: &["gb"], libretro_name: "Nintendo - Game Boy" },
        SystemDecl { id: "gbc", name: "Game Boy Color", extensions: &["gbc"], libretro_name: "Nintendo - Game Boy Color" },
        SystemDecl { id: "gba", name: "Game Boy Advance", extensions: &["gba"], libretro_name: "Nintendo - Game Boy Advance" },
        SystemDecl { id: "nes", name: "NES", extensions: &["nes", "fds"], libretro_name: "Nintendo - Nintendo Entertainment System" },
        SystemDecl { id: "snes", name: "Super Nintendo", extensions: &["sfc", "smc"], libretro_name: "Nintendo - Super Nintendo Entertainment System" },
        SystemDecl { id: "md", name: "Mega Drive", extensions: &["md", "gen", "smd"], libretro_name: "Sega - Mega Drive - Genesis" },
        SystemDecl { id: "ps1", name: "PlayStation", extensions: &["cue", "chd", "pbp", "m3u"], libretro_name: "Sony - PlayStation" },
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

    /// Where a save of this game goes: `saves/` mirrors `roms/`, with the ROM's name and the
    /// save's own extension (e.g. "srm", or "state1" for a save state).
    pub fn save_path(&self, saves: &Path, extension: &str) -> PathBuf {
        saves.join(self.path.with_extension(extension))
    }
}

/// Folders never scanned: hidden ones (including macOS's) and Windows' volume information.
fn skip_dir(name: &str) -> bool { name.starts_with('.') || name.eq_ignore_ascii_case("System Volume Information") }

/// Files never listed: hidden ones, including the "._name" files macOS leaves on FAT cards.
fn skip_file(name: &str) -> bool { name.starts_with('.') }

/// How deep folders are followed, so a loop or a huge tree can't stall the scan.
const MAX_DEPTH: usize = 8;

/// Finds every game under `roms`, sorted by system order then title. Unreadable folders and
/// files are skipped; the scan never fails as a whole.
pub fn scan(roms: &Path, systems: &[SystemDecl]) -> Vec<Game> {
    let mut files = Vec::new();
    walk(roms, roms, 0, &mut files);
    let parts = parts_of_other_games(roms, &files);
    let mut games: Vec<Game> = files.iter()
        .filter(|f| !parts.contains(*f))
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
        } else if kind.is_file() && !skip_file(name) {
            if let Ok(rel) = e.path().strip_prefix(root) { out.push(rel.to_path_buf()); }
        }
    }
}

fn extension(p: &Path) -> Option<String> { p.extension()?.to_str().map(str::to_ascii_lowercase) }

/// The system a file belongs to: by its extension, or for a zip, by the first file inside it
/// with a known extension.
fn system_of(path: &Path, systems: &[SystemDecl]) -> Option<&'static str> {
    let by_ext = |ext: &str| systems.iter().find(|s| s.extensions.contains(&ext)).map(|s| s.id);
    let ext = extension(path)?;
    if ext == "zip" {
        return zip::file_names(path).ok()?.iter().find_map(|n| by_ext(&extension(Path::new(n))?));
    }
    by_ext(&ext)
}

/// Files that are part of another game: those a .cue or .m3u in the same folder refers to.
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
            if let Some(n) = name { parts.insert(dir.join(n)); }
        }
    }
    parts
}

#[cfg(test)]
mod tests;
