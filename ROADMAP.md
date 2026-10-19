# Specular Roadmap

## V0

Goals:
 - Basic game playing and management - list, search, play, save, load
 - Basic UI Customization - Grid/List view, accent colours
 - Basic Hardware configuration/status (brightness, LEDs, Wifi, bluetooth, battery) 
 - All advanced settings exposed through config files
 - Installer (to internal storage)
 
### Hardware/Kernel Integration

Completed:
 - Battery level and charging in the status bar
 - Brightness
 - LEDs show the two accent colours, with an LED brightness setting
 - Volume, with an on-screen gauge; the FN switch mutes
 - WiFi On/Off/Auto: Auto turns WiFi on only while something needs it (e.g. box art downloads). Scan, join with the on-screen keyboard, forget. A network is saved only once it connects, so a wrong password is never kept
 - Bluetooth: search, pair, connect, disconnect, forget. Bluetooth headphones as a sound output with their own volume; the output can switch mid-game, and falls back to the speaker if the headphones fail
 - Sleep: a tap of the power button, or after an idle time (never during a game). The game is paused and saved first, and its menu is open on waking. Power presses are ignored briefly after waking
 - Power menu: hold the power button for 1 s for Power off and Restart, also during a game (saved first)
 - Gameplay paced by the display's vsync (`FBIO_WAITFORVSYNC`), with a timer when the device has none
 - Clock in the status bar, hidden until the time has been set
 - Controls that appear late or disconnect are picked up again
 - The device's address on the Wi-Fi screen while connected, for file transfer, kept current as it changes; with Developer details, also the other interfaces' (e.g. the USB link)
 - Everything except the display, backlight, LEDs, volume, sleep and WiFi driver goes through standard Linux interfaces (evdev, power supply, LED class, ALSA, wpa_supplicant, BlueZ), shared by any Linux device

TODO:
 - Rumble from games (built; to check on the Brick)
 - Menus paced by vsync too (built; to measure on the Brick)
 - Separate volume levels for speaker and headphones (built; to check on the Brick)

### Core Integration and Support

Completed:
 - Our own libretro runtime: the core runs in Specular's process on its own thread, drawing into a texture the UI shows
 - mGBA for GB, GBC and GBA, with our patches: it reads the ROM file in place instead of copying it into memory, says when the game writes its save, and makes a save state in one pass
 - Sound resampled to the output's rate, the game's speed matched to the display (e.g. 59.73 → 60.2 Hz), with small rate adjustments to keep the sound queue half full
 - Frame pacing: the core runs once per vsync, after a delay that adapts to how long it takes (lower input latency)
 - Battery saves: written when the game writes its save, else checked every 10 s of play; also on quit, sleep and stop. Written off the game's thread
 - One save state slot, from the in-game menu
 - Autosave every 10 s of play, on sleep and on quit; games resume from it at launch
 - Save states compressed (about 11× smaller, less card wear)
 - Watchdog: a core stuck on one frame for 6 s restarts Specular
 - After a crash, a core stuck or a power cut during a game, Specular says at start-up which core stopped with which game (from a launch record, `cache/launch.json`)
 - Screen scaling: integer, fit, stretch
 - Measured on the Brick: GBA at full speed (60.2 fps) beside the UI; Specular itself uses about 29 MB while a game runs
 - Zipped games play: the ROM is unzipped into memory, never onto the card; a zip that can't be played says why
 - Cores' own log messages go into Specular's log (debug messages left out, and at most 30 a second)
 - Nestopia for the NES (and the Famicom Disk System, with `disksys.rom`), and SwanStation for the PS1 with its software renderer; PS1 games run without a BIOS, on SwanStation's built-in OpenBIOS. Both built for the PC and the Brick, licences checked, and checked on whether their save states restore the save memory (both do)
 - Battery saves protected from cores whose save states also restore the save memory: each state carries a checksum of the save it was made with, and a newer `.srm` that a loaded state would overwrite is kept as `<name>.newer.srm`
 - A game that needs a BIOS file it doesn't have says which file to put in `bios/`; environment calls a core makes that Specular refuses are logged

 - Cores added to the SD card (`cores/<name>_libretro.so` with its `.info`, as the libretro buildbot ships them) for the systems Specular has: the SNES and Mega Drive appear once one serves them, and a "Core" setting per system and per game chooses where there are several (Specular's own by default). Their options are learned at their first game, and are settings from then on (relearned when the core is replaced). A core without an `.info`, for no system Specular has, or built for another processor is turned away with why, in the log and (with Developer details) on Settings → Cores. Their splash shows their `.info`'s name, authors and licence; their battery saves are protected as if their states restore save memory

TODO:
 - Speed check of Nestopia and SwanStation on the Brick (measured on a PC only), including whether SwanStation's drawing thread helps there
 - PS1 games on several discs: changing discs (SwanStation's disk control interface is refused today)
 - Zipped Famicom Disk System games (Nestopia tells them apart by the `.fds` in the path, which an unzipped ROM's path lacks)

### Library and Game Management

Completed:
 - Library scanned from `roms/` (any folder layout), the system worked out from the file extension; rescanned when files change
 - Systems without a core are hidden
 - Games shown as a list or a box art grid
 - Games identified by checksum (also inside zips), GBA header code (ROM hacks get their base game's art) and file name, against compiled-in libretro-database tables
 - Box art: the user's own PNG beside the ROM, then the cache, then downloaded from libretro-thumbnails over WiFi. Games on screen first, then the rest in the background, paused while a game runs
 - Fuzzy name matching, only when unambiguous: one title holding every word of the file's name, with the same numbers ("II" is "2"), so sequels never collapse; the ROM's own region preferred. The log names each fuzzy match and each game not identified (with why), and counts the rest by method
 - The user's own art may be JPEG as well as PNG
 - PS1 discs identified by serial: from `SYSTEM.CNF` in a `.bin`, `.iso`, `.cue` or `.m3u`, or a `.pbp`'s `DISC_ID`, against a compiled-in Redump table
 - Saves follow a renamed or moved ROM
 - "No SD card" and "no games" shown when they apply
 - Search: from Home, or X in any game list, across every system, best matches first; case, accents and punctuation ignored
 - A letter index: L2/R2 jump between first letters in game lists and grids, with the letter shown large for a moment

TODO:
 - PS1 `.chd` discs identified by serial (today by name only)

### Installer

Completed:
 - Settings → Internal storage: install, with an optional backup of TrimUI's system to the SD card, and an option to keep the card bootable
 - Restore from a backup, listing what each holds, with a confirmation
 - Progress screen with steps; it can't be left, and sleep and power off wait for it
 - Failures show the message, a hint and the log file's location
 - An install or restore cut short (crash, power cut) is recognised: one still running is shown again, and an unfinished one can be run again
 - Developer details show why installing isn't possible

### Customization

Completed:
 - Accent and second accent colours from a palette; they also tint the light bar and the device's LEDs
 - Games as a list or grid
 - Brightness, speaker, headphone and Bluetooth volume, LED brightness, rumble strength, scaling, sleep time
 - Developer details: developer-only settings and screens
 - Settings are declared once (key, type, default, who sees them); the settings screens are generated from them
 - Settings stored as JSON on the SD card: only values that differ from the default, unknown keys kept, writes safe against power cuts, a damaged file set aside
 - Select shows or hides the Wi-Fi password on the keyboard, with a hint; it is hidden each time the keyboard opens
 - Settings files in `{sd card}/config/v0/` (`specular.json`, `systems/<system>.json`, `games/<game>.json`; see docs/settings.md), beside fully documented JSON Schemas generated from the declarations: every setting with what it does, its values and its default. Schemas are read-only and rewritten only when they change; each settings file names its schema (`$schema`), so editors explain and check it
 - Settings moved from `configs/` to `config/v0/` at start-up, safe against power cuts; the old folder is kept. Each future format change adds a step to a new `vN/` folder
 - Per-system and per-game settings: Start on a game (or "Game settings" in the in-game menu) opens its settings, with "All <system> games…" for its system's; overrides are marked, X resets one, and changes apply to the running game. Every layer is loaded before its games are listed
 - Core options as settings (`cores/<core>.json`, overridable per system and per game), with their schemas; shown with Developer details on the game and system screens, and changed mid-game. The cores' option lists are written by `make cores` into `data/cores/` and compiled in, with constants for Specular's code; the launch logs how a core's options differ from its list. Specular's fixed choices (mGBA's idle loop removal, Nestopia's NTSC filter off, SwanStation's software renderer) are now defaults

### Aesthetics and Presentation

Completed:
 - The Light Bar look: monochrome with user accents, a glowing bar along the top that mirrors the device's LED bar, Geist type
 - Screens: library, game list/grid, settings, WiFi, Bluetooth, on-screen keyboard, power menu, in-game menu, installer
 - Motion: short, consistent transitions for focus, scrolling and screens
 - Box art zooms into the running game and back out
 - In-game menu slides in over the dimmed game
 - Toasts, and notes that never take the focus
 - Generated covers for games without box art
 - Emoji in WiFi network and Bluetooth device names
 - A list's drawing cost doesn't grow with its length (checked with 3,000 games)
 - 60 fps while animating; nothing drawn while idle
 - The design mockup (`design/mockup.html`) shows the Light Bar look only
 - Credits in Settings: every project shipped (cores, fonts, data, libraries) and its licence, worked out when the binary is built; the full licence texts installed beside it
 - Fallback fonts from `{sd card}/fonts/`, for non-Latin game names
 - A splash while a game starts: "Powered by" its core, the core's copyright notice and web address (from the credits), over the zooming box art until the game's first frame, for at least 1 s
 - The light bar, and the LEDs showing the accent, take each system's colour on its game list and while its games are played ("Tint by system", on by default)
 - Interface sounds, made in code from the mockup's tick: a quiet tick as the focus moves, short tones as screens open and close and switches turn on and off; none over a game, even paused. The output opens at the first sound and closes when idle. "Interface sounds" turns them off
 - Pluggable lights and rumble effects, each with a setting: the LEDs and light bar pulse red as the battery becomes low and flash when the game saves; rumble can follow the game's loud sounds (off by default). The light bar mirrors the LEDs, also during effects

TODO:
 - Interface sounds: to check on the Brick (how loud they are, and that the speaker doesn't pop as their output opens and closes)
 - Title screens and screenshots as art
 - Profile FemtoVG's per-frame cost while scrolling (about 47 % of a core on the Brick): text layout cache hits, how many row slots redraw, whether off-screen slots draw

## V1

### Hardware/Kernel Integration

TODO:
 - USB audio output (headset dongles, USB-C DACs)
 - Confirm the press-to-photon latency estimate with a camera of at least 240 fps (an LED and the screen in one shot), once one is available

### Core Integration and Support

TODO:
 - Systems from cores' `.info` files (the cores added to the card serve only the systems Specular has: a new one also needs its game table, colour and screen size)
 - 3D cores, with a GL context shared with the UI's (built and measured on the Brick; SwanStation's hardware renderer would be the first user)
 - SNES and Mega Drive cores (each needs a licence, speed and save-memory check). bsnes is likely too slow for a Cortex-A53, and BlastEm's 68000 emulation is an x86 recompiler; also consider Beetle Supafaust (SNES) and ClownMDEmu (Mega Drive)

### Library and Game Management

TODO:
 - ScreenScraper as an optional art source (needs a project developer ID, each user's account, and quota handling)
