//! Specular's UI: draws the core's screens with Slint and carries out the app's effects. All
//! slow work happens off the UI thread: files on an I/O thread (and the library scan on its
//! own), hardware (with file transfer and SSH) on a hardware thread, each radio on a thread of
//! its own (`radio_thread`), the time zone on its own (`timezone`), box art on the art service's
//! threads (`art`), interface sounds on a thread of their own (`sounds`), and opening a game's
//! sound outputs on short-lived threads; their results come back as messages to the app, or as
//! calls on the UI thread.

mod art;
mod frames;
mod game;
mod keys;
mod sounds;
mod timezone;
mod watch;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Model, SharedPixelBuffer, VecModel};
use specular_core::art::ArtKind;
use specular_core::card::{Card, CardCmd};
use specular_core::app::{App, Credit, Credits, Effect, InputTarget, LeftOut, LocalTime, Msg, SavedState, WifiNeed};
use specular_core::device::{Accent, Audio, Device, DeviceProfile, Hardware, Motors, Output, Rgb, Rumble, SshCmd};
use specular_core::effects::{self, Effects, Event};
use specular_core::rumble::RumbleLine;
use specular_core::library::{self, Game, SystemDecl, user_cores};
use specular_core::input::{Button, ButtonEvent};
use specular_core::launch::{self, LaunchRecord};
use specular_core::list;
use specular_core::list::Rect;
use specular_core::palette;
use specular_core::model::{Icon, ImageRef, Layout, Row, RowKind, ScreenView};
use specular_core::wireless::bluetooth::BtKind;
use specular_core::nav::Transition;
use specular_core::settings::{self, Layer, builtin, store};
use specular_core::timezone::ZoneEvent;
use specular_core::wireless::Wireless;
use specular_core::wireless::wifi::WifiLink;

slint::include_modules!();

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// How Specular names itself to web servers (box art, the time zone lookup).
const USER_AGENT: &str = concat!("Specular/", env!("CARGO_PKG_VERSION"), " (retro game launcher)");

/// Installs a device's Slint platform (how it draws and how its buttons arrive). It is given the
/// callback for button events, which the platform must call on the UI thread.
pub type InstallPlatform = Box<dyn FnOnce(Box<dyn FnMut(ButtonEvent)>) -> Result<(), Error>>;

/// Now, on the monotonic clock button times use.
pub fn now() -> Duration { clock(libc::CLOCK_MONOTONIC) }

/// The CPU time the calling thread has used.
fn cpu_time() -> Duration { clock(libc::CLOCK_THREAD_CPUTIME_ID) }

fn clock(id: libc::clockid_t) -> Duration {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(id, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// Gives Slint the fonts to fall back on for characters Geist lacks: Specular's emoji font
/// (network and device names have emoji), then the font files in the SD card's `fonts/` (for
/// scripts such as Japanese; see docs/look-and-motion.md). Slint looks for fallback fonts only
/// among the system's and those in $SLINT_FONT_PATH (files and folders, separated as in $PATH),
/// not among fonts built into the UI, so the emoji font ships as a file rather than built in:
/// `fonts/` next to the binary, or the source tree's NotoEmoji.ttf in a development build. A
/// $SLINT_FONT_PATH already set is used instead of the emoji font (the card's fonts are still
/// added). Slint reads the fonts once, as it starts, so this must run before that, and before any
/// other thread starts (it sets an environment variable). Returns the card's files it doesn't use,
/// and why.
fn use_fallback_fonts(data: &DataRoot) -> Vec<user_cores::TurnedAway> {
    let own = match std::env::var_os("SLINT_FONT_PATH") {
        Some(set) => std::env::split_paths(&set).collect(),
        None => {
            let installed = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("fonts"))).filter(|d| d.is_dir());
            vec![installed.unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/NotoEmoji.ttf").into())]
        }
    };
    let (card, turned_away) = card_fonts(&data.fonts());
    let paths: Vec<PathBuf> = own.into_iter().chain(card).collect();
    match std::env::join_paths(&paths) {
        // SAFETY: before any other thread exists (see above).
        Ok(list) => unsafe { std::env::set_var("SLINT_FONT_PATH", list) },
        // Only a $SLINT_FONT_PATH already set can hold a ':' (`card_fonts` leaves such paths out).
        Err(e) => eprintln!("fonts: can't list the fallback fonts ({e}): {paths:?}"),
    }
    turned_away
}

/// The font files in `dir` (the SD card's `fonts/`), sorted by name, and the other files there, with
/// why each can't be used (Slint would skip them without a word); both logged. The folder's guide
/// (`guides::README`) is left out of both.
fn card_fonts(dir: &Path) -> (Vec<PathBuf>, Vec<user_cores::TurnedAway>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Default::default(),
        Err(e) => { eprintln!("fonts: can't read {}: {e}", dir.display()); return Default::default() }
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| !p.ends_with(specular_core::guides::README)).collect();
    paths.sort();
    let (mut used, mut turned_away) = (Vec::new(), Vec::new());
    for path in paths {
        match font_size(&path) {
            Ok(size) => used.push((path, size)),
            Err(why) => {
                eprintln!("fonts: not using {}: {why}", path.display());
                turned_away.push(user_cores::TurnedAway { file: path.file_name().unwrap_or_default().to_string_lossy().into_owned(), why });
            }
        }
    }
    // Slint reads each one whole into memory, so their sizes count towards Specular's.
    let list: Vec<String> = used.iter().map(|(p, size)| format!("{} ({:.1} MB)", p.file_name().unwrap_or_default().to_string_lossy(), *size as f64 / 1e6)).collect();
    if list.is_empty() { eprintln!("fonts: {} has no fonts in it", dir.display()); } else { eprintln!("fonts: from {}: {}", dir.display(), list.join(", ")); }
    (used.into_iter().map(|(p, _)| p).collect(), turned_away)
}

/// The size of the font file at `path`, or why Slint can't use it: it reads only TrueType and
/// OpenType fonts (and collections of them), and only files directly in the folder.
fn font_size(path: &Path) -> Result<u64, String> {
    use std::io::Read;
    if path.is_dir() { return Err("fonts in folders aren't looked for".into()); }
    if path.as_os_str().as_encoded_bytes().contains(&b':') { return Err("its path has a ':' in it, which $SLINT_FONT_PATH can't hold".into()); }
    let mut file = std::fs::File::open(path).map_err(|e| format!("can't open it: {e}"))?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic).map_err(|e| format!("can't read it: {e}"))?;
    if !matches!(&magic, b"\0\x01\0\0" | b"OTTO" | b"true" | b"ttcf") { return Err("it isn't a TrueType or OpenType font".into()); }
    file.metadata().map(|m| m.len()).map_err(|e| format!("can't read it: {e}"))
}

/// Where Specular's data lives: the SD card's folders.
#[derive(Clone, Debug)]
pub struct DataRoot(pub PathBuf);

impl DataRoot {
    pub fn roms(&self) -> PathBuf { self.0.join("roms") }
    /// The settings files, in the folder of their current format.
    pub fn config(&self) -> PathBuf { settings::migrate::config_dir(&self.0) }
    pub fn cache(&self) -> PathBuf { self.0.join("cache") }
    pub fn saves(&self) -> PathBuf { self.0.join("saves") }
    pub fn bios(&self) -> PathBuf { self.0.join("bios") }
    /// Cores the user adds (see `user_cores`).
    pub fn cores(&self) -> PathBuf { self.0.join("cores") }
    /// The options learned from the cores the user added (`user_cores::learn`).
    pub fn learned_options(&self) -> PathBuf { self.cache().join("cores") }
    /// Fonts the user adds, for characters Specular's own fonts lack.
    pub fn fonts(&self) -> PathBuf { self.0.join("fonts") }
}

/// Runs Specular on `device` until the event loop ends. `platform` sets up the device's platform;
/// without one, Slint's default window is used and keyboard keys act as buttons (see `KEYS`)
/// and the headphone jack (`HEADPHONES_KEY`).
/// The device is also used from other threads, to open sound outputs (which can block).
/// `credits` are what the binary ships (worked out by its build script; see specular-credits).
/// Must be called before any other thread starts (see `use_fallback_fonts`).
pub fn run(device: Box<dyn Device + Send + Sync>, data: DataRoot, platform: Option<InstallPlatform>, credits: &'static [Credit]) -> Result<(), Error> {
    let fonts_turned_away = use_fallback_fonts(&data);
    quit_on_signals();
    let profile = device.profile();
    let keyboard = platform.is_none();
    if let Some(platform) = platform {
        platform(Box::new(|e| post(Msg::Button(e))))?;
    }
    let window = AppWindow::new()?;
    let theme = window.global::<Theme>();
    theme.set_u(profile.ppi as f32 / 400.0);
    window.window().set_size(slint::PhysicalSize::new(profile.width, profile.height));

    let (io, io_thread) = io_thread(data.clone());
    let systems = playable_systems();
    // The radios and the headphone jack get threads of their own; the rest of the hardware goes
    // to the hardware thread.
    let mut hardware = device.hardware();
    let device: game::SharedDevice = Arc::from(device);
    let (wifi, bluetooth, installer, time_zone) = (hardware.wifi.take(), hardware.bluetooth.take(), hardware.installer.take(), hardware.time_zone.take());
    let card = match hardware.card.take() {
        Some(service) => Some(radio_thread("card", service, Msg::Card)),
        None => {
            eprintln!("card: this device doesn't report its SD card; it counts as always in and writable");
            post(Msg::Card(Card::mounted()));
            None
        }
    };
    match hardware.headphones.take() {
        Some(jack) => { std::thread::spawn(move || jack.watch(Box::new(|plugged| post(Msg::Headphones(plugged))))); }
        None => eprintln!("input: this device has no headphone detection; headphones get the speaker's volume"),
    }
    let rumble = hardware.rumble.take().map(rumble_thread);
    if rumble.is_none() { eprintln!("rumble: this device has none"); }
    let mut app = App::new(systems.clone(), settings::declarations(&systems));
    app.set_fonts_turned_away(fonts_turned_away);
    app.has_radios(wifi.is_some(), bluetooth.is_some());
    app.has_installer(installer.is_some());
    app.has_file_transfer(hardware.file_transfer.is_some());
    app.has_ssh(hardware.ssh.is_some());
    let cpu_steps = Arc::new(OnceLock::new());
    let (hardware, hardware_thread) = hardware_thread(hardware, cpu_steps.clone());
    app.has_time_zone(time_zone.is_some());
    // Beside the binary, where `make install` puts it (and the build does, for development).
    let texts = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("licences.txt"))).filter(|p| p.is_file());
    if texts.is_none() { eprintln!("credits: there's no licences.txt beside the binary"); }
    app.set_credits(Credits { projects: credits, texts });
    // Without WiFi, the network (if any) is assumed to be there.
    let online = wifi.is_none();
    // Before the art threads start, so neither misses box art not yet in cache/art/boxart/.
    specular_core::art::move_old_cache(&data.cache());
    // Every system Specular declares: one may get a core from the card later.
    let art_worker = art::worker(data.roms(), data.cache(), library::builtin_systems(), vec![Box::new(art::LibretroThumbnails::new())], online,
        |rom| on_ui(move |r| r.art_stored(rom)),
        |wanted| post(Msg::WifiNeed { need: WifiNeed::BoxArt, wanted, time: now() }));
    let art_loader = art::loader(data.roms(), data.cache(),
        |rom, kind, picture| on_ui(move |r| r.art_loaded((rom, kind), picture.map(slint::Image::from_rgba8))));
    let runner = Rc::new(Runner {
        app: RefCell::new(app),
        window: window.as_weak(),
        rows: Rc::new(VecModel::default()),
        hints: Rc::new(VecModel::default()),
        move_hints: Rc::new(VecModel::default()),
        status_icons: Rc::new(VecModel::default()),
        profile,
        hardware,
        cpu_steps,
        wifi: wifi.map(|s| radio_thread("wifi", s, Msg::Wifi)),
        bluetooth: bluetooth.map(|s| radio_thread("bluetooth", s, Msg::Bluetooth)),
        installer: installer.map(installer_thread),
        time_zone: time_zone.map(timezone::thread),
        card,
        io,
        reading: Cell::new(0),
        data,
        sounds: sounds::Sounds::start(device.clone()),
        device,
        output: RefCell::new(Output::Speaker),
        sound_requests: Cell::new(0),
        playing: RefCell::default(),
        game_screen: RefCell::default(),
        repeat: slint::Timer::default(),
        sleep_timer: slint::Timer::default(),
        wifi_timer: slint::Timer::default(),
        launches: Cell::new(0),
        clock: slint::Timer::default(),
        ignore_power_until: Cell::new(Duration::ZERO),
        shown_layout: Cell::new(None),
        slots_changed: Cell::new(0),
        slots: Cell::new((0, 0, 0)),
        repeating: Cell::new(false),
        art: RefCell::default(),
        picture: RefCell::default(),
        picture_timer: slint::Timer::default(),
        toast_timer: slint::Timer::default(),
        volume_timer: slint::Timer::default(),
        letter_timer: slint::Timer::default(),
        launched: Cell::new(None),
        splash_timer: slint::Timer::default(),
        tint: Cell::new(None),
        stats_timer: slint::Timer::default(),
        frame_times: RefCell::default(),
        rumble,
        effects: RefCell::new(Effects::new(effects::builtin())),
        effects_timer: slint::Timer::default(),
        effects_due: Cell::new(None),
        lights_ready: Cell::new(false),
        lights_shown: Cell::new(None),
        art_worker,
        art_online: Cell::new(online),
        art_loader,
        on_screen: RefCell::default(),
        requested: RefCell::default(),
        art_kinds: RefCell::default(),
    });
    runner.show_theme();
    window.set_rows(runner.rows.clone().into());
    window.set_hints(runner.hints.clone().into());
    window.set_move_hints(runner.move_hints.clone().into());
    window.set_status_icons(runner.status_icons.clone().into());
    window.window().set_rendering_notifier(|state, api| {
        let Some(r) = self::runner() else { return };
        match state {
            slint::RenderingState::BeforeRendering => r.before_render(api),
            slint::RenderingState::AfterRendering => r.after_render(),
            _ => {}
        }
    }).map_err(|e| format!("this window can't show games: {e:?}"))?;
    RUNNER.with(|r| *r.borrow_mut() = Some(runner.clone()));
    if keyboard {
        let headphones = Cell::new(false);
        window.on_key(move |text, pressed| match key_button(&text) {
            Some(button) => { post(Msg::Button(ButtonEvent { button, pressed, time: now() })); true }
            None if text.to_lowercase() == HEADPHONES_KEY => {
                if pressed {
                    headphones.set(!headphones.get());
                    eprintln!("input: simulated headphones {}", if headphones.get() { "plugged in" } else { "unplugged" });
                    post(Msg::Headphones(headphones.get()));
                }
                true
            }
            None => false,
        });
    }
    runner.tick_clock();
    runner.refresh(None);
    let _script = std::env::var("SPECULAR_SCRIPT").ok().map(|s| script(&window, &s));
    window.run()?;
    // A game being played writes its battery save as it ends.
    if let Some(p) = runner.playing.borrow_mut().take() && let Err(e) = p.session.quit() { eprintln!("game: saving: {e}"); }
    // Dropping the runner closes the I/O and hardware threads' queues; wait for the last saves,
    // and for file transfer to stop.
    RUNNER.with(|r| r.borrow_mut().take());
    drop(runner);
    let _ = io_thread.join();
    let _ = hardware_thread.join();
    Ok(())
}

/// The systems Specular can play with its own cores, until the card's are found (see `io_thread`).
/// Other systems' games aren't listed at all, rather than listed and failing to start (see
/// docs/library-and-box-art.md).
fn playable_systems() -> Vec<SystemDecl> { user_cores::playable(library::builtin_systems(), &[]) }

/// Exits if the game's core gets stuck (see `Session::watch`): its thread can't be stopped, so
/// the only way out is to end Specular, which the service manager then starts again. The battery
/// save is as the game last wrote it, at most about 10 s old. The launch record, left behind,
/// says why, for the notice after the restart. The rumble stops at once, rather than run on until
/// Specular has ended.
fn watch_game(p: &game::Playing, rumble: Option<RumbleLine>) {
    let (path, mut record) = p.record.clone();
    p.session.watch(STUCK_LIMIT, move || {
        eprintln!("game: the core has spent {} s on one frame; it has stopped responding, restarting", STUCK_LIMIT.as_secs());
        if let Some(line) = rumble { line.set_level(0); } // for good: effects still run until the exit
        record.reason = Some("it stopped responding".into());
        if let Err(e) = record.write(&path) { eprintln!("game: launch record {}: {e}", path.display()); }
        on_ui(|r| r.toast("The game stopped responding. Restarting…"));
        std::thread::sleep(Duration::from_millis(1500));
        std::process::exit(2);
    });
}

/// Makes SIGTERM (a service manager stopping Specular) and SIGINT (Ctrl-C) end the event loop
/// like a normal quit, so the game's battery save and the settings are written on the way out.
/// The signals are blocked in every thread (this runs before any other thread starts, and threads
/// inherit it) and taken by one thread that waits for them, where it is safe to do anything.
fn quit_on_signals() {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTERM);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        std::thread::spawn(move || loop {
            let mut sig = 0;
            if libc::sigwait(&set, &mut sig) == 0 {
                // A service manager stopping Specular may already have closed the pipe it reads
                // Specular's messages from, and eprintln! panics when stderr can't be written:
                // send them to /dev/null instead, so the way out (saving the game) can't panic.
                use std::io::Write;
                if writeln!(std::io::stderr(), "specular: signal {sig}, quitting").is_err() {
                    let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
                    if null >= 0 { libc::dup2(null, 2); libc::close(null); }
                }
                let _ = slint::invoke_from_event_loop(|| { let _ = slint::quit_event_loop(); });
            }
        });
    }
}

thread_local! {
    /// The running UI, reachable from messages posted by other threads.
    static RUNNER: RefCell<Option<Rc<Runner>>> = RefCell::default();
}

/// The running UI (on the UI thread; None on others, and once it has stopped).
fn runner() -> Option<Rc<Runner>> { RUNNER.with(|r| r.borrow().clone()) }

/// Runs `f` with the running UI on the UI thread, from any thread.
fn on_ui(f: impl FnOnce(&Rc<Runner>) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || if let Some(r) = runner() { f(&r) });
}

/// Arms `timer` to run `f` with the running UI after `delay`, replacing what it was armed for.
fn after(timer: &slint::Timer, delay: Duration, f: impl Fn(&Rc<Runner>) + 'static) {
    timer.start(slint::TimerMode::SingleShot, delay, move || if let Some(r) = runner() { f(&r) });
}

/// `after`, at `due` (on `now()`'s clock); stops `timer` if nothing is due.
fn at(timer: &slint::Timer, due: Option<Duration>, f: impl Fn(&Rc<Runner>) + 'static) {
    match due { Some(due) => after(timer, due.saturating_sub(now()), f), None => timer.stop() }
}

/// Delivers a message to the app on the UI thread, from any thread.
fn post(msg: Msg) { on_ui(move |r| r.handle(msg)); }

enum HwCmd {
    Brightness(u8),
    Lights(Vec<Rgb>),
    /// Sets the level of the outputs whose volume is `setting` (`Output::volume_setting`).
    Volume { setting: &'static str, level: u8 },
    Muted(bool),
    /// Sound goes to this output now (which levels that sets: `Audio::set_volume`).
    Output(Output),
    Sleep,
    PowerOff,
    Restart,
    /// Start (true) or stop the file transfer server.
    FileTransfer(bool),
    Ssh(SshCmd),
    /// Set the CPU's minimum speed (MHz; None: the system's default; see `device::Cpu`).
    CpuMin(Option<u32>),
}

/// The volume as the hardware thread keeps it: the level for each volume setting
/// (`Output::volume_setting`), with mute held separately, since muting sets the hardware to 0
/// without losing the level. Each change sets the levels `Audio::set_volume` asks for.
struct Volume { levels: HashMap<&'static str, u8>, output: Output, muted: bool }

impl Volume {
    fn set_level(&mut self, audio: &dyn Audio, setting: &'static str, level: u8) -> std::io::Result<()> {
        self.levels.insert(setting, level);
        self.write(audio, |o| o.volume_setting() == setting)
    }

    fn set_muted(&mut self, audio: &dyn Audio, muted: bool) -> std::io::Result<()> {
        self.muted = muted;
        self.write(audio, |_| true)
    }

    fn set_output(&mut self, audio: &dyn Audio, output: Output) -> std::io::Result<()> {
        self.output = output;
        self.write(audio, |o| matches!(o, Output::Bluetooth(_)))
    }

    /// Sets the levels of the outputs `which` picks, out of the speaker, the headphones and the
    /// current output if it is Bluetooth: each its level, or 0 while muted or before its setting
    /// is known. Sets all of them even if one fails, and returns the first error.
    fn write(&self, audio: &dyn Audio, which: impl Fn(&Output) -> bool) -> std::io::Result<()> {
        let bluetooth = matches!(self.output, Output::Bluetooth(_)).then(|| self.output.clone());
        [Output::Speaker, Output::Headphones].into_iter().chain(bluetooth).filter(which).fold(Ok(()), |result, output| {
            let level = if self.muted { 0 } else { self.levels.get(output.volume_setting()).copied().unwrap_or(0) };
            eprintln!("sound: {output:?} volume {level}{}", if self.muted { " (muted)" } else { "" });
            result.and(audio.set_volume(&output, level))
        })
    }
}

/// Tells the app how the device's last run ended, if not cleanly (`Power::last_shutdown`),
/// logging what the system recorded. A word Specular doesn't know is reported as `Unexpected`, as
/// the system's contract says (a newer system's reason for an end that wasn't clean).
fn tell_last_shutdown(power: &dyn specular_core::device::Power) {
    use specular_core::device::LastShutdown;
    match power.last_shutdown() {
        Ok(None) => eprintln!("power: no shutdown record: the last run ended cleanly"),
        Ok(Some(word)) => {
            let known = LastShutdown::parse(&word);
            let how = known.unwrap_or(LastShutdown::Unexpected);
            let unknown = if known.is_none() { ", a word Specular doesn't know" } else { "" };
            eprintln!("power: the last run didn't end cleanly: {word:?}{unknown}, reported as {how:?}");
            post(Msg::LastShutdown(how));
        }
        Err(e) => eprintln!("power: can't read how the last run ended: {e}"),
    }
}

/// How often the battery is read (on some devices each read is a slow bus transfer).
const BATTERY_INTERVAL: Duration = Duration::from_secs(10);

/// The hardware thread: carries out commands, and reads the battery every `BATTERY_INTERVAL`.
/// It ends once the sender is dropped. File transfer is stopped when it starts and when it ends,
/// so the SD card isn't left open to the network by a run of Specular that quit or crashed with
/// the file transfer screen open. Commands are carried out in order, so a stop sent before a
/// sleep is done before it. As it starts, it readies the CPU (`Cpu::prepare`) and puts its speed
/// steps in `cpu_steps` (left empty if it can't).
fn hardware_thread(hw: Hardware, cpu_steps: Arc<OnceLock<Vec<u32>>>) -> (mpsc::Sender<HwCmd>, std::thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        stop_transfer(&hw);
        match hw.cpu.as_ref().map(|c| c.prepare()) {
            Some(Ok(steps)) => { eprintln!("cpu: speed steps {steps:?} MHz"); let _ = cpu_steps.set(steps); }
            Some(Err(e)) => eprintln!("cpu: no minimum speed can be set: {e}"),
            None => eprintln!("cpu: this device's speed can't be set"),
        }
        tell_last_shutdown(&*hw.power);
        let mut next_battery = Instant::now();
        let mut volume = Volume { levels: HashMap::new(), output: Output::Speaker, muted: false };
        loop {
            if Instant::now() >= next_battery {
                if let Ok(b) = hw.power.battery() { post(Msg::Battery(b)); }
                next_battery = Instant::now() + BATTERY_INTERVAL;
            }
            let result = match rx.recv_timeout(next_battery.saturating_duration_since(Instant::now())) {
                Ok(HwCmd::Brightness(l)) => hw.display.set_brightness(l),
                Ok(HwCmd::Lights(c)) => hw.lights.show(&c),
                Ok(HwCmd::Volume { setting, level }) => volume.set_level(&*hw.audio, setting, level),
                Ok(HwCmd::Muted(m)) => volume.set_muted(&*hw.audio, m),
                Ok(HwCmd::Output(o)) => volume.set_output(&*hw.audio, o),
                Ok(HwCmd::Sleep) => {
                    let r = hw.power.sleep();
                    post(Msg::Woke);
                    next_battery = Instant::now(); // it may have changed a lot
                    r
                }
                Ok(HwCmd::PowerOff) => hw.power.shutdown(),
                Ok(HwCmd::Restart) => hw.power.restart(),
                Ok(HwCmd::FileTransfer(start)) => {
                    if let Some(t) = &hw.file_transfer {
                        let result = if start { t.start() } else { t.stop() };
                        if let Err(e) = &result { eprintln!("transfer: {}: {e}", if start { "starting" } else { "stopping" }); }
                        post(Msg::FileTransfer { start, result });
                    }
                    Ok(())
                }
                Ok(HwCmd::CpuMin(mhz)) => {
                    if let Some(cpu) = &hw.cpu && let Err(e) = cpu.set_min(mhz) { eprintln!("cpu: setting the minimum to {}: {e}", specular_core::cpu::text(mhz)); }
                    Ok(())
                }
                Ok(HwCmd::Ssh(cmd)) => {
                    if let Some(ssh) = &hw.ssh {
                        let result = ssh.send(cmd);
                        eprintln!("ssh: {cmd:?}: {result:?}");
                        post(Msg::Ssh { cmd, result });
                    }
                    Ok(())
                }
                Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
                Err(mpsc::RecvTimeoutError::Disconnected) => return stop_transfer(&hw),
            };
            if let Err(e) = result { eprintln!("hardware: {e}"); }
        }
    });
    (tx, handle)
}

fn stop_transfer(hw: &Hardware) {
    if let Some(t) = &hw.file_transfer && let Err(e) = t.stop() { eprintln!("transfer: stopping: {e}"); }
}

/// A radio's thread: it owns the radio's service, starts it (its events become messages to the
/// app, `msg`), then carries out the commands sent, in order. Commands can block for seconds
/// (loading a driver), which holds up only the commands after them.
fn radio_thread<C: Send + 'static, E: Send + 'static>(name: &str, mut service: Box<dyn Wireless<C, E>>, msg: fn(E) -> Msg) -> mpsc::Sender<C> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        service.start(Box::new(move |e| post(msg(e))));
        for cmd in rx { service.send(cmd); }
    });
    if let Err(e) = spawned { eprintln!("{name}: can't start its thread: {e}"); }
    tx
}

/// The installer's thread: first has it follow a task an earlier run of Specular left running,
/// and asks it what can be done (so a prepared restore is asked about at start-up); then asks
/// again and starts its tasks when told (their progress is reported from a thread of their own).
fn installer_thread(installer: Box<dyn specular_core::install::Installer>) -> mpsc::Sender<specular_core::app::InstallCmd> {
    use specular_core::app::InstallCmd;
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("installer".into()).spawn(move || {
        installer.follow(Box::new(|p| post(Msg::InstallProgress(p))));
        post(Msg::InstallOffer(installer.offer()));
        for cmd in rx {
            match cmd {
                InstallCmd::Offer => post(Msg::InstallOffer(installer.offer())),
                InstallCmd::Start(task) => installer.start(task, Box::new(|p| post(Msg::InstallProgress(p)))),
            }
        }
    });
    if let Err(e) = spawned { eprintln!("install: can't start the installer's thread: {e}"); }
    tx
}

/// The rumble's thread: runs the motors as the line says, skipping to the latest state when it
/// falls behind (setting the motors can block). Each different error is logged once; the first
/// success after errors logs "working again".
fn rumble_thread(mut rumble: Box<dyn Rumble>) -> RumbleLine {
    let line = RumbleLine::default();
    let wanted = line.clone();
    let spawned = std::thread::Builder::new().name("rumble".into()).spawn(move || {
        let (mut motors, mut failing) = (Motors::STILL, None);
        loop {
            motors = wanted.next(motors);
            match rumble.set(motors) {
                Ok(()) => if failing.take().is_some() { eprintln!("rumble: working again"); },
                Err(e) => { let e = e.to_string(); if failing.as_ref() != Some(&e) { eprintln!("rumble: {e}"); failing = Some(e); } }
            }
        }
    });
    if let Err(e) = spawned { eprintln!("rumble: can't start its thread: {e}"); }
    line
}

/// What the I/O thread is asked to do.
enum Io {
    /// Read the card afresh, as reading number `reading` (`Effect::ReadCard`).
    Read { reading: u32, writable: bool },
    /// Stop reading the card (`Effect::ForgetCard`).
    Forget,
    /// Write a settings file.
    Save(PathBuf, serde_json::Map<String, serde_json::Value>),
    /// A game was ended without saving (`Effect::AbandonGame`), so its launch record (at the
    /// path) is likely still on its card: it is removed when that card can next be written, or
    /// the next start would report the game as stopped unexpectedly.
    Abandoned(PathBuf, LaunchRecord),
    /// List the `.bad` settings files again (`Effect::CheckSetAside`), tagged with the card's
    /// reading then, so the answer is dropped if the card is read afresh or forgotten meanwhile.
    CheckSetAside(u32),
}

/// The I/O thread: reads the card when asked (`read_card`), and saves settings. It works in the
/// order asked, so the card's settings files are read before any save asked for after, and saves
/// asked for before reach the card they were made for. Saves queued together are merged, so
/// holding a button that changes a value writes the file once rather than once per step.
fn io_thread(data: DataRoot) -> (mpsc::Sender<Io>, std::thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel::<Io>();
    let handle = std::thread::spawn(move || {
        // The card's scan, whether settings may be written to its config folder, and whether the
        // next reading is the run's first (see `read_card`).
        let (mut scan, mut writable, mut first) = (None::<watch::Stop>, false, true);
        let (mut saves, mut abandoned) = (HashMap::new(), None::<(PathBuf, LaunchRecord)>);
        while let Ok(cmd) = rx.recv() {
            for cmd in std::iter::once(cmd).chain(rx.try_iter()) {
                match cmd {
                    Io::Save(path, values) => { saves.insert(path, values); continue; }
                    Io::Abandoned(path, record) => { abandoned = Some((path, record)); continue; }
                    Io::CheckSetAside(reading) => { post_reading(reading, Msg::SetAside(store::set_aside(&data.config()))); continue; }
                    _ => {}
                }
                store_settings(&mut saves, writable);
                if let Some(s) = scan.take() { s.stop(); }
                (scan, writable) = match cmd {
                    Io::Read { reading, writable } => read_card(&data, reading, writable, std::mem::take(&mut first)),
                    _ => (None, false),
                };
                if writable && let Some((path, record)) = &abandoned {
                    match record.remove_if_this(path) {
                        Ok(true) => { eprintln!("game: removed the launch record of the game ended without saving"); abandoned = None; }
                        Ok(false) => {} // another card: kept for when that game's is back
                        Err(e) => wrote_to_card(&format!("game: removing the launch record {}", path.display()), &e),
                    }
                }
            }
            store_settings(&mut saves, writable);
        }
    });
    (tx, handle)
}

/// Writes the settings files in `saves` (emptying it), if the card's config folder can take them.
fn store_settings(saves: &mut HashMap<PathBuf, serde_json::Map<String, serde_json::Value>>, writable: bool) {
    if !writable || saves.is_empty() { saves.clear(); return; }
    // One report for the batch: a failure isn't hidden by another file written after it.
    let mut result = Ok(());
    for (path, values) in saves.drain() {
        if let Err(e) = store::store_layer(&path, &values) {
            wrote_to_card(&format!("settings: {}", path.display()), &e);
            result = result.and(Err(format!("{}: {e}", path.display())));
        }
    }
    post(Msg::SettingsSaved(result));
}

/// Reads the card, as reading number `reading`: brings its config folder up to date (only if
/// `writable`), loads its settings files, then starts its scan (`scan_thread`). Returns what
/// stops the scan (None if the card can't be watched: then it scans once), and whether settings
/// can be written to the card. `first`: whether this is the run's first reading; only that one
/// reports the launch record an earlier run left behind.
fn read_card(data: &DataRoot, reading: u32, writable: bool, first: bool) -> (Option<watch::Stop>, bool) {
    // False if the card isn't writable or migrating failed: then nothing is written to the config
    // folder (see `migrate`), and changes stay in memory.
    let migrated = writable && settings::migrate::migrate(&data.0);
    if writable && !migrated { on_ui(|r| r.toast("Changes to settings can't be saved on the SD card, so they last only until the device turns off. Restarting it tries again.")); }
    let (config, systems) = (data.config(), playable_systems());
    let decls = settings::declarations(&systems);
    // The global layer, then each system's (any may get a core from the card) and each of
    // Specular's cores'; the card's cores' are loaded as they are found, and games' with the
    // library.
    let cores = library::cores(&systems).into_iter().map(|c| Layer::Core(c.id().into()));
    for layer in [Layer::Global].into_iter().chain(library::builtin_systems().iter().map(|s| Layer::System(s.id.into()))).chain(cores) {
        post_reading(reading, Msg::SettingsLoaded(layer.clone(), load_layer(&config, &decls, &layer)));
    }
    // Before the scan, so no game can have been launched (writing a new record) yet.
    if first && let Some(r) = LaunchRecord::left_behind(&launch::path(&data.cache())) { post_reading(reading, Msg::LastGameStopped(r)); }
    let watch = watch::CardWatch::new().map_err(|e| eprintln!("library: can't watch for new games or cores: {e}")).ok();
    let stop = watch.as_ref().map(watch::CardWatch::stopper);
    let data = data.clone();
    std::thread::spawn(move || scan_thread(data, reading, migrated, watch));
    (stop, migrated)
}

/// Delivers a message from reading number `reading` of the card, unless the card has been read
/// afresh or forgotten since (see `Runner::reading`).
fn post_reading(reading: u32, msg: Msg) { on_ui(move |r| if r.reading.get() == reading { r.handle(msg) }); }

/// Logs `<what>: <e>` for a failed write to the card, and tells the app if the card refused it
/// as read-only (`Msg::CardReadOnly`).
fn wrote_to_card(what: &str, e: &std::io::Error) {
    eprintln!("{what}: {e}");
    tell_if_read_only(e);
}

/// Tells the app if a write to the card failed because it is read-only (`Msg::CardReadOnly`).
fn tell_if_read_only(e: &std::io::Error) {
    if e.raw_os_error() == Some(libc::EROFS) { post(Msg::CardReadOnly); }
}

/// The library scan of reading number `reading` of the card, on a thread of its own: looks for
/// the card's cores, then scans the games and moves the saves of renamed ROMs; again whenever the
/// files on the card change (`watch`), until it is stopped. Schemas and guides are written, and
/// `cache/cores/` made, only if the config folder is `writable` (see `read_card`).
fn scan_thread(data: DataRoot, reading: u32, writable: bool, watch: Option<watch::CardWatch>) {
    let (config, roms, saves, cores_dir, learned) = (data.config(), data.roms(), data.saves(), data.cores(), data.learned_options());
    // The games whose settings files are loaded, and the files found to match no game (logged).
    let (mut loaded, mut logged_orphans) = (HashSet::new(), HashSet::new());
    let (mut on_card, mut found, mut cores_loaded) = (user_cores::UserCores::default(), None, HashSet::new());
    let all = library::builtin_systems();
    // Set at the first look: `found` starts as None, so it always counts as a change. `every`
    // has the systems no core plays too, whose games are counted rather than listed.
    let (mut every, mut decls) = (Vec::new(), Vec::new());
    let mut no_emulator = Vec::new(); // as last logged
    loop {
        // `cache/cores/` is made beforehand so it can be watched: options learned are written
        // there as a core plays its first game, and become settings at the next look. (Only on
        // a writable card.)
        if writable && let Err(e) = std::fs::create_dir_all(&learned) { wrote_to_card(&format!("cores: can't make {}", learned.display()), &e); }
        if let Some(w) = &watch { w.watch(&roms, &[&cores_dir, &learned]); }
        let now = on_card.find(&cores_dir, &learned, &all);
        if found.as_ref() != Some(&now) {
            log_cores(&cores_dir, &now);
            every = user_cores::served(all.clone(), &now.cores);
            let systems: Vec<SystemDecl> = every.iter().filter(|s| !s.cores.is_empty()).cloned().collect();
            decls = settings::declarations(&systems);
            // Before the app hears of them, so no game can be played with one before its
            // options' file is loaded.
            for c in now.cores.iter().filter(|c| cores_loaded.insert(c.id())) {
                let layer = Layer::Core(c.id().into());
                post_reading(reading, Msg::SettingsLoaded(layer.clone(), load_layer(&config, &decls, &layer)));
            }
            // (A guide written to a watched folder makes one more scan, which writes nothing.)
            if writable { write_schemas(&data, &decls, &systems); write_guides(&data, &every); }
            post_reading(reading, Msg::Cores { systems: systems.clone(), settings: decls.clone(), turned_away: now.turned_away.clone() });
            found = Some(now);
        }
        let scan = library::scan(&roms, &every);
        let now_no_emulator = scan.no_emulator(&every);
        if now_no_emulator != no_emulator {
            for (system, n) in &now_no_emulator { eprintln!("library: {n} {system} games aren't listed: no core plays them"); }
            no_emulator = now_no_emulator.clone();
        }
        let games = scan.games;
        for (from, to) in specular_core::library::saves::follow_moved_roms(&roms, &saves, &games) {
            eprintln!("saves: {} was renamed or moved to {}; its saves moved with it", from.display(), to.display());
        }
        // Before the games are listed, so no game can be played or have its settings shown
        // before they are loaded.
        let orphans = load_games_layers(reading, &config, &decls, &games, &scan.unplayable, &mut loaded, &mut logged_orphans);
        let set_aside = store::set_aside(&config);
        post_reading(reading, Msg::LeftOut(LeftOut { no_emulator: now_no_emulator, set_aside, orphans }));
        post_reading(reading, Msg::Library(games));
        // (Dropping the watch on the way out lets go of the card's folders, so a card pulled can
        // be unmounted.)
        if !watch.as_ref().is_some_and(watch::CardWatch::wait_for_change) { return; }
    }
}

/// Logs the cores found in the card's `cores/` (`dir`), and the files there turned away.
fn log_cores(dir: &Path, found: &user_cores::Found) {
    if found.cores.is_empty() && found.turned_away.is_empty() { eprintln!("cores: none in {}", dir.display()); }
    for c in &found.cores {
        let Some(u) = c.user else { continue };
        eprintln!("cores: {} ({}) plays {}: {}; {}", c.name, c.id(), u.systems.join(", "), u.path.display(), u.status());
    }
    for t in &found.turned_away { eprintln!("cores: not using {}: {}", dir.join(&t.file).display(), t.why); }
}

/// Writes the settings files' schemas (`settings::schema::all`) for `systems`, only the files whose
/// contents changed.
fn write_schemas(data: &DataRoot, decls: &[settings::SettingDecl], systems: &[SystemDecl]) {
    for (file, schema) in settings::schema::all(decls, systems) { write_generated("settings", &data.config().join(file), &store::json_text(&schema)); }
}

/// Writes the card's guide files (`guides::guides`) for `systems` (every system, with the cores
/// that serve it), only those whose text changed. Writing one makes its folder (`roms/`, `bios/`,
/// …) if the card has none.
fn write_guides(data: &DataRoot, systems: &[SystemDecl]) {
    for (file, text) in specular_core::guides::guides(systems, &settings::migrate::old_folders(&data.0)) { write_generated("guides", &data.0.join(file), &text); }
}

/// Writes a generated file (`store::store_generated`), logging it under `area` if it changed.
fn write_generated(area: &str, path: &Path, text: &str) {
    match store::store_generated(path, text) {
        Ok(true) => eprintln!("{area}: wrote {}", path.display()),
        Ok(false) => {}
        Err(e) => wrote_to_card(&format!("{area}: can't write {}", path.display()), &e),
    }
}

/// Reads `layer`'s file from the config folder, logging what it holds and what of it is ignored
/// (`settings::ignored`). A file that can't be read gives an empty layer (a damaged one is moved
/// aside: see `store::load_layer`).
fn load_layer(config: &Path, decls: &[settings::SettingDecl], layer: &Layer) -> serde_json::Map<String, serde_json::Value> {
    let path = layer.path(config);
    let values = store::load_layer(&path).unwrap_or_else(|e| {
        match e {
            store::LoadError::Io(e) => eprintln!("settings: can't read {}, so its settings are left out: {e}", path.display()),
            store::LoadError::Corrupt { moved_to, reason } => eprintln!("settings: {} isn't a JSON object ({reason}), so its settings are left out; it was moved to {}", path.display(), moved_to.display()),
        }
        Default::default()
    });
    let set: Vec<String> = values.iter().filter(|(k, _)| *k != settings::SCHEMA_KEY).map(|(k, v)| format!("{k} = {v}")).collect();
    if !set.is_empty() { eprintln!("settings: loaded {}: {}", path.display(), set.join(", ")); }
    for (key, why) in settings::ignored(decls, layer, &values) { eprintln!("settings: {}: {key} is ignored: {why}", path.display()); }
    values
}

/// Loads the settings file of each of `games` that has one and isn't `loaded` yet. Returns every file
/// under `games/` that matches no game on the card, `games` or `unplayable` (a ROM renamed, say), as
/// its game's path under `roms/` without the file type; logs each the first time (`logged`). Such
/// a file is loaded if its game appears in a later scan.
fn load_games_layers(reading: u32, config: &Path, decls: &[settings::SettingDecl], games: &[Game], unplayable: &[Game], loaded: &mut HashSet<String>, logged: &mut HashSet<String>) -> Vec<String> {
    let mut files = settings::games_with_settings(config);
    for g in unplayable { if let Layer::Game { game, .. } = g.settings_layer() { files.remove(&game); } }
    for g in games {
        let layer = g.settings_layer();
        let Layer::Game { game, .. } = &layer else { continue };
        if files.remove(game) && loaded.insert(game.clone()) { post_reading(reading, Msg::SettingsLoaded(layer.clone(), load_layer(config, decls, &layer))); }
    }
    for f in files.iter().filter(|f| logged.insert(f.to_string())) { eprintln!("settings: games/{f}.json is for no game in the library, so it isn't loaded"); }
    files.into_iter().collect()
}

/// Power-key presses are ignored from the request to sleep until this long after waking. The
/// press that woke the device is delivered as it resumes, before the sleep command returns (it
/// also restarts WiFi), and must not send the device straight back to sleep.
const WAKE_GRACE: Duration = Duration::from_millis(1500);

struct Runner {
    app: RefCell<App>,
    window: slint::Weak<AppWindow>,
    rows: Rc<VecModel<RowData>>,
    hints: Rc<VecModel<Hint>>,
    /// The hints for the buttons that move the focus, on a line above the bar (see `moves_focus`).
    move_hints: Rc<VecModel<Hint>>,
    status_icons: Rc<VecModel<IconData>>,
    profile: DeviceProfile,
    hardware: mpsc::Sender<HwCmd>,
    /// The CPU's speed steps (MHz), once the hardware thread has them; never, where its speed
    /// can't be set.
    cpu_steps: Arc<OnceLock<Vec<u32>>>,
    /// The radios' threads (None for a radio the device doesn't have).
    wifi: Option<mpsc::Sender<specular_core::wireless::wifi::WifiCmd>>,
    bluetooth: Option<mpsc::Sender<specular_core::wireless::bluetooth::BtCmd>>,
    /// The installer's thread (None on a device without one).
    installer: Option<mpsc::Sender<specular_core::app::InstallCmd>>,
    /// The time zone's thread (None on a device whose zone can't be set).
    time_zone: Option<mpsc::Sender<specular_core::timezone::ZoneCmd>>,
    /// The card service's thread (None on a device that doesn't report its card).
    card: Option<mpsc::Sender<CardCmd>>,
    io: mpsc::Sender<Io>,
    /// Counts the card's readings (`Effect::ReadCard`) and forgettings: what an earlier reading
    /// sends is stale, and dropped (`post_reading`).
    reading: Cell<u32>,
    data: DataRoot,
    device: game::SharedDevice,
    sounds: sounds::Sounds,
    /// Where sound goes: what the app last asked for (`Effect::SoundTo`). Games launched later
    /// open it; a running game is moved to it (`route_sound`).
    output: RefCell<Output>,
    /// Counts requests to move a running game's sound: an output opened for an older one is
    /// stale (`sound_opened`).
    sound_requests: Cell<u32>,
    /// The game being played (also while its menu is open).
    playing: RefCell<Option<game::Playing>>,
    game_screen: RefCell<game::Screen>,
    /// Fires when the next held-button repeat is due.
    repeat: slint::Timer,
    /// Fires when the device is due to sleep from being left alone.
    sleep_timer: slint::Timer,
    /// Fires when WiFi on Auto (on only while something needs the network) may be due to turn
    /// off (`App::wifi_deadline`).
    wifi_timer: slint::Timer,
    /// How many games have been launched: a game that finishes loading after another launch is
    /// stale (`started`).
    launches: Cell<u32>,
    clock: slint::Timer,
    ignore_power_until: Cell<Duration>,
    /// The layout last drawn: a new one gets fresh row slots.
    shown_layout: Cell<Option<Layout>>,
    /// For the frame statistics (`frames::Work`): row slots changed so far, and (slots, slots
    /// holding a row, those on screen) as of the last `refresh`.
    slots_changed: Cell<u64>,
    slots: Cell<(usize, usize, usize)>,
    /// Pictures loaded so far, by ROM path: the kind the game was shown as when it was loaded,
    /// and an image, or None if the game has none yet. A game not in it, or in it with another
    /// kind, hasn't been loaded (or was dropped, see `MAX_PICTURES`).
    art: RefCell<HashMap<PathBuf, (ArtKind, Option<slint::Image>)>>,
    /// Whether the last message was a tick while a button was held (the presses it makes repeat).
    repeating: Cell<bool>,
    /// The game whose picture is beside the list. It follows the focus, but while a direction is
    /// held only once the focus rests (`PICTURE_DELAY`), so held scrolling doesn't load each one.
    picture: RefCell<Option<ArtKey>>,
    picture_timer: slint::Timer,
    toast_timer: slint::Timer,
    volume_timer: slint::Timer,
    letter_timer: slint::Timer,
    /// When the game launched last was launched, until its first frame is in (see `SPLASH_TIME`).
    launched: Cell<Option<Duration>>,
    /// Shows the launched game once its splash has had `SPLASH_TIME`.
    splash_timer: slint::Timer,
    /// `App::tint` as last read (None: the first accent): the light bar's colour, and the LEDs'
    /// that show the first accent.
    tint: Cell<Option<Rgb>>,
    /// Updates the frame statistics each second while they are on.
    stats_timer: slint::Timer,
    /// The UI's animation frames, measured while the frame statistics are on.
    frame_times: RefCell<Option<frames::FrameTimes>>,
    /// The line to the rumble's thread (None on a device without rumble).
    rumble: Option<RumbleLine>,
    /// The lights' and the rumble's effects (see `show_lights`).
    effects: RefCell<Effects>,
    /// Fires when an effect next changes: at `effects_due` (on `now()`'s clock; None while stopped).
    effects_timer: slint::Timer,
    effects_due: Cell<Option<Duration>>,
    /// Whether the accents' settings have been applied: until then the lights aren't set (they
    /// would show the default colours, not the user's).
    lights_ready: Cell<bool>,
    /// What the lights were last set to (their colour per accent, and the LEDs' brightness).
    lights_shown: Cell<Option<([Rgb; 2], i64)>>,
    art_worker: mpsc::Sender<art::ArtCmd>,
    /// Whether the art worker was last told the network is there.
    art_online: Cell<bool>,
    art_loader: mpsc::Sender<Vec<ArtKey>>,
    /// What the art worker and loader were last told (less the pictures loaded since), so
    /// unchanged lists aren't sent again.
    on_screen: RefCell<Vec<PathBuf>>,
    requested: RefCell<Vec<ArtKey>>,
    /// What the art worker was last told each system's games are shown with (`tell_art_kinds`).
    art_kinds: RefCell<HashMap<&'static str, ArtKind>>,
}

/// A game's picture as shown: its ROM path, and the kind it is shown as.
type ArtKey = (PathBuf, ArtKind);

/// The shortest time a launched game's splash shows: it hides the game until its first frame is
/// in and this long after the launch. The game runs meanwhile; only showing it waits.
const SPLASH_TIME: Duration = Duration::from_millis(1000);
/// How long the core may take over one frame before the game counts as stuck (`watch_game`).
const STUCK_LIMIT: Duration = Duration::from_secs(6);
/// How long a toast shows at least; a long one stays long enough to be read (`TOAST_READING`).
const TOAST_TIME: Duration = Duration::from_millis(2500);
/// How long a toast shows per character: about 15 characters a second, an easy reading pace.
const TOAST_READING: Duration = Duration::from_millis(65);
/// How long the volume shows after the last change.
const VOLUME_TIME: Duration = Duration::from_millis(1200);
/// How long the letter index's letter shows after the last jump.
const LETTER_TIME: Duration = Duration::from_millis(700);
/// While a direction is held, the picture beside the list changes only once the focus has
/// rested this long.
const PICTURE_DELAY: Duration = Duration::from_millis(120);
/// When more than this many pictures are loaded, all but those on screen and the one beside the
/// list are dropped.
const MAX_PICTURES: usize = 64;

impl Runner {
    fn handle(self: &Rc<Self>, msg: Msg) {
        // While a game is played, its buttons go straight to it.
        if let Msg::Button(e) = &msg && game::for_game(e.button) && self.app.borrow().input_target() == InputTarget::Game {
            if let Some(p) = self.playing.borrow().as_ref() { p.button(e); }
            return;
        }
        match &msg {
            Msg::Button(e) if e.button == Button::Power && e.time < self.ignore_power_until.get() => return,
            Msg::Woke => self.ignore_power_until.set(now() + WAKE_GRACE),
            // The app's clock moves only with buttons and ticks; a question WiFi opens (the time
            // zone's) times its guard from it.
            Msg::Wifi(_) => self.handle(Msg::Tick(now())),
            Msg::Library(games) => { let _ = self.art_worker.send(art::ArtCmd::Library(games.clone())); }
            Msg::Battery(b) => self.tell_effects(Event::Battery(*b)),
            _ => {}
        }
        // (A new time zone shows at once.)
        let needs_clock = matches!(msg, Msg::Woke | Msg::GameEnded(_) | Msg::GameStopped(_) | Msg::TimeZone(ZoneEvent::Set { result: Ok(()), .. }));
        // The window can be resized (on a desktop), changing how many columns a grid has.
        if let Some(w) = self.window.upgrade() { self.app.borrow_mut().set_grid_columns(list::grid_columns(w.get_grid_width())); }
        // A tick while a button is held makes its repeats. (The flag stays set until the next
        // message, such as the button's release.)
        self.repeating.set(matches!(msg, Msg::Tick(_)) && self.app.borrow().next_deadline().is_some());
        let effects = self.app.borrow_mut().handle(msg);
        for e in effects { self.apply(e); }
        // Timers stood still while asleep: catch the clock up, now that the app has handled the
        // wake (the tick this sends must see its idle time reset). After a game ends the idle count
        // restarts from this tick.
        if needs_clock { self.tick_clock(); }
        self.tell_art_online();
        self.tell_art_kinds();
        // Each of the app's deadlines has a timer, which gives the app a `Tick` when it is due; all
        // are re-armed after every message, as the deadlines change.
        let app = self.app.borrow();
        let tick = |r: &Rc<Runner>| r.handle(Msg::Tick(now()));
        for (timer, due) in [(&self.repeat, app.next_deadline()), (&self.sleep_timer, app.sleep_deadline()), (&self.wifi_timer, app.wifi_deadline())] {
            at(timer, due, tick);
        }
    }

    /// Tells the art worker whether WiFi is connected, when that changes: it downloads only then.
    fn tell_art_online(&self) {
        let online = self.app.borrow().ctx().wifi.as_ref().is_none_or(|w| matches!(w.link, WifiLink::Connected { .. }));
        if online != self.art_online.replace(online) { let _ = self.art_worker.send(art::ArtCmd::Online(online)); }
    }

    /// Tells the art worker which kind of picture each system's games are shown with, when that
    /// changes (a setting, or a system's settings loaded): it downloads only those.
    fn tell_art_kinds(&self) {
        let app = self.app.borrow();
        let ctx = app.ctx();
        let kinds: HashMap<&'static str, ArtKind> = ctx.systems.iter().map(|s| (s.id, specular_core::art::kind(&ctx.settings, s.id))).collect();
        if kinds != *self.art_kinds.borrow() {
            *self.art_kinds.borrow_mut() = kinds.clone();
            let _ = self.art_worker.send(art::ArtCmd::Kinds(kinds));
        }
    }

    fn apply(self: &Rc<Self>, e: Effect) {
        match e {
            Effect::Redraw(t) => self.refresh(t),
            Effect::SaveSettings(layer) => {
                let values = self.app.borrow().ctx().settings.layer(&layer);
                let _ = self.io.send(Io::Save(layer.path(&self.data.config()), values));
            }
            Effect::ReadCard { writable } => {
                let _ = self.io.send(Io::Read { reading: self.next_reading(), writable });
            }
            Effect::CheckSetAside => { let _ = self.io.send(Io::CheckSetAside(self.reading.get())); }
            Effect::ForgetCard => {
                self.next_reading();
                let _ = self.io.send(Io::Forget);
                let _ = self.art_worker.send(art::ArtCmd::Library(vec![]));
            }
            Effect::Card(cmd) => if let Some(t) = &self.card { let _ = t.send(cmd); },
            Effect::Apply { key, value } => self.apply_setting(key, &value),
            Effect::Launch(game) => self.launch(&game),
            Effect::CoreOptions(options) => if let Some(p) = self.playing.borrow().as_ref() { p.session.set_options(options); },
            Effect::PauseGame(paused) => {
                // Play stops for the game menu (also as the device wakes), or for the card-wait
                // screen, where the card is gone and the autosave can't be written.
                let save = paused && self.app.borrow().ctx().card_writable();
                self.with_game("", |p| { p.pause(paused); if save { p.session.autosave(); } Ok("") });
                if paused { self.tell_effects(Event::GameStopped); }
                // (Under the card-wait screen the card is gone: there is no state to look at.)
                if paused && self.app.borrow().ctx().card_in() { self.tell_saved_state(); }
            }
            Effect::SaveState => if self.with_game("Couldn't save the state", |p| p.session.save_state(&p.state).map(|()| "State saved")) { self.tell_effects(Event::StateSaved) },
            Effect::LoadState => {
                self.with_game("Couldn't load the state", |p| if p.state.exists() { p.session.load_state(&p.state).map(|()| "State loaded") } else { Ok("No saved state yet") });
                self.tell_discs(); // the state may have been made on another disc
            }
            Effect::ChangeDisc(disc) => { self.with_game("Couldn't change the disc", |p| p.session.change_disc(disc).map(|()| "Disc changed")); self.tell_discs(); }
            Effect::ResetGame => { self.with_game("", |p| { p.session.reset(); Ok("") }); }
            Effect::Notice(text) => self.toast(&text),
            Effect::Mute(muted) => { let _ = self.hardware.send(HwCmd::Muted(muted)); }
            Effect::ShowVolume { level, muted, output } => {
                let Some(w) = self.window.upgrade() else { return };
                w.set_volume(level as f32 / builtin::VOLUME_MAX as f32);
                w.set_volume_text(level.to_string().into());
                w.set_volume_muted(muted);
                w.set_volume_mark(match output { Output::Speaker => IconKind::None, Output::Headphones => IconKind::Headphones, Output::Bluetooth(_) => IconKind::Bluetooth });
                self.show_for(&self.volume_timer, VOLUME_TIME, AppWindow::set_volume_shown);
            }
            Effect::ShowLetter(letter) => {
                let Some(w) = self.window.upgrade() else { return };
                w.set_letter(letter.to_string().into());
                self.show_for(&self.letter_timer, LETTER_TIME, AppWindow::set_letter_shown);
            }
            Effect::QuitGame | Effect::AbandonGame => {
                // (The game's texture is freed, and stops being shown, at the next draw.)
                let playing = self.playing.borrow_mut().take();
                if let Some(p) = playing {
                    if e == Effect::QuitGame {
                        self.keep_left_off(&p);
                        if let Err(e) = p.session.quit() { eprintln!("game: saving: {e}"); }
                    } else {
                        let (path, record) = p.record.clone();
                        p.session.abandon();
                        eprintln!("game: ended without saving: the SD card can't be written");
                        let _ = self.io.send(Io::Abandoned(path, record));
                    }
                }
                let _ = self.art_worker.send(art::ArtCmd::Paused(false));
                self.tell_effects(Event::GameStopped);
            }
            Effect::Sleep => {
                self.ignore_power_until.set(Duration::MAX); // until `Msg::Woke`
                // A game's battery save is written first: power can fail while asleep.
                let save = self.app.borrow().ctx().card_writable();
                self.with_game("Couldn't save the game before sleeping", |p| { p.pause(true); if save { p.session.flush().map(|()| "") } else { Ok("") } });
                self.tell_effects(Event::GameStopped);
                let _ = self.hardware.send(HwCmd::Sleep);
            }
            // A running game was quit by the `QuitGame` just before this, which writes its saves.
            // Settings still queued for the I/O thread are written when the system stops Specular
            // on its way down (SIGTERM ends the event loop; `run` then waits for the I/O thread).
            Effect::PowerOff => { let _ = self.hardware.send(HwCmd::PowerOff); }
            Effect::Restart => { let _ = self.hardware.send(HwCmd::Restart); }
            Effect::Wifi(cmd) => if let Some(t) = &self.wifi { let _ = t.send(cmd); },
            Effect::Bluetooth(cmd) => if let Some(t) = &self.bluetooth { let _ = t.send(cmd); },
            Effect::Installer(cmd) => if let Some(t) = &self.installer { let _ = t.send(cmd); },
            Effect::TimeZone(cmd) => if let Some(t) = &self.time_zone { let _ = t.send(cmd); },
            Effect::SoundTo(output) => self.set_output(output),
            Effect::Sound(sound) => self.sounds.play(sound),
            Effect::FileTransfer(start) => { let _ = self.hardware.send(HwCmd::FileTransfer(start)); }
            Effect::Ssh(cmd) => { let _ = self.hardware.send(HwCmd::Ssh(cmd)); }
        }
    }

    /// Advances the reading number, so messages from earlier readings are dropped
    /// (`post_reading`); returns the new number.
    fn next_reading(&self) -> u32 {
        self.reading.set(self.reading.get() + 1);
        self.reading.get()
    }

    /// Tells the app the running game's discs, as its core says now.
    fn tell_discs(&self) {
        if let Some(p) = self.playing.borrow().as_ref() { post(Msg::Discs(p.session.discs())); }
    }

    /// Tells the app when the running game's save state was made (its file's time), for its menu.
    fn tell_saved_state(&self) {
        let Some(state) = self.playing.borrow().as_ref().map(|p| p.state.clone()) else { return };
        let made = match std::fs::metadata(&state).and_then(|m| m.modified()) {
            Ok(t) => Some(local_minute(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => { eprintln!("saves: can't see when {} was saved: {e}", state.display()); None }
        };
        // A clock that isn't set yet (see `local_time`) can't say what today is.
        let now = Some(local_minute(std::time::SystemTime::now())).filter(|n| n.year >= CLOCK_SET);
        post(Msg::SavedState(made.map(|made| SavedState { made, now })));
    }

    /// Keeps the frame on screen as `p` is quit, as its game's picture of where it was left off:
    /// the art worker stores it. If its system's games are shown that way, the game shrinks into
    /// this frame, which the list shows from then on without waiting for the stored one. Nothing
    /// is kept if the game hadn't been shown yet (quit during its splash).
    fn keep_left_off(&self, p: &game::Playing) {
        let Some(w) = self.window.upgrade() else { return };
        if !w.get_game_revealed() { return; }
        let Some(frame) = p.frames.take_shown() else { return };
        let (rom, system) = (p.record.1.game.clone(), p.record.1.system.as_str());
        if specular_core::art::kind(&self.app.borrow().ctx().settings, system) == ArtKind::LeftOff {
            let image = slint::Image::from_rgba8(SharedPixelBuffer::clone_from_slice(&frame.rgba, frame.width, frame.height));
            w.set_flyer(Cover { known: true, has_art: true, art: image.clone(), ..w.get_flyer() });
            self.art.borrow_mut().insert(rom.clone(), (ArtKind::LeftOff, Some(image)));
        }
        let _ = self.art_worker.send(art::ArtCmd::LeftOff(rom, art::Picture { width: frame.width, height: frame.height, rgba: frame.rgba }));
    }

    /// Runs `f` on the game being played, if any, and shows what it says (nothing for "") or,
    /// if it fails, `failed` (its error is logged). Returns whether it ran and succeeded.
    fn with_game(&self, failed: &str, f: impl FnOnce(&game::Playing) -> std::io::Result<&'static str>) -> bool {
        let result = match self.playing.borrow().as_ref() { Some(p) => f(p), None => return false };
        match result {
            Ok("") => {}
            Ok(done) => self.toast(done),
            Err(e) => { eprintln!("game: {failed}: {e}"); self.toast(failed); return false; }
        }
        true
    }

    /// Sends sound to `output` from now on: its volume, the next game, and the game being played.
    fn set_output(&self, output: Output) {
        eprintln!("sound: output {output:?}");
        let _ = self.hardware.send(HwCmd::Output(output.clone()));
        self.sounds.set_output(output.clone());
        *self.output.borrow_mut() = output;
        self.route_sound();
    }

    /// Moves the running game's sound to `self.output` if it plays on another stream (see
    /// `Output::same_stream`). The output is opened on a thread of its own (a Bluetooth one can
    /// take a second), then handed over by `sound_opened`. Each call makes any earlier open still
    /// in progress stale.
    fn route_sound(&self) {
        let request = self.sound_requests.get() + 1;
        self.sound_requests.set(request);
        let wanted = self.output.borrow().clone();
        let playing = self.playing.borrow();
        let Some(p) = playing.as_ref() else { return };
        // Reopening isn't needed, and would fail: the game still holds the ALSA device, which
        // doesn't mix.
        if p.output.borrow().same_stream(&wanted) { *p.output.borrow_mut() = wanted; return; }
        let device = self.device.clone();
        std::thread::spawn(move || {
            let sound = game::open_sound(device.as_ref(), &wanted);
            on_ui(move |r| r.sound_opened(request, wanted, sound));
        });
    }

    /// `output` was opened (`sound`, None if it couldn't be) for move request `request`. Unless a
    /// newer request or another game came since (each launch calls `route_sound` in `started`,
    /// which makes this request stale), the game plays on it; otherwise it is dropped. A
    /// Bluetooth output that failed is reported to the app, which picks another output.
    fn sound_opened(&self, request: u32, output: Output, sound: Option<Box<dyn specular_core::device::SoundOut>>) {
        if request != self.sound_requests.get() { return; }
        let playing = self.playing.borrow();
        let Some(p) = playing.as_ref() else { return };
        if sound.is_none() && matches!(output, Output::Bluetooth(_)) { return post(Msg::SoundFailed(output)); }
        eprintln!("sound: the game's sound goes to {output:?}{}", if sound.is_none() { ", which didn't open: silence" } else { "" });
        p.session.set_sound(sound);
        *p.output.borrow_mut() = output;
    }

    /// Starts or stops the frame statistics: the game's, shown over it, and the UI's animation
    /// frames (see the `frames` module), shown at the bottom right, also over a game, and
    /// logged as each animation ends.
    fn show_frame_stats(&self, on: bool) {
        let Some(w) = self.window.upgrade() else { return };
        w.set_stats(Default::default());
        w.set_ui_stats(Default::default());
        *self.frame_times.borrow_mut() = on.then(|| frames::FrameTimes::new(self.profile.refresh_hz));
        if !on { self.stats_timer.stop(); return; }
        let mut last = 0;
        self.stats_timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), move || {
            let Some(r) = runner() else { return };
            let Some(w) = r.window.upgrade() else { return };
            // (Setting the same text doesn't redraw, so an idle screen stays idle.)
            if let Some(f) = r.frame_times.borrow().as_ref() { w.set_ui_stats(f.summary().into()); }
            // (A game's frame count starts at 0.)
            let Some(s) = r.playing.borrow().as_ref().map(|p| p.session.stats()) else { last = 0; return w.set_stats("".into()) };
            let fps = s.frames.saturating_sub(last);
            last = s.frames;
            let sound = s.sound_queued.map_or("no sound".into(), |q| format!("sound queued {:.0} ms", q.as_secs_f64() * 1e3));
            // (What the motors are told now, the game's and the effects' together.)
            let rumble = r.rumble.as_ref().map(|l| l.wanted()).filter(|&m| m != Motors::STILL)
                .map_or(String::new(), |m| format!(" · rumble {} %", m.strong.max(m.weak) as u32 * 100 / 0xffff));
            let cpu = s.cpu_min.map_or(String::new(), |mhz| format!(" · CPU ≥ {mhz} MHz"));
            // Two lines, so that all of it fits across the Brick's screen.
            w.set_stats(format!("{fps} fps · core {:.1} ms, 95 % under {:.1} ms\n{sound}{rumble}{cpu}", s.median.as_secs_f64() * 1e3, s.p95.as_secs_f64() * 1e3).into());
        });
    }

    /// Shows `text` for a moment, longer the longer it is.
    fn toast(&self, text: &str) {
        let Some(w) = self.window.upgrade() else { return };
        w.set_toast(text.into());
        self.show_for(&self.toast_timer, TOAST_TIME.max(TOAST_READING * text.chars().count() as u32), AppWindow::set_toast_shown);
    }

    /// Shows something for `time` from now, extending a showing under way: `shown(true)` now, then
    /// `shown(false)` on `timer`.
    fn show_for(&self, timer: &slint::Timer, time: Duration, shown: fn(&AppWindow, bool)) {
        let Some(w) = self.window.upgrade() else { return };
        shown(&w, true);
        after(timer, time, move |r| if let Some(w) = r.window.upgrade() { shown(&w, false) });
    }

    /// Starts `game`: its cover flies to where the game will be while it loads (once the redraw
    /// that follows shows it playing), with the splash over it until the game is shown
    /// (`SPLASH_TIME`). Art work stops while it runs, and loaded pictures are dropped once it has
    /// started, to leave it the memory and the CPU.
    fn launch(self: &Rc<Self>, game: &Game) {
        let _ = self.art_worker.send(art::ArtCmd::Paused(true));
        let system = self.app.borrow().ctx().systems.iter().find(|s| s.id == game.system).cloned();
        if let Some(w) = self.window.upgrade() {
            let cover = self.app.borrow().view().rows.iter().find(|r| r.id.0 == game.path.to_string_lossy()).map(|r| self.cover(r));
            w.set_flyer(cover.unwrap_or_else(empty_cover));
            let (gw, gh) = system.as_ref().map_or((0, 0), |s| s.screen);
            w.set_game_width(gw as i32);
            w.set_game_height(gh as i32);
            let menu = self.badges([(Button::Menu, "Game menu".to_string())].iter()).pop().unwrap_or_default();
            w.set_splash(self.app.borrow().splash().map(|s| SplashText {
                core: s.core.into(), copyright: s.copyright.unwrap_or_default().into(), url: s.url.unwrap_or_default().into(), menu,
            }).unwrap_or_default());
            w.set_game_revealed(false);
        }
        self.launched.set(Some(now()));
        self.splash_timer.stop(); // an earlier launch's mustn't show this game
        let launch = self.launches.get() + 1;
        self.launches.set(launch);
        let Some(core) = self.app.borrow().playing_core() else { return self.started(launch, Err(format!("There's no emulator for {} games.", system.map_or(game.system, |s| s.name)))) };
        let extensions = system.as_ref().map_or(&[][..], |s| s.extensions);
        if let Err(why) = core.check_bios(&self.data.roms().join(&game.path), extensions, &self.data.bios()) { return self.started(launch, Err(why)); }
        let options = self.app.borrow().core_options();
        let (cpu_min, why) = self.app.borrow().cpu_min(self.cpu_steps.get().cloned().unwrap_or_default());
        eprintln!("cpu: {why}");
        let (autosave, why) = self.app.borrow().autosave();
        eprintln!("game: {why}");
        game::Playing::start(self, game, core, options, cpu_min, autosave, move |r| on_ui(move |me| me.started(launch, r)));
    }

    /// Launch number `launch` (see `launches`) is running, or couldn't start.
    fn started(&self, launch: u32, result: Result<game::Playing, String>) {
        // The game was quit while it loaded (and maybe another launched since).
        if launch != self.launches.get() || self.app.borrow().playing().is_none() {
            match result {
                Ok(p) => if let Err(e) = p.session.quit() { eprintln!("game: saving: {e}"); },
                Err(e) => eprintln!("game: {e}"),
            }
            return;
        }
        match result {
            Ok(p) => {
                watch_game(&p, self.rumble.clone());
                // The output may have failed to open (the app then picks another), or changed while
                // the game loaded.
                let failed = p.sound_failed && *p.output.borrow() == *self.output.borrow();
                if failed { post(Msg::SoundFailed(self.output.borrow().clone())); }
                // Its options may have been changed (in its settings, over its menu) while it loaded.
                p.session.set_options(self.app.borrow().core_options());
                *self.playing.borrow_mut() = Some(p);
                self.tell_discs();
                self.tell_saved_state();
                self.route_sound();
                // (Kept until now: the list shows under the flying cover.)
                self.art.borrow_mut().clear();
                self.requested.borrow_mut().clear();
                // The game may have been paused (its menu opened) while it loaded.
                if self.app.borrow().input_target() == InputTarget::App { self.with_game("", |p| { p.pause(true); Ok("") }); }
            }
            Err(e) => {
                eprintln!("game: {e}");
                let _ = self.art_worker.send(art::ArtCmd::Paused(false));
                post(Msg::GameEnded(Some(e)));
            }
        }
    }

    /// The game has a new frame: draw it, and, while the game has the buttons (not under its menu),
    /// give the effects the loudest of its sound since the last.
    fn game_frame(&self) {
        if let Some(w) = self.window.upgrade() { w.window().request_redraw(); }
        if self.app.borrow().input_target() != InputTarget::Game { return; }
        let Some(peak) = self.playing.borrow().as_ref().map(|p| p.session.take_sound_peak()) else { return };
        self.tell_effects(Event::Sound(peak));
    }

    /// Gives `event` to the effects (logging each that starts) and shows what they show now.
    fn tell_effects(&self, event: Event) {
        self.effects.borrow_mut().event(event, now(), |name| eprintln!("effects: {name}"));
        self.show_lights();
    }

    /// Shows what the lights show now, the accents (the system's tint, if there is one, in place of
    /// the first) under any effect running, on the LEDs and the on-screen light bar (which shows
    /// the LED bar's colour, the first accent's), passes the effects' rumble on, and arms the timer
    /// for the effects' next change.
    fn show_lights(&self) {
        let [first, second] = self.accents();
        let (frame, due) = self.effects.borrow().frame(now(), [self.tint.get().unwrap_or(first), second]);
        let level = self.app.borrow().ctx().settings.get(builtin::LED_BRIGHTNESS, &Layer::Global).and_then(|(v, _)| v.as_i64()).unwrap_or(0);
        let shown = Some((frame.lights, level));
        if self.lights_ready.get() && self.lights_shown.replace(shown) != shown {
            let Rgb(r, g, b) = frame.light(Accent::First);
            if let Some(w) = self.window.upgrade() { w.global::<Theme>().set_tint(slint::Color::from_rgb_u8(r, g, b)); }
            let scale = |c: u8| (c as i64 * level / builtin::LED_BRIGHTNESS_MAX) as u8;
            let leds = self.profile.leds.iter().map(|&led| { let Rgb(r, g, b) = frame.light(led); Rgb(scale(r), scale(g), scale(b)) });
            let _ = self.hardware.send(HwCmd::Lights(leds.collect()));
        }
        if let Some(line) = &self.rumble { line.set_effects(frame.rumble); }
        if self.effects_due.replace(due) == due { return; }
        at(&self.effects_timer, due, |r| {
            r.effects_due.set(None); // so the same deadline, if it comes again, re-arms the timer
            r.show_lights();
        });
    }

    /// Before each frame is drawn: the game's newest frame goes into its texture.
    fn before_render(&self, api: &slint::GraphicsAPI) {
        let Some(w) = self.window.upgrade() else { return };
        if let Some(times) = self.frame_times.borrow_mut().as_mut() { times.drawing(cpu_time()); }
        let playing = self.playing.borrow();
        // The launched game's first frame: it shows once its splash has had its time.
        if self.game_screen.borrow_mut().before_render(api, playing.as_ref().map(|p| &*p.frames), &w) && let Some(launched) = self.launched.take() {
            let first = now() - launched;
            let wait = SPLASH_TIME.saturating_sub(first);
            eprintln!("game: first frame {} ms after the launch; the splash hides it {} ms more", first.as_millis(), wait.as_millis());
            after(&self.splash_timer, wait, |r| if let Some(w) = r.window.upgrade() { w.set_game_revealed(true) });
        }
    }

    /// After each frame is drawn (before it is presented): records it for the frame statistics,
    /// if they are on.
    fn after_render(&self) {
        let mut times = self.frame_times.borrow_mut();
        let (Some(times), Some(w)) = (times.as_mut(), self.window.upgrade()) else { return };
        let work = frames::Work { slots_changed: self.slots_changed.get(), slots: self.slots.get() };
        let lines = times.drawn(now(), cpu_time(), w.window().has_active_animations(), work);
        lines.into_iter().flatten().for_each(|line| eprintln!("ui: {line}"));
    }

    /// Carries out the settings the device acts on; ignores the rest.
    fn apply_setting(&self, key: &'static str, value: &serde_json::Value) {
        let level = value.as_i64().unwrap_or(0);
        self.effects.borrow_mut().apply(key, value);
        let cmd = match key {
            builtin::BRIGHTNESS => HwCmd::Brightness(level as u8),
            builtin::VOLUME | builtin::HEADPHONE_VOLUME | builtin::BLUETOOTH_VOLUME => HwCmd::Volume { setting: key, level: level as u8 },
            builtin::FRAME_STATS => { self.show_frame_stats(value.as_bool().unwrap_or(false)); return; }
            builtin::SCALING => {
                if let Some(w) = self.window.upgrade() { w.set_scaling(value.as_str().unwrap_or("integer").into()); }
                return;
            }
            builtin::RUMBLE => { if let Some(line) = &self.rumble { line.set_level(level as u8); } return; }
            // The theme and the lights each depend on several settings, so they are read back from
            // the settings (already updated) rather than taken from `value`.
            builtin::ACCENT | builtin::SECOND_ACCENT | builtin::LED_BRIGHTNESS => {
                self.show_theme();
                self.lights_ready.set(true);
                self.show_lights();
                return;
            }
            // An effect turned off stops showing.
            builtin::LOW_BATTERY_LIGHTS | builtin::SAVE_FLASH | builtin::RUMBLE_FOLLOWS_SOUND => { self.show_lights(); return; }
            _ => return,
        };
        let _ = self.hardware.send(cmd);
    }

    /// The accent and second accent, as the settings resolve them.
    fn accents(&self) -> [Rgb; 2] {
        let app = self.app.borrow();
        let s = &app.ctx().settings;
        let first = s.colour(builtin::ACCENT, &Layer::Global).unwrap_or_default();
        [first, s.colour(builtin::SECOND_ACCENT, &Layer::Global).unwrap_or(first)]
    }

    /// Sets the theme's colours, but for the light bar's tint, which `show_lights` sets with the LEDs.
    fn show_theme(&self) {
        let Some(w) = self.window.upgrade() else { return };
        let [first, second] = self.accents();
        let colour = |Rgb(r, g, b): Rgb| slint::Color::from_rgb_u8(r, g, b);
        let theme = w.global::<Theme>();
        theme.set_accent(colour(first));
        theme.set_focus(colour(second));
        theme.set_warning(colour(palette::WARNING));
    }

    /// Shows the app's current screen; `t` animates a change of screen.
    fn refresh(&self, t: Option<Transition>) {
        let Some(w) = self.window.upgrade() else { return };
        let (view, playing, menu) = {
            let app = self.app.borrow();
            (app.view(), app.playing().is_some(), app.playing().is_some() && app.input_target() == InputTarget::App)
        };
        let (grid, keys) = (view.layout == Layout::Grid, view.layout == Layout::Keys);
        w.set_layout(match view.layout { Layout::List => ScreenLayout::List, Layout::ListWithPicture => ScreenLayout::ListWithPicture, Layout::Grid => ScreenLayout::Grid, Layout::Keys => ScreenLayout::Keys });
        // Set before the list's size is read below: the game menu narrows the list, and a line of
        // hints shortens it.
        w.set_playing(playing);
        w.set_game_menu(menu);
        let (moves, bar): (Vec<_>, Vec<_>) = view.hints.iter().partition(|(b, _)| moves_focus(*b));
        let moves = self.badges(moves.into_iter());
        if self.move_hints.iter().ne(moves.iter().cloned()) { self.move_hints.set_vec(moves); }
        // Grids and the keyboard sit between the gutters; a list's rows reach the edges.
        let (width, height) = (if grid || keys { w.get_grid_width() } else { w.get_list_width() }, w.get_list_height());
        // Where the focus can stop on a note depends on how much of the list shows.
        if !grid && !keys { self.app.borrow_mut().set_list_area(width, height); }
        let app = self.app.borrow();
        let focus = app.focus();
        let shape = list::picture_shape(&view.rows);
        if w.get_picture_shape() != shape { w.set_picture_shape(shape); }
        let cells = grid.then_some(shape);
        let layout = if keys { keys::layout(&view.rows, width, height) } else { list::layout(&view.rows, focus, app.reading(), width, height, cells) };
        // Rows are drawn from a fixed set of slots, enough for every row near the view; a row
        // always uses slot `index % slots`, so while the list scrolls a row keeps its slot and
        // only the slots of rows coming into view change. A new screen or layout gets fresh
        // slots, so the old one's rows don't animate into the new one.
        // The keyboard draws every key at once, so it needs a slot per key.
        let slots = if keys { view.rows.len() } else { list::max_drawn(width, height, cells) };
        debug_assert!(layout.drawn.len() <= slots, "two drawn rows would share a slot");
        // `focus` is 0 on a screen where no row can take the focus: nothing is focused then.
        let focusable = view.rows.get(focus).is_some_and(|r| r.kind.focusable());
        let mut wanted = vec![empty_row(); slots];
        for i in layout.drawn.clone() {
            wanted[i % slots] = self.row_data(&view.rows[i], focusable && i == focus, layout.rects[i]);
        }
        let on_screen = layout.drawn.clone().filter(|&i| { let r = layout.rects[i]; r.y + r.height > layout.scroll && r.y < layout.scroll + height }).count();
        self.slots.set((slots, layout.drawn.len(), on_screen));
        let changed = if t.is_none() && self.shown_layout.get() == Some(view.layout) && self.rows.row_count() == slots {
            wanted.into_iter().enumerate().filter(|(slot, r)| self.rows.row_data(*slot).as_ref() != Some(r))
                .map(|(slot, r)| self.rows.set_row_data(slot, r)).count()
        } else {
            self.rows.set_vec(wanted);
            slots
        };
        self.slots_changed.set(self.slots_changed.get() + changed as u64);
        self.shown_layout.set(Some(view.layout));
        let row = view.rows.get(focus).map_or(&[][..], |r| &r.hints);
        // The focused row's hints first, then the screen's.
        let hints = self.badges(row.iter().chain(bar));
        if self.hints.iter().ne(hints.iter().cloned()) { self.hints.set_vec(hints); }
        w.set_screen_title(view.title.as_str().into());
        if let Some(entry) = &view.entry {
            // `entry.caret` counts characters; slicing needs a byte offset.
            let caret = entry.text.char_indices().nth(entry.caret).map_or(entry.text.len(), |(i, _)| i);
            w.set_entry_before(entry.text[..caret].into());
            w.set_entry_after(entry.text[caret..].into());
            w.set_entry_height(keys::ENTRY_HEIGHT);
        }
        w.set_crumb(app.parent_title().unwrap_or_default().into());
        w.set_plate_shown(focusable);
        let plate = layout.rects.get(focus).copied().unwrap_or_default();
        w.set_plate_x(plate.x);
        w.set_plate_width(plate.width);
        w.set_plate_top(plate.y);
        w.set_plate_height(plate.height);
        // Scrolls of more than a view (paging, wrapping from the last row to the first) jump:
        // animating them would pass rows that aren't drawn.
        w.set_scroll_jumps((layout.scroll - w.get_scroll()).abs() > height);
        w.set_scroll(layout.scroll);
        let end = layout.rects.last().map_or(0.0, |r| r.y + r.height);
        w.set_more_above(layout.scroll > 0.5);
        w.set_more_below(end - layout.scroll > height + 0.5);
        let status: Vec<IconData> = app.status().iter().map(icon_data).collect();
        if self.status_icons.iter().ne(status.iter().cloned()) { self.status_icons.set_vec(status); }
        if let Some(t) = t {
            w.set_shift(if t == Transition::Push { 64.0 } else { -64.0 });
            w.set_transitions(w.get_transitions() + 1);
        }
        let (held, playing, tint) = (self.repeating.get(), app.playing().is_some(), app.tint());
        drop(app);
        // Recoloured only when the tint changes (LED writes can be slow).
        if tint != self.tint.replace(tint) { self.show_lights(); }
        // While a game is played the library is hidden and its menu has no art, so no art is
        // loaded (see `launch`).
        if !playing { self.update_art(&w, &view, focus, &layout.drawn, held); }
    }

    /// Keeps the art service busy with what is on screen: the worker finds missing art for the
    /// rows drawn, and the loader loads the pictures shown (a grid's, nearest the focus first, or
    /// the one beside the list, which waits for the focus to rest while `held`; see `picture`).
    fn update_art(&self, w: &AppWindow, view: &ScreenView, focus: usize, drawn: &std::ops::Range<usize>, held: bool) {
        let rom = |i: usize| view.rows.get(i).and_then(art_of).map(|(_, rom, kind)| (rom.clone(), kind));
        let on_screen: Vec<PathBuf> = drawn.clone().filter_map(rom).map(|(path, _)| path).collect();
        if *self.on_screen.borrow() != on_screen {
            let _ = self.art_worker.send(art::ArtCmd::OnScreen(on_screen.clone()));
            *self.on_screen.borrow_mut() = on_screen;
        }
        let wanted: Vec<ArtKey> = match view.layout {
            Layout::Keys => Vec::new(),
            Layout::Grid => {
                let mut near: Vec<usize> = drawn.clone().collect();
                near.sort_by_key(|&i| i.abs_diff(focus));
                near.into_iter().filter_map(rom).collect()
            }
            Layout::ListWithPicture => {
                let focused = rom(focus);
                if focused != *self.picture.borrow() {
                    if held {
                        after(&self.picture_timer, PICTURE_DELAY, move |r| { *r.picture.borrow_mut() = focused.clone(); r.refresh(None); });
                    } else {
                        self.picture_timer.stop();
                        *self.picture.borrow_mut() = focused;
                    }
                }
                let picture = self.picture.borrow().clone();
                let row = picture.as_ref().and_then(|p| view.rows.iter().find(|r| art_of(r).is_some_and(|(_, rom, kind)| (rom, kind) == (&p.0, p.1))));
                // A picture still loading leaves the last one up, rather than blinking to nothing.
                if let Some(cover) = row.map(|r| self.cover(r)) && cover.known { w.set_picture(cover); }
                picture.into_iter().collect()
            }
            Layout::List => vec![],
        };
        let art = self.art.borrow();
        let wanted: Vec<ArtKey> = wanted.into_iter().filter(|p| !art.get(&p.0).is_some_and(|(k, _)| *k == p.1)).collect();
        if !wanted.is_empty() && *self.requested.borrow() != wanted {
            let _ = self.art_loader.send(wanted.clone());
            *self.requested.borrow_mut() = wanted;
        }
    }

    /// A picture arrived from the loader.
    fn art_loaded(&self, key: ArtKey, picture: Option<slint::Image>) {
        self.requested.borrow_mut().retain(|p| p != &key);
        let mut art = self.art.borrow_mut();
        art.insert(key.0, (key.1, picture));
        if art.len() > MAX_PICTURES {
            let (on_screen, picture) = (self.on_screen.borrow(), self.picture.borrow());
            art.retain(|p, _| on_screen.contains(p) || picture.as_ref().is_some_and(|(rom, _)| rom == p));
        }
        drop(art);
        self.refresh(None);
    }

    /// The art worker stored a new picture: load it if it is shown.
    fn art_stored(&self, rom: PathBuf) {
        self.art.borrow_mut().remove(&rom);
        self.refresh(None);
    }

    /// The cover for a row with a picture: the picture if loaded, else a generated one.
    fn cover(&self, r: &Row) -> Cover {
        let Some((system, rom, kind)) = art_of(r) else { return empty_cover() };
        let known = self.art.borrow().get(rom).filter(|(k, _)| *k == kind).map(|(_, p)| p.clone());
        let (top, bottom) = cover_colours(&r.label);
        Cover {
            known: known.is_some(), has_art: matches!(known, Some(Some(_))), art: known.flatten().unwrap_or_else(no_art),
            top, bottom, title: r.label.as_str().into(), system: system.to_uppercase().into(),
        }
    }

    /// A screen's row `r`, ready to draw at `rect` (u).
    fn row_data(&self, r: &Row, focused: bool, rect: Rect) -> RowData {
        let (kind, value, on, fraction) = match &r.kind {
            RowKind::Header => (Kind::Header, String::new(), false, 0.0),
            RowKind::Note { detail } => (Kind::Note, detail.clone().unwrap_or_default(), false, 0.0),
            RowKind::Text { detail } | RowKind::Image { detail, .. } => (Kind::Text, detail.clone().unwrap_or_default(), false, 0.0),
            RowKind::Toggle { on } => (Kind::Toggle, String::new(), *on, 0.0),
            RowKind::Choice { options, selected, .. } => (Kind::Choice, options.get(*selected).cloned().unwrap_or_default(), false, 0.0),
            RowKind::Number { value, min, max, unit, .. } => (Kind::Number, format!("{value}{unit}"), false, (value - min) as f32 / (max - min).max(1) as f32),
            RowKind::Action { value } => (Kind::Action, value.clone().unwrap_or_default(), false, 0.0),
            RowKind::Key { .. } => (Kind::Key, String::new(), false, 0.0),
        };
        // A note is drawn in the lines the layout made room for.
        let label = match &r.kind {
            RowKind::Note { detail } => list::note_lines(&r.label, detail.as_deref(), rect.width).join("\n"),
            _ => r.label.clone(),
        };
        RowData {
            shown: true, focused, kind, label: label.into(), value: value.into(), on, fraction,
            overridden: r.overridden, x: rect.x, top: rect.y, width: rect.width, height: rect.height,
            cover: self.cover(r),
            // Transparent (not drawn) for rows without one.
            swatch: match r.kind { RowKind::Choice { swatch: Some(Rgb(r, g, b)), .. } => slint::Color::from_rgb_u8(r, g, b), _ => Default::default() },
            icon: r.icon.as_ref().map(icon_data).unwrap_or_default(),
        }
    }

    /// Hints as drawn, for the buttons the device has; hints side by side with the same label
    /// share a badge ("L1/R1").
    fn badges<'a>(&self, hints: impl Iterator<Item = &'a (Button, String)>) -> Vec<Hint> {
        let mut out: Vec<Hint> = vec![];
        for (b, label) in hints.filter(|(b, _)| self.profile.buttons.contains(b)) {
            match out.last_mut() {
                Some(h) if h.label == label.as_str() => h.button = format!("{}/{}", h.button, button_label(*b)).into(),
                _ => out.push(Hint { button: button_label(*b).into(), label: label.as_str().into() }),
            }
        }
        out
    }

    /// Shows the time, and wakes again at the start of the next minute (not every second).
    /// Also gives the app a reading of the clock each minute, which starts its idle count (for
    /// "Sleep after") after startup, waking and the end of a game.
    fn tick_clock(self: &Rc<Self>) {
        self.handle(Msg::Tick(now()));
        let mut next = Duration::from_secs(60);
        if let (Some(w), Some((h, m, s))) = (self.window.upgrade(), local_time()) {
            w.set_clock(format!("{h:02}:{m:02}").into());
            next = Duration::from_secs(60 - s as u64);
        }
        after(&self.clock, next, |r| r.tick_clock());
    }
}

/// The system, ROM and kind of picture of a row that shows a game's picture.
fn art_of(r: &Row) -> Option<(&'static str, &PathBuf, ArtKind)> {
    match &r.kind { RowKind::Image { image: ImageRef::Art { system, rom, kind, .. }, .. } => Some((system, rom, *kind)), _ => None }
}

thread_local! {
    static NO_ART: slint::Image = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(1, 1));
}

/// The `art` of a cover without any: a 1×1 image (never drawn, as `has_art` is false). Not
/// Slint's empty `Image::default()`, which never equals another image, nor itself (i-slint-core
/// 1.18.1, `ImageInner::eq`): at each `Runner::refresh`, every row slot or cover holding one
/// would compare unequal to the copy already shown, be set again, and be laid out and have its
/// text shaped again.
fn no_art() -> slint::Image { NO_ART.with(Clone::clone) }

fn empty_cover() -> Cover { Cover { art: no_art(), ..Default::default() } }

/// The data of a row slot that holds no row.
fn empty_row() -> RowData { RowData { cover: empty_cover(), ..Default::default() } }

/// A generated cover's gradient (top, bottom), from the title, so each game keeps its own.
fn cover_colours(title: &str) -> (slint::Color, slint::Color) {
    // FNV-1a: any stable hash will do.
    let hash = title.bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193));
    let hue = (hash % 360) as f32;
    (slint::Color::from_hsva(hue, 0.62, 0.493, 1.0), slint::Color::from_hsva(hue + 40.0, 2.0 / 3.0, 0.21, 1.0))
}

/// An icon as the UI draws it. Unknown values become -1 (Slint has no optionals), and a battery
/// that is charging counts as plugged in.
fn icon_data(icon: &Icon) -> IconData {
    match *icon {
        Icon::Wifi { bars, locked } => IconData { kind: IconKind::Wifi, bars: bars.map_or(-1, i32::from), locked, ..Default::default() },
        Icon::Bluetooth { kind, lit } => {
            let kind = match kind {
                BtKind::Headphones => IconKind::Headphones, BtKind::Speaker => IconKind::Speaker, BtKind::Controller => IconKind::Controller,
                BtKind::Keyboard => IconKind::Keyboard, BtKind::Phone => IconKind::Phone, BtKind::Computer => IconKind::Computer,
                BtKind::Other => IconKind::Bluetooth,
            };
            IconData { kind, lit, ..Default::default() }
        }
        Icon::Battery(b) => IconData {
            kind: IconKind::Battery, percent: b.percent.map_or(-1, i32::from), plugged: b.plugged || b.charging, low: b.low(), ..Default::default()
        },
    }
}

/// The local time (hours, minutes, seconds), or None while the clock is clearly unset (before
/// 2025, as on a device that hasn't had network time yet). In the zone in effect now: glibc's
/// `localtime_r` keeps the zone it loaded first, so `tzset` has it look again (a stat of
/// /etc/localtime, or a look at $TZ).
fn local_time() -> Option<(u32, u32, u32)> {
    let tm = local(unsafe { libc::time(std::ptr::null_mut()) });
    (tm.tm_year + 1900 >= CLOCK_SET).then_some((tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32))
}

/// A clock before this year isn't set yet (see `local_time`).
const CLOCK_SET: i32 = 2025;

/// `t` (seconds since 1970) in local time, in the zone in effect now (see `local_time`).
fn local(t: libc::time_t) -> libc::tm {
    unsafe extern "C" { fn tzset(); }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { tzset(); libc::localtime_r(&t, &mut tm) };
    tm
}

/// `t` in local time, to the minute.
fn local_minute(t: std::time::SystemTime) -> LocalTime {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let tm = local(secs as libc::time_t);
    LocalTime { year: tm.tm_year + 1900, month: tm.tm_mon as u8 + 1, day: tm.tm_mday as u8, hour: tm.tm_hour as u8, minute: tm.tm_min as u8 }
}

/// Whether a screen's hint for `b` goes on the line above the hint bar: the shoulder buttons,
/// which move the focus (L1/R1 a page, L2/R2 a letter), as a game list's bar has no room for them.
/// A focused row's own hints always go in the bar.
fn moves_focus(b: Button) -> bool { matches!(b, Button::L1 | Button::R1 | Button::L2 | Button::R2) }

fn button_label(b: Button) -> &'static str {
    use Button::*;
    match b {
        A => "A", B => "B", X => "X", Y => "Y", L1 => "L1", R1 => "R1", L2 => "L2", R2 => "R2",
        Select => "SELECT", Start => "START", Menu => "MENU", F1 => "F1", F2 => "F2",
        Up => "↑", Down => "↓", Left => "←", Right => "→", VolUp => "+", VolDown => "−", Power => "⏻", Mute => "MUTE",
    }
}

/// Desktop keys: (the key's text as Slint reports it, the button, its name in SPECULAR_SCRIPT).
/// Arrows are the d-pad; X/Z/S/A are A/B/X/Y; Q/W are L1/R1; Enter, Tab and Esc are Start,
/// Select and Menu.
const KEYS: &[(&str, Button, &str)] = &[
    ("\u{f700}", Button::Up, "up"), ("\u{f701}", Button::Down, "down"), ("\u{f702}", Button::Left, "left"),
    ("\u{f703}", Button::Right, "right"), ("x", Button::A, "a"), ("z", Button::B, "b"), ("s", Button::X, "x"),
    ("a", Button::Y, "y"), ("q", Button::L1, "l1"), ("w", Button::R1, "r1"), ("1", Button::L2, "l2"),
    ("2", Button::R2, "r2"), ("\n", Button::Start, "start"), ("\t", Button::Select, "select"),
    ("\u{1b}", Button::Menu, "menu"), ("-", Button::VolDown, "voldown"), ("=", Button::VolUp, "volup"), ("p", Button::Power, "power"),
];

/// The desktop key that plugs simulated headphones into the jack, or unplugs them (`Msg::Headphones`).
const HEADPHONES_KEY: &str = "h";

fn key_button(text: &str) -> Option<Button> {
    let t = text.to_lowercase();
    KEYS.iter().find(|(k, _, _)| *k == t).map(|&(_, b, _)| b)
}

/// For development and visual checks: `SPECULAR_SCRIPT="down,down,a"` presses those buttons (by
/// the names in `KEYS`) 400 ms apart (or `SPECULAR_SCRIPT_MS`), the first after two steps, each
/// held for 50 ms (so a game sees it: games read buttons once a frame), then, one step after the
/// last, saves the window as raw RGBA to `SPECULAR_SNAPSHOT` (default /tmp/specular.rgba) and
/// quits. `down:3000` holds the button for 3000 ms instead (it repeats as if held), and the next
/// press comes at the first step after it is released. Unknown names and unreadable times are
/// skipped.
fn script(window: &AppWindow, names: &str) -> slint::Timer {
    let mut steps: Vec<(Button, u64)> = names.split(',').filter_map(|n| {
        let (n, hold) = n.trim().split_once(':').unwrap_or((n.trim(), "50"));
        Some((KEYS.iter().find(|(_, _, name)| *name == n)?.1, hold.parse().ok()?))
    }).collect();
    steps.reverse();
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut started = false;
    let step: u64 = std::env::var("SPECULAR_SCRIPT_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(400);
    // Steps left to wait for a held button's release.
    let mut holding = 0;
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(step), move || {
        if !started { started = true; return; } // let the scan and the first frames finish
        if holding > 0 { holding -= 1; return; }
        if let Some((button, hold)) = steps.pop() {
            post(Msg::Button(ButtonEvent { button, pressed: true, time: now() }));
            slint::Timer::single_shot(Duration::from_millis(hold), move || post(Msg::Button(ButtonEvent { button, pressed: false, time: now() })));
            holding = hold / step;
            return;
        }
        let Some(w) = weak.upgrade() else { return };
        if let Ok(shot) = w.window().take_snapshot() {
            let path = std::env::var("SPECULAR_SNAPSHOT").unwrap_or_else(|_| "/tmp/specular.rgba".into());
            let _ = std::fs::write(&path, shot.as_bytes());
            eprintln!("snapshot: {}×{} RGBA in {path}", shot.width(), shot.height());
        }
        let _ = slint::quit_event_loop();
    });
    timer
}

#[cfg(test)]
mod tests {
    #[test]
    fn desktop_keys_match_slints_key_codes() {
        use slint::platform::Key;
        for (key, code) in [(Key::UpArrow, "\u{f700}"), (Key::DownArrow, "\u{f701}"), (Key::LeftArrow, "\u{f702}"),
                            (Key::RightArrow, "\u{f703}"), (Key::Return, "\n"), (Key::Tab, "\t"), (Key::Escape, "\u{1b}")] {
            assert_eq!(slint::SharedString::from(key).as_str(), code);
        }
    }

    /// Records each `set_volume`.
    #[derive(Default)]
    struct Levels(std::sync::Mutex<Vec<(super::Output, u8)>>);

    impl super::Audio for Levels {
        fn set_volume(&self, output: &super::Output, level: u8) -> std::io::Result<()> {
            self.0.lock().unwrap().push((output.clone(), level));
            Ok(())
        }
    }

    impl Levels {
        fn take(&self) -> Vec<(super::Output, u8)> { std::mem::take(&mut self.0.lock().unwrap()) }
    }

    #[test]
    fn the_speaker_and_headphones_get_their_levels_whichever_plays() {
        use super::{Output::*, builtin};
        let audio = Levels::default();
        let mut v = super::Volume { levels: Default::default(), output: Speaker, muted: false };
        let headset = Bluetooth(specular_core::wireless::bluetooth::BtAddress("00:11:22:33:44:55".into()));
        v.set_level(&audio, builtin::VOLUME, 12).unwrap();
        v.set_level(&audio, builtin::HEADPHONE_VOLUME, 6).unwrap();
        v.set_level(&audio, builtin::BLUETOOTH_VOLUME, 9).unwrap();
        assert_eq!(audio.take(), [(Speaker, 12), (Headphones, 6)]);
        // The device moves between the speaker and the jack by itself.
        v.set_output(&audio, Headphones).unwrap();
        assert_eq!(audio.take(), []);
        v.set_level(&audio, builtin::VOLUME, 13).unwrap();
        assert_eq!(audio.take(), [(Speaker, 13)]);
        // A Bluetooth output's level is set as sound moves to it, and while it plays.
        v.set_output(&audio, headset.clone()).unwrap();
        v.set_level(&audio, builtin::BLUETOOTH_VOLUME, 10).unwrap();
        assert_eq!(audio.take(), [(headset.clone(), 9), (headset.clone(), 10)]);
        // Muting silences every output given a level, also one changed while muted, and
        // unmuting restores them.
        v.set_muted(&audio, true).unwrap();
        v.set_level(&audio, builtin::HEADPHONE_VOLUME, 7).unwrap();
        assert_eq!(audio.take(), [(Speaker, 0), (Headphones, 0), (headset.clone(), 0), (Headphones, 0)]);
        v.set_muted(&audio, false).unwrap();
        assert_eq!(audio.take(), [(Speaker, 13), (Headphones, 7), (headset, 10)]);
        v.set_output(&audio, Speaker).unwrap();
        assert_eq!(audio.take(), []);
    }

    #[test]
    fn rows_without_art_equal_themselves() {
        // Slint's quirk that `no_art` works around; if this fails, `no_art` can go.
        assert_ne!(slint::Image::default(), slint::Image::default());
        assert_eq!(super::empty_row(), super::empty_row());
    }

    #[test]
    fn only_font_files_directly_in_the_cards_folder_are_used() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str, bytes: &[u8]| std::fs::write(dir.path().join(name), bytes).unwrap();
        file("b.otf", b"OTTO and the rest");
        file("a.ttf", b"\0\x01\0\0 and the rest");
        file("readme.txt", b"Fonts go here");
        file("tiny.ttf", b"\0");
        file("odd:name.ttc", b"ttcf");
        std::fs::create_dir(dir.path().join("more")).unwrap();
        file("README.txt", b"Specular's guide");
        let (used, turned_away) = super::card_fonts(dir.path());
        assert_eq!(used, [dir.path().join("a.ttf"), dir.path().join("b.otf")]);
        let names: Vec<&str> = turned_away.iter().map(|t| t.file.as_str()).collect();
        assert_eq!(names, ["more", "odd:name.ttc", "readme.txt", "tiny.ttf"]);
        assert_eq!(super::card_fonts(&dir.path().join("missing")), Default::default());
    }
}
