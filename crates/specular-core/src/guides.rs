//! The guide files Specular writes into the SD card's folders (`<folder>/README.txt`): what goes in
//! each folder, and how, for someone looking at the card on a PC. They are made from what Specular
//! knows (its systems, their cores and BIOS files), so they can't drift from the code, and written
//! the way the settings schemas are (`store::store_generated`: read-only, and only when their text
//! changes). Specular never reads them back. Crystaline OS writes the card's own README.txt and those in `updates/` and
//! `backups/`; Specular leaves those alone.

use std::path::PathBuf;

use crate::library::{self, SystemDecl, user_cores};
use crate::settings::migrate::CONFIG;

/// A guide file's name.
pub const README: &str = "README.txt";

/// Each guide's first line.
const HEADER: &str = "Specular, the device's game menu, writes this file: changes to it are lost.";

/// Where Specular lists what it can't use, as home shows it ("3 things need attention").
const ATTENTION: &str = "on the device's home screen, under \"things need attention\"";

/// Every guide file, by its path relative to the data root (the card). `systems` is every system
/// Specular declares, each with the cores that serve it (`user_cores::served`: none for a system
/// no emulator plays); `old_folders`, the older config folders still on the card
/// (`settings::migrate::old_folders`).
pub fn guides(systems: &[SystemDecl], old_folders: &[&str]) -> Vec<(PathBuf, String)> {
    let mut files = vec![
        ("roms", roms(systems)),
        ("bios", bios(systems)),
        ("cores", cores(systems)),
        ("fonts", fonts()),
        ("cache", cache()),
    ];
    files.extend(old_folders.iter().map(|&f| (f, old_config())));
    files.into_iter().map(|(folder, text)| (PathBuf::from(folder).join(README), text)).collect()
}

fn roms(systems: &[SystemDecl]) -> String {
    let (playable, unplayable): (Vec<&SystemDecl>, Vec<&SystemDecl>) = systems.iter().partition(|s| !s.cores.is_empty());
    let has = |ext: &str| playable.iter().any(|s| s.extensions.contains(&ext));
    let mut t = Text::new();
    t.para("Your games go in this folder, in any folders you like inside it (by system, by genre, or none). Specular tells each game's system by its file type:");
    for s in &playable { t.item(&format!("{}: {}", s.name, types(s))); }
    t.para("A .zip holding one game's file works too.");
    if has("cue") { t.para("A .cue file lists a disc's tracks (its .bin files): keep them in the same folder as the .cue. A .bin without its .cue isn't shown."); }
    if has("m3u") { t.para("For a game on several discs, an .m3u file lists the disc files, one file name per line: keep the discs in the same folder as the .m3u. The game is shown once, and the menu over the game (the Menu button) changes discs."); }
    if !unplayable.is_empty() {
        let names: Vec<String> = unplayable.iter().map(|s| format!("{} ({})", s.name, types(s))).collect();
        t.para(&format!("No emulator on the device plays these yet, so their games aren't shown: {}. To play them, add an emulator core for them to cores/ (see cores/README.txt).", names.join(", ")));
    }
    t.para("Box art: to show your own picture for a game, put it beside the game, with the same name, as a .png, .jpg or .jpeg (Tetris.png beside Tetris.gb). Without one, Specular downloads the game's box art when the device is online.");
    t.para(&format!("Save states go in a hidden .states folder in each folder of games. Files and folders whose names start with a dot are skipped, and so are folders more than {} levels inside roms/. The games' own saves are in saves/, at the top of the card.", library::MAX_DEPTH));
    t.0
}

fn bios(systems: &[SystemDecl]) -> String {
    let mut t = Text::new();
    t.para("BIOS files: the start-up program of a console, which some emulators need for some games. Specular doesn't come with them; copy them from your own console. Put them directly in this folder, named as below (in capital or small letters: SCPH5501.BIN or scph5501.bin).");
    let cores = library::cores(systems);
    for &c in &cores {
        let plays: Vec<&str> = systems.iter().filter(|s| s.cores.iter().any(|k| k.id() == c.id())).map(|s| s.name).collect();
        let added = if c.user.is_some() { "; a core added to cores/" } else { "" };
        t.para(&format!("{} ({}{added})", c.name, plays.join(", ")));
        let optional = c.user.map_or(&[][..], |u| &u.optional_bios[..]);
        if c.bios.is_empty() && optional.is_empty() { t.item("Needs none."); }
        for b in c.bios {
            let (files, it) = if b.files.len() > 1 { (format!("one of {}", b.files.join(", ")), "one") } else { (b.files.join(", "), "it") };
            t.item(&match b.fallback {
                Some(f) => format!("{} use {files}. Without {it}, {} uses {f} instead, which may not run every game.", b.games, c.name),
                None => format!("{} need {files}. Without {it}, they don't start.", b.games),
            });
        }
        if !optional.is_empty() { t.item(&format!("Optional, for some games: {}.", optional.join(", "))); }
    }
    let folders = cores.iter().flat_map(|c| c.bios).flat_map(|b| b.files).any(|f| f.contains('/'));
    if folders { t.para("But a name with a folder in it (dc/dc_boot.bin) goes in that folder inside this one, with capital and small letters exactly as shown."); }
    t.0
}

fn cores(systems: &[SystemDecl]) -> String {
    let own: Vec<String> = library::cores(systems).iter().filter(|c| c.user.is_none()).map(|c| format!("{}.so", c.library)).collect();
    let mut t = Text::new();
    t.para("Emulators you add (\"cores\"), beside Specular's own. Specular runs libretro cores, as RetroArch does. For each one, put two files directly in this folder:");
    t.item(&format!("<name>_libretro.so, the core itself, built for Linux on this device's processor: {}. A core built for another processor (a PC's, say) isn't used.", user_cores::this_processor()));
    t.item("<name>_libretro.info, its core info file, which says which systems it plays.");
    t.para("The libretro buildbot (buildbot.libretro.com) has the cores: pick the Linux build for this processor, and unzip it before copying the .so here. libretro's core info repository (github.com/libretro/libretro-core-info) has every .info file.");
    t.para("A core plays a system below if its .info names the system's database (or, if it names none, its systemid), and its supported_extensions, if it lists any, include one of the system's file types:");
    for s in systems {
        let by: Vec<&str> = s.cores.iter().map(|c| c.name).collect();
        let now = if by.is_empty() { "No emulator plays it yet.".to_string() } else { format!("Played now by {}.", by.join(", ")) };
        t.item(&format!("{}: database \"{}\" (systemid {}); {}. {now}", s.name, s.libretro_name, s.libretro_id, types(s)));
    }
    t.para(&format!("A core for a system Specular already plays can be chosen with the Emulator setting of that system, or of one game; Specular's own stays the default. A core with the same file name as one of Specular's own ({}) isn't used: Specular uses its own.", own.join(", ")));
    t.para(&format!("Specular finds out a core's options the first time the core plays a game. From then on they can be set in {CONFIG}/cores/<name>.json; <name>.schema.json beside it lists each option and its values."));
    t.para(&format!("Every core is listed on the device in Settings > Cores, with the files in this folder Specular doesn't use, and why; those files are also {ATTENTION}."));
    t.0
}

fn fonts() -> String {
    let mut t = Text::new();
    t.para("Fonts for letters Specular's own fonts lack, such as Japanese or Chinese in game names. Put TrueType or OpenType files (.ttf, .otf, .ttc) directly in this folder, not in folders inside it.");
    t.para(&format!("Specular reads them as it starts: restart the device after adding one. Files here it can't use are listed {ATTENTION}."));
    t.0
}

fn cache() -> String {
    let mut t = Text::new();
    t.para("Things Specular can make again: pictures of the games (downloaded box art, title screens and screenshots, and a picture of where you left off in each game), what it looked up about each game, and the options of the cores in cores/.");
    t.para("Deleting this folder, or anything in it, is safe: Specular makes it again. Pictures are downloaded again when the device is online, and a core's options are found out again when it next plays a game.");
    t.0
}

fn old_config() -> String {
    let mut t = Text::new();
    t.para(&format!("Specular no longer reads this folder: its settings were copied to {CONFIG}/, where they are kept now. This folder is left as it was, so an older version of Specular would still find its settings. Changes made here have no effect; delete it if you won't go back to an older version."));
    t.0
}

/// A system's file types: ".nes .fds".
fn types(s: &SystemDecl) -> String { s.extensions.iter().map(|x| format!(".{x}")).collect::<Vec<_>>().join(" ") }

/// A guide's text: `HEADER`, then paragraphs and list items, wrapped to `WIDTH` columns, as a
/// text editor may not wrap lines itself.
struct Text(String);

const WIDTH: usize = 78;

impl Text {
    fn new() -> Self { Self(format!("{HEADER}\n")) }

    fn para(&mut self, text: &str) { self.0.push('\n'); self.wrap(text, "", ""); }

    /// A list item, which follows a paragraph or another item with no blank line between.
    fn item(&mut self, text: &str) { self.wrap(text, "  - ", "    "); }

    /// Adds `text`, its first line after `first` and the others after `rest`.
    fn wrap(&mut self, text: &str, first: &str, rest: &str) {
        let (mut line, mut empty) = (first.to_string(), true);
        for word in text.split_whitespace() {
            if !empty && line.chars().count() + 1 + word.chars().count() > WIDTH {
                self.0 += &line;
                self.0.push('\n');
                (line, empty) = (rest.to_string(), true);
            }
            if !empty { line.push(' '); }
            line += word;
            empty = false;
        }
        self.0 += &line;
        self.0.push('\n');
    }
}

#[cfg(test)]
mod tests;
