# Specular Frontend

Specular is a retro-gaming frontend: a launcher plus a libretro emulation runtime, for Linux handhelds. It should feel like part of the device: minimal and polished, with seamless transitions into and out of games, and frugal with memory and battery. It should work on any device. The TrimUI Brick is the reference device and the first port.

## Ethos

1. Simplicity and Stability.
   Reasonable, or dynamically selected default settings for everything. The UI exposes only the settings that any random sample of people is likely to disagree on (i.e. purely subjective pref, rather than all settings the system supports). It just works.
   
2. Fluid, seamless, delightful.
   Seamless and consistent experience throughout browsing, gaming, and configuring. The user cannot tell what is part of specular and what isn't. A minimalist aesthetic, with subtle and delightful visuals and animations. Polished end-to-end.
   
3. First-class Debugging and Development.
   Local environment with batteries included. Unit, integration, and E2E testing.
 
4. Full Control.
   All possible settings not in the UI, are available through documented config files.

## Installing on your device

### Supported Devices

- TrimUI Brick (TG3040), including the Brick Hammer edition

### Installation

Specular comes with Crystaline OS. Follow its installation guide; there is nothing else to install.

## Developer setup

The desktop build runs on Linux. macOS and Windows aren't supported yet. On Windows, WSL 2 may work but is untested.

You need:
- **Rust**, through [rustup](https://rustup.rs). The pinned toolchain is installed on first build.
- **A C compiler** (one dependency builds C code).
- **To run the desktop build:** X11 or Wayland with libxkbcommon, and OpenGL/EGL. ALSA (libasound) is needed for sound; without it Specular runs silently.
- **Python 3**, for the scripts in `tools/`.

Run it with `cargo run -p specular-desktop -- [data folder]`. The data folder stands in for the SD card (default `./sdcard`). It is laid out like one: `roms/`, `bios/`, `saves/`, `config/v0/` (settings) and `cache/` (box art). Games only appear for systems that have an emulator core. `make cores` builds them for this PC into `target/cores/host`; run with `SPECULAR_CORES=target/cores/host`. Run the tests with `make test`, which builds the cores first (plain `cargo test` skips the tests that need a core); building the cores needs CMake and git.

Running on a device goes through Crystaline OS, which includes Specular and builds it for the device with this repository's `Makefile`.

See [DEVELOPMENT.md](DEVELOPMENT.md) for the development and debugging features.
