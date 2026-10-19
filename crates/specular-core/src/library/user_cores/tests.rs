use super::*;
use crate::library::builtin_systems;
use crate::settings::{self, Layer, Settings};

// Real core info files, from libretro-core-info (see LICENSE beside them).
const MGBA: &str = include_str!("mgba_libretro.info");
pub(crate) const SNES9X: &str = include_str!("snes9x_libretro.info");
const GENESIS_PLUS_GX: &str = include_str!("genesis_plus_gx_libretro.info");
pub(crate) const GAMBATTE: &str = include_str!("gambatte_libretro.info");

/// The first 20 bytes of an ELF shared library for `arch`, which is all `check_library` reads.
fn elf(arch: &Arch, kind: u16) -> Vec<u8> {
    let mut h = vec![0x7f, b'E', b'L', b'F', if arch.bits == 64 { 2 } else { 1 }, if arch.little { 1 } else { 2 }, 1];
    h.resize(16, 0);
    let half = |n: u16| if arch.little { n.to_le_bytes() } else { n.to_be_bytes() };
    h.extend(half(kind));
    h.extend(half(arch.machine));
    h
}

/// A library this machine can load, as far as `check_library` can tell.
pub(crate) fn library() -> Vec<u8> { elf(&Arch::OURS, 3) }

/// A card's `cores/`, with these files.
pub(crate) fn cores_dir(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, content) in files { std::fs::write(dir.path().join(name), content).unwrap(); }
    dir
}

fn systems_named(info: &str) -> Result<Vec<&'static str>, String> { systems_of(&Info::parse(info), &builtin_systems()) }

#[test]
fn a_cores_systems_are_those_its_info_names_by_database() {
    assert_eq!(systems_named(MGBA).unwrap(), ["gb", "gbc", "gba"]); // its systemid names only the GBA
    assert_eq!(systems_named(SNES9X).unwrap(), ["snes"]);
    assert_eq!(systems_named(GENESIS_PLUS_GX).unwrap(), ["md"]); // and Sega systems Specular doesn't have
    assert_eq!(systems_named(GAMBATTE).unwrap(), ["gb", "gbc"]);
}

#[test]
fn without_a_database_the_systemid_names_its_system() {
    assert_eq!(systems_named("corename = \"Mine\"\nsystemid = \"super_nes\"\n").unwrap(), ["snes"]);
    // A database wins over the systemid.
    assert_eq!(systems_named("systemid = \"super_nes\"\ndatabase = \"Nintendo - Game Boy\"\n").unwrap(), ["gb"]);
}

#[test]
fn a_core_for_no_system_specular_has_or_none_of_its_files_is_turned_away() {
    assert_eq!(systems_named("database = \"Sega - Saturn|NEC - PC Engine - TurboGrafx 16\"").unwrap_err(),
               "its .info is for Sega - Saturn, NEC - PC Engine - TurboGrafx 16, which Specular doesn't have");
    assert_eq!(systems_named("# no system\ncorename = \"Mine\"").unwrap_err(), "its .info is for no system, which Specular doesn't have");
    assert_eq!(systems_named("database = \"Nintendo - Game Boy|Nintendo - Game Boy Color\"\nsupported_extensions = \"gbc|cgb\"").unwrap(), ["gbc"]);
    assert_eq!(systems_named("database = \"Nintendo - Game Boy\"\nsupported_extensions = \"gbs\"").unwrap_err(),
               "it plays none of the files of Game Boy (its .info lists .gbs)");
}

#[test]
fn a_library_for_another_machine_is_turned_away_with_what_it_is_built_for() {
    let dir = tempfile::tempdir().unwrap();
    let check = |content: &[u8]| { let p = dir.path().join("x.so"); std::fs::write(&p, content).unwrap(); check_library(&p) };
    assert_eq!(check(&library()), Ok(()));
    let other = Arch { machine: if Arch::OURS.machine == 183 { 62 } else { 183 }, ..Arch::OURS };
    let err = check(&elf(&other, 3)).unwrap_err();
    assert!(err.starts_with("it is built for ") && err.contains(", and this device is "), "{err}");
    assert_eq!(check(&elf(&Arch { machine: 8, bits: 32, little: false }, 3)).unwrap_err(),
               format!("it is built for MIPS (32-bit, big-endian), and this device is {}", Arch::OURS));
    assert_eq!(check(&elf(&Arch::OURS, 2)).unwrap_err(), "it isn't a shared library");
    assert_eq!(check(b"#!/bin/sh\necho not a core, but long enough\n").unwrap_err(), "it isn't a library (not an ELF file)");
    assert_eq!(check(b"\x7fELF").unwrap_err(), "it isn't a library (it is too short)");
    // The cores built for this machine pass.
    for core in std::env::var_os("SPECULAR_CORES").and_then(|d| std::fs::read_dir(d).ok()).into_iter().flatten().flatten() {
        if core.path().extension().is_some_and(|e| e == "so") { assert_eq!(check_library(&core.path()), Ok(()), "{}", core.path().display()); }
    }
}

#[test]
fn cores_on_the_card_are_found_and_the_rest_turned_away_with_why() {
    let other = elf(&Arch { machine: if Arch::OURS.machine == 183 { 62 } else { 183 }, ..Arch::OURS }, 3);
    let dir = cores_dir(&[
        ("snes9x_libretro.so", &library()), ("snes9x_libretro.info", SNES9X.as_bytes()),
        ("gambatte_libretro.so", &library()), ("gambatte_libretro.info", GAMBATTE.as_bytes()),
        ("mgba_libretro.so", &library()), ("mgba_libretro.info", MGBA.as_bytes()),
        ("bare_libretro.so", &library()),
        ("arm_libretro.so", &other), ("arm_libretro.info", SNES9X.as_bytes()),
        ("saturn_libretro.so", &library()), ("saturn_libretro.info", b"database = \"Sega - Saturn\""),
        ("lonely_libretro.info", SNES9X.as_bytes()),
        ("snes9x.so", &library()),
        ("readme.txt", b"left alone"), (".hidden_libretro.so", b""),
    ]);
    let found = UserCores::default().find(dir.path(), &dir.path().join("learned"), &builtin_systems());
    let names: Vec<(&str, &[&str])> = found.cores.iter().map(|c| (c.name, c.user.unwrap().systems.as_slice())).collect();
    assert_eq!(names, [("Gambatte", &["gb", "gbc"][..]), ("Snes9x", &["snes"][..])]);
    let away: Vec<(&str, &str)> = found.turned_away.iter().map(|t| (t.file.as_str(), t.why.as_str())).collect();
    assert_eq!(away.len(), 6, "{away:#?}");
    assert!(away[0].0 == "arm_libretro.so" && away[0].1.starts_with("it is built for "), "{away:?}");
    assert_eq!(&away[1..], [
        ("bare_libretro.so", "there is no bare_libretro.info beside it, to say which systems it plays (the libretro buildbot has one for each core)"),
        ("lonely_libretro.info", "there is no lonely_libretro.so beside it"),
        ("mgba_libretro.so", "Specular has its own mGBA, which it uses instead"),
        ("saturn_libretro.so", "its .info is for Sega - Saturn, which Specular doesn't have"),
        ("snes9x.so", "a core's files are named <name>_libretro.so and <name>_libretro.info"),
    ]);
    let snes9x = found.cores[1];
    let u = snes9x.user.unwrap();
    assert_eq!((snes9x.id(), u.display_name.as_str(), u.notice.as_str()), ("snes9x", "Nintendo - SNES / SFC (Snes9x)", "Snes9x Team · Non-commercial"));
    assert!(snes9x.states_restore_save && !u.learned && snes9x.bios.is_empty()); // its firmware is optional
    assert!(core_options::list(snes9x).options.is_empty());
    // No folder: no cores.
    assert_eq!(UserCores::default().find(&dir.path().join("none"), dir.path(), &builtin_systems()), Found::default());
}

#[test]
fn a_user_core_serves_its_systems_after_their_own_core_and_one_without_becomes_playable() {
    let dir = cores_dir(&[("snes9x_libretro.so", &library()), ("snes9x_libretro.info", SNES9X.as_bytes()),
                          ("gambatte_libretro.so", &library()), ("gambatte_libretro.info", GAMBATTE.as_bytes())]);
    let found = UserCores::default().find(dir.path(), dir.path(), &builtin_systems());
    let systems = playable(builtin_systems(), &found.cores);
    let cores: Vec<(&str, Vec<&str>)> = systems.iter().map(|s| (s.id, s.cores.iter().map(|c| c.id()).collect())).collect();
    assert_eq!(cores, [("gb", vec!["mgba", "gambatte"]), ("gbc", vec!["mgba", "gambatte"]), ("gba", vec!["mgba"]), ("nes", vec!["nestopia"]),
                       ("snes", vec!["snes9x"]), ("ps1", vec!["swanstation"])]); // no core for the Mega Drive
}

#[test]
fn a_look_that_finds_nothing_changed_gives_the_same_cores() {
    let dir = cores_dir(&[("snes9x_libretro.so", &library()), ("snes9x_libretro.info", SNES9X.as_bytes())]);
    let mut cores = UserCores::default();
    let first = cores.find(dir.path(), dir.path(), &builtin_systems());
    assert_eq!(cores.find(dir.path(), dir.path(), &builtin_systems()), first);
    std::fs::write(dir.path().join("snes9x_libretro.info"), SNES9X.replace("Snes9x Team", "Others")).unwrap();
    let changed = cores.find(dir.path(), dir.path(), &builtin_systems());
    assert_ne!(changed, first);
    assert_eq!(changed.cores[0].user.unwrap().notice, "Others · Non-commercial");
}

#[test]
fn required_firmware_is_checked_before_launch() {
    let info = "database = \"Sony - PlayStation\"\nfirmware_count = 2\nfirmware0_path = \"SCPH1001.BIN\"\nfirmware0_opt = \"false\"\n\
                firmware1_path = \"extra.bin\"\nfirmware1_opt = \"true\"\ncorename = \"Mine\"";
    let dir = cores_dir(&[("mine_libretro.so", &library()), ("mine_libretro.info", info.as_bytes())]);
    let core = UserCores::default().find(dir.path(), dir.path(), &builtin_systems()).cores[0];
    let bios = tempfile::tempdir().unwrap();
    assert_eq!(core.check_bios(Path::new("Game.cue"), &["cue"], bios.path()).unwrap_err(), "Games played with Mine need scph1001.bin in the bios folder");
    std::fs::write(bios.path().join("scph1001.bin"), b"").unwrap();
    assert!(core.check_bios(Path::new("Game.cue"), &["cue"], bios.path()).is_ok());
}

/// What a core might declare.
fn options(default: &str) -> CoreOptions {
    serde_json::from_value(serde_json::json!({ "options": [
        { "key": "snes9x_overclock", "label": "Overclock", "values": [{ "value": "off" }, { "value": "on" }], "default": default },
    ]})).unwrap()
}

#[test]
fn options_are_learned_at_the_first_launch_and_again_when_the_core_is_replaced() {
    let (card, learned) = (cores_dir(&[("snes9x_libretro.so", &library()), ("snes9x_libretro.info", SNES9X.as_bytes())]), tempfile::tempdir().unwrap());
    let mut cores = UserCores::default();
    let find = |cores: &mut UserCores| cores.find(card.path(), learned.path(), &builtin_systems()).cores[0];
    let first = find(&mut cores);
    assert!(!first.user.unwrap().learned);
    assert!(settings::declarations(&playable(builtin_systems(), &[first])).iter().all(|d| d.core != Some("snes9x")));
    // Its first game: what it declares is kept, and the next look finds it with them.
    assert!(learn(first, learned.path(), &options("off")).unwrap());
    let learnt = find(&mut cores);
    assert!(learnt.user.unwrap().learned);
    assert_eq!(*core_options::list(learnt), options("off"));
    let decls = settings::declarations(&playable(builtin_systems(), &[learnt]));
    let overclock = decls.iter().find(|d| d.key == "snes9x_overclock").expect("a setting for its option");
    assert_eq!((overclock.core, overclock.section), (Some("snes9x"), "Snes9x"));
    // Kept, it isn't written again, nor is the core made again.
    assert!(!learn(learnt, learned.path(), &options("off")).unwrap());
    assert!(std::ptr::eq(find(&mut cores), learnt));
    // A core that declares otherwise is learned again.
    assert!(learn(learnt, learned.path(), &options("on")).unwrap());
    assert_eq!(core_options::list(find(&mut cores)).options[0].default, "on");
    // A replaced core's options are learned again: it runs with its own defaults until then.
    std::fs::write(card.path().join("snes9x_libretro.so"), [library(), vec![0; 8]].concat()).unwrap();
    let replaced = find(&mut cores);
    assert!(!replaced.user.unwrap().learned && core_options::list(replaced).options.is_empty());
    assert_eq!(replaced.user.unwrap().status(), "its options are learned when it next plays a game");
    // Specular's own cores have nothing to learn.
    let mgba = crate::library::cores(&builtin_systems())[0];
    assert!(!learn(mgba, learned.path(), &options("off")).unwrap());
}

#[test]
fn a_system_with_several_cores_has_a_core_setting_for_it_and_its_games() {
    let dir = cores_dir(&[("gambatte_libretro.so", &library()), ("gambatte_libretro.info", GAMBATTE.as_bytes())]);
    let found = UserCores::default().find(dir.path(), dir.path(), &builtin_systems());
    let systems = playable(builtin_systems(), &found.cores);
    let mut s = Settings::new(settings::declarations(&systems));
    let gb = systems.iter().find(|s| s.id == "gb").unwrap();
    let (game, system) = (Layer::Game { system: "gb".into(), game: "gb/Tetris".into() }, Layer::System("gb".into()));
    assert_eq!(settings::core_of(&s, gb, &game).unwrap().id(), "mgba"); // Specular's own, by default
    s.set("core.gb", &game, "gambatte".into()).unwrap();
    assert_eq!(settings::core_of(&s, gb, &game).unwrap().id(), "gambatte");
    assert_eq!(settings::core_of(&s, gb, &system).unwrap().id(), "mgba");
    // Only for its system's and its games' files.
    assert_eq!(s.set("core.gb", &Layer::Global, "gambatte".into()), Err(settings::SettingError::NotAtThisLayer));
    assert_eq!(s.set("core.gb", &Layer::System("gbc".into()), "gambatte".into()), Err(settings::SettingError::NotAtThisLayer));
    assert!(s.rows(&Layer::System("gba".into()), crate::settings::Visibility::User, None).iter().all(|r| !r.id.0.starts_with("core.")));
    assert!(s.rows(&system, crate::settings::Visibility::User, None).iter().any(|r| r.id.0 == "core.gb"));
    // A system with one core has nothing to choose.
    assert!(s.decl("core.gba").is_none());
    // A core chosen that is gone (its files removed) leaves the default.
    let mut s = Settings::new(settings::declarations(&builtin_systems()));
    s.set_layer(game.clone(), serde_json::from_str(r#"{"core.gb": "gambatte"}"#).unwrap());
    assert_eq!(settings::core_of(&s, &builtin_systems()[0], &game).unwrap().id(), "mgba");
}
