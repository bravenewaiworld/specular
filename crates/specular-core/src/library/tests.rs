use std::io::Write;
use std::path::Path;

use super::*;

fn touch(root: &Path, rel: &str, content: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// A minimal zip holding files with these names, without contents; file i's checksum is 0x1000 + i.
pub(crate) fn zip_of(names: &[&str]) -> Vec<u8> {
    let (mut out, mut dir) = (Vec::new(), Vec::new());
    for (i, n) in names.iter().enumerate() {
        let offset = out.len() as u32;
        out.extend(b"PK\x03\x04"); out.extend([0u8; 22]); out.extend((n.len() as u16).to_le_bytes()); out.extend([0u8; 2]); out.extend(n.as_bytes());
        dir.extend(b"PK\x01\x02"); dir.extend([0u8; 12]); dir.extend((0x1000 + i as u32).to_le_bytes()); dir.extend([0u8; 8]); dir.extend((n.len() as u16).to_le_bytes()); dir.extend([0u8; 12]);
        dir.extend(offset.to_le_bytes()); dir.extend(n.as_bytes());
    }
    let start = out.len() as u32;
    out.extend(&dir);
    out.extend(b"PK\x05\x06"); out.extend([0u8; 4]);
    out.extend((names.len() as u16).to_le_bytes()); out.extend((names.len() as u16).to_le_bytes());
    out.extend((dir.len() as u32).to_le_bytes()); out.extend(start.to_le_bytes()); out.extend([0u8; 2]);
    out
}

#[test]
fn finds_games_by_extension_and_zip_contents() {
    let dir = tempfile::tempdir().unwrap();
    let roms = dir.path();
    touch(roms, "gba/Golden Sun (USA, Europe).gba", b"");
    touch(roms, "gba/RPG/Mother 3 (Japan).GBA", b""); // any folder depth, any case
    touch(roms, "snes/Super Metroid (Japan, USA).zip", &zip_of(&["readme.txt", "Super Metroid (Japan, USA).sfc"]));
    touch(roms, "misc/notes.txt", b"");
    touch(roms, "gba/._Golden Sun (USA, Europe).gba", b""); // macOS metadata
    touch(roms, ".Trashes/old.gba", b"");
    touch(roms, "broken.zip", b"not a zip");
    let games = scan(roms, &builtin_systems());
    let found: Vec<(&str, &str)> = games.iter().map(|g| (g.system, g.title.as_str())).collect();
    assert_eq!(found, [("gba", "Golden Sun"), ("gba", "Mother 3"), ("snes", "Super Metroid")]);
}

#[test]
fn disc_tracks_and_playlist_discs_are_not_games() {
    let dir = tempfile::tempdir().unwrap();
    let roms = dir.path();
    touch(roms, "ps1/Vagrant Story (USA).cue", b"FILE \"Vagrant Story (USA).bin\" BINARY\n  TRACK 01 MODE2/2352\n");
    touch(roms, "ps1/Vagrant Story (USA).bin", b"");
    touch(roms, "ps1/FF9/Final Fantasy IX (USA).m3u", b"Final Fantasy IX (USA) (Disc 1).chd\n./final fantasy ix (usa) (disc 2).chd\n");
    touch(roms, "ps1/FF9/Final Fantasy IX (USA) (Disc 1).chd", b"");
    touch(roms, "ps1/FF9/Final Fantasy IX (USA) (Disc 2).chd", b"");
    let games = scan(roms, &builtin_systems());
    let paths: Vec<&Path> = games.iter().map(|g| g.path.as_path()).collect();
    assert_eq!(paths, [Path::new("ps1/FF9/Final Fantasy IX (USA).m3u"), Path::new("ps1/Vagrant Story (USA).cue")]);
}

#[test]
fn saves_mirror_the_rom_tree() {
    let g = Game { system: "gba", path: "gba/RPG/Golden Sun (USA, Europe).gba".into(), title: "Golden Sun".into(), sort_key: String::new() };
    assert_eq!(g.save_path(Path::new("/sd/saves"), "srm"), Path::new("/sd/saves/gba/RPG/Golden Sun (USA, Europe).srm"));
    assert_eq!(g.stem(), "Golden Sun (USA, Europe)");
    assert_eq!(g.settings_layer().path(Path::new("/sd/config/v0")), Path::new("/sd/config/v0/games/gba/RPG/Golden Sun (USA, Europe).json"));
}

#[test]
fn titles_and_sorting() {
    assert_eq!(display_title("Legend of Zelda, The - The Minish Cap (USA) (Rev 1)"), "The Legend of Zelda - The Minish Cap");
    assert_eq!(display_title("Tetris [!]"), "Tetris");
    assert_eq!(display_title("(Homebrew)"), "(Homebrew)");
    let mut names = ["Mega Man 10", "mega man 2", "The Adventures of Batman", "Castlevania"];
    names.sort_by_key(|n| sort_key(n));
    assert_eq!(names, ["The Adventures of Batman", "Castlevania", "mega man 2", "Mega Man 10"]);
    // Accented letters sort with their plain ones.
    let mut names = ["Pokemon Pinball", "Zelda", "Pokémon Emerald", "Æon"];
    names.sort_by_key(|n| sort_key(n));
    assert_eq!(names, ["Æon", "Pokémon Emerald", "Pokemon Pinball", "Zelda"]);
}

#[test]
fn index_letters() {
    for (title, letter) in [("Golden Sun", 'G'), ("the Legend of Zelda", 'L'), ("Éclair", 'E'), ("Æon", 'A'), ("1942", '#'), ("'98 Koshien", '#'), ("ポケモン", '#'), ("", '#')] {
        assert_eq!(index_letter(title), letter, "{title}");
    }
}

/// A zip holding one file, `data`, deflated or stored as it is.
fn zip_with(name: &str, data: &[u8], deflate: bool) -> Vec<u8> {
    let body = if deflate {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    } else { data.to_vec() };
    let (crc, n) = (crc32fast::hash(data).to_le_bytes(), (name.len() as u16).to_le_bytes());
    let method = (if deflate { 8u16 } else { 0 }).to_le_bytes();
    let sizes = [(body.len() as u32).to_le_bytes(), (data.len() as u32).to_le_bytes()].concat();
    let mut out = [&b"PK\x03\x04"[..], &[0; 4], &method, &[0; 4], &crc, &sizes, &n, &[0; 2], name.as_bytes(), &body].concat();
    let start = out.len() as u32;
    let dir = [&b"PK\x01\x02"[..], &[0; 6], &method, &[0; 4], &crc, &sizes, &n, &[0; 12], &[0; 4], name.as_bytes()].concat();
    out.extend(&dir);
    out.extend([&b"PK\x05\x06"[..], &[0; 4], &[1, 0, 1, 0], &(dir.len() as u32).to_le_bytes(), &start.to_le_bytes(), &[0; 2]].concat());
    out
}

#[test]
fn unpacks_a_file_from_a_zip_checking_it() {
    let tmp = tempfile::tempdir().unwrap();
    let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    for deflate in [false, true] {
        let p = tmp.path().join("a.zip");
        std::fs::write(&p, zip_with("game.gba", &data, deflate)).unwrap();
        let entries = zip::entries(&p).unwrap();
        assert_eq!((entries[0].name.as_str(), entries[0].size), ("game.gba", data.len() as u64));
        let mut out = Vec::new();
        zip::unpack(&p, &entries[0], &mut out).unwrap();
        assert!(out == data, "deflate: {deflate}");
        // A damaged byte is caught by the checksum (stored) or the decoder (deflated).
        let mut zip = std::fs::read(&p).unwrap();
        zip[30 + 8 + 100] ^= 0xff;
        std::fs::write(&p, zip).unwrap();
        assert!(zip::unpack(&p, &entries[0], &mut Vec::new()).is_err());
    }
}

#[test]
fn reads_zip_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let p = dir.join("a.zip");
    std::fs::File::create(&p).unwrap().write_all(&zip_of(&["a.gb", "b.txt"])).unwrap();
    let entries: Vec<(String, u32)> = zip::entries(&p).unwrap().into_iter().map(|e| (e.name, e.crc32)).collect();
    assert_eq!(entries, [("a.gb".into(), 0x1000), ("b.txt".into(), 0x1001)]);
}

#[test]
fn each_system_has_a_colour_of_its_own_that_isnt_the_default_accent() {
    let tints: Vec<&str> = builtin_systems().iter().map(|s| s.tint).collect();
    for t in &tints { assert!(crate::palette::find(t).is_some(), "{t} isn't in the palette"); }
    assert_eq!(tints.iter().collect::<std::collections::HashSet<_>>().len(), tints.len(), "two systems share a colour: {tints:?}");
    assert!(!tints.contains(&"teal")); // `ACCENT`'s default
}

/// The splash shown while a game starts finds its core's credit by name: each `CoreDecl::name`
/// must be the name on a `core:` line.
#[test]
fn every_core_is_in_the_credits_by_its_name() {
    let tsv = include_str!("../../../../data/credits.tsv");
    let listed: Vec<&str> = tsv.lines().filter_map(|l| l.strip_prefix("core:")?.split('\t').nth(1)).collect();
    for core in cores(&builtin_systems()) { assert!(listed.contains(&core.name), "data/credits.tsv has no core named {}", core.name); }
}

#[test]
fn a_missing_bios_stops_only_the_games_that_need_one() {
    let systems = builtin_systems();
    let core = |id: &str| systems.iter().find(|s| s.id == id).unwrap().cores[0];
    let bios = tempfile::tempdir().unwrap();
    let (nes, fds, ps1) = (core("nes"), Path::new("Zelda.fds"), core("ps1"));
    assert!(nes.check_bios(Path::new("Mario.nes"), bios.path()).is_ok());
    assert_eq!(nes.check_bios(fds, bios.path()).unwrap_err(), "Famicom Disk System games need disksys.rom in the bios folder");
    assert!(ps1.check_bios(Path::new("Game.cue"), bios.path()).is_ok()); // OpenBIOS instead
    touch(bios.path(), "DISKSYS.ROM", b""); // FAT names in any case
    assert!(nes.check_bios(fds, bios.path()).is_ok());
}
