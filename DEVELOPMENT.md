# Specular Frontend Development Guide

## Running Specular

- **On the desktop.** The desktop build runs Specular in a 1024×768 window, against a data folder on your PC that stands in for the SD card (default `./sdcard`). Sound is real. Everything else is simulated:
  - The battery drains slowly.
  - Hardware actions are printed instead of performed.
  - WiFi and Bluetooth follow a recorded or hand-written scenario (see Wireless traces and scenarios).
  - The installer can be made to fail (`SPECULAR_INSTALL_FAILS`), or to look like a task was left running (`SPECULAR_INSTALL_RUNNING`).
- **Cores** are loaded from `SPECULAR_CORES`, or else from `cores/` next to the binary.
- **Keyboard as gamepad:**
  - Arrows: D-pad. X/Z/S/A: A/B/X/Y.
  - Q/W: L1/R1. 1/2: L2/R2.
  - Enter: Start. Tab: Select. Esc: Menu.
  - −/=: volume. P: power button.
- **On a device.** Specular can be run once in the foreground, with its output on your terminal. Or it can replace the installed one and be restarted. See the Crystaline OS development guide.

## Seeing what it does

- **Log.** Messages go to stderr, prefixed by area (`game:`, `sound:`, `art:`, `saves:`, `settings:`, `input:`, `install:`, `core:`, …). On a device they end up in the system log.
- **Wireless trace.** Every call to and event from the WiFi and Bluetooth services is logged with a timestamp. Passwords are redacted. The trace is always on.
- **Frame statistics** (a developer setting): fps, the core's median and 95th-percentile time per frame, and how much sound is queued, drawn over the game.
- **Developer details** (a setting): shows developer-only settings, an icon gallery, and why installing to internal storage isn't possible.
- **Keep Wi-Fi on** (a developer setting): stops WiFi Auto from turning WiFi off, so SSH over WiFi stays up.
- **Watchdog.** A core stuck on one frame for 6 s is reported, and Specular exits so the service manager restarts it.

## Reproducing and testing

- **Unit and integration tests** (`cargo test`):
  - The app's behaviour: messages in, effects out.
  - Settings, the library and saves, using temporary folders.
  - List layout and held-button repeat.
  - The on-screen keyboard.
  - WiFi against a fake service socket replaying output recorded on a device.
  - Bluetooth parsing.
  - The installer's progress protocol.
  - The runtime, with a real mGBA core on a generated ROM. This needs a host build of the core; without it these tests are skipped.
- **Wireless traces and scenarios.**
  - A wireless trace from a device converts into a scenario (`tools/trace-to-scenario.py`).
  - The desktop build replays a scenario (`SPECULAR_SCENARIO=<file>`): the same events, with the same answers and timing. A WiFi or Bluetooth problem seen on a device can be reproduced on the PC.
  - A default scenario is built in.
- **Scripted input and screenshots.**
  - `SPECULAR_SCRIPT=down,a,…` plays a list of button presses into the app, one every `SPECULAR_SCRIPT_MS` (default 400).
  - It then saves a screenshot (raw RGBA) to `SPECULAR_SNAPSHOT` and quits.
  - This is the basis for end-to-end checks; there is no test runner around it yet.
- **Benchmarks.** An ignored test times focus moves in a 3,000-game list (`cargo test -- --ignored --nocapture`).

## Measuring and exploring

These are cargo examples. Build them for a device and run them there through Crystaline OS (`tools/run-specular.sh <binary> [args]`).

- **Headless player** (`specular-runtime`'s `play`). Runs a core and a ROM with no UI, optionally paced by vsync. It reports the core's time per frame and can save the last frame.
- **Device measurements** (`devices/brick/examples/s*`). Each measures one part of the stack on a device:
  - Toolchain.
  - Slint on the device's GPU.
  - Frames handed from another thread.
  - Display timing.
- **Input injection** (`inject`). Writes button presses, including long holds, into a device's input, to drive Specular remotely.
- **WiFi and Bluetooth consoles** (`specular-linux`'s `wifi` and `bluetooth`). An interactive WiFi console, and a scripted Bluetooth session with a test tone. Both talk to the real services on a device.

## Data

- **Game identification tables** are generated from a pinned libretro-database commit (`tools/trim-db.py`). They are compiled in.
