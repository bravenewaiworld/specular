//! A Slint platform for a device without a window system: one full-screen window drawn with
//! FemtoVG on a GL surface the device supplies (`GlSurface`: a driver's own display, or
//! `KmsSurface`), and our own event loop. The loop draws only when something changed or an
//! animation is running. Given a vsync, an animation draws one frame per refresh, each started
//! once the last is on screen (`FramePacer`). When idle, it sleeps in poll() until the next Slint
//! timer, that vsync, a wake-up from another thread, or a button (or, while an input device is
//! missing, until the next try to open it).

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::VecDeque;
use std::ffi::{CStr, c_void};
use std::fs::File;
use std::io::{Read, Write};
use std::num::NonZeroU32;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::platform::femtovg_renderer::{FemtoVGRenderer, OpenGLInterface};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{PhysicalSize, Window};

use specular_core::device::Vsync;
use specular_linux::egl::{CopyTarget, EglDisplay};
use specular_linux::input::{ButtonEvent, Input, clock};
use specular_linux::kms::{Kms, ModeRequest, Output, Rect};
use specular_linux::vsync::FramePacer;

type Error = Box<dyn std::error::Error + Send + Sync>;

/// What the platform draws on: a GL context for the UI thread, and a way to show each frame.
pub trait GlSurface {
    /// Makes the UI's context and the on-screen surface current on the calling thread.
    fn make_current(&self) -> Result<(), Error>;
    /// Shows the frame just drawn. May block until a refresh.
    fn present(&self) -> Result<(), Error>;
    /// The address of a GL function, or null if there is none.
    fn proc_address(&self, name: &CStr) -> *const c_void;
    /// Whether a frame should be drawn and presented even though nothing in the UI changed (a
    /// newly connected output has nothing on it until one is).
    fn wants_frame(&self) -> bool { false }
}

/// For an `EglDisplay` on `Native::Default`, whose swap shows the frame (the Brick's PowerVR NULL
/// window system). On GBM, use `KmsSurface`.
impl GlSurface for EglDisplay {
    fn make_current(&self) -> Result<(), Error> { EglDisplay::make_current(self) }
    fn present(&self) -> Result<(), Error> { self.swap_buffers() }
    fn proc_address(&self, name: &CStr) -> *const c_void { EglDisplay::proc_address(self, name) }
}

/// A DRM/KMS display: EGL draws into GBM buffers, and each present flips one onto the screen at
/// the next vblank. A TV plugged in (HDMI; also one in at start-up) takes the picture and the
/// panel goes off, until it is unplugged: the UI is still drawn at the panel's size, and each
/// frame copied onto the TV scaled to fit (`kms::fit`). The connectors are read again at the
/// present after each hotplug event. With `SPECULAR_FLIP_STATS` set (to anything), logs how the
/// flips went every 10 s while frames are presented (`kms::FlipStats`), as a `kms:` line.
pub struct KmsSurface {
    /// On the TV's GBM surface while the picture is on one; given back in `drop`, before `kms`
    /// goes.
    tv: RefCell<Option<CopyTarget>>,
    /// Declared before `kms`, so it is dropped first: its EGL surface lives on the GBM surface.
    egl: EglDisplay,
    kms: RefCell<Kms>,
    /// A mode other threads ask for, set at the next present.
    request: ModeRequest,
    /// When the last flip statistics were logged, if they are.
    stats: Option<Cell<Duration>>,
    /// Set by the hotplug thread (and at start-up): read the connectors at the next present.
    hotplug: Arc<AtomicBool>,
    /// When the picture started moving to another output, until its first frame there.
    moving: Cell<Option<Duration>>,
    on_switch: RefCell<OnSwitch>,
}

/// Told of each move of the picture once the new output shows its first frame: the output, where
/// the UI's picture is on it (on a TV), and how long the move took.
pub type OnSwitch = Box<dyn FnMut(&Output, Option<Rect>, Duration)>;

impl KmsSurface {
    /// Opens EGL on `kms`'s GBM surface, and follows the hotplug events of `card` (the card `kms`
    /// was opened on) on a thread of its own. A mode put in `request` is switched to at the next
    /// present.
    pub fn new(kms: Kms, card: &Path, request: ModeRequest, on_switch: OnSwitch) -> Result<Self, Error> {
        let stats = std::env::var_os("SPECULAR_FLIP_STATS").map(|_| Cell::new(clock(libc::CLOCK_MONOTONIC)));
        let hotplug = Arc::new(AtomicBool::new(true));
        let (flag, name) = (hotplug.clone(), card.file_name().unwrap_or_default().to_string_lossy().into_owned());
        std::thread::Builder::new().name("hotplug".into()).spawn(move || {
            // Waking the event loop makes it draw (`wants_frame`), which presents.
            let woken = specular_linux::hotplug::watch(&name, || { flag.store(true, Relaxed); let _ = slint::invoke_from_event_loop(|| {}); });
            if let Err(e) = woken { eprintln!("hotplug: {e}; a TV plugged in or out won't be noticed"); }
        })?;
        Ok(Self { tv: RefCell::default(), egl: EglDisplay::open(kms.native())?, kms: RefCell::new(kms), request, stats, hotplug,
            moving: Cell::default(), on_switch: RefCell::new(on_switch) })
    }

    /// Reads the connectors, and moves the picture if it should be elsewhere now.
    fn follow_outputs(&self, kms: &mut Kms) {
        match kms.find_tv() {
            Ok(tv) if tv.as_ref() != kms.tv() => self.move_to(kms, tv),
            Ok(_) => {}
            Err(e) => eprintln!("display: reading the connectors: {e}"),
        }
    }

    /// Moves the picture to `tv` (None: the panel) at the next present; to the panel if it can't.
    fn move_to(&self, kms: &mut Kms, tv: Option<Output>) {
        if self.moving.get().is_none() { self.moving.set(Some(clock(libc::CLOCK_MONOTONIC))); }
        if let Some(old) = self.tv.take() { self.egl.destroy_copy_target(old); }
        let name = tv.as_ref().map(|o| o.name.clone());
        let target = kms.show_on(tv).map_err(Error::from)
            .and_then(|()| kms.tv_native().map(|s| self.egl.create_copy_target(s)).transpose());
        match target {
            Ok(target) => *self.tv.borrow_mut() = target,
            Err(e) => {
                eprintln!("display: can't show the picture on {}: {e}; it stays on the panel", name.as_deref().unwrap_or("the panel"));
                if kms.tv().is_some() && let Err(e) = kms.show_on(None) { eprintln!("display: back to the panel: {e}"); }
            }
        }
    }

    /// Ends the frame drawn on the output showing the picture: on the panel, swaps the main
    /// surface; on a TV, copies the frame onto the TV's surface.
    fn show(&self, kms: &mut Kms) -> Result<(), Error> {
        match (self.tv.borrow().as_ref(), kms.picture()) {
            (Some(tv), Some(picture)) => self.egl.copy_to(tv, kms.size(), (kms.mode().width(), kms.mode().height()), picture)?,
            _ => self.egl.swap_buffers()?,
        }
        Ok(kms.present()?)
    }
}

impl Drop for KmsSurface {
    fn drop(&mut self) {
        if let Some(tv) = self.tv.take() { self.egl.destroy_copy_target(tv); }
    }
}

/// How often `KmsSurface` logs its flip statistics.
const FLIP_STATS_EVERY: Duration = Duration::from_secs(10);

impl GlSurface for KmsSurface {
    fn make_current(&self) -> Result<(), Error> { self.egl.make_current() }

    fn present(&self) -> Result<(), Error> {
        let mut kms = self.kms.borrow_mut();
        if self.hotplug.swap(false, Relaxed) { self.follow_outputs(&mut kms); }
        // A mode asked for just before the picture moved may be the other output's.
        if let Some(mode) = self.request.lock().unwrap().take() && let Err(e) = kms.set_mode(mode) { eprintln!("{e}"); }
        if let Err(e) = self.show(&mut kms) {
            let Some(tv) = kms.tv().map(|o| o.name.clone()) else { return Err(e) };
            eprintln!("display: showing the picture on {tv} failed ({e}); back to the panel");
            self.move_to(&mut kms, None);
            self.show(&mut kms)?;
        }
        if let Some(since) = self.moving.take() {
            (self.on_switch.borrow_mut())(kms.output(), kms.picture(), clock(libc::CLOCK_MONOTONIC).saturating_sub(since));
        }
        if let Some(since) = &self.stats {
            let now = clock(libc::CLOCK_MONOTONIC);
            if now - since.get() >= FLIP_STATS_EVERY {
                let s = kms.take_flip_stats();
                eprintln!("kms: {} flips in {:.1} s, {} late, longest gap {:.1} ms", s.flips, (now - since.get()).as_secs_f64(), s.late, s.longest.as_secs_f64() * 1000.0);
                since.set(now);
            }
        }
        Ok(())
    }

    fn proc_address(&self, name: &CStr) -> *const c_void { self.egl.proc_address(name) }

    fn wants_frame(&self) -> bool { self.hotplug.load(Relaxed) }
}

struct Gl<S>(Rc<S>);

// SAFETY: the surface's context is made current before Slint draws (`ensure_current`), and
// `get_proc_address` returns the driver's functions for it.
unsafe impl<S: GlSurface> OpenGLInterface for Gl<S> {
    fn ensure_current(&self) -> Result<(), Error> { self.0.make_current() }
    fn swap_buffers(&self) -> Result<(), Error> { self.0.present() }
    fn resize(&self, _: NonZeroU32, _: NonZeroU32) -> Result<(), Error> { Ok(()) }
    fn get_proc_address(&self, name: &CStr) -> *const c_void { self.0.proc_address(name) }
}

/// The one window, covering the panel. (Slint draws popups inside it.)
struct ScreenWindow { window: Window, renderer: FemtoVGRenderer, size: PhysicalSize, needs_redraw: Cell<bool> }

impl WindowAdapter for ScreenWindow {
    fn window(&self) -> &Window { &self.window }
    fn size(&self) -> PhysicalSize { self.size }
    fn renderer(&self) -> &dyn slint::platform::Renderer { &self.renderer }
    fn request_redraw(&self) { self.needs_redraw.set(true) }
}

type Job = Box<dyn FnOnce() + Send>;

/// Work queued from other threads, and the eventfd that wakes the loop for it.
struct Wake { jobs: Mutex<VecDeque<Job>>, quit: AtomicBool, fd: File }

impl Wake {
    fn wake(&self) { let _ = (&self.fd).write(&1u64.to_ne_bytes()); }
}

/// The Slint platform. Create it on the thread that will run the event loop, where the surface's
/// context is current.
pub struct GlPlatform<S: GlSurface> {
    surface: Rc<S>,
    size: PhysicalSize,
    window: OnceCell<Rc<ScreenWindow>>,
    wake: Arc<Wake>,
    pacer: FramePacer,
    input: RefCell<Input>,
    on_button: RefCell<Option<Box<dyn FnMut(ButtonEvent)>>>,
    on_present: RefCell<Option<Box<dyn FnMut(Duration)>>>,
}

impl<S: GlSurface + 'static> GlPlatform<S> {
    /// A window of `size` pixels on `surface`, with buttons from `input`. Animations are paced by `vsync`; without it, by `present`
    /// alone.
    pub fn new(surface: S, size: PhysicalSize, input: Input, vsync: Option<Box<dyn Vsync>>) -> Result<Self, Error> {
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 { return Err(std::io::Error::last_os_error().into()); }
        let wake = Arc::new(Wake { jobs: Mutex::default(), quit: AtomicBool::new(false), fd: unsafe { File::from(OwnedFd::from_raw_fd(fd)) } });
        let woken = wake.clone();
        Ok(Self {
            surface: Rc::new(surface),
            size,
            window: OnceCell::new(),
            pacer: FramePacer::new(vsync, move || woken.wake()),
            wake,
            input: RefCell::new(input),
            on_button: RefCell::default(),
            on_present: RefCell::default(),
        })
    }

    /// The surface, e.g. for `EglDisplay::shared_context`.
    pub fn surface(&self) -> Rc<S> { self.surface.clone() }

    /// Sets what receives button presses and releases (called on the event loop's thread).
    pub fn on_button(&self, f: impl FnMut(ButtonEvent) + 'static) { *self.on_button.borrow_mut() = Some(Box::new(f)); }

    /// Sets what is called after each frame is presented, with the CLOCK_MONOTONIC time the
    /// present returned (the same timeline as button times). For measuring.
    pub fn on_present(&self, f: impl FnMut(Duration) + 'static) { *self.on_present.borrow_mut() = Some(Box::new(f)); }

    /// Draws if needed and the pacer allows. Returns whether another frame is due and may be
    /// drawn at once (only when the present alone paces drawing).
    ///
    /// Only an animation frame arms the pacer, so only the frame after it waits for the vsync (and
    /// picks up anything else that changed meanwhile, such as a game's new frame). Other redraws
    /// go at once: a game's thread paces its own frames, and holding a one-off redraw to the next
    /// vsync would delay every following game frame by a refresh.
    fn draw(&self) -> Result<bool, PlatformError> {
        let Some(w) = self.window.get() else { return Ok(false) };
        if self.surface.wants_frame() { w.needs_redraw.set(true); }
        if !w.needs_redraw.get() || !self.pacer.ready() { return Ok(false); }
        w.needs_redraw.set(false);
        w.renderer.render()?;
        if let Some(f) = self.on_present.borrow_mut().as_mut() { f(clock(libc::CLOCK_MONOTONIC)); }
        // Slint knows an animation runs only while drawing it: once its clock moves on, it
        // reports none until the next render. So ask now, and keep `needs_redraw` set until the
        // pacer lets the next frame be drawn.
        if !w.window.has_active_animations() { return Ok(false); }
        w.needs_redraw.set(true);
        self.pacer.presented();
        Ok(self.pacer.ready())
    }

    /// Sleeps until there is something to do (at most `timeout`), then reads wake-ups and buttons.
    fn wait(&self, timeout: Option<Duration>) {
        // Rounded up: rounding down would wake just before a timer is due and spin until it is.
        let ms = timeout.map_or(-1, |d| d.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32);
        let mut fds: Vec<libc::pollfd> = std::iter::once(self.wake.fd.as_raw_fd()).chain(self.input.borrow().fds())
            .map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }).collect();
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, ms) };
        if fds[0].revents != 0 { let _ = (&self.wake.fd).read(&mut [0u8; 8]); }
        if fds[1..].iter().any(|f| f.revents != 0) {
            let mut events = Vec::new();
            self.input.borrow_mut().read(&mut events);
            self.send_buttons(events);
        }
    }

    /// Passes button events to the `on_button` handler, in order (dropped if none is set).
    fn send_buttons(&self, events: Vec<ButtonEvent>) {
        if let Some(f) = self.on_button.borrow_mut().as_mut() { events.into_iter().for_each(f); }
    }
}

impl<S: GlSurface + 'static> Platform for GlPlatform<S> {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        if self.window.get().is_some() { return Err("the screen has one window".into()); }
        let renderer = FemtoVGRenderer::new(Gl(self.surface.clone()))?;
        let w = Rc::new_cyclic(|weak: &std::rc::Weak<ScreenWindow>| ScreenWindow {
            window: Window::new(weak.clone()), renderer, size: self.size, needs_redraw: Cell::new(true),
        });
        w.window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
        w.window.set_size(self.size);
        let _ = self.window.set(w.clone());
        Ok(w)
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        loop {
            slint::platform::update_timers_and_animations();
            let jobs: Vec<Job> = self.wake.jobs.lock().unwrap().drain(..).collect();
            jobs.into_iter().for_each(|job| job());
            if self.wake.quit.swap(false, Relaxed) { return Ok(()); }
            let again = self.draw()?;
            let timer = if again { Some(Duration::ZERO) } else { slint::platform::duration_until_next_timer_update() };
            let mut events = Vec::new(); // `retry` may open a device and report its state, e.g. a mute switch already on
            let retry = self.input.borrow_mut().retry(&mut events);
            self.send_buttons(events);
            self.wait([timer, retry].into_iter().flatten().min());
        }
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn slint::platform::EventLoopProxy>> {
        Some(Box::new(Proxy(self.wake.clone())))
    }
}

struct Proxy(Arc<Wake>);

impl slint::platform::EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        self.0.quit.store(true, Relaxed);
        self.0.wake();
        Ok(())
    }
    fn invoke_from_event_loop(&self, job: Job) -> Result<(), slint::EventLoopError> {
        self.0.jobs.lock().unwrap().push_back(job);
        self.0.wake();
        Ok(())
    }
}
