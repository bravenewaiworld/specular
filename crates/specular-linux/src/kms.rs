//! The panel through DRM/KMS: the connected display's preferred mode, a GBM surface for EGL to
//! draw into (`egl::Native::Gbm`), and each finished frame put on screen by a page flip at the next
//! vblank. Also a vsync for games and animations from the card's vblank counter. libdrm and libgbm
//! are loaded at runtime, as the GPU libraries are.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::Duration;

use specular_core::device::Vsync;

/// `struct drm_mode_modeinfo` (libdrm's `drmModeModeInfo`).
#[repr(C)]
#[derive(Clone, Copy)]
struct ModeInfo {
    clock: u32, // kHz
    hdisplay: u16, hsync_start: u16, hsync_end: u16, htotal: u16, hskew: u16,
    vdisplay: u16, vsync_start: u16, vsync_end: u16, vtotal: u16, vscan: u16,
    vrefresh: u32, flags: u32, kind: u32, name: [c_char; 32],
}

#[repr(C)]
struct Resources {
    count_fbs: c_int, fbs: *const u32,
    count_crtcs: c_int, crtcs: *const u32,
    count_connectors: c_int, connectors: *const u32,
    count_encoders: c_int, encoders: *const u32,
    min_width: u32, max_width: u32, min_height: u32, max_height: u32,
}

#[repr(C)]
struct Connector {
    connector_id: u32, encoder_id: u32, connector_type: u32, connector_type_id: u32,
    connection: u32, mm_width: u32, mm_height: u32, subpixel: u32,
    count_modes: c_int, modes: *const ModeInfo,
    count_props: c_int, props: *const u32, prop_values: *const u64,
    count_encoders: c_int, encoders: *const u32,
}

#[repr(C)]
struct Encoder { encoder_id: u32, encoder_type: u32, crtc_id: u32, possible_crtcs: u32, possible_clones: u32 }

#[repr(C)]
struct Version {
    major: c_int, minor: c_int, patch: c_int,
    name_len: c_int, name: *const c_char, date_len: c_int, date: *const c_char, desc_len: c_int, desc: *const c_char,
}

// drm_mode.h, drm.h
const DRM_MODE_CONNECTED: u32 = 1;
const DRM_MODE_TYPE_PREFERRED: u32 = 1 << 3;
const DRM_MODE_PAGE_FLIP_EVENT: u32 = 1;
const DRM_EVENT_FLIP_COMPLETE: u32 = 2;
/// Connector types of built-in panels (LVDS, eDP, DSI), preferred over external outputs.
const INTERNAL: [u32; 3] = [7, 14, 16];
const CONNECTOR_TYPES: [&str; 21] = ["Unknown", "VGA", "DVI-I", "DVI-D", "DVI-A", "Composite", "SVIDEO", "LVDS", "Component",
    "DIN", "DP", "HDMI-A", "HDMI-B", "TV", "eDP", "Virtual", "DSI", "DPI", "Writeback", "SPI", "USB"];
/// `GBM_FORMAT_XRGB8888` (DRM fourcc "XR24").
pub const FORMAT_XRGB8888: u32 = 0x3432_5258;
const GBM_BO_USE_SCANOUT: u32 = 1;
const GBM_BO_USE_RENDERING: u32 = 4;

/// Loads `name` from `lib` as a function pointer of type `T`.
unsafe fn sym<T: Copy>(lib: &libloading::Library, name: &[u8]) -> io::Result<T> {
    unsafe { lib.get::<T>(name).map(|s| *s) }.map_err(|e| io::Error::other(format!("{}: {e}", String::from_utf8_lossy(&name[..name.len() - 1]))))
}

/// The libdrm and libgbm functions used.
struct Libs {
    _drm: libloading::Library,
    _gbm: libloading::Library,
    get_version: unsafe extern "C" fn(c_int) -> *mut Version,
    free_version: unsafe extern "C" fn(*mut Version),
    get_resources: unsafe extern "C" fn(c_int) -> *mut Resources,
    free_resources: unsafe extern "C" fn(*mut Resources),
    get_connector: unsafe extern "C" fn(c_int, u32) -> *mut Connector,
    free_connector: unsafe extern "C" fn(*mut Connector),
    get_encoder: unsafe extern "C" fn(c_int, u32) -> *mut Encoder,
    free_encoder: unsafe extern "C" fn(*mut Encoder),
    add_fb: unsafe extern "C" fn(c_int, u32, u32, u8, u8, u32, u32, *mut u32) -> c_int,
    rm_fb: unsafe extern "C" fn(c_int, u32) -> c_int,
    set_crtc: unsafe extern "C" fn(c_int, u32, u32, u32, u32, *const u32, c_int, *const ModeInfo) -> c_int,
    page_flip: unsafe extern "C" fn(c_int, u32, u32, u32, *mut c_void) -> c_int,
    gbm_create_device: unsafe extern "C" fn(c_int) -> *mut c_void,
    gbm_device_destroy: unsafe extern "C" fn(*mut c_void),
    gbm_surface_create: unsafe extern "C" fn(*mut c_void, u32, u32, u32, u32) -> *mut c_void,
    gbm_surface_destroy: unsafe extern "C" fn(*mut c_void),
    gbm_surface_lock_front_buffer: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    gbm_surface_release_buffer: unsafe extern "C" fn(*mut c_void, *mut c_void),
    /// Returns `union gbm_bo_handle` (8 bytes); the DRM handle is its `u32`.
    gbm_bo_get_handle: unsafe extern "C" fn(*mut c_void) -> u64,
    gbm_bo_get_stride: unsafe extern "C" fn(*mut c_void) -> u32,
}

impl Libs {
    fn load() -> io::Result<Self> {
        let open = |name: &str| unsafe { libloading::Library::new(name) }.map_err(|e| io::Error::other(format!("{name}: {e}")));
        let (drm, gbm) = (open("libdrm.so.2")?, open("libgbm.so.1")?);
        unsafe {
            Ok(Self {
                get_version: sym(&drm, b"drmGetVersion\0")?,
                free_version: sym(&drm, b"drmFreeVersion\0")?,
                get_resources: sym(&drm, b"drmModeGetResources\0")?,
                free_resources: sym(&drm, b"drmModeFreeResources\0")?,
                get_connector: sym(&drm, b"drmModeGetConnector\0")?,
                free_connector: sym(&drm, b"drmModeFreeConnector\0")?,
                get_encoder: sym(&drm, b"drmModeGetEncoder\0")?,
                free_encoder: sym(&drm, b"drmModeFreeEncoder\0")?,
                add_fb: sym(&drm, b"drmModeAddFB\0")?,
                rm_fb: sym(&drm, b"drmModeRmFB\0")?,
                set_crtc: sym(&drm, b"drmModeSetCrtc\0")?,
                page_flip: sym(&drm, b"drmModePageFlip\0")?,
                gbm_create_device: sym(&gbm, b"gbm_create_device\0")?,
                gbm_device_destroy: sym(&gbm, b"gbm_device_destroy\0")?,
                gbm_surface_create: sym(&gbm, b"gbm_surface_create\0")?,
                gbm_surface_destroy: sym(&gbm, b"gbm_surface_destroy\0")?,
                gbm_surface_lock_front_buffer: sym(&gbm, b"gbm_surface_lock_front_buffer\0")?,
                gbm_surface_release_buffer: sym(&gbm, b"gbm_surface_release_buffer\0")?,
                gbm_bo_get_handle: sym(&gbm, b"gbm_bo_get_handle\0")?,
                gbm_bo_get_stride: sym(&gbm, b"gbm_bo_get_stride\0")?,
                _drm: drm,
                _gbm: gbm,
            })
        }
    }
}

/// A display mode, as the connector offers it.
#[derive(Clone, Copy)]
pub struct Mode { info: ModeInfo }

impl Mode {
    pub fn width(&self) -> u32 { self.info.hdisplay as u32 }
    pub fn height(&self) -> u32 { self.info.vdisplay as u32 }
    /// The refresh rate its timings give (the pixel clock over the total pixels per frame). The
    /// panel's real rate can differ, when the clock it gets isn't the one asked for: measure it
    /// with `DrmVsync::measure`.
    pub fn refresh_hz(&self) -> f64 {
        let total = self.info.htotal as f64 * self.info.vtotal as f64;
        if total == 0.0 { 0.0 } else { self.info.clock as f64 * 1000.0 / total }
    }
    fn preferred(&self) -> bool { self.info.kind & DRM_MODE_TYPE_PREFERRED != 0 }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}x{} {:.2} Hz ({} kHz){}", self.width(), self.height(), self.refresh_hz(), self.info.clock,
            if self.preferred() { " preferred" } else { "" })
    }
}

/// How the page flips have gone since the last `Kms::take_flip_stats`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlipStats {
    pub flips: u32,
    /// Flips that came more than 1.5 refreshes after the one before.
    pub late: u32,
    /// The longest time between two flips, idle gaps included (which `late` leaves out).
    pub longest: Duration,
}

/// The panel, owned while open (this process is the card's DRM master). Frames are drawn with
/// EGL into the GBM surface (`native`), then `present` puts each on screen.
pub struct Kms {
    libs: Libs,
    card: File,
    connector: u32,
    crtc: u32,
    /// The CRTC's position in the card's list, which vblank waits name it by.
    crtc_index: u32,
    mode: Mode,
    /// Every mode the output offers, as it lists them.
    modes: Vec<Mode>,
    /// Whether the next present sets `mode` (a full mode set) rather than flipping.
    mode_set: bool,
    device: *mut c_void,
    surface: *mut c_void,
    /// Each GBM buffer presented so far, with its DRM framebuffer.
    framebuffers: Vec<(*mut c_void, u32)>,
    /// The buffer on screen.
    shown: Option<*mut c_void>,
    /// The buffer flipped to, until the flip completes.
    flipping: Option<*mut c_void>,
    last_flip: Option<Duration>,
    stats: FlipStats,
}

impl Kms {
    /// Opens `card` (e.g. /dev/dri/card0): its first connected built-in panel (else its first
    /// connected output), that output's preferred mode and a CRTC that can drive it, and a GBM
    /// surface of that size. Logs what it found and chose, as `kms:` lines. Nothing changes on
    /// screen until the first `present`.
    pub fn open(card: &Path) -> io::Result<Kms> {
        let libs = Libs::load()?;
        let file = File::options().read(true).write(true).open(card).map_err(|e| context(e, card.display()))?;
        let fd = file.as_raw_fd();
        eprintln!("kms: {} ({})", card.display(), driver(&libs, fd));
        let res = unsafe { (libs.get_resources)(fd) };
        if res.is_null() { return Err(context(io::Error::last_os_error(), "drmModeGetResources (not a KMS device?)")); }
        let picked = pick(&libs, fd, unsafe { &*res });
        unsafe { (libs.free_resources)(res) };
        let (connector, crtc, crtc_index, mode, modes) = picked?;
        eprintln!("kms: using {mode} on CRTC {crtc}");
        let device = unsafe { (libs.gbm_create_device)(fd) };
        if device.is_null() { return Err(context(io::Error::last_os_error(), "gbm_create_device")); }
        let surface = unsafe { (libs.gbm_surface_create)(device, mode.width(), mode.height(), FORMAT_XRGB8888, GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING) };
        if surface.is_null() {
            unsafe { (libs.gbm_device_destroy)(device) };
            return Err(context(io::Error::last_os_error(), "gbm_surface_create (XRGB8888, scanout)"));
        }
        Ok(Kms { libs, card: file, connector, crtc, crtc_index, mode, modes, mode_set: true, device, surface, framebuffers: Vec::new(),
            shown: None, flipping: None, last_flip: None, stats: FlipStats::default() })
    }

    /// The mode in use.
    pub fn mode(&self) -> Mode { self.mode }

    /// Every mode the output offers.
    pub fn modes(&self) -> &[Mode] { &self.modes }

    /// Switches to `mode` (one of `modes`, of the same size) at the next present, with a full mode
    /// set: the panel may blank for a moment.
    pub fn set_mode(&mut self, mode: Mode) -> io::Result<()> {
        if (mode.width(), mode.height()) != (self.mode.width(), self.mode.height()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("kms: {mode} isn't the surface's size")));
        }
        eprintln!("kms: switching to {mode}");
        self.mode = mode;
        self.mode_set = true;
        Ok(())
    }

    /// For EGL: the GBM device and surface, and the surface's format.
    pub fn native(&self) -> crate::egl::Native {
        crate::egl::Native::Gbm { device: self.device, surface: self.surface, format: FORMAT_XRGB8888 }
    }

    /// The CRTC's position in the card's list, to open more `DrmVsync`s later.
    pub fn crtc_index(&self) -> u32 { self.crtc_index }

    /// A vsync for other threads, on its own file of the same card (`card`: the path `open` was
    /// given).
    pub fn vsync(&self, card: &Path) -> io::Result<DrmVsync> { DrmVsync::open(card, self.crtc_index) }

    /// Puts the frame EGL just finished (after `eglSwapBuffers`) on screen at the next vblank. The
    /// first, and the first after `set_mode`, sets the mode. If the frame before is still waiting for its flip, waits for it first
    /// (at most a refresh).
    pub fn present(&mut self) -> io::Result<()> {
        let bo = unsafe { (self.libs.gbm_surface_lock_front_buffer)(self.surface) };
        if bo.is_null() { return Err(context(io::Error::last_os_error(), "gbm_surface_lock_front_buffer (no frame drawn?)")); }
        let result = self.flip_to(bo);
        if result.is_err() { unsafe { (self.libs.gbm_surface_release_buffer)(self.surface, bo) }; }
        result
    }

    fn flip_to(&mut self, bo: *mut c_void) -> io::Result<()> {
        let fb = self.framebuffer(bo)?;
        let fd = self.card.as_raw_fd();
        self.wait_for_flip()?;
        if self.mode_set {
            let r = unsafe { (self.libs.set_crtc)(fd, self.crtc, fb, 0, 0, &self.connector, 1, &self.mode.info) };
            if r != 0 { return Err(context(io::Error::from_raw_os_error(-r), format!("drmModeSetCrtc on CRTC {} (another program holds the display?)", self.crtc))); }
            if let Some(old) = self.shown.replace(bo) { unsafe { (self.libs.gbm_surface_release_buffer)(self.surface, old) }; }
            self.mode_set = false;
            self.last_flip = None;
            return Ok(());
        }
        let r = unsafe { (self.libs.page_flip)(fd, self.crtc, fb, DRM_MODE_PAGE_FLIP_EVENT, std::ptr::null_mut()) };
        if r != 0 { return Err(context(io::Error::from_raw_os_error(-r), format!("drmModePageFlip on CRTC {}", self.crtc))); }
        self.flipping = Some(bo);
        Ok(())
    }

    /// The DRM framebuffer of a GBM buffer, made the first time it is shown. (A GBM surface
    /// cycles through a few buffers, so the list stays short.)
    fn framebuffer(&mut self, bo: *mut c_void) -> io::Result<u32> {
        if let Some(&(_, fb)) = self.framebuffers.iter().find(|(b, _)| *b == bo) { return Ok(fb); }
        let (handle, stride) = unsafe { ((self.libs.gbm_bo_get_handle)(bo) as u32, (self.libs.gbm_bo_get_stride)(bo)) };
        let mut fb = 0;
        let r = unsafe { (self.libs.add_fb)(self.card.as_raw_fd(), self.mode.width(), self.mode.height(), 24, 32, stride, handle, &mut fb) };
        if r != 0 { return Err(context(io::Error::from_raw_os_error(-r), "drmModeAddFB")); }
        self.framebuffers.push((bo, fb));
        Ok(fb)
    }

    /// Waits until the flip in progress, if any, has completed, then gives the buffer it replaced
    /// back to GBM.
    fn wait_for_flip(&mut self) -> io::Result<()> {
        while self.flipping.is_some() {
            let mut buf = [0u8; 1024];
            let n = (&self.card).read(&mut buf).map_err(|e| context(e, "reading DRM events"))?;
            let mut events = &buf[..n];
            // `struct drm_event` (type, length), then for a flip `struct drm_event_vblank`:
            // user_data (8 bytes), tv_sec, tv_usec, sequence, crtc_id.
            while events.len() >= 8 {
                let word = |i: usize| u32::from_ne_bytes(events[i..i + 4].try_into().unwrap());
                let (kind, len) = (word(0), word(4) as usize);
                if len < 8 || len > events.len() { break; }
                if kind == DRM_EVENT_FLIP_COMPLETE && len >= 32 {
                    self.flipped(Duration::new(word(16) as u64, word(20) * 1000));
                }
                events = &events[len..];
            }
        }
        Ok(())
    }

    fn flipped(&mut self, at: Duration) {
        if let Some(old) = std::mem::replace(&mut self.shown, self.flipping.take()) {
            unsafe { (self.libs.gbm_surface_release_buffer)(self.surface, old) };
        }
        let refresh = Duration::from_secs_f64(1.0 / self.mode.refresh_hz().max(1.0));
        if let Some(gap) = self.last_flip.map(|last| at.saturating_sub(last)) {
            if gap > refresh * 3 / 2 && gap < Duration::from_secs(1) { self.stats.late += 1; }
            self.stats.longest = self.stats.longest.max(gap);
        }
        self.stats.flips += 1;
        self.last_flip = Some(at);
    }

    /// How the flips have gone since the last call. A gap of a second or more counts as idle,
    /// not late (nothing changed on screen).
    pub fn take_flip_stats(&mut self) -> FlipStats { std::mem::take(&mut self.stats) }
}

impl Drop for Kms {
    fn drop(&mut self) {
        let _ = self.wait_for_flip();
        let fd = self.card.as_raw_fd();
        unsafe {
            for &(_, fb) in &self.framebuffers { (self.libs.rm_fb)(fd, fb); }
            if let Some(bo) = self.shown { (self.libs.gbm_surface_release_buffer)(self.surface, bo); }
            (self.libs.gbm_surface_destroy)(self.surface);
            (self.libs.gbm_device_destroy)(self.device);
        }
    }
}

/// The connector, CRTC (id and index) and mode to use, and the connector's modes, logging every connector and mode seen.
fn pick(libs: &Libs, fd: c_int, res: &Resources) -> io::Result<(u32, u32, u32, Mode, Vec<Mode>)> {
    let ids = |p: *const u32, n: c_int| if p.is_null() || n <= 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(p, n as usize) } };
    let crtcs = ids(res.crtcs, res.count_crtcs);
    // (built-in, connector, the CRTCs it can use as bits, the CRTC driving it now, mode)
    let mut best: Option<(bool, u32, u32, Option<u32>, Mode, Vec<Mode>)> = None;
    for &id in ids(res.connectors, res.count_connectors) {
        let c = unsafe { (libs.get_connector)(fd, id) };
        if c.is_null() { continue; }
        let conn = unsafe { &*c };
        let name = format!("{}-{}", CONNECTOR_TYPES.get(conn.connector_type as usize).unwrap_or(&"?"), conn.connector_type_id);
        let modes: Vec<Mode> = if conn.modes.is_null() { Vec::new() } else {
            unsafe { std::slice::from_raw_parts(conn.modes, conn.count_modes.max(0) as usize) }.iter().map(|&info| Mode { info }).collect()
        };
        let connected = conn.connection == DRM_MODE_CONNECTED;
        eprintln!("kms: connector {name} ({id}): {}, {} modes, {}x{} mm{}", if connected { "connected" } else { "not connected" },
            modes.len(), conn.mm_width, conn.mm_height,
            modes.iter().map(|m| format!("\n  {m}")).collect::<String>());
        let internal = INTERNAL.contains(&conn.connector_type);
        let mode = modes.iter().find(|m| m.preferred()).or(modes.first()).copied();
        if let (true, Some(mode)) = (connected, mode) {
            if best.as_ref().is_none_or(|b| internal && !b.0) {
                // The CRTCs its encoders can drive, and the one driving it now (preferred), if any.
                let (mut possible, mut current_crtc) = (0, None);
                for &e in ids(conn.encoders, conn.count_encoders) {
                    let enc = unsafe { (libs.get_encoder)(fd, e) };
                    if enc.is_null() { continue; }
                    let enc_ref = unsafe { &*enc };
                    possible |= enc_ref.possible_crtcs;
                    if e == conn.encoder_id && enc_ref.crtc_id != 0 { current_crtc = Some(enc_ref.crtc_id); }
                    unsafe { (libs.free_encoder)(enc) };
                }
                best = Some((internal, id, possible, current_crtc, mode, modes.clone()));
            }
        }
        unsafe { (libs.free_connector)(c) };
    }
    let (_, connector, possible, current_crtc, mode, modes) = best.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "kms: no connected output with a mode"))?;
    let index = current_crtc.and_then(|id| crtcs.iter().position(|&c| c == id))
        .or_else(|| (0..crtcs.len()).find(|&i| possible & (1 << i) != 0))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "kms: no CRTC can drive the output"))?;
    Ok((connector, crtcs[index], index as u32, mode, modes))
}

/// The card's driver name and version, for the log.
fn driver(libs: &Libs, fd: c_int) -> String {
    let v = unsafe { (libs.get_version)(fd) };
    if v.is_null() { return "unknown driver".into(); }
    let text = unsafe {
        let v = &*v;
        let s = |p: *const c_char| if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() };
        format!("{} {}.{}.{}, {}", s(v.name), v.major, v.minor, v.patch, s(v.desc))
    };
    unsafe { (libs.free_version)(v) };
    text
}

fn context(e: io::Error, what: impl std::fmt::Display) -> io::Error { io::Error::new(e.kind(), format!("{what}: {e}")) }

/// `DRM_IOCTL_WAIT_VBLANK`: `_IOWR('d', 0x3a, union drm_wait_vblank)`, 24 bytes.
const DRM_IOCTL_WAIT_VBLANK: libc::c_ulong = 0xC018_643A;
const DRM_VBLANK_RELATIVE: u32 = 0x1;
const DRM_VBLANK_SECONDARY: u32 = 0x2000_0000;
const DRM_VBLANK_HIGH_CRTC_SHIFT: u32 = 1;
const DRM_VBLANK_HIGH_CRTC_MASK: u32 = 0x3e;

/// `union drm_wait_vblank`: the request (type, sequence, signal) and the reply (type, sequence,
/// tv_sec, tv_usec) share it.
#[repr(C)]
struct WaitVblank { kind: u32, sequence: u32, sec: i64, usec: i64 }

/// Waits for a CRTC's next vblank. Works on any open file of the card (it needs no DRM master),
/// so each thread that waits has its own.
pub struct DrmVsync { card: File, crtc_bits: u32 }

impl DrmVsync {
    /// Opens `card` for waits on the CRTC at `crtc_index` in the card's list. Fails if a wait
    /// fails (the CRTC is off, or the driver has no vblank interrupt), so callers can fall back
    /// to a timer.
    pub fn open(card: &Path, crtc_index: u32) -> io::Result<Self> {
        let crtc_bits = match crtc_index {
            0 => 0,
            1 => DRM_VBLANK_SECONDARY,
            i => (i << DRM_VBLANK_HIGH_CRTC_SHIFT) & DRM_VBLANK_HIGH_CRTC_MASK,
        };
        let mut v = Self { card: File::options().read(true).write(true).open(card).map_err(|e| context(e, card.display()))?, crtc_bits };
        v.wait_at()?;
        Ok(v)
    }

    /// Waits for the next vblank; returns when it was, on CLOCK_MONOTONIC.
    fn wait_at(&mut self) -> io::Result<Duration> {
        let mut w = WaitVblank { kind: DRM_VBLANK_RELATIVE | self.crtc_bits, sequence: 1, sec: 0, usec: 0 };
        if unsafe { libc::ioctl(self.card.as_raw_fd(), DRM_IOCTL_WAIT_VBLANK as _, &mut w) } < 0 {
            return Err(context(io::Error::last_os_error(), "DRM_IOCTL_WAIT_VBLANK"));
        }
        Ok(Duration::new(w.sec as u64, w.usec as u32 * 1000))
    }

    /// The display's real refresh rate, from the times of `vblanks` vblanks.
    pub fn measure(&mut self, vblanks: u32) -> io::Result<f64> {
        let start = self.wait_at()?;
        let mut end = start;
        for _ in 0..vblanks { end = self.wait_at()?; }
        let took = (end - start).as_secs_f64();
        if took <= 0.0 { return Err(io::Error::other("vblank times didn't advance")); }
        Ok(vblanks as f64 / took)
    }
}

impl Vsync for DrmVsync {
    fn wait(&mut self) -> io::Result<()> { self.wait_at().map(|_| ()) }
}
