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
 - WiFi On/Off/Auto: Auto turns WiFi on only while something needs it (e.g. box art downloads). Scan, join with the on-screen keyboard, forget. A network is saved only once it connects, so a wrong password is never kept. Select shows or hides the password as it is typed (hidden each time the keyboard opens)
 - Bluetooth: search, pair, connect, disconnect, forget. Bluetooth headphones as a sound output with their own volume; the output can switch mid-game, and falls back to the speaker if the headphones fail
 - Sleep: a tap of the power button, or after an idle time (never during a game, nor while a PC is connected over the USB network link). The game is paused and saved first, and its menu is open on waking. Power presses are ignored briefly after waking
 - Power menu: hold the power button for 1 s for Power off and Restart, also during a game (saved first)
 - Clock in the status bar, hidden until the time has been set
 - Controls that appear late or disconnect are picked up again
 - The device's address on the Wi-Fi screen while connected, for file transfer, kept current as it changes; with Developer details, also the other interfaces' (e.g. the USB link)
 - Menus paced by the vsync, as games are. Measured on the Brick: a median of 16.6 ms between frames, scrolling and screens alike (except zooming into and out of a game, see TODO), with a frame late now and then (often an animation's first)
 - Everything except the display, backlight, the LEDs' brightness cap, volume, sleep and the WiFi driver goes through standard Linux interfaces (evdev, power supply, LED class, ALSA, wpa_supplicant, BlueZ, the framebuffer's vsync), shared by any Linux device

TODO:
 - File transfer: Settings → File transfer runs Crystaline OS's WebDAV server and browser page only while open, and shows the addresses to open (WiFi's, and the USB cable's while a PC is connected) and that anyone on the network can reach the card meanwhile. "No SD card" without one; a failed start says why. No idle sleep while open; sleeping stops it, waking starts it again; stopped whenever Specular starts and quits (built; on the Brick, uploading, downloading and deleting over the USB link work, and leaving the screen stops the server; but Crystaline OS leaves the loopback interface down, so Specular's check that the server is up fails and stops it after 5 s (docs/brick.md, Findings). Once the OS brings `lo` up: the rest of DEVELOPMENT.md's check, WiFi and the browser page by a person)
 - SSH on release systems: an "SSH" switch and "Import SSH keys from the SD card" in the developer settings (built; to check on the Brick, see DEVELOPMENT.md)
 - Time zone: Automatic (looked up from ipinfo.io each time WiFi connects, once the user allows it) or picked by hand, through the system's `brick-timezone`; the clock changes at once (built; to check on the Brick, see DEVELOPMENT.md)
 - Rumble from games: checked on the Brick without hands (the motor follows a game's sound, the buttons keep working, the game menu stops it); left: that a game's rumble feels right, PS1 games included (they get a DualShock)
 - SD card pulled and inserted, in the library and mid-game; a read-only card; a write refused as read-only runs `mount-card` (built; on the Brick, a card turned read-only and a repaired one checked; pulling and inserting cards needs a person, see DEVELOPMENT.md)
 - Zooming into and out of a game runs at half rate on the Brick (a median of 33 ms between frames, most of them late): find why
 - Separate volume levels for speaker and headphones, each in its own codec control (built; on the Brick, the levels set at start-up and muting checked; switching outputs needs a person, see DEVELOPMENT.md)

### Core Integration and Support

Completed:
 - Our own libretro runtime: the core runs in Specular's process on its own thread, drawing into a texture the UI shows
 - mGBA for GB, GBC and GBA, with our patches: it reads the ROM file in place instead of copying it into memory, says when the game writes its save, and makes a save state in one pass
 - Sound resampled to the output's rate, the game's speed matched to the display (e.g. 59.73 → 60.2 Hz), with small rate adjustments to keep the sound queue half full
 - Frame pacing: the core runs once per vsync of the display (`FBIO_WAITFORVSYNC`; a timer when the device has none), after a delay that adapts to how long it takes (lower input latency)
 - Battery saves: written when the game writes its save, else checked every 10 s of play; also on quit, sleep and stop. Written off the game's thread
 - One save state slot, from the in-game menu, which says when it was saved; loading it and restarting the game ask first, saying what is lost
 - Autosave as the game menu opens, on sleep and on quit, and every 10 s of play for cores whose state is quick to make (not SwanStation's: it cost PS1 games a frame each time; `game.autosave` in the config files overrides it); games resume from it at launch. Each state reuses the last one's buffer once the writer hands it back, and the frames just after a state is made don't count as misses for the CPU minimum. Checked on the Brick: the 10 s misses of PS1 games are gone, their autosave is written as the menu opens and on quit and resumed at launch, and GBA games still autosave every 10 s without misses, with repeated save states and loads
 - Save states compressed (about 11× smaller, less card wear)
 - Watchdog: a core stuck on one frame for 6 s restarts Specular
 - After a crash, a core stuck or a power cut during a game, Specular says at start-up which core stopped with which game (from a launch record, `cache/launch.json`)
 - Screen scaling: integer, fit, stretch
 - Measured on the Brick: Nestopia at full speed (Contra 6.8 ms a frame, Zelda 5.7 ms) SwanStation reaches full speed only with the CPU at 1.8 GHz or more (Metal Gear Solid, Castlevania: Symphony of the Night), its drawing thread helping (docs/runtime.md)
 - The CPU's speed held up from the game's own frames, through the OS's `brick-power cpu-min`: PS1 games start with a minimum of 1.8 GHz; two frames that miss their refresh within a second raise it a step (from the system's default, the first step is 1.2 GHz; lower steps only when set in the config files), and it stays there until the game ends; the system's default in the game's menu and outside games. `power.cpu_min` in the config files turns it off or fixes it, per system or per game. Logged with each change's reason, and shown in the frame statistics
 - Measured on the Brick: GBA at full speed (60.2 fps) beside the UI; Specular itself uses about 29 MB while a game runs
 - Zipped games play, Famicom Disk System games too: the ROM is unzipped into memory, never onto the card, and given to the core under the zip's name; a zip that can't be played says why
 - Cores' own log messages go into Specular's log (debug messages left out, and at most 30 a second)
 - Nestopia for the NES (and the Famicom Disk System, with `disksys.rom`), and SwanStation for the PS1 with its software renderer; PS1 games run without a BIOS, on SwanStation's built-in OpenBIOS. Both built for the PC and the Brick, licences checked, and checked on whether their save states restore the save memory (both do)
 - Battery saves protected from cores whose save states also restore the save memory: each state carries a checksum of the save it was made with, and a newer `.srm` that a loaded state would overwrite is kept as `<name>.newer.srm`
 - A game that needs a BIOS file it doesn't have says which file to put in `bios/`; environment calls a core makes that Specular refuses are logged
 - A PS1 game that OpenBIOS halts (a BIOS function or patch it lacks, or its fatal error) is quit, saying which BIOS files to put in `bios/` on the SD card; its autosave, which would halt again, is removed. Crashes, the watchdog and games that hang in their own code carry no sign of the BIOS, so say nothing of it
 - PS1 games on several discs (an `.m3u`, or a `.pbp` holding several): the disc changes from the in-game menu, and a game resumes on the disc it was on
 - Cores added to the SD card (`cores/<name>_libretro.so` with its `.info`, as the libretro buildbot ships them) for the systems Specular has: the SNES and Mega Drive appear once one serves them, and a "Core" setting per system and per game chooses where there are several (Specular's own by default). Their options are learned at their first game, and are settings from then on (relearned when the core is replaced). A core without an `.info`, for no system Specular has, or built for another processor is turned away with why, in the log, on Settings → Cores and among what needs attention on home. Their splash shows their `.info`'s name, authors and licence; their battery saves are protected as if their states restore save memory

TODO:
 - The OpenBIOS halt notice with a real game (only a generated disc has been tried: Symphony of the Night and Metal Gear Solid run on OpenBIOS)
 - The CPU minimum, checked on the Brick (start at 1.8 GHz, the system's default in the game menu and after quitting, `power.cpu_min` off per system, none for NES; PS1 games at 60 fps; a game without a start level going to 1.2 GHz after two misses; the frame statistics' second line). Left:
   - Steps up have no settling time: a burst of misses (as when something reads the framebuffer) raised Contra three steps in a second, and with no step down it keeps them until the game ends
   - A disc load early in a PS1 game raises it to 2.0 GHz for the rest of the game: Symphony of the Night's title screen 60 s in (5 misses in a second), Metal Gear Solid's opening 18 s in. Symphony of the Night runs on time at 1.8 GHz otherwise
   - Measure what it costs in battery (docs/runtime.md, Open questions)

### Library and Game Management

Completed:
 - Library scanned from `roms/` (any folder layout), the system worked out from the file extension; rescanned when files change
 - Systems without a core are hidden
 - Games identified by checksum (also inside zips), GBA header code (ROM hacks get their base game's art) and file name, against compiled-in libretro-database tables
 - Box art: the user's own PNG or JPEG beside the ROM, then the cache, then downloaded from libretro-thumbnails over WiFi. Games on screen first, then the rest in the background, paused while a game runs
 - Fuzzy name matching, only when unambiguous: one title holding every word of the file's name, with the same numbers ("II" is "2"), so sequels never collapse; the ROM's own region preferred. The log names each fuzzy match and each game not identified (with why), and counts the rest by method
 - PS1 discs identified by serial: from `SYSTEM.CNF` in a `.bin`, `.iso`, `.cue` or `.m3u`, or a `.pbp`'s `DISC_ID`, against a compiled-in Redump table
 - Saves follow a renamed or moved ROM
 - "No games" shown when there are none
 - Search: from Home, or X in any game list, across every system, best matches first; case, accents and punctuation ignored
 - A letter index: L2/R2 jump between first letters in game lists and grids, with the letter shown large for a moment

TODO:
 - The SD card followed as the system reports it (`/run/card/state`): its message shown in place of the library while it is out, being checked or unusable, above it while read-only, and once after a repair; the library, the settings and the cores read afresh when a card comes; nothing written to a card that can't take it; a game whose card goes paused until that card is back, then saved (built; to check on the Brick, see DEVELOPMENT.md)
### Installer

Completed:
 - Settings → Internal storage: install, with an optional backup of TrimUI's system to the SD card, and an option to keep the card bootable
 - Restore from a backup, listing what each holds, with a confirmation
 - Progress screen with steps; it can't be left, and sleep and power off wait for it
 - Failures show the message, a hint and the log file's location
 - An install or restore cut short (crash, power cut) is recognised: one still running is shown again, and an unfinished one can be run again
 - Developer details show why installing isn't possible

TODO:
 - Settings → Updates: the running version; install an update file from the SD card's `updates/` (an older one with a warning, one that can't be installed with why), then restart; start the previous version (built; to check on the Brick, see DEVELOPMENT.md)
 - Restore from the installed system: it prepares the SD card, whose system then asks once (Restore, Not now, Cancel the restore), never restoring by itself (built; to check on the Brick, see DEVELOPMENT.md)

### Customization

Completed:
 - Accent and second accent colours from a palette; they also tint the light bar and the device's LEDs
 - Games as a list or grid
 - Brightness, speaker, headphone and Bluetooth volume, LED brightness, rumble strength, scaling, sleep time
 - Developer details: developer-only settings and screens
 - Settings are declared once (key, type, default, who sees them); the settings screens are generated from them
 - Settings stored as JSON on the SD card: only values that differ from the default, unknown keys kept, writes safe against power cuts, a damaged file set aside
 - Settings files in `{sd card}/config/v0/` (`specular.json`, `systems/<system>.json`, `games/<game>.json`; see docs/settings.md), beside fully documented JSON Schemas generated from the declarations: every setting with what it does, its values and its default. Schemas are read-only and rewritten only when they change; each settings file names its schema (`$schema`), so editors explain and check it
 - Settings moved from `configs/` to `config/v0/` at start-up, safe against power cuts; the old folder is kept. Each future format change adds a step to a new `vN/` folder
 - Per-system and per-game settings: Start on a game (or "Game settings" in the in-game menu) opens its settings, with "All <system> games…" for its system's; overrides are marked, X resets one, and changes apply to the running game. Every layer is loaded before its games are listed
 - A `README.txt` in each folder of the card the user fills (`roms/`, `bios/`, `cores/`, `fonts/`) and in `cache/` and old config folders, generated from the declarations (file types per system, BIOS files per core, what a core needs), read-only and rewritten only when it changes (see docs/guides-and-problems.md)
 - "N things need attention" on home, opening a list of what Specular can't use, each with why and what to do: games no emulator plays, cores and fonts turned away, settings files set aside or for no game, settings values ignored, and settings that failed to save
 - Core options as settings (`cores/<core>.json`, overridable per system and per game), with their schemas; shown with Developer details on the game and system screens, and changed mid-game. Specular's own choices (mGBA's idle loop removal, Nestopia's NTSC filter off, SwanStation's software renderer) are those options' defaults

### Aesthetics and Presentation

Completed:
 - The Light Bar look: monochrome with user accents, a glowing bar along the top that mirrors the device's LED bar, Geist type
 - Screens: library, game list/grid, settings, WiFi, Bluetooth, on-screen keyboard, power menu, in-game menu, installer
 - Motion: short, consistent transitions for focus, scrolling and screens
 - A game's picture zooms into the running game, and the game shrinks back into it
 - In-game menu slides in over the dimmed game
 - Toasts, and notes that never take the focus
 - Generated covers for games without a picture
 - Emoji in WiFi network and Bluetooth device names
 - A list's drawing cost doesn't grow with its length (checked with 3,000 games)
 - Scrolling lays out only the rows coming into view and the two the focus moves between, and the status and hint bars are drawn from cached textures; profiled on the desktop (docs/look-and-motion.md)
 - 60 fps while animating; nothing drawn while idle
 - Scrolling measured on the Brick: 6.1–6.6 ms of UI-thread CPU a frame, 37–40 % of a core at the 1.0–1.2 GHz the governor picks (47 % before the fixes)
 - The design mockup (`design/mockup.html`) shows the Light Bar look only
 - Every setting's help on screen (Y About on its row), the same text as its schema's; the help texts name values by their labels
 - Hints for L1/R1 Page and L2/R2 Letter on game lists, on a line above the hint bar; "Game menu" with the Menu button on a game's splash
 - At start-up, a notice when the last run didn't end in order (the battery ran out, or the device stopped without powering off), from Crystaline OS's `/run/brick/last-shutdown`
 - Credits in Settings: every project shipped (cores, fonts, data, libraries) and its licence, worked out when the binary is built; the full licence texts installed beside it
 - Fallback fonts from `{sd card}/fonts/`, for non-Latin game names
 - A splash while a game starts: "Powered by" its core, the core's copyright notice and web address (from the credits), over the zooming picture until the game's first frame, for at least 1 s
 - The light bar, and the LEDs showing the accent, take each system's colour on its game list and while its games are played ("Tint by system", on by default)
 - Interface sounds, made in code from the mockup's tick: a quiet tick as the focus moves, short tones as screens open and close and switches turn on and off; none over a game, even paused. "Interface sounds" turns them off
 - Pluggable lights and rumble effects, each with a setting: the LEDs and light bar pulse red as the battery becomes low and flash when the game saves; rumble can follow the game's loud sounds (off by default). The light bar mirrors the LEDs, also during effects
 - Title screens, screenshots or where you left off as games' pictures ("Art", per system), each falling back to box art. Only the kind shown is downloaded (then box art for games without one); the frame on screen is kept, off the UI thread, as each game is quit. Pictures take their kind's shape (the system's screen for all but box art) beside the list, in the grid and as they zoom into the game. Box art already downloaded is kept

TODO:
 - Interface sounds: to check on the Brick by ear (how loud they are, and that the speaker doesn't pop as their output opens and closes). With the FN switch on mute none play, as designed
 - No Manuals gaps found in the 2026-10-19 audit, not yet decided: toasts vanish and replace each other, so an important error can't be read again (a short message history, or errors that wait for a press); the power button's tap (sleep) and hold (power menu) are explained nowhere on screen

## V1

### Hardware/Kernel Integration

TODO:
 - USB audio output (headset dongles, USB-C DACs)
 - Step the CPU minimum down during a game when it has room to spare, tuned so it never costs missed frames (the consequence of a false step down is a worse game than a shorter battery life); see docs/runtime.md, Findings, "The CPU minimum with the step down at 85 % of a refresh" (2026-10-19), for Symphony of the Night's lone misses and the rule tried
 - CPU profile chosen by Specular for the menus and for saving power (the OS's `brick-power profile performance|balanced|saver`; games already get a minimum speed of their own), never a setting in the UI; overridable only in the config files
 - The top USB-C port's power on or off (`brick-power usb-top on|off`, which refuses while a device is attached)
 - At 3 % battery, the game is saved and quit (the OS powers off at 1 %)
 - A power-saving mode: dimming, the screen off, WiFi and Bluetooth off while unused, the LEDs off, a lower refresh rate
 - The panel run at the game's own refresh rate, once the OS can set it (`DISP_LCD_SET_FPS` on `/dev/disp`, 40–60.2 Hz)
 - Confirm the press-to-photon latency estimate with a camera of at least 240 fps (an LED and the screen in one shot), once one is available

### Core Integration and Support

TODO:
 - Systems from cores' `.info` files (the cores added to the card serve only the systems Specular has: a new one also needs its game table, colour and screen size)
 - 3D cores, with a GL context shared with the UI's (the Brick spike made one; SwanStation's hardware renderer would be the first user)
 - SNES and Mega Drive cores of Specular's own, each checked for its licence, speed and save memory (candidates in docs/runtime.md)

### Library and Game Management

TODO:
 - ScreenScraper as an optional art source (needs a project developer ID, each user's account, and quota handling)
 - PS1 `.chd` discs identified by serial (today by name only). Needs a CHD decompressor: the libchdr SwanStation already bundles, or the pure-Rust `chd` crate (rejected for now: three decoders to read two sectors)

### Aesthetics and Presentation

TODO:
 - The in-game menu's blurred backdrop: a blurred copy of the game's frame, made once as the menu opens (docs/look-and-motion.md)
 - Button hints driven by the focused element, as a feature of the UI framework: each row says what its buttons do, so none shows that does nothing (today a screen's own hints show beside the focused row's, e.g. home's "A Open" while a note being read has the focus). B (back) is the exception: its hint comes from whatever decides where B returns to
