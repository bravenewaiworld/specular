# Mainline boards

Status: In progress · Updated: 2026-11-12

The port to boards running mainline Linux under Crystaline OS's mainline family (`devices/mainline`): first the Powkiddy RGB20 Pro (Rockchip RK3566), later the Orange Pi 5. How Specular draws through DRM/KMS, how one binary serves every board, and what was measured. Unlike the Brick, everything here goes through standard Linux interfaces, so this is also the reference for any future Linux device with a KMS display.

## Decisions

- **One binary for every mainline board; what differs is data** (2026-11-11). `devices/mainline/src/boards.rs` holds a `Board` per board (name, ppi, buttons, input devices, backlight, ALSA device, volume control), found at start-up from `/proc/device-tree/compatible`. Crystaline OS's mainline family ships one root filesystem for all its boards, so its Specular must already serve them all. Rejected: a device crate per board (`devices/rgb20pro`; a binary per board in a shared root filesystem).
- **The display is DRM/KMS with a GBM surface, presented by page flips** (2026-11-11). EGL draws into GBM buffers; each present flips the new buffer onto the CRTC at the next vblank (`specular-linux/src/kms.rs`). libdrm and libgbm are loaded at runtime, as the GPU libraries are, so no build needs their headers. Rejected: the DRM fbdev emulation (`/dev/fb0`: no vsync wait and a copy per frame); atomic modesetting (more code for nothing a single full-screen plane needs, until per-game mode switching).
- **The drawing loop is shared with the Brick** (2026-11-11). `specular_ui::platform::GlPlatform` is the Brick's former event loop, over a `GlSurface` that each device supplies: the Brick's driver display, or `KmsSurface`. Rejected: a copy of the loop per display kind (every pacing fix made twice).
- **Crystaline OS's services are shared too** (2026-11-11). SSH, file transfer, the time zone, the SD card and the last-shutdown record are contracts of Crystaline OS, not of the Brick, so they live in `crates/specular-crystaline`. Rejected: copying them into each device crate.
- **The refresh rate is measured, not taken from the mode** (2026-11-11). At start-up the device waits 30 vblanks and uses their rate as `DeviceProfile::refresh_hz`, logged with the mode's own; without vblanks it takes the mode's. On the RGB20 Pro the mode says 60.00 Hz but the panel runs at 59.33 Hz (Findings), and games and sound are paced against the profile's rate.
- **On the RGB20 Pro, Select and Start held together are Menu** (2026-11-12). It has no Menu, Home or FN button. `Input::menu_chord` reports Menu while both are held, and Select and Start are still reported themselves, so a game sees both pressed for a moment; hints call it SELECT+START (`DeviceProfile::labels`). Rejected: R3, a stick click (PS1 analog games use it); holding Select (a short press would reach the game only on release); FN (there is none: an older ROCKNIX driver's list had one). The stick clicks are left out of the profile until games get them.
- **A game gets the panel mode nearest its own rate, where the board allows it** (2026-11-12). Off on the RGB20 Pro until its panel takes a switch (Findings). As a game starts, the runtime asks the device for the game's rate (`Device::match_refresh`); the mainline device switches to the panel mode within 0.5 % of it (GBA 59.73 Hz, PAL 50 Hz), through a full mode set at the UI's next present, and back to the usual mode as the game ends. Further off, the usual mode stays and the sound is resampled as before. 90 and 120 Hz are never picked for a game. Rejected: one rate for every game (the Brick's way: games run slightly fast or stutter).
- **Sticks reach games as libretro analog input, and the left stick moves through the menus** (2026-11-12). A game's thread reads the sticks once a frame (`Device::open_sticks`, at the core's input poll); the UI's input turns the left stick into d-pad presses marked `ButtonEvent::stick`, which a game doesn't get. A 10 % dead zone. Rejected: sticks only in games (a way to move that does nothing in the menus).
- **Rumble drives the motor's own device on boards without a gamepad program** (2026-11-12): the board table names it (`pwm-vibrator`); the Brick keeps padbridge's gamepad.
- **The RGB20 Pro's LEDs stay with the system** (2026-11-11). The green and red LEDs show status and charging; `leds` is empty, so Specular's accent lights don't drive them.

## Design

### Display

`Kms::open` picks the first connected built-in panel (LVDS, eDP or DSI; else the first connected output), its preferred mode and a CRTC that can drive it (the one driving it now first), and makes a GBM surface of that size in XRGB8888. Specular is then the card's DRM master. `EglDisplay::open(Native::Gbm { .. })` takes an EGL 1.5 platform display on the GBM device and a config whose native visual is the surface's format.

`KmsSurface::present`: `eglSwapBuffers`, then lock the new front buffer, give it a DRM framebuffer the first time it is seen, and flip to it. The first present sets the mode instead. If the flip before is still pending, it first waits for that flip's event (at most a refresh), then gives the buffer it replaced back to GBM. So two buffers are locked at most.

Vsync for games and for the menus' animations is `DrmVsync`: `DRM_IOCTL_WAIT_VBLANK` on a file of its own (no DRM master needed), one per waiting thread, plugged into `FramePacer` and the game thread's `Vsync` as `FbVsync` is on the Brick.

### Board table and hardware

| Service | How |
|---|---|
| Buttons | evdev, the board's input devices by name (`specular-linux`). The RGB20 Pro: `gpio-keys-control` (d-pad as `BTN_DPAD_*`, triggers as `BTN_TL2`/`BTN_TR2` keys), `gpio-keys-vol`, `rk805 pwrkey`; Menu is Select + Start |
| Battery | power-supply class (`rk817-battery`) |
| Backlight, panel off | backlight class (`specular-linux/src/backlight.rs`): levels 1–10 from 1/16 of `max_brightness` up on a curve; panel off is `bl_power` (the backlight only) |
| Volume | one mixer control for every output, through amixer: `rk817ext`'s "Master Playback Volume" (0–255, −95 to 0 dB). Levels 2 dB apart, the top at 0 dB |
| Sound | ALSA `default:CARD=rk817ext` (card 0 is HDMI) |
| Headphones | none: the jack cuts the speaker in hardware and reports nothing |
| SSH, file transfer, time zone, SD card, power off, restart | Crystaline OS (`specular-crystaline`) |
| Sleep, WiFi, CPU speed | Crystaline OS (`sleepnow`, the `wpa_supplicant` service with the board's driver, `crystaline-power`), through `specular-crystaline` |
| Bluetooth | BlueZ (`specular-linux`) |
| Rumble | force feedback on the board's motor device (`EvdevRumble::motor`) |
| Sticks | the board's joystick device (`EvdevSticks`): ranges from the driver, the left stick also as the d-pad in menus |
| Panel rate | `Device::match_refresh`: the mode nearest a game's rate (`Kms::set_mode` at the next present) |
| Menu button | Select + Start together (`Input::menu_chord`), named SELECT+START in hints |

## Findings

- (2026-11-11, device) The RGB20 Pro's DSI panel offers 12 modes, all 1024×768: 60.00 (preferred, 51.07 MHz), 120.00, 90.00, 75.47, 59.73, 59.94, 60.10, 60.00, 57.50, 50.01, 49.95 and 50.00 Hz. Source: `kms:` lines.
- (2026-11-11, device) In its preferred 60.00 Hz mode the RGB20 Pro's panel refreshes at 59.33 Hz: the display clock gets 50.5 MHz instead of 51.07 (Crystaline OS's stage 2 finding on the RK3566's display clock). Source: `kms: refresh measured`, `devices/mainline/examples/kms_spike.rs`.
- (2026-11-11, device) Panfrost (Mesa 26.0.1, Mali-G52) on the RGB20 Pro: clearing and flipping 300 frames took 5.04 s, 298 flips, none more than 1.5 refreshes after the one before, the longest gap 16.9 ms (one refresh), the slowest present 17.2 ms. Source: `kms_spike`.
- (2026-11-12, device) A GBA game (mGBA) on the RGB20 Pro, run from the library: 594 flips in each 10 s for 50 s (59.4 a second: one per refresh at 59.33 Hz), none late, the longest gap 16.9 ms. The first 10 s, with the launch and its zoom, had 9 late flips and the 10 s after 2 (around the first autosave's flash, 33.7 ms). The first frame came 648 ms after the launch. Source: `SPECULAR_FLIP_STATS=1`, with `SPECULAR_SCRIPT` launching the game.
- (2026-11-12, device) With Crystaline OS's display clock fix, every RGB20 Pro panel mode measures within 20 ppm of its rate over 60 vblanks (60.00 → 59.999, 59.73 → 59.728, 50.00 → 50.000, 120 → 120.001), and flipped 299 of 300 frames with none late. Whether 90 and 120 Hz show a picture is still to be seen. Source: `kms_spike 300 all`.
- (2026-11-12, device) A GBA game (mGBA, 59.727 Hz) switched the panel to its 59.73 Hz mode as it started and back to 60 Hz as it ended; in between, 598 flips per 10 s with none late, apart from 5 around the first autosave's flash and 1 as the game menu opened. Source: `display:` and `kms:` lines.
- (2026-11-12, device) After any panel mode switch on the RGB20 Pro the panel shows garbled vertical bands (seen by a person, after a GBA game's switch to 59.73 Hz), and it stays garbled back in the usual 60 Hz mode until the next start, although the panel driver reset and re-initialised the panel at each switch (12 `Panel init sequence done` for 12 modes) and the DSI host's and PHY's registers back at 60 Hz match a fresh start's but for one PHY register (0x3ac: 4, then 5). So the RGB20 Pro's board entry turns per-game rates off (`game_rates`). ROCKNIX's live switching garbled the panel the same way. Source: dmesg with the panel driver's debug output, `devmem` dumps of 0xfe060000 and 0xfe850000.
- (2026-11-12, device) Mesa can't keep its shader cache on the RGB20 Pro: `/root/.cache` is on the read-only root filesystem, so every start compiles its shaders again. Source: Specular's log.
- (2026-11-11, device) Specular starts on the RGB20 Pro and draws its first screen; the battery, buttons, time zone and SD card services start without errors. Source: Specular's log, a `SPECULAR_SNAPSHOT`.
