# Specular Frontend Development Guide

## Running Specular

- **On the desktop.** The desktop build runs Specular in a 1024×768 window, against a data folder on your PC that stands in for the SD card (its argument: `specular <folder>`; default `./sdcard`). Sound is real. Everything else is simulated:
  - The battery drains slowly, from 80 % or `SPECULAR_BATTERY`.
  - Hardware actions are printed instead of performed.
  - WiFi and Bluetooth follow a recorded or hand-written scenario (see Wireless traces and scenarios).
  - The installer can be made to fail (`SPECULAR_INSTALL_FAILS`), or to look like a task was left running (`SPECULAR_INSTALL_RUNNING`).
- **Cores** are loaded from `SPECULAR_CORES`, or else from `cores/` next to the binary.
- **Cores the user adds** go in the data folder's `cores/`: `<name>_libretro.so` beside its `<name>_libretro.info`. To try one, copy a host-built core under a name of its own with an `.info` naming the systems it plays: `cp target/cores/host/mgba_libretro.so sdcard/cores/mine_libretro.so`, then `sdcard/cores/mine_libretro.info` holding `corename = "Mine"` and `database = "Nintendo - Game Boy Advance"` (or a real `.info` from libretro-core-info, renamed). The GBA then has two cores: its game and system settings get a "Core" row. Its options are learned the first time it plays a game; delete `cache/cores/` to learn them again.
- **BIOS files** go in the data folder's `bios/`. PS1 games run without one, on SwanStation's built-in OpenBIOS (some games don't); Famicom Disk System games need `disksys.rom`. The log says which files were looked for.
- **Keyboard as gamepad:**
  - Arrows: D-pad. X/Z/S/A: A/B/X/Y.
  - Q/W: L1/R1. 1/2: L2/R2.
  - Enter: Start. Tab: Select. Esc: Menu.
  - −/=: volume. P: power button.
  - H: plugs headphones into the jack, or unplugs them (simulated).
- **On a device.** Specular can be run once in the foreground, with its output on your terminal. Or it can replace the installed one and be restarted. See the Crystaline OS development guide.

## Seeing what it does

- **Log.** Messages go to stderr, prefixed by area (`game:`, `sound:`, `art:`, `saves:`, `settings:`, `input:`, `install:`, `core:`, `fonts:`, …). On a device they end up in the system log. At start-up, `fonts:` says which of the SD card's fonts are used, and why any aren't. At start-up `input:` says which headphone jack device is followed, or that none was found; then each plug and unplug, and `sound:` each output switch and volume set. Each launch logs, with `game:`, when the game's first frame came (in ms after the launch), and how much longer the splash hides it (the splash shows for at least 1 s).
  - `rumble:` says at start-up which gamepad rumbles (`rumble: TRIMUI Player1 (/dev/input/event3)`), or that none offers it yet (it is looked for again when something next rumbles), or that the device has none; then a gamepad gone, and errors (each once, until another comes, or `rumble: working again`). `game: the core uses rumble` says the running core asked for it (mGBA asks once its game first rumbles). On the desktop, each change of the motors is printed (`simulated: rumble strong 65535 weak 65535`).
  - `effects:` has a line each time a lights or rumble effect starts (`effects: low battery`, `effects: save flash`, `effects: rumble following the sound`, the last once per stretch of play, until the game pauses). On the desktop, saving a state from the in-game menu flashes the light bar, and `SPECULAR_BATTERY=16` starts the simulated battery a minute from low.
  - `cores:` names each core found on the card, the systems it plays, its file and whether its options are learned, and each file in `cores/` not used, with why (at start-up and whenever that changes); then `cores: learned <core>'s options` at a core's first game. With Developer details, Settings → Cores lists the same.
  - `settings:` names each settings file loaded with its values, each value ignored (its file, key and why), and each file under `games/` for no game in the library.
  - At launch, `game:` gives the core's options as given (`game: mGBA's options: …`), those it doesn't take, and how what it declares differs from its list in `data/cores/` (to be expected only on another processor); then each option changed mid-game, with its old and new values.
  - The running core's own messages come as `core: <core> <level>: <message>` (e.g. `core: mGBA warn: …`), without its debug ones and at most 30 a second; `core: <core>: N more messages dropped` says how many were left out.
  - `art:` has a line for each game identified by fuzzy name, and for each it couldn't identify, with the reason (e.g. several similar names, or only sequels'), plus disc serials that can't be read or aren't in the table. Each time the art worker runs out of work it adds how many games it identified by each method. Identifications are cached, so delete `cache/identify/` to see them again.
- **Wireless trace.** Every call to and event from the WiFi and Bluetooth services is logged with a timestamp. Passwords are redacted. The trace is always on.
- **Frame statistics** (a developer setting): fps, the core's median and 95th-percentile time per frame, how much sound is queued, and the rumble's strength while it rumbles (the game's and the effects' together, after the "Rumble" setting), drawn over the game. Also, at the bottom right (on every screen, and over a game), the times between the UI's frames while something animates: how many, how many came late (more than 1.5 refreshes after the one before, so a refresh showed a frame twice), how many extra (under half a refresh: two frames for one refresh, one never shown) and the longest. Each animation that ends also logs a line (`ui: 9 frame times: median 16.6 ms, longest 17.0 ms; 0 late, 0 extra`).
- **Checking the menus' frame pacing on the Brick.** Turn on Developer details, then Frame statistics, and run Specular in the foreground to see its log. Scroll a long list, hold a direction, open and close screens and the in-game menu, and launch a game (its cover zooms into it): the counts should stay at 0 late and 0 extra, and each `ui:` line should show a median of about 16.6 ms (60.2 Hz). Then leave it idle: no `ui:` lines, and `top` shows Specular at about 0.5 % CPU. Every frame late (a median near 33 ms) means each wait for the vsync ends a refresh late (a kernel whose `FBIO_WAITFORVSYNC` takes the framebuffer lock; see docs/brick.md). While a game runs, show the volume gauge or a toast over it: the game's fps should stay at 60. `s1_slint 20 --continuous --vsync` checks the same without Specular, and should show 60.2 frames/s, 0 late and about 1 ms in the swap (against most of a refresh without `--vsync`); `s1_slint 20 --idle --vsync` should show no frames and about 0.5 % CPU.
- **Developer details** (a setting): shows developer-only settings, every core and the files in the card's `cores/` turned away with why (Settings → Cores), an icon gallery, the other network interfaces' IPv4 addresses (on the Wi-Fi screen, to find the device over USB), and why installing to internal storage isn't possible.
- **Keep Wi-Fi on** (a developer setting): stops WiFi Auto from turning WiFi off, so SSH over WiFi stays up.
- **Watchdog.** A core stuck on one frame for 6 s is reported, and Specular exits so the service manager restarts it.
- **Launch record.** `cache/launch.json` exists while a game runs. If Specular finds it at start-up (after a crash, the watchdog or a power cut), it logs it in full (`game: the last game didn't end; its launch record: …`), says which core stopped with which game, and removes it.

## Reproducing and testing

- **Unit and integration tests** (`cargo test`):
  - The app's behaviour: messages in, effects out.
  - Settings, the library and saves, using temporary folders.
  - List layout and held-button repeat.
  - The on-screen keyboard.
  - WiFi against a fake service socket replaying output recorded on a device.
  - Bluetooth parsing.
  - The installer's progress protocol.
  - The lights and rumble effects, with plain events and times, and the rumble handed between threads.
  - The runtime, with real cores (mGBA, Nestopia, SwanStation) on generated ROMs (one mGBA ROM rumbles: it has Drill Dozer's game code and turns the cartridge's rumble pin on), including whether a core's save states restore its save memory. This needs host builds of the cores (`make test` builds them); without them these tests are skipped.
- **Wireless traces and scenarios.**
  - A wireless trace from a device converts into a scenario (`tools/trace-to-scenario.py`).
  - The desktop build replays a scenario (`SPECULAR_SCENARIO=<file>`): the same events, with the same answers and timing. A WiFi or Bluetooth problem seen on a device can be reproduced on the PC.
  - A default scenario is built in.
- **Scripted input and screenshots.**
  - `SPECULAR_SCRIPT=down,a,…` plays a list of button presses into the app, one every `SPECULAR_SCRIPT_MS` (default 400).
  - It then saves a screenshot (raw RGBA) to `SPECULAR_SNAPSHOT` and quits.
  - This is the basis for end-to-end checks; there is no test runner around it yet.
- **Benchmarks.** Ignored tests time focus moves in a 3,000-game list and a search of 3,000 titles (`cargo test --release -- --ignored --nocapture`).

## Measuring and exploring

These are cargo examples. Build them for a device and run them there through Crystaline OS: `make specular` puts them in its `build/specular/aarch64-unknown-linux-gnu/release/examples/`, and `tools/run-specular.sh <binary> [args]` runs one.

- **Headless player** (`specular-runtime`'s `play`). Runs a core and a ROM with no UI, optionally paced by vsync. It reports the core's time per frame and can save the last frame. `-o key=value` sets a core option (use Specular's defaults, its `CoreDecl::defaults`, e.g. `-o nestopia_blargg_ntsc_filter=disabled`); `SPECULAR_BIOS` is its `bios/` folder. Every environment call the core makes that Specular refuses is logged once, with its number.
- **Device measurements** (`devices/brick/examples/s*`). Each measures one part of the stack on a device:
  - Toolchain.
  - Slint on the device's GPU.
  - Frames handed from another thread.
  - Display timing.
- **Input injection** (`inject`). Writes button presses, including long holds, into a device's input, to drive Specular remotely.
- **WiFi and Bluetooth consoles** (`specular-linux`'s `wifi` and `bluetooth`). An interactive WiFi console, and a scripted Bluetooth session with a test tone. Both talk to the real services on a device.

## Data

- **Game identification tables** are generated from a pinned libretro-database commit (`tools/trim-db.py`). They are compiled in.
- **Core option lists** (`data/cores/<core>.json`): what each core `tools/build-cores.sh` builds declares, compiled into Specular as its options' settings and as constants (see docs/settings.md). `make cores` (for this PC) loads each core it built and rewrites a list only if it changed; commit the change with the core's update. The `core-options` example does that alone (`cargo run -p specular-runtime --example core-options -- data/cores target/cores/host/*.so`). A test fails if a list differs from the core built for this PC, and one if a core built has no list. A new core needs its list before its `CoreDecl` can name it (`options: <core>::LIST`): build it, run `make cores`, then add the `CoreDecl`.
- **Credits** are worked out by each device's build script (`crates/specular-credits`), from the crates it links and `data/credits.tsv` (what isn't a crate; its header explains it). A new core in `tools/build-cores.sh` needs a line there, or the build fails. A build warning (`credits:`) names any crate that ships no licence text, and what to add. The licence texts are written to `licences.txt` beside the binary (`target/<profile>/`).
