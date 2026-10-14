use super::*;

/// Tests that start a session take turns: only one game runs at a time.
static ONE_GAME: Mutex<()> = Mutex::new(());

/// The host build of mGBA (`tools/build-cores.sh host`); tests that need it are skipped without.
fn mgba() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../build/cores/host/mgba_libretro.so");
    p.exists().then_some(p)
}

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
        LaunchSpec { core, rom, save: save.clone(), autosave: None, system_dir: dir.path().into(), options: vec![], rom_by_path: false },
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
    let spec = |core: PathBuf| LaunchSpec { core, rom: dir.path().join("none.gba"), save: dir.path().join("x.srm"), autosave: None, system_dir: dir.path().into(), options: vec![], rom_by_path: false };
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
                               system_dir: dir.path().into(), options: vec![], rom_by_path: false };
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
        LaunchSpec { core, rom: rom_path, save: dir.path().join("loop.srm"), autosave: None, system_dir: dir.path().into(), options: vec![], rom_by_path: false },
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
