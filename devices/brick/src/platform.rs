//! A Slint platform for the Brick: one full-screen window drawn with FemtoVG on our EGL display,
//! and our own event loop. The loop draws only when something changed or an animation is
//! running. Otherwise it sleeps in poll() until the next Slint timer, a wake-up from another
//! thread, or a button (or, while an input device is missing, until the next try to open it).

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::VecDeque;
use std::ffi::{CStr, c_void};
use std::fs::File;
use std::io::{Read, Write};
use std::num::NonZeroU32;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::platform::femtovg_renderer::{FemtoVGRenderer, OpenGLInterface};
use slint::platform::{Platform, PlatformError, WindowAdapter, WindowEvent};
use slint::{PhysicalSize, Window};

use crate::egl::EglDisplay;
use specular_linux::input::{ButtonEvent, Input, clock};

type Error = Box<dyn std::error::Error + Send + Sync>;

/// The panel's size in pixels.
pub const SCREEN: PhysicalSize = PhysicalSize::new(1024, 768);

/// The input devices read: padbridge's virtual pad, and the power key (`axp2202-pek`: the power
/// key input of the AXP2202 power-management chip).
const INPUT_DEVICES: &[&str] = &["TRIMUI Player1", "axp2202-pek"];

struct Gl(Rc<EglDisplay>);

unsafe impl OpenGLInterface for Gl {
    fn ensure_current(&self) -> Result<(), Error> { self.0.make_current() }
    fn swap_buffers(&self) -> Result<(), Error> { self.0.swap_buffers() }
    fn resize(&self, _: NonZeroU32, _: NonZeroU32) -> Result<(), Error> { Ok(()) }
    fn get_proc_address(&self, name: &CStr) -> *const c_void { self.0.proc_address(name) }
}

/// The one window, covering the panel. (Slint draws popups inside it.)
struct BrickWindow { window: Window, renderer: FemtoVGRenderer, needs_redraw: Cell<bool> }

impl WindowAdapter for BrickWindow {
    fn window(&self) -> &Window { &self.window }
    fn size(&self) -> PhysicalSize { SCREEN }
    fn renderer(&self) -> &dyn slint::platform::Renderer { &self.renderer }
    fn request_redraw(&self) { self.needs_redraw.set(true) }
}

type Job = Box<dyn FnOnce() + Send>;

/// Work queued from other threads, and the eventfd that wakes the loop for it.
struct Wake { jobs: Mutex<VecDeque<Job>>, quit: AtomicBool, fd: File }

impl Wake {
    fn wake(&self) { let _ = (&self.fd).write(&1u64.to_ne_bytes()); }
}

/// The Slint platform for the Brick. Create it on the thread that will run the event loop (the
/// UI context is made current there), before any other thread starts (see `fonts`).
pub struct BrickPlatform {
    egl: Rc<EglDisplay>,
    window: OnceCell<Rc<BrickWindow>>,
    wake: Arc<Wake>,
    input: RefCell<Input>,
    on_button: RefCell<Option<Box<dyn FnMut(ButtonEvent)>>>,
    on_present: RefCell<Option<Box<dyn FnMut(Duration)>>>,
}

impl BrickPlatform {
    pub fn new() -> Result<Self, Error> {
        crate::fonts::work_around_empty_fontconfig()?;
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if fd < 0 { return Err(std::io::Error::last_os_error().into()); }
        Ok(Self {
            egl: Rc::new(EglDisplay::open()?),
            window: OnceCell::new(),
            wake: Arc::new(Wake { jobs: Mutex::default(), quit: AtomicBool::new(false), fd: unsafe { File::from(OwnedFd::from_raw_fd(fd)) } }),
            input: RefCell::new(Input::new(INPUT_DEVICES)),
            on_button: RefCell::default(),
            on_present: RefCell::default(),
        })
    }

    /// The EGL display: for GL function lookup and contexts for other threads.
    pub fn egl(&self) -> Rc<EglDisplay> { self.egl.clone() }

    /// Sets what receives button presses and releases (called on the event loop's thread).
    pub fn on_button(&self, f: impl FnMut(ButtonEvent) + 'static) { *self.on_button.borrow_mut() = Some(Box::new(f)); }

    /// Sets what is called after each frame is presented, with the CLOCK_MONOTONIC time the
    /// present returned (the same timeline as button times). For measuring.
    pub fn on_present(&self, f: impl FnMut(Duration) + 'static) { *self.on_present.borrow_mut() = Some(Box::new(f)); }

    /// Draws if needed. Returns whether an animation is running (so another frame is due).
    fn draw(&self) -> Result<bool, PlatformError> {
        let Some(w) = self.window.get() else { return Ok(false) };
        if w.needs_redraw.replace(false) {
            w.renderer.render()?;
            if let Some(f) = self.on_present.borrow_mut().as_mut() { f(clock(libc::CLOCK_MONOTONIC)); }
        }
        let animating = w.window.has_active_animations();
        if animating { w.needs_redraw.set(true); }
        Ok(animating)
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
            if let Some(f) = self.on_button.borrow_mut().as_mut() { events.into_iter().for_each(f); }
        }
    }
}

impl Platform for BrickPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        if self.window.get().is_some() { return Err("the Brick has one window".into()); }
        let renderer = FemtoVGRenderer::new(Gl(self.egl.clone()))?;
        let w = Rc::new_cyclic(|weak: &std::rc::Weak<BrickWindow>| BrickWindow {
            window: Window::new(weak.clone()), renderer, needs_redraw: Cell::new(true),
        });
        w.window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
        w.window.set_size(SCREEN);
        let _ = self.window.set(w.clone());
        Ok(w)
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        loop {
            slint::platform::update_timers_and_animations();
            let jobs: Vec<Job> = self.wake.jobs.lock().unwrap().drain(..).collect();
            jobs.into_iter().for_each(|job| job());
            if self.wake.quit.swap(false, Relaxed) { return Ok(()); }
            // While animating, the swap in draw() paces the loop at the display's rate.
            let animating = self.draw()?;
            let timer = if animating { Some(Duration::ZERO) } else { slint::platform::duration_until_next_timer_update() };
            let retry = self.input.borrow_mut().retry();
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
