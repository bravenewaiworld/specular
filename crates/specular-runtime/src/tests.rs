use super::*;

/// Tests that start a session (or use the core log) take turns: only one game runs at a time.
pub(crate) static ONE_GAME: Mutex<()> = Mutex::new(());

/// The host build of the core `name` (its library's name without `_libretro.so`, e.g. "mgba") in
/// `$SPECULAR_CORES` (`make test` builds them and sets it); tests that need it are skipped without.
fn core(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("SPECULAR_CORES")?).join(format!("{name}_libretro.so"));
    p.exists().then_some(p)
}

fn mgba() -> Option<PathBuf> { core("mgba") }

/// Counts the frames written. Its queue has no room, so no silence is added (it is half the
/// queue) and the rate is never adjusted: each game frame's sound is written as it is.
struct Count(Arc<Mutex<usize>>);
impl SoundOut for Count {
    fn rate(&self) -> u32 { 48_000 }
    fn write(&mut self, f: &[[i16; 2]]) -> io::Result<usize> { *self.0.lock().unwrap() += f.len(); Ok(f.len()) }
    fn queued(&mut self) -> usize { 0 }
    fn capacity(&self) -> usize { 0 }
}

/// Outputs that discard everything: no sound or rumble, a 60 Hz display without vsync, and
/// callbacks that do nothing. A test sets the fields it checks over these (`..outputs()`).
fn outputs() -> Outputs {
    Outputs { sound: None, vsync: None, display_hz: 60.0, frames: Arc::default(), on_frame: Box::new(|| {}), rumble: None, on_save: Box::new(|| {}), on_bios_halt: Box::new(|| {}),
              write_failed: Box::new(|_| {}) }
}

#[test]
fn runs_a_gba_game_with_frames_sound_saves_and_states() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = mgba() else { eprintln!("skipped: no host mGBA build"); return };
    let dir = tempfile::tempdir().unwrap();
    // A blank cartridge: mGBA takes a file as a GBA ROM if it starts with a branch instruction
    // and has the fixed byte 0x96 at 0xb2. This one branches to itself (`b .`), an endless loop,
    // which is enough for frames, sound and save memory to work.
    let rom = dir.path().join("blank.gba");
    let mut blank = vec![0u8; 1 << 16];
    blank[..4].copy_from_slice(&0xeaff_fffe_u32.to_le_bytes());
    blank[0xb2] = 0x96;
    std::fs::write(&rom, blank).unwrap();
    let save = dir.path().join("saves/blank.srm");
    let frames = Arc::new(FrameSlot::default());
    let (count, sound) = (Arc::new(Mutex::new(0usize)), Arc::new(Mutex::new(0usize)));
    let c = count.clone();
    let session = Session::start(
        LaunchSpec { core, rom, save: save.clone(), autosave: None, system_dir: dir.path().into(), options: vec![], option_list: None, rom_by_path: false, controller: None, states_restore_save: false, record: None },
        Outputs { sound: Some(Box::new(Count(sound.clone()))), frames: frames.clone(), on_frame: Box::new(move || *c.lock().unwrap() += 1), ..outputs() },
    ).unwrap();
    assert_eq!((session.info().width, session.info().height), (240, 160));
    assert!((session.info().fps - 59.7275).abs() < 0.01, "{}", session.info().fps);
    std::thread::sleep(Duration::from_millis(500));
    let f = frames.take().expect("a frame");
    assert_eq!((f.width, f.height, f.rgba.len()), (240, 160, 240 * 160 * 4));
    let n = *count.lock().unwrap();
    assert!((20..=40).contains(&n), "{n} frames in 0.5 s"); // paced at about 60 a second
    // mGBA makes 32768 samples a second: about 804 a frame at 48 kHz.
    let written = *sound.lock().unwrap();
    assert!(((n - 1) * 780..=(n + 1) * 830).contains(&written), "{written} sound frames for {n} frames");
    let state = dir.path().join("saves/blank.state1");
    session.save_state(&state).unwrap();
    session.load_state(&state).unwrap();
    // Written compressed; an uncompressed state, as earlier builds wrote, still loads.
    let written = std::fs::read(&state).unwrap();
    assert!(written.starts_with(&ZSTD_MAGIC));
    let raw = zstd::decode_all(written.as_slice()).unwrap();
    // Loose: this blank game's state shrinks far more than a real game's (about 11 times).
    assert!(written.len() * 4 < raw.len(), "{} bytes compressed from {}", written.len(), raw.len());
    std::fs::write(&state, raw).unwrap();
    session.load_state(&state).unwrap();
    session.quit().unwrap();
    assert_eq!(std::fs::metadata(&save).unwrap().len(), 128 << 10); // mGBA before detecting the save type
}

#[test]
fn the_frame_shown_last_is_kept_until_taken() {
    let slot = FrameSlot::default();
    let frame = |w| Frame { width: w, height: 1, rgba: vec![0; 4] };
    slot.publish(frame(1));
    slot.publish(frame(2)); // replaces 1, never taken
    let shown = slot.take().unwrap();
    assert_eq!(shown.width, 2);
    slot.give_back(shown);
    // The game draws into a spare frame (1), not into the one on screen.
    assert_eq!(slot.spare().width, 1);
    assert_eq!(slot.spare().width, 0);
    assert_eq!(slot.take_shown().map(|f| f.width), Some(2));
    assert!(slot.take_shown().is_none());
}

#[test]
fn a_missing_core_or_rom_is_an_error() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let spec = |core: PathBuf| LaunchSpec { core, rom: dir.path().join("none.gba"), save: dir.path().join("x.srm"), autosave: None, system_dir: dir.path().into(), options: vec![], option_list: None, rom_by_path: false, controller: None, states_restore_save: false, record: None };
    assert!(Session::start(spec(dir.path().join("none.so")), outputs()).is_err());
    if let Some(core) = mgba() { assert!(Session::start(spec(core), outputs()).err().unwrap().contains("none.gba")); }
}

/// A GBA ROM whose code (after the header) is `code`, ARM instructions. mGBA takes a file as a
/// GBA ROM if it starts with a branch instruction and has the fixed byte 0x96 at 0xb2.
fn rom(code: &[u32]) -> Vec<u8> {
    let mut rom = vec![0u8; 1 << 16];
    rom[..4].copy_from_slice(&0xea00_002e_u32.to_le_bytes()); // b 0xc0
    rom[0xb2] = 0x96;
    for (i, c) in code.iter().enumerate() { rom[0xc0 + i * 4..][..4].copy_from_slice(&c.to_le_bytes()); }
    rom
}

#[test]
fn the_battery_save_is_written_once_the_game_writes_it_and_the_game_resumes_from_its_autosave() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = mgba() else { eprintln!("skipped: no host mGBA build"); return };
    let dir = tempfile::tempdir().unwrap();
    let rom_path = dir.path().join("writes.gba");
    // Writes 0x42 to the start of the cartridge's SRAM, then loops.
    std::fs::write(&rom_path, rom(&[0xe3a0_040e /* mov r0, #0x0e000000 */, 0xe3a0_1042 /* mov r1, #0x42 */,
                                    0xe5c0_1000 /* strb r1, [r0] */, 0xeaff_fffe /* b . */])).unwrap();
    let (save, autosave) = (dir.path().join("writes.srm"), dir.path().join(".states/writes.autosave"));
    let spec = || LaunchSpec { core: core.clone(), rom: rom_path.clone(), save: save.clone(), autosave: Some(autosave.clone()),
                               system_dir: dir.path().into(), options: vec![], option_list: None, rom_by_path: false, controller: None, states_restore_save: false, record: None };
    let saves = Arc::new(Mutex::new(0usize));
    let out = || {
        let saves = saves.clone();
        Outputs { on_save: Box::new(move || *saves.lock().unwrap() += 1), ..outputs() }
    };
    let session = Session::start(spec(), out()).unwrap();
    // mGBA makes the ENV_SAVE_RAM_WRITTEN call 15 frames (a quarter of a second) after the game's
    // last write: long before CHECK_INTERVAL.
    std::thread::sleep(Duration::from_millis(800));
    let srm = std::fs::read(&save).expect("the battery save written");
    assert_eq!((srm.len(), srm[0]), (32 << 10, 0x42)); // detected as SRAM
    assert!(!autosave.exists());
    session.quit().unwrap();
    assert_eq!(*saves.lock().unwrap(), 1); // the game saved; writing it again on quitting isn't saving
    let state = std::fs::read(&autosave).expect("the autosave written on quitting");
    assert!(!state.is_empty());
    // Started again, it resumes; an autosave the core can't load leaves it starting afresh.
    Session::start(spec(), out()).unwrap().quit().unwrap();
    std::fs::write(&autosave, b"not a state").unwrap();
    Session::start(spec(), out()).unwrap().quit().unwrap();
    assert_ne!(std::fs::read(&autosave).unwrap(), b"not a state"); // replaced on quitting
}

#[test]
fn the_games_rumble_stops_while_it_is_paused_and_when_it_ends() {
    use specular_core::device::Motors;
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = mgba() else { eprintln!("skipped: no host mGBA build"); return };
    let dir = tempfile::tempdir().unwrap();
    // Drill Dozer's game code, for which mGBA gives the cartridge rumble (on GPIO pin 3), and code
    // that turns it on.
    let mut rumbles = rom(&[0xe3a0_0408 /* mov r0, #0x08000000 */, 0xe280_00c4 /* add r0, r0, #0xc4 (GPIO data) */,
                            0xe3a0_1008 /* mov r1, #8 */, 0xe1c0_10b2 /* strh r1, [r0, #2] (direction: pin 3 out) */,
                            0xe1c0_10b0 /* strh r1, [r0] (pin 3 on) */, 0xeaff_fffe /* b . */]);
    rumbles[0xac..0xb0].copy_from_slice(b"V49E");
    let rom_path = dir.path().join("rumbles.gba");
    std::fs::write(&rom_path, rumbles).unwrap();
    let line = RumbleLine::default();
    line.set_level(specular_core::settings::builtin::RUMBLE_MAX as u8);
    let session = Session::start(
        LaunchSpec { core, rom: rom_path, save: dir.path().join("rumbles.srm"), autosave: None, system_dir: dir.path().into(), options: vec![], option_list: None, rom_by_path: false, controller: None, states_restore_save: false, record: None },
        Outputs { rumble: Some(line.clone()), ..outputs() },
    ).unwrap();
    let full = Motors { strong: 0xffff, weak: 0xffff };
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(line.wanted(), full);
    session.pause(true);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(line.wanted(), Motors::STILL);
    session.pause(false);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(line.wanted(), full);
    session.quit().unwrap();
    assert_eq!(line.wanted(), Motors::STILL);
}

/// A zip of these files, deflated.
fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let (mut out, mut dir) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        let body = e.finish().unwrap();
        // From the method on, the local header and the directory entry share these fields.
        let common = [&8u16.to_le_bytes()[..], &[0; 4], &crc32fast::hash(data).to_le_bytes(), &(body.len() as u32).to_le_bytes(),
                      &(data.len() as u32).to_le_bytes(), &(name.len() as u16).to_le_bytes()].concat();
        dir.extend([&b"PK\x01\x02"[..], &[0; 6], &common, &[0; 12], &(out.len() as u32).to_le_bytes(), name.as_bytes()].concat());
        out.extend([&b"PK\x03\x04"[..], &[0; 4], &common, &[0; 2], name.as_bytes(), &body].concat());
    }
    let n = (files.len() as u16).to_le_bytes();
    let end = [&b"PK\x05\x06"[..], &[0; 4], &n, &n, &(dir.len() as u32).to_le_bytes(), &(out.len() as u32).to_le_bytes(), &[0; 2]].concat();
    [out, dir, end].concat()
}

#[test]
fn a_zipped_game_is_unzipped_into_memory_and_its_launch_recorded_while_it_runs() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = mgba() else { eprintln!("skipped: no host mGBA build"); return };
    let dir = tempfile::tempdir().unwrap();
    let record_path = dir.path().join("cache/launch.json");
    let game = specular_core::library::Game { system: "gba", path: "game.zip".into(), title: "Game".into(), sort_key: String::new() };
    let record = LaunchRecord::new("mGBA", &core, &game);
    let spec = |rom: PathBuf| LaunchSpec { core: core.clone(), rom, save: dir.path().join("game.srm"), autosave: None, system_dir: dir.path().into(),
                                           options: vec![], option_list: None, rom_by_path: true, controller: None, states_restore_save: false, record: Some((record_path.clone(), record.clone())) };
    let write = |name: &str, data: &[u8]| { let p = dir.path().join(name); std::fs::write(&p, data).unwrap(); p };
    // mGBA is given the unzipped ROM's path, which has no extension: it tells a GBA ROM by its contents.
    let game = write("game.zip", &zip(&[("readme.txt", b"hello"), ("game.gba", &rom(&[0xeaff_fffe /* b . */]))]));
    let o = outputs();
    let frames = o.frames.clone();
    let session = Session::start(spec(game.clone()), o).unwrap();
    assert_eq!((session.info().width, session.info().height), (240, 160));
    std::thread::sleep(Duration::from_millis(200));
    assert!(frames.take().is_some());
    assert_eq!(specular_core::settings::store::load_layer(&record_path).unwrap()["game"], "game.zip");
    session.quit().unwrap();
    assert!(dir.path().join("game.srm").exists());
    assert!(!record_path.exists()); // the game ended cleanly
    let none = write("none.zip", &zip(&[("readme.txt", b"hello")]));
    assert_eq!(Session::start(spec(none), outputs()).err().unwrap(), "the zip holds no game mGBA plays");
    let zipped = std::fs::read(&game).unwrap();
    let cut = write("cut.zip", &zipped[..zipped.len() - 30]); // the directory's end is gone
    assert_eq!(Session::start(spec(cut), outputs()).err().unwrap(), "the zip can't be read (not a zip)");
    // A byte of the ROM's compressed data, which ends where the directory starts.
    let mut damaged = zipped.clone();
    damaged[zipped.windows(4).position(|w| w == b"PK\x01\x02").unwrap() - 4] ^= 0xff;
    let damaged = write("damaged.zip", &damaged);
    assert!(Session::start(spec(damaged), outputs()).err().unwrap().starts_with("the zip is damaged"));
    assert!(!record_path.exists()); // nor is it left by a game that couldn't start
}

#[test]
fn abandoning_writes_nothing_and_saves_that_fail_are_reported() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = mgba() else { eprintln!("skipped: no host mGBA build"); return };
    let dir = tempfile::tempdir().unwrap();
    let rom_path = dir.path().join("game.gba");
    std::fs::write(&rom_path, rom(&[0xeaff_fffe /* b . */])).unwrap();
    // Saves go under a file, where no folder can be made.
    std::fs::write(dir.path().join("file"), b"").unwrap();
    let spec = |saves: &Path| LaunchSpec { core: core.clone(), rom: rom_path.clone(), save: saves.join("game.srm"), autosave: Some(saves.join("game.autosave")),
                                           system_dir: dir.path().into(), options: vec![], option_list: None, rom_by_path: false, controller: None, states_restore_save: false, record: None };
    let failed = Arc::new(Mutex::new(0));
    let out = || {
        let f = failed.clone();
        Outputs { write_failed: Box::new(move |_| *f.lock().unwrap() += 1), ..outputs() }
    };
    Session::start(spec(dir.path()), out()).unwrap().abandon();
    assert!(!dir.path().join("game.autosave").exists());
    assert!(Session::start(spec(&dir.path().join("file")), out()).unwrap().quit().is_err());
    assert_eq!(*failed.lock().unwrap(), 2); // the battery save and the autosave
}

struct Instant0;
impl Vsync for Instant0 { fn wait(&mut self) -> io::Result<()> { Ok(()) } }

#[test]
fn the_frame_delay_leaves_room_for_the_slowest_runs_and_the_ui() {
    let ms = |m: f64| Duration::from_secs_f64(m / 1000.0);
    let delay_after = |run: f64| {
        let mut p = Pacer::new(Some(Box::new(Instant0)), 60.0);
        for _ in 0..60 { p.ran(ms(run)); }
        p.delay.as_secs_f64() * 1000.0
    };
    assert!((delay_after(3.0) - (16.667 - 3.0 - 6.0)).abs() < 0.01);
    assert_eq!(delay_after(20.0), 0.0); // a slow core starts at once
    assert!((delay_after(0.1) - 8.333).abs() < 0.01); // never more than half a refresh
    // Without a vsync there is no delay to set.
    let mut p = Pacer::new(None, 60.0);
    for _ in 0..60 { p.ran(ms(1.0)); }
    assert_eq!(p.delay, Duration::ZERO);
}

#[test]
fn the_sound_moves_to_a_new_output_between_frames() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = mgba() else { eprintln!("skipped: no host mGBA build"); return };
    let dir = tempfile::tempdir().unwrap();
    let rom_path = dir.path().join("loop.gba");
    std::fs::write(&rom_path, rom(&[0xeaff_fffe /* b . */])).unwrap();
    let (first, second) = (Arc::new(Mutex::new(0usize)), Arc::new(Mutex::new(0usize)));
    let session = Session::start(
        LaunchSpec { core, rom: rom_path, save: dir.path().join("loop.srm"), autosave: None, system_dir: dir.path().into(), options: vec![], option_list: None, rom_by_path: false, controller: None, states_restore_save: false, record: None },
        Outputs { sound: Some(Box::new(Count(first.clone()))), ..outputs() },
    ).unwrap();
    std::thread::sleep(Duration::from_millis(200)); // frames reach the first output
    session.set_sound(Some(Box::new(Count(second.clone()))));
    std::thread::sleep(Duration::from_millis(50)); // the switch lands
    let played_first = *first.lock().unwrap();
    std::thread::sleep(Duration::from_millis(200)); // only the second output gets more
    assert!(played_first > 0);
    assert_eq!(*first.lock().unwrap(), played_first); // no longer written to
    assert!(*second.lock().unwrap() > 0);
    session.quit().unwrap();
}

/// An NES ROM (iNES, mapper 0, with a battery) that shows a background of stripes, plays a
/// square wave and loops; with `save`, it first writes 0x42 to the start of its battery memory.
fn nes_rom(save: bool) -> Vec<u8> {
    let mut code = vec![
        0x78, 0xd8, 0xa2, 0xff, 0x9a, // sei; cld; ldx #$ff; txs
        0x2c, 0x02, 0x20, 0x10, 0xfb, 0x2c, 0x02, 0x20, 0x10, 0xfb, // wait for two vblanks
        0xa9, 0x3f, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, // PPU address $3f00: the palette
        0xa9, 0x0f, 0x8d, 0x07, 0x20, 0xa9, 0x16, 0x8d, 0x07, 0x20, 0xa9, 0x2a, 0x8d, 0x07, 0x20,
        0xa9, 0x20, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, // PPU address $2000: the screen
        0xa9, 0x01, 0xa2, 0x00, 0xa0, 0x04, 0x8d, 0x07, 0x20, 0xe8, 0xd0, 0xfa, 0x88, 0xd0, 0xf7, // 1024 × tile 1
        0xa9, 0x00, 0x8d, 0x05, 0x20, 0x8d, 0x05, 0x20, // scroll to 0, 0
        0xa9, 0x1e, 0x8d, 0x01, 0x20, // show the background and sprites
        0xa9, 0x01, 0x8d, 0x15, 0x40, 0xa9, 0xbf, 0x8d, 0x00, 0x40, 0xa9, 0xfd, 0x8d, 0x02, 0x40, 0xa9, 0x00, 0x8d, 0x03, 0x40, // a tone
    ];
    if save { code.extend([0xa9, 0x42, 0x8d, 0x00, 0x60]); } // lda #$42; sta $6000
    let lp = 0xc000 + code.len() as u16;
    code.extend([0x4c, lp as u8, (lp >> 8) as u8, 0x40]); // jmp to itself; rti
    let rti = lp + 3;
    let mut prg = vec![0u8; 16 << 10];
    prg[..code.len()].copy_from_slice(&code);
    let [rl, rh] = rti.to_le_bytes();
    prg[0x3ffa..].copy_from_slice(&[rl, rh, 0x00, 0xc0, rl, rh]); // NMI, reset, IRQ
    let mut chr = vec![0u8; 8 << 10];
    chr[16..24].fill(0x55); // tile 1: stripes
    let mut rom = b"NES\x1a\x01\x01\x02\0\0\0\0\0\0\0\0\0".to_vec(); // 16 KiB PRG, 8 KiB CHR, battery
    rom.extend(prg);
    rom.extend(chr);
    rom
}

#[test]
fn runs_an_nes_game_with_frames_sound_and_its_battery_save() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = core("nestopia") else { eprintln!("skipped: no host Nestopia build"); return };
    let dir = tempfile::tempdir().unwrap();
    let rom = dir.path().join("test.nes");
    std::fs::write(&rom, nes_rom(true)).unwrap();
    let save = dir.path().join("test.srm");
    let frames = Arc::new(FrameSlot::default());
    let sound = Arc::new(Mutex::new(0usize));
    let session = Session::start(
        LaunchSpec { core, rom, save: save.clone(), autosave: None, system_dir: dir.path().into(),
                     options: vec![("nestopia_blargg_ntsc_filter".into(), "disabled".into())], option_list: None, rom_by_path: false, controller: None, states_restore_save: true, record: None },
        Outputs { sound: Some(Box::new(Count(sound.clone()))), frames: frames.clone(), ..outputs() },
    ).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let f = frames.take().expect("a frame");
    assert_eq!((f.width, f.height), (256, 224)); // 8 lines of overscan hidden at the top and bottom
    assert!(f.rgba.chunks(4).any(|p| p[..3] != [0, 0, 0]), "the stripes drawn");
    assert!(*sound.lock().unwrap() > 0);
    session.quit().unwrap();
    let srm = std::fs::read(&save).expect("the battery save written on quitting");
    assert_eq!((srm.len(), srm[0]), (8 << 10, 0x42));
}

/// A PS-X EXE that loops. SwanStation boots it only through a real BIOS: with its built-in
/// OpenBIOS (all a test has), OpenBIOS runs its own shell instead, which is enough for frames,
/// states and the memory card.
fn ps1_exe() -> Vec<u8> {
    let mut exe = vec![0u8; 4096];
    exe[..8].copy_from_slice(b"PS-X EXE");
    // The initial pc, the load address, the code's size and the initial stack.
    for (at, v) in [(0x10, 0x8001_0000u32), (0x18, 0x8001_0000), (0x1c, 2048), (0x30, 0x801f_fff0)] {
        exe[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    exe[2048..2052].copy_from_slice(&0x0800_4000u32.to_le_bytes()); // j 0x80010000; then a nop
    exe
}

/// The pad Specular gives SwanStation's player 1 (`CoreDecl::controller`: a DualShock).
fn swanstation_pad() -> Option<std::ffi::c_uint> {
    use specular_core::library::{builtin_systems, cores};
    cores(&builtin_systems()).into_iter().find(|c| c.id() == "swanstation").unwrap().controller
}

#[test]
fn runs_a_ps1_game_without_a_bios() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = core("swanstation") else { eprintln!("skipped: no host SwanStation build"); return };
    let dir = tempfile::tempdir().unwrap();
    let rom = dir.path().join("loop.exe");
    std::fs::write(&rom, ps1_exe()).unwrap();
    let save = dir.path().join("loop.srm");
    let frames = Arc::new(FrameSlot::default());
    let session = Session::start(
        LaunchSpec { core, rom, save: save.clone(), autosave: None, system_dir: dir.path().join("bios"),
                     options: vec![("swanstation_GPU_Renderer".into(), "Software".into())], option_list: None, rom_by_path: false,
                     controller: swanstation_pad(), states_restore_save: true, record: None },
        Outputs { frames: frames.clone(), ..outputs() },
    ).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let f = frames.take().expect("a frame");
    assert!(f.rgba.chunks(4).any(|p| p[..3] != [0, 0, 0]), "OpenBIOS's shell drawn");
    session.quit().unwrap();
    assert_eq!(std::fs::metadata(&save).unwrap().len(), 128 << 10); // memory card 1
}

/// Raw 2352-byte Mode 2 (Form 1) sectors holding `data`'s 2048-byte sectors, as a `.bin` has them.
fn mode2_sectors(data: &[u8]) -> Vec<u8> {
    let bcd = |n: usize| (n / 10 * 16 + n % 10) as u8;
    data.chunks(2048).enumerate().flat_map(|(lba, d)| {
        let mut s = vec![0u8; 2352];
        s[1..11].fill(0xff); // the sync pattern: 00, ten ff, 00
        let at = lba + 150; // two seconds of lead-in
        s[12..16].copy_from_slice(&[bcd(at / 75 / 60), bcd(at / 75 % 60), bcd(at % 75), 2]);
        s[24..24 + d.len()].copy_from_slice(d);
        s
    }).collect()
}

/// A PS1 disc (`.bin`) whose only file is `PSX.EXE;1`, holding `exe`: an ISO 9660 volume with
/// just what OpenBIOS reads to boot it (its volume descriptor, path table and root directory).
fn ps1_disc_booting(exe: &[u8]) -> Vec<u8> {
    const ROOT: u32 = 19;
    const EXE: u32 = 20;
    let mut iso = vec![0u8; 2048 * (EXE as usize + exe.len().div_ceil(2048) + 8)]; // and a few spare sectors after
    let both_ends = |n: u32| [n.to_le_bytes(), n.to_be_bytes()].concat();
    // A directory record: its extent's first sector and size, whether it is a folder, its name.
    let record = |lba: u32, size: u32, folder: bool, name: &[u8]| {
        let mut r = vec![0u8; 33 + name.len() + (name.len() + 1) % 2];
        r[0] = r.len() as u8;
        r[2..10].copy_from_slice(&both_ends(lba));
        r[10..18].copy_from_slice(&both_ends(size));
        r[25] = if folder { 2 } else { 0 };
        r[28] = 1; // volume sequence number
        r[32] = name.len() as u8;
        r[33..33 + name.len()].copy_from_slice(name);
        r
    };
    let sector = |n: u32| 2048 * n as usize;
    let pvd = sector(16);
    iso[pvd..pvd + 7].copy_from_slice(b"\x01CD001\x01");
    let sectors = (iso.len() / 2048) as u32;
    iso[pvd + 80..pvd + 88].copy_from_slice(&both_ends(sectors)); // the volume's size
    iso[pvd + 128..pvd + 132].copy_from_slice(&[0, 8, 8, 0]); // 2048-byte blocks, both ends
    iso[pvd + 132..pvd + 140].copy_from_slice(&both_ends(10)); // the path table's size
    iso[pvd + 140..pvd + 144].copy_from_slice(&18u32.to_le_bytes()); // its sector
    iso[pvd + 156..pvd + 190].copy_from_slice(&record(ROOT, 2048, true, b"\0")); // the root folder's record
    iso[sector(17)..sector(17) + 7].copy_from_slice(b"\xffCD001\x01"); // the descriptors' end
    // The path table: the root folder only (its name's length 1, its sector, its parent 1, its
    // name 0, a pad byte).
    iso[sector(18)..sector(18) + 10].copy_from_slice(&[&[1, 0][..], &ROOT.to_le_bytes(), &[1, 0, 0, 0]].concat());
    let root: Vec<u8> = [record(ROOT, 2048, true, b"\0"), record(ROOT, 2048, true, b"\x01"), record(EXE, exe.len() as u32, false, b"PSX.EXE;1")].concat();
    iso[sector(ROOT)..sector(ROOT) + root.len()].copy_from_slice(&root);
    iso[sector(EXE)..sector(EXE) + exe.len()].copy_from_slice(exe);
    mode2_sectors(&iso)
}

/// A PS-X EXE that calls the BIOS's A0 function BFh, which Sony's BIOS leaves unused and OpenBIOS
/// doesn't implement, then loops.
fn ps1_exe_calling_a_missing_bios_function() -> Vec<u8> {
    let mut exe = ps1_exe();
    let code: [u32; 5] = [
        0x240a_00a0, // addiu $t2, $zero, 0xa0
        0x0140_f809, // jalr $t2: the BIOS's A0 functions, at 0xa0, take the function in $t1
        0x2409_00bf, // addiu $t1, $zero, 0xbf (in the delay slot): the function
        0x0800_4003, // j 0x8001000c: to itself (`ps1_exe` loads the code at 0x80010000)
        0,           // nop
    ];
    for (i, w) in code.iter().enumerate() { exe[2048 + 4 * i..2048 + 4 * i + 4].copy_from_slice(&w.to_le_bytes()); }
    exe
}

#[test]
fn a_ps1_game_that_openbios_halts_says_so_and_loses_its_autosave() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = core("swanstation") else { eprintln!("skipped: no host SwanStation build"); return };
    let dir = tempfile::tempdir().unwrap();
    let rom = dir.path().join("Halts.bin");
    std::fs::write(&rom, ps1_disc_booting(&ps1_exe_calling_a_missing_bios_function())).unwrap();
    // An autosave from an earlier play (not a state SwanStation can load: the game starts afresh).
    let autosave = dir.path().join("Halts.autosave");
    std::fs::write(&autosave, b"made on OpenBIOS").unwrap();
    let (halted, halt) = mpsc::channel();
    let session = Session::start(
        LaunchSpec { core, rom, save: dir.path().join("Halts.srm"), autosave: Some(autosave.clone()), system_dir: dir.path().join("bios"),
                     options: vec![("swanstation_GPU_Renderer".into(), "Software".into())], option_list: None, rom_by_path: false, controller: None, states_restore_save: true, record: None },
        Outputs { on_bios_halt: Box::new(move || { let _ = halted.send(()); }), ..outputs() },
    ).unwrap();
    halt.recv_timeout(Duration::from_secs(20)).expect("OpenBIOS halted the game");
    session.quit().unwrap();
    assert!(!autosave.exists(), "the autosave removed, and not written on quitting");
}

/// For a core whose states restore its save memory: a state made with one battery save, loaded
/// over a newer one, puts the old one back in the core, and the newer one is kept as
/// `.newer.srm` rather than overwritten.
fn a_newer_battery_save_is_kept(core: PathBuf, rom_name: &str, rom: Vec<u8>, options: &[(&str, &str)]) {
    let dir = tempfile::tempdir().unwrap();
    let rom_path = dir.path().join(rom_name);
    std::fs::write(&rom_path, rom).unwrap();
    let (save, state, newer) = (dir.path().join("game.srm"), dir.path().join("game.state1"), dir.path().join("game.newer.srm"));
    let spec = || LaunchSpec { core: core.clone(), rom: rom_path.clone(), save: save.clone(), autosave: None, system_dir: dir.path().into(),
        options: options.iter().map(|&(k, v)| (k.into(), v.into())).collect(), option_list: None, rom_by_path: false, controller: None, states_restore_save: true, record: None };
    let run = |load: Option<&Path>, save_to: Option<&Path>| {
        let session = Session::start(spec(), outputs()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        if let Some(p) = load { session.load_state(p).unwrap(); }
        if let Some(p) = save_to { session.save_state(p).unwrap(); }
        session.quit().unwrap();
    };
    // A first run writes the save, which gives the size of the game's save memory; the test's
    // saves are made that size.
    run(None, None);
    let size = std::fs::metadata(&save).unwrap().len() as usize;
    let (old, new) = (vec![0x11u8; size], vec![0x22u8; size]);
    std::fs::write(&save, &old).unwrap();
    run(None, Some(&state));
    assert_eq!(std::fs::read(&save).unwrap(), old);
    // A newer save, then the old state loaded: the state brings the old save back.
    std::fs::write(&save, &new).unwrap();
    run(Some(&state), None);
    assert_eq!(std::fs::read(&save).unwrap(), old, "the state restored the save memory");
    assert_eq!(std::fs::read(&newer).unwrap(), new, "the newer save kept");
    // Loading the state again over the save it was made with keeps nothing.
    std::fs::remove_file(&newer).unwrap();
    run(Some(&state), None);
    assert!(!newer.exists());
}

#[test]
fn nestopia_states_restore_the_battery_save_and_a_newer_one_is_kept() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = core("nestopia") else { eprintln!("skipped: no host Nestopia build"); return };
    a_newer_battery_save_is_kept(core, "loop.nes", nes_rom(false), &[("nestopia_blargg_ntsc_filter", "disabled")]);
}

#[test]
fn swanstation_states_restore_the_memory_card_and_a_newer_one_is_kept() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = core("swanstation") else { eprintln!("skipped: no host SwanStation build"); return };
    a_newer_battery_save_is_kept(core, "loop.exe", ps1_exe(), &[("swanstation_GPU_Renderer", "Software")]);
}

#[test]
fn a_save_on_the_card_is_kept_unless_the_state_was_made_with_it_or_it_is_being_written() {
    let dir = tempfile::tempdir().unwrap();
    let (save, newer) = (dir.path().join("g.srm"), dir.path().join("g.newer.srm"));
    let state = |save_crc| LoadedState { path: dir.path().join("g.state1"), save_crc };
    // No save on the card.
    keep_newer_save(&save, b"new", &state(None)).unwrap();
    assert!(!newer.exists());
    std::fs::write(&save, b"on card").unwrap();
    keep_newer_save(&save, b"on card", &state(None)).unwrap(); // the same as written
    keep_newer_save(&save, b"new", &state(Some(crc32fast::hash(b"on card")))).unwrap(); // the state's
    assert!(!newer.exists());
    keep_newer_save(&save, b"new", &state(None)).unwrap(); // unknown: kept
    assert_eq!(std::fs::read(&newer).unwrap(), b"on card");
    assert!(!save.exists());
}

#[test]
fn a_state_carries_its_save_crc_and_disc_after_its_zstd_frame() {
    let frame = zstd::bulk::compress(b"state", 1).unwrap();
    let tags = StateTags { save_crc: Some(0x1234_5678), disc: Some(Disc { index: 1, path: "/roms/ps1/Game (Disc 2).bin".into() }) };
    let file = with_tags(frame.clone(), &tags);
    assert_eq!(tags_of(&file), tags);
    assert_eq!(zstd::decode_all(file.as_slice()).unwrap(), b"state"); // zstd skips them
    assert_eq!(tags_of(&frame), StateTags::default());
    assert_eq!(with_tags(frame.clone(), &StateTags::default()), frame);
    // As earlier builds wrote them: the crc alone, and a tag not known.
    let crc_only = [&frame[..], &SKIPPABLE, &8u32.to_le_bytes(), b"SRAM", &7u32.to_le_bytes(), &SKIPPABLE, &5u32.to_le_bytes(), b"NEW?x"].concat();
    assert_eq!(tags_of(&crc_only), StateTags { save_crc: Some(7), disc: None });
    assert_eq!(tags_of(&crc_only[..crc_only.len() - 3]).save_crc, Some(7)); // cut short
}


#[test]
fn the_option_lists_are_what_the_cores_built_declare() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/cores");
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".json") else { continue };
        let Some(core) = core(id) else { eprintln!("skipped {id}: no host build"); continue };
        let listed: CoreOptions = serde_json::from_str(&std::fs::read_to_string(entry.path()).unwrap()).unwrap();
        assert!(listed.options.len() > 10, "{id}");
        assert_eq!(declared_options(&core).unwrap(), listed, "data/cores/{id}.json is out of date: `make cores` writes it again");
    }
}

#[test]
fn options_changed_mid_game_reach_the_core() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = mgba() else { eprintln!("skipped: no host mGBA build"); return };
    let dir = tempfile::tempdir().unwrap();
    let rom_path = dir.path().join("red.gba");
    // Makes the backdrop pure red (palette entry 0 = 0x001f), shows it (DISPCNT = 0), and loops.
    std::fs::write(&rom_path, rom(&[0xe3a0_0405 /* mov r0, #0x05000000 */, 0xe3a0_101f /* mov r1, #0x1f */, 0xe1c0_10b0 /* strh r1, [r0] */,
                                    0xe3a0_0404 /* mov r0, #0x04000000 */, 0xe3a0_1000 /* mov r1, #0 */, 0xe1c0_10b0 /* strh r1, [r0] */,
                                    0xeaff_fffe /* b . */])).unwrap();
    let frames = Arc::new(FrameSlot::default());
    let list: &'static CoreOptions = Box::leak(Box::new(declared_options(&core).unwrap()));
    let session = Session::start(
        LaunchSpec { core, rom: rom_path, save: dir.path().join("red.srm"), autosave: None, system_dir: dir.path().into(),
                     options: vec![], option_list: Some(list), rom_by_path: false, controller: None, states_restore_save: false, record: None },
        Outputs { frames: frames.clone(), ..outputs() },
    ).unwrap();
    // The colour of the middle of the newest frame, after a few more.
    let colour = || {
        std::thread::sleep(Duration::from_millis(300));
        let f = frames.take().expect("a frame");
        let i = ((f.height / 2 * f.width + f.width / 2) * 4) as usize;
        let c = [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2]];
        frames.give_back(f);
        c
    };
    assert_eq!(colour(), [255, 0, 0]);
    // mGBA's colour correction, which it reads again when told its options changed, dulls it.
    session.set_options(vec![("mgba_color_correction".into(), "GBA".into())]);
    let corrected = colour();
    assert_ne!(corrected, [255, 0, 0]);
    assert!(corrected[0] > corrected[1], "{corrected:?}");
    // Left out again, it goes back to the core's default.
    session.set_options(vec![]);
    assert_eq!(colour(), [255, 0, 0]);
    session.quit().unwrap();
}

#[test]
fn values_given_that_a_core_doesnt_take_keep_its_default() {
    let value = |v: &str| specular_core::settings::core_options::OptionValue { value: v.into(), label: None };
    let declared = CoreOptions { categories: vec![], options: vec![specular_core::settings::core_options::CoreOption {
        key: "a".into(), label: "A".into(), info: None, category: None, values: vec![value("x"), value("y")], default: "x".into() }] };
    let mut o = options::Options::new(vec![("a".into(), "z".into()), ("b".into(), "1".into())]);
    o.declare(declared);
    assert_eq!(o.get("a").unwrap().to_str().unwrap(), "x");
    assert_eq!(o.refused(), ["a = z: not one of its values; it keeps \"x\"", "b = 1: it has no such option"]);
    assert!(!o.updated);
    assert_eq!(o.give(vec![("a".into(), "y".into()), ("b".into(), "1".into())]), ["option a: x → y"]);
    assert!(std::mem::take(&mut o.updated));
    assert!(o.give(vec![("a".into(), "y".into())]).is_empty());
    assert!(!o.updated);
}

#[test]
fn a_core_added_to_the_card_plays_and_its_options_are_learned_from_its_first_game() {
    use specular_core::library::{builtin_systems, user_cores::{self, UserCores}};
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mgba) = mgba() else { eprintln!("skipped: no host mGBA build"); return };
    // mGBA under another name, with the .info the libretro buildbot ships with it.
    let (card, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::copy(&mgba, card.path().join("mine_libretro.so")).unwrap();
    std::fs::write(card.path().join("mine_libretro.info"), include_str!("../../specular-core/src/library/user_cores/mgba_libretro.info")).unwrap();
    let mut cores = UserCores::default();
    let found = cores.find(card.path(), cache.path(), &builtin_systems());
    assert!(found.turned_away.is_empty(), "{:?}", found.turned_away);
    let core = found.cores[0];
    let rom_path = card.path().join("blank.gba");
    std::fs::write(&rom_path, rom(&[0xeaff_fffe /* b . */])).unwrap();
    let session = Session::start(
        LaunchSpec { core: core.user.unwrap().path.clone(), rom: rom_path, save: card.path().join("blank.srm"), autosave: None, system_dir: card.path().into(),
                     options: vec![], option_list: None, rom_by_path: false, controller: None, states_restore_save: core.states_restore_save, record: None },
        outputs(),
    ).unwrap();
    let declared = session.info().options.clone();
    session.quit().unwrap();
    assert_eq!(declared, declared_options(&mgba).unwrap());
    assert!(user_cores::learn(core, cache.path(), &declared).unwrap());
    let learnt = cores.find(card.path(), cache.path(), &builtin_systems()).cores[0];
    assert_eq!(*specular_core::settings::core_options::list(learnt), declared);
    assert!(!user_cores::learn(learnt, cache.path(), &declared).unwrap());
}

/// A PS1 disc image (`.bin`: raw 2352-byte Mode 2 sectors) that is only an ISO 9660 volume
/// descriptor: enough for SwanStation to open it, not to boot it.
fn ps1_disc() -> Vec<u8> {
    let mut iso = vec![0u8; 2048 * 32];
    iso[2048 * 16..2048 * 16 + 6].copy_from_slice(b"\x01CD001");
    mode2_sectors(&iso)
}

#[test]
fn a_ps1_game_on_two_discs_changes_disc_and_resumes_on_the_one_it_was_on() {
    use specular_core::app::Discs;
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = core("swanstation") else { eprintln!("skipped: no host SwanStation build"); return };
    let dir = tempfile::tempdir().unwrap();
    for n in [1, 2] { std::fs::write(dir.path().join(format!("Game (Disc {n}).bin")), ps1_disc()).unwrap(); }
    let rom = dir.path().join("Game.m3u");
    std::fs::write(&rom, "Game (Disc 1).bin\nGame (Disc 2).bin\n").unwrap();
    let autosave = dir.path().join(".states/Game.autosave");
    let spec = || LaunchSpec { core: core.clone(), rom: rom.clone(), save: dir.path().join("Game.srm"), autosave: Some(autosave.clone()),
        system_dir: dir.path().join("bios"), options: vec![("swanstation_GPU_Renderer".into(), "Software".into())], option_list: None,
        rom_by_path: false, controller: swanstation_pad(), states_restore_save: true, record: None };
    let on = |disc| Discs { count: 2, current: Some(disc) };
    let session = Session::start(spec(), outputs()).unwrap();
    assert_eq!(session.discs(), on(0));
    std::thread::sleep(Duration::from_millis(200));
    session.change_disc(1).unwrap();
    assert_eq!(session.discs(), on(1));
    assert_eq!(session.change_disc(2).unwrap_err().to_string(), "the game has no disc 3");
    assert_eq!(session.discs(), on(1));
    std::thread::sleep(Duration::from_millis(200));
    session.quit().unwrap();
    // The autosave records the disc, and the game resumes on it.
    let disc = tags_of(&std::fs::read(&autosave).unwrap()).disc.expect("the disc recorded");
    assert_eq!((disc.index, disc.path), (1, dir.path().join("Game (Disc 2).bin").to_string_lossy().into_owned()));
    let session = Session::start(spec(), outputs()).unwrap();
    assert_eq!(session.discs(), on(1));
    // SwanStation's own states keep the disc: one made on the second, without Specular's record
    // of it, loaded on the first, puts the second back.
    let state = dir.path().join(".states/Game.state1");
    session.save_state(&state).unwrap();
    let file = std::fs::read(&state).unwrap();
    std::fs::write(&state, &file[..zstd::zstd_safe::find_frame_compressed_size(&file).unwrap()]).unwrap();
    session.change_disc(0).unwrap();
    session.load_state(&state).unwrap();
    assert_eq!(session.discs(), on(1));
    session.change_disc(0).unwrap();
    assert_eq!(session.discs(), on(0));
    session.quit().unwrap();
    assert_eq!(tags_of(&std::fs::read(&autosave).unwrap()).disc.map(|d| d.index), Some(0));
}

/// A Famicom Disk System BIOS of our own (Nintendo's is copyrighted): it sets the backdrop red,
/// shows it and loops. 8 KiB, mapped at $e000.
fn fds_bios() -> Vec<u8> {
    let mut code = vec![
        0x78, 0xd8, 0xa2, 0xff, 0x9a, // sei; cld; ldx #$ff; txs
        0x2c, 0x02, 0x20, 0x10, 0xfb, 0x2c, 0x02, 0x20, 0x10, 0xfb, // wait for two vblanks
        0xa9, 0x3f, 0x8d, 0x06, 0x20, 0xa9, 0x00, 0x8d, 0x06, 0x20, // PPU address $3f00: the backdrop
        0xa9, 0x16, 0x8d, 0x07, 0x20, // red
        0xa9, 0x00, 0x8d, 0x05, 0x20, 0x8d, 0x05, 0x20, // scroll to 0, 0
        0xa9, 0x0a, 0x8d, 0x01, 0x20, // show the background
    ];
    let lp = 0xe000 + code.len() as u16;
    code.extend([0x4c, lp as u8, (lp >> 8) as u8, 0x40]); // jmp to itself; rti
    let [rl, rh] = (lp + 3).to_le_bytes();
    let mut bios = vec![0u8; 8 << 10];
    bios[..code.len()].copy_from_slice(&code);
    bios[0x1ffa..].copy_from_slice(&[rl, rh, 0x00, 0xe0, rl, rh]); // NMI, reset, IRQ
    bios
}

#[test]
fn a_zipped_famicom_disk_system_game_is_given_to_nestopia_by_its_extension() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(core) = core("nestopia") else { eprintln!("skipped: no host Nestopia build"); return };
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("bios")).unwrap();
    std::fs::write(dir.path().join("bios/disksys.rom"), fds_bios()).unwrap();
    // fwNES's format: a 16-byte header, then one side of 65500 bytes, blank.
    let disk = [&b"FDS\x1a\x01"[..], &[0; 11], &vec![0; 65500]].concat();
    let rom = dir.path().join("Zelda.zip");
    std::fs::write(&rom, zip(&[("readme.txt", b"hello"), ("Zelda (Japan).FDS", &disk)])).unwrap();
    // A folder of links left by a process that ended (no pid reaches pid_max) is removed.
    let pid_max = std::fs::read_to_string("/proc/sys/kernel/pid_max").unwrap();
    let (links, ours) = (unzip::runtime_dir(), unzip::runtime_dir().join(std::process::id().to_string()));
    let left = links.join(pid_max.trim());
    std::fs::create_dir_all(&left).unwrap();
    std::os::unix::fs::symlink("/proc/self/fd/3", left.join("Old.fds")).unwrap();
    let frames = Arc::new(FrameSlot::default());
    let session = Session::start(
        LaunchSpec { core, rom, save: dir.path().join("Zelda.srm"), autosave: None, system_dir: dir.path().join("bios"),
                     options: vec![("nestopia_blargg_ntsc_filter".into(), "disabled".into())], option_list: None, rom_by_path: false, controller: None, states_restore_save: true, record: None },
        Outputs { frames: frames.clone(), ..outputs() },
    ).unwrap();
    assert!(!left.exists());
    assert!(ours.join("Zelda.fds").is_symlink());
    std::thread::sleep(Duration::from_millis(300));
    let f = frames.take().expect("a frame");
    assert!(f.rgba.chunks(4).all(|p| p[0] > 100 && p[1] < 100), "our BIOS's red backdrop");
    session.quit().unwrap();
    assert!(!ours.exists());
}
