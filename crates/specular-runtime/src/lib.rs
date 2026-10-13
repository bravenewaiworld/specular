//! Specular's game runtime: runs a libretro core on its own thread, paced by the display, with
//! sound, battery saves and save states. libretro's callbacks carry no context, so a game's state
//! is kept per thread (see `frontend`). One game runs at a time: a core keeps its own state in
//! globals, and loading its library again gives the same, already loaded, copy.
//!
//! Each display refresh, the game's thread waits for the refresh, then a frame delay (how long
//! the core's start can be put off and still leave time for its run and the UI's draw before the
//! next refresh; starting late means buttons are read late, so a press reaches the screen
//! sooner), then runs the core once and hands the frame to the UI. Without a vsync, or when the
//! display's rate is far from the game's, the core is run by a timer at its own frame rate.

mod core;
mod frontend;
mod sound;

use std::collections::{HashMap, VecDeque};
use std::ffi::CString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering::Relaxed};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use specular_core::device::{SoundOut, Vsync};
use specular_core::input::Button;

use crate::core::Core;
use crate::frontend::Frontend;

/// What to run.
pub struct LaunchSpec {
    /// The core's shared library, e.g. `.../cores/mgba_libretro.so`.
    pub core: PathBuf,
    pub rom: PathBuf,
    /// The battery save (`.srm`): loaded before the game starts, written as the game changes it.
    pub save: PathBuf,
    /// Where cores look for BIOS files (`bios/`).
    pub system_dir: PathBuf,
    /// Core options to set, by key (e.g. "mgba_idle_optimization"); the rest keep their defaults.
    pub options: Vec<(String, String)>,
}

/// Where the game's output goes.
pub struct Outputs {
    pub sound: Option<Box<dyn SoundOut>>,
    pub vsync: Option<Box<dyn Vsync>>,
    /// The display's refresh rate (Hz).
    pub display_hz: f64,
    pub frames: Arc<FrameSlot>,
    /// Called on the game's thread after each new frame is put in `frames`.
    pub on_frame: Box<dyn Fn() + Send>,
}

/// About the running game.
#[derive(Clone, Debug, PartialEq)]
pub struct GameInfo {
    pub width: u32,
    pub height: u32,
    /// The shape to show the picture in (width / height), which may differ from its pixels'.
    pub aspect: f32,
    pub fps: f64,
}

/// How long the core took to make its recent frames (the last `STATS_FRAMES`), and how many
/// frames it has made since the game started.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats { pub frames: u64, pub median: Duration, pub p95: Duration, pub worst: Duration }

/// How many of the latest frames `Stats` covers.
const STATS_FRAMES: usize = 600;

/// The run times behind `Stats`, shared with the game's thread.
#[derive(Default)]
struct Runs {
    count: u64,
    recent: VecDeque<Duration>,
    /// When the core's current run began; None between runs (see `Session::watch`).
    running_since: Option<Instant>,
}

/// A picture from the game, 8-bit RGBA.
#[derive(Default)]
pub struct Frame { pub width: u32, pub height: u32, pub rgba: Vec<u8> }

/// The newest frame, handed from the game's thread to the UI. Neither side waits for the other:
/// a frame the UI hasn't taken yet is replaced by a newer one. Frames are reused.
#[derive(Default)]
pub struct FrameSlot { inner: Mutex<(Option<Frame>, Vec<Frame>)> }

impl FrameSlot {
    /// The newest frame, if there is one the UI hasn't taken. Give it back when done with it.
    pub fn take(&self) -> Option<Frame> { self.inner.lock().unwrap().0.take() }
    pub fn give_back(&self, f: Frame) { self.inner.lock().unwrap().1.push(f); }
    fn spare(&self) -> Frame { self.inner.lock().unwrap().1.pop().unwrap_or_default() }
    fn publish(&self, f: Frame) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(old) = inner.0.replace(f) { inner.1.push(old); }
    }
}

/// The libretro joypad button for one of the device's buttons. L2/R2 are the shoulder buttons'
/// second row; Menu, the volume keys and Power never reach the game.
pub fn joypad_bit(b: Button) -> Option<u16> {
    use Button::*;
    let id = match b {
        B => 0, Y => 1, Select => 2, Start => 3, Up => 4, Down => 5, Left => 6, Right => 7,
        A => 8, X => 9, L1 => 10, R1 => 11, L2 => 12, R2 => 13,
        _ => return None,
    };
    Some(1 << id)
}

/// Whether a game is running (see `Session::start`).
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Clears `RUNNING` when dropped: when the game's thread ends (even by a panic), or if it
/// couldn't be started.
struct Running;

impl Drop for Running {
    fn drop(&mut self) { RUNNING.store(false, Relaxed); }
}

enum Cmd {
    Pause(bool),
    Reset,
    SaveState(PathBuf, mpsc::Sender<io::Result<()>>),
    LoadState(PathBuf, mpsc::Sender<io::Result<()>>),
    /// Writes the battery save now, if it changed (before sleeping).
    Flush(mpsc::Sender<io::Result<()>>),
    Quit(mpsc::Sender<io::Result<()>>),
}

/// A running game. Dropping it quits the game (writing its battery save).
pub struct Session {
    cmds: mpsc::Sender<Cmd>,
    buttons: Arc<AtomicU16>,
    thread: Option<std::thread::JoinHandle<()>>,
    info: GameInfo,
    runs: Arc<Mutex<Runs>>,
}

impl Session {
    /// Loads the core and the game, on the game's own thread, and starts it running.
    /// Fails if another game is running (see the module's documentation).
    pub fn start(spec: LaunchSpec, out: Outputs) -> Result<Session, String> {
        if RUNNING.swap(true, Relaxed) { return Err("another game is running".into()); }
        let running = Running;
        let (cmds, rx) = mpsc::channel();
        let (started, ready) = mpsc::channel();
        let (buttons, runs) = (Arc::new(AtomicU16::new(0)), Arc::new(Mutex::new(Runs::default())));
        let (b, r) = (buttons.clone(), runs.clone());
        let thread = std::thread::Builder::new().name("game".into())
            .spawn(move || { let _running = running; game_thread(spec, out, b, r, rx, started) })
            .map_err(|e| e.to_string())?;
        match ready.recv() {
            Ok(Ok(info)) => Ok(Session { cmds, buttons, thread: Some(thread), info, runs }),
            Ok(Err(e)) => { let _ = thread.join(); Err(e) }
            Err(_) => { let _ = thread.join(); Err("the game's thread stopped while loading".into()) }
        }
    }

    pub fn info(&self) -> &GameInfo { &self.info }

    pub fn stats(&self) -> Stats {
        let runs = self.runs.lock().unwrap();
        let mut sorted: Vec<Duration> = runs.recent.iter().copied().collect();
        sorted.sort();
        let worst = sorted.last().copied().unwrap_or_default();
        Stats { frames: runs.count, median: percentile(&sorted, 50), p95: percentile(&sorted, 95), worst }
    }

    /// Calls `stuck`, once, if one run of the core (one frame, normally milliseconds) goes on for
    /// `limit`: the core is caught in a loop or a deadlock, and its thread can't be stopped. Time
    /// paused or waiting for the display isn't a run, so it never counts. Watches from a thread of
    /// its own, so it still works while the UI thread waits on the stuck game (to quit it, say),
    /// until the game ends. The core's other calls (save states, reset) aren't watched.
    pub fn watch(&self, limit: Duration, stuck: impl FnOnce() + Send + 'static) {
        let runs = Arc::downgrade(&self.runs);
        let running_since = move || runs.upgrade().map(|r| r.lock().unwrap().running_since);
        std::thread::spawn(move || while let Some(since) = running_since() {
            if since.is_some_and(|t| t.elapsed() >= limit) { return stuck(); }
            std::thread::sleep(limit / 4);
        });
    }

    /// The buttons held now, as `joypad_bit`s.
    pub fn set_buttons(&self, bits: u16) { self.buttons.store(bits, Relaxed); }

    pub fn pause(&self, paused: bool) { let _ = self.cmds.send(Cmd::Pause(paused)); }
    pub fn reset(&self) { let _ = self.cmds.send(Cmd::Reset); }
    pub fn save_state(&self, path: &Path) -> io::Result<()> { self.ask(|r| Cmd::SaveState(path.into(), r)) }
    pub fn load_state(&self, path: &Path) -> io::Result<()> { self.ask(|r| Cmd::LoadState(path.into(), r)) }
    pub fn flush(&self) -> io::Result<()> { self.ask(Cmd::Flush) }

    /// Ends the game: writes its battery save and unloads the core.
    pub fn quit(mut self) -> io::Result<()> { self.stop() }

    fn ask(&self, cmd: impl FnOnce(mpsc::Sender<io::Result<()>>) -> Cmd) -> io::Result<()> {
        let (tx, rx) = mpsc::channel();
        self.cmds.send(cmd(tx)).map_err(|_| io::Error::other("the game has stopped"))?;
        rx.recv().map_err(|_| io::Error::other("the game has stopped"))?
    }

    fn stop(&mut self) -> io::Result<()> {
        let Some(thread) = self.thread.take() else { return Ok(()) };
        let r = self.ask(Cmd::Quit);
        let _ = thread.join();
        r
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Err(e) = self.stop() { eprintln!("game: {e}"); }
    }
}

/// The `q`th percentile of `sorted` (zero if it is empty).
fn percentile(sorted: &[Duration], q: usize) -> Duration { sorted.get(sorted.len() * q / 100).copied().unwrap_or_default() }

/// How often the battery save is checked for changes and written.
const SAVE_INTERVAL: Duration = Duration::from_secs(10);
/// The UI's time to draw and present a frame once it has it; the frame delay leaves this much.
const UI_TIME: Duration = Duration::from_millis(6);

/// Times the game's frames (see the module doc).
struct Pacer {
    vsync: Option<Box<dyn Vsync>>,
    period: Duration,
    next: Instant,
    delay: Duration,
    /// How long the latest runs of the core took.
    runs: Vec<Duration>,
}

impl Pacer {
    fn new(vsync: Option<Box<dyn Vsync>>, hz: f64) -> Self {
        Self { vsync, period: Duration::from_secs_f64(1.0 / hz), next: Instant::now(), delay: Duration::ZERO, runs: Vec::new() }
    }

    /// Waits until the core should run.
    fn wait(&mut self) {
        if let Some(v) = self.vsync.as_mut() {
            match v.wait() {
                Ok(()) => { std::thread::sleep(self.delay); return; }
                Err(e) => { eprintln!("game: waiting for the display failed ({e}); pacing by a timer"); self.vsync = None; }
            }
        }
        let now = Instant::now();
        if self.next > now { std::thread::sleep(self.next - now); }
        // After a stall (or a pause), start again from now rather than rushing to catch up.
        self.next = (self.next + self.period).max(now);
    }

    /// With a vsync, records how long a run took, and every 60 runs sets the frame delay to what
    /// the slowest runs (the 95th percentile) leave.
    fn ran(&mut self, took: Duration) {
        if self.vsync.is_none() { return; }
        self.runs.push(took);
        if self.runs.len() < 60 { return; }
        self.runs.sort();
        self.delay = self.period.saturating_sub(percentile(&self.runs, 95) + UI_TIME).min(self.period / 2);
        self.runs.clear();
    }
}

fn game_thread(spec: LaunchSpec, out: Outputs, buttons: Arc<AtomicU16>, runs: Arc<Mutex<Runs>>, cmds: mpsc::Receiver<Cmd>,
               started: mpsc::Sender<Result<GameInfo, String>>) {
    let mut game = match Game::load(&spec, buttons, out.frames.clone(), out.on_frame) {
        Ok(g) => g,
        Err(e) => { let _ = started.send(Err(e)); return; }
    };
    let av = game.av;
    let fps = av.timing.fps.max(1.0);
    // A display within MAX_SPEED_CHANGE of the game's rate paces it (so it runs at the display's
    // rate, its sound sped up or slowed to match); otherwise a timer does, at the game's own.
    let speed = out.display_hz / fps;
    let by_display = out.vsync.is_some() && (speed - 1.0).abs() <= sound::MAX_SPEED_CHANGE;
    let mut pacer = Pacer::new(if by_display { out.vsync } else { None }, if by_display { out.display_hz } else { fps });
    let mut sound = sound::Sound::new(out.sound, av.timing.sample_rate, if by_display { speed } else { 1.0 });
    let g = av.geometry;
    let _ = started.send(Ok(GameInfo { width: g.base_width, height: g.base_height, aspect: g.aspect_ratio, fps }));

    let (mut paused, mut next_save) = (false, Instant::now() + SAVE_INTERVAL);
    loop {
        let cmd = match if paused { cmds.recv().map_err(|_| mpsc::TryRecvError::Disconnected) } else { cmds.try_recv() } {
            Ok(cmd) => Some(cmd),
            Err(mpsc::TryRecvError::Empty) => None,
            // The session is gone without saying quit: save, and end.
            Err(mpsc::TryRecvError::Disconnected) => { let _ = game.write_save(); return; }
        };
        if let Some(cmd) = cmd {
            match cmd {
                Cmd::Pause(p) => paused = p,
                Cmd::Reset => unsafe { (game.core.reset)() },
                Cmd::SaveState(path, reply) => { let _ = reply.send(game.save_state(&path)); }
                Cmd::LoadState(path, reply) => { let _ = reply.send(game.load_state(&path)); }
                Cmd::Flush(reply) => { let _ = reply.send(game.write_save()); }
                Cmd::Quit(reply) => { let _ = reply.send(game.write_save()); return; }
            }
            continue;
        }
        pacer.wait();
        let t = Instant::now();
        runs.lock().unwrap().running_since = Some(t);
        unsafe { (game.core.run)() };
        let took = t.elapsed();
        pacer.ran(took);
        {
            let mut r = runs.lock().unwrap();
            r.running_since = None;
            r.count += 1;
            r.recent.push_back(took);
            if r.recent.len() > STATS_FRAMES { r.recent.pop_front(); }
        }
        frontend::with(|f| { sound.play(&f.samples); f.samples.clear(); });
        if Instant::now() >= next_save {
            if let Err(e) = game.write_save() { eprintln!("game: battery save: {e}"); }
            next_save = Instant::now() + SAVE_INTERVAL;
        }
    }
}

/// A loaded core with its game. Unloads both when dropped.
struct Game {
    core: Core,
    av: core::AvInfo,
    save: PathBuf,
    /// The battery save as last written (or read), to write only when it changes.
    saved: Vec<u8>,
    /// The save file exists but couldn't be loaded yet, because the core had no save memory
    /// then: it must not be overwritten until it is.
    save_pending: bool,
    /// Kept for the core, which may hold on to the path. (The ROM's contents are freed once the
    /// core has loaded them: libretro only keeps them valid that long, unless a core asks for
    /// longer with SET_CONTENT_INFO_OVERRIDE, which Specular refuses; so cores copy what they
    /// need, and keeping our copy would only double the game's memory.)
    _rom_path: CString,
}

impl Game {
    fn load(spec: &LaunchSpec, buttons: Arc<AtomicU16>, frames: Arc<FrameSlot>, on_frame: Box<dyn Fn() + Send>) -> Result<Self, String> {
        let core = Core::load(&spec.core)?;
        let cstring = |p: &Path| CString::new(p.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string());
        let (system_dir, save_dir, rom_path) = (cstring(&spec.system_dir)?, cstring(spec.save.parent().unwrap_or(Path::new(".")))?, cstring(&spec.rom)?);
        // SAFETY: null pointers and false are valid values for every field.
        let mut info: core::SystemInfo = unsafe { std::mem::zeroed() };
        unsafe { (core.get_system_info)(&mut info) };
        // A core that reads the file itself gets only its path; others get its contents.
        let rom = if info.need_fullpath { None } else {
            Some(MappedFile::open(&spec.rom).map_err(|e| format!("reading {}: {e}", spec.rom.display()))?)
        };
        let rom_data = rom.as_ref().map_or(&[][..], |m| m.bytes());
        frontend::install(Some(Frontend {
            buttons, frames, on_frame,
            pixel_format: core::PIXEL_0RGB1555, // libretro's default
            samples: Vec::new(),
            variables: HashMap::new(),
            overrides: spec.options.iter().cloned().collect(),
            system_dir, save_dir,
        }));
        frontend::connect(&core);
        unsafe { (core.init)() };
        let game = core::GameInfo { path: rom_path.as_ptr(), data: if rom_data.is_empty() { std::ptr::null() } else { rom_data.as_ptr().cast() },
            size: rom_data.len(), meta: std::ptr::null() };
        if !unsafe { (core.load_game)(&game) } {
            unsafe { (core.deinit)() };
            frontend::install(None);
            return Err(format!("the core could not load {}", spec.rom.display()));
        }
        let mut av = core::AvInfo::default();
        unsafe { (core.get_system_av_info)(&mut av) };
        let mut g = Game { core, av, save: spec.save.clone(), saved: Vec::new(), save_pending: false, _rom_path: rom_path };
        drop(rom);
        g.read_save();
        Ok(g)
    }

    /// Fills the core's battery memory from the save file, if there is one.
    fn read_save(&mut self) {
        let Ok(data) = std::fs::read(&self.save) else { return };
        let ram = self.core.save_ram();
        self.save_pending = ram.is_empty();
        let n = ram.len().min(data.len());
        ram[..n].copy_from_slice(&data[..n]);
        if data.len() != ram.len() { eprintln!("game: {} is {} bytes, the game's save memory {}", self.save.display(), data.len(), ram.len()); }
        self.saved = ram.to_vec();
    }

    /// Writes the battery save if it changed since last written. The size is read each time:
    /// some cores (mGBA) only know it once the game has used its save memory.
    fn write_save(&mut self) -> io::Result<()> {
        // A core that only now has save memory gets the file first.
        if self.save_pending { self.read_save(); return Ok(()); }
        let ram = self.core.save_ram();
        if ram.is_empty() || ram == self.saved.as_slice() { return Ok(()); }
        let data = ram.to_vec();
        write_atomically(&self.save, &data)?;
        self.saved = data;
        Ok(())
    }

    fn save_state(&mut self, path: &Path) -> io::Result<()> {
        let size = unsafe { (self.core.serialize_size)() };
        let mut data = vec![0u8; size];
        if size == 0 || !unsafe { (self.core.serialize)(data.as_mut_ptr().cast(), size) } {
            return Err(io::Error::other("the core could not save its state"));
        }
        write_atomically(path, &data)
    }

    fn load_state(&mut self, path: &Path) -> io::Result<()> {
        let data = std::fs::read(path)?;
        if !unsafe { (self.core.unserialize)(data.as_ptr().cast(), data.len()) } {
            return Err(io::Error::other("the core could not load this state"));
        }
        Ok(())
    }
}

impl Drop for Game {
    fn drop(&mut self) {
        unsafe {
            (self.core.unload_game)();
            (self.core.deinit)();
        }
        frontend::install(None);
    }
}

/// A file mapped into memory, read-only: the ROM handed to a core. Unlike a buffer read into
/// the heap, unmapping it gives the memory straight back (the heap keeps freed large blocks).
struct MappedFile { ptr: *mut libc::c_void, len: usize }

impl MappedFile {
    fn open(path: &Path) -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        let f = std::fs::File::open(path)?;
        let len = f.metadata()?.len() as usize;
        if len == 0 { return Ok(Self { ptr: std::ptr::null_mut(), len: 0 }); }
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, f.as_raw_fd(), 0) };
        if ptr == libc::MAP_FAILED { return Err(io::Error::last_os_error()); }
        Ok(Self { ptr, len })
    }

    fn bytes(&self) -> &[u8] {
        if self.len == 0 { return &[]; }
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }
}

impl Drop for MappedFile {
    fn drop(&mut self) { if self.len > 0 { unsafe { libc::munmap(self.ptr, self.len); } } }
}

/// Writes `data` to `path` so a power cut leaves either the old file or the new one (FAT has no
/// journal): to `<path>.tmp`, flushed, renamed over `path`, and the rename itself flushed.
fn write_atomically(path: &Path, data: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    std::fs::File::open(dir)?.sync_all()
}

#[cfg(test)]
mod tests;
