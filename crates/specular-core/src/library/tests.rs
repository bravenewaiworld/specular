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
    assert_eq!(g.settings_layer().path(Path::new("/sd/configs")), Path::new("/sd/configs/games/gba/RPG/Golden Sun (USA, Europe).json"));
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

#[test]
fn reads_zip_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let p = dir.join("a.zip");
    std::fs::File::create(&p).unwrap().write_all(&zip_of(&["a.gb", "b.txt"])).unwrap();
    let entries: Vec<(String, u32)> = zip::entries(&p).unwrap().into_iter().map(|e| (e.name, e.crc32)).collect();
    assert_eq!(entries, [("a.gb".into(), 0x1000), ("b.txt".into(), 0x1001)]);
}
