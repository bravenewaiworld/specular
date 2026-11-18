//! EGL with a GLES 3 context for the UI, on whatever window system a device's driver offers: the
//! driver's own default display (the Brick's PowerVR "NULL window system", which draws straight to
//! the panel), or a GBM surface that `kms::Kms` puts on the panel; and on GBM, a second surface on
//! a TV's GBM surface that each frame is copied onto, scaled (`CopyTarget`). The EGL and GLES
//! libraries are loaded at runtime, so a binary has no link-time dependency on a GPU driver.

use std::ffi::{CStr, c_void};
use std::sync::Arc;

use khronos_egl as egl;

use crate::kms::{Rect, fit};

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

/// A second on-screen surface, on another GBM surface of the same format (a TV's:
/// `kms::Kms::tv_native`), that the frames drawn on the main surface are copied onto, scaled to
/// fit (`EglDisplay::copy_to`). Drop it before its GBM surface goes.
pub struct CopyTarget {
    libs: Arc<Libs>,
    display: egl::Display,
    surface: egl::Surface,
    gl: CopyGl,
    picture: Rect,
    /// `glBlitFramebuffer`'s source and destination corners (GL's rows count from the bottom).
    blit: [i32; 8],
}

impl CopyTarget {
    /// Where the main surface's picture goes on this one (`kms::fit`).
    pub fn picture(&self) -> Rect { self.picture }
}

impl Drop for CopyTarget {
    fn drop(&mut self) {
        if let Err(e) = self.libs.egl.destroy_surface(self.display, self.surface) { eprintln!("egl: destroying the second surface: {e}"); }
    }
}

// GLES 3 (gl3.h).
const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;
const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
const GL_LINEAR: u32 = 0x2601;
const GL_SCISSOR_TEST: u32 = 0x0C11;

/// The GL functions `copy_to` uses.
struct CopyGl {
    bind_framebuffer: unsafe extern "C" fn(u32, u32),
    blit_framebuffer: unsafe extern "C" fn(i32, i32, i32, i32, i32, i32, i32, i32, u32, u32),
    clear_color: unsafe extern "C" fn(f32, f32, f32, f32),
    clear: unsafe extern "C" fn(u32),
    disable: unsafe extern "C" fn(u32),
}

impl EglDisplay {
    /// A `CopyTarget` on the GBM surface `gbm_surface` of size `to`, for a main surface of size
    /// `from`. Needs a GLES 3 context (for the blit).
    pub fn create_copy_target(&self, gbm_surface: *mut c_void, from: (u32, u32), to: (u32, u32)) -> Result<CopyTarget, Error> {
        if self.version < 3 { return Err("copying the picture to a second display needs GLES 3".into()); }
        let f = |name: &CStr| {
            let p = self.proc_address(name);
            if p.is_null() { Err(format!("GL function {name:?} not found")) } else { Ok(p) }
        };
        // SAFETY: each is the GLES function of that name, whose C signature matches the field's.
        let gl = unsafe {
            CopyGl {
                bind_framebuffer: std::mem::transmute(f(c"glBindFramebuffer")?),
                blit_framebuffer: std::mem::transmute(f(c"glBlitFramebuffer")?),
                clear_color: std::mem::transmute(f(c"glClearColor")?),
                clear: std::mem::transmute(f(c"glClear")?),
                disable: std::mem::transmute(f(c"glDisable")?),
            }
        };
        let surface = unsafe { self.libs.egl.create_window_surface(self.display, self.config, gbm_surface, None)? };
        let picture = fit(from, to);
        let bottom = (to.1 - picture.y - picture.height) as i32;
        let (x, width, height) = (picture.x as i32, picture.width as i32, picture.height as i32);
        let blit = [0, 0, from.0 as i32, from.1 as i32, x, bottom, x + width, bottom + height];
        Ok(CopyTarget { libs: self.libs.clone(), display: self.display, surface, gl, picture, blit })
    }

    /// Copies the frame just drawn into the main surface's back buffer onto `to`, scaled into its
    /// `picture` with black around it, swaps `to` (ending the frame there, as `swap_buffers` does
    /// on the main surface), and makes the main surface current again. While a `CopyTarget` shows
    /// the picture the main surface is never swapped: each frame is drawn into its back buffer at
    /// its own size, and copied from there.
    pub fn copy_to(&self, to: &CopyTarget) -> Result<(), Error> {
        let egl = &self.libs.egl;
        // The target as the draw surface, the main one as the read surface: framebuffer 0 is then
        // the target's back buffer for drawing and the main surface's for reading.
        egl.make_current(self.display, Some(to.surface), Some(self.surface), Some(self.context))?;
        let (g, [sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1]) = (&to.gl, to.blit);
        unsafe {
            (g.bind_framebuffer)(GL_READ_FRAMEBUFFER, 0);
            (g.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, 0);
            (g.disable)(GL_SCISSOR_TEST);
            (g.clear_color)(0.0, 0.0, 0.0, 1.0);
            (g.clear)(GL_COLOR_BUFFER_BIT);
            (g.blit_framebuffer)(sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1, GL_COLOR_BUFFER_BIT, GL_LINEAR);
        }
        let swapped = egl.swap_buffers(self.display, to.surface);
        self.make_current()?;
        Ok(swapped?)
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
