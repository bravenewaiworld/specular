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
mod log;
mod options;
mod sound;
mod unzip;

use std::collections::VecDeque;
use std::ffi::CString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering::Relaxed};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use specular_core::device::{SoundOut, Vsync};
use specular_core::input::Button;
use specular_core::launch::LaunchRecord;
use specular_core::rumble::RumbleLine;
use specular_core::settings::core_options::CoreOptions;

use crate::core::Core;
use crate::frontend::Frontend;

/// What to run.
pub struct LaunchSpec {
    /// The core's shared library, e.g. `.../cores/mgba_libretro.so`.
    pub core: PathBuf,
    /// The game's file. A zip is unzipped into memory first, unless the core opens zips itself
    /// (see `unzip`).
    pub rom: PathBuf,
    /// The battery save (`.srm`): loaded before the game starts; written when the game has
    /// written its save memory (or, with a core that doesn't say when, within 10 s of play of it
    /// changing), on `Session::flush`, and when the game ends.
    pub save: PathBuf,
    /// The automatic save state, if any: the game resumes from it if it exists (one the core
    /// can't load is ignored, and replaced by the next), and it is written every 10 s of play
    /// (time paused doesn't count), on `Session::flush`, and when the game ends. Written
    /// zstd-compressed, like every save state, so other frontends can't read it as it is.
    pub autosave: Option<PathBuf>,
    /// Where cores look for BIOS files (`bios/`).
    pub system_dir: PathBuf,
    /// Core options to set, by key (e.g. "mgba_idle_optimization"); the rest keep the core's
    /// defaults. A value the core doesn't take is logged, and its option keeps its default.
    pub options: Vec<(String, String)>,
    /// The options the core should declare (its list in data/cores/): how what it declares differs
    /// is logged at launch. None for a core without a list.
    pub option_list: Option<&'static CoreOptions>,
    /// Give the core only the ROM's path, even if it asks for the contents. Only for a core known
    /// to then read the file itself (libretro doesn't promise that a core will); it can map the
    /// file instead of copying it.
    pub rom_by_path: bool,
    /// The launch record, and where to keep it (see `specular_core::launch`): written on the
    /// game's thread before the core is loaded, removed once the game has ended (or failed to
    /// start) and its core is unloaded. Since only one game runs at a time (`Session::start`), one
    /// launch's ending can't remove the record of another.
    pub record: Option<(PathBuf, LaunchRecord)>,
    /// The core's save states hold its save memory too, so loading one rolls the battery save
    /// back. Then, the first time the battery save is written after a state is loaded, a save on
    /// the card that the state wasn't made with (a newer one) is first renamed to
    /// `<name>.newer.srm`.
    pub states_restore_save: bool,
}

/// Where the game's output goes.
pub struct Outputs {
    /// Where the sound plays at first (`Session::set_sound` changes it).
    pub sound: Option<Box<dyn SoundOut>>,
    pub vsync: Option<Box<dyn Vsync>>,
    /// The display's refresh rate (Hz).
    pub display_hz: f64,
    pub frames: Arc<FrameSlot>,
    /// Called on the game's thread after each new frame is put in `frames`.
    pub on_frame: Box<dyn Fn() + Send>,
    /// Where the game's rumble goes (None: the core is offered none). Stilled while the game is
    /// paused, and when it ends.
    pub rumble: Option<RumbleLine>,
    /// Called on the game's thread when the game has saved in-game: its changed battery save was
    /// queued for writing while it played (not the writes on flush or quit). At once with a core
    /// that says when its save memory was written (`ENV_SAVE_RAM_WRITTEN`), else at the next check,
    /// up to 10 s of play later.
    pub on_save: Box<dyn Fn() + Send>,
}

/// About the running game.
#[derive(Clone, Debug, PartialEq)]
pub struct GameInfo {
    pub width: u32,
    pub height: u32,
    /// The shape to show the picture in (width / height), which may differ from its pixels'.
    pub aspect: f32,
    pub fps: f64,
    /// The options the core declared as it loaded the game (for learning those of a core the user
    /// added: `user_cores::learn` in specular-core).
    pub options: CoreOptions,
}

/// How long the core took to make its recent frames (the last `STATS_FRAMES`), and how many
/// frames it has made since the game started.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub frames: u64, pub median: Duration, pub p95: Duration, pub worst: Duration,
    /// How much sound was waiting to play at the last frame (None without sound): rate control
    /// keeps it near half the output's queue.
    pub sound_queued: Option<Duration>,
}

/// How many of the latest frames `Stats` covers.
const STATS_FRAMES: usize = 600;

/// The run times behind `Stats`, shared with the game's thread.
#[derive(Default)]
struct Runs {
    count: u64,
    recent: VecDeque<Duration>,
    /// When the core's current run began; None between runs (see `Session::watch`).
    running_since: Option<Instant>,
    sound_queued: Option<Duration>,
    /// The loudest sample since `Session::take_sound_peak` last took it.
    sound_peak: u16,
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
    /// Writes the battery save and the autosave now (before sleeping).
    Flush(mpsc::Sender<io::Result<()>>),
    /// Plays on this output from the next frame (None: silence); see `Session::set_sound`.
    Sound(Option<Box<dyn SoundOut>>),
    /// Gives the core these option values instead; see `Session::set_options`.
    Options(Vec<(String, String)>),
    Quit(mpsc::Sender<io::Result<()>>),
}

/// A running game. Dropping it quits the game (writing its battery save and autosave).
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
        Stats { frames: runs.count, median: percentile(&sorted, 50), p95: percentile(&sorted, 95), worst, sound_queued: runs.sound_queued }
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

    /// The loudest sample of the game's sound (0–32768, left or right) since the last call.
    pub fn take_sound_peak(&self) -> u16 { std::mem::take(&mut self.runs.lock().unwrap().sound_peak) }

    /// The buttons held now, as `joypad_bit`s.
    pub fn set_buttons(&self, bits: u16) { self.buttons.store(bits, Relaxed); }

    pub fn pause(&self, paused: bool) { let _ = self.cmds.send(Cmd::Pause(paused)); }
    pub fn reset(&self) { let _ = self.cmds.send(Cmd::Reset); }
    /// From the next frame, plays the game's sound on `out` (None: silence), closing the previous
    /// output. Returns at once: open `out` beforehand, since opening can block.
    pub fn set_sound(&self, out: Option<Box<dyn SoundOut>>) { let _ = self.cmds.send(Cmd::Sound(out)); }
    /// Gives the core these option values from now on, as `LaunchSpec::options` does at launch
    /// (its other options go back to its defaults). If any value changes, the core is told the
    /// next time it asks (libretro's GET_VARIABLE_UPDATE, usually each frame); some options only
    /// take effect when the game is started again.
    pub fn set_options(&self, options: Vec<(String, String)>) { let _ = self.cmds.send(Cmd::Options(options)); }
    /// Saves the game's state to `path` (zstd-compressed); `load_state` also takes uncompressed ones.
    pub fn save_state(&self, path: &Path) -> io::Result<()> { self.ask(|r| Cmd::SaveState(path.into(), r)) }
    pub fn load_state(&self, path: &Path) -> io::Result<()> { self.ask(|r| Cmd::LoadState(path.into(), r)) }
    /// Writes the battery save and the autosave (before sleeping, say), returning once they are
    /// on the card. The error is the first of any write since the last flush or save state,
    /// including the ones made in the background.
    pub fn flush(&self) -> io::Result<()> { self.ask(Cmd::Flush) }

    /// Ends the game: writes its battery save and autosave, and unloads the core.
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

/// How much play passes between autosaves, and between checks for a changed battery save (for
/// cores that don't say when the game writes it).
const CHECK_INTERVAL: Duration = Duration::from_secs(10);
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
    let report = |r: io::Result<()>, path: &Path| if let Err(e) = r { eprintln!("game: launch record {}: {e}", path.display()); };
    if let Some((path, r)) = &spec.record { report(r.write(path), path); }
    play(&spec, out, buttons, runs, cmds, started);
    log::stop(); // (here, not in `Game`'s drop, so it also runs when loading fails)
    // Not reached if the core crashes the process, or hangs and the watchdog ends it.
    if let Some((path, _)) = &spec.record { report(LaunchRecord::remove(path), path); }
}

/// Loads and runs the game until it is quit (or fails to load); its core is unloaded on return.
fn play(spec: &LaunchSpec, out: Outputs, buttons: Arc<AtomicU16>, runs: Arc<Mutex<Runs>>, cmds: mpsc::Receiver<Cmd>,
        started: mpsc::Sender<Result<GameInfo, String>>) {
    let mut game = match Game::load(spec, buttons, out.frames.clone(), out.on_frame, out.rumble) {
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
    let options = frontend::with(|f| f.options.declared.clone()).unwrap_or_default();
    let _ = started.send(Ok(GameInfo { width: g.base_width, height: g.base_height, aspect: g.aspect_ratio, fps, options }));

    // Counted in frames played, so time paused doesn't count.
    let check_frames = (CHECK_INTERVAL.as_secs_f64() * fps) as u32;
    let (mut paused, mut frames_to_check) = (false, check_frames);
    loop {
        let cmd = match if paused { cmds.recv().map_err(|_| mpsc::TryRecvError::Disconnected) } else { cmds.try_recv() } {
            Ok(cmd) => Some(cmd),
            Err(mpsc::TryRecvError::Empty) => None,
            // The session is gone without saying quit: save, and end.
            Err(mpsc::TryRecvError::Disconnected) => { game.save_all(None); return; }
        };
        if let Some(cmd) = cmd {
            match cmd {
                Cmd::Pause(p) => {
                    paused = p;
                    frontend::with(|f| f.pause_rumble(p));
                }
                Cmd::Reset => unsafe { (game.core.reset)() },
                Cmd::SaveState(path, reply) => game.save_state(path, reply),
                Cmd::LoadState(path, reply) => { let _ = reply.send(game.load_state(&path)); }
                Cmd::Flush(reply) => game.save_all(Some(reply)),
                Cmd::Sound(out) => sound.set_out(out),
                Cmd::Options(options) => game.set_options(options),
                Cmd::Quit(reply) => { game.save_all(Some(reply)); return; }
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
        let (queued, peak) = frontend::with(|f| {
            let played = (sound.play(&f.samples), sound::peak(&f.samples));
            f.samples.clear();
            played
        }).unwrap_or_default();
        {
            let mut r = runs.lock().unwrap();
            r.sound_queued = queued;
            r.sound_peak = r.sound_peak.max(peak);
        }
        if frontend::with(|f| std::mem::take(&mut f.save_ram_written)) == Some(true) && game.write_save(false) { (out.on_save)(); }
        frames_to_check -= 1;
        if frames_to_check == 0 {
            if game.write_save(false) { (out.on_save)(); }
            game.write_autosave();
            frames_to_check = check_frames;
        }
    }
}

/// A loaded core with its game. Unloads both when dropped.
struct Game {
    core: Core,
    /// What the core calls itself, for the log.
    name: String,
    av: core::AvInfo,
    save: PathBuf,
    /// The battery save as last written (or read), to write only when it changes.
    saved: Vec<u8>,
    /// The save file exists but couldn't be loaded yet, because the core had no save memory
    /// then: it must not be overwritten until it is.
    save_pending: bool,
    states_restore_save: bool,
    /// The state last loaded, if it restored the save memory: kept until the battery save is
    /// next written, which checks it (see `LaunchSpec::states_restore_save`).
    loaded_state: Option<LoadedState>,
    autosave: Option<PathBuf>,
    writer: Writer,
    /// Kept for the core, which may hold on to the ROM's path, and open or map the file there (an
    /// unzipped ROM's file goes when this does). (The ROM's contents are freed once the core has
    /// loaded them: libretro only keeps them valid that long, unless a core asks for longer with
    /// SET_CONTENT_INFO_OVERRIDE, which Specular refuses; so cores copy what they need, and
    /// keeping our copy would only double the game's memory.)
    _rom: (CString, Option<unzip::Unzipped>),
}

impl Game {
    fn load(spec: &LaunchSpec, buttons: Arc<AtomicU16>, frames: Arc<FrameSlot>, on_frame: Box<dyn Fn() + Send>, rumble: Option<RumbleLine>) -> Result<Self, String> {
        let core = Core::load(&spec.core)?;
        // SAFETY: null pointers and false are valid values for every field.
        let mut info: core::SystemInfo = unsafe { std::mem::zeroed() };
        unsafe { (core.get_system_info)(&mut info) };
        let name = info.name();
        log::start(name.clone());
        let unzipped = unzip::if_needed(&spec.rom, &info)?;
        let rom_file = unzipped.as_ref().map_or(spec.rom.as_path(), |u| &u.path);
        let cstring = |p: &Path| CString::new(p.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string());
        let (system_dir, save_dir, rom_path) = (cstring(&spec.system_dir)?, cstring(spec.save.parent().unwrap_or(Path::new(".")))?, cstring(rom_file)?);
        // A core that reads the file itself (as it says, or as the spec says) gets only its path;
        // others get its contents too.
        let rom = if info.need_fullpath || spec.rom_by_path { None } else {
            Some(MappedFile::open(rom_file).map_err(|e| format!("reading {}: {e}", spec.rom.display()))?)
        };
        let rom_data = rom.as_ref().map_or(&[][..], |m| m.bytes());
        frontend::install(Some(Frontend {
            buttons, frames, on_frame, options: options::Options::new(spec.options.clone()), system_dir, save_dir, rumble, ..Frontend::default()
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
        log_options(&name, spec);
        let mut av = core::AvInfo::default();
        unsafe { (core.get_system_av_info)(&mut av) };
        let writer = Writer::new().map_err(|e| format!("starting the save writer: {e}"))?;
        let mut g = Game { core, name, av, save: spec.save.clone(), saved: Vec::new(), save_pending: false,
                           states_restore_save: spec.states_restore_save, loaded_state: None,
                           autosave: spec.autosave.clone(), writer, _rom: (rom_path, unzipped) };
        drop(rom);
        g.read_save();
        if let Some(p) = spec.autosave.as_deref().filter(|p| p.exists()) && let Err(e) = g.load_state(p) {
            // Starting afresh instead: the next autosave replaces it.
            eprintln!("game: resuming from {}: {e}", p.display());
        }
        Ok(g)
    }

    /// See `Session::set_options`.
    fn set_options(&mut self, options: Vec<(String, String)>) {
        for c in frontend::with(|f| f.options.give(options)).unwrap_or_default() { eprintln!("game: {} {c}", self.name); }
    }

    /// Fills the core's battery memory from the save file, if there is one.
    fn read_save(&mut self) {
        let Ok(data) = std::fs::read(&self.save) else { return };
        let ram = self.core.save_ram();
        self.save_pending = ram.is_empty();
        let n = ram.len().min(data.len());
        ram[..n].copy_from_slice(&data[..n]);
        if !ram.is_empty() && data.len() != ram.len() { eprintln!("game: {} is {} bytes, the game's save memory {}", self.save.display(), data.len(), ram.len()); }
        self.saved = ram.to_vec();
    }

    /// Queues the battery save to be written if it changed since last written; with `always`,
    /// even if it didn't (the last write may have failed: the writer's errors aren't seen here).
    /// The size is read each time: some cores (mGBA) only know it once the game has used its
    /// save memory. Returns whether it was queued.
    fn write_save(&mut self, always: bool) -> bool {
        // A core that only now has save memory gets the file first.
        if self.save_pending {
            if !self.core.save_ram().is_empty() { self.read_save(); }
            return false;
        }
        let ram = self.core.save_ram();
        if ram.is_empty() || (!always && ram == self.saved.as_slice()) { return false; }
        self.saved = ram.to_vec();
        self.writer.write_save(self.save.clone(), self.saved.clone(), self.loaded_state.take());
        true
    }

    /// The crc32 of the save memory as it is now; None if the core has none.
    fn save_crc(&mut self) -> Option<u32> {
        let ram = self.core.save_ram();
        (!ram.is_empty()).then(|| crc32fast::hash(ram))
    }

    /// Queues the autosave to be written, if there is one.
    fn write_autosave(&mut self) {
        let Some(path) = self.autosave.clone() else { return };
        if let Err(e) = self.queue_state(path) { eprintln!("game: autosave: {e}"); }
    }

    /// Writes a save state to `path`, then sends `reply` once it is on the card.
    fn save_state(&mut self, path: PathBuf, reply: mpsc::Sender<io::Result<()>>) {
        match self.queue_state(path) {
            Ok(()) => self.writer.reply(reply),
            Err(e) => { let _ = reply.send(Err(e)); }
        }
    }

    /// Queues the battery save (even if unchanged) and the autosave, then `reply` (if any), sent
    /// once they are on the card, with the first error since the writer's last reply.
    fn save_all(&mut self, reply: Option<mpsc::Sender<io::Result<()>>>) {
        self.write_save(true);
        self.write_autosave();
        if let Some(r) = reply { self.writer.reply(r); }
    }

    /// Queues the game's state to be written to `path` as a save state, with the crc32 of the
    /// save memory as it is now.
    fn queue_state(&mut self, path: PathBuf) -> io::Result<()> {
        let size = unsafe { (self.core.serialize_size)() };
        let mut data = vec![0u8; size];
        if size == 0 || !unsafe { (self.core.serialize)(data.as_mut_ptr().cast(), size) } {
            return Err(io::Error::other("the core could not save its state"));
        }
        let save_crc = self.save_crc();
        self.writer.write_state(path, data, save_crc);
        Ok(())
    }

    /// Loads a save state written by `Writer::write_state`, or an uncompressed one (as earlier
    /// builds wrote them).
    fn load_state(&mut self, path: &Path) -> io::Result<()> {
        let mut data = std::fs::read(path)?;
        let mut save_crc = None;
        if data.starts_with(&ZSTD_MAGIC) {
            save_crc = save_crc_of(&data);
            data = zstd::decode_all(data.as_slice())?;
        }
        if !unsafe { (self.core.unserialize)(data.as_ptr().cast(), data.len()) } {
            return Err(io::Error::other("the core could not load this state"));
        }
        if self.states_restore_save { self.loaded_state = Some(LoadedState { path: path.into(), save_crc }); }
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

/// Logs, once the core has loaded its game (by when it has declared its options), how its options
/// differ from its list, the values it is given, and those it doesn't take.
fn log_options(name: &str, spec: &LaunchSpec) {
    let Some((declared, refused)) = frontend::with(|f| (f.options.declared.clone(), f.options.refused())) else { return };
    let differences = spec.option_list.map(|l| l.differences(&declared)).unwrap_or_default();
    for d in differences { eprintln!("game: {name}'s options differ from Specular's list (data/cores/): {d}"); }
    let given: Vec<String> = spec.options.iter().map(|(k, v)| format!("{k} = {v}")).collect();
    eprintln!("game: {name}'s options: {}", if given.is_empty() { "its defaults".into() } else { format!("{}, the rest its defaults", given.join("; ")) });
    for r in refused { eprintln!("game: {name} doesn't take {r}"); }
}

/// The options a core declares as it loads, given no values: for `make cores`, which writes them
/// to data/cores/ (the `core-options` example). Options a core declares only once it has a game
/// aren't seen. Fails if a game is running, whose core's state this would disturb (see
/// `Session::start`).
pub fn declared_options(core: &Path) -> Result<CoreOptions, String> {
    if RUNNING.swap(true, Relaxed) { return Err("a game is running".into()); }
    let _running = Running;
    let core = Core::load(core)?;
    frontend::install(Some(Frontend::default()));
    frontend::connect(&core);
    unsafe { (core.init)() };
    let declared = frontend::with(|f| std::mem::take(&mut f.options.declared)).unwrap_or_default();
    unsafe { (core.deinit)() };
    frontend::install(None);
    Ok(declared)
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

/// Writes files on a thread of its own, in the order queued, so the game's thread never waits
/// for the card: flushing a write to an SD card can take longer than a frame. Once dropped, its
/// thread finishes the writes queued, then ends; `Job::Reply` is how to wait for them.
struct Writer(mpsc::Sender<Job>);

enum Job {
    /// A battery save, written as it is. With `loaded_state`, a different save already on the
    /// card is kept first (see `keep_newer_save`).
    Save { path: PathBuf, data: Vec<u8>, loaded_state: Option<LoadedState> },
    /// A save state: compressed, then followed by the crc32 of the save memory it was made with.
    State { path: PathBuf, data: Vec<u8>, save_crc: Option<u32> },
    /// Sends how the writes since the last reply went: the first error, if any failed.
    Reply(mpsc::Sender<io::Result<()>>),
}

impl Writer {
    fn new() -> io::Result<Self> {
        let (jobs, rx) = mpsc::channel();
        std::thread::Builder::new().name("game-saves".into()).spawn(move || {
            let mut failed = None;
            for job in rx {
                let (path, written) = match job {
                    Job::Save { path, data, loaded_state } => {
                        let kept = loaded_state.map_or(Ok(()), |s| keep_newer_save(&path, &data, &s));
                        let written = kept.and_then(|()| write_atomically(&path, &data));
                        (path, written)
                    }
                    Job::State { path, data, save_crc } => {
                        // Level 1, the fastest of zstd's usual levels: higher ones shrink a GBA
                        // state only about 4 % more.
                        let data = zstd::bulk::compress(&data, 1).map(|z| with_save_crc(z, save_crc));
                        let written = data.and_then(|data| write_atomically(&path, &data));
                        (path, written)
                    }
                    Job::Reply(reply) => { let _ = reply.send(failed.take().map_or(Ok(()), Err)); continue; }
                };
                if let Err(e) = written {
                    eprintln!("game: writing {}: {e}", path.display());
                    failed.get_or_insert(e);
                }
            }
        })?;
        Ok(Self(jobs))
    }

    /// Writes a battery save uncompressed, which other emulators and tools must be able to read.
    fn write_save(&self, path: PathBuf, data: Vec<u8>, loaded_state: Option<LoadedState>) {
        let _ = self.0.send(Job::Save { path, data, loaded_state });
    }
    /// Writes a save state, zstd-compressed, to wear the storage less (an autosave is written
    /// every 10 s of play): a Pokémon Emerald state shrinks about 11 times, 516 KiB to 45 KiB.
    fn write_state(&self, path: PathBuf, data: Vec<u8>, save_crc: Option<u32>) {
        let _ = self.0.send(Job::State { path, data, save_crc });
    }
    fn reply(&self, reply: mpsc::Sender<io::Result<()>>) { let _ = self.0.send(Job::Reply(reply)); }
}

/// The first bytes of every zstd frame. A core's own state format starting with them by chance
/// would be taken for compressed, fail to decompress and not load; mGBA's states don't start
/// with them.
const ZSTD_MAGIC: [u8; 4] = zstd::zstd_safe::MAGICNUMBER.to_le_bytes();

/// A save state loaded into a core whose states restore its save memory.
struct LoadedState {
    path: PathBuf,
    /// The crc32 of the save memory the state was made with; None if it doesn't say.
    save_crc: Option<u32>,
}

/// Called before the battery save at `path` is first overwritten with `data` after `state` was
/// loaded. A save on the card that is neither `data` nor the save the state was made with is newer
/// than the state (the game saved after it was made), so it is renamed to `<name>.newer.srm`
/// (replacing any earlier one) rather than lost. If the state doesn't record its save's crc32, any
/// save on the card other than `data` is renamed. A failed rename stops the write.
fn keep_newer_save(path: &Path, data: &[u8], state: &LoadedState) -> io::Result<()> {
    let on_card = match std::fs::read(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        r => r?,
    };
    let crc = crc32fast::hash(&on_card);
    if on_card == data || state.save_crc == Some(crc) { return Ok(()); }
    let newer = path.with_extension("newer.srm");
    let made_with = state.save_crc.map_or("unknown".into(), |c| format!("{c:08x}"));
    eprintln!("game: {} (crc32 {crc:08x}) isn't the save the state {} was made with (crc32 {made_with}) and would be overwritten by \
               the save it restored (crc32 {:08x}): keeping it as {}", path.display(), state.path.display(), crc32fast::hash(data), newer.display());
    std::fs::rename(path, &newer)?;
    std::fs::File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()
}

/// The start of the skippable frame (zstd's tools pass over it) that follows a save state's zstd
/// frame: marker, length and the tag "SRAM". The crc32 (little-endian) of the save memory the
/// state was made with comes after it.
const SAVE_CRC_HEAD: [u8; 12] = {
    let [a, b, c, d] = 0x184d_2a5e_u32.to_le_bytes(); // one of zstd's skippable frame markers
    [a, b, c, d, 8, 0, 0, 0, b'S', b'R', b'A', b'M'] // then the frame's length (8), and the tag
};

/// Appends the save crc32 frame (`SAVE_CRC_HEAD`) to a compressed state, if there is a crc.
fn with_save_crc(mut state: Vec<u8>, crc: Option<u32>) -> Vec<u8> {
    if let Some(crc) = crc { state.extend(SAVE_CRC_HEAD.iter().chain(&crc.to_le_bytes())); }
    state
}

/// The save crc32 at the end of a compressed state file, if it has one (see `with_save_crc`).
fn save_crc_of(file: &[u8]) -> Option<u32> {
    let (head, crc) = file[file.len().checked_sub(16)?..].split_at(12);
    (head == SAVE_CRC_HEAD).then(|| u32::from_le_bytes(crc.try_into().unwrap()))
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
