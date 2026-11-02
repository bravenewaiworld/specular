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

A package is a ROM like any other: a file in `roms/<system>/` that Specular scans, identifies and shows. Which native core plays it is decided by its system (`CoreDecl::native`): the GameCube's (`roms/gc/*.tesseract`) is Tesseract. A package is an uncompressed tar (ustar) whose first entry, `manifest.json`, names its `core`, its `system` and its `program` (`specular_runtime::native::package`); the rest belongs to the core (Tesseract's: the disc image and the IPL fonts, read in place; its converter/README.md). Specular takes the program out into `cache/native/<the ROM's path>/program` (again only when the package changes) and runs it there; that folder is also the core's cache. The game's saves go in `saves/<the ROM's path without its extension>/`. Specular doesn't ship a native core: each package brings its program, and its licences.

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
    pub fn start(launch: NativeLaunch, out: NativeOutputs) -> Result<NativeSession, StartFailure>; // waits for STARTED
    pub fn info(&self) -> &NativeInfo;
    pub fn set_buttons(&self, bits: u32);   // native::button_bit; doesn't wait
    pub fn frames(&self) -> u64;            // presented so far: the watchdog, the CPU minimum
    pub fn resume(&self) -> Result<(), String>;
    pub fn pause(&self) -> Result<(), String>;    // the picture goes to NativeOutputs::frames
    pub fn save_state(&self, path: &Path) -> io::Result<()>;
    pub fn load_state(&self, path: &Path) -> io::Result<()>;
    pub fn set_sound(&self, out: Option<Box<dyn SoundOut>>);
    pub fn quit(self) -> Result<(), String>;          // kills a core that doesn't exit in 5 s; dropping quits too
    pub fn abandon(self);                             // kills it: the card can't be written
    pub fn watch(&self, limit: Duration, stuck: impl FnOnce() + Send + 'static); // no frame for `limit` while running
}
```

A core that exits or closes its socket unasked has stopped: `NativeOutputs::on_stop` gets its `ERROR` text, if it sent one, and Specular takes the screen back. One that fails before `STARTED` makes `start` fail with `StartFailure::Said` and its text.

Shared memory (`SpnShared`): the buttons and their change time (a sequence lock), the sound ring (stereo 16-bit, 8192 frames, at the rate the core sets), the frames presented and the last frame's time, and the picture (RGBA, up to 1024x1024) written before `PAUSED`.

### In the UI

`specular_ui::game::GameSession` is a libretro or a native session; the menu, quitting, sleep and the sound outputs work the same for both. A native game starts paused; once the launch splash has had its time it is resumed, unless its menu is open. Resuming sets `specular_core::device::SCREEN_LENT`, which stops the device's platform drawing (the Brick's draws two frames once it is cleared: the first can tear); pausing takes the picture the core hands back and draws the menu over it. While a native core runs, the CPU's minimum is its top step (default when paused). What it doesn't have says so: save states ("Save states aren't supported for these games yet"), disc changes, reset, core options.

Failures: a package that isn't one, or is for another core, is refused before the core starts; what a core says as it fails (`SPN_ERROR`, before or after starting) is told to the player as said; a core that ends unasked ends the game ("The game stopped. It closed unexpectedly."). The watchdog ends a core that presents nothing for `STUCK_LIMIT` while it has the screen, and the game with it ("It stopped responding."); Specular carries on, unlike with a stuck libretro core, which takes Specular down with it.

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
- Launch records: a native game leaves none (Specular survives a stuck or crashed core, so there is nothing to report after a restart).
- Toasts over a running game: Specular can't draw while the core has the screen. The core could draw an image Specular hands it (Tesseract already draws an overlay).
