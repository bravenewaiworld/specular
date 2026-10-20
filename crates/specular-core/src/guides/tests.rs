use super::*;
use crate::library::builtin_systems;
use crate::library::user_cores::{UserCores, served, tests::{SNES9X, cores_dir, library}};

fn guide<'a>(all: &'a [(PathBuf, String)], folder: &str) -> &'a str {
    &all.iter().find(|(p, _)| *p == PathBuf::from(folder).join(README)).unwrap_or_else(|| panic!("no guide in {folder}")).1
}

/// The guides' text with its lines joined, to match sentences however they are wrapped.
fn flat(text: &str) -> String { text.split_whitespace().collect::<Vec<_>>().join(" ") }

#[test]
fn each_folder_has_a_guide_which_says_it_is_generated_and_fits_the_width() {
    let all = guides(&builtin_systems(), &["configs"]);
    let folders: Vec<String> = all.iter().map(|(p, _)| p.parent().unwrap().display().to_string()).collect();
    assert_eq!(folders, ["roms", "bios", "cores", "fonts", "cache", "configs"]);
    for (path, text) in &all {
        assert!(text.starts_with(HEADER), "{path:?}");
        // Only a word longer than a line (none here) could make one longer.
        assert!(text.lines().all(|l| l.chars().count() <= WIDTH), "{path:?}:\n{text}");
    }
    assert!(flat(guide(&all, "configs")).contains("its settings were copied to config/v0/"));
}

#[test]
fn the_roms_guide_lists_the_file_types_and_the_systems_no_emulator_plays() {
    let roms = flat(guide(&guides(&builtin_systems(), &[]), "roms"));
    assert!(roms.contains("Game Boy Advance: .gba"), "{roms}");
    assert!(roms.contains("PlayStation: .cue .chd .pbp .m3u"), "{roms}");
    assert!(roms.contains("A .bin without its .cue isn't shown"), "{roms}");
    assert!(roms.contains("so their games aren't shown: Super Nintendo (.sfc .smc), Mega Drive (.md .gen .smd)."), "{roms}");
    assert!(roms.contains(".png, .jpg or .jpeg"), "{roms}");
}

#[test]
fn a_core_added_to_the_card_moves_its_system_to_the_playable_ones_and_its_bios_files_are_listed() {
    let card = cores_dir(&[("snes9x_libretro.so", &library()), ("snes9x_libretro.info", SNES9X.as_bytes())]);
    let found = UserCores::default().find(card.path(), card.path(), &builtin_systems());
    let all = guides(&served(builtin_systems(), &found.cores), &[]);
    let roms = flat(guide(&all, "roms"));
    assert!(roms.contains("Super Nintendo: .sfc .smc"), "{roms}");
    assert!(roms.contains("so their games aren't shown: Mega Drive"), "{roms}");
    let bios = flat(guide(&all, "bios"));
    assert!(bios.contains("Nestopia (NES) - Famicom Disk System games need disksys.rom. Without it, they don't start."), "{bios}");
    assert!(bios.contains("SwanStation uses its built-in OpenBIOS instead"), "{bios}");
    assert!(bios.contains("Snes9x (Super Nintendo; a core added to cores/) - Optional, for some games: BS-X.bin, STBIOS.bin."), "{bios}");
    let cores = flat(guide(&all, "cores"));
    assert!(cores.contains(&user_cores::this_processor()), "{cores}");
    assert!(cores.contains("Super Nintendo: database \"Nintendo - Super Nintendo Entertainment System\" (systemid super_nes); .sfc .smc. Played now by Snes9x."), "{cores}");
    assert!(cores.contains("Mega Drive: database \"Sega - Mega Drive - Genesis\" (systemid mega_drive); .md .gen .smd. No emulator plays it yet."), "{cores}");
    assert!(cores.contains("(mgba_libretro.so, nestopia_libretro.so, swanstation_libretro.so)"), "{cores}");
}
