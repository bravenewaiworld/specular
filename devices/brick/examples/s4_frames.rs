//! Spike checks "Frames from another thread", "Composite vs direct" and "Latency"
//! (specular/ROADMAP.md, step 2): game frames from another thread, what showing them costs, and
//! how long a button press takes to reach the screen.
//!
//! A stand-in emulator thread with its own GL context (sharing objects with the UI's) makes a
//! 256×224 frame on the CPU, uploads it into a free texture and hands it over with a GL fence.
//! Like a core, it reads the buttons at the start of each frame: a frame carries the time of the
//! first press it saw. The UI waits on the fence on the GPU (no CPU stall) and shows the frame,
//! scaled 3×, in one of two ways:
//! - composite: as an image in a Slint scene (optionally with a menu panel sliding over it every
//!   3 s, to measure with UI motion too);
//! - direct: drawn by our own tiny shader, skipping Slint (what plain gameplay could use).
//!
//! The emulator is paced either by its own clock at a core's frame rate, or by the display: it
//! starts each frame right after a present, as an emulator synced to the display would.
//!
//! A texture goes back to the emulator only after the present that replaced it, with a fence the
//! emulator waits on before reusing it, so a texture is never overwritten while it can still be
//! drawn. If the UI falls behind, the emulator waits for a free texture.
//!
//! Prints every 5 s: frames made and shown, frames dropped (replaced before they were shown),
//! UI-thread CPU per shown frame; where a frame's time goes, from ready to the end of the present
//! that showed it (waiting for the UI thread, waiting to be drawn, drawing and swapping); and for
//! presses, the time from the press (kernel timestamp) to the end of the present of the first
//! frame made after it.
//!
//! Usage: s4_frames [composite|direct] [options]
//!   --hz N        emulator frame rate (default 60.0988, the NES/SNES NTSC rate)
//!   --vsync       pace the emulator by the display instead of --hz
//!   --menu        composite: slide a menu over the game every 3 s
//!   --seconds N   run time (default 20)
//!   --capture     composite: read back the 300th–302nd screen redraws before their swap, as raw
//!                 1024×768 RGBA in /tmp/capture-N.rgba

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use glow::HasContext;
use specular_brick::egl::{EglDisplay, SharedContext};
use specular_linux::input::{Input, clock};
use specular_brick::platform::{BrickPlatform, SCREEN};

const W: i32 = 256;
const H: i32 = 224;
/// Textures in circulation: one on screen, one waiting to be shown, one being filled.
const TEXTURES: usize = 3;

fn monotonic() -> Duration { clock(libc::CLOCK_MONOTONIC) }
fn micros(d: Duration) -> u64 { d.as_micros() as u64 }

/// A finished frame: its texture, the fence that signals when its upload is done, when it was
/// ready, the press it is the first to reflect (all CLOCK_MONOTONIC), and when the UI took it.
struct Frame { texture: glow::Texture, fence: glow::Fence, ready: Duration, press: Option<Duration>, taken: Duration }
/// A texture handed back, with the fence that signals when the GPU is done drawing from it.
struct Free { texture: glow::Texture, fence: Option<glow::Fence> }
// SAFETY: textures and fences are names valid in every context that shares objects.
unsafe impl Send for Frame {}
unsafe impl Send for Free {}

/// Shared between the threads: the newest unread press, and what to report.
#[derive(Default)]
struct Stats {
    /// Kernel time (µs) of the first press the emulator hasn't seen yet; 0 if none.
    press_us: AtomicU64,
    made: AtomicU64, shown: AtomicU64, dropped: AtomicU64,
    /// Sums over shown frames (µs): ready → taken by the UI → drawing starts → present returns.
    to_taken: AtomicU64, to_draw: AtomicU64, to_present: AtomicU64,
    presses: AtomicU64, press_us_sum: AtomicU64, press_us_max: AtomicU64,
}

/// Reads the pad on its own thread, so both modes see presses the same way.
fn watch_buttons(stats: Arc<Stats>) {
    let mut input = Input::new(&["TRIMUI Player1"]);
    let mut events = Vec::new();
    loop {
        let mut fds: Vec<libc::pollfd> = input.fds().map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }).collect();
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 1000) };
        input.retry(&mut events);
        input.read(&mut events);
        for e in events.drain(..).filter(|e| e.pressed) {
            let _ = stats.press_us.compare_exchange(0, micros(e.time).max(1), Relaxed, Relaxed);
        }
    }
}

/// How the emulator decides when to start a frame.
enum Pace { Clock(f64), AfterPresent(mpsc::Receiver<()>) }

/// The stand-in emulator: makes frames into free textures, calling `deliver` with each, until
/// `deliver` returns false or no texture comes back.
fn emulator(ctx: SharedContext, pace: Pace, free: mpsc::Receiver<Free>, recycle: mpsc::Sender<Free>,
            mut deliver: impl FnMut(Frame) -> bool, stats: Arc<Stats>) {
    ctx.make_current().expect("make shared context current");
    let gl = unsafe { glow::Context::from_loader_function_cstr(|n| ctx.proc_address(n)) };
    for _ in 0..TEXTURES { let _ = recycle.send(Free { texture: unsafe { make_texture(&gl) }, fence: None }); }
    drop(recycle);
    let mut pixels = vec![0u8; (W * H * 4) as usize];
    let start = Instant::now();
    for n in 0u32.. {
        match &pace {
            Pace::Clock(hz) => {
                let next = start + Duration::from_secs_f64(n as f64 / hz);
                if let Some(d) = next.checked_duration_since(Instant::now()) { std::thread::sleep(d); }
            }
            Pace::AfterPresent(go) => if n > 0 && go.recv().is_err() { return },
        }
        // Like a core, read the buttons at the start of the frame.
        let press = match stats.press_us.swap(0, Relaxed) { 0 => None, us => Some(Duration::from_micros(us)) };
        // A moving pattern, written byte by byte like a software renderer would. The red channel
        // is (x + n) & 255, which lets a read-back check that a shown frame is a single frame.
        for y in 0..H {
            for x in 0..W {
                let i = ((y * W + x) * 4) as usize;
                let band = ((x + n as i32 * 2) / 16 + y / 16) & 1;
                pixels[i] = (x + n as i32) as u8; pixels[i + 1] = (y * band) as u8; pixels[i + 2] = 160; pixels[i + 3] = 255;
            }
        }
        let Ok(Free { texture, fence }) = free.recv() else { return };
        let fence = unsafe {
            if let Some(f) = fence { gl.wait_sync(f, 0, glow::TIMEOUT_IGNORED); gl.delete_sync(f); }
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, W, H, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&pixels)));
            let f = gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0).unwrap();
            gl.flush(); // so the fence can signal without this thread doing more GL work
            f
        };
        stats.made.fetch_add(1, Relaxed);
        if !deliver(Frame { texture, fence, ready: monotonic(), press, taken: Duration::ZERO }) { return; }
    }
}

unsafe fn make_texture(gl: &glow::Context) -> glow::Texture {
    unsafe {
        let t = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(t));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, W, H);
        for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] { gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32); }
        t
    }
}

/// The UI side of the hand-over: the frame on screen and the one waiting for the next present.
struct Shown { on_screen: Option<Frame>, next: Option<Frame>, free: mpsc::Sender<Free>, go: Option<mpsc::Sender<()>> }

impl Shown {
    /// Takes a new frame to show at the next present (the UI context must be current). A frame
    /// still waiting is dropped, its press passed on to the new one.
    fn arrive(&mut self, gl: &glow::Context, mut f: Frame, stats: &Stats) {
        f.taken = monotonic();
        unsafe { gl.wait_sync(f.fence, 0, glow::TIMEOUT_IGNORED); }
        if let Some(old) = self.next.take() {
            stats.dropped.fetch_add(1, Relaxed);
            f.press = old.press.or(f.press);
            self.release(gl, old);
        }
        self.next = Some(f);
    }

    /// After a present (`draw` = when drawing started): the waiting frame is now on screen, and
    /// the one it replaced is free.
    fn presented(&mut self, gl: &glow::Context, draw: Duration, stats: &Stats) {
        let Some(f) = self.next.take() else { return };
        let now = monotonic();
        stats.shown.fetch_add(1, Relaxed);
        stats.to_taken.fetch_add(micros(f.taken.saturating_sub(f.ready)), Relaxed);
        stats.to_draw.fetch_add(micros(draw.saturating_sub(f.taken)), Relaxed);
        stats.to_present.fetch_add(micros(now.saturating_sub(draw.max(f.taken))), Relaxed);
        if let Some(p) = f.press {
            let l = micros(now.saturating_sub(p));
            stats.presses.fetch_add(1, Relaxed);
            stats.press_us_sum.fetch_add(l, Relaxed);
            stats.press_us_max.fetch_max(l, Relaxed);
        }
        if let Some(old) = self.on_screen.replace(f) { self.release(gl, old); }
        if let Some(go) = &self.go { let _ = go.send(()); }
    }

    fn release(&self, gl: &glow::Context, f: Frame) {
        unsafe { gl.delete_sync(f.fence); }
        let fence = unsafe { gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0).ok() };
        let _ = self.free.send(Free { texture: f.texture, fence });
    }
}

/// Prints one report line from the stats since the previous call (which this resets).
struct Reporter { prev: [u64; 6], cpu: Duration, at: Instant }
impl Reporter {
    fn new() -> Self { Self { prev: [0; 6], cpu: clock(libc::CLOCK_THREAD_CPUTIME_ID), at: Instant::now() } }
    fn report(&mut self, s: &Stats) {
        let now = [&s.made, &s.shown, &s.dropped, &s.to_taken, &s.to_draw, &s.to_present].map(|a| a.load(Relaxed));
        let d: Vec<u64> = now.iter().zip(self.prev).map(|(a, b)| a - b).collect();
        let (presses, sum, max) = (s.presses.swap(0, Relaxed), s.press_us_sum.swap(0, Relaxed), s.press_us_max.swap(0, Relaxed));
        let (cpu, wall) = (clock(libc::CLOCK_THREAD_CPUTIME_ID), self.at.elapsed().as_secs_f64());
        let ms = |us: u64, n: u64| if n > 0 { us as f64 / n as f64 / 1e3 } else { 0.0 };
        println!("{:.1} made/s, {:.1} shown/s, {} dropped, UI {:.2} ms CPU/frame | ready→taken {:.1}, →drawing {:.1}, →presented {:.1} ms | {} presses: {:.1} avg, {:.1} max ms to present",
            d[0] as f64 / wall, d[1] as f64 / wall, d[2],
            if d[1] > 0 { (cpu - self.cpu).as_secs_f64() * 1e3 / d[1] as f64 } else { 0.0 },
            ms(d[3], d[1]), ms(d[4], d[1]), ms(d[5], d[1]), presses, ms(sum, presses), max as f64 / 1e3);
        (self.prev, self.cpu, self.at) = (now, cpu, Instant::now());
    }
}

struct Options { direct: bool, hz: f64, vsync: bool, menu: bool, seconds: u64, capture: bool }

fn options() -> Options {
    let args: Vec<String> = std::env::args().collect();
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok());
    Options {
        direct: args.get(1).is_some_and(|m| m == "direct"),
        hz: value("--hz").unwrap_or(60.0988),
        vsync: args.iter().any(|a| a == "--vsync"),
        menu: args.iter().any(|a| a == "--menu"),
        seconds: value("--seconds").unwrap_or(20.0) as u64,
        capture: args.iter().any(|a| a == "--capture"),
    }
}

/// The emulator's pacing, and (if paced by the display) the sender the UI signals after presents.
fn pace(o: &Options) -> (Pace, Option<mpsc::Sender<()>>) {
    if o.vsync { let (tx, rx) = mpsc::channel(); (Pace::AfterPresent(rx), Some(tx)) } else { (Pace::Clock(o.hz), None) }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let o = options();
    let stats = Arc::new(Stats::default());
    let s = stats.clone();
    std::thread::spawn(move || watch_buttons(s));
    if o.direct { run_direct(&o, stats) } else { run_composite(&o, stats) }
}

/// Direct: our own loop. Each new frame is drawn with one textured quad and presented.
fn run_direct(o: &Options, stats: Arc<Stats>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let egl = EglDisplay::open()?;
    let gl = unsafe { glow::Context::from_loader_function_cstr(|n| egl.proc_address(n)) };
    let ctx = egl.shared_context()?;
    let (free_tx, free_rx) = mpsc::channel();
    let (frame_tx, frame_rx) = mpsc::channel::<Frame>();
    let (pace, go) = pace(o);
    let (s, recycle) = (stats.clone(), free_tx.clone());
    std::thread::spawn(move || emulator(ctx, pace, free_rx, recycle, move |f| frame_tx.send(f).is_ok(), s));

    let program = unsafe { blit_program(&gl)? };
    let mut shown = Shown { on_screen: None, next: None, free: free_tx, go };
    let mut reporter = Reporter::new();
    let (end, mut next_report) = (Instant::now() + Duration::from_secs(o.seconds), Instant::now() + Duration::from_secs(5));
    while Instant::now() < end {
        // Wait for a frame; keep only the newest.
        let Ok(f) = frame_rx.recv_timeout(Duration::from_millis(100)) else { continue };
        shown.arrive(&gl, f, &stats);
        while let Ok(f) = frame_rx.try_recv() { shown.arrive(&gl, f, &stats); }
        let draw = monotonic();
        let texture = shown.next.as_ref().unwrap().texture;
        unsafe {
            gl.viewport(0, 0, SCREEN.width as i32, SCREEN.height as i32);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.viewport((SCREEN.width as i32 - W * 3) / 2, (SCREEN.height as i32 - H * 3) / 2, W * 3, H * 3);
            gl.use_program(Some(program));
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
        egl.swap_buffers()?;
        shown.presented(&gl, draw, &stats);
        if Instant::now() >= next_report { reporter.report(&stats); next_report += Duration::from_secs(5); }
    }
    Ok(())
}

unsafe fn blit_program(gl: &glow::Context) -> Result<glow::Program, String> {
    unsafe {
        // A full-viewport quad from gl_VertexID (no vertex buffer), flipped so row 0 is the top.
        let vs = "#version 300 es\nout vec2 uv;\nvoid main() { vec2 p = vec2(gl_VertexID & 1, gl_VertexID >> 1);\n uv = vec2(p.x, 1.0 - p.y); gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0); }";
        let fs = "#version 300 es\nprecision mediump float;\nin vec2 uv; uniform sampler2D tex; out vec4 color;\nvoid main() { color = texture(tex, uv); }";
        let program = gl.create_program()?;
        for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
            let s = gl.create_shader(kind)?;
            gl.shader_source(s, src);
            gl.compile_shader(s);
            if !gl.get_shader_compile_status(s) { return Err(gl.get_shader_info_log(s)); }
            gl.attach_shader(program, s);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) { return Err(gl.get_program_info_log(program)); }
        Ok(program)
    }
}

// Some colours are written as rgb(): inside slint!, a hex colour whose digits read like a number
// with an exponent is rejected by Rust's tokenizer.
slint::slint! {
    import "../../../assets/fonts/Geist.ttf";
    export component Scene inherits Window {
        in property <image> frame;
        in property <bool> menu;
        background: black;
        default-font-family: "Geist";
        Image {
            source: root.frame; image-rendering: pixelated;
            width: 768px; height: 672px; x: (root.width - self.width) / 2; y: (root.height - self.height) / 2;
        }
        Rectangle {
            width: 470px; height: root.height;
            x: root.menu ? 0 : -self.width;
            animate x { duration: 240ms; easing: cubic-bezier(0.2, 0, 0, 1); }
            background: rgba(8, 9, 10, 82%);
            for item[i] in ["Resume", "Save state", "Load state", "Quit game"] : Text {
                x: 48px; y: 200px + i * 92px; text: item; font-size: 42px; color: i == 0 ? rgb(242, 242, 242) : rgb(138, 143, 150);
            }
        }
    }
}

thread_local! {
    /// Runs on the UI thread for each frame the emulator delivers (composite mode).
    static ON_FRAME: RefCell<Option<Box<dyn FnMut(Frame)>>> = RefCell::default();
}

/// Composite: the frame is an image in the Slint scene; every new frame redraws the scene.
fn run_composite(o: &Options, stats: Arc<Stats>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let platform = BrickPlatform::new()?;
    let egl = platform.egl();
    let gl = Rc::new(unsafe { glow::Context::from_loader_function_cstr(|n| egl.proc_address(n)) });
    let ctx = egl.shared_context()?;
    let (free_tx, free_rx) = mpsc::channel();
    let (pace, go) = pace(o);
    let shown = Rc::new(RefCell::new(Shown { on_screen: None, next: None, free: free_tx.clone(), go }));
    let draw_started = Rc::new(Cell::new(Duration::ZERO));
    let (sh, g, s, d) = (shown.clone(), gl.clone(), stats.clone(), draw_started.clone());
    platform.on_present(move |_| sh.borrow_mut().presented(&g, d.get(), &s));
    slint::platform::set_platform(Box::new(platform)).map_err(|e| format!("{e:?}"))?;
    let scene = Scene::new()?;

    let (weak, g, s) = (scene.as_weak(), gl.clone(), stats.clone());
    ON_FRAME.with(|f| *f.borrow_mut() = Some(Box::new(move |frame: Frame| {
        let image = unsafe {
            slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(frame.texture.0, (W as u32, H as u32).into()).build()
        };
        shown.borrow_mut().arrive(&g, frame, &s);
        if let Some(scene) = weak.upgrade() { scene.set_frame(image); }
    })));
    let s = stats.clone();
    std::thread::spawn(move || emulator(ctx, pace, free_rx, free_tx, |f| {
        slint::invoke_from_event_loop(move || ON_FRAME.with(|h| if let Some(h) = h.borrow_mut().as_mut() { h(f) })).is_ok()
    }, s));

    let (capture, mut n) = (o.capture, 0);
    scene.window().set_rendering_notifier(move |state, _| {
        match state {
            slint::RenderingState::BeforeRendering => draw_started.set(monotonic()),
            // --capture: read back what was drawn, just before the swap.
            slint::RenderingState::AfterRendering if capture => {
                n += 1;
                if !(300..303).contains(&n) { return; }
                let mut px = vec![0u8; (SCREEN.width * SCREEN.height * 4) as usize];
                unsafe { gl.read_pixels(0, 0, SCREEN.width as i32, SCREEN.height as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px))); }
                let _ = std::fs::write(format!("/tmp/capture-{n}.rgba"), px);
            }
            _ => {}
        }
    }).map_err(|e| format!("{e:?}"))?;
    scene.show()?;

    let toggle = slint::Timer::default();
    if o.menu {
        let weak = scene.as_weak();
        toggle.start(slint::TimerMode::Repeated, Duration::from_secs(3), move || { if let Some(s) = weak.upgrade() { s.set_menu(!s.get_menu()); } });
    }
    let mut reporter = Reporter::new();
    let report = slint::Timer::default();
    report.start(slint::TimerMode::Repeated, Duration::from_secs(5), move || reporter.report(&stats));
    let stop = slint::Timer::default();
    stop.start(slint::TimerMode::SingleShot, Duration::from_secs(o.seconds), || { let _ = slint::quit_event_loop(); });
    scene.run()?;
    Ok(())
}
