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
        LaunchSpec { core, rom, save: save.clone(), autosave: None, system_dir: dir.path().into(), options: vec![], rom_by_path: false, states_restore_save: false, record: None },
        Outputs { sound: Some(Box::new(Count(sound.clone()))), vsync: None, display_hz: 60.0, frames: frames.clone(),
                  on_frame: Box::new(move || *c.lock().unwrap() += 1) },
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
fn a_missing_core_or_rom_is_an_error() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let out = || Outputs { sound: None, vsync: None, display_hz: 60.0, frames: Arc::default(), on_frame: Box::new(|| {}) };
    let spec = |core: PathBuf| LaunchSpec { core, rom: dir.path().join("none.gba"), save: dir.path().join("x.srm"), autosave: None, system_dir: dir.path().into(), options: vec![], rom_by_path: false, states_restore_save: false, record: None };
    assert!(Session::start(spec(dir.path().join("none.so")), out()).is_err());
    if let Some(core) = mgba() { assert!(Session::start(spec(core), out()).err().unwrap().contains("none.gba")); }
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
                               system_dir: dir.path().into(), options: vec![], rom_by_path: false, states_restore_save: false, record: None };
    let out = || Outputs { sound: None, vsync: None, display_hz: 60.0, frames: Arc::default(), on_frame: Box::new(|| {}) };
    let session = Session::start(spec(), out()).unwrap();
    // mGBA makes the ENV_SAVE_RAM_WRITTEN call 15 frames (a quarter of a second) after the game's
    // last write: long before CHECK_INTERVAL.
    std::thread::sleep(Duration::from_millis(800));
    let srm = std::fs::read(&save).expect("the battery save written");
    assert_eq!((srm.len(), srm[0]), (32 << 10, 0x42)); // detected as SRAM
    assert!(!autosave.exists());
    session.quit().unwrap();
    let state = std::fs::read(&autosave).expect("the autosave written on quitting");
    assert!(!state.is_empty());
    // Started again, it resumes; an autosave the core can't load leaves it starting afresh.
    Session::start(spec(), out()).unwrap().quit().unwrap();
    std::fs::write(&autosave, b"not a state").unwrap();
    Session::start(spec(), out()).unwrap().quit().unwrap();
    assert_ne!(std::fs::read(&autosave).unwrap(), b"not a state"); // replaced on quitting
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
                                           options: vec![], rom_by_path: true, states_restore_save: false, record: Some((record_path.clone(), record.clone())) };
    let out = || Outputs { sound: None, vsync: None, display_hz: 60.0, frames: Arc::new(FrameSlot::default()), on_frame: Box::new(|| {}) };
    let write = |name: &str, data: &[u8]| { let p = dir.path().join(name); std::fs::write(&p, data).unwrap(); p };
    // mGBA is given the unzipped ROM's path, which has no extension: it tells a GBA ROM by its contents.
    let game = write("game.zip", &zip(&[("readme.txt", b"hello"), ("game.gba", &rom(&[0xeaff_fffe /* b . */]))]));
    let o = out();
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
    assert_eq!(Session::start(spec(none), out()).err().unwrap(), "the zip holds no game mGBA plays");
    let zipped = std::fs::read(&game).unwrap();
    let cut = write("cut.zip", &zipped[..zipped.len() - 30]); // the directory's end is gone
    assert_eq!(Session::start(spec(cut), out()).err().unwrap(), "the zip can't be read (not a zip)");
    // A byte of the ROM's compressed data, which ends where the directory starts.
    let mut damaged = zipped.clone();
    damaged[zipped.windows(4).position(|w| w == b"PK\x01\x02").unwrap() - 4] ^= 0xff;
    let damaged = write("damaged.zip", &damaged);
    assert!(Session::start(spec(damaged), out()).err().unwrap().starts_with("the zip is damaged"));
    assert!(!record_path.exists()); // nor is it left by a game that couldn't start
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
        LaunchSpec { core, rom: rom_path, save: dir.path().join("loop.srm"), autosave: None, system_dir: dir.path().into(), options: vec![], rom_by_path: false, states_restore_save: false, record: None },
        Outputs { sound: Some(Box::new(Count(first.clone()))), vsync: None, display_hz: 60.0, frames: Arc::default(), on_frame: Box::new(|| {}) },
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
                     options: vec![("nestopia_blargg_ntsc_filter".into(), "disabled".into())], rom_by_path: false, states_restore_save: true, record: None },
        Outputs { sound: Some(Box::new(Count(sound.clone()))), vsync: None, display_hz: 60.0, frames: frames.clone(), on_frame: Box::new(|| {}) },
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
                     options: vec![("swanstation_GPU_Renderer".into(), "Software".into())], rom_by_path: false, states_restore_save: true, record: None },
        Outputs { sound: None, vsync: None, display_hz: 60.0, frames: frames.clone(), on_frame: Box::new(|| {}) },
    ).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let f = frames.take().expect("a frame");
    assert!(f.rgba.chunks(4).any(|p| p[..3] != [0, 0, 0]), "OpenBIOS's shell drawn");
    session.quit().unwrap();
    assert_eq!(std::fs::metadata(&save).unwrap().len(), 128 << 10); // memory card 1
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
        options: options.iter().map(|&(k, v)| (k.into(), v.into())).collect(), rom_by_path: false, states_restore_save: true, record: None };
    let out = || Outputs { sound: None, vsync: None, display_hz: 60.0, frames: Arc::default(), on_frame: Box::new(|| {}) };
    let run = |load: Option<&Path>, save_to: Option<&Path>| {
        let session = Session::start(spec(), out()).unwrap();
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
fn a_state_carries_its_save_crc_after_its_zstd_frame() {
    let frame = zstd::bulk::compress(b"state", 1).unwrap();
    let file = with_save_crc(frame.clone(), Some(0x1234_5678));
    assert_eq!(save_crc_of(&file), Some(0x1234_5678));
    assert_eq!(zstd::decode_all(file.as_slice()).unwrap(), b"state"); // zstd skips it
    assert_eq!(save_crc_of(&frame), None);
    assert_eq!(with_save_crc(frame.clone(), None), frame);
}

