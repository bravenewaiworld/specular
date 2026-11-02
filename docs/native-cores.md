# Native cores

Status: In progress · Updated: 2026-11-02

Specular's own core protocol, beside libretro: how a core that is a program of its own plays a game in Specular as seamlessly as a libretro core does. Its first user is Tesseract, the GameCube recompiler: each game is converted on a PC into a package, which Specular lists as a ROM of the GameCube system and plays through Tesseract's core. Native cores matter because some systems are only playable on the device as recompiled programs, which fit neither libretro's model (a library called once per frame, handing pixels back) nor its costs (a copy of every frame, and Specular's GL drawing it).

## Decisions

- **A native core is a program of its own, which Specular starts for a game and talks to over a socket and shared memory** (2026-11-02). The game runs exactly as fast as it does alone; a crash or fatal error ends the game, not Specular; quitting frees everything. Rejected: a library in Specular's process, as libretro cores are. On the Brick, F-Zero GX built as a shared library ran 12 % slower than as a program (see Findings), and Tesseract's runtime can't be shut down and started again in one process (its GPU threads can't be stopped safely, it ends with `_exit`, and it sets process-wide signal handlers).
- **The core draws straight to the screen while the game plays, and hands the screen back for everything else** (2026-11-02). Specular stops drawing before it resumes the core; when the game menu opens, the core stops presenting, hands back a copy of its picture and parks, and Specular draws the menu over that picture. Players can't tell the difference from a libretro core. Rejected: Specular compositing the core's frames every frame (a full-screen copy each frame on a device whose memory bus is the GameCube games' bottleneck, plus the UI thread's ~4 ms of CPU a frame).
- **The screen keeps its size (1024x768 on the Brick) while a native core plays** (2026-11-02). Shrinking `fb0` to the game's 640x480 under a live EGL surface garbles frames and hangs the driver for seconds; doing it safely means tearing down and rebuilding Specular's EGL and renderer at every handoff (est. 0.1–0.75 s). Keeping full size costs about 1 p95 fps (2 %) in F-Zero GX's race (see Findings). Rejected: shrinking `fb0` as Tesseract's own Brick script does.
- **Specular owns the buttons, the sound output and where saves go** (2026-11-02). Buttons are written to shared memory as they change and read by the core when the game polls; sound goes through a ring in shared memory into Specular's sound output, so volume, headphones and the menu's sounds behave as with any core. Menu, the volume keys and Power never reach the core.

## Design

### Packages

A package is a ROM like any other: a file in `roms/<system>/` that Specular scans, identifies and shows. Which native core plays it is decided by its system; the package's format belongs to the core that makes and reads it (Tesseract's converter). For now Tesseract's package is its disc image.

### The protocol

`crates/specular-runtime/native/specular_native.h` defines it (MIT, for cores to copy); `spn_client.c` is a core's side of it in C, and `mock_core.c` a core for the tests. Specular's side is `specular_runtime::native`.

Specular runs the core's program as `PROGRAM --specular FD`, FD one end of a `SOCK_SEQPACKET` socket pair. Each message is one packet: a header (`type`, `text_len`, `a`, `b`) and UTF-8 text.

```
core      → Specular  HELLO (a: protocol 1; "NAME\tVERSION")
Specular  → core      START (SCM_RIGHTS: the shared memory; "package=…\nsaves=…\ncache=…\nclock=…\n")
core      → Specular  STARTED (a, b: the largest picture's size; frames per second). It boots, presenting nothing.
Specular  → core      RESUME   once Specular has stopped drawing     → RESUMED; the core draws
Specular  → core      PAUSE                                          → PAUSED; picture in shared memory; the core draws nothing
Specular  → core      SAVE_STATE / LOAD_STATE (path), while paused   → STATE_DONE (a: 0 or errno; why)
Specular  → core      QUIT, while paused                             → saves written; exits 0
core      → Specular  SAVED (the game saved), RUMBLE (a: strength), ERROR (why; then it exits)
```

```rust
impl NativeSession {
    pub fn start(launch: NativeLaunch, out: NativeOutputs) -> Result<NativeSession, String>; // waits for STARTED
    pub fn info(&self) -> &NativeInfo;
    pub fn set_buttons(&self, bits: u32);   // native::button_bit; doesn't wait
    pub fn frames(&self) -> u64;            // presented so far: the watchdog, the CPU minimum
    pub fn resume(&mut self) -> Result<(), String>;
    pub fn pause(&mut self) -> Result<(), String>;    // the picture goes to NativeOutputs::frames
    pub fn save_state(&mut self, path: &Path) -> io::Result<()>;
    pub fn load_state(&mut self, path: &Path) -> io::Result<()>;
    pub fn set_sound(&self, out: Option<Box<dyn SoundOut>>);
    pub fn quit(self) -> Result<(), String>;          // kills a core that doesn't exit in 5 s; dropping quits too
}
```

A core that exits or closes its socket unasked has stopped: `NativeOutputs::on_stop` gets its `ERROR` text (or "the game stopped unexpectedly"), and Specular takes the screen back.

Shared memory (`SpnShared`): the buttons and their change time (a sequence lock), the sound ring (stereo 16-bit, 8192 frames, at the rate the core sets), the frames presented and the last frame's time, and the picture (RGBA, up to 1024x1024) written before `PAUSED`.

### Tesseract's side

`runtime/system/specular.c`: Specular's buttons go through the Brick's own pad mapping (`pad.h`), the AI DMA's samples go to the ring, and each VI field counts a frame. Pausing stops presenting, waits for a copy of the next frame (`gx_snapshot`, which also waits until every earlier present has reached the driver), then parks the guest at its next field. The game starts parked, before its first field. The memory card is `SAVES/memcard-A.raw` and the pipeline cache `CACHE/pipelines`.

## Findings

- (2026-11-02, device) On the Brick, Specular's EGL window surface and a `VK_KHR_display` swapchain in another process take turns on `fb0` with neither exiting: whichever presents last owns the screen. The game's swapchain is up 0.8 s after launch with Specular's surface alive (as without it); EGL is back on screen 2–8 ms after the game stops presenting. EGL's first frame after taking the screen back can tear (once in four): draw two. Source: experiments on the Brick.
- (2026-11-02, device) Changing `fb0`'s size under a live EGL surface garbles or drops frames; a swap hung 2.9 s and a surface's destruction 16.5 s, with a PowerVR stalled-context warning. `eglTerminate`, `fbset`, `eglInitialize` works (17–31 ms plus 18–22 ms) but loses every GL object. Source: as above.
- (2026-11-02, device) F-Zero GX's race, paced and on screen: `fb0` at 640x480 gives p95 54.7 fps, 909 MB/s of GPU memory traffic; at 1024x768 53.6 fps, 1052 MB/s (and the display engine's 70 vs 179 MB/s). Mean 59.0 both. Source: as above, 3 alternating runs each.
- (2026-11-02, device) F-Zero GX as a shared library loaded by a small host, unpaced: 116.7 s for the race input vs 99.8 s as a program (mean 67.7 vs 76.5 fps); `-Bsymbolic` didn't help. Source: as above.

- (2026-11-02, device) F-Zero GX as a native core on the Brick, driven by `examples/native_drive.rs` (no UI; `fb0` at 1024x768): 226–425 ms from launch to STARTED; RESUMED within 1.2 ms, the game's first flip 88 ms after the first resume and 4 ms after later ones; 1199 frames in 20 s; PAUSED in 47–50 ms, after which the screen doesn't change until the next resume; the paused picture 640x448 and correct; quit in 142 ms, exit status 0; the memory card and pipeline cache made in the folders Specular gave. Source: the Brick.

## Open questions

- Save states: Tesseract's runtime has none yet (guest RAM, ARAM, the guest threads, the hardware's and the GPU's state).
- Toasts over a running game: Specular can't draw while the core has the screen. The core could draw an image Specular hands it (Tesseract already draws an overlay).
