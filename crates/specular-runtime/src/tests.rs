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
        LaunchSpec { core, rom, save: save.clone(), system_dir: dir.path().into(), options: vec![] },
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
    session.quit().unwrap();
    assert_eq!(std::fs::metadata(&save).unwrap().len(), 128 << 10); // mGBA before detecting the save type
}

#[test]
fn a_missing_core_or_rom_is_an_error() {
    let _turn = ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let out = || Outputs { sound: None, vsync: None, display_hz: 60.0, frames: Arc::default(), on_frame: Box::new(|| {}) };
    let spec = |core: PathBuf| LaunchSpec { core, rom: dir.path().join("none.gba"), save: dir.path().join("x.srm"), system_dir: dir.path().into(), options: vec![] };
    assert!(Session::start(spec(dir.path().join("none.so")), out()).is_err());
    if let Some(core) = mgba() { assert!(Session::start(spec(core), out()).err().unwrap().contains("none.gba")); }
}
