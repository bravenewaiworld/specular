//! EGL on the Brick's PowerVR GPU. TrimUI's closed driver has only PowerVR's "NULL window system":
//! there is no windowing at all, and a window surface created without a native window renders
//! straight to the panel (/dev/fb0, whose buffers the display driver swaps on vsync). So there is
//! no DRM, Wayland or X11. All PowerVR specifics live here.

use std::ffi::{CStr, c_void};
use std::sync::Arc;

use khronos_egl as egl;

type Egl = egl::DynamicInstance<egl::EGL1_4>;
type Error = Box<dyn std::error::Error + Send + Sync>;

/// From EGL_KHR_create_context (not in EGL 1.4 itself): a config that supports GLES 3.
const EGL_OPENGL_ES3_BIT: egl::Int = 0x40;

/// The EGL and GLES libraries, loaded once and shared by every context.
struct Libs { egl: Egl, gles: libloading::Library }

impl Libs {
    /// The address of a GL function: from libGLESv2's exported symbols first, then from
    /// eglGetProcAddress (extensions). Before EGL 1.5, eglGetProcAddress may return NULL for core
    /// functions; on this driver it was not seen to (spike, 2026-10-12), but the order is free.
    fn proc_address(&self, name: &CStr) -> *const c_void {
        if let Ok(sym) = unsafe { self.gles.get::<*const c_void>(name.to_bytes_with_nul()) } {
            if !sym.is_null() { return *sym; }
        }
        match self.egl.get_proc_address(name.to_str().unwrap_or_default()) {
            Some(f) => f as *const c_void,
            None => { eprintln!("egl: GL function {name:?} not found"); std::ptr::null() }
        }
    }
}

/// The display, the on-screen surface and the UI's GL context.
pub struct EglDisplay {
    libs: Arc<Libs>,
    display: egl::Display,
    config: egl::Config,
    version: egl::Int,
    surface: egl::Surface,
    context: egl::Context,
}

impl EglDisplay {
    /// Opens the display with a GLES 3 context (GLES 2 if 3 is unavailable), current on the
    /// calling thread, with swaps locked to vsync.
    pub fn open() -> Result<Self, Error> {
        let egl = unsafe { Egl::load_required_from_filename("libEGL.so.1")? };
        let gles = unsafe { libloading::Library::new("libGLESv2.so.2")? };
        let display = unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }.ok_or("no EGL display")?;
        egl.initialize(display)?;
        let (config, version) = [(EGL_OPENGL_ES3_BIT, 3), (egl::OPENGL_ES2_BIT, 2)]
            .into_iter()
            .find_map(|(bit, version)| {
                let attrs = [egl::RED_SIZE, 8, egl::GREEN_SIZE, 8, egl::BLUE_SIZE, 8, egl::ALPHA_SIZE, 8,
                    egl::STENCIL_SIZE, 8, egl::RENDERABLE_TYPE, bit, egl::NONE];
                egl.choose_first_config(display, &attrs).ok().flatten().map(|c| (c, version))
            })
            .ok_or("no suitable EGL config")?;
        // No native window: the NULL window system draws to the panel.
        let surface = unsafe { egl.create_window_surface(display, config, std::ptr::null_mut(), None)? };
        let context = egl.create_context(display, config, None, &[egl::CONTEXT_CLIENT_VERSION, version, egl::NONE])?;
        egl.make_current(display, Some(surface), Some(surface), Some(context))?;
        egl.swap_interval(display, 1)?;
        eprintln!("egl: {} {}, GLES {version} context",
            egl.query_string(Some(display), egl::VENDOR)?.to_string_lossy(),
            egl.query_string(Some(display), egl::VERSION)?.to_string_lossy());
        Ok(Self { libs: Arc::new(Libs { egl, gles }), display, config, version, surface, context })
    }

    /// The address of a GL function, or null if there is none.
    pub fn proc_address(&self, name: &CStr) -> *const c_void { self.libs.proc_address(name) }

    /// A context sharing textures and other objects with the UI's, for another thread (the
    /// emulator's). It has no surface: it draws only into textures. It must not be used after
    /// this display is dropped.
    pub fn shared_context(&self) -> Result<SharedContext, Error> {
        let context = self.libs.egl.create_context(self.display, self.config, Some(self.context),
            &[egl::CONTEXT_CLIENT_VERSION, self.version, egl::NONE])?;
        Ok(SharedContext { libs: self.libs.clone(), display: self.display, context })
    }

    /// Makes the UI context and the on-screen surface current on the calling thread.
    pub fn make_current(&self) -> Result<(), Error> {
        Ok(self.libs.egl.make_current(self.display, Some(self.surface), Some(self.surface), Some(self.context))?)
    }

    /// Presents the frame; blocks until the next vsync.
    pub fn swap_buffers(&self) -> Result<(), Error> {
        Ok(self.libs.egl.swap_buffers(self.display, self.surface)?)
    }
}

/// A GL context for a thread other than the UI's; see `EglDisplay::shared_context`.
pub struct SharedContext { libs: Arc<Libs>, display: egl::Display, context: egl::Context }

// SAFETY: EGL display and context handles are valid process-wide; a context is current on at most
// one thread at a time, which `make_current` on the receiving thread establishes.
unsafe impl Send for SharedContext {}

impl SharedContext {
    /// Makes the context current on the calling thread, with no surface.
    pub fn make_current(&self) -> Result<(), Error> {
        Ok(self.libs.egl.make_current(self.display, None, None, Some(self.context))?)
    }

    /// The address of a GL function, or null if there is none.
    pub fn proc_address(&self, name: &CStr) -> *const c_void { self.libs.proc_address(name) }
}
