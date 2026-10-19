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
    assert_eq!(db.by_name("Legend of Zelda, The - The Minish Cap (Europe)"), Ok(("Legend of Zelda, The - The Minish Cap (Europe)", Method::Name)));
    assert_eq!(db.by_name("The Legend of Zelda - The Minish Cap"), Ok(("Legend of Zelda, The - The Minish Cap (USA)", Method::Name)));
    assert_eq!(db.by_name("Unknown Game"), Err(NoMatch::Unknown));
    assert_eq!(normalise("Legend of Zelda, The - Minish Cap (USA) [!]"), "legend of zelda minish cap");
    assert_eq!(normalise("Mario & Luigi"), "mario and luigi");
    assert_eq!(normalise("Final Fantasy XIII-2 / Mega Man II / Street Fighter IV"), "final fantasy 13 2 mega man 2 street fighter 4");
    // Not numerals: out of range, written oddly, or inside a word.
    assert_eq!(normalise("Mix XL IIII Vixen"), "mix xl iiii vixen");
}

const NAMES: &str = "# names
00000001\t\tFinal Fantasy II (Japan)
00000002\t\tFinal Fantasy III (Japan)
00000003\t\tMega Man II (Europe)
00000004\t\tGolden Sun (USA, Europe)
00000005\t\tGolden Sun (Japan)
00000006\t\tPokemon - Ruby Version (USA)
00000007\t\tPokemon - Sapphire Version (USA)
00000008\t\tLegend of Zelda, The - The Minish Cap (Europe) (En,Fr,De,Es,It)
00000009\t\tLegend of Zelda, The - The Minish Cap (USA)
0000000A\t\tSonic Advance (USA)
0000000B\t\tSonic Advance 2 (USA)
";

#[test]
fn names_match_with_consistent_numerals_and_the_roms_own_region() {
    let db = GameDb::parse(NAMES);
    let name = |stem| db.by_name(stem).map(|(n, by)| (n.to_string(), by));
    let ok = |n: &str, by| Ok((n.to_string(), by));
    // Roman numerals and digits are the same number; sequels stay apart.
    assert_eq!(name("Mega Man 2"), ok("Mega Man II (Europe)", Method::Name));
    assert_eq!(name("Final Fantasy 2 (J)"), ok("Final Fantasy II (Japan)", Method::Name));
    assert_eq!(name("Final Fantasy III"), ok("Final Fantasy III (Japan)", Method::Name));
    // The ROM's own region first, in No-Intro's or GoodTools' tags; else USA first.
    assert_eq!(name("Golden Sun (J) [!]"), ok("Golden Sun (Japan)", Method::Name));
    assert_eq!(name("Golden Sun (Europe) (Rev 1)"), ok("Golden Sun (USA, Europe)", Method::Name));
    assert_eq!(name("Golden Sun (Korea)"), ok("Golden Sun (USA, Europe)", Method::Name));
    // Fuzzy: the one title holding the name's words, in order, with the same numbers.
    assert_eq!(name("Zelda - Minish Cap (Europe)"), ok("Legend of Zelda, The - The Minish Cap (Europe) (En,Fr,De,Es,It)", Method::FuzzyName));
    assert_eq!(name("Zelda Minish Cap"), ok("Legend of Zelda, The - The Minish Cap (USA)", Method::FuzzyName));
    assert_eq!(name("Pokemon Ruby"), ok("Pokemon - Ruby Version (USA)", Method::FuzzyName));
    assert_eq!(name("Fantasy III"), ok("Final Fantasy III (Japan)", Method::FuzzyName));
    assert_eq!(name("Sonic Advance II Hack"), Err(NoMatch::Unknown)); // extra words never match
    // Rejected: several titles, or only titles with other numbers.
    assert_eq!(name("Pokemon Version"), Err(NoMatch::Ambiguous(2, vec!["Pokemon - Ruby Version (USA)".into(), "Pokemon - Sapphire Version (USA)".into()])));
    assert_eq!(name("Final Fantasy"), Err(NoMatch::Numbers("Final Fantasy II (Japan)".into())));
    assert_eq!(name("Mega Man"), Err(NoMatch::Numbers("Mega Man II (Europe)".into())));
    // "sonic advance 2" has another number, which leaves one title.
    assert_eq!(name("Sonic"), ok("Sonic Advance (USA)", Method::FuzzyName));
    assert_eq!(name("(Homebrew)"), Err(NoMatch::Unknown));
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
    let e = Entry { size: 10, modified: 20, identity: Some(Identity { name: "Pokemon".into(), by: Method::HeaderCode }), missing: [Some(31), Some(30), None, None] };
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
    assert_eq!(thumbnail_url(&gba(), ArtKind::BoxArt, "Pokemon - Emerald Version (USA, Europe)").unwrap(),
        "http://thumbnails.libretro.com/Nintendo%20-%20Game%20Boy%20Advance/Named_Boxarts/Pokemon%20-%20Emerald%20Version%20%28USA%2C%20Europe%29.png");
    assert!(thumbnail_url(&gba(), ArtKind::BoxArt, "Tom & Jerry: Tales").unwrap().ends_with("/Tom%20_%20Jerry_%20Tales.png"));
    assert!(thumbnail_url(&gba(), ArtKind::TitleScreen, "X").unwrap().ends_with("/Named_Titles/X.png"));
    assert!(thumbnail_url(&gba(), ArtKind::Screenshot, "X").unwrap().ends_with("/Named_Snaps/X.png"));
    assert_eq!(thumbnail_url(&gba(), ArtKind::LeftOff, "X"), None);
}

#[test]
fn kinds_are_the_art_settings_values_and_fall_back_to_box_art() {
    let decls = crate::settings::declarations(&builtin_systems());
    let decl = decls.iter().find(|d| d.key == crate::settings::builtin::ART).unwrap();
    let crate::settings::SettingKind::Choice(values) = decl.kind else { panic!() };
    assert_eq!(values.iter().map(|(v, _)| *v).collect::<Vec<_>>(), ArtKind::ALL.map(ArtKind::id));
    let mut settings = crate::settings::Settings::new(decls);
    assert_eq!(kind(&settings, "gba"), ArtKind::BoxArt);
    settings.set(crate::settings::builtin::ART, &crate::settings::Layer::System("gba".into()), "left-off".into()).unwrap();
    assert_eq!((kind(&settings, "gba"), kind(&settings, "nes")), (ArtKind::LeftOff, ArtKind::BoxArt));
    for k in ArtKind::ALL { assert_eq!(k.fallbacks()[0], k); assert_eq!(k.fallbacks().last(), Some(&ArtKind::BoxArt)); }
    assert_eq!((ArtKind::BoxArt.shape(&gba()), ArtKind::Screenshot.shape(&gba())), ((1, 1), (240, 160)));
}

#[test]
fn pictures_are_the_kinds_then_box_art_with_the_users_first() {
    let dir = tempfile::tempdir().unwrap();
    let (roms, cache, rom) = (dir.path().join("roms"), dir.path().join("cache"), Path::new("gba/Golden Sun.gba"));
    let cached = |k| cache.join("art").join(k).join("gba/Golden Sun.png");
    assert_eq!(cached_image(&cache, ArtKind::TitleScreen, rom), cached("title"));
    assert_eq!(pictures(&roms, &cache, rom, ArtKind::BoxArt), [cached("boxart")]);
    std::fs::create_dir_all(roms.join("gba")).unwrap();
    std::fs::write(roms.join("gba/Golden Sun.jpg"), b"").unwrap();
    assert_eq!(pictures(&roms, &cache, rom, ArtKind::LeftOff), [cached("left-off"), roms.join("gba/Golden Sun.jpg"), cached("boxart")]);
}

#[test]
fn old_cache_moves_into_the_box_art_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (cache, art) = (dir.path(), dir.path().join("art"));
    move_old_cache(cache); // no cache yet
    assert!(!art.exists());
    for f in ["gba/Golden Sun.png", "nes/Zelda.png", "Top.png", "title/Odd.png", "boxart/nes/Zelda.png"] {
        std::fs::create_dir_all(art.join(f).parent().unwrap()).unwrap();
        std::fs::write(art.join(f), f).unwrap();
    }
    move_old_cache(cache);
    let entries = |dir: &Path| { let mut e: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name()).collect(); e.sort(); e };
    assert_eq!(entries(&art), ["boxart", "title"]);
    assert_eq!(entries(&art.join("boxart")), ["Top.png", "gba", "nes"]);
    assert_eq!(std::fs::read_to_string(art.join("boxart/gba/Golden Sun.png")).unwrap(), "gba/Golden Sun.png");
    assert!(art.join("title/Odd.png").exists());
    // One already moved (or downloaded since) wins.
    assert_eq!(std::fs::read_to_string(art.join("boxart/nes/Zelda.png")).unwrap(), "boxart/nes/Zelda.png");
    move_old_cache(cache);
    assert_eq!(entries(&art), ["boxart", "title"]);
}

#[test]
fn builtin_tables_parse() {
    for s in builtin_systems().iter().filter(|s| !s.game_db.is_empty()) {
        let db = GameDb::parse(s.game_db);
        assert!(db.version.contains("libretro-database"), "{}", s.id);
        assert!(s.game_db.lines().count() > 1000 && s.game_db.lines().nth(1).and_then(|l| db.by_crc(u32::from_str_radix(&l[..8], 16).ok()?)).is_some(), "{}", s.id);
        if s.id == "ps1" { assert_eq!(db.by_code("SCUS-94163"), Some("Final Fantasy VII (USA) (Disc 1)")); }
    }
}

/// A disc image with `SYSTEM.CNF` holding `cnf` in its root, in sectors of `size` bytes with the
/// data `offset` bytes in.
fn disc_image(cnf: &str, size: usize, offset: usize) -> Vec<u8> {
    let mut data = vec![[0u8; 2048]; 22];
    data[16][..6].copy_from_slice(b"\x01CD001");
    // The root directory's record: sector 20, one sector long.
    data[16][158..162].copy_from_slice(&20u32.to_le_bytes());
    data[16][166..170].copy_from_slice(&2048u32.to_le_bytes());
    // The root: ".", then SYSTEM.CNF in sector 21.
    let mut record = |at: usize, name: &[u8], sector: u32, len: u32| {
        let r = &mut data[20][at..];
        r[0] = (33 + name.len()).next_multiple_of(2) as u8;
        r[2..6].copy_from_slice(&sector.to_le_bytes());
        r[10..14].copy_from_slice(&len.to_le_bytes());
        r[32] = name.len() as u8;
        r[33..33 + name.len()].copy_from_slice(name);
        r[0] as usize
    };
    let next = record(0, b"\0", 20, 2048);
    record(next, b"SYSTEM.CNF;1", 21, cnf.len() as u32);
    data[21][..cnf.len()].copy_from_slice(cnf.as_bytes());
    data.iter().flat_map(|s| { let mut raw = vec![0u8; size]; raw[offset..offset + 2048].copy_from_slice(s); raw }).collect()
}

/// A PBP whose PARAM.SFO holds a TITLE and this DISC_ID.
fn pbp(disc_id: &str) -> Vec<u8> {
    let keys = b"TITLE\0DISC_ID\0\0\0";
    let values = [b"Some Game\0\0\0".as_slice(), format!("{disc_id}\0\0\0\0").as_bytes()].concat();
    let mut sfo = b"\0PSF\x01\x01\0\0".to_vec();
    for n in [20 + 2 * 16, 20 + 2 * 16 + keys.len(), 2] { sfo.extend((n as u32).to_le_bytes()); }
    // Each entry: the key's offset, the value's format, length, maximum length and offset.
    for (key_at, len, value_at) in [(0u16, 10u32, 0u32), (6, disc_id.len() as u32 + 1, 12)] {
        sfo.extend(key_at.to_le_bytes());
        sfo.extend(0x0204u16.to_le_bytes());
        for n in [len, len, value_at] { sfo.extend(n.to_le_bytes()); }
    }
    sfo.extend(keys);
    sfo.extend(values);
    // "\0PBP", a version, then the offsets of PARAM.SFO and of what follows it.
    let mut out = b"\0PBP\0\0\x01\0".to_vec();
    out.extend(40u32.to_le_bytes());
    out.extend((40 + sfo.len() as u32).to_le_bytes());
    out.resize(40, 0);
    out.extend(sfo);
    out.extend([0u8; 64]);
    out
}

#[test]
fn reads_playstation_disc_serials() {
    let dir = tempfile::tempdir().unwrap();
    let cnf = "BOOT = cdrom:\\SLUS_012.34;1\r\nTCB = 4\r\nEVENT = 10\r\n";
    let serial = |p: &Path| ps1_serial(p).map_err(|e| e.to_string());
    let some = Ok(Some("SLUS-01234".to_string()));
    // Raw Mode 2 sectors (as PlayStation .bin files are), raw Mode 1, and plain ISO.
    for (name, size, offset) in [("a.bin", 2352, 24), ("b.bin", 2352, 16), ("c.iso", 2048, 0)] {
        assert_eq!(serial(&rom(dir.path(), name, &disc_image(cnf, size, offset))), some, "{name}");
    }
    // Through a .cue's first file, and an .m3u's first disc.
    let cue = rom(dir.path(), "Game (USA).cue", b"FILE \"a.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n");
    assert_eq!(serial(&cue), some);
    assert_eq!(serial(&rom(dir.path(), "Game.m3u", b"#EXTM3U\r\nGame (USA).cue\r\nother.cue\r\n")), some);
    assert_eq!(serial(&rom(dir.path(), "loop.m3u", b"Game.m3u\n")), Ok(None));
    // A PBP's DISC_ID.
    assert_eq!(serial(&rom(dir.path(), "Game.pbp", &pbp("SLUS01234"))), some);
    // No serial: a disc that boots PSX.EXE, a CHD (not read).
    assert_eq!(serial(&rom(dir.path(), "d.bin", &disc_image("BOOT = cdrom:PSX.EXE;1\n", 2352, 24))), Ok(None));
    assert_eq!(serial(&rom(dir.path(), "e.chd", b"MComprHD")), Ok(None));
    // Truncated or garbage files are errors.
    let full = disc_image(cnf, 2352, 24);
    let garbage: [(&str, &[u8]); 5] = [("f.bin", &full[..2352 * 17]), ("g.bin", &full[..2352 * 21]), ("h.iso", &[7; 5000]),
        ("i.pbp", &pbp("SLUS01234")[..50]), ("j.pbp", b"\0PBP\0\0\x01\0\xff\xff\xff\xff\0\0\0\0")];
    for (name, bytes) in garbage {
        assert!(serial(&rom(dir.path(), name, bytes)).is_err(), "{name}");
    }
    assert!(serial(&rom(dir.path(), "k.cue", b"FILE \"missing.bin\" BINARY\n")).is_err());
    // A PBP whose DISC_ID points outside its PARAM.SFO has no serial.
    let mut bad = pbp("SLUS01234");
    bad[40 + 20 + 16 + 12] = 0xff;
    assert_eq!(serial(&rom(dir.path(), "l.pbp", &bad)), Ok(None));
}

#[test]
fn identifies_discs_by_serial_else_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let ps1 = builtin_systems().into_iter().find(|s| s.id == "ps1").unwrap();
    let db = GameDb::parse("#\nDEADBEEF\tSLUS-01234\tSome Game (USA) (Disc 1)\nDEADBEEE\tSLUS-01234\tSome Game (USA) (Disc 2)\n");
    rom(dir.path(), "track.bin", &disc_image("BOOT=cdrom:\\SLUS_012.34;1", 2352, 24));
    let cue = rom(dir.path(), "renamed.cue", b"FILE \"track.bin\" BINARY\n");
    assert_eq!(identify(&cue, &ps1, &db).unwrap(), Some(Identity { name: "Some Game (USA) (Disc 1)".into(), by: Method::DiscSerial }));
    // A disc that can't be read is still known by its name.
    let broken = rom(dir.path(), "Some Game (USA).cue", b"FILE \"missing.bin\" BINARY\n");
    assert_eq!(identify(&broken, &ps1, &db).unwrap().map(|i| i.by), Some(Method::Name));
}

#[test]
fn user_images_are_png_else_jpeg() {
    let dir = tempfile::tempdir().unwrap();
    let (roms, rom) = (dir.path(), Path::new("gba/Golden Sun.gba"));
    std::fs::create_dir_all(roms.join("gba")).unwrap();
    assert_eq!(user_image(roms, rom), None);
    for ext in ["jpeg", "jpg", "png"] {
        std::fs::write(roms.join(rom).with_extension(ext), b"").unwrap();
        assert_eq!(user_image(roms, rom), Some(roms.join(rom).with_extension(ext)));
    }
}
