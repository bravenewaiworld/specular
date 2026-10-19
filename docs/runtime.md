# Runtime

Status: In progress · Updated: 2026-10-18

Specular's own libretro runtime (`specular-runtime`): how it loads a core, paces it by the display, plays its sound, and keeps its saves and states safe, and which cores it runs. It decides how games feel (input latency, smoothness, sound) and how safe the player's progress is.

## Decisions

- **Our own libretro runtime, in Rust** (2026-10-13). It ports the algorithms of muOS's runtime ("Pickles", the `retro/` folder of MustardOS/frontend, GPLv3) for the hard parts: sound and video rate control, frame pacing and battery-save writing. Rejected: reusing Pickles' files (about 53,000 lines of C tied to muOS's own UI); building on NextUI (MinUI-derived code has no licence, and NextUI itself has been PolyForm Noncommercial since 2026-08-15), which stays a reference only.
- **Cores are chosen system by system: mGBA for GB, GBC and GBA** (2026-10-13). One core (MPL-2.0) for all three; the most accurate GBA core, and it detects Pokémon Emerald hacks itself. The first release plays these three systems. Rejected: Gambatte for GB (GPL-2.0-only, which clashes with GPLv3 code in the same process); gpSP for GBA (lighter; it may come later as a per-game choice if a game needs it).
- **Nestopia for the NES** (2026-10-18). GPL-2.0-or-later, with LGPL-2.1-or-later and MIT code, so it can share a process with GPLv3 code. It is the more accurate of the two candidates (cycle-based, with the most boards, and the Famicom Disk System), and the NES is light enough that accuracy costs little: 0.5 ms a frame on a desktop (see Findings). Rejected: QuickNES (its licence is fine too, LGPL-2.1-or-later and GPL-2.0-or-later; it is faster but less accurate, drawing a scanline at a time with fewer boards, and the speed isn't needed).
- **SwanStation for the PS1, with its software renderer** (2026-10-18). GPL-3.0; the code it bundles is BSD, MIT, public domain or GPL-2.0-or-later (see Findings), and it has a recompiler for ARM64. Its hardware renderers need a GL context shared with the UI, which comes with 3D cores (V1). Rejected: PCSX ReARMed (possibly GPLv2-only).
- **A PS1 game starts without a BIOS, on SwanStation's built-in OpenBIOS** (2026-10-18). SwanStation runs OpenBIOS (MIT) when `bios/` has no BIOS, so games start, though some don't run with it; Specular logs the files it looked for. Rejected: refusing to start PS1 games without a BIOS (it would stop games that run, and SwanStation also finds a known BIOS dump under any name, which a check of file names can't see). Famicom Disk System games have no such fallback: without `disksys.rom` they don't start, and the player is told which file to add.
- **SNES and Mega Drive wait for V1** (2026-10-18). They need more research: bsnes is likely too slow for the Brick's Cortex-A53, and BlastEm's 68000 emulation is an x86 recompiler (both inferred, unmeasured); Beetle Supafaust and ClownMDEmu are candidates too. Rejected: bsnes and BlastEm for V0 (speed).
- **Battery saves are protected from cores whose save states also restore save memory** (2026-10-18). Loading such a state rolls the save memory back, and writing it would lose newer in-game saves. Each save state is stored with the crc32 of the save memory as it was when the state was made. For the cores flagged (`CoreDecl::states_restore_save`: Nestopia and SwanStation; not mGBA), the first time the `.srm` is written after a state was loaded, an `.srm` on the card that is neither the state's (by its crc32) nor what is about to be written is a newer save, and is first renamed to `<name>.newer.srm`, so it isn't lost. A state without a crc32 counts as not matching. Rejected: RetroArch's copy-back of the save memory after loading (restores only the bytes, not the core's own save-chip state; see Findings); blocking writes after a load.
- **Games are paced by the display's vsync, with an adaptive frame delay** (2026-10-12). Latency comes from when frames are started, not from the toolkit. The core runs once per vsync, started as late as it safely can, so buttons are read late and a press reaches the screen sooner. On the Brick this cuts start-to-screen from 32 ms to about 6 ms (see [brick.md](brick.md)). Rejected: draw-then-swap, as toolkits do by default (frames queue for two vsyncs); a core on its own clock (it drifts against the display, and its frames sometimes wait a whole refresh).
- **The game's speed is matched to the display** (2026-10-13). A game within 5 % of the display's rate runs at the display's rate, its sound resampled to match (a Game Boy game at 59.73 fps plays 0.8 % fast on a 60.2 Hz panel). Further off, a timer paces it at its own rate.
- **Sound goes through libasound, loaded at runtime** (2026-10-13), like the GPU libraries, so the same code runs on the Brick and on a desktop, and runs silently without it.
- **Battery saves are written when the game writes them** (2026-10-13), on a thread of their own, never the game's: flushing to an SD card can take longer than a frame.
- **An autosave every 10 s of play; games resume from it** (2026-10-13). Also written on sleep and on quit.
- **Save states live in a hidden `.states/` folder beside the ROM** (2026-10-13), named after it. Battery saves stay in `saves/`.
- **Save states are zstd-compressed (level 1)** (2026-10-14), about 11 times less wear on the card or internal storage, as the autosave is written every 10 s. Battery saves stay uncompressed, so other emulators and tools can read them. Rejected: uncompressed states (readable by other frontends, but far more writes).
- **mGBA gets only the ROM's path, and maps the file** (2026-10-13). Rejected: handing it the contents, as libretro allows (it copies them, and copies a GBA ROM again on the first write to its cartridge area, as Pokémon games' clock does: 64 MB for a 32 MB ROM).
- **A zipped ROM is unzipped into memory, and the core given its `/proc/self/fd/<n>` path** (2026-10-18). A memfd holds it, so nothing is written to the card and mGBA still maps the file (its pages are shared, not copied). Only for a core that can't open zips (none among its `valid_extensions`, and no `block_extract`), as libretro has frontends do; the file inside is the first with one of the core's extensions. A ROM that would leave less than 64 MB of memory available (`MemAvailable`) is refused. Saves stay named after the zip. Rejected: building mGBA with zip support (every core would need its own, and our mapping patch works on files); unzipping to a file on the card (writes, and a leftover file after a crash); a temporary file under `/tmp` (tmpfs: the same memory, plus a file to clean up).
- **Cores' log messages go into Specular's log, through a C shim** (2026-10-18). libretro's log callback is C-variadic, which Rust 1.98 can't define on stable, so `src/log.c` (built with `cc`) formats the message and hands it to Rust. Lines read `core: <core> <level>: <message>`. Debug messages are dropped before they are formatted, and at most 30 are written a second, the rest counted and the count logged, so a core logging every frame costs little and can't flood the log. Rejected: nightly Rust (`c_variadic`); a log thread (writing a line to stderr is cheap, and the cap bounds how many).
- **Games rumble through libretro's rumble interface, for the first player** (2026-10-18). The core's strong and weak motor strengths go to the rumble thread (see [architecture.md](architecture.md)). The game's rumble is stilled while it is paused (its menu, sleep) and restored on resuming (what the core asks meanwhile, resetting or loading a state from the menu, is kept for then), and stilled when it ends, by the frontend being removed, which also covers a core that fails to load. A crash or the watchdog ending Specular closes its rumble device, and the kernel stops the effect then. Rejected: leaving the rumble on over the menu (a held strength would buzz until the game resumes).
- **Cores declare their options in libretro's version 2, and can change them mid-game** (2026-10-18). Version 2 carries each option's info text, categories and value labels, which go into the settings' schemas (see [settings.md](settings.md)); versions 0 and 1 are read too, for cores that use them. A change is handed to the game's thread and the core told through GET_VARIABLE_UPDATE, which cores ask about each frame. Rejected: version 0 (a description and bare values only); restarting the game to change an option (some cores read theirs only at launch anyway, and say so).
- **A watchdog ends Specular if a core spends 6 s on one frame** (2026-10-13). A stuck core's thread can't be stopped; the service manager restarts Specular. The battery save is then as the game last wrote it, at most about 10 s old.

## Design

### Running a game

One game runs at a time, on a thread of its own: a core keeps its state in globals, and loading its library again gives the same, already loaded, copy. libretro's callbacks carry no context, so the game's state is kept in a thread-local on the game's thread, where every call into the core is made.

```rust
pub struct LaunchSpec {
    pub core: PathBuf,                  // e.g. cores/mgba_libretro.so
    pub rom: PathBuf,                   // a zip is unzipped into memory first (see Decisions)
    pub save: PathBuf,                  // battery save (.srm)
    pub autosave: Option<PathBuf>,      // resumed from at launch
    pub system_dir: PathBuf,            // bios/
    pub options: Vec<(String, String)>, // core options; the rest keep the core's defaults
    pub option_list: Option<&'static CoreOptions>, // what the core should declare (data/cores/); differences are logged
    pub rom_by_path: bool,              // only for cores known to read the file themselves
    pub record: Option<(PathBuf, LaunchRecord)>, // the launch record (see architecture.md)
    pub states_restore_save: bool,      // keep a newer .srm when a state rolls it back (see Saves and states)
}
pub struct Outputs {
    pub sound: Option<Box<dyn SoundOut>>,
    pub vsync: Option<Box<dyn Vsync>>,
    pub display_hz: f64,
    pub frames: Arc<FrameSlot>,
    pub on_frame: Box<dyn Fn() + Send>,  // called on the game's thread after each new frame
    pub rumble: Option<RumbleLine>,      // None: the core is offered no rumble
    pub on_save: Box<dyn Fn() + Send>,   // the game saved in-game (for the effects)
}
impl Session {
    pub fn start(spec: LaunchSpec, out: Outputs) -> Result<Session, String>;
    pub fn watch(&self, limit: Duration, stuck: impl FnOnce() + Send + 'static);
    pub fn set_buttons(&self, bits: u16);
    pub fn take_sound_peak(&self) -> u16;     // the loudest sample since the last call, for the effects
    pub fn pause(&self, paused: bool);
    pub fn set_sound(&self, out: Option<Box<dyn SoundOut>>);  // open it beforehand: opening can block
    pub fn set_options(&self, options: Vec<(String, String)>); // as LaunchSpec::options; the core reads them again (GET_VARIABLE_UPDATE)
    pub fn save_state(&self, path: &Path) -> io::Result<()>;
    pub fn load_state(&self, path: &Path) -> io::Result<()>;
    pub fn flush(&self) -> io::Result<()>;   // battery save and autosave, on the card when it returns
    pub fn quit(self) -> io::Result<()>;     // dropping a Session quits too
}
```

A zip that can't be unzipped fails the start with a message for the player ("the zip can't be read", "the zip holds no game mGBA plays", "the zip is damaged", "the game is too large to unzip"), shown as "The game couldn't start: …"; the log has the zip's path, what it holds and the sizes. A ROM unzipped is logged with its size and how long unzipping took.

Frames are handed over in a `FrameSlot` holding the newest frame: neither side waits, and a frame the UI hasn't taken is replaced by a newer one. The UI uploads it on its own thread just before Slint draws (the only time its GL context is sure to be current), into one texture the window shows. 3D cores will need a GL context shared with the UI's instead.

A core's options are given as `LaunchSpec::options` says, the rest left at the core's defaults (see [settings.md](settings.md) for where the values come from). Cores are told Specular takes libretro's options version 2, the newest, and their declarations are read in any version. At launch the log has how the core's options differ from its list in `data/cores/` (`game: mGBA's options differ from Specular's list (data/cores/): …`), the values given (`game: mGBA's options: mgba_idle_optimization = Remove Known, the rest its defaults`), and any it doesn't take (an option it doesn't have, or a value it doesn't list), which keep its default. `set_options` logs each value that changes (`game: mGBA option mgba_color_correction: OFF → GBA`). Some options only take effect once the game starts again (mGBA marks those "(Restart)").

Only the libretro environment calls Specular needs are accepted; the rest are refused, which cores must cope with. Each call refused is logged once a game, with its number (`game: refused the core's environment call 13 (0xd)`). A change of timing or picture size while running is accepted but not acted on: frames carry their own size, and pacing and sound keep the timing the game started with. Buttons are read when the core asks, so as late as possible.

### Frame pacing

Each display refresh, the game's thread waits for the vsync, then sleeps for the frame delay, runs the core once and hands the frame to the UI, which draws as soon as it has it. (On the Brick, the UI's animations are paced by the same vsync, one frame per refresh; while one runs over the game, the game's frame is drawn with its next frame, a refresh later. See [brick.md](brick.md).)

- Every 60 frames the delay is set to the period minus the slowest runs (the 95th percentile) and 6 ms for the UI to draw and present, capped at half the period.
- If waiting for the vsync fails, the game falls back to a timer.
- On a timer, after a stall or a pause, timing restarts from now rather than rushing to catch up.

### Sound

Every game's sound is resampled, by linear interpolation, to the output's rate (48 kHz through ALSA). The ratio absorbs two drifts: the speed-up from running at the display's rate, and the clocks of the core and the sound card disagreeing. For the second, the ratio follows how full the output's queue is, aiming at half full (what RetroArch calls dynamic rate control), changing by at most 0.5 % so the pitch change can't be heard. ALSA queues 64 ms; Bluetooth 100 ms, since bluez-alsa takes sound in chunks of 10 ms or more behind its own rate converter. The output can change mid-game, and falls back to the device's own if Bluetooth headphones fail (see [wireless.md](wireless.md)); moving between the speaker and the headphone jack keeps the stream, which the kernel routes.

### Saves and states

| File | Where | Written |
|---|---|---|
| Battery save | `saves/<path under roms/>.srm` | when the game has written its save memory; else checked every 10 s of play; on quit, sleep and SIGTERM |
| Autosave | `roms/<folder>/.states/<name>.autosave` | every 10 s of play (time paused doesn't count), on sleep and quit |
| Save state (one slot) | `roms/<folder>/.states/<name>.state1` | from the in-game menu |
| Newer battery save | `saves/<path under roms/>.newer.srm` | renamed from the `.srm` when a loaded state would overwrite it (see below); an older one is replaced |

- A battery save is written only if it changed. Its size is read each time, as some cores (mGBA) only know it once the game has used its save memory. A save file that exists but couldn't be loaded yet (the core had no save memory then) is never overwritten until it is.
- Our mGBA build says when the game has finished writing its save memory, through a private environment call (`0x20001`, from the range libretro leaves to frontends and their cores), made once the save memory has gone 15 frames without a write. Other cores fall back to the 10 s check.
- Writes go through one writer thread, in order. `flush` and `quit` return the first error since the last flush.
- An autosave the core can't load is ignored, and replaced by the next.
- States start with zstd's frame marker; anything else is loaded as an uncompressed state (as earlier builds wrote them). mGBA's, Nestopia's and SwanStation's own state formats don't start with that marker.
- After the zstd frame, a state has a zstd skippable frame (zstd's tools pass over it) holding the crc32 of the save memory the state was made with: `5e 2a 4d 18`, `08 00 00 00`, `SRAM`, then the crc32 (little-endian). A core without save memory gets none.
- **A newer battery save is kept** (see Decisions). After a state is loaded into a core whose states restore save memory, the next `.srm` write first checks the file on the card, on the writer's thread: if it is neither the state's save (crc32) nor what is being written, it is renamed to `.newer.srm`, logged with both crc32s and the paths. If the rename fails, the write is skipped and the error reported like any write's. The autosave a game resumes from counts as a loaded state.
- On sleep the game is paused and its saves flushed first; waking opens its menu.
- **Saves follow a renamed or moved ROM.** Specular records the size and checksum of each ROM played in `saves/.specular-saves.json`. After a scan, the saves (battery saves and states) of a recorded ROM that is gone move to the one game without battery saves whose ROM has the same size and checksum. With none, or several, they stay.

### Cores

```rust
pub struct CoreDecl {        // specular-core's library module: how Specular runs a core
    pub name: &'static str,
    pub library: &'static str,                            // e.g. "nestopia_libretro"
    pub options: &'static str,                            // its option list, data/cores/<id>.json (see settings.md)
    pub defaults: &'static [(&'static str, &'static str)], // Specular's defaults for some options
    pub rom_by_path: bool,
    pub states_restore_save: bool,                        // see Saves and states
    pub bios: &'static [Bios],
}
pub struct Bios {
    pub games: &'static str,                     // "Famicom Disk System games", for messages
    pub extensions: &'static [&'static str],     // the games that use it; empty: all
    pub files: &'static [&'static str],          // any one will do; matched ignoring case
    pub fallback: Option<&'static str>,          // what the core uses without; None: they can't start
}
impl CoreDecl { pub fn check_bios(&self, rom: &Path, bios_dir: &Path) -> Result<(), String>; }
```

| Core | Systems | Specular's defaults for its options | BIOS in `bios/` | States restore save memory |
|---|---|---|---|---|
| mGBA | GB, GBC, GBA | `mgba_idle_optimization = Remove Known` | none | no |
| Nestopia | NES, Famicom Disk System | `nestopia_blargg_ntsc_filter = disabled` | `disksys.rom`, for `.fds` games only; required | yes (battery memory) |
| SwanStation | PS1 | `swanstation_GPU_Renderer = Software` | `scph5500.bin`, `scph5501.bin`, `scph5502.bin`, `psxonpsp660.bin`, `ps1_rom.bin`, or a known dump under any name; else OpenBIOS | yes (memory card 1) |

A missing required BIOS stops the launch before the core is loaded, with "The game couldn't start: Famicom Disk System games need disksys.rom in the bios folder"; the log names the folder and the files looked for. Nestopia's NTSC filter, on by default, widens the picture to 602 pixels, blurs it and costs time. SwanStation's memory card 1 is the libretro save memory (its default), so it is saved as the game's `.srm`; card 2 is off.

### Patches to mGBA

Kept in `tools/core-patches/mgba`; `tools/build-cores.sh` fetches mGBA at a pinned commit and applies them:

1. `0001`: map ROM files (copy-on-write) instead of copying them into memory.
2. `0002`: tell the frontend when save memory has been written.
3. `0003`: make a save state in one pass rather than two.

mGBA runs with `mgba_idle_optimization = "Remove Known"`, which skips the busy-wait loops of known games: it saves battery without changing how they run (TrimUI's own setting too). Nestopia and SwanStation are built unpatched.

## Findings

- (2026-10-18, desktop) mGBA reads its options again when GET_VARIABLE_UPDATE says they changed: colour correction turned on mid-game changes the next frames. Its fixed-interval frameskip turned on mid-game has no effect (it reloads its configuration, which drops the value; inferred from its source), though it works from launch. Source: `crates/specular-runtime/src/tests.rs`.

- (2026-10-14, device) On the Brick, GBA games run at the panel's 60.2 Hz beside the UI. mGBA takes a median 5–7 ms a frame (95 % under 9–15 ms) at the 816 MHz the `ondemand` governor picks, with about 30–50 ms of sound queued. Source: frame statistics on the Brick.
- (2026-10-14, device) While a game runs, Specular itself uses about 29 MB of the 66 MB resident (the rest is mGBA and its game); 31 MB in the menus. Source: Brick measurements.
- (2026-10-14, device) Before the mapping patch, mGBA held two 32 MB copies of a ROM. Mapped, most of a 32 MB ROM is file pages the kernel can drop. Source: Brick measurements.
- (2026-10-14, device) Making a 516 KB mGBA state (pokehns) takes 3.5–4.7 ms of the game's thread with patch `0003`, 6–7 ms without; no frame was late either way. Source: Brick measurements.
- (2026-10-14, device) zstd level 1 shrinks that state to about 45 KB, in about 6 ms on the writer's thread; decompressing at launch takes about 4 ms. Source: Brick measurements.
- (2026-10-18, desktop) mGBA plays a ROM given as `/proc/self/fd/<n>`, a path without an extension: it tells GB, GBC and GBA games apart by their contents. Source: `crates/specular-runtime/src/tests.rs`.
- (2026-10-18, upstream) mGBA asks for the rumble interface only when a game first rumbles, then sets both motors to the same strength each frame: the share of the frame the cartridge's rumble pin was on. A GBA cartridge has rumble only if mGBA knows the game (by its code, e.g. Drill Dozer's `V49E`); GB and GBC cartridges by their type in the header (MBC5 with rumble). Source: mGBA's `src/platform/libretro/libretro.c` (`_setRumble`), `src/core/interface.c` (`mRumbleIntegrator`), `src/gba/overrides.c`; `crates/specular-runtime/src/tests.rs` plays the first.
- (2026-10-13, upstream) mGBA detects Pokémon Emerald hacks itself (128 KiB flash and the cartridge clock). Source: mGBA.
- (2026-10-13, upstream) libretro keeps the ROM contents it hands a core valid only until loading returns, unless the core asks for longer with `SET_CONTENT_INFO_OVERRIDE` (Specular refuses it). So cores copy what they need, and Specular frees its copy once the core has loaded. Source: libretro.h.
- (2026-10-13, upstream) The usual NES, SNES and Mega Drive cores are non-commercial (snes9x, Genesis Plus GX, PicoDrive) or possibly GPLv2-only (FCEUmm, and PCSX ReARMed for PS1). GPLv3-compatible candidates: bsnes, BlastEm, QuickNES or Nestopia, and SwanStation. Source: the cores' licences.
- (2026-10-18, upstream) Nestopia (libretro/nestopia): GPL-2.0-or-later in every core file's header and the README; its NTSC filter (nes_ntsc) is LGPL-2.1-or-later; libretro-common is MIT. QuickNES (libretro/QuickNES_Core): its LICENSE file is the GPL-2.0 text; Shay Green's files are LGPL-2.1-or-later and the later mappers GPL-2.0-or-later. Both are compatible with GPL-3.0-only. Source: the cores' sources.
- (2026-10-18, upstream) SwanStation (libretro/swanstation): LICENSE is GPL-3.0, with no headers in its files. Bundled: libchdr (BSD-3-Clause), with zstd (BSD-3-Clause), the LZMA SDK (public domain) and miniz (MIT); xxHash (BSD-2-Clause); VIXL (BSD-3-Clause, ARM builds); Xbyak (BSD-3-Clause, x86-64 builds); OpenBIOS from PCSX-Redux (MIT); glad's loader with Khronos' `khrplatform.h` (MIT); stb_image (MIT or public domain); libretro-common (MIT); a Vulkan loader from Dolphin and DuckStation, and PGXP from Beetle PSX (both GPL-2.0-or-later). Source: SwanStation's `dep/` and `src/core/pgxp.cpp`.
- (2026-10-18, upstream, desktop) Nestopia's `retro_unserialize` restores the battery memory: the board's work RAM is saved in each state and loaded back. Its libretro save memory is that battery RAM (or EEPROM). Famicom Disk System disk writes are saved by the core itself, as `<name>.sav` in the save directory (`saves/<folder>/`). Source: Nestopia's `NstBoard.cpp` and `libretro.cpp`; `crates/specular-runtime/src/tests.rs`.
- (2026-10-18, upstream, desktop) SwanStation's `retro_unserialize` restores memory cards: each card's contents are in the state and replace the card's on loading. Memory card 1 is the libretro save memory by default (128 KiB, Specular's `.srm`), card 2 is off; the other card types write `.mcd` files into the save directory. SwanStation also finds a BIOS by content: any file of a BIOS's size whose MD5 it knows, preferring the console's region. Source: SwanStation's `memory_card.cpp`, `libretro_host_interface.cpp` and `host_interface.cpp`; `crates/specular-runtime/src/tests.rs`.
- (2026-10-18, desktop) On this PC (Intel Core Ultra 7 355, heavily loaded by other work: load average about 45 on 8 threads), Nestopia takes a median 0.48 ms a frame (95 % under 0.51 ms) on a generated test ROM (background and sprites on, a tone). SwanStation, on a generated test disc that draws two full-screen shaded triangles over and over (more drawing than games do), takes a median 3.4 ms (95 % under 10 ms) with its drawing thread off, and 19 ms (95 % under 42 ms) with it on, its default: then the core waits for that thread, which the loaded PC kept short of time. Neither has been measured on the Brick. Source: `play` on `target/cores/host`.
- (2026-10-18, desktop) With no BIOS, SwanStation boots a disc (through its `SYSTEM.CNF`) on OpenBIOS, but not a PS-X EXE given directly: that patches the Sony BIOS's jump to its shell, so OpenBIOS shows its own shell instead. Source: `play`.
- (2026-10-18, desktop) With its NTSC filter on (its default), Nestopia crashed in the filter's drawing after save states were loaded in a third game session in one process; with it off, not in six. Source: runtime tests.
- (2026-10-18, desktop) Environment calls refused: Nestopia makes 45 (VFS interface) and 59 (message interface version); SwanStation also makes 69 (core options display callback), 23 (rumble), 57 and 13 (disk control: changing discs), 47 (audio/video enable) and 40 (preferred hardware renderer). Both run correctly without them; without 13, multi-disc games can't change discs. Source: `play`.
- (2026-10-13, upstream) mGBA's libretro build doesn't replace the game's save memory when loading a save state (`retro_unserialize`). Source: mGBA's libretro port.
- (2026-10-13, upstream) RetroArch's "Block SaveRAM Overwrite" copies the save memory before loading a state and copies it back afterwards. That restores only the bytes, not the core's own save-chip state (such as the detected chip type or a flash write in progress). Source: RetroArch.
