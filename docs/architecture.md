# Architecture

Status: Implemented · Updated: 2026-10-19

How Specular is put together: the toolkit, the crates and threads, the boundary a device port implements, where plugins will plug in, and the memory and licence limits that shape all of it. Most later choices follow from two goals: the UI must stay smooth on a slow handheld, and the same code must run on any Linux device.

## Decisions

- **Slint, in Rust** (2026-10-12). A polished, animated UI at low memory, in one language from the UI down to the emulator runtime. Rejected: Qt Quick (about 80 MB of RAM on a 1 GB device), Flutter (can't load plugins at runtime, and needs its own embedder), LVGL (hard to make polished), a custom engine (too costly to build and maintain).
- **Slint's FemtoVG renderer** (2026-10-12). Rejected: Skia (about 6–10 MB more memory, and the text doesn't need it).
- **Nothing slow on the UI thread** (2026-10-12). Slint draws on the UI thread. To hold 60 fps while animating and draw nothing while idle, scanning, image decoding, file and hardware access run on other threads or are done ahead of time.
- **Logic is separate from the UI toolkit** (2026-10-12). `specular-core` holds navigation, screen models, settings, the library and the device interfaces, with no Slint code; the Slint layer only draws and passes input on. The toolkit could be replaced, and all behaviour is testable without a screen.
- **Screens are data** (2026-10-12). The core describes every screen with a few patterns (lists of rows, a grid of pictures, a sheet for long choices), which a fixed set of prefab components draws (see [look-and-motion.md](look-and-motion.md)).
- **Emulation runs in Specular's process** (2026-10-12). The core runs on its own thread and its frames are shown in the UI's scene. This is what lets a game's picture zoom straight into the live game and the in-game menu animate over it. The cost: a crashing core takes the UI down with it. The service manager restarts Specular, and the launch record says which core stopped (below).
- **A launch record says, after a crash, which core stopped with which game; the game's thread writes and removes it** (2026-10-18). As a game starts, `cache/launch.json` (core, game, start time) is written; it is removed once the game ends cleanly. At start-up, one still there (a crash, the watchdog, a power cut) is logged, shown as a notice and removed; the watchdog adds to it why it ended the game. One game runs at a time, on its own thread, so a launch quit while loading can't remove the record of the next, and a crash while loading the core is caught too. Rejected: the UI writing it (two launches overlapping could mix up their records); the app doing so (it never touches files: it gets the record in a message and shows a notice).
- **Game frames always go through the UI's scene** (2026-10-12). On the Brick this costs about 1 ms of CPU per frame more than a direct blit and adds no latency (see [brick.md](brick.md)). Rejected: drawing plain gameplay straight to the screen and handing over to the UI only for menus and transitions (a second drawing path for no gain).
- **Device-independent** (2026-10-12). A port is a device crate, `devices/<name>`, holding its profile, hardware services and display platform. Device names appear nowhere else. `specular-linux` implements the standard Linux interfaces, so most ports override only their quirks.
- **Wired headphones are an output with their own volume** (2026-10-18), like Bluetooth: the speaker and headphones each keep their level, and plugging in or out switches at once, also mid-game. The kernel's sound driver switches between them itself, each at its own level: Specular gives the device both levels whenever they change, whichever plays (`Audio::set_volume`), and a port says when headphones are plugged in (`HeadphoneJack`, from the evdev switch `SW_HEADPHONE_INSERT` in `specular-linux`), which only decides the level the volume keys and the gauge act on. Rejected: Specular setting the playing output's level on each jack change (it races the kernel's switch, and is wrong while Specular isn't running); one level for both (headphones at the ear want far less than a small speaker at arm's length, so every plug meant re-adjusting); reopening the game's sound when the jack changes (the kernel already routes it, and with no mixing the device is busy while the old stream is open).
- **Rumble is run by a thread of its own, which is handed only the latest state** (2026-10-18). The game's thread (a game's own rumble) and the UI's (effects) each set their part and never wait; the rumble thread runs the motors at the stronger of the two, scaled by the "Rumble" strength setting, and skips to the latest state when it falls behind. Setting the motors can block: on Linux a force-feedback upload to a gamepad presented by a program (the Brick's padbridge) waits until that program answers. A port gives `Hardware::rumble`; `specular-linux` drives the first gamepad that offers `FF_RUMBLE`, found by its capabilities, as the headphone jack is. Rejected: setting the motors on the game's thread (a frame would wait for padbridge); a queue of changes (it would play out every stale strength after a hold-up); the motor's own device (see [brick.md](brick.md)).
- **No async runtime** (2026-10-12). Plain threads and channels.
- **Memory: at most 50 MB for Specular itself while a game runs** (2026-10-12). Everything except the core and its game; a ceiling, not a goal. Idle memory matters less, but the design is low-memory throughout, so there is one way of working rather than two.
- **Startup is not a priority** (2026-10-12). 10–15 s is fine, as long as a good loading screen shows meanwhile.
- **Every extension point is a registry from the start; plugins come later** (2026-10-12). Screens, settings, systems and emulators, themes, and art sources.
- **GPLv3** (2026-10-12). Anything GPL we build on decides the licence: the algorithms ported from muOS's runtime are GPLv3, and Slint's GPL option is GPL-3.0-only, so Specular as a whole is GPL-3.0-only (2026-10-18). Rejected: "or later" (Slint's option doesn't allow it).
- **Specular has its own repository** (2026-10-18). Crystaline OS includes Specular and builds it for devices. Rejected: staying inside the Brick's OS repository (only ever meant as a start).
- **Specular builds itself and its cores, and installs them; a system supplies only its compiler and where to install** (2026-10-18). The `Makefile` is the whole interface (`DEVICE`, `TARGET`, `CROSS_COMPILE`, `CORE_CFLAGS`, `DESTDIR`, `PREFIX`), so the installed layout and the core patches (which add calls only Specular understands) live in one place, and working on Specular needs nothing from Crystaline OS. Rejected: Crystaline OS building the cores and laying out Specular's files.
- **The credits are worked out when a device's binary is built** (2026-10-18). Its build script (`specular-credits`) lists the Rust crates linked into it, with their licences, and adds what `data/credits.tsv` lists: Specular, the cores, fonts, the libretro-database data (CC-BY-SA) and Slint. It writes the list the credits screen shows, and `licences.txt`, every distinct licence text after the projects it covers, which is installed beside the binary. Crates are those `cargo tree` gives for the device's package and platform: normal dependencies only, since build scripts and procedural macros run during the build and ship nothing. Rejected: `cargo metadata`'s dependency graph alone (it resolves features for the whole workspace, so the Brick's credits would list the desktop's window crates); the UI crate's build script (it can't tell which device it is built for); a hand-kept list of crates (it would drift); `cargo-about` and the like (one more tool every build needs).
- **Mistakes in the repository's own credits fail the build; a crate without a licence text only warns** (2026-10-18). A core `tools/build-cores.sh` builds without a line in `data/credits.tsv` (or a line for a core it doesn't build), a licence text that isn't there, or a crate declaring no licence at all stop the build with what to fix. A crate whose package ships no licence text gets a build warning naming it and the fix (its text in `data/licences/`, named on a `crate` line), and is listed in `licences.txt` as having none. Rejected: failing on that too (updating a dependency would break a device's build over something only we can fix, by hand).

## Design

### Crates

```
crates/
  specular-core/     the app (messages in, effects out), screen models, navigation, held-button
                     repeat, list layout, settings, library, saves, box art identification, lights
                     and rumble effects, device, wireless and installer interfaces. No Slint, no
                     files or hardware touched by the app.
  specular-runtime/  the libretro runtime (see runtime.md)
  specular-credits/  works out what a binary ships and its licences; run by the device crates'
                     build scripts
  specular-ui/       Slint theme, prefabs and screens; carries out the app's effects; owns the
                     threads; art service; run(device)
  specular-linux/    standard Linux implementations: evdev (buttons, the jack, rumble), power
                     supply, LED class, ALSA, wpa_supplicant, BlueZ, framebuffer vsync and frame
                     pacing
devices/
  desktop/           Slint's default window, simulated hardware, keyboard as gamepad
  brick/             the TrimUI Brick (see brick.md)
design/              design/mockup.html, the interactive look (see look-and-motion.md)
data/db/             game tables for box art (see library-and-box-art.md)
data/credits.tsv     what Specular ships besides crates, for the credits (see Credits below)
data/licences/       licence texts not found anywhere else
tools/               build-cores.sh and its core-patches/, trim-db.py, trace-to-scenario.py
Makefile             builds Specular and its cores for a device, and installs them
```

Each device crate is a small binary that builds its device and calls `specular_ui::run`, with the credits its build script worked out.

### The app

The app is a state machine: devices and the UI feed it messages (buttons, time passing, scan results, radio events) and carry out the effects it returns (write a file, set the brightness, launch a game). It never touches files, hardware or the UI, so all of its behaviour is tested with plain messages.

```rust
impl App {
    pub fn new(systems: Vec<SystemDecl>, settings: Vec<SettingDecl>) -> Self;
    pub fn handle(&mut self, msg: Msg) -> Vec<Effect>;
    pub fn view(&self) -> Rc<ScreenView>;
    pub fn register(&mut self, id: ScreenId, screen: Box<dyn Screen>);
}

/// One kind of screen. Built-in screens implement it; plugins will register their own.
pub trait Screen {
    /// Built once and kept until the library, the settings or the screen shown change.
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView;
    /// Presses the app didn't take itself (it handles focus moves, Menu and the power button).
    fn press(&self, ctx: &mut Ctx, state: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome;
    /// Text typed on the on-screen keyboard this screen opened was confirmed.
    fn entered(&self, ctx: &mut Ctx, state: &mut ScreenState, purpose: &str, text: String) -> Result<Outcome, String>;
    /// Whether L2 and R2 jump between the rows' first letters (lists sorted by title).
    fn letter_index(&self) -> bool;
}
```

### Threads

| Thread | Work |
|---|---|
| UI | Slint, the app, uploading game frames |
| I/O | migrating, loading and saving settings (saves queued together are merged) |
| Scan | looking for the cores on the card (then writing the settings' schemas), library scans (loading the games' settings), then moving saves of renamed ROMs |
| Hardware | the device's hardware services; reads the battery every 10 s |
| One per radio | WiFi and Bluetooth commands, which can block for seconds |
| Headphone jack | waits for plugging and unplugging |
| Sounds | plays the interface sounds; opens their output at the first, closes it when idle |
| Rumble | runs the motors, at the latest state asked for (`rumble::RumbleLine`) |
| Installer | the device's installer |
| Art worker, art loader | finding and storing pictures (and the frame each game was left on); decoding pictures for display |
| Game, save writer, watchdog | the running core; its file writes; detecting a stuck core |
| Signals | turns SIGTERM and SIGINT into a normal quit, so saves are written |
| Vsync | in a display platform that paces animations by the display (the Brick's): after an animation's frame is presented, waits for the vsync that shows it and wakes the UI |

Opening a sound output can block, so a game's opens on short-lived threads, and the interface sounds' output on the Sounds thread.

### The device boundary

The only thing a port implements:

```rust
pub struct DeviceProfile {
    pub name: &'static str,
    pub width: u32, pub height: u32,  // pixels
    pub ppi: u32,                     // UI sizes scale by ppi / 400
    pub refresh_hz: f64,              // what games are paced against
    pub buttons: &'static [Button],   // hints show only these
    pub leds: &'static [Accent],      // each LED, in chain order, and which accent it shows
}

pub trait Device {
    fn profile(&self) -> DeviceProfile;
    /// Called once; the services then move to the hardware thread.
    fn hardware(&self) -> Hardware;
    fn open_sound(&self, output: &Output) -> io::Result<Box<dyn SoundOut>>;
    /// None: games are paced by a timer at their own rate. Each call opens a separate one.
    fn open_vsync(&self) -> Option<Box<dyn Vsync>>;
}

pub struct Hardware {
    pub power: Box<dyn Power>,        // battery, sleep, shutdown, restart
    pub display: Box<dyn Display>,    // brightness 1–10, panel on/off
    pub lights: Box<dyn Lights>,      // one colour per LED
    pub audio: Box<dyn Audio>,        // volume 0–20 per output
    pub wifi: Option<Box<Wifi>>,                  // see wireless.md
    pub bluetooth: Option<Box<Bluetooth>>,
    pub installer: Option<Box<dyn Installer>>,    // see installer.md
    pub headphones: Option<Box<dyn HeadphoneJack>>,
    pub rumble: Option<Box<dyn Rumble>>,
}

/// 0–65535 each, as libretro and Linux take them; a device with one motor runs the stronger.
pub struct Motors { pub strong: u16, pub weak: u16 }

pub trait Rumble: Send {
    /// Until the next call (`Motors::STILL` stops). May block: called on the rumble thread.
    fn set(&mut self, motors: Motors) -> io::Result<()>;
}

/// Where sound plays. Each has its own volume setting (all Bluetooth devices share one). The
/// speaker and the jack are one sound stream, which the kernel's driver routes.
pub enum Output { Speaker, Headphones, Bluetooth(BtAddress) }

pub trait HeadphoneJack: Send {
    /// Blocks on a thread of its own: the state at the start, then each change.
    fn watch(self: Box<Self>, plugged: Box<dyn FnMut(bool) + Send>);
}
```

Sound goes to the first connected Bluetooth device taking sound, else to headphones in the jack, else to the speaker; the volume keys and the gauge act on that output's level.

The display platform is passed to `run` separately, since it is Slint-specific. It installs a Slint platform (how pixels reach the screen) and delivers button events on the UI thread; without one, Slint's default window is used with the keyboard as buttons.

```rust
pub type InstallPlatform = Box<dyn FnOnce(Box<dyn FnMut(ButtonEvent)>) -> Result<(), Error>>;
pub fn run(device: Box<dyn Device + Send + Sync>, data: DataRoot, platform: Option<InstallPlatform>, credits: &'static [Credit]) -> Result<(), Error>;
```

3D cores will also need a GL context shared with the UI's, which the platform will have to provide (the Brick spike already made one; see [brick.md](brick.md)).

### Credits

`specular_credits::generate`, run by each `devices/<name>/build.rs`, writes two files to `$OUT_DIR`: `credits.rs`, the projects as Rust, and `licences.txt`. It also copies `licences.txt` into the folder the binary is built in, so a development build finds it beside itself as an installed one does; `make install` takes it from there. A core's version (its commit) and web address come from `tools/build-cores.sh`, and its copyright notice from its line in `data/credits.tsv`. A crate line in `data/credits.tsv` can choose which of a crate's licences Specular uses (Slint's crates: GPL-3.0-only), or give the licence texts its package lacks.

```rust
// devices/<name>/src/main.rs
const CREDITS: &[Credit] = include!(concat!(env!("OUT_DIR"), "/credits.rs"));

// specular_core::app
pub struct Credit {
    pub kind: CreditKind,              // groups the credits screen, in this order
    pub name: &'static str,
    pub version: Option<&'static str>, // a crate's version, or a core's commit
    pub licence: &'static str,         // SPDX
    pub url: Option<&'static str>,
    pub copyright: Option<&'static str>, // cores only
}
pub enum CreditKind { Specular, Core, Font, Data, Library }
pub struct Credits { pub projects: &'static [Credit], pub texts: Option<PathBuf> }
pub struct Splash { pub core: &'static str, pub copyright: Option<&'static str>, pub url: Option<&'static str> }
impl App {
    pub fn set_credits(&mut self, credits: Credits);
    pub fn splash(&self) -> Option<Splash>; // for the game being played
}
```

The UI gives the app the credits with `licences.txt`'s path, if it is beside the binary. Settings → Credits (in the System section, for everyone) lists Specular, then the cores, fonts, data and libraries, each with its licence; A shows a project's web address. The splash shown while a game starts names its core with the core's credit, found by the core's name (`CoreDecl::name`; a test checks that every core has one).

### The launch record

`specular_core::launch`, kept with the settings' power-cut-safe store:

```rust
pub struct LaunchRecord {
    pub core: String,            // "mGBA" (CoreDecl::name)
    pub library: PathBuf,        // .../cores/mgba_libretro.so (a user core's on the card)
    pub system: String,
    pub game: PathBuf,           // under roms/
    pub started: u64,            // seconds since 1970, by the device's clock
    pub reason: Option<String>,  // why Specular ended the game itself: "it stopped responding"
}
impl LaunchRecord {
    pub fn write(&self, path: &Path) -> io::Result<()>;
    pub fn remove(path: &Path) -> io::Result<()>;
    pub fn left_behind(path: &Path) -> Option<Self>;  // at start-up: logged and removed
}
// The app turns one left behind into a notice.
Msg::LastGameStopped(LaunchRecord)
```

### Registries

| Extension point | Registry today |
|---|---|
| Screens | `App::register(ScreenId, Box<dyn Screen>)`; `ScreenId` is a string so plugins can add their own |
| Settings | the `Vec<SettingDecl>` given to `App::new`: Specular's and its cores' options (`settings::declarations`; see [settings.md](settings.md)) |
| Systems and emulators | the `Vec<SystemDecl>` given to `App::new`, each naming its `CoreDecl`s; the cores the user adds to the card (`library::user_cores`; see [runtime.md](runtime.md)) join them through `Msg::Cores`, which replaces the systems and the settings |
| Art sources | the ordered `Vec<Box<dyn ArtSource>>` given to the art worker |
| Lights and rumble effects | the ordered `Vec<Box<dyn effects::Effect>>` given to `Effects::new` (see [look-and-motion.md](look-and-motion.md)) |
| Themes | not yet: there is one theme, `crates/specular-ui/ui/theme.slint` |

## Findings

- (2026-10-18, desktop) The desktop binary ships 273 projects under 189 distinct licence texts (a 645 KB `licences.txt`); the Brick's, 206 projects (507 KB). Five crates (`lyon_algorithms`, `lyon_extra`, `lyon_geom`, `lyon_path`, `taffy`) ship no licence text in their packages; theirs are in `data/licences/`, from their repositories. Source: the builds' `licences.txt`.
