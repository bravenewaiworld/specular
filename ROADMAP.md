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
 - Rumble from games
 - Menus paced by vsync too (today only gameplay is)
 - Separate volume levels for speaker and headphones

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
 - Screen scaling: integer, fit, stretch
 - Measured on the Brick: GBA at full speed (60.2 fps) beside the UI; Specular itself uses about 29 MB while a game runs

TODO:
 - User added/pluggable libretro cores (loaded from `{sd card}/cores/{name}`)
 - NES and PS1 cores, from QuickNES or Nestopia, and SwanStation (each needs a licence, speed and save-memory check)
 - Protect battery saves from cores whose save states also restore the save memory, by storing a hash of the `.srm` with each state and renaming a newer `.srm` to `<name>.newer.srm` before it is overwritten
 - Check that zipped ROMs launch (they are listed, but mGBA is built without zip support)
 - Core log messages captured into Specular's log
 - Launch record (`cache/launch.json`), so after a crash Specular says which core stopped with which game

### Library and Game Management

Completed:
 - Library scanned from `roms/` (any folder layout), the system worked out from the file extension; rescanned when files change
 - Systems without a core are hidden
 - Games shown as a list or a box art grid
 - Games identified by checksum (also inside zips), GBA header code (ROM hacks get their base game's art) and file name, against compiled-in libretro-database tables
 - Box art: the user's own PNG beside the ROM, then the cache, then downloaded from libretro-thumbnails over WiFi. Games on screen first, then the rest in the background, paused while a game runs
 - Saves follow a renamed or moved ROM
 - "No SD card" and "no games" shown when they apply
 - Search: from Home, or X in any game list, across every system, best matches first; case, accents and punctuation ignored
 - A letter index: L2/R2 jump between first letters in game lists and grids, with the letter shown large for a moment

TODO:
 - Fuzzy name matching for box art (only when unambiguous: one candidate after dropping region and revision tags, every number and roman numeral agreeing, the ROM's own region preferred), JPEG user images, PS1 disc serials

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
 - Brightness, volume, Bluetooth volume, LED brightness, scaling, sleep time
 - Developer details: developer-only settings and screens
 - Settings are declared once (key, type, default, who sees them); the settings screens are generated from them
 - Settings stored as JSON on the SD card: only values that differ from the default, unknown keys kept, writes safe against power cuts, a damaged file set aside
 - Select shows or hides the Wi-Fi password on the keyboard, with a hint; it is hidden each time the keyboard opens
 - Settings files in `{sd card}/config/v0/` (`specular.json`, `systems/<system>.json`, `games/<game>.json`; see docs/settings.md), beside fully documented JSON Schemas generated from the declarations: every setting with what it does, its values and its default. Schemas are read-only and rewritten only when they change; each settings file names its schema (`$schema`), so editors explain and check it
 - Settings moved from `configs/` to `config/v0/` at start-up, safe against power cuts; the old folder is kept. Each future format change adds a step to a new `vN/` folder

TODO:
 - Per-system and per-game settings (the stored layers exist; they aren't loaded or editable yet)
 - Core options as settings, declared from each core's option list (`cores/<core>.json`, overridable per system and per game), with their schemas

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

TODO:
 - Pre-game splash screen with `Powered By: ` core name, core's copyright notice, and repo URL.
 - Light bar tinted per system
 - Sounds
 - Title screens and screenshots as art
 - Profile FemtoVG's per-frame cost while scrolling (about 47 % of a core on the Brick): text layout cache hits, how many row slots redraw, whether off-screen slots draw
 - Pluggable LED and rumble effects (e.g. the light bar reacting to low battery or a save in game, rumble following sound peaks)

## V1

### Hardware/Kernel Integration

TODO:
 - USB audio output (headset dongles, USB-C DACs)
 - Confirm the press-to-photon latency estimate with a camera of at least 240 fps (an LED and the screen in one shot), once one is available

### Core Integration and Support

TODO:
 - 3D cores, with a GL context shared with the UI's (built and measured on the Brick; SwanStation's hardware renderer would be the first user)
 - SNES and Mega Drive cores (each needs a licence, speed and save-memory check). bsnes is likely too slow for a Cortex-A53, and BlastEm's 68000 emulation is an x86 recompiler; also consider Beetle Supafaust (SNES) and ClownMDEmu (Mega Drive)

### Library and Game Management

TODO:
 - ScreenScraper as an optional art source (needs a project developer ID, each user's account, and quota handling)
