//! EGL with a GLES 3 context for the UI, on whatever window system a device's driver offers: the
//! driver's own default display (the Brick's PowerVR "NULL window system", which draws straight to
//! the panel), or a GBM surface that `kms::Kms` puts on the panel. The EGL and GLES libraries are
//! loaded at runtime, so a binary has no link-time dependency on a GPU driver.

use std::ffi::{CStr, c_void};
use std::sync::Arc;

use khronos_egl as egl;

type Egl = egl::DynamicInstance<egl::EGL1_4>;
type Error = Box<dyn std::error::Error + Send + Sync>;

/// From EGL_KHR_create_context (not in EGL 1.4 itself): a config that supports GLES 3.
const EGL_OPENGL_ES3_BIT: egl::Int = 0x40;
/// From EGL_KHR_platform_gbm.
const EGL_PLATFORM_GBM_KHR: egl::Enum = 0x31D7;

/// What the on-screen surface is made on.
pub enum Native {
    /// The driver's default display, with a window surface made without a native window (the
    /// Brick's PowerVR NULL window system draws that to the panel).
    Default,
    /// A GBM device and surface (`kms::Kms::native`), whose buffers have this DRM format. The
    /// `Kms` must outlive the display.
    Gbm { device: *mut c_void, surface: *mut c_void, format: u32 },
}

/// The EGL and GLES libraries, loaded once and shared by every context.
struct Libs { egl: Egl, gles: libloading::Library }

impl Libs {
    /// The address of a GL function: from libGLESv2's exported symbols first, then from
    /// eglGetProcAddress (extensions). Before EGL 1.5, eglGetProcAddress may return NULL for core
    /// functions.
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
    /// calling thread, with swaps locked to vsync (on GBM, `Kms::present` paces instead).
    pub fn open(native: Native) -> Result<Self, Error> {
        let egl = unsafe { Egl::load_required_from_filename("libEGL.so.1")? };
        let gles = unsafe { libloading::Library::new("libGLESv2.so.2")? };
        let (display, window, format) = match native {
            Native::Default => (unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }.ok_or("no EGL display")?, std::ptr::null_mut(), None),
            Native::Gbm { device, surface, format } => {
                let egl15 = egl.upcast::<egl::EGL1_5>().ok_or("EGL 1.5 needed for a GBM display")?;
                (unsafe { egl15.get_platform_display(EGL_PLATFORM_GBM_KHR, device, &[egl::ATTRIB_NONE]) }?, surface, Some(format))
            }
        };
        egl.initialize(display)?;
        let (config, version) = [(EGL_OPENGL_ES3_BIT, 3), (egl::OPENGL_ES2_BIT, 2)]
            .into_iter()
            .find_map(|(bit, version)| choose_config(&egl, display, bit, format).map(|c| (c, version)))
            .ok_or("no suitable EGL config")?;
        let surface = unsafe { egl.create_window_surface(display, config, window, None)? };
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

    /// How many vsyncs a swap waits for (1 by default, as `open` sets).
    pub fn set_swap_interval(&self, interval: egl::Int) -> Result<(), Error> {
        Ok(self.libs.egl.swap_interval(self.display, interval)?)
    }

    /// Ends the frame. On the default display this presents it (blocking until a vsync); on a GBM
    /// surface it only makes it the front buffer, which `kms::Kms::present` then shows.
    pub fn swap_buffers(&self) -> Result<(), Error> {
        Ok(self.libs.egl.swap_buffers(self.display, self.surface)?)
    }
}

/// An RGBA8888 config with a stencil buffer (FemtoVG needs one) for GLES `bit`. On GBM, its native
/// visual must be the surface's format, and alpha is whatever that format has.
fn choose_config(egl: &Egl, display: egl::Display, bit: egl::Int, format: Option<u32>) -> Option<egl::Config> {
    let alpha = if format.is_some() { 0 } else { 8 };
    let attrs = [egl::RED_SIZE, 8, egl::GREEN_SIZE, 8, egl::BLUE_SIZE, 8, egl::ALPHA_SIZE, alpha,
        egl::STENCIL_SIZE, 8, egl::RENDERABLE_TYPE, bit, egl::NONE];
    let mut configs = Vec::with_capacity(egl.matching_config_count(display, &attrs).ok()?);
    egl.choose_config(display, &attrs, &mut configs).ok()?;
    configs.into_iter().find(|&c| format.is_none_or(|f| egl.get_config_attrib(display, c, egl::NATIVE_VISUAL_ID).ok() == Some(f as egl::Int)))
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
