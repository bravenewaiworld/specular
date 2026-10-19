# Runtime

Status: In progress · Updated: 2026-10-19

Specular's own libretro runtime (`specular-runtime`): how it loads a core, paces it by the display, plays its sound, and keeps its saves and states safe, and which cores it runs. It decides how games feel (input latency, smoothness, sound) and how safe the player's progress is.

## Decisions

- **Our own libretro runtime, in Rust** (2026-10-13). It ports the algorithms of muOS's runtime ("Pickles", the `retro/` folder of MustardOS/frontend, GPLv3) for the hard parts: sound and video rate control, frame pacing and battery-save writing. Rejected: reusing Pickles' files (about 53,000 lines of C tied to muOS's own UI); building on NextUI (MinUI-derived code has no licence, and NextUI itself has been PolyForm Noncommercial since 2026-08-15), which stays a reference only.
- **Cores are chosen system by system: mGBA for GB, GBC and GBA** (2026-10-13). One core (MPL-2.0) for all three; the most accurate GBA core, and it detects Pokémon Emerald hacks itself. Rejected: Gambatte for GB (GPL-2.0-only, which clashes with GPLv3 code in the same process); gpSP for GBA (lighter; it may come later as a per-game choice if a game needs it).
- **Nestopia for the NES** (2026-10-18). GPL-2.0-or-later, with LGPL-2.1-or-later and MIT code, so it can share a process with GPLv3 code. It is the more accurate of the two candidates (cycle-based, with the most boards, and the Famicom Disk System), and the NES is light enough that accuracy costs little: 0.5 ms a frame on a desktop (see Findings). Rejected: QuickNES (its licence is fine too, LGPL-2.1-or-later and GPL-2.0-or-later; it is faster but less accurate, drawing a scanline at a time with fewer boards, and the speed isn't needed).
- **SwanStation for the PS1, with its software renderer** (2026-10-18). GPL-3.0; the code it bundles is BSD, MIT, public domain or GPL-2.0-or-later (see Findings), and it has a recompiler for ARM64. Its hardware renderers need a GL context shared with the UI, which comes with 3D cores (V1). Rejected: PCSX ReARMed (possibly GPLv2-only).
- **A PS1 game starts without a BIOS, on SwanStation's built-in OpenBIOS** (2026-10-18). SwanStation runs OpenBIOS (MIT) when `bios/` has no BIOS, so games start, though some don't run with it; Specular logs the files it looked for. Rejected: refusing to start PS1 games without a BIOS (it would stop games that run, and SwanStation also finds a known BIOS dump under any name, which a check of file names can't see). Famicom Disk System games have no such fallback: without `disksys.rom` they don't start, and the player is told which file to add.
- **A PS1 game that OpenBIOS halts is quit, and the player is told which BIOS file to add** (2026-10-19). OpenBIOS halts for good on a BIOS function it doesn't implement, a patch of its code it doesn't know, or its fatal error, and just before, it writes to PCSX-Redux's debug port. Our SwanStation build turns that write, made with a disc in, into a private environment call (`0x20002`, `ENV_BIOS_HALTED`). Specular then removes the game's autosave and writes no more (a resume would halt again, even with a BIOS added: SwanStation's states hold the BIOS they were made with), quits the game, and says "The game stopped: SwanStation ran the game on its built-in OpenBIOS, which lacks something the game uses. It may need a BIOS file: put one of scph5500.bin, … in the bios folder on the SD card". Only a halt is told apart: a crash, the watchdog, a game that won't load, or one that hangs or misbehaves in its own code carry no sign of whether the BIOS was the cause (see Findings), so they say nothing of it. Rejected: OpenBIOS's own messages (they go to a dummy TTY device, so never reach the emulator); a black or still screen as the sign (games show both on a real BIOS too); a notice over the frozen game, left running (its autosave would keep the halted state).
- **SNES and Mega Drive wait for V1** (2026-10-18). They need more research: bsnes is likely too slow for the Brick's Cortex-A53, and BlastEm's 68000 emulation is an x86 recompiler (both inferred, unmeasured); Beetle Supafaust and ClownMDEmu are candidates too. Rejected: bsnes and BlastEm for V0 (speed).
- **Battery saves are protected from cores whose save states also restore save memory** (2026-10-18). Loading such a state rolls the save memory back, and writing it would lose newer in-game saves. Each save state is stored with the crc32 of the save memory as it was when the state was made. For the cores flagged (`CoreDecl::states_restore_save`: Nestopia and SwanStation; not mGBA), the first time the `.srm` is written after a state was loaded, an `.srm` on the card that is neither the state's (by its crc32) nor what is about to be written is a newer save, and is first renamed to `<name>.newer.srm`, so it isn't lost. A state without a crc32 counts as not matching. Rejected: RetroArch's copy-back of the save memory after loading (restores only the bytes, not the core's own save-chip state; see Findings); blocking writes after a load.
- **Games are paced by the display's vsync, with an adaptive frame delay** (2026-10-12). Latency comes from when frames are started, not from the toolkit. The core runs once per vsync, started as late as it safely can, so buttons are read late and a press reaches the screen sooner. On the Brick this cuts start-to-screen from 32 ms to about 6 ms (see [brick.md](brick.md)). Rejected: draw-then-swap, as toolkits do by default (frames queue for two vsyncs); a core on its own clock (it drifts against the display, and its frames sometimes wait a whole refresh).
- **The game's speed is matched to the display** (2026-10-13). A game within 5 % of the display's rate runs at the display's rate, its sound resampled to match (a Game Boy game at 59.73 fps plays 0.8 % fast on a 60.2 Hz panel). Further off, a timer paces it at its own rate.
- **Specular holds the CPU's speed up from the game's own frames: a minimum, raised when frames miss their refresh, lowered when they have time to spare** (2026-10-19). On the Brick, `ondemand` leaves PS1 games at 53–56 fps: a game paced by the vsync idles for part of every refresh, and a missed refresh adds a whole refresh of idle, so the governor settles near 1.2 GHz (see Findings). Only Specular sees a missed refresh. A game starts at its core's start level (SwanStation 1800 MHz; the others the system's default). Two frames that miss their refresh within 1 s raise the minimum a step; one alone (a scene loading) doesn't. After 10 s with none missed it drops a step, if the slowest runs (the 95th percentile), slowed in proportion to the lower speed, plus the UI's 6 ms, would fit in 75 % of a refresh. The automatic minimum uses only steps from 1200 MHz up: the first step up from the system's default is to 1200, and a step down from 1200 is to the system's default. On the Brick the steps up to 1008 MHz run at 900 mV and 1200 MHz at 937.5 mV (Crystaline OS's docs/power.md), so holding 1200 costs little, and `ondemand` already runs a busy game mostly at or above it, so a lower minimum would gain little. The frame delay is measured afresh at each change: one sized for the faster speed would make frames miss at the slower one, and the minimum would swing. In the game's menu, asleep and outside games, the minimum is the system's default. Above it the governor still chooses, and a heat limit still caps it. A config-file setting (`power.cpu_min`, per system or per game) turns it off or fixes it, at any step. Rejected: lowering `ondemand`'s `up_threshold` (58.5–59.1 fps at 60, the clock jumping to 2 GHz and straight back hundreds of times a minute); a fixed speed per system in the OS (it can't see what a game needs, and would hold it in the menus too); the OS's profiles alone (they set the range `ondemand` works in, not where it settles: even `performance`'s minimum, 1.2 GHz, is short of SwanStation's 1.8).
- **Sound goes through libasound, loaded at runtime** (2026-10-13), like the GPU libraries, so the same code runs on the Brick and on a desktop, and runs silently without it.
- **Battery saves are written when the game writes them** (2026-10-13), on a thread of their own, never the game's: flushing to an SD card can take longer than a frame.
- **An autosave every 10 s of play; games resume from it** (2026-10-13). Also written on sleep and on quit.
- **Save states live in a hidden `.states/` folder beside the ROM** (2026-10-13), named after it. Battery saves stay in `saves/`.
- **Save states are zstd-compressed (level 1)** (2026-10-14), about 11 times less wear on the card or internal storage, as the autosave is written every 10 s. Battery saves stay uncompressed, so other emulators and tools can read them. Rejected: uncompressed states (readable by other frontends, but far more writes).
- **mGBA gets only the ROM's path, and maps the file** (2026-10-13). Rejected: handing it the contents, as libretro allows (it copies them, and copies a GBA ROM again on the first write to its cartridge area, as Pokémon games' clock does: 64 MB for a 32 MB ROM).
- **A zipped ROM is unzipped into memory** (2026-10-18). A memfd holds it, so nothing is written to the card and mGBA still maps the file (its pages are shared, not copied). Only for a core that can't open zips (none among its `valid_extensions`, and no `block_extract`), as libretro has frontends do; the file inside is the first with one of the core's extensions. A ROM that would leave less than 64 MB of memory available (`MemAvailable`) is refused. Saves stay named after the zip. Rejected: building mGBA with zip support (every core would need its own, and our mapping patch works on files); unzipping to a file on the card (writes, and a leftover file after a crash); a temporary file under `/tmp` (tmpfs: the same memory, and a ROM-sized file left behind by a crash).
- **The core gets an unzipped ROM under the zip's name, with the ROM's extension** (2026-10-18): `Zelda.zip` holding `Zelda (Japan).FDS` is `Zelda.fds`, a symbolic link to the memfd's `/proc/self/fd/<n>` in a folder of the process's own, `$XDG_RUNTIME_DIR/specular/<pid>/` (else `specular-<uid>/<pid>/` in the temporary folder, `/tmp`), both in memory. Cores look at the path: Nestopia plays a Famicom Disk System game only if its path has `.fds` in it, and names that game's own save file after it (see Findings), so the zip's name keeps it with Specular's saves; the extension is in lower case, as Nestopia looks only for `.fds` and `.FDS`. The link lasts as long as the game, as a core may open its path again; the folder goes when the game ends, and the folders of processes that have ended (a crash) are removed before the next ROM is unzipped. If the link can't be made, the core is given `/proc/self/fd/<n>` (cores that tell a game by its contents, as mGBA does, still play it). Rejected: the ROM's own name inside the zip (it may have folders in it, and Nestopia's save file would then not be named like Specular's); `retro_game_info::meta` or a core option (Nestopia reads neither for this); patching Nestopia (every core that looks at extensions would need it); removing the link once the core has loaded (a core opening the path again would fail).
- **Discs change through libretro's disk control interface, from the in-game menu** (2026-10-18). Specular takes the interface in both versions (version 1 adds disc labels and the disc to start on). The in-game menu has a Disc row ("Disc 1 of 3"; a disc's label repeats the game's name, too long for the row, so only the log shows it) for a game on more than one disc; right (or A) and left change to the next or previous disc, wrapping around, and the game resumes. A change opens the tray, selects the disc and closes the tray, as libretro requires. A change that fails puts the disc that was in back if it can, and says why; the game goes on. Rejected: choosing a disc first and changing it with A (one more press, and a game asks for the next disc, one step away).
- **A game resumes on the disc it was on** (2026-10-18). Each save state of a game on several discs records the disc in (its number and its image's path, as the core gives them; see Saves and states). Before the game loads, the core is told to start on the autosave's disc (`set_initial_image`, version 1 of the interface); after a state loads, if the core has another disc in, the state's disc is put in. SwanStation keeps the disc in its own states as well (see Findings), so for it the record changes nothing but the disc it boots; it is for cores whose states don't. Rejected: relying on the core's states (libretro doesn't require them to keep it); a file of its own beside the state, or the saves index (either can disagree with the state, if Specular stops between the two writes).
- **Cores' log messages go into Specular's log, through a C shim** (2026-10-18). libretro's log callback is C-variadic, which Rust 1.98 can't define on stable, so `src/log.c` (built with `cc`) formats the message and hands it to Rust. Lines read `core: <core> <level>: <message>`. Debug messages are dropped before they are formatted, and at most 30 are written a second, the rest counted and the count logged, so a core logging every frame costs little and can't flood the log. Rejected: nightly Rust (`c_variadic`); a log thread (writing a line to stderr is cheap, and the cap bounds how many).
- **Games rumble through libretro's rumble interface, for the first player** (2026-10-18). The core's strong and weak motor strengths go to the rumble thread (see [architecture.md](architecture.md)). The game's rumble is stilled while it is paused (its menu, sleep) and restored on resuming (what the core asks meanwhile, resetting or loading a state from the menu, is kept for then), and stilled when it ends, by the frontend being removed, which also covers a core that fails to load. A crash or the watchdog ending Specular closes its rumble device, and the kernel stops the effect then. Rejected: leaving the rumble on over the menu (a held strength would buzz until the game resumes).
- **PS1 games get a DualShock** (2026-10-19). SwanStation's default pad is the digital one, which has no motors, so no PS1 game could rumble. A DualShock starts in digital mode, so a game that doesn't know it sees a plain pad; the d-pad also drives it in analog mode (SwanStation's default options). It is plugged in once the game is loaded, as RetroArch does (`CoreDecl::controller`). A core the user added keeps its default. Rejected: a setting for the pad (the Brick has no sticks, so the DualShock loses nothing); forcing analog mode (some games behave differently with one).
- **Cores declare their options in libretro's version 2, and can change them mid-game** (2026-10-18). Version 2 carries each option's info text, categories and value labels, which go into the settings' schemas (see [settings.md](settings.md)); versions 0 and 1 are read too, for cores that use them. A change is handed to the game's thread and the core told through GET_VARIABLE_UPDATE, which cores ask about each frame. Rejected: version 0 (a description and bare values only); restarting the game to change an option (some cores read theirs only at launch anyway, and say so).
- **Cores the user adds go in `{sd card}/cores/`, as the libretro buildbot ships them: `<name>_libretro.so` beside its `<name>_libretro.info`** (2026-10-18). The `.info` (RetroArch's core info file) says which systems the core plays, its name, display name, authors, licence and firmware; one without is turned away. Rejected: loading any library and asking it (`retro_get_system_info` names no system, only extensions, which systems share; and a core's code would run while Specular looks, where a crash takes the menus down); a description file of our own (the buildbot already ships one with each core); one folder per core (not how cores are shipped).
- **A core's systems are those its `.info` names in `database`, else by its `systemid`; only systems Specular declares are served** (2026-10-18). `database` lists every system a core plays, by the libretro database names `SystemDecl::libretro_name` holds; `systemid` names one (mGBA's is `game_boy_advance`, though it plays GB and GBC too), so it is used only for a `.info` without a database. Of those, a system is served only if the core plays some of its files (`supported_extensions`, when listed). Systems aren't added from `.info` files in V0: a system needs its game table, colour, screen size and header rules, which a `.info` doesn't have. Rejected: `systemid` alone (one system per core); extensions alone (`.bin` and `.cue` belong to several systems).
- **A user core is checked to be a library for this machine before it is loaded** (2026-10-18): an ELF file of its word size, byte order and processor, else it is turned away with what it is built for ("it is built for 64-bit ARM (AArch64), and this device is x86-64"). Rejected: leaving it to `dlopen`, whose message ("wrong ELF class: ELFCLASS64") says little to a player.
- **A user core is assumed to restore save memory from its states** (2026-10-18), so battery-save protection is on for it: Specular can't know, and the protection only costs a check of the `.srm` on the first write after a state is loaded. Rejected: assuming it doesn't (a newer in-game save could be lost).
- **A user core named as one of Specular's is turned away** (2026-10-18). Its settings file (`cores/<name>.json`) and options would be shared with Specular's own build of it, which has Specular's patches and defaults. Rejected: replacing Specular's own (a core without our patches would quietly lose what they do, such as mGBA mapping its ROM).
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
    pub option_list: Option<&'static CoreOptions>, // what the core should declare (data/cores/); differences are logged. None for a user core
    pub rom_by_path: bool,              // only for cores known to read the file themselves
    pub controller: Option<c_uint>,     // CoreDecl::controller, plugged in once the game is loaded
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
    pub on_bios_halt: Box<dyn Fn() + Send>, // the core's built-in BIOS halted the game for good (its autosave is removed)
    pub cpu_min: CpuMin,                 // chooses the CPU's minimum speed from the frames (see CPU speed)
    pub set_cpu_min: Box<dyn Fn(Option<u32>) + Send>, // sets it, without blocking (None: the system's default)
}
pub struct GameInfo { pub width: u32, pub height: u32, pub aspect: f32, pub fps: f64,
                      pub options: CoreOptions }  // what the core declared as it loaded the game (to learn a user core's)
impl Session {
    pub fn start(spec: LaunchSpec, out: Outputs) -> Result<Session, String>;
    pub fn info(&self) -> &GameInfo;
    pub fn watch(&self, limit: Duration, stuck: impl FnOnce() + Send + 'static);
    pub fn set_buttons(&self, bits: u16);
    pub fn take_sound_peak(&self) -> u16;     // the loudest sample since the last call, for the effects
    pub fn pause(&self, paused: bool);
    pub fn set_sound(&self, out: Option<Box<dyn SoundOut>>);  // open it beforehand: opening can block
    pub fn set_options(&self, options: Vec<(String, String)>); // as LaunchSpec::options; the core reads them again (GET_VARIABLE_UPDATE)
    pub fn discs(&self) -> Discs;            // as the core says now; Discs { count, current: Option<usize> } (specular-core's app)
    pub fn change_disc(&self, index: usize) -> io::Result<()>;
    pub fn save_state(&self, path: &Path) -> io::Result<()>;
    pub fn load_state(&self, path: &Path) -> io::Result<()>;
    pub fn flush(&self) -> io::Result<()>;   // battery save and autosave, on the card when it returns
    pub fn quit(self) -> io::Result<()>;     // dropping a Session quits too
}
```

A zip that can't be unzipped fails the start with a message for the player ("the zip can't be read", "the zip holds no game mGBA plays", "the zip is damaged", "the game is too large to unzip"), shown as "The game couldn't start: …"; the log has the zip's path, what it holds and the sizes. A ROM unzipped is logged with its size and how long unzipping took, then the path the core is given (`game: giving Nestopia /run/user/1000/specular/812/Zelda.fds (a link to /proc/self/fd/31)`), or why the link couldn't be made.

A core with the disk control interface has its discs logged at launch (`game: SwanStation's discs: 2 ("Game (Disc 1)", "Game (Disc 2)"), disc 1 in`), as has the disc it is told to start on, each change (`game: SwanStation changed (disc 1 in) to disc 2 of 2 ("Game (Disc 2)")`), each failure with why and the disc in after it, and a state loaded that was made on another disc. The UI tells the app the discs once the game has started, after a state is loaded and after a change (`Msg::Discs`); the app shows the Disc row from that.

Frames are handed over in a `FrameSlot` holding the newest frame: neither side waits, and a frame the UI hasn't taken is replaced by a newer one. The UI uploads it on its own thread just before Slint draws (the only time its GL context is sure to be current), into one texture the window shows. 3D cores will need a GL context shared with the UI's instead.

A core's options are given as `LaunchSpec::options` says, the rest left at the core's defaults (see [settings.md](settings.md) for where the values come from). Cores are told Specular takes libretro's options version 2, the newest, and their declarations are read in any version. At launch the log has how the core's options differ from its list in `data/cores/` (`game: mGBA's options differ from Specular's list (data/cores/): …`), the values given (`game: mGBA's options: mgba_idle_optimization = Remove Known, the rest its defaults`), and any it doesn't take (an option it doesn't have, or a value it doesn't list), which keep its default. `set_options` logs each value that changes (`game: mGBA option mgba_color_correction: OFF → GBA`). Some options only take effect once the game starts again (mGBA marks those "(Restart)").

Only the libretro environment calls Specular needs are accepted; the rest are refused, which cores must cope with. Each call refused is logged once a game, with its number (`game: refused the core's environment call 13 (0xd)`). A change of timing or picture size while running is accepted but not acted on: frames carry their own size, and pacing and sound keep the timing the game started with. Buttons are read when the core asks, so as late as possible.

### Frame pacing

Each display refresh, the game's thread waits for the vsync, then sleeps for the frame delay, runs the core once and hands the frame to the UI, which draws as soon as it has it. (On the Brick, the UI's animations are paced by the same vsync, one frame per refresh; while one runs over the game, the game's frame is drawn with its next frame, a refresh later. See [brick.md](brick.md).)

- Every 60 frames the delay is set to the period minus the slowest runs (the 95th percentile) and 6 ms for the UI to draw and present, capped at half the period.
- If waiting for the vsync fails, the game falls back to a timer.
- On a timer, after a stall or a pause, timing restarts from now rather than rushing to catch up.
- When the CPU's minimum speed changes, the delay goes back to none and is measured again.

### CPU speed

```rust
// specular-core's cpu module: the rules and the clock of play; the device sets the speed
pub struct CpuMin { /* the CPU's steps, the level now, the time of play, the 10 s on-time window */ }
pub enum Decision { Set(Option<u32>, String), Note(String) }  // a new minimum and why; or only a line for the log
impl CpuMin {
    pub fn new(steps: Vec<u32>, start: Option<u32>) -> Self;   // follows the frames, from `start` (rounded up to a step ≥ 1200 MHz)
    pub fn fixed(steps: Vec<u32>, mhz: Option<u32>) -> Self;   // stays at `mhz` (rounded up to a step, the lower ones too)
    pub fn frame(&mut self, gap: Duration, run: Duration, period: Duration) -> Option<Decision>;  // gap: since the previous frame's refresh
    pub fn current(&self) -> Option<u32>;                      // MHz; None: the system's default
}
pub fn for_game(setting: &str, start: Option<u32>, steps: Vec<u32>) -> (CpuMin, &'static str);  // as power.cpu_min says
impl App { pub fn cpu_min(&self, steps: Vec<u32>) -> (CpuMin, String); }  // the running game's, at its layer, and a line for the log

// specular-core's device module: Hardware::cpu (None where the speed can't be set)
pub trait Cpu: Send {
    fn prepare(&self) -> Result<Vec<u32>, String>;     // once, before any set_min: the speed steps (MHz, ascending)
    fn set_min(&self, mhz: Option<u32>) -> Result<(), String>;
}
// CoreDecl::cpu_start: Option<u32>                     // its games' start level: SwanStation Some(1800)
```

At start-up the hardware thread readies the CPU (on the Brick, `brick-power profile balanced`, as `cpu-min` is refused until a profile has been applied since boot, then its status's `cpu-steps`) and keeps its steps; a game launched before then gets no minimum. At a launch the app gives the game's `CpuMin`: by default `CpuMin::new` from the core's `cpu_start`; `power.cpu_min` (a config-file setting, see [settings.md](settings.md)) set to `"off"` or a speed gives a fixed one. The game's thread sets its start level, then only measures: it hands `CpuMin` each frame paced by the vsync, with the time since the vsync's previous return and the core's run. `CpuMin` adds the gaps up into its clock of play and counts a gap of more than 1.5 periods as a missed refresh. The pacer gives no gap for the first frame, the first after a pause (so a pause isn't counted as play) or a frame paced by a timer (which has no missed refreshes to see), and the game's loop leaves those frames out; on a timer the start level stays. Pausing sets the system's default and resuming the game's level again; the game's end (`Drop`) sets the system's default too. `set_cpu_min` sends each change to the hardware thread, which calls `Cpu::set_min` there (`brick-power cpu-min <MHz>|default` on the Brick, which rounds up to a step); the game's thread never waits for it. On the desktop the simulated CPU has the Brick's steps and prints what it would set (`simulated: CPU minimum 1800 MHz`).

The log has the steps at start-up (`cpu: speed steps [408, …, 2000] MHz`), or why none can be set; at each launch, the start level and where it comes from (`cpu: SwanStation starts with a minimum of 1800 MHz (the core's start level, then as the frames need; power.cpu_min = "auto", its default)`); each change with why (`cpu: minimum 2000 MHz (2 frames missed their refresh in 0.4 s at 1800 MHz)`, `cpu: minimum 1608 MHz (10 s on time; slowest runs 4.0 ms, about 4.5 ms at 1608 MHz)`, `cpu: minimum the system's default (while the game is paused)`); once a game, frames still missing their refresh at the top step (a heat limit may be holding the CPU back); each `brick-power` run with its exit status and output, and why a run failed. The frame statistics show the minimum while it is set (`CPU ≥ 1800 MHz`).

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
- When the core says its built-in BIOS halted the game (`0x20002`), the autosave is removed (through the writer, after any writes queued before) and not written again.
- Writes go through one writer thread, in order. `flush` and `quit` return the first error since the last flush.
- An autosave the core can't load is ignored, and replaced by the next.
- States start with zstd's frame marker; anything else is loaded as an uncompressed state (as earlier builds wrote them). mGBA's, Nestopia's and SwanStation's own state formats don't start with that marker.
- After the zstd frame, a state has zstd skippable frames (zstd's tools pass over them), each `5e 2a 4d 18`, the length of the tag and value (little-endian, as are the numbers in them), a 4-byte tag and its value. Frames with unknown tags are skipped, so later builds can add tags.
  - `SRAM`: the crc32 of the save memory the state was made with (`5e 2a 4d 18`, `08 00 00 00`, `SRAM`, the crc32). A core without save memory gets none.
  - `DISC`: for a game on several discs, the disc in (a u32, from 0), then its image's path as the core gives it (UTF-8; empty if the core gives none).
- **A newer battery save is kept** (see Decisions). After a state is loaded into a core whose states restore save memory, the next `.srm` write first checks the file on the card, on the writer's thread: if it is neither the state's save (crc32) nor what is being written, it is renamed to `.newer.srm`, logged with both crc32s and the paths. If the rename fails, the write is skipped and the error reported like any write's. The autosave a game resumes from counts as a loaded state.
- On sleep the game is paused and its saves flushed first; waking opens its menu.
- **Saves follow a renamed or moved ROM.** Specular records the size and checksum of each ROM played in `saves/.specular-saves.json`. After a scan, the saves (battery saves and states) of a recorded ROM that is gone move to the one game without battery saves whose ROM has the same size and checksum. With none, or several, they stay.

### Cores

```rust
pub struct CoreDecl {        // specular-core's library module: how Specular runs a core
    pub name: &'static str,
    pub library: &'static str,                            // e.g. "nestopia_libretro"
    pub options: &'static str,                            // its option list, data/cores/<id>.json: mgba::LIST (see settings.md)
    pub defaults: &'static [(&'static str, &'static str)], // Specular's defaults for some options, by the list's constants
    pub rom_by_path: bool,
    pub controller: Option<u32>,                          // libretro device for player 1: SwanStation's DualShock; None: the core's default
    pub states_restore_save: bool,                        // see Saves and states
    pub cpu_start: Option<u32>,                           // the CPU's minimum its games start at (MHz; see CPU speed): SwanStation's 1800
    pub bios: &'static [Bios],
    pub user: Option<&'static UserCore>,                  // a core the user added (see Cores the user adds)
}
pub struct Bios {
    pub games: &'static str,                     // "Famicom Disk System games", for messages
    pub extensions: &'static [&'static str],     // the games that use it; empty: all
    pub files: &'static [&'static str],          // any one will do; matched ignoring case
    pub fallback: Option<&'static str>,          // what the core uses without; None: they can't start
}
impl CoreDecl {
    pub fn id(&self) -> &'static str;  // "mgba": its library's name less "_libretro"
    pub fn check_bios(&self, rom: &Path, extensions: &[&str], bios_dir: &Path) -> Result<(), String>; // a zip by its game: the first file with one of the system's extensions
    pub fn bios_halt_notice(&self) -> String; // for Outputs::on_bios_halt: the files its fallback stands in for
}
```

| Core | Systems | Specular's defaults for its options | BIOS in `bios/` | States restore save memory |
|---|---|---|---|---|
| mGBA | GB, GBC, GBA | `mgba_idle_optimization = Remove Known` | none | no |
| Nestopia | NES, Famicom Disk System | `nestopia_blargg_ntsc_filter = disabled` | `disksys.rom`, for `.fds` games only; required | yes (battery memory) |
| SwanStation | PS1 | `swanstation_GPU_Renderer = Software` | `scph5500.bin`, `scph5501.bin`, `scph5502.bin`, `psxonpsp660.bin`, `ps1_rom.bin`, or a known dump under any name; else OpenBIOS | yes (memory card 1) |

Cores the user adds (see below) have no defaults of Specular's for their options, their `.info`'s required firmware as their BIOS, and save memory protected (states restore it, as far as Specular knows).

A missing required BIOS stops the launch before the core is loaded, with "The game couldn't start: Famicom Disk System games need disksys.rom in the bios folder"; the log names the folder and the files looked for. A PS1 game that OpenBIOS halts is quit with "The game stopped: …" (see Decisions); the UI sends the app `Msg::GameStopped`. Nestopia's NTSC filter, on by default, widens the picture to 602 pixels, blurs it and costs time. SwanStation's memory card 1 is the libretro save memory (its default), so it is saved as the game's `.srm`; card 2 is off.

### Cores the user adds

`specular_core::library::user_cores`. The scan thread looks in `{sd card}/cores/` before each library scan (at start-up, when files there or in `roms/` change, and when options are learned), and each core found becomes a `CoreDecl`, added to the systems it plays after their own core (`playable`). The app is told with `Msg::Cores` (see [settings.md](settings.md)). A look that finds a core's files unchanged gives the same `CoreDecl`, so looking leaks nothing until a core changes (each is leaked: settings take `'static` text from it).

```rust
pub struct UserCore {
    pub path: PathBuf,              // {sd card}/cores/snes9x_libretro.so
    pub display_name: String,       // the .info's display_name, for the splash
    pub notice: String,             // "Snes9x Team · Non-commercial": its authors and licence, for the splash
    pub systems: Vec<&'static str>,
    pub learned: bool,              // its options have been learned (see settings.md)
    /* the library's size and modification time, when found */
}
impl UserCore { pub fn status(&self) -> &'static str; }       // whether its options are learned
pub struct UserCores { /* the CoreDecls made so far, and what from */ }
impl UserCores { pub fn find(&mut self, dir: &Path, learned: &Path, systems: &[SystemDecl]) -> Found; }
pub struct Found { pub cores: Vec<&'static CoreDecl>, pub turned_away: Vec<TurnedAway> }
pub struct TurnedAway { pub file: String, pub why: String }
pub fn playable(systems: Vec<SystemDecl>, found: &[&'static CoreDecl]) -> Vec<SystemDecl>;  // less systems with no core
pub fn check_library(path: &Path) -> Result<(), String>;      // an ELF shared library for this machine
pub fn learn(core: &'static CoreDecl, dir: &Path, declared: &CoreOptions) -> io::Result<bool>;
```

A core's `CoreDecl::name` is its `.info`'s `corename` (else its file's name), its id its file's name less `_libretro.so`. Its required firmware (`firmwareN_opt` not `"true"`) is checked as Specular's cores' BIOS files are ("Games played with PCSX ReARMed need scph1001.bin in the bios folder"); optional firmware isn't, as its `.info` doesn't say what it is for. The runtime gets the core's own `need_fullpath` and `block_extract` from the core itself, as for any core.

A file is turned away, with why, when: it is named `<name>_libretro.so` (or `.info`) without the other beside it; it is a `.so` or `.info` named otherwise; its `.info` names no system Specular has, or only systems whose files it doesn't play; it isn't an ELF shared library for this machine; or it has the name of one of Specular's cores. Hidden files and others (a readme) are left alone. Each look whose result changed is logged: `cores: Snes9x (snes9x) plays snes: …/cores/snes9x_libretro.so; its options are learned`, `cores: not using …/cores/x_libretro.so: <why>`, or `cores: none in …/cores`. With Developer details, Settings → Cores lists every core, Specular's and the card's (with its file and whether its options are learned), and the files turned away with why.

### Patches to mGBA

Kept in `tools/core-patches/mgba`; `tools/build-cores.sh` fetches mGBA at a pinned commit and applies them:

1. `0001`: map ROM files (copy-on-write) instead of copying them into memory.
2. `0002`: tell the frontend when save memory has been written.
3. `0003`: make a save state in one pass rather than two.

mGBA runs with `mgba_idle_optimization = "Remove Known"`, which skips the busy-wait loops of known games: it saves battery without changing how they run (TrimUI's own setting too). Nestopia and SwanStation are built unpatched.

## Findings

- (2026-10-18, upstream) In libretro's core info files, `database` lists every system a core plays by its libretro database name ("Nintendo - Game Boy|Nintendo - Game Boy Color|Nintendo - Game Boy Advance" for mGBA), while `systemid` names one (`game_boy_advance`). Every sample looked at has a `database`: mGBA, Snes9x, bsnes, Beetle Supafaust, Genesis Plus GX, PicoDrive, ClownMDEmu, FCEUmm, Nestopia, Gambatte, SwanStation, Beetle PSX HW, PCSX ReARMed. Their `systemid`s: `game_boy`, `game_boy_advance`, `nes`, `super_nes`, `mega_drive`, `playstation` (GBC's `game_boy_color` is inferred: no sample has it). Firmware a core can't do without has `firmwareN_opt = "false"`. The files are MIT-licensed. Source: libretro-core-info; the samples in `crates/specular-core/src/library/user_cores/`.

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
- (2026-10-19, desktop) Without a BIOS, Castlevania: Symphony of the Night (USA) and Metal Gear Solid (USA, Disc 1, Rev 1) boot on OpenBIOS (which skips Sony's intro) and played their intros for 60 s without a halt. SwanStation logs `No BIOS image found for <region> region` as it falls back to OpenBIOS, but nothing when a file of a BIOS's size it doesn't know is there (it then uses OpenBIOS silently). Source: `play`.
- (2026-10-19, upstream) OpenBIOS halts for good (a message, then `while (1)`) on a function of the A0, B0 or C0 tables it doesn't implement, a patch of its code it doesn't recognise (it isn't permissive by default), a memory card or backup unit function it doesn't implement, and its fatal error (the A0 function A1h, after POST code 0F: the boot file couldn't be loaded, or a CD read aborted while the game ran). Its messages go through `printf` to its TTY device, a dummy unless it is built with a TTY console, so none reach the emulator. Just before halting it writes to 0x1F802081, PCSX-Redux's debug break (five places in SwanStation's copy); Sony's BIOSes never write there. Its POST codes go to 0x1F802041; 09 means the game was started. Source: pcsx-redux/nugget's `openbios/` (`kernel/handlers.c`, `patches/patches.c`, `card/device.c`, `card/backupunit.c`, `main/main.c`, `tty/tty.c`, `fileio/stdio.c`); SwanStation's `dep/openbios/openbios.bin`.
- (2026-10-19, upstream, desktop) A halted OpenBIOS spins in the emulated CPU: `retro_run` returns each frame as usual, so the watchdog never sees it. A generated disc whose `PSX.EXE` calls the A0 function BFh halted 7.7 s in (frame 462, POST 09). A disc with neither `SYSTEM.CNF` nor `PSX.EXE`, and a `.cue` whose `.bin` is missing, didn't halt in 15 s: OpenBIOS shows its shell. SwanStation starts the BIOS with no disc when the image won't open (`Failed to load CD image … Booting BIOS with no disc`), and `retro_load_game` succeeds. SwanStation's states hold the BIOS image (`Bus::DoState` saves `g_bios`), so a state made on OpenBIOS runs on OpenBIOS when loaded, with a BIOS file or without. Source: SwanStation's `bus.cpp` and `system.cpp`; `crates/specular-runtime/src/tests.rs`, `play`.
- (2026-10-18, desktop) With its NTSC filter on (its default), Nestopia crashed in the filter's drawing after save states were loaded in a third game session in one process; with it off, not in six. Source: runtime tests.
- (2026-10-18, desktop) Environment calls refused: Nestopia makes 45 (VFS interface) and 59 (message interface version); SwanStation also makes 69 (core options display callback), 47 (audio/video enable), 40 (preferred hardware renderer) and, when loading a state, 72 (save state context); it asks for rumble (23) too, which is refused only on a device without rumble. Both run correctly without them. Source: `play`, `crates/specular-runtime/src/tests.rs`.
- (2026-10-18, upstream, desktop) SwanStation gives the extended disk control interface (58) in `retro_init` when the frontend says version 1 (57), else the basic one (13). An `.m3u` playlist is its discs, each labelled with its file name less the extension; a `.pbp` holding several discs is its discs too, labelled "<title> (Disc n)". It selects a disc only while the tray is open. Opening the tray ends what the drive was doing, and its "shell open" status (the drive's lid-open flag) stays set until the game next reads the status, so a change made between two frames is seen by the game (inferred: not checked with a game). `set_initial_image`'s path is ignored. Its states keep the disc in (an extra record after the machine's state, with the disc's number and path, matched again by file name if the path changed), and loading one puts that disc back. Source: SwanStation's `libretro_host_interface.cpp`, `cd_image_m3u.cpp`, `cd_image_pbp.cpp` and `cdrom.cpp`; `crates/specular-runtime/src/tests.rs`.
- (2026-10-18, upstream, desktop) Nestopia takes a game for a Famicom Disk System one only if its path has `.fds` (or `.FDS`) anywhere in it; otherwise it isn't given `disksys.rom`, and loading fails. It names that game's own save file (`<name>.sav`, its disk writes) after the path's file name, less its extension. It accepts any 8 KiB `disksys.rom` (an unknown one is only logged), so the tests boot a Famicom Disk System game on a BIOS of their own. Source: Nestopia's `libretro.cpp` (`retro_load_game`, `extract_basename`) and `NstFds.cpp`; `crates/specular-runtime/src/tests.rs`.
- (2026-10-13, upstream) mGBA's libretro build doesn't replace the game's save memory when loading a save state (`retro_unserialize`). Source: mGBA's libretro port.
- (2026-10-13, upstream) RetroArch's "Block SaveRAM Overwrite" copies the save memory before loading a state and copies it back afterwards. That restores only the bytes, not the core's own save-chip state (such as the detected chip type or a flash write in progress). Source: RetroArch.
- (2026-10-19, device) On the Brick, Nestopia runs at full speed: 60.2 fps, a median 6.8 ms a frame for Contra (95 % under 8.2 ms) and 5.7 ms for Zelda, about 40 % of a core with the CPU at 1.2 GHz; in Specular, Contra 6.1 ms (95 % under 8.5 ms) with the sound queue at its 32 ms. Source: the headless player paced by vsync, 30 s each; frame statistics.
- (2026-10-19, device) On the Brick, SwanStation (software renderer, drawing thread on) reaches full speed only with the CPU at 1.8 GHz or more. Over 60 s of each game's opening, headless and paced by vsync (3,612 frames would be full speed):

  | | `ondemand` (1.2–1.4 GHz) | 1.6 GHz | 1.8 GHz | 2.0 GHz |
  |---|---|---|---|---|
  | Metal Gear Solid | 3,340 (55.7 fps), 95 % under 23.1 ms | 3,596, 95 % under 15.2 ms | 3,602 | 3,604, median 11.2 ms, 95 % under 12.7 ms |
  | Castlevania: SotN | 3,210 (53.5 fps), median 15.4 ms | 3,600 | 3,612 | 3,609, median 10.2 ms |

  In Specular, Symphony of the Night's title screen ran at 48–53 fps under `ondemand` (core 15.5–16.4 ms, 95 % under 18.8–20.9 ms, the sound queue down to 9–20 ms) and 60 fps at 2 GHz (11.5–11.7 ms). With `up_threshold` at 60, 58.5–59.1 fps; under `schedutil`, 40–47 fps, the CPU mostly at 408–816 MHz (Contra 54 fps). Why: see the next finding. Source: `play` with `brick-power`'s profile, fixed speeds through `scaling_max_freq` with the `performance` governor.
- (2026-10-19, device) Why `ondemand` leaves PS1 games short of full speed: below `up_threshold` it sets the clock in proportion to load (min + load × (max − min), from the busiest CPU of the policy, which covers all four cores), and a game paced by the vsync is idle for part of every refresh. As the clock rises the load falls, so it settles where the two meet: Symphony of the Night's game thread needs 28 % of a core at 2 GHz (about 560 MHz of work), which settles at about 1,170 MHz; Metal Gear Solid's (32 %) at about 1,230 MHz. The traces agree: Metal Gear Solid ran at 1.2 GHz 62 % of the time and 1.0 GHz 28 %; Symphony of the Night at 1.4 GHz 43 %, 1.2 GHz 24 %, 1.0 GHz 19 %, so a run took 13.3 ms against 10.0 ms at 2 GHz, and its longest ones (1.5 % of frames) overran the refresh. The busiest CPU stays well under 95 % for three reasons:
  - The wait for the vsync is idle time, and a frame that overruns adds a whole refresh of it. So misses lower the load and the clock further: Metal Gear Solid's misses came mostly in one 8.9 s stretch with the clock at 1.0 GHz, where 20 % of frames overran (1.6 % at 1.2 GHz).
  - The work is split: SwanStation's drawing thread took 9–32 % of a core beside the game thread's 42–50 % (Specular's UI thread another 20 %), and the threads moved between CPUs 5–33 times a second, so no one CPU carries all of it.
  - With `up_threshold` at 60 the clock swings: Symphony of the Night jumped to 2 GHz 260 times in 58 s, staying a single 50 ms sampling period each time, as the load at 2 GHz (about 30 %) is low enough to drop straight back to 0.8–1.2 GHz.
  `schedutil` (this kernel's isn't frequency-invariant: it aims each CPU at about 80 % busy at the current clock) spirals down the same way: it spent 23–28 s stretches at 408–600 MHz, where 38–76 % of frames overran. Ruled out: CPU hotplug (all four cores stayed online; the kernel has no autohotplug), thermal limits (cooling state 0 of 8, about 29 °C), `scaling_max_freq` (2 GHz), and the tunables (`sampling_rate` 50 ms, `sampling_down_factor` 1, `io_is_busy` 0, `ignore_nice_load` 0, `powersave_bias` 0). At 2 GHz the game thread's frames had headroom: per second, the 95th percentile was at most 13.2 ms (Metal Gear Solid; median 7.7 ms) and 10.8 ms (Symphony of the Night), the longest single frames 24 ms (scene loads). Method: the headless player with a temporary per-frame trace (wait start, vsync, run start and end, in `CLOCK_MONOTONIC`), and a sampler reading the clock, each CPU's cpuidle time and each thread's `stat` (CPU time, CPU last run on) every 10 ms, 60 s per game under each setting; then Symphony of the Night in Specular, sampled the same way. Per-CPU busy shares from cpuidle overstate a CPU that sleeps through a sampling window (its time is counted on waking), so the loads above come from the threads' times. The kernel has no tracefs, `schedstat` or cpufreq statistics.
- (2026-10-19, device) With Crystaline OS's `brick-power cpu-min 1800` (a floor under `ondemand`, which then stays at it), both games run at full speed: Metal Gear Solid 3,602 frames in 60 s (95 % under 13.9 ms, the longest 14.8 ms), Symphony of the Night 3,612 (95 % under 11.4 ms). With a floor of 1,608 MHz, Metal Gear Solid lost a few frames (3,599, the longest 20.0 ms) and Symphony of the Night one (3,611). Source: `play` paced by vsync, 60 s each, as above.
- (2026-10-19, device) SwanStation's drawing thread helps on the Brick: at 2 GHz, Metal Gear Solid without it ran at 55.9 fps (95 % under 17.5 ms), with it at full speed; Symphony of the Night was at full speed either way (median 11.8 ms without, 10.2 ms with). Source: as above, `swanstation_GPU_UseThread`.
- (2026-10-19, device) A BIOS (`scph5501.bin`) changes neither game's speed: Metal Gear Solid at 2 GHz 3,603 frames, under `ondemand` 3,455; Symphony of the Night at 2 GHz 3,609. Source: as above.
- (2026-10-19, upstream) SwanStation's default pad is the digital one (`DEFAULT_CONTROLLER_1_TYPE`), which has no motors; a frontend that never calls `retro_set_controller_port_device` gets no rumble from PS1 games. The device set is applied at the next frame, so it can be set after the game has loaded. Source: SwanStation's `settings.h` and `libretro_host_interface.cpp`.
- (2026-10-19, device) The CPU minimum on the Brick (Crystaline OS 20261019-160224, Specular a3ecd39). At start-up: `brick-power profile balanced`, the nine steps read, `cpu-min default`. Symphony of the Night started at 1800 MHz (`scaling_min_freq` and the clock followed), held it for 50 s on its title screen, then went to 2000 MHz ("2 frames missed their refresh in 0.1 s at 1800 MHz", as its demo began loading) and missed two more at 2000 ("the top step: a heat limit may be holding the CPU below it", with the heat limit at step 0): 57–60 fps, core 11.7 ms, 95 % under 12.0 ms. Metal Gear Solid did the same 17 s in (during the BIOS's start and the disc's first loads), then played its opening at 59–61 fps (core 5.0–8.9 ms, 95 % under 8.0–14.5 ms). Neither stepped down again: the step down asks for runs under about 6.5 ms (see ROADMAP.md). The game menu set the default and resuming 2000 MHz again; quitting set the default. Contra set no minimum at launch, but its launch missed twice, which set a minimum of 408 MHz, the default's own speed; nothing changed over the next 60 s (the governor at 0.8–1.4 GHz). With `"power.cpu_min": "off"` in `config/v0/systems/ps1.json`, the launch line named the file and no minimum was set: Symphony of the Night ran at 46 fps (core 15.3 ms, 95 % under 18.3 ms, the sound queue empty). Source: the `cpu:` log lines, `scaling_min_freq` and `scaling_cur_freq` read every second, `brick-power`'s status, the frame statistics.

## Open questions

- A PS1 game resumed from an autosave made without a BIOS goes on running OpenBIOS once a BIOS file is added (its states hold the BIOS), until the autosave goes. Should a BIOS added make such games start afresh, or offer to? A state tag naming the BIOS it was made on would tell them apart.
- What the CPU minimum costs in battery: PS1 games at 1.8–2.0 GHz against the 1.2 GHz `ondemand` settles at. With the UI's 6 ms counted, SwanStation's slowest runs (11–13 ms at 2 GHz) never leave room for a step down, so its games stay at 1.8 GHz or above. A measurement on the Brick, and a check that its games then run at full speed, would say whether that is right.
