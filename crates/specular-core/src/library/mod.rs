//! The game library: which systems exist, and which games are on the card.
//!
//! Games are found by scanning `roms/` recursively and matching each file to a system by its
//! extension. A zip is matched by the file inside it. Files that only make up another game (the
//! tracks a .cue sheet lists, the discs an .m3u playlist lists) aren't listed on their own.
//! Symbolic links are not followed.

mod names;
pub mod saves;
mod search;
pub mod user_cores;
pub mod zip;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub use names::{display_title, index_letter, sort_key};
pub use search::search;

use crate::settings::core_options::{mgba, nestopia, swanstation};

/// An emulated system and the files that hold its games.
#[derive(Clone, Debug)]
pub struct SystemDecl {
    /// Short, stable id, used in file paths and settings (e.g. "gba").
    pub id: &'static str,
    pub name: &'static str,
    /// Its colour, a palette id (`palette::PALETTE`): the light bar and the LEDs showing the first
    /// accent take it on its game list and while its games are played (see `App::tint`).
    pub tint: &'static str,
    /// Lower-case extensions of its game files, without the dot.
    pub extensions: &'static [&'static str],
    /// Its name in the libretro databases and thumbnail server (e.g. "Nintendo - Game Boy Advance").
    pub libretro_name: &'static str,
    /// Its `systemid` in libretro's core info files (e.g. "game_boy_advance"), which name only one
    /// system per core (see `user_cores`).
    pub libretro_id: &'static str,
    /// Its native picture size in pixels (width, height), which sizes a game's place on screen
    /// before its first frame arrives.
    pub screen: (u32, u32),
    /// Its games' checksums and names (see `art::GameDb`); empty if there is no table.
    pub game_db: &'static str,
    /// Where its ROM header holds a game code that the table lists as a serial: (offset, length).
    /// Identifies ROMs no checksum matches, such as hacks, which keep their base game's code.
    pub header_code: Option<(u64, usize)>,
    /// Reads a disc image's serial, as the table lists it (see `art::ps1_serial`). Its files are
    /// never hashed: an image is too large, and a .cue or .m3u isn't the disc itself.
    pub disc_serial: Option<fn(&Path) -> std::io::Result<Option<String>>>,
    /// The libretro cores that play its games, the default first (Specular's own, if it has one),
    /// then those the user added (`user_cores`); its "Core" setting chooses among several
    /// (`settings::core_of`). Empty while there is none: its games aren't listed.
    pub cores: Vec<&'static CoreDecl>,
    /// The size of a header some dumps put before the ROM (such as the 512 bytes that old SNES
    /// copier devices added), which the table's checksums leave out: skipped when hashing a file
    /// whose size is that much over a multiple of 1 KiB.
    pub dump_header: u64,
}

/// A libretro core, and how Specular runs it.
#[derive(Debug)]
pub struct CoreDecl {
    /// Its name for the player, e.g. "mGBA".
    pub name: &'static str,
    /// Its library's name without ".so", e.g. "mgba_libretro".
    pub library: &'static str,
    /// The options it declares, as JSON (`settings::core_options::CoreOptions`): its
    /// `data/cores/<id>.json`, which `make cores` writes from its build.
    pub options: &'static str,
    /// Specular's defaults for some of its options, by key: given to it at launch unless a
    /// settings file says otherwise. Its other options are left at its own defaults.
    pub defaults: &'static [(&'static str, &'static str)],
    /// Give it only the ROM's path, although it asks for the contents (see
    /// `LaunchSpec::rom_by_path` in specular-runtime).
    pub rom_by_path: bool,
    /// The libretro device for player 1's port, where the core's default lacks something Specular
    /// uses (`LaunchSpec::controller` in specular-runtime); None: its default.
    pub controller: Option<u32>,
    /// Its save states hold the save memory too, so loading one rolls the battery save back (see
    /// `LaunchSpec::states_restore_save` in specular-runtime).
    pub states_restore_save: bool,
    /// The BIOS files it looks for in `bios/`.
    pub bios: &'static [Bios],
    /// For a core the user added, what its `.info` says and where it is; None for one Specular
    /// builds, which is in Specular's own cores folder.
    pub user: Option<&'static user_cores::UserCore>,
}

/// BIOS files a core looks for in `bios/` for some of its games.
#[derive(Debug)]
pub struct Bios {
    /// Its games that use one, in the plural (e.g. "Famicom Disk System games"), for messages.
    pub games: &'static str,
    /// Their extensions; empty for all of the core's games.
    pub extensions: &'static [&'static str],
    /// The file names the core looks for, in lower case; any one of them will do. A name with a
    /// folder in it ("dc/dc_boot.bin") is looked for as it is.
    pub files: &'static [&'static str],
    /// What the core uses instead when none is there; None if those games can't start without.
    pub fallback: Option<&'static str>,
}

impl CoreDecl {
    /// Checks that `bios_dir` holds a BIOS file the game at `rom` needs, logging what was looked
    /// for when none is there. Err: why the game can't start, for the player. File names are
    /// matched ignoring case, as FAT cards do. In a zip, the game is the first file with one of
    /// `extensions` (its system's, which should be its core's, as the runtime unzips by those).
    pub fn check_bios(&self, rom: &Path, extensions: &[&str], bios_dir: &Path) -> Result<(), String> {
        let mut ext = extension(rom).unwrap_or_default();
        if ext == "zip" {
            ext = zip::entries(rom).unwrap_or_default().iter().filter_map(|e| extension(Path::new(&e.name))).find(|x| extensions.contains(&x.as_str())).unwrap_or_default();
        }
        let present: HashSet<String> = std::fs::read_dir(bios_dir).into_iter().flatten().flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_lowercase)).collect();
        for b in self.bios.iter().filter(|b| b.extensions.is_empty() || b.extensions.contains(&ext.as_str())) {
            if b.files.iter().any(|f| present.contains(*f) || (f.contains('/') && bios_dir.join(f).is_file())) { continue; }
            let files = b.files.join(", ");
            eprintln!("game: no BIOS for {} in {} (looked for {files}){}", b.games, bios_dir.display(),
                b.fallback.map_or(String::new(), |f| format!("; {} uses {f}", self.name)));
            if b.fallback.is_none() { return Err(format!("{} need {} in the bios folder", b.games, b.which())); }
        }
        Ok(())
    }

    /// Why the game stopped, for the player, when the core says its stand-in for a missing BIOS
    /// file (`Bios::fallback`) has halted it for good (specular-runtime's `Outputs::on_bios_halt`):
    /// which files to put where. Names the first `Bios` with a fallback, whatever the game (each
    /// core has at most one).
    pub fn bios_halt_notice(&self) -> String {
        match self.bios.iter().find_map(|b| Some((b, b.fallback?))) {
            Some((b, fallback)) => format!("{} ran the game on {fallback}, which lacks something the game uses. It may need a BIOS file: put {} in the bios folder on the SD card",
                self.name, b.which()),
            None => format!("{}'s built-in BIOS halted it", self.name),
        }
    }
}

impl Bios {
    /// Its files, for the player: "disksys.rom", or "one of a.bin, b.bin".
    fn which(&self) -> String {
        let files = self.files.join(", ");
        if self.files.len() > 1 { format!("one of {files}") } else { files }
    }
}

impl CoreDecl {
    /// Its short name, as `tools/build-cores.sh` builds it: its library's, less "_libretro"
    /// ("mgba"). Names its option list in `data/cores/` and its options' file in the config folder.
    pub fn id(&self) -> &'static str { self.library.strip_suffix("_libretro").unwrap_or(self.library) }

    /// Specular's default for one of its options (`defaults`), if it has one.
    pub fn default_of(&self, key: &str) -> Option<&'static str> { self.defaults.iter().find(|(k, _)| *k == key).map(|(_, v)| *v) }
}

/// mGBA, for GB, GBC and GBA alike. "Remove Known" skips the busy-wait loops of known games, which
/// saves battery without changing how they run (TrimUI's own setting too). It is mGBA's default
/// too; it is Specular's so that a change to mGBA's doesn't change it. It gets only the ROM's path,
/// and our build (tools/core-patches/mgba) maps that file instead of copying it. Given the
/// contents, mGBA copies them, and copies a GBA ROM again on the first write to its cartridge area
/// (as Pokémon games' clock does): 64 MB of memory for a 32 MB ROM.
const MGBA: &CoreDecl = &CoreDecl {
    name: "mGBA", library: "mgba_libretro", options: mgba::LIST,
    defaults: &[(mgba::IDLE_OPTIMIZATION, mgba::idle_optimization::REMOVE_KNOWN)], rom_by_path: true,
    controller: None, states_restore_save: false, bios: &[], user: None,
};

/// Nestopia, for the NES and the Famicom Disk System. Its NTSC filter is off: it widens the
/// picture to 602 pixels, blurring it, and costs time. Its states hold the battery memory
/// (docs/runtime.md).
const NESTOPIA: &CoreDecl = &CoreDecl {
    name: "Nestopia", library: "nestopia_libretro", options: nestopia::LIST,
    defaults: &[(nestopia::BLARGG_NTSC_FILTER, nestopia::blargg_ntsc_filter::DISABLED)], rom_by_path: false,
    controller: None, states_restore_save: true,
    bios: &[Bios { games: "Famicom Disk System games", extensions: &["fds"], files: &["disksys.rom"], fallback: None }], user: None,
};

/// SwanStation, for the PS1, with its software renderer (the hardware ones need a GL context
/// shared with the UI). Without a BIOS it runs its built-in OpenBIOS, which some games don't run
/// with. It also recognises a known BIOS dump by its contents, whatever its name; `check_bios`
/// goes by name only, so it may log a missing BIOS that the core then finds. Its states hold the
/// memory card (docs/runtime.md). Player 1 has a DualShock: SwanStation's default pad has no
/// motors, so games couldn't rumble. A DualShock starts in digital mode, so a game that doesn't
/// know it sees a plain pad, and the d-pad drives it in analog mode too (its default options).
const SWANSTATION: &CoreDecl = &CoreDecl {
    name: "SwanStation", library: "swanstation_libretro", options: swanstation::LIST,
    defaults: &[(swanstation::GPU_RENDERER, swanstation::gpu_renderer::SOFTWARE)], rom_by_path: false,
    controller: Some(PS_DUALSHOCK), states_restore_save: true,
    bios: &[Bios { games: "PlayStation games", extensions: &[],
        files: &["scph5500.bin", "scph5501.bin", "scph5502.bin", "psxonpsp660.bin", "ps1_rom.bin"], fallback: Some("its built-in OpenBIOS") }], user: None,
};

/// libretro's DualShock for SwanStation: `RETRO_DEVICE_SUBCLASS(RETRO_DEVICE_ANALOG, 0)`.
const PS_DUALSHOCK: u32 = (1 << 8) | 5;

/// The cores of `systems`, each once, in their order.
pub fn cores(systems: &[SystemDecl]) -> Vec<&'static CoreDecl> {
    let mut cores: Vec<&'static CoreDecl> = Vec::new();
    for &c in systems.iter().flat_map(|s| &s.cores) { if !cores.iter().any(|k| k.id() == c.id()) { cores.push(c); } }
    cores
}

/// The systems Specular knows out of the box. Disc images for PS1 are listed by their .cue sheet,
/// .chd, .pbp or .m3u; a .bin is only a track of a .cue. Their colours differ, and none is teal,
/// the default accent, so the change to a system's colour shows (see docs/look-and-motion.md).
pub fn builtin_systems() -> Vec<SystemDecl> {
    vec![
        SystemDecl { id: "gb", name: "Game Boy", tint: "lime", extensions: &["gb"], libretro_id: "game_boy", libretro_name: "Nintendo - Game Boy",
            screen: (160, 144), game_db: include_str!("../../../../data/db/gb.tsv"), cores: vec![MGBA], header_code: None, disc_serial: None, dump_header: 0 },
        SystemDecl { id: "gbc", name: "Game Boy Color", tint: "rose", extensions: &["gbc"], libretro_id: "game_boy_color", libretro_name: "Nintendo - Game Boy Color",
            screen: (160, 144), game_db: include_str!("../../../../data/db/gbc.tsv"), cores: vec![MGBA], header_code: None, disc_serial: None, dump_header: 0 },
        SystemDecl { id: "gba", name: "Game Boy Advance", tint: "indigo", extensions: &["gba"], libretro_id: "game_boy_advance", libretro_name: "Nintendo - Game Boy Advance",
            screen: (240, 160), game_db: include_str!("../../../../data/db/gba.tsv"), cores: vec![MGBA], header_code: Some((0xac, 4)), disc_serial: None, dump_header: 0 },
        SystemDecl { id: "nes", name: "NES", tint: "coral", extensions: &["nes", "fds"], libretro_id: "nes", libretro_name: "Nintendo - Nintendo Entertainment System",
            screen: (256, 240), game_db: include_str!("../../../../data/db/nes.tsv"), cores: vec![NESTOPIA], header_code: None, disc_serial: None, dump_header: 0 },
        SystemDecl { id: "snes", name: "Super Nintendo", tint: "violet", extensions: &["sfc", "smc"], libretro_id: "super_nes", libretro_name: "Nintendo - Super Nintendo Entertainment System",
            screen: (256, 224), game_db: include_str!("../../../../data/db/snes.tsv"), cores: vec![], header_code: None, disc_serial: None, dump_header: 512 },
        SystemDecl { id: "md", name: "Mega Drive", tint: "sky", extensions: &["md", "gen", "smd"], libretro_id: "mega_drive", libretro_name: "Sega - Mega Drive - Genesis",
            screen: (320, 224), game_db: include_str!("../../../../data/db/md.tsv"), cores: vec![], header_code: None, disc_serial: None, dump_header: 0 },
        SystemDecl { id: "ps1", name: "PlayStation", tint: "amber", extensions: &["cue", "chd", "pbp", "m3u"], libretro_id: "playstation", libretro_name: "Sony - PlayStation",
            screen: (320, 240), game_db: include_str!("../../../../data/db/ps1.tsv"), cores: vec![SWANSTATION], header_code: None,
            disc_serial: Some(crate::art::ps1_serial), dump_header: 0 },
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

/// A file's extension, in lower case.
pub fn extension(p: &Path) -> Option<String> { p.extension()?.to_str().map(str::to_ascii_lowercase) }

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
        parts.extend(listed_files(ext.as_deref() == Some("cue"), &text).map(|n| lower(&dir.join(n))));
    }
    parts
}

/// The files a .cue sheet (if `cue`) or else an .m3u playlist lists, in order, relative to its
/// folder. A byte order mark (which Windows tools often write) is skipped.
pub(crate) fn listed_files(cue: bool, text: &str) -> impl Iterator<Item = &str> {
    text.trim_start_matches('\u{feff}').lines().map(str::trim).filter_map(move |line| {
        let name = if cue {
            // FILE "Track 01.bin" BINARY
            line.strip_prefix("FILE ").and_then(|r| r.split('"').nth(1))
        } else {
            Some(line).filter(|l| !l.is_empty() && !l.starts_with('#'))
        };
        name.map(|n| n.trim_start_matches("./"))
    })
}

#[cfg(test)]
pub(crate) mod tests;
