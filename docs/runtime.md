# Runtime

Status: In progress · Updated: 2026-10-18

Specular's own libretro runtime (`specular-runtime`): how it loads a core, paces it by the display, plays its sound, and keeps its saves and states safe, and which cores it runs. It decides how games feel (input latency, smoothness, sound) and how safe the player's progress is.

## Decisions

- **Our own libretro runtime, in Rust** (2026-10-13). It ports the algorithms of muOS's runtime ("Pickles", the `retro/` folder of MustardOS/frontend, GPLv3) for the hard parts: sound and video rate control, frame pacing and battery-save writing. Rejected: reusing Pickles' files (about 53,000 lines of C tied to muOS's own UI); building on NextUI (MinUI-derived code has no licence, and NextUI itself has been PolyForm Noncommercial since 2026-08-15), which stays a reference only.
- **Cores are chosen system by system: mGBA for GB, GBC and GBA** (2026-10-13). One core (MPL-2.0) for all three; the most accurate GBA core, and it detects Pokémon Emerald hacks itself. The first release plays these three systems. Rejected: Gambatte for GB (GPL-2.0-only, which clashes with GPLv3 code in the same process); gpSP for GBA (lighter; it may come later as a per-game choice if a game needs it).
- **NES and PS1 cores come from the GPLv3-compatible candidates; SNES and Mega Drive wait for V1** (2026-10-18): QuickNES or Nestopia (NES), SwanStation (PS1). Each is chosen after a licence check, a speed check on the Brick, and the save-memory check below. SNES and Mega Drive need more research: bsnes is likely too slow for the Brick's Cortex-A53, and BlastEm's 68000 emulation is an x86 recompiler (both inferred, unmeasured); Beetle Supafaust and ClownMDEmu are candidates too. Rejected: bsnes and BlastEm for V0 (speed).
- **Battery saves are protected from cores whose save states also restore save memory** (2026-10-18). Loading such a state rolls the save memory back, and writing it would lose newer in-game saves. For those cores only, each save state is stored with a hash of the `.srm` as it was when the state was made. When the `.srm` is about to be overwritten after a state was loaded, and the `.srm` on the card doesn't match that state's hash (a newer in-game save exists), the existing file is first renamed to `<name>.newer.srm`, so nothing is lost. Not built yet: mGBA's states don't restore save memory, so no current core is affected; each new core is checked. Rejected: RetroArch's copy-back of the save memory after loading (restores only the bytes, not the core's own save-chip state; see Findings); blocking writes after a load.
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
    pub options: Vec<(String, String)>, // core options; the rest keep their defaults
    pub rom_by_path: bool,              // only for cores known to read the file themselves
}
pub struct Outputs {
    pub sound: Option<Box<dyn SoundOut>>,
    pub vsync: Option<Box<dyn Vsync>>,
    pub display_hz: f64,
    pub frames: Arc<FrameSlot>,
    pub on_frame: Box<dyn Fn() + Send>,  // called on the game's thread after each new frame
}
impl Session {
    pub fn start(spec: LaunchSpec, out: Outputs) -> Result<Session, String>;
    pub fn watch(&self, limit: Duration, stuck: impl FnOnce() + Send + 'static);
    pub fn set_buttons(&self, bits: u16);
    pub fn pause(&self, paused: bool);
    pub fn set_sound(&self, out: Option<Box<dyn SoundOut>>);  // open it beforehand: opening can block
    pub fn save_state(&self, path: &Path) -> io::Result<()>;
    pub fn load_state(&self, path: &Path) -> io::Result<()>;
    pub fn flush(&self) -> io::Result<()>;   // battery save and autosave, on the card when it returns
    pub fn quit(self) -> io::Result<()>;     // dropping a Session quits too
}
```

A zip that can't be unzipped fails the start with a message for the player ("the zip can't be read", "the zip holds no game mGBA plays", "the zip is damaged", "the game is too large to unzip"), shown as "The game couldn't start: …"; the log has the zip's path, what it holds and the sizes. A ROM unzipped is logged with its size and how long unzipping took.

Frames are handed over in a `FrameSlot` holding the newest frame: neither side waits, and a frame the UI hasn't taken is replaced by a newer one. The UI uploads it on its own thread just before Slint draws (the only time its GL context is sure to be current), into one texture the window shows. 3D cores will need a GL context shared with the UI's instead.

Only the libretro environment calls Specular needs are accepted; the rest are refused, which cores must cope with. A change of timing or picture size while running is accepted but not acted on: frames carry their own size, and pacing and sound keep the timing the game started with. Buttons are read when the core asks, so as late as possible.

### Frame pacing

Each display refresh, the game's thread waits for the vsync, then sleeps for the frame delay, runs the core once and hands the frame to the UI, which draws as soon as it has it.

- Every 60 frames the delay is set to the period minus the slowest runs (the 95th percentile) and 6 ms for the UI to draw and present, capped at half the period.
- If waiting for the vsync fails, the game falls back to a timer.
- On a timer, after a stall or a pause, timing restarts from now rather than rushing to catch up.

### Sound

Every game's sound is resampled, by linear interpolation, to the output's rate (48 kHz through ALSA). The ratio absorbs two drifts: the speed-up from running at the display's rate, and the clocks of the core and the sound card disagreeing. For the second, the ratio follows how full the output's queue is, aiming at half full (what RetroArch calls dynamic rate control), changing by at most 0.5 % so the pitch change can't be heard. ALSA queues 64 ms; Bluetooth 100 ms, since bluez-alsa takes sound in chunks of 10 ms or more behind its own rate converter. The output can change mid-game, and falls back to the speaker if Bluetooth headphones fail (see [wireless.md](wireless.md)).

### Saves and states

| File | Where | Written |
|---|---|---|
| Battery save | `saves/<path under roms/>.srm` | when the game has written its save memory; else checked every 10 s of play; on quit, sleep and SIGTERM |
| Autosave | `roms/<folder>/.states/<name>.autosave` | every 10 s of play (time paused doesn't count), on sleep and quit |
| Save state (one slot) | `roms/<folder>/.states/<name>.state1` | from the in-game menu |

- A battery save is written only if it changed. Its size is read each time, as some cores (mGBA) only know it once the game has used its save memory. A save file that exists but couldn't be loaded yet (the core had no save memory then) is never overwritten until it is.
- Our mGBA build says when the game has finished writing its save memory, through a private environment call (`0x20001`, from the range libretro leaves to frontends and their cores), made once the save memory has gone 15 frames without a write. Other cores fall back to the 10 s check.
- Writes go through one writer thread, in order. `flush` and `quit` return the first error since the last flush.
- An autosave the core can't load is ignored, and replaced by the next.
- States start with zstd's frame marker; anything else is loaded as an uncompressed state (as earlier builds wrote them). mGBA's own state format doesn't start with that marker.
- On sleep the game is paused and its saves flushed first; waking opens its menu.
- **Saves follow a renamed or moved ROM.** Specular records the size and checksum of each ROM played in `saves/.specular-saves.json`. After a scan, the saves (battery saves and states) of a recorded ROM that is gone move to the one game without battery saves whose ROM has the same size and checksum. With none, or several, they stay.

### Patches to mGBA

Kept in `tools/core-patches/mgba`; `tools/build-cores.sh` fetches mGBA at a pinned commit and applies them:

1. `0001`: map ROM files (copy-on-write) instead of copying them into memory.
2. `0002`: tell the frontend when save memory has been written.
3. `0003`: make a save state in one pass rather than two.

mGBA runs with `mgba_idle_optimization = "Remove Known"`, which skips the busy-wait loops of known games: it saves battery without changing how they run (TrimUI's own setting too).

## Findings

- (2026-10-14, device) On the Brick, GBA games run at the panel's 60.2 Hz beside the UI. mGBA takes a median 5–7 ms a frame (95 % under 9–15 ms) at the 816 MHz the `ondemand` governor picks, with about 30–50 ms of sound queued. Source: frame statistics on the Brick.
- (2026-10-14, device) While a game runs, Specular itself uses about 29 MB of the 66 MB resident (the rest is mGBA and its game); 31 MB in the menus. Source: Brick measurements.
- (2026-10-14, device) Before the mapping patch, mGBA held two 32 MB copies of a ROM. Mapped, most of a 32 MB ROM is file pages the kernel can drop. Source: Brick measurements.
- (2026-10-14, device) Making a 516 KB mGBA state (pokehns) takes 3.5–4.7 ms of the game's thread with patch `0003`, 6–7 ms without; no frame was late either way. Source: Brick measurements.
- (2026-10-14, device) zstd level 1 shrinks that state to about 45 KB, in about 6 ms on the writer's thread; decompressing at launch takes about 4 ms. Source: Brick measurements.
- (2026-10-18, desktop) mGBA plays a ROM given as `/proc/self/fd/<n>`, a path without an extension: it tells GB, GBC and GBA games apart by their contents. Source: `crates/specular-runtime/src/tests.rs`.
- (2026-10-13, upstream) mGBA detects Pokémon Emerald hacks itself (128 KiB flash and the cartridge clock). Source: mGBA.
- (2026-10-13, upstream) libretro keeps the ROM contents it hands a core valid only until loading returns, unless the core asks for longer with `SET_CONTENT_INFO_OVERRIDE` (Specular refuses it). So cores copy what they need, and Specular frees its copy once the core has loaded. Source: libretro.h.
- (2026-10-13, upstream) The usual NES, SNES and Mega Drive cores are non-commercial (snes9x, Genesis Plus GX, PicoDrive) or possibly GPLv2-only (FCEUmm, and PCSX ReARMed for PS1). GPLv3-compatible candidates: bsnes, BlastEm, QuickNES or Nestopia, and SwanStation. Source: the cores' licences.
- (2026-10-13, upstream) mGBA's libretro build doesn't replace the game's save memory when loading a save state (`retro_unserialize`). Source: mGBA's libretro port.
- (2026-10-13, upstream) RetroArch's "Block SaveRAM Overwrite" copies the save memory before loading a state and copies it back afterwards. That restores only the bytes, not the core's own save-chip state (such as the detected chip type or a flash write in progress). Source: RetroArch.
