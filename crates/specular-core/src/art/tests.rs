use std::io::Write;

use super::*;
use super::db::normalise;
use crate::library::builtin_systems;

const TABLE: &str = "# test table v1
1F1C08FB\tBPEE\tPokemon - Emerald Version (USA, Europe)
11111111\tBPEJ\tPocket Monsters - Emerald (Japan)
22222222\tBPEE\tPokemon - Emerald Version (USA, Europe) (Beta)
33333333\t\tLegend of Zelda, The - The Minish Cap (Europe)
44444444\t\tLegend of Zelda, The - The Minish Cap (USA)
";

fn gba() -> SystemDecl { builtin_systems().into_iter().find(|s| s.id == "gba").unwrap() }

fn rom(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::File::create(&p).unwrap().write_all(bytes).unwrap();
    p
}

#[test]
fn looks_up_by_checksum_code_and_name() {
    let db = GameDb::parse(TABLE);
    assert_eq!(db.version, "test table v1");
    assert_eq!(db.by_crc(0x1F1C08FB), Some("Pokemon - Emerald Version (USA, Europe)"));
    // Released before beta.
    assert_eq!(db.by_code("BPEE"), Some("Pokemon - Emerald Version (USA, Europe)"));
    // Exact name, else the preferred region among normalised matches.
    assert_eq!(db.by_name("Legend of Zelda, The - The Minish Cap (Europe)"), Some("Legend of Zelda, The - The Minish Cap (Europe)"));
    assert_eq!(db.by_name("The Legend of Zelda - The Minish Cap"), Some("Legend of Zelda, The - The Minish Cap (USA)"));
    assert_eq!(db.by_name("Unknown Game"), None);
    assert_eq!(normalise("Legend of Zelda, The - Minish Cap (USA) [!]"), "legend of zelda minish cap");
    assert_eq!(normalise("Mario & Luigi"), "mario and luigi");
}

#[test]
fn identifies_hacks_by_header_code_and_zips_by_their_directory() {
    let dir = tempfile::tempdir().unwrap();
    let db = GameDb::parse(TABLE);
    let mut hack = vec![0u8; 0x200];
    hack[0xac..0xb0].copy_from_slice(b"BPEE");
    let id = identify(&rom(dir.path(), "Emerald Rogue.gba", &hack), &gba(), &db).unwrap();
    assert_eq!(id, Some(Identity { name: "Pokemon - Emerald Version (USA, Europe)".into(), by: Method::HeaderCode }));
    // A lower-case "code" is not a game code; nor does this file's name match anything.
    hack[0xac..0xb0].copy_from_slice(b"bpee");
    assert_eq!(identify(&rom(dir.path(), "x.gba", &hack), &gba(), &db).unwrap(), None);
    // A zip's checksums come from its directory, where `zip_of` gives the first file 0x1000.
    let table = format!("{TABLE}00001000\t\tSome Game (USA)\n");
    let db = GameDb::parse(&table);
    let zip = rom(dir.path(), "renamed.zip", &crate::library::tests::zip_of(&["game.gba"]));
    assert_eq!(identify(&zip, &gba(), &db).unwrap().map(|i| (i.name, i.by)), Some(("Some Game (USA)".into(), Method::Crc)));
}

#[test]
fn skips_snes_copier_headers_when_hashing() {
    let dir = tempfile::tempdir().unwrap();
    let snes = builtin_systems().into_iter().find(|s| s.id == "snes").unwrap();
    let body = vec![7u8; 1024];
    let crc = crc32fast::hash(&body);
    let db = GameDb::parse(&format!("#\n{crc:08X}\t\tGame (USA)\n"));
    let headered = [vec![0u8; 512], body.clone()].concat();
    for (name, bytes) in [("a.sfc", &body), ("b.smc", &headered)] {
        assert_eq!(identify(&rom(dir.path(), name, bytes), &snes, &db).unwrap().map(|i| i.by), Some(Method::Crc), "{name}");
    }
}

#[test]
fn index_keeps_entries_for_unchanged_files_and_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identify/gba.json");
    let rom = Path::new("gba/Emerald Rogue.gba");
    let e = Entry { size: 10, modified: 20, identity: Some(Identity { name: "Pokemon".into(), by: Method::HeaderCode }), missing_since: Some(30) };
    let mut index = ArtIndex::load(path.clone(), "v1");
    index.set(rom, &e);
    index.save().unwrap();
    let index = ArtIndex::load(path.clone(), "v1");
    assert_eq!(index.get(rom, 10, 20), Some(e));
    assert_eq!(index.get(rom, 11, 20), None); // the file changed
    assert_eq!(ArtIndex::load(path, "v2").get(rom, 10, 20), None); // a new game table
}

#[test]
fn thumbnail_addresses() {
    assert_eq!(thumbnail_url(&gba(), "Pokemon - Emerald Version (USA, Europe)"),
        "http://thumbnails.libretro.com/Nintendo%20-%20Game%20Boy%20Advance/Named_Boxarts/Pokemon%20-%20Emerald%20Version%20%28USA%2C%20Europe%29.png");
    assert!(thumbnail_url(&gba(), "Tom & Jerry: Tales").ends_with("/Tom%20_%20Jerry_%20Tales.png"));
}

#[test]
fn builtin_tables_parse() {
    for s in builtin_systems().iter().filter(|s| !s.game_db.is_empty()) {
        let db = GameDb::parse(s.game_db);
        assert!(db.version.contains("libretro-database"), "{}", s.id);
        assert!(s.game_db.lines().count() > 1000 && s.game_db.lines().nth(1).and_then(|l| db.by_crc(u32::from_str_radix(&l[..8], 16).ok()?)).is_some(), "{}", s.id);
    }
}
