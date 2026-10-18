# TrimUI Brick

Status: Implemented · Updated: 2026-10-18

The TrimUI Brick port (`devices/brick`): how Specular draws on the Brick's display, how each hardware service maps onto it, and what was measured there. The Brick (including the all-aluminium Hammer edition) is the reference device, so its measurements back many decisions in the other docs.

## Decisions

- **The TrimUI Brick is the reference device and the first port** (2026-10-12).
- **The stack was checked on the Brick before building on it** (2026-10-12). The spike's platform became `devices/brick/src`, and its measurements (`devices/brick/examples/s*`) stay as a regression check.
- **Our kernel implements `FBIO_WAITFORVSYNC` on `/dev/fb0`** (2026-10-12), so games can be paced by the display (see [runtime.md](runtime.md)). Rejected: inferring the vsync phase from when a blocking swap returns (kept as the approach for a device whose kernel lacks the call).
- **Backlight levels 1–10 map to 16–255 on a curve** (2026-10-12), since perceived brightness is roughly logarithmic; level 1 stays readable indoors.
- **The LEDs are limited to about 40/255** (2026-10-12), as TrimUI's firmware does: they are very bright at full power.
- **Volume steps are 2 dB apart, the top one at 0 dB** (2026-10-13). Louder only clips.
- **Long or detached runs on the device go over SSH, not adb** (2026-10-12). The stock adbd kills a session's processes when the session exits.

## Design

### Profile

1024×768 at 400 ppi (so 1 u is one pixel), refreshing at about 60.2 Hz. Buttons: the d-pad, A, B, X, Y, L1, R1, L2, R2, Select, Start, Menu, F1, F2, the volume keys, Power, and the FN switch, used as Mute. 14 RGB LEDs in chain order (LED-class names `sunxi_led0r` … `sunxi_led13b`): the 8 along the top show the first accent; F2 and F1 on the front, then the 4 under the shoulder buttons, show the second.

The OS's service manager runs Specular as the frontend and restarts it when it exits. If it fails 3 times within a minute, the system falls back to its `status` screen until the next boot (2026-10-13).

### Display platform

TrimUI's closed PowerVR driver has only PowerVR's "NULL window system": no windowing at all. A window surface created without a native window renders straight to the panel (`/dev/fb0`, whose buffers the display driver swaps on vsync). So there is no DRM, Wayland or X11, and Specular brings its own Slint platform: one full-screen window drawn with FemtoVG on a GLES 3 context, and its own event loop. The loop draws only when something changed or an animation runs; otherwise it sleeps in `poll()` until the next Slint timer, a wake-up from another thread, a button, or the next try to open a missing input device. While animating, the swap paces the loop at the display's rate. All PowerVR specifics are in `egl.rs`.

GL functions are looked up in `libGLESv2`'s exported symbols first, then through `eglGetProcAddress`. The EGL and GLES libraries are loaded at runtime.

Buttons come from two evdev devices: padbridge's virtual pad (`TRIMUI Player1`), which presents the Brick's buttons as a standard gamepad and reports the FN switch as `SW_TABLET_MODE`, and the power key of the AXP2202 power chip (`axp2202-pek`). Either can appear late or restart; they are reopened.

### Hardware services

| Service | How |
|---|---|
| Battery | power-supply class (`specular-linux`) |
| Backlight, panel on/off | Allwinner's `/dev/disp` ioctls (`DISP_LCD_SET_BRIGHTNESS`, `DISP_DEVICE_SWITCH`); the kernel has no `/sys/class/backlight` |
| LEDs | LED class (`specular-linux`), capped as above |
| Volume | the codec's digital "DAC volume" through `amixer` (0.75 dB per step, 159 = 0 dB); one level for speaker and headphone jack. Bluetooth volume through bluez-alsa |
| Sound | ALSA `default`; Bluetooth through bluez-alsa |
| Vsync | `FBIO_WAITFORVSYNC` on `/dev/fb0`; without it, a timer |
| Sleep | `/usr/bin/sleepnow`, which also stops WiFi around sleep (its driver can't suspend while connected) |
| Power off, restart | `/sbin/poweroff`, `/sbin/reboot` |
| WiFi | wpa_supplicant on `wlan0`. Power on loads the XR829's `xr829_wlan` driver and starts the runit services `wpa_supplicant` and `wifi-dhcp`; power off stops them (waiting for wpa_supplicant to exit) and unloads the driver. On joining, the old address is flushed and udhcpc restarted, so it asks at once rather than at its next retry (up to 20 s away) |
| Bluetooth | BlueZ |
| Installer | `brick-install` (see [installer.md](installer.md)) |

Power-key presses are ignored from the request to sleep until 1.5 s after waking: the press that woke the device is delivered as it resumes, before the sleep command returns, and must not send it straight back to sleep.

### Fonts on TrimUI's stock system

TrimUI's stock system ships fontconfig but no fonts, and Slint's text layout (fontique) panics when fontconfig finds none. When fontconfig is present, `fonts.rs` writes Geist and a config naming it under `/tmp/specular-fonts` and points `FONTCONFIG_FILE` at it. Fontconfig is loaded at runtime, so our own system image can leave it out.

## Findings

Unless noted, measured with the CPU at the 600–816 MHz the `ondemand` governor picks for the load.

- (2026-10-12, device) TrimUI's GPU libraries offer OpenGL ES 3.2 and EGL 1.4, with only PowerVR's NULL window system. The display runs at about 60.2 Hz. Source: `devices/brick/examples/s0_toolchain.rs`, `s1_slint.rs`.
- (2026-10-12, device) A Rust `aarch64-unknown-linux-gnu` binary linked with the cross toolchain (GCC 6.4, glibc 2.23) runs, needing glibc 2.18 symbols at most. It is dynamic, since it loads the GPU libraries (and cores) at runtime. Source: `s0_toolchain.rs`.
- (2026-10-12, device) Slint's FemtoVG renderer on our EGL holds 60.2 fps, locked to vsync, with no late frames. A full redraw costs about 4 ms of CPU; the rest of each 16.6 ms is waiting for vsync. An idle screen draws nothing: about 0.5 % CPU. Source: `s1_slint.rs`.
- (2026-10-12, device) `eglGetProcAddress` also returns core GL functions in our own context (EGL 1.4 doesn't promise it). Source: `s1_slint.rs`.
- (2026-10-12, device) A press reaches the screen in 7–10 ms on average (worst 12 ms), from the kernel's timestamp to the end of the present, with evdev read inside our event loop and injected presses (padbridge's own hop not included). Source: `s1_slint.rs`, `inject.rs`.
- (2026-10-12, device) An emulator thread with a shared, surfaceless GL context can upload CPU-made frames into textures and hand them over with a GL fence; Slint shows them as borrowed textures, and a texture goes back only after the present that replaced it. No tearing: each read-back frame held exactly one emulator frame. Source: `s4_frames.rs`.
- (2026-10-12, device) Drawing each game frame through the Slint scene costs 2.8–3.3 ms of UI-thread CPU per frame, against 2.1–2.3 ms for our own one-quad blit. With the emulator paced by the display, latency is the same either way: a press reaches the end of the present 24–25 ms later on average (33 ms worst). An emulator on its own clock (60.0988 Hz) drifts against the display (60.2 Hz), and its frames sometimes wait a whole refresh: 21–38 ms on average. Source: `s4_frames.rs`.
- (2026-10-12, device) Draw-then-swap puts a frame on screen 32 ms (two vsyncs) after it was started: the swap waits for the previous frame's flip, and the new frame flips at the vsync after that. Starting each frame only once the previous one is on screen takes 16 ms, at the same 60 fps. Also delaying the start by 10 ms takes 6 ms, still at 60 fps; at 13 ms, frames start to miss the vsync. Measured with the display driver's record of the flip (`/dev/fb0`'s pan offset), which may differ from the panel by up to a vsync. Source: `s5_scanout.rs`.
- (2026-10-12, upstream) TrimUI's display driver has a vsync wait internally (`fb_wait_for_vsync`), but its `FBIO_WAITFORVSYNC` ioctl is stubbed out and returns at once. Source: the Brick's kernel source.
- (2026-10-12, device) With our kernel's `FBIO_WAITFORVSYNC`, the wait returns when the display takes a new frame, and buffer switches follow it by a median 0.1 ms. Waiting for it, then drawing: the swap no longer blocks (about 1 ms), 60 fps, 16.6 ms from frame start to on screen; with a 10 ms frame delay, 6.5 ms. Source: `s5_scanout.rs`.
- (2026-10-12, device) The spike used 20 MB resident with the scene, fonts and frame textures. The first frame was on screen about 1.1 s after `main()` started. Source: the spike (`devices/brick/examples`).
- (2026-10-12, device) Geist from 27 to 96 px is clean on the panel. Source: `s1_slint.rs`.
- (2026-10-12, device) The GL context and textures survive sleep, and rendering resumes without errors. Source: `s1_slint.rs`.
- (2026-10-12, device) After `sleepnow` restarts WiFi, the WiFi driver can leave the system in a state where every swap waits about two vsyncs (25–50 fps instead of 60), with "miss interrupt" warnings from the XR829. Restarting WiFi again fixes it. Tracked in Crystaline OS's roadmap. Source: Brick sleep tests.
- (2026-10-12, inferred) Press to photons: a press waits about 8 ms on average for the next frame to read it, the frame then takes 32 ms (draw-then-swap) or about 6 ms (paced, 10 ms frame delay) to go on screen, and scanout takes up to 16 ms to reach a given row: about 48 ms or about 22 ms to the middle of the screen. Source: the measurements above.
- (2026-10-13, device) TrimUI's ALSA set-up has a software volume ("Soft Volume Master"), but the driver refuses writes to it. The DAC volume control goes up to +72 dB. Source: `amixer` on the Brick.
- (2026-10-14, device) The XR829 carries WiFi and Bluetooth on one chip; loading and unloading the WiFi driver powers only the WiFi part. Source: `devices/brick/src/wifi.rs`.
