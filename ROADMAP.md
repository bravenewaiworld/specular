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
- **The runtime is our own libretro runtime, in Rust** (`specular-runtime`), decided 2026-10-13. It ports the algorithms of muOS's runtime ("Pickles", the `retro/` folder of MustardOS/frontend, GPLv3) for the hard parts: audio/video rate control, frame pacing and battery-save writing. Pickles' files can't be reused as they are: it is about 53,000 lines of C tied to muOS's own UI. NextUI is reference only: MinUI-derived code has no licence, and NextUI itself has been PolyForm Noncommercial since 2026-08-15. Cores are chosen system by system.
- **Cores (2026-10-13): mGBA for GB, GBC and GBA.** One core (MPL-2.0) for all three. It is the most accurate GBA core, and it detects Pokémon Emerald hacks itself (128 KiB flash and the cartridge clock). Gambatte, the usual GB choice, is GPL-2.0-only, which clashes with GPLv3 code in the same process; gpSP is lighter and can come later as a per-game choice if a game needs it.
- **Sound** goes through libasound, loaded at runtime like the GPU libraries, so the same code runs on the Brick and on a desktop.
- **Device-independent.** A port supplies a device crate, `devices/<name>`, holding its screen profile, its hardware services and its display platform. Device names appear nowhere else. `specular-linux` implements the standard Linux interfaces (evdev, power supply, LED class, ALSA), so most ports override only their quirks.
- **Look: the Light Bar direction** (see `design/`). It is monochrome with one user-chosen accent. A thin glowing bar along the top edge mirrors the device's LED bar, and the LEDs show the same colour. The type is Geist with Geist Mono for numbers. Focus plates copy the Brick Hammer's case: sharp top corners, rounded bottom corners. Motion is short, with a slight overshoot. The Instrument direction was rejected as cluttered: too many typefaces, and hard to read at real size.
- **Licence: fully open source.** Anything we build on that is GPL decides the licence: GPLv3 for Pickles, and Slint's GPL-3.0-only option, which makes Specular GPLv3 as a whole. The details are for later.
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

Decided 2026-10-13:
- **Plain HTTP**, no TLS: the server sends the same files either way, and a TLS stack would add about 1.5 MB, root certificates to keep current, and a dependence on the clock being set.
- **The lookup tables are compiled in.** `tools/trim-db.py` trims libretro-database (pinned to one commit) to CRC, serial and name per system, in `data/db/` (CC-BY-SA, credited in `data/db/NOTICE`).
- **The user's own art goes next to the ROM,** with the same name: `roms/gba/Golden Sun.png`. PNG only for now.
- **The whole library is fetched,** games on screen first, then the rest in the background, one at a time, paused while a game runs.
- **Header codes identify ROM hacks** as their base game (a GBA header's 4-letter game code is kept by hacks), which gives them the base game's art.

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

### 2. Brick spike: done (2026-10-12)

The spike checked the stack on the reference device. It lives in `devices/brick/` (the platform in `src/`, the measurements in `examples/`) and stays as a regression check. Build it with `make specular`; `tools/run-specular.sh <binary> [args]` runs a build on a Brick over adb, with the test card's frontend service stopped.

Brick facts it builds on: the GPU libraries offer OpenGL ES 3.2 and EGL 1.4 with only PowerVR's NULL window system (drawing goes straight to `/dev/fb0`; no DRM, Wayland or X11), and the display runs at about 60.2 Hz.

Results, with the CPU at the 600–816 MHz the `ondemand` governor picks for this load:

| Check | Result |
|---|---|
| Toolchain | Rust `aarch64-unknown-linux-gnu`, linked with the cross toolchain in `tools/env.sh` (GCC 6.4, glibc 2.23); the binary needs glibc 2.18 symbols at most. Dynamic, since it loads the GPU libraries (and later the cores) at runtime |
| Slint on our EGL | FemtoVG on a GLES 3 context: 60.2 fps, locked to vsync, no late frames. A full redraw costs about 4 ms of CPU; the rest of each 16.6 ms is waiting for vsync. Idle draws nothing: about 0.5 % CPU |
| GL function lookup | `eglGetProcAddress` also returns core functions in our own context; the lookup tries `libGLESv2` first anyway, which costs nothing |
| Input | evdev read inside our event loop; a press reaches the screen in 7–10 ms on average (worst 12 ms), measured from the kernel's timestamp to the end of the present, with injected presses (padbridge's own hop not included) |
| Frames from another thread | An emulator thread with a shared, surfaceless GL context uploads CPU-made frames into textures and hands them over with a GL fence; Slint shows them as borrowed textures. A texture goes back to the emulator only after the present that replaced it, with a fence. No tearing: each read-back rendered frame holds exactly one emulator frame |
| Composite vs direct | Drawing each game frame through the Slint scene costs 2.8–3.3 ms of UI-thread CPU per frame, against 2.1–2.3 ms for our own one-quad blit. With the emulator paced by the display, the latency is the same in both: a press reaches the end of the present 24–25 ms later on average (33 ms worst). An emulator on its own clock (60.0988 Hz) drifts against the display (60.2 Hz), and its frames sometimes wait a whole refresh: 21–38 ms on average |
| Display queue | Drawing and then swapping, as every toolkit does by default, puts a frame on screen 32 ms (two vsyncs) after it was started: the swap waits for the previous frame's flip, and the new frame flips at the vsync after that. Starting each frame only once the previous one is on screen takes 16 ms at the same 60 fps. Also delaying the start by 10 ms (a "frame delay", so the work and the reading of the buttons happen later) takes 6 ms, still at 60 fps; at 13 ms, frames start to miss the vsync. Measured with the display driver's record of the flip (`/dev/fb0`'s pan offset) |
| Vsync event | Our kernel now implements the standard `FBIO_WAITFORVSYNC` on `/dev/fb0`: it returns when the display takes a new frame (buffer switches follow it by a median 0.1 ms). Waiting for it, then drawing: the swap no longer blocks (about 1 ms), 60 fps, 16.6 ms from frame start to on screen; with a 10 ms frame delay, 6.5 ms at 60 fps |
| Memory, start-up | 20 MB resident with the scene, fonts and frame textures. First frame on screen about 1.1 s after `main()` starts |
| Text | Geist from 27 to 96 px is clean on the panel |
| Sleep | The GL context and textures survive sleep, and rendering resumes without errors |

Proposed from these results (to confirm):
- **Keep FemtoVG.** The Skia renderer would cost about 6–10 MB more, and the text doesn't need it.
- **Always composite.** Slint adds about 1 ms of CPU per frame and no latency, so gameplay frames go through the Slint scene. The second, direct path isn't needed.
- **Pace everything by the display, not by draw-then-swap.** Latency comes from when frames are started, not from the toolkit. Our platform owns the event loop, so it can start each frame (the emulator's work, then the UI's draw) at a set time after the previous flip, with a frame delay that adapts to how long the core takes. Measured on this hardware, that cuts the start-to-screen time from 32 ms to about 6 ms. The vsync event is `FBIO_WAITFORVSYNC` (implemented in our kernel for this; see "Vsync event"). On a device whose kernel lacks it, the time a blocking swap returns gives the vsync phase.

Press to photons, by the same measurements: the press waits about 8 ms on average for the next frame to read it, then the frame takes 32 ms (draw-then-swap) or about 6 ms (paced, with a 10 ms frame delay) to go on screen, then scanout takes up to 16 ms to reach a given row. That totals about 48 ms or about 22 ms to the middle of the screen. Confirming it with a camera (a video of at least 240 fps of an LED and the screen) is deferred until such a camera is available.

Found along the way:
- **Fonts.** TrimUI's stock system has fontconfig but no fonts, and Slint's text layout (fontique) panics in that case. `devices/brick/src/fonts.rs` gives fontconfig one of our fonts. Fontconfig is loaded at runtime, so our own image can leave it out.
- **Slint's licence** option is GPL-3.0-only, not "or later", so Specular as a whole is GPLv3.
- **WiFi after waking.** After `sleepnow` restarts WiFi, the WiFi driver can leave the system in a state where every swap waits about two vsyncs (25–50 fps instead of 60), with "miss interrupt" warnings from the XR829. Restarting WiFi again fixes it. Tracked in the repo-wide `ROADMAP.md`.
- **adb.** The stock adbd kills a session's processes when the session exits. Long or detached runs (such as sleep tests) go over SSH.

### 3. Skeleton

1. Workspace and core models, navigation and held-direction repeat, with tests.
2. Settings: declarations, the layered JSON store, generated settings rows.
3. Library scan and the system table, starting with GB, GBC, GBA, NES, SNES, MD and PS1.
4. Hardware interfaces with a simulated version and the Brick version: battery, backlight, LEDs, sleep (including ignoring the power key briefly after waking).
5. The Slint theme and prefabs from the Light Bar tokens, and the image loader.
6. A test-pattern runtime: launch zoom, in-game menu, quit.
7. The app with desktop and Brick builds. With no SD card (the data folder missing or empty, as a device's unused mount point is), the library says so.
8. `make specular`; the test card's frontend service switches from `status` to Specular (2026-10-13: it runs Specular, and falls back to `status` until the next boot after 3 failures within a minute).

Long lists (2026-10-12): the core's list layout gives the rows within a screen of the view, and the UI draws them from a fixed set of about 30 reusable row slots (each row keeps its slot while in view), so a list's cost doesn't grow with its length. The app builds a screen's rows once and keeps them until its content changes; a focus move takes about 15 µs (desktop) with 3,000 games. On the Brick, 3,000 games use 22 MB resident (21 MB with none); scrolling continuously at 20 moves a second uses about 47 % of one core at the governor's low clock, mostly drawing.

### 4. Library and box art: done (2026-10-13)

1. Identification: CRC (zips from their directory), the GBA header's game code, file names. Disc serials are for later.
2. The trimmed libretro-database (`tools/trim-db.py`, `data/db/`), compiled in.
3. The art sources and the `cache/` layout: the user's PNG next to the ROM, then `cache/art/` (mirroring `roms/`, 400 px), filled from libretro-thumbnails; what is known per ROM is in `cache/identify/`.
4. The download queue: games on screen first, then the rest, one at a time; it waits a minute after a network error and pauses while a game runs.
5. Saves in the mirrored `saves/` tree. Saves follow a renamed or moved ROM: `saves/.specular-saves.json` records the size and checksum of each ROM played, and after a scan the saves of a ROM that is gone move to the one game without saves with the same file.

### 5. Runtime

1. Done: the libretro runtime, in Rust (`specular-runtime`): sound resampled to 48 kHz with the game's speed matched to the display and dynamic rate control, battery saves, save states. The headless player (`play` example) reports a core's time per frame.
2. Done, measured on the Brick (2026-10-14): GBA games run at 61 fps (the panel's 60.2 Hz) beside the UI, mGBA taking a median 5-7 ms a frame (95 % under 9-15 ms) at the 816 MHz the `ondemand` governor picks, with about 30-50 ms of sound queued. Specular uses about 29 MB of the 66 MB resident while a game runs (the rest is mGBA and its game), 31 MB in the menus. mGBA gets the ROM's path and, with our patch (`tools/core-patches/mgba`), maps the file instead of copying it, so most of a game's 32 MB ROM is file pages the kernel can drop (2026-10-14; before, mGBA held two 32 MB copies). Frame pacing driven by the display's vsync: the core runs once per vsync, started after a frame delay that adapts to how long the core takes, and the UI draws as soon as the frame is ready. Uses `FBIO_WAITFORVSYNC` (in the Brick's kernel since the spike). A camera check of press-to-photon latency is deferred (it needs a camera of at least 240 fps).
3. 3D-capable cores with a shared GL context.
4. Per-system cores and defaults: mGBA for GB, GBC and GBA (`tools/build-cores.sh`). The MVP plays these three (decided 2026-10-13); systems with no core are left out of the library. NES, SNES, Mega Drive and PS1 come after it, each with a licence and speed check: the usual cores are non-commercial (snes9x, Genesis Plus GX, PicoDrive) or possibly GPLv2-only (FCEUmm, PCSX ReARMed); bsnes, BlastEm, QuickNES or Nestopia, and SwanStation are the GPLv3-compatible candidates.
5. Done: pausing on sleep, writing the battery save first; waking reopens the in-game menu.
6. Done: battery saves written every 10 s when changed, on quit, on sleep and on a stop signal.
7. Done: a watchdog: a core that spends 6 s on one frame (stuck in a loop or a deadlock) makes Specular exit, and the service manager restarts it.

### Later

- Plugin loading through the existing registries.
- ScreenScraper.
- Box art: fuzzy name matching (sequels and region variants make it risky), refreshing stored art by ETag, JPEG user images, PS1 disc serials.
- Title screens and screenshots as art.
- Sounds.
- A letter index for long lists.
- Light bar reactions in-game, such as low battery or a save.
- Fonts for non-Latin game names: CJK fonts cost 15 MB or more.

## Deferred optimizations

Minor or unproven savings, left until profiling shows they matter.

| Where | Observation | Idea |
|---|---|---|
| Scrolling | Continuous scrolling at 20 moves a second uses about 47 % of one core on the Brick at the governor's low clock, mostly drawing (2026-10-12) | Profile FemtoVG's per-frame cost: text layout cache hits, how many row slots redraw per move, whether off-screen slots still draw |

## Open questions

- Whether folder names count as a hint when working out a ROM's system.
