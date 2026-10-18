# Architecture

Status: Implemented · Updated: 2026-10-18

How Specular is put together: the toolkit, the crates and threads, the boundary a device port implements, where plugins will plug in, and the memory and licence limits that shape all of it. Most later choices follow from two goals: the UI must stay smooth on a slow handheld, and the same code must run on any Linux device.

## Decisions

- **Slint, in Rust** (2026-10-12). A polished, animated UI at low memory, in one language from the UI down to the emulator runtime. Rejected: Qt Quick (about 80 MB of RAM on a 1 GB device), Flutter (can't load plugins at runtime, and needs its own embedder), LVGL (hard to make polished), a custom engine (too costly to build and maintain).
- **Slint's FemtoVG renderer** (2026-10-12). Rejected: Skia (about 6–10 MB more memory, and the text doesn't need it).
- **Nothing slow on the UI thread** (2026-10-12). Slint draws on the UI thread. To hold 60 fps while animating and draw nothing while idle, scanning, image decoding, file and hardware access run on other threads or are done ahead of time.
- **Logic is separate from the UI toolkit** (2026-10-12). `specular-core` holds navigation, screen models, settings, the library and the device interfaces, with no Slint code; the Slint layer only draws and passes input on. The toolkit could be replaced, and all behaviour is testable without a screen.
- **Screens are data** (2026-10-12). The core describes every screen with a few patterns (lists of rows, a grid of box art, a sheet for long choices), which a fixed set of prefab components draws (see [look-and-motion.md](look-and-motion.md)).
- **Emulation runs in Specular's process** (2026-10-12). The core runs on its own thread and its frames are shown in the UI's scene. This is what lets box art zoom straight into the live game and the in-game menu animate over it. The cost: a crashing core takes the UI down with it. The service manager restarts Specular, and a launch record says which core crashed (2026-10-18): when a game starts, Specular writes `cache/launch.json` (core, game, start time) and removes it on a clean quit; at start-up, if it is still there, Specular says which core stopped with which game, then removes it.
- **Game frames always go through the UI's scene** (2026-10-12). On the Brick this costs about 1 ms of CPU per frame more than a direct blit and adds no latency (see [brick.md](brick.md)). Rejected: drawing plain gameplay straight to the screen and handing over to the UI only for menus and transitions (a second drawing path for no gain).
- **Device-independent** (2026-10-12). A port is a device crate, `devices/<name>`, holding its profile, hardware services and display platform. Device names appear nowhere else. `specular-linux` implements the standard Linux interfaces, so most ports override only their quirks.
- **No async runtime** (2026-10-12). Plain threads and channels.
- **Memory: at most 50 MB for Specular itself while a game runs** (2026-10-12). Everything except the core and its game; a ceiling, not a goal. Idle memory matters less, but the design is low-memory throughout, so there is one way of working rather than two.
- **Startup is not a priority** (2026-10-12). 10–15 s is fine, as long as a good loading screen shows meanwhile.
- **Every extension point is a registry from the start; plugins come later** (2026-10-12). Screens, settings, systems and emulators, themes, and art sources.
- **GPLv3** (2026-10-12). Anything GPL we build on decides the licence: the algorithms ported from muOS's runtime are GPLv3, and Slint's GPL option is GPL-3.0-only, so Specular as a whole is GPL-3.0-only (2026-10-18). Rejected: "or later" (Slint's option doesn't allow it).
- **Specular has its own repository** (2026-10-18). Crystaline OS includes Specular and builds it for devices. Rejected: staying inside the Brick's OS repository (only ever meant as a start).
- **The credits screen is generated at build time from a licence list** (2026-10-18): Rust crates' licences, Slint, the cores, fonts, and the libretro-database data (CC-BY-SA).

## Design

### Crates

```
crates/
  specular-core/     the app (messages in, effects out), screen models, navigation, held-button
                     repeat, list layout, settings, library, saves, box art identification, device,
                     wireless and installer interfaces. No Slint, no files or hardware touched by the app.
  specular-runtime/  the libretro runtime (see runtime.md)
  specular-ui/       Slint theme, prefabs and screens; carries out the app's effects; owns the
                     threads; art service; run(device)
  specular-linux/    standard Linux implementations: evdev, power supply, LED class, ALSA,
                     wpa_supplicant, BlueZ, framebuffer vsync
devices/
  desktop/           Slint's default window, simulated hardware, keyboard as gamepad
  brick/             the TrimUI Brick (see brick.md)
design/              design spec and design/mockup.html
data/db/             game tables for box art (see library-and-box-art.md)
```

Each device crate is a small binary that builds its device and calls `specular_ui::run`.

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
    fn entered(&self, ctx: &mut Ctx, state: &mut ScreenState, purpose: &str, text: String) -> Result<Vec<Effect>, String>;
}
```

### Threads

| Thread | Work |
|---|---|
| UI | Slint, the app, uploading game frames |
| I/O | loading and saving settings (saves queued together are merged) |
| Scan | library scans, then moving saves of renamed ROMs |
| Hardware | the device's hardware services; reads the battery every 10 s |
| One per radio | WiFi and Bluetooth commands, which can block for seconds |
| Installer | the device's installer |
| Art worker, art loader | finding and storing box art; decoding pictures for display |
| Game, save writer, watchdog | the running core; its file writes; detecting a stuck core |
| Signals | turns SIGTERM and SIGINT into a normal quit, so saves are written |

Opening a sound output can block, so it happens on short-lived threads.

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
    /// None: games are paced by a timer at their own rate.
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
}
```

The display platform is passed to `run` separately, since it is Slint-specific. It installs a Slint platform (how pixels reach the screen) and delivers button events on the UI thread; without one, Slint's default window is used with the keyboard as buttons.

```rust
pub type InstallPlatform = Box<dyn FnOnce(Box<dyn FnMut(ButtonEvent)>) -> Result<(), Error>>;
pub fn run(device: Box<dyn Device + Send + Sync>, data: DataRoot, platform: Option<InstallPlatform>) -> Result<(), Error>;
```

3D cores will also need a GL context shared with the UI's, which the platform will have to provide (the Brick spike already made one; see [brick.md](brick.md)).

### Registries

| Extension point | Registry today |
|---|---|
| Screens | `App::register(ScreenId, Box<dyn Screen>)`; `ScreenId` is a string so plugins can add their own |
| Settings | the `Vec<SettingDecl>` given to `App::new` (see [settings.md](settings.md)) |
| Systems and emulators | the `Vec<SystemDecl>` given to `App::new`, each naming its `CoreDecl` |
| Art sources | the ordered `Vec<Box<dyn ArtSource>>` given to the art worker |
| Themes | not yet: there is one theme, `crates/specular-ui/ui/theme.slint` |
