//! The running game, as the UI shows it: its session (`specular_runtime`), the buttons held for
//! it, and the texture its frames are drawn into. Frames are uploaded on the UI thread just before
//! Slint draws (the only time its GL context is sure to be current), into one texture that the
//! window shows as a borrowed GL texture.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::Duration;

use glow::HasContext;
use specular_core::device::{Device, Output, SCREEN_LENT, SoundOut};
use specular_core::app::{Discs, Msg};
use specular_core::cpu::CpuMin;
use specular_core::effects::Event;
use specular_core::input::{Button, ButtonEvent};
use specular_core::launch::{self, LaunchRecord};
use specular_core::library::{Autosave, CoreDecl, Game, user_cores};
use specular_runtime::native::{self, NativeLaunch, NativeOutputs, NativeSession, StartFailure};
use specular_runtime::{FrameSlot, LaunchSpec, Outputs, Session, StartError, Stats};

use crate::{AppWindow, Runner};

/// What the player is told when `core` couldn't start a game of `system` (its name).
fn start_failure(core: &CoreDecl, system: &str, e: &StartError) -> String {
    match e {
        StartError::Unzip(why) => format!("The game couldn't start: {why}."),
        StartError::Core(_) => match core.user {
            Some(u) => format!("{} in cores/ on the SD card doesn't work on this device.", u.path.file_name().unwrap_or_default().to_string_lossy()),
            None => format!("{} couldn't be started. Restart the device and try again.", core.name),
        },
        StartError::Game(_) => format!("{} couldn't open this game. The file may be damaged, or not a {system} game.", core.name),
        StartError::Other(_) => "The game couldn't start. Try again.".into(),
    }
}

/// Where Specular's own cores are installed: `$SPECULAR_CORES`, else `cores/` next to Specular's
/// binary.
fn cores_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("SPECULAR_CORES") { return d.into(); }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().unwrap_or(Path::new(".")).join("cores")
}

/// A game's session: a libretro core's, or a native core's (`specular_runtime::native`), which is
/// a program of its own that draws straight to the screen while the game runs.
pub enum GameSession {
    Libretro(Session),
    Native(NativeSession),
}

pub use specular_core::library::NATIVE_HAS_NO_STATES;

impl GameSession {
    /// Calls `stuck` if the game makes no progress for `limit` while it runs (`Session::watch`,
    /// `NativeSession::watch`).
    pub fn watch(&self, limit: Duration, stuck: impl FnOnce() + Send + 'static) {
        match self { Self::Libretro(s) => s.watch(limit, stuck), Self::Native(n) => n.watch(limit, stuck) }
    }
    pub fn set_options(&self, options: Vec<(String, String)>) { if let Self::Libretro(s) = self { s.set_options(options) } }
    /// Writes the autosave (a state: a native core has none; its saves are the game's own).
    pub fn autosave(&self) { if let Self::Libretro(s) = self { s.autosave() } }
    pub fn save_state(&self, path: &Path) -> io::Result<()> {
        match self { Self::Libretro(s) => s.save_state(path), Self::Native(n) => n.save_state(path).map_err(native_state_error) }
    }
    pub fn load_state(&self, path: &Path) -> io::Result<()> {
        match self { Self::Libretro(s) => s.load_state(path), Self::Native(n) => n.load_state(path).map_err(native_state_error) }
    }
    pub fn change_disc(&self, index: usize) -> io::Result<()> {
        match self { Self::Libretro(s) => s.change_disc(index), Self::Native(_) => Err(io::ErrorKind::Unsupported.into()) }
    }
    pub fn reset(&self) { if let Self::Libretro(s) = self { s.reset() } }
    /// Writes the game's saves now: a native core's are written as the game makes them.
    pub fn flush(&self) -> io::Result<()> { match self { Self::Libretro(s) => s.flush(), Self::Native(_) => Ok(()) } }
    pub fn discs(&self) -> Discs { match self { Self::Libretro(s) => s.discs(), Self::Native(_) => Discs::default() } }
    pub fn bios(&self) -> Option<String> { match self { Self::Libretro(s) => s.bios(), Self::Native(_) => None } }
    pub fn set_sound(&self, out: Option<Box<dyn SoundOut>>) { match self { Self::Libretro(s) => s.set_sound(out), Self::Native(n) => n.set_sound(out) } }
    /// The frame statistics: a native core's are only its frames (they can't be shown over it).
    pub fn stats(&self) -> Stats {
        match self {
            Self::Libretro(s) => s.stats(),
            Self::Native(n) => Stats { frames: n.frames(), median: Duration::ZERO, p95: Duration::ZERO, worst: Duration::ZERO, sound_queued: None, cpu_min: None },
        }
    }
    pub fn take_sound_peak(&self) -> u16 { match self { Self::Libretro(s) => s.take_sound_peak(), Self::Native(_) => 0 } }
    /// Ends the game, writing its saves. A native core gives the screen back first.
    pub fn quit(self) -> io::Result<()> {
        match self {
            Self::Libretro(s) => s.quit(),
            Self::Native(n) => { let r = n.quit(); SCREEN_LENT.store(false, Relaxed); r.map_err(io::Error::other) }
        }
    }
    /// Ends the game without saving (the card can't be written).
    pub fn abandon(self) {
        match self { Self::Libretro(s) => s.abandon(), Self::Native(n) => { n.abandon(); SCREEN_LENT.store(false, Relaxed); } }
    }
}

/// A native core's answer to a state request: one it doesn't support is said plainly.
fn native_state_error(e: io::Error) -> io::Error {
    if e.kind() == io::ErrorKind::Unsupported { io::Error::new(e.kind(), NATIVE_HAS_NO_STATES) } else { e }
}

/// A game being played.
pub struct Playing {
    pub session: GameSession,
    pub frames: Arc<FrameSlot>,
    /// The file of the save state the menu saves and loads (one slot for now).
    pub state: PathBuf,
    /// Where its launch record is kept, and the record (`LaunchSpec::record`): `watch_game`
    /// rewrites it with the reason before ending Specular.
    pub record: (PathBuf, LaunchRecord),
    /// The output its sound was last sent to; if that couldn't be opened, the game is silent.
    pub output: RefCell<Output>,
    /// Whether `output` couldn't be opened at launch, so the game started silent.
    pub sound_failed: bool,
    /// The buttons held, as libretro joypad bits (a native core's: `native::button_bit`s).
    held: Cell<u32>,
    /// A native core's: the CPU's minimum while it runs (the top step: its games need all of
    /// it), and what sets it (the hardware thread).
    native_cpu: Option<(Option<u32>, Box<dyn Fn(Option<u32>) + Send>)>,
    /// Whether the watchdog ended a native core that had stopped responding (`watch_game`).
    pub stuck: Arc<AtomicBool>,
}

/// A device the UI can use from other threads too (to open sound outputs, which can block).
pub type SharedDevice = Arc<dyn Device + Send + Sync>;

impl Playing {
    /// Starts `game` with `core` (`App::playing_core`), on `bios` (`CoreDecl::starts_on`, recorded
    /// in its states: `LaunchSpec::bios`), given these `options` (`App::core_options`)
    /// and CPU minimum (`App::cpu_min`, set through the hardware thread), with `r`'s data folders,
    /// device, sound output and rumble, on another thread, since reading a
    /// large ROM or opening a Bluetooth output can take a second or more; then calls `started`
    /// there with the result (Err: why, for the player; the details are logged). The caller checks the BIOS first (`CoreDecl::check_bios`). The UI is told of each new frame (`Runner::game_frame`) and each in-game
    /// save, and the app if the core's built-in BIOS halts the game (`Msg::GameStopped`). Before opening the game's output, that thread waits for the interface sounds to close
    /// theirs. A core the user added has the options it declares learned, once the game has
    /// started (`user_cores::learn`).
    pub fn start(r: &Runner, game: &Game, core: &'static CoreDecl, bios: Option<&str>, options: Vec<(String, String)>, cpu_min: CpuMin, autosave: Autosave,
                 started: impl FnOnce(Result<Self, String>) + Send + 'static) {
        if core.native { return Self::start_native(r, game, core, started); }
        let (data, output, rumble) = (&r.data, r.output.borrow().clone(), r.rumble.clone());
        let system = r.app.borrow().ctx().systems.iter().find(|s| s.id == game.system).map_or(game.system, |s| s.name);
        let library = core.user.map_or_else(|| cores_dir().join(format!("{}.so", core.library)), |u| u.path.clone());
        let record = (launch::path(&data.cache()), LaunchRecord::new(core.name, &library, game));
        let spec = LaunchSpec {
            core: library,
            rom: data.roms().join(&game.path),
            save: game.save_path(&data.saves(), "srm"),
            autosave: Some(game.state_path(&data.roms(), "autosave")),
            autosave_when: autosave,
            system_dir: data.bios(),
            options,
            // Only to log how a core differs from its list: a user core's list is what it declared
            // itself, and a change is learned again (`user_cores::learn`) rather than logged.
            option_list: core.user.is_none().then(|| specular_core::settings::core_options::list(core)),
            rom_by_path: core.rom_by_path,
            controller: core.controller,
            record: Some(record.clone()),
            states_restore_save: core.states_restore_save,
            bios: bios.map(str::to_owned),
        };
        let frames = Arc::new(FrameSlot::default());
        // One message to the UI at a time: frames arriving while one is pending are drawn by it.
        let pending = Arc::new(AtomicBool::new(false));
        let on_frame = Box::new(move || {
            if pending.swap(true, Relaxed) { return; }
            let pending = pending.clone();
            crate::on_ui(move |r| { pending.store(false, Relaxed); r.game_frame(); });
        });
        let on_save = Box::new(|| crate::on_ui(|r| r.tell_effects(Event::GameSaved)));
        // The app is told to quit the game, unless another ROM is playing by the time the UI
        // thread has this (the same one quit and started again would be quit too).
        let rom = game.path.clone();
        let on_bios_halt = Box::new(move || {
            let rom = rom.clone();
            crate::on_ui(move |r| if r.playing.borrow().as_ref().is_some_and(|p| p.record.1.game == rom) {
                let why = core.bios_halt_notice();
                eprintln!("game: stopped: {why}");
                crate::post(Msg::GameStopped(why));
            });
        });
        let (vsync, display_hz) = (r.device.open_vsync(), r.profile.refresh_hz);
        let hardware = r.hardware.clone();
        let set_cpu_min = Box::new(move |mhz| { let _ = hardware.send(crate::HwCmd::CpuMin(mhz)); });
        let state = game.state_path(&data.roms(), "state1");
        let (roms, saves, learned, played, device, sounds) = (data.roms(), data.saves(), data.learned_options(), game.clone(), r.device.clone(), r.sounds.clone());
        std::thread::spawn(move || {
            sounds.close();
            let sound = open_sound(device.as_ref(), &output);
            let sound_failed = sound.is_none();
            // The runtime logs failed save writes; this only tells the app of one refused as read-only.
            let write_failed = Box::new(crate::tell_if_read_only);
            let out = Outputs { sound, vsync, display_hz, frames: frames.clone(), on_frame, rumble, on_save, on_bios_halt, write_failed, cpu_min, set_cpu_min };
            // So its saves can follow it if it is renamed (see `library::saves`).
            if let Err(e) = specular_core::library::saves::note_played(&roms, &saves, &played) { eprintln!("saves: {e}"); }
            if let Err(e) = specular_core::library::saves::move_old_state(&roms, &saves, &played) { eprintln!("saves: {e}"); }
            let session = Session::start(spec, out).map_err(|e| { eprintln!("game: {e}"); start_failure(core, system, &e) });
            let declared = session.as_ref().ok().map(|s| s.info().options.clone());
            started(session.map(|session| Self {
                session: GameSession::Libretro(session), frames, state, record, output: RefCell::new(output), sound_failed, held: Cell::new(0),
                native_cpu: None, stuck: Arc::default(),
            }));
            let Some(d) = declared else { return };
            match user_cores::learn(core, &learned, &d) {
                Ok(true) => eprintln!("cores: learned {}'s options ({}), kept in {}", core.name, d.options.len(), learned.display()),
                Err(e) => eprintln!("cores: can't keep {}'s options in {}: {e}", core.name, learned.display()),
                Ok(false) => {}
            }
        });
    }

    /// A button for the game.
    pub fn button(&self, e: &ButtonEvent) {
        let bit = match &self.session {
            GameSession::Libretro(_) => specular_runtime::joypad_bit(e.button).map(u32::from),
            GameSession::Native(_) => native::button_bit(e.button),
        };
        let Some(bit) = bit else { return };
        let held = if e.pressed { self.held.get() | bit } else { self.held.get() & !bit };
        self.set_buttons(held);
    }

    fn set_buttons(&self, held: u32) {
        self.held.set(held);
        match &self.session { GameSession::Libretro(s) => s.set_buttons(held as u16), GameSession::Native(n) => n.set_buttons(held) }
    }

    /// Whether a native core plays it.
    pub fn is_native(&self) -> bool { matches!(self.session, GameSession::Native(_)) }

    /// Pauses or resumes the game. Pausing releases every button, so none stays held in the game.
    ///
    /// A native core gets the screen as it resumes (Specular draws nothing meanwhile:
    /// `SCREEN_LENT`) and gives it back as it pauses, with its picture in `frames`, which is
    /// then drawn under the menu. Pausing it waits for the core (about 50 ms).
    pub fn pause(&self, paused: bool) {
        if paused { self.set_buttons(0); }
        let GameSession::Native(n) = &self.session else {
            if let GameSession::Libretro(s) = &self.session { s.pause(paused); }
            return;
        };
        let set_cpu = |mhz| if let Some((_, set)) = &self.native_cpu { set(mhz) };
        if paused {
            if !n.running() { return; }
            if let Err(e) = n.pause() { eprintln!("game: pausing: {e}"); }
            SCREEN_LENT.store(false, Relaxed);
            set_cpu(None);
            crate::on_ui(|r| r.game_frame());
        } else {
            if n.running() { return; }
            set_cpu(self.native_cpu.as_ref().and_then(|(top, _)| *top));
            SCREEN_LENT.store(true, Relaxed);
            if let Err(e) = n.resume() {
                eprintln!("game: resuming: {e}");
                SCREEN_LENT.store(false, Relaxed);
                set_cpu(None);
                crate::on_ui(|r| r.game_frame());
            }
        }
    }

    /// `start` for a native core (`CoreDecl::native`): its program comes out of the game's
    /// package into the cache (`cache/native/<the ROM's path>/`, also the core's own cache), and
    /// the game's saves go in `saves/<the ROM's path without its extension>/`. The game starts
    /// paused: `pause(false)` hands it the screen.
    fn start_native(r: &Runner, game: &Game, core: &'static CoreDecl, started: impl FnOnce(Result<Self, String>) + Send + 'static) {
        let data = &r.data;
        let package = data.roms().join(&game.path);
        let saves = data.saves().join(game.path.with_extension(""));
        let cache = data.cache().join("native").join(&game.path);
        let record = (launch::path(&data.cache()), LaunchRecord::new(core.name, &cache.join("program"), game));
        let system = r.app.borrow().ctx().systems.iter().find(|s| s.id == game.system).map_or(game.system, |s| s.name);
        let frames = Arc::new(FrameSlot::default());
        let state = game.state_path(&data.roms(), "state1");
        let stuck = Arc::new(AtomicBool::new(false));
        let top = r.cpu_steps.get().and_then(|s| s.last().copied());
        let hardware = r.hardware.clone();
        let output = r.output.borrow().clone();
        let (roms, all_saves, played, device, sounds) = (data.roms(), data.saves(), game.clone(), r.device.clone(), r.sounds.clone());
        // The app is told the game stopped, unless another ROM is playing by the time the UI
        // thread has this.
        let (rom, stuck_seen, stop_hardware) = (game.path.clone(), stuck.clone(), hardware.clone());
        let on_stop = Box::new(move |said: Option<String>| {
            SCREEN_LENT.store(false, Relaxed);
            // A core that ends while running never pauses, which is where the CPU's minimum
            // otherwise goes back to the system's default.
            let _ = stop_hardware.send(crate::HwCmd::CpuMin(None));
            let why = if stuck_seen.load(Relaxed) { "It stopped responding.".to_owned() } else { said.map_or("It closed unexpectedly.".into(), |s| sentence(&s)) };
            crate::on_ui(move |r| {
                r.game_frame(); // Specular has the screen again
                if r.playing.borrow().as_ref().is_some_and(|p| p.record.1.game == rom) {
                    eprintln!("game: stopped: {why}");
                    crate::post(Msg::GameStopped(why));
                }
            });
        });
        let on_save = Box::new(|| crate::on_ui(|r| r.tell_effects(Event::GameSaved)));
        let rumble = r.rumble.clone();
        let on_rumble = Box::new(move |level: u16| if let Some(l) = &rumble { l.set_game(specular_core::device::Motors { strong: level, weak: level }) });
        let clock = native_clock();
        std::thread::spawn(move || {
            sounds.close();
            let sound = open_sound(device.as_ref(), &output);
            let sound_failed = sound.is_none();
            let unpacked = native::package::manifest(&package).and_then(|m| {
                if m.core != core.library { return Err(format!("{}: a package for {}, not {}", package.display(), m.core, core.library)); }
                native::package::unpack_program(&package, &m, &cache)
            });
            let program = match unpacked {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("game: {e}");
                    return started(Err(format!("This isn't a {system} game {} can play. Convert the disc again with Tesseract's converter.", core.name)));
                }
            };
            let launch = NativeLaunch { program, args: vec![], package, saves, cache, clock, options: vec![] };
            let out = NativeOutputs { sound, frames: frames.clone(), on_save, on_rumble, on_stop };
            let session = NativeSession::start(launch, out).map_err(|e| {
                eprintln!("game: {e}");
                match e {
                    StartFailure::Said(why) => sentence(&why),
                    StartFailure::Other(_) => format!("{} couldn't start this game. Restart the device and try again.", core.name),
                }
            });
            let set: Box<dyn Fn(Option<u32>) + Send> = Box::new(move |mhz| { let _ = hardware.send(crate::HwCmd::CpuMin(mhz)); });
            started(session.map(|n| Self {
                session: GameSession::Native(n), frames, state, record, output: RefCell::new(output), sound_failed, held: Cell::new(0),
                native_cpu: Some((top, set)), stuck,
            }));
            // After the start: the first time, this reads the whole package (a disc image, ~1.5 GB:
            // about 20 s from the Brick's card), which held the launch up when it came first.
            if let Err(e) = specular_core::library::saves::note_played(&roms, &all_saves, &played) { eprintln!("saves: {e}"); }
        });
    }
}

/// `s` as a sentence: a capital first letter and a full stop.
fn sentence(s: &str) -> String {
    let s = s.trim().trim_end_matches('.');
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect::<String>() + ".")
}

/// The local time now, in seconds since 2000-01-01 00:00 (a native core's clock).
fn native_clock() -> u64 {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let tm = crate::local(now);
    (now as i64 + tm.tm_gmtoff as i64 - 946_684_800).max(0) as u64
}

/// Opens `output` for a game's sound; None (having said why) if it can't be. May block: a
/// Bluetooth output can take a second.
pub fn open_sound(device: &dyn Device, output: &Output) -> Option<Box<dyn SoundOut>> {
    device.open_sound(output).map_err(|e| eprintln!("sound: opening {output:?}: {e}")).ok()
}

/// Whether `b` goes to the game while it is played (the app keeps Menu, Power and volume).
pub fn for_game(b: Button) -> bool { specular_runtime::joypad_bit(b).is_some() }

/// The GL side: the texture frames go into. Lives in the window's rendering notifier.
#[derive(Default)]
pub struct Screen {
    gl: Option<glow::Context>,
    texture: Option<(glow::Texture, u32, u32)>,
}

impl Screen {
    /// Before Slint draws: uploads the newest of the game's `frames`, if any, into the texture
    /// (making a new one, and showing it, when the size changes); returns whether a frame was
    /// uploaded. With no
    /// game, frees the texture (here, where the GL context is current) and stops showing it.
    pub fn before_render(&mut self, api: &slint::GraphicsAPI, frames: Option<&FrameSlot>, window: &AppWindow) -> bool {
        let slint::GraphicsAPI::NativeOpenGL { get_proc_address } = api else { return false };
        let gl = self.gl.get_or_insert_with(|| unsafe { glow::Context::from_loader_function_cstr(|n| get_proc_address(n)) });
        let Some(frames) = frames else {
            if let Some((t, _, _)) = self.texture.take() {
                window.set_game_frame(slint::Image::default());
                unsafe { gl.delete_texture(t) };
            }
            return false;
        };
        let Some(frame) = frames.take() else { return false };
        unsafe {
            if self.texture.is_none_or(|(_, w, h)| (w, h) != (frame.width, frame.height)) {
                if let Some((t, _, _)) = self.texture.take() { gl.delete_texture(t); }
                let Ok(t) = gl.create_texture() else { frames.give_back(frame); return false };
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA as i32, frame.width as i32, frame.height as i32, 0,
                    glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
                // Sharp pixels when scaled up.
                for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] { gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32); }
                for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] { gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32); }
                self.texture = Some((t, frame.width, frame.height));
                let image = slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(t.0, (frame.width, frame.height).into()).build();
                window.set_game_frame(image);
            }
            let (t, w, h) = self.texture.unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&frame.rgba)));
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        frames.give_back(frame);
        true
    }
}
