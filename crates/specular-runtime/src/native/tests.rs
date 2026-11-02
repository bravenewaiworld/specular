//! The protocol against `native/mock_core.c`, built with the system's C compiler.

use std::path::PathBuf;
use std::sync::mpsc;

use super::*;

/// The mock core, built once per test run.
fn mock_core() -> PathBuf {
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| {
        let native = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("native");
        let out = std::env::temp_dir().join(format!("specular-mock-core-{}", std::process::id()));
        let status = Command::new(std::env::var("CC").unwrap_or("cc".into()))
            .args(["-std=gnu11", "-Wall", "-Werror", "-O1", "-pthread", "-o"]).arg(&out)
            .arg(native.join("mock_core.c")).arg(native.join("spn_client.c"))
            .status().expect("a C compiler");
        assert!(status.success());
        out
    }).clone()
}

struct Harness { dir: tempfile::TempDir, frames: Arc<FrameSlot>, saved: mpsc::Receiver<()>, stopped: mpsc::Receiver<String>, played: Arc<Mutex<Vec<[i16; 2]>>> }

struct Out(Arc<Mutex<Vec<[i16; 2]>>>);
impl SoundOut for Out {
    fn rate(&self) -> u32 { 32_000 }
    fn write(&mut self, f: &[[i16; 2]]) -> io::Result<usize> { self.0.lock().unwrap().extend_from_slice(f); Ok(f.len()) }
    fn queued(&mut self) -> usize { 0 }
    fn capacity(&self) -> usize { 4096 }
}

fn start(env: Option<(&str, &str)>) -> (NativeSession, Harness) {
    let dir = tempfile::tempdir().unwrap();
    let frames = Arc::new(FrameSlot::default());
    let (saved_tx, saved) = mpsc::channel();
    let (stopped_tx, stopped) = mpsc::channel();
    let played = Arc::new(Mutex::new(Vec::new()));
    // The mock reads MOCK_FAIL from its environment: given through `env`, a wrapper's.
    let (program, args) = match env {
        Some((k, v)) => (PathBuf::from("/usr/bin/env"), vec![format!("{k}={v}").into(), mock_core().into()]),
        None => (mock_core(), vec![]),
    };
    let launch = NativeLaunch {
        program, args, package: dir.path().join("game.pkg"), saves: dir.path().join("saves"), cache: dir.path().join("cache"),
        clock: 123, options: vec![],
    };
    let saved_tx = Mutex::new(saved_tx);
    let stopped_tx = Mutex::new(stopped_tx);
    let out = NativeOutputs {
        sound: Some(Box::new(Out(played.clone()))), frames: frames.clone(),
        on_save: Box::new(move || { let _ = saved_tx.lock().unwrap().send(()); }),
        on_rumble: Box::new(|_| {}),
        on_stop: Box::new(move |why| { let _ = stopped_tx.lock().unwrap().send(why); }),
    };
    let session = NativeSession::start(launch, out).unwrap();
    (session, Harness { dir, frames, saved, stopped, played })
}

#[test]
fn the_layout_matches_the_header() {
    // sizeof(SpnShared) by the C compiler, against Rust's.
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("size.c");
    std::fs::write(&src, "#include \"specular_native.h\"\n#include <stdio.h>\n#include <stddef.h>\nint main(void) { printf(\"%zu %zu %zu %zu\", sizeof(SpnShared), offsetof(SpnShared, sound), offsetof(SpnShared, frames), offsetof(SpnShared, frame)); return 0; }\n").unwrap();
    let exe = dir.path().join("size");
    let native = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("native");
    assert!(Command::new("cc").arg("-I").arg(&native).arg(&src).arg("-o").arg(&exe).status().unwrap().success());
    let printed = String::from_utf8(Command::new(&exe).output().unwrap().stdout).unwrap();
    let ptr = std::ptr::null::<Shared>();
    let offset = |f: *const u8| f as usize - ptr as usize;
    let rust = unsafe { format!("{} {} {} {}", std::mem::size_of::<Shared>(), offset((&raw const (*ptr).sound).cast()),
        offset((&raw const (*ptr).frames).cast()), offset((&raw const (*ptr).frame).cast())) };
    assert_eq!(printed, rust);
    assert_eq!(std::mem::size_of::<Header>(), 24);
}

#[test]
fn a_game_starts_paused_runs_pauses_and_quits() {
    let (mut s, h) = start(None);
    assert_eq!(s.info(), &NativeInfo { name: "Mock".into(), version: "1.0".into(), width: 320, height: 240, fps: 60.0 });
    assert_eq!(s.frames(), 0, "nothing is presented before resume");
    s.set_buttons(button_bit(Button::A).unwrap() | button_bit(Button::Start).unwrap());
    s.resume().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    s.pause().unwrap();
    let presented = s.frames();
    assert!(presented >= 5, "{presented}");
    let f = h.frames.take().expect("the paused frame");
    assert_eq!((f.width, f.height, f.rgba.len()), (320, 240, 320 * 240 * 4));
    assert_eq!(f.rgba[0], 1 << 4, "the buttons reached the core (their low byte: A)");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(s.frames(), presented, "paused means paused");
    // The sound went through, at the buttons' value.
    let played = h.played.lock().unwrap().clone();
    assert!(played.len() > 1000, "{}", played.len());
    assert!(played.iter().any(|&[l, _]| l == ((1 << 4) | (1 << 13)) as i16));
    s.quit().unwrap();
    assert_eq!(std::fs::read_to_string(h.dir.path().join("saves/quit.txt")).unwrap(), "quit");
    assert!(h.saved.try_recv().is_ok(), "the save was reported");
    assert!(h.stopped.try_recv().is_err(), "a quit asked for isn't a stop");
}

#[test]
fn states_save_and_load_while_paused() {
    let (mut s, h) = start(None);
    s.resume().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let state = h.dir.path().join("a.state");
    s.save_state(&state).unwrap(); // pauses first
    let saved: u64 = std::fs::read_to_string(&state).unwrap().parse().unwrap();
    assert_eq!(saved, s.frames());
    s.resume().unwrap();
    std::thread::sleep(Duration::from_millis(100));
    s.load_state(&state).unwrap();
    let e = s.load_state(&h.dir.path().join("missing")).unwrap_err();
    assert_eq!(e.to_string(), "no such state");
}

#[test]
fn a_core_that_fails_says_why() {
    let (mut s, h) = start(Some(("MOCK_FAIL", "resume")));
    assert!(s.resume().is_err());
    assert_eq!(h.stopped.recv_timeout(Duration::from_secs(5)).unwrap(), "the disc can't be read");
    drop(s); // doesn't hang on a core that has gone
}

#[test]
fn a_program_that_isnt_a_core_is_turned_away() {
    let dir = tempfile::tempdir().unwrap();
    let launch = NativeLaunch { program: "/bin/true".into(), args: vec![], package: dir.path().join("p"), saves: dir.path().join("s"), cache: dir.path().join("c"), clock: 0, options: vec![] };
    let out = NativeOutputs { sound: None, frames: Arc::new(FrameSlot::default()), on_save: Box::new(|| {}), on_rumble: Box::new(|_| {}), on_stop: Box::new(|_| {}) };
    assert_eq!(NativeSession::start(launch, out).err().unwrap(), "the core has stopped");
}
