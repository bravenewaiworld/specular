//! Spike checks "Frames from another thread" and "Composite vs direct" (specular/ROADMAP.md,
//! step 2): game frames from another thread, and what showing them costs.
//!
//! A stand-in emulator thread with its own GL context (sharing objects with the UI's) makes a
//! 256×224 frame on the CPU at an emulator core's frame rate, uploads it into a free texture and
//! hands it over with a GL fence. The UI waits on the fence on the GPU (no CPU stall) and shows the
//! frame, scaled 3×, in one of two ways:
//! - composite: as an image in a Slint scene, with a menu panel sliding over it every 3 s;
//! - direct: drawn by our own tiny shader, skipping Slint (what plain gameplay could use).
//!
//! A texture goes back to the emulator only after the present that replaced it, with a fence the
//! emulator waits on before reusing it, so a texture is never overwritten while it can still be
//! drawn. If the UI falls behind, the emulator waits for a free texture.
//!
//! Prints every 5 s: frames made and shown, frames dropped (replaced before they were shown),
//! UI-thread CPU per shown frame, and the time from a frame being ready to the end of the present
//! that showed it.
//!
//! Usage: s4_frames [composite|direct] [core rate in Hz (default 60.0988, the NES/SNES NTSC rate)]
//!        [seconds (default 20)] [--capture: in composite mode, read back the 300th–302nd screen
//!        redraws before their swap, as raw 1024×768 RGBA in /tmp/capture-N.rgba]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use glow::HasContext;
use specular_brick::egl::{EglDisplay, SharedContext};
use specular_brick::input::clock;
use specular_brick::platform::{BrickPlatform, SCREEN};

const W: i32 = 256;
const H: i32 = 224;
/// Textures in circulation: one on screen, one waiting to be shown, one being filled.
const TEXTURES: usize = 3;

/// A finished frame: its texture, the fence that signals when its upload is done, and when it was
/// ready (CLOCK_MONOTONIC).
struct Frame { texture: glow::Texture, fence: glow::Fence, ready: Duration }
/// A texture handed back, with the fence that signals when the GPU is done drawing from it.
struct Free { texture: glow::Texture, fence: Option<glow::Fence> }
// SAFETY: textures and fences are names valid in every context that shares objects.
unsafe impl Send for Frame {}
unsafe impl Send for Free {}

fn monotonic() -> Duration { clock(libc::CLOCK_MONOTONIC) }

#[derive(Default)]
struct Counters { made: AtomicU64, shown: AtomicU64, dropped: AtomicU64, latency_us: AtomicU64 }

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

/// The stand-in emulator: makes frames at `hz` into free textures, calling `deliver` with each,
/// until `deliver` returns false or no texture comes back.
fn emulator(ctx: SharedContext, hz: f64, free: mpsc::Receiver<Free>, recycle: mpsc::Sender<Free>,
            mut deliver: impl FnMut(Frame) -> bool, counters: Arc<Counters>) {
    ctx.make_current().expect("make shared context current");
    let gl = unsafe { glow::Context::from_loader_function_cstr(|n| ctx.proc_address(n)) };
    for _ in 0..TEXTURES { let _ = recycle.send(Free { texture: unsafe { make_texture(&gl) }, fence: None }); }
    drop(recycle);
    let mut pixels = vec![0u8; (W * H * 4) as usize];
    let period = Duration::from_secs_f64(1.0 / hz);
    let start = Instant::now();
    for n in 0u32.. {
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
        counters.made.fetch_add(1, Relaxed);
        if !deliver(Frame { texture, fence, ready: monotonic() }) { return; }
        let next = start + period * (n + 1);
        if let Some(d) = next.checked_duration_since(Instant::now()) { std::thread::sleep(d); }
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
struct Shown { on_screen: Option<Frame>, next: Option<Frame>, free: mpsc::Sender<Free> }

impl Shown {
    /// Takes a new frame to show at the next present (the UI context must be current). A frame
    /// still waiting is dropped.
    fn arrive(&mut self, gl: &glow::Context, f: Frame, counters: &Counters) {
        unsafe { gl.wait_sync(f.fence, 0, glow::TIMEOUT_IGNORED); }
        if let Some(old) = self.next.replace(f) { counters.dropped.fetch_add(1, Relaxed); self.release(gl, old); }
    }

    /// After a present: the waiting frame is now on screen, and the one it replaced is free.
    fn presented(&mut self, gl: &glow::Context, counters: &Counters) {
        let Some(f) = self.next.take() else { return };
        counters.shown.fetch_add(1, Relaxed);
        counters.latency_us.fetch_add(monotonic().saturating_sub(f.ready).as_micros() as u64, Relaxed);
        if let Some(old) = self.on_screen.replace(f) { self.release(gl, old); }
    }

    fn release(&self, gl: &glow::Context, f: Frame) {
        unsafe { gl.delete_sync(f.fence); }
        let fence = unsafe { gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE, 0).ok() };
        let _ = self.free.send(Free { texture: f.texture, fence });
    }
}

/// Prints one report line from the counters since the previous call.
struct Reporter { prev: [u64; 4], cpu: Duration, at: Instant }
impl Reporter {
    fn new() -> Self { Self { prev: [0; 4], cpu: clock(libc::CLOCK_THREAD_CPUTIME_ID), at: Instant::now() } }
    fn report(&mut self, c: &Counters) {
        let now = [&c.made, &c.shown, &c.dropped, &c.latency_us].map(|a| a.load(Relaxed));
        let d: Vec<u64> = now.iter().zip(self.prev).map(|(a, b)| a - b).collect();
        let (cpu, wall) = (clock(libc::CLOCK_THREAD_CPUTIME_ID), self.at.elapsed().as_secs_f64());
        println!("{:.1} made/s, {:.1} shown/s, {} dropped, UI thread {:.2} ms CPU per shown frame, ready to presented {:.1} ms avg",
            d[0] as f64 / wall, d[1] as f64 / wall, d[2],
            if d[1] > 0 { (cpu - self.cpu).as_secs_f64() * 1e3 / d[1] as f64 } else { 0.0 },
            if d[1] > 0 { d[3] as f64 / d[1] as f64 / 1e3 } else { 0.0 });
        (self.prev, self.cpu, self.at) = (now, cpu, Instant::now());
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();
    let direct = args.get(1).is_some_and(|m| m == "direct");
    let hz: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60.0988);
    let seconds: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(20);
    let capture = args.iter().any(|a| a == "--capture");
    if direct { run_direct(hz, seconds) } else { run_composite(hz, seconds, capture) }
}

/// Direct: our own loop. Each new frame is drawn with one textured quad and presented.
fn run_direct(hz: f64, seconds: u64) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let counters = Arc::new(Counters::default());
    let egl = EglDisplay::open()?;
    let gl = unsafe { glow::Context::from_loader_function_cstr(|n| egl.proc_address(n)) };
    let ctx = egl.shared_context()?;
    let (free_tx, free_rx) = mpsc::channel();
    let (frame_tx, frame_rx) = mpsc::channel::<Frame>();
    let (c, recycle) = (counters.clone(), free_tx.clone());
    std::thread::spawn(move || emulator(ctx, hz, free_rx, recycle, move |f| frame_tx.send(f).is_ok(), c));

    let program = unsafe { blit_program(&gl)? };
    let mut shown = Shown { on_screen: None, next: None, free: free_tx };
    let mut reporter = Reporter::new();
    let (end, mut next_report) = (Instant::now() + Duration::from_secs(seconds), Instant::now() + Duration::from_secs(5));
    while Instant::now() < end {
        // Wait for a frame; keep only the newest.
        let Ok(f) = frame_rx.recv_timeout(Duration::from_millis(100)) else { continue };
        shown.arrive(&gl, f, &counters);
        while let Ok(f) = frame_rx.try_recv() { shown.arrive(&gl, f, &counters); }
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
        shown.presented(&gl, &counters);
        if Instant::now() >= next_report { reporter.report(&counters); next_report += Duration::from_secs(5); }
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

thread_local! {
    /// Runs on the UI thread for each frame the emulator delivers (composite mode).
    static ON_FRAME: RefCell<Option<Box<dyn FnMut(Frame)>>> = RefCell::default();
}

/// Composite: the frame is an image in the Slint scene; every new frame redraws the scene.
fn run_composite(hz: f64, seconds: u64, capture: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let counters = Arc::new(Counters::default());
    let platform = BrickPlatform::new()?;
    let egl = platform.egl();
    let gl = Rc::new(unsafe { glow::Context::from_loader_function_cstr(|n| egl.proc_address(n)) });
    let ctx = egl.shared_context()?;
    let (free_tx, free_rx) = mpsc::channel();
    let shown = Rc::new(RefCell::new(Shown { on_screen: None, next: None, free: free_tx.clone() }));
    let (s, g, c) = (shown.clone(), gl.clone(), counters.clone());
    platform.on_present(move |_| s.borrow_mut().presented(&g, &c));
    slint::platform::set_platform(Box::new(platform)).map_err(|e| format!("{e:?}"))?;
    let scene = Scene::new()?;
    scene.show()?;

    let (weak, g, c) = (scene.as_weak(), gl.clone(), counters.clone());
    ON_FRAME.with(|f| *f.borrow_mut() = Some(Box::new(move |frame: Frame| {
        let image = unsafe {
            slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(frame.texture.0, (W as u32, H as u32).into()).build()
        };
        shown.borrow_mut().arrive(&g, frame, &c);
        if let Some(s) = weak.upgrade() { s.set_frame(image); }
    })));
    let c = counters.clone();
    std::thread::spawn(move || emulator(ctx, hz, free_rx, free_tx, |f| {
        slint::invoke_from_event_loop(move || ON_FRAME.with(|h| if let Some(h) = h.borrow_mut().as_mut() { h(f) })).is_ok()
    }, c));

    // --capture: read back what was drawn, just before the swap.
    if capture {
        let mut n = 0;
        scene.window().set_rendering_notifier(move |state, _| {
            if !matches!(state, slint::RenderingState::AfterRendering) { return; }
            n += 1;
            if !(300..303).contains(&n) { return; }
            let mut px = vec![0u8; (SCREEN.width * SCREEN.height * 4) as usize];
            unsafe { gl.read_pixels(0, 0, SCREEN.width as i32, SCREEN.height as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px))); }
            let _ = std::fs::write(format!("/tmp/capture-{n}.rgba"), px);
        }).map_err(|e| format!("{e:?}"))?;
    }

    // The menu slides in and out every 3 s, so composite mode is measured with UI motion too.
    let toggle = slint::Timer::default();
    let weak = scene.as_weak();
    toggle.start(slint::TimerMode::Repeated, Duration::from_secs(3), move || { if let Some(s) = weak.upgrade() { s.set_menu(!s.get_menu()); } });

    let mut reporter = Reporter::new();
    let report = slint::Timer::default();
    report.start(slint::TimerMode::Repeated, Duration::from_secs(5), move || reporter.report(&counters));
    let stop = slint::Timer::default();
    stop.start(slint::TimerMode::SingleShot, Duration::from_secs(seconds), || { let _ = slint::quit_event_loop(); });
    scene.run()?;
    Ok(())
}
