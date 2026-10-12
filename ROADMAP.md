# Specular roadmap

Specular is a retro-gaming frontend: a launcher plus a libretro emulation runtime, for Linux handhelds. It should feel like part of the device: minimal and polished, with seamless transitions into and out of games, and frugal with memory and battery. It works on any device. The TrimUI Brick (Hammer edition) is the reference device and the first port.

## Decisions

- **Stack: Slint, in Rust.**
  - Qt Quick was rejected because it costs about 80 MB of RAM on a 1 GB device. Flutter can't load plugins at runtime and needs its own embedder. LVGL is hard to make polished. A fully custom engine was rejected because of how much it would cost to build and maintain.
  - Slint draws on the UI thread. To stay at 60 fps while animating and about 0 fps when idle, all heavy work (scanning, image decoding, file and hardware access) runs on other threads or is done ahead of time.
- **Logic is separate from the UI toolkit.** `specular-core` holds navigation, screen models, settings, the library and the hardware interfaces, and has no Slint code. The Slint layer only draws and passes input on, so the toolkit could be replaced later.
- **Screens are data.** The core describes every screen using a few patterns:
  - lists of rows (header, text, toggle, choice, number, action);
  - a grid of box art;
  - a sheet (a picker for long lists).

  A fixed set of prefab Slint components draws them, and one theme and motion system drives every animation.
- **Emulation runs in the same process.** The libretro core runs on its own thread and draws to an off-screen texture that the UI shows. This is what lets box art zoom straight into the live game and the in-game menu animate over it.
  - The cost of drawing each game frame through the UI is still to be measured on the Brick. If it is too high, plain gameplay draws straight to the screen and the UI takes over only for transitions and menus.
  - The cost of this design is that a crashing core takes the UI down with it. The service manager restarts Specular, and a launch record left on the card lets it say which core crashed.
- **The runtime is our own clean-room libretro runtime**, using GPLv3 code from muOS's runtime ("Pickles") for the hard parts: audio/video rate control, rendering for 3D cores, and save states. NextUI is reference only: MinUI-derived code has no licence, and NextUI itself has been PolyForm Noncommercial since 2026-08-15. Cores are chosen system by system.
- **Device-independent.** A port supplies a device crate, `devices/<name>`, holding its screen profile, its hardware services and its display platform. Device names appear nowhere else. `specular-linux` implements the standard Linux interfaces (evdev, power supply, LED class, ALSA), so most ports override only their quirks.
- **Look: the Light Bar direction** (see `design/`). It is monochrome with one user-chosen accent. A thin glowing bar along the top edge mirrors the device's LED bar, and the LEDs show the same colour. The type is Geist with Geist Mono for numbers. Focus plates copy the Brick Hammer's case: sharp top corners, rounded bottom corners. Motion is short, with a slight overshoot. The Instrument direction was rejected as cluttered: too many typefaces, and hard to read at real size.
- **Licence: fully open source.** Anything we build on that is GPL decides the licence: GPLv3 for Pickles, and Slint's GPLv3 option. The details are for later.
- **Repository:** Specular stays in this repo for now and will be split off later.
- **Memory:** while a game runs, Specular itself (everything except the core and its game) uses as little as possible, under an upper bound of 50 MB (a ceiling, not a goal). Idle memory matters less, but the design is low-memory throughout, so there is one way of working rather than two.
- **Startup:** not a priority. 10–15 s is fine, as long as a good loading screen shows while things start.
- **Plugins come later,** but every extension point is a registry from the start: screens, settings, systems and emulators, themes, and art sources.

## SD card layout

| Folder | Contents |
|---|---|
| `roms/` | Games, scanned recursively. The system is worked out from the file extension, and from the header where an extension is shared. |
| `bios/` | BIOS files |
| `saves/` | Battery saves (`.srm`) and save states, in a directory tree that mirrors `roms/`, with each file named after its ROM (e.g. `roms/gba/Golden Sun.gba` → `saves/gba/Golden Sun.srm`). Smarter matching can come later. |
| `configs/` | Every setting and override as plain JSON, layered: built-in defaults < global < per system < per game. Only overrides are stored. Unknown keys are kept. |
| `cache/` | Things Specular can rebuild: box art, the library index. Safe to delete. |

Settings are declared once, with a key, type, default, label and visibility (user, advanced or developer). The settings screens are generated from those declarations, so a setting needs no hand-written UI. A setting appears only where people genuinely disagree. Everything else has a good default or is detected automatically. Developer details (frame time, GPU, memory, file paths) appear only on request.

Writes must survive a power cut on FAT32: write a temporary file, fsync it, rename it over the original, then fsync the directory.

## Box art

Decided 2026-10-12:
- **Source:** libretro-thumbnails, box art only to start with. Specular downloads an image from the device over Wi-Fi when the cache doesn't have it. There's no PC companion yet.
- **Rules:** never bundle or mirror the art, because it is the publishers' copyright. RetroArch's on-demand download is the precedent we follow.

Box art is found in two steps, each with a hook for every system:

1. **Identify the ROM**, once per file. The result is cached, keyed by path, size and modification time. Methods, in order:
   1. CRC32: read from zip headers for free, otherwise hashed for files of 64 MB or less.
   2. Disc serial, for discs too large to hash: PS1 `SYSTEM.CNF`, PSP `PARAM.SFO`, or the Saturn/Dreamcast header.
   3. File name: exact, then normalised (region and revision tags stripped, articles and numerals made consistent), then fuzzy, with a preferred region.

   The lookup data is libretro-database, which is licensed CC-BY-SA: we credit it, and data derived from it keeps that licence. Specular ships only the supported systems' data, mostly 1–8 MB each, so identification works offline. The result is the game's No-Intro/Redump name and how confident the match is.
2. **Find the art.** Sources, tried in order, first hit wins:
   1. An image the user supplied.
   2. `cache/`.
   3. Art extracted from the ROM, for systems that have any: PSP `ICON0`/`PIC1`, DS and 3DS icons, the GameCube banner. Cartridge systems up to the SNES and Mega Drive have none.
   4. libretro-thumbnails (needs the network).
   5. Later: a screenshot Specular takes when you leave a game.

   Images are shrunk to preview size as they are stored, so the UI only loads small, ready-to-use files.

How it behaves:
- Missing art is queued and fetched whenever Wi-Fi is up.
- Downloads go one at a time, honour ETags (so unchanged images aren't fetched again) and send an identifying User-Agent.
- A fuzzy match never replaces an image the user supplied.

Address: `thumbnails.libretro.com/<db system name>/Named_Boxarts/<name>.png`. In the name, each of `&*/:`<>?\|"` is replaced by `_`.

Edge cases:
- **Name matching:** sequels collapsing in fuzzy matching ("Super Mario Bros." vs "… 2"), and region variants.
- **Unusual games:** hacks and translations (the base game's art is acceptable), multi-disc `.m3u` sets, arcade short names (they need a lookup table), betas and prototypes.
- **Discs:** a CHD's built-in checksum matches Redump only if it was made from the same tracks.

Later:
- ScreenScraper as an optional source: better art and checksum lookups. It needs a developer ID for the project, which we must request from them, plus each user's own account, and it has daily quotas.
- Title screens and screenshots as other art kinds.
- Not planned: LaunchBox (no licence for third-party use), IGDB (its secret can't ship in open source), MobyGames (paid), TheGamesDB (name search only).

## Architecture

```
specular/
  design/                  design spec and the HTML mockup
  crates/
    specular-core/         models, navigation, input repeat, settings, library, art, registries,
                           theme tokens, App (messages in, effects out), device interfaces. No Slint.
    specular-runtime/      the libretro runtime boundary: frame handoff, the Runtime trait;
                           later the interface to the C runtime
    specular-ui/           Slint theme, motion, prefabs and screens; turns core views into
                           Slint models; image loading; run(device)
    specular-linux/        standard Linux implementations: evdev, power_supply, leds, ALSA
  devices/
    desktop/               winit window, simulated hardware, keyboard as gamepad (development)
    brick/                 TrimUI Brick: EGL on the PowerVR display, backlight, LED zones, profile
```

- Each device crate is a small binary that builds its device and calls `specular_ui::run`.
- No async runtime: plain threads and channels. The threads are UI, I/O (settings, scan, art), image decoding (2), hardware polling, and the runtime.

The device boundary is the only thing a port implements:

```rust
pub struct DeviceProfile {
    pub name: &'static str,
    pub screen: Size,                 // pixels
    pub ppi: u16,                     // sizes scale by ppi / 400, so text keeps its physical size
    pub refresh_hz: f32,              // what the runtime paces against (Brick: about 60.2)
    pub buttons: &'static [Button],   // hints show only buttons the device has
    pub leds: Option<LedLayout>,
}
pub trait Device { fn profile(&self) -> &DeviceProfile; fn hardware(&self) -> Hardware; }
/// Slint side: how pixels reach the screen and input reaches the app. None = Slint's default.
pub trait DevicePlatform {
    fn platform(&self, input: InputSink) -> Option<Box<dyn slint::platform::Platform>>;
    fn shared_gl_context(&self) -> Option<Box<dyn SharedContext>>;  // None: CPU frames only
}
```

Box art is two interfaces, both registered per system:

```rust
pub trait Identifier: Send + Sync { fn identify(&self, rom: &RomFile, db: &GameDb) -> Option<GameIdentity>; }
pub trait ArtSource: Send + Sync {
    fn id(&self) -> &'static str;
    fn needs_network(&self) -> bool;
    fn find(&self, game: &GameIdentity, rom: &RomFile, kind: ArtKind) -> Result<Option<ArtImage>, ArtError>;
}
```

## Work

### 1. Design: done

- Spec and interactive mockup in `design/`, published as a private artifact.
- Light Bar chosen.
- Next: trim the mockup and spec to Light Bar only.

### 2. Brick spike: next

The spike checks that the stack works on the reference device. It lives in `devices/brick/examples/` and stays as a regression check.

**Brick facts it builds on:**
- The GPU libraries support OpenGL ES 3.2 and EGL 1.4, but the only window system is PowerVR's NULL one: drawing goes straight to `/dev/fb0`, with no DRM, Wayland or X11.
- The display runs at about 60.2 Hz.
- On this driver, `eglGetProcAddress` can return NULL for core GLES functions.

**Dev loop:**
1. `tools/run-specular.sh` pushes a binary over adb.
2. It stops the test card's frontend service and runs the binary in the foreground.
3. It restarts the service afterwards.

**Toolchain:**
- Rust `aarch64-unknown-linux-gnu`, linked with the cross toolchain from `tools/env.sh`, which uses glibc 2.23 (the device has 2.33 or newer). The fallback is `cargo-zigbuild`.
- The binary is dynamic, because it loads the GPU libraries and the cores at runtime.

**Steps:**
1. Slint's FemtoVG renderer on our EGL setup. Check that it is vsync-locked and that idle really means no redraws.
2. GL function lookup: core functions through `dlsym`, extensions through `eglGetProcAddress`.
3. evdev input inside our own event loop. Measure the time from button press to the first changed frame.
4. A shared GL context on a worker thread, with fences, shown through a Slint borrowed texture.
5. Game-frame cost, with test patterns at 60.0988 and 50 Hz:
   - "composite" mode: every frame is drawn through Slint;
   - "direct" mode: the frame is drawn straight to the screen.

   For each mode, measure the UI-thread time per frame, GPU time per frame, the timing between screen updates, and input latency.
6. Memory (Specular under 50 MB while a game runs), startup time, text quality from 26 to 160 px, and the cost of glows.
7. Whether contexts and textures survive sleep.

**Fallbacks:**
- Text quality: if FemtoVG's text isn't good enough, use the Skia renderer, which costs about 6–10 MB more.
- Frame cost: if drawing game frames through Slint costs too much, use direct mode during gameplay.

### 3. Skeleton

1. Workspace and core models, navigation and held-direction repeat, with tests.
2. Settings: declarations, the layered JSON store, generated settings rows.
3. Library scan and the system table, starting with GB, GBC, GBA, NES, SNES, MD and PS1.
4. Hardware interfaces with a simulated version and the Brick version: battery, backlight, LEDs, sleep (including ignoring the power key briefly after waking).
5. The Slint theme and prefabs from the Light Bar tokens, and the image loader.
6. A test-pattern runtime: launch zoom, in-game menu, quit.
7. The app with desktop and Brick builds, and a screen for when there's no SD card.
8. `make specular`; the test card's frontend service switches from `status` to Specular.

### 4. Library and box art

1. Identification: CRC, disc serial, file names.
2. The trimmed libretro-database.
3. The art sources and the `cache/` layout.
4. The download queue that runs when Wi-Fi is up.
5. Saves in the mirrored `saves/` tree, including moving them when a ROM is renamed or moved.

### 5. Runtime

1. The libretro runtime in C, behind `specular-runtime`: audio/video rate control and save states, borrowing from Pickles.
2. 3D-capable cores with a shared GL context.
3. Per-system cores and defaults.
4. Pausing on sleep.
5. Periodic saves of battery-backed game memory.
6. A watchdog for a core that stops producing frames.

### Later

- Plugin loading through the existing registries.
- ScreenScraper.
- Title screens and screenshots as art.
- Sounds.
- A letter index for long lists.
- Light bar reactions in-game, such as low battery or a save.
- Fonts for non-Latin game names: CJK fonts cost 15 MB or more.

## Open questions

- Whether folder names count as a hint when working out a ROM's system.
- After repeated crashes, should the test card fall back to `status`?
