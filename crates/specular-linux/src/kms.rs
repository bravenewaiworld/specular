//! The display through DRM/KMS: the built-in panel in its preferred mode, or a TV (HDMI) in its
//! place while one is connected; a GBM surface for EGL to draw into (`egl::Native::Gbm`), and
//! each finished frame put on screen by a page flip at the next vblank. Also a vsync for games and
//! animations from the card's vblank counter. libdrm and libgbm are loaded at runtime, as the GPU
//! libraries are.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
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
struct DrmConnector {
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
const DRM_MODE_FLAG_INTERLACE: u32 = 1 << 4;
const DRM_MODE_FLAG_DBLSCAN: u32 = 1 << 5;
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
    get_connector: unsafe extern "C" fn(c_int, u32) -> *mut DrmConnector,
    free_connector: unsafe extern "C" fn(*mut DrmConnector),
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

impl PartialEq for Mode {
    fn eq(&self, o: &Self) -> bool { self.info.clock == o.info.clock && self.refresh_hz() == o.refresh_hz() && self.info.name == o.info.name }
}

impl std::fmt::Debug for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{self}") }
}

impl Mode {
    pub fn width(&self) -> u32 { self.info.hdisplay as u32 }
    pub fn height(&self) -> u32 { self.info.vdisplay as u32 }
    pub fn size(&self) -> (u32, u32) { (self.width(), self.height()) }
    /// The refresh rate its timings give (the pixel clock over the total pixels per frame). The
    /// panel's real rate can differ, when the clock it gets isn't the one asked for: measure it
    /// with `DrmVsync::measure`.
    pub fn refresh_hz(&self) -> f64 {
        let total = self.info.htotal as f64 * self.info.vtotal as f64;
        if total == 0.0 { 0.0 } else { self.info.clock as f64 * 1000.0 / total }
    }
    /// Whether it draws every line each refresh (not interlaced or line-doubled). Specular uses
    /// only such modes: interlacing makes thin lines of the UI flicker.
    pub fn progressive(&self) -> bool { self.info.flags & (DRM_MODE_FLAG_INTERLACE | DRM_MODE_FLAG_DBLSCAN) == 0 }
    fn preferred(&self) -> bool { self.info.kind & DRM_MODE_TYPE_PREFERRED != 0 }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}x{} {:.2} Hz ({} kHz){}{}", self.width(), self.height(), self.refresh_hz(), self.info.clock,
            if self.progressive() { "" } else { " interlaced" }, if self.preferred() { " preferred" } else { "" })
    }
}

/// A mode for the display's owner to switch to at its next present (`set_mode`), asked for from
/// any thread.
pub type ModeRequest = std::sync::Arc<std::sync::Mutex<Option<Mode>>>;

/// The index (in the card's list) of the CRTC showing the picture, or `NO_CRTC` while it moves to
/// another output. `Kms` sets it; `DrmVsync`s read it at each wait, so they follow the picture.
pub type ActiveCrtc = Arc<AtomicU32>;

/// `ActiveCrtc` while no CRTC shows the picture (between turning one off and the next on).
pub const NO_CRTC: u32 = u32::MAX;

/// How the page flips have gone since the last `Kms::take_flip_stats`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlipStats {
    pub flips: u32,
    /// Flips that came more than 1.5 refreshes after the one before.
    pub late: u32,
    /// The longest time between two flips, idle gaps included (which `late` leaves out).
    pub longest: Duration,
}

/// A rectangle in pixels, from the top left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect { pub x: u32, pub y: u32, pub width: u32, pub height: u32 }

impl std::fmt::Display for Rect {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "{}x{} at {},{}", self.width, self.height, self.x, self.y) }
}

/// Where a picture of size `from` goes on a screen of size `to`: as large as fits keeping its
/// shape, centred (black bars at the sides or above and below). Assumes square pixels.
pub fn fit(from: (u32, u32), to: (u32, u32)) -> Rect {
    let ((fw, fh), (tw, th)) = (from, to);
    let (width, height) = if tw as u64 * fh as u64 >= th as u64 * fw as u64 {
        (((th as u64 * fw as u64 + fh as u64 / 2) / fh as u64) as u32, th)
    } else {
        (tw, ((tw as u64 * fh as u64 + fw as u64 / 2) / fw as u64) as u32)
    };
    Rect { x: (tw - width) / 2, y: (th - height) / 2, width, height }
}

/// A connected display output (not to be confused with `device::Output`, where sound goes) and
/// how Specular drives it: its connector, a CRTC for it, and the mode chosen for it (the main
/// output's preferred one, `choose_main`; a TV's from `tv_mode`).
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// As the kernel names it (e.g. DSI-1, HDMI-A-1).
    pub name: String,
    /// A TV or monitor (not a built-in panel).
    pub external: bool,
    pub mode: Mode,
    /// Every mode it offers, as it lists them.
    pub modes: Vec<Mode>,
    connector: u32,
    crtc: u32,
    crtc_index: u32,
}

/// The largest picture an external display gets: bigger modes (4K) would have the GPU scale
/// every frame up to millions more pixels, for no sharper menus.
const TV_MAX: (u32, u32) = (1920, 1080);

/// The mode to use on an external display: its preferred mode, unless that is interlaced or
/// bigger than `TV_MAX`; then the largest progressive mode within `TV_MAX`, nearest 60 Hz.
fn tv_mode(modes: &[Mode]) -> Option<Mode> {
    let usable = |m: &&Mode| m.progressive() && m.width() <= TV_MAX.0 && m.height() <= TV_MAX.1;
    modes.iter().filter(usable).find(|m| m.preferred())
        .or_else(|| modes.iter().filter(usable).max_by(|a, b| (a.width() * a.height()).cmp(&(b.width() * b.height()))
            .then((b.refresh_hz() - 60.0).abs().total_cmp(&(a.refresh_hz() - 60.0).abs()))))
        .copied()
}

/// The mode of `modes` nearest `hz` (a game's rate), if one is within `within` of it (a
/// fraction) and is progressive and of `usual`'s size: `usual` is the output's chosen mode, whose
/// size its GBM surface has, and only modes of that size can be switched to.
pub fn nearest_mode(modes: &[Mode], usual: Mode, hz: f64, within: f64) -> Option<Mode> {
    let off = |m: &Mode| (m.refresh_hz() / hz - 1.0).abs();
    modes.iter().filter(|m| m.progressive() && m.size() == usual.size() && off(m) <= within)
        .min_by(|a, b| off(a).total_cmp(&off(b))).copied()
}

/// A connector as read from the card.
#[derive(Clone, Debug)]
struct Connector {
    id: u32,
    name: String,
    internal: bool,
    connected: bool,
    modes: Vec<Mode>,
    /// The CRTCs its encoders can drive, as bits of their indexes.
    possible_crtcs: u32,
    /// The CRTC driving it now, if any.
    current_crtc: Option<u32>,
}

/// The output to show the UI on at start-up: the first connected built-in panel (LVDS, eDP,
/// DSI), else the first connected output, in its preferred mode (else its first).
fn choose_main(connectors: &[Connector], crtcs: &[u32]) -> io::Result<Output> {
    let usable = |c: &&Connector| c.connected && !c.modes.is_empty();
    let c = connectors.iter().filter(usable).find(|c| c.internal).or_else(|| connectors.iter().find(usable))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "kms: no connected output with a mode"))?;
    let mode = c.modes.iter().find(|m| m.preferred()).unwrap_or(&c.modes[0]);
    output(c, *mode, crtcs, None)
}

/// The external output to show the UI on instead of the main one, if one is connected with a
/// usable mode (`tv_mode`). Always None when the main output is itself external (a board without
/// a built-in panel): the picture then never moves.
fn choose_tv(connectors: &[Connector], crtcs: &[u32], main: &Output) -> Option<Output> {
    if main.external { return None; }
    connectors.iter().filter(|c| c.connected && !c.internal && c.id != main.connector)
        .find_map(|c| output(c, tv_mode(&c.modes)?, crtcs, Some(main.crtc)).ok())
}

/// `c` in `mode`, on the CRTC driving it now (even if that is `avoid`), else on the first it can
/// use other than `avoid` (the panel's CRTC: it would be free once the panel is off, but with a
/// CRTC of its own the TV never has to take over the panel's), else on `avoid`.
fn output(c: &Connector, mode: Mode, crtcs: &[u32], avoid: Option<u32>) -> io::Result<Output> {
    let possible = |i: &usize| c.possible_crtcs & (1 << i) != 0;
    let index = c.current_crtc.and_then(|id| crtcs.iter().position(|&x| x == id))
        .or_else(|| (0..crtcs.len()).filter(possible).min_by_key(|&i| Some(crtcs[i]) == avoid))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("kms: no CRTC can drive {}", c.name)))?;
    Ok(Output { name: c.name.clone(), external: !c.internal, mode, modes: c.modes.clone(), connector: c.id, crtc: crtcs[index], crtc_index: index as u32 })
}

/// An output the picture is on, or can be: its GBM surface, whose buffers it scans out, and the
/// buffers taken from that surface for the screen (`gbm_surface_lock_front_buffer`) and not yet
/// given back.
struct Screen {
    output: Output,
    /// The mode in use (`output.mode` unless `set_mode` changed it).
    mode: Mode,
    /// Whether the next present sets `mode` (a full mode set) rather than flipping.
    mode_set: bool,
    surface: *mut c_void,
    /// Each GBM buffer presented so far, with its DRM framebuffer.
    framebuffers: Vec<(*mut c_void, u32)>,
    /// The buffer on screen.
    shown: Option<*mut c_void>,
    /// The buffer flipped to, until the flip completes.
    flipping: Option<*mut c_void>,
}

impl Screen {
    /// `output`, with a GBM surface of its mode's size. Nothing changes on screen until it is
    /// presented to.
    fn new(libs: &Libs, device: *mut c_void, output: Output) -> io::Result<Self> {
        let mode = output.mode;
        let surface = unsafe { (libs.gbm_surface_create)(device, mode.width(), mode.height(), FORMAT_XRGB8888, GBM_BO_USE_SCANOUT | GBM_BO_USE_RENDERING) };
        if surface.is_null() { return Err(context(io::Error::last_os_error(), format!("gbm_surface_create ({}x{} XRGB8888, scanout)", mode.width(), mode.height()))); }
        Ok(Self { output, mode, mode_set: true, surface, framebuffers: Vec::new(), shown: None, flipping: None })
    }

    /// The DRM framebuffer of a GBM buffer, made the first time it is shown. (A GBM surface
    /// cycles through a few buffers, so the list stays short.)
    fn framebuffer(&mut self, libs: &Libs, fd: c_int, bo: *mut c_void) -> io::Result<u32> {
        if let Some(&(_, fb)) = self.framebuffers.iter().find(|(b, _)| *b == bo) { return Ok(fb); }
        let (handle, stride) = unsafe { ((libs.gbm_bo_get_handle)(bo) as u32, (libs.gbm_bo_get_stride)(bo)) };
        let mut fb = 0;
        let r = unsafe { (libs.add_fb)(fd, self.output.mode.width(), self.output.mode.height(), 24, 32, stride, handle, &mut fb) };
        if r != 0 { return Err(context(io::Error::from_raw_os_error(-r), "drmModeAddFB")); }
        self.framebuffers.push((bo, fb));
        Ok(fb)
    }

    /// Turns its CRTC off (the output goes dark) and gives its buffers back to GBM. The next
    /// present to it sets its mode again. Call with no flip pending.
    fn off(&mut self, libs: &Libs, fd: c_int) {
        let r = unsafe { (libs.set_crtc)(fd, self.output.crtc, 0, 0, 0, std::ptr::null(), 0, std::ptr::null()) };
        if r != 0 { eprintln!("kms: turning {} off (CRTC {}): {}", self.output.name, self.output.crtc, io::Error::from_raw_os_error(-r)); }
        if let Some(bo) = self.shown.take() { unsafe { (libs.gbm_surface_release_buffer)(self.surface, bo) }; }
        self.mode_set = true;
    }

    /// Frees its framebuffers and GBM surface (the EGL surface on it must be gone already).
    fn destroy(&mut self, libs: &Libs, fd: c_int) {
        unsafe {
            for (_, fb) in self.framebuffers.drain(..) { (libs.rm_fb)(fd, fb); }
            if let Some(bo) = self.shown.take() { (libs.gbm_surface_release_buffer)(self.surface, bo); }
            (libs.gbm_surface_destroy)(self.surface);
        }
    }
}

/// The display, owned while open (this process is the card's DRM master). The UI is drawn with
/// EGL into the main output's GBM surface (`native`): usually the built-in panel, whose size the
/// UI keeps. While a TV is connected (`find_tv`, `show_on`) the picture is on the TV instead and
/// the panel is off: the UI's frames are then copied, scaled, onto the TV's own GBM surface
/// (`tv_native`) by the owner (`egl::EglDisplay::copy_to`). `present` puts each frame on whichever
/// output shows the picture.
pub struct Kms {
    libs: Libs,
    card: File,
    device: *mut c_void,
    main: Screen,
    tv: Option<Screen>,
    active_crtc: ActiveCrtc,
    last_flip: Option<Duration>,
    stats: FlipStats,
}

impl Kms {
    /// Opens `card` (e.g. /dev/dri/card0) on its main output (`choose_main`), with a GBM surface
    /// of that size. Logs what it found and chose, as `kms:` lines. Nothing changes on screen
    /// until the first `present`.
    pub fn open(card: &Path) -> io::Result<Kms> {
        let libs = Libs::load()?;
        let file = File::options().read(true).write(true).open(card).map_err(|e| context(e, card.display()))?;
        let fd = file.as_raw_fd();
        eprintln!("kms: {} ({})", card.display(), driver(&libs, fd));
        let (crtcs, connectors) = read_connectors(&libs, fd)?;
        let output = choose_main(&connectors, &crtcs)?;
        eprintln!("kms: using {} in {} on CRTC {}", output.name, output.mode, output.crtc);
        let device = unsafe { (libs.gbm_create_device)(fd) };
        if device.is_null() { return Err(context(io::Error::last_os_error(), "gbm_create_device")); }
        let active_crtc = Arc::new(AtomicU32::new(output.crtc_index));
        let main = Screen::new(&libs, device, output).inspect_err(|_| unsafe { (libs.gbm_device_destroy)(device) })?;
        Ok(Kms { libs, card: file, device, main, tv: None, active_crtc, last_flip: None, stats: FlipStats::default() })
    }

    fn active(&self) -> &Screen { self.tv.as_ref().unwrap_or(&self.main) }

    /// The output showing the picture, in the mode it was chosen in.
    pub fn output(&self) -> &Output { &self.active().output }

    /// The UI's size: the main output's mode's, whichever output shows it.
    pub fn size(&self) -> (u32, u32) { self.main.output.mode.size() }

    /// The mode in use on the output showing the picture.
    pub fn mode(&self) -> Mode { self.active().mode }

    /// Every mode the output showing the picture offers.
    pub fn modes(&self) -> &[Mode] { &self.active().output.modes }

    /// Switches the output showing the picture to `mode` (one of its `modes`, of the same size) at
    /// the next present, with a full mode set: the screen may blank for a moment.
    pub fn set_mode(&mut self, mode: Mode) -> io::Result<()> {
        let screen = self.tv.as_mut().unwrap_or(&mut self.main);
        if !screen.output.modes.contains(&mode) || mode.size() != screen.output.mode.size() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("kms: {mode} isn't a mode of {} at its surface's size", screen.output.name)));
        }
        eprintln!("kms: switching {} to {mode}", screen.output.name);
        screen.mode = mode;
        screen.mode_set = true;
        Ok(())
    }

    /// For EGL: the GBM device and the main output's surface, and the surface's format.
    pub fn native(&self) -> crate::egl::Native {
        crate::egl::Native::Gbm { device: self.device, surface: self.main.surface, format: FORMAT_XRGB8888 }
    }

    /// The TV's GBM surface while the picture is on one, for a second EGL surface
    /// (`EglDisplay::create_copy_target`).
    pub fn tv_native(&self) -> Option<*mut c_void> { self.tv.as_ref().map(|s| s.surface) }

    /// The CRTC showing the picture, shared with every `DrmVsync` opened from it.
    pub fn active_crtc(&self) -> ActiveCrtc { self.active_crtc.clone() }

    /// A vsync for other threads, on its own file of the same card (`card`: the path `open` was
    /// given). It follows the picture from output to output.
    pub fn vsync(&self, card: &Path) -> io::Result<DrmVsync> { DrmVsync::open(card, self.active_crtc()) }

    /// Reads the connectors again (after a hotplug event), logging each, and returns the external
    /// output the picture should be on (`choose_tv`): None for the main output.
    pub fn find_tv(&self) -> io::Result<Option<Output>> {
        let (crtcs, connectors) = read_connectors(&self.libs, self.card.as_raw_fd())?;
        Ok(choose_tv(&connectors, &crtcs, &self.main.output))
    }

    /// The external output showing the picture, if any.
    pub fn tv(&self) -> Option<&Output> { self.tv.as_ref().map(|s| &s.output) }

    /// Moves the picture to `tv` (None: back to the main output) at the next present, turning the
    /// output that showed it off. Any EGL surface on the TV's GBM surface must be destroyed
    /// first. If the TV's GBM surface can't be made, the picture stays on (goes back to) the main
    /// output; if waiting for a pending flip fails, nothing changes.
    pub fn show_on(&mut self, tv: Option<Output>) -> io::Result<()> {
        self.wait_for_flip()?;
        self.active_crtc.store(NO_CRTC, Ordering::Relaxed);
        self.last_flip = None;
        let fd = self.card.as_raw_fd();
        match self.tv.take() {
            Some(mut old) => { old.off(&self.libs, fd); old.destroy(&self.libs, fd); }
            None if tv.is_some() => self.main.off(&self.libs, fd),
            None => {}
        }
        // The main output in its usual mode again: a game's mode (`set_mode`) is dropped, and asked
        // for again as the game resumes.
        (self.main.mode, self.main.mode_set) = (self.main.output.mode, true);
        let Some(tv) = tv else { return Ok(()) };
        self.tv = Some(Screen::new(&self.libs, self.device, tv)?);
        Ok(())
    }

    /// Puts the frame just finished on the output showing the picture, at its next vblank: the
    /// main surface's (after `eglSwapBuffers`), or the TV's (after the copy onto it). The first
    /// present to an output, and the first after `set_mode`, sets the mode. If the frame before
    /// is still waiting for its flip, waits for it first (at most a refresh).
    pub fn present(&mut self) -> io::Result<()> {
        let surface = self.active().surface;
        let bo = unsafe { (self.libs.gbm_surface_lock_front_buffer)(surface) };
        if bo.is_null() { return Err(context(io::Error::last_os_error(), "gbm_surface_lock_front_buffer (no frame drawn?)")); }
        let result = self.flip_to(bo);
        if result.is_err() { unsafe { (self.libs.gbm_surface_release_buffer)(surface, bo) }; }
        result
    }

    fn flip_to(&mut self, bo: *mut c_void) -> io::Result<()> {
        let fd = self.card.as_raw_fd();
        self.wait_for_flip()?;
        let screen = self.tv.as_mut().unwrap_or(&mut self.main);
        let fb = screen.framebuffer(&self.libs, fd, bo)?;
        let crtc = screen.output.crtc;
        if screen.mode_set {
            let r = unsafe { (self.libs.set_crtc)(fd, crtc, fb, 0, 0, &screen.output.connector, 1, &screen.mode.info) };
            if r != 0 { return Err(context(io::Error::from_raw_os_error(-r), format!("drmModeSetCrtc for {} on CRTC {crtc} (another program holds the display?)", screen.output.name))); }
            if let Some(old) = screen.shown.replace(bo) { unsafe { (self.libs.gbm_surface_release_buffer)(screen.surface, old) }; }
            screen.mode_set = false;
            self.active_crtc.store(screen.output.crtc_index, Ordering::Relaxed);
            self.last_flip = None;
            return Ok(());
        }
        let r = unsafe { (self.libs.page_flip)(fd, crtc, fb, DRM_MODE_PAGE_FLIP_EVENT, std::ptr::null_mut()) };
        if r != 0 { return Err(context(io::Error::from_raw_os_error(-r), format!("drmModePageFlip for {} on CRTC {crtc}", screen.output.name))); }
        screen.flipping = Some(bo);
        Ok(())
    }

    /// Waits until the flip in progress, if any, has completed, then gives the buffer it replaced
    /// back to GBM. (Only the output showing the picture flips.)
    fn wait_for_flip(&mut self) -> io::Result<()> {
        while self.active().flipping.is_some() {
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
        let screen = self.tv.as_mut().unwrap_or(&mut self.main);
        if let Some(old) = std::mem::replace(&mut screen.shown, screen.flipping.take()) {
            unsafe { (self.libs.gbm_surface_release_buffer)(screen.surface, old) };
        }
        let refresh = Duration::from_secs_f64(1.0 / screen.mode.refresh_hz().max(1.0));
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
        if let Some(mut tv) = self.tv.take() { tv.destroy(&self.libs, fd); }
        self.main.destroy(&self.libs, fd);
        unsafe { (self.libs.gbm_device_destroy)(self.device) };
    }
}

/// The card's CRTCs and connectors, logging every connector and mode seen.
fn read_connectors(libs: &Libs, fd: c_int) -> io::Result<(Vec<u32>, Vec<Connector>)> {
    let res = unsafe { (libs.get_resources)(fd) };
    if res.is_null() { return Err(context(io::Error::last_os_error(), "drmModeGetResources (not a KMS device?)")); }
    let r = unsafe { &*res };
    let ids = |p: *const u32, n: c_int| if p.is_null() || n <= 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(p, n as usize) } };
    let crtcs = ids(r.crtcs, r.count_crtcs).to_vec();
    let mut connectors = Vec::new();
    for &id in ids(r.connectors, r.count_connectors) {
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
        let (mut possible_crtcs, mut current_crtc) = (0, None);
        for &e in ids(conn.encoders, conn.count_encoders) {
            let enc = unsafe { (libs.get_encoder)(fd, e) };
            if enc.is_null() { continue; }
            let enc_ref = unsafe { &*enc };
            possible_crtcs |= enc_ref.possible_crtcs;
            if e == conn.encoder_id && enc_ref.crtc_id != 0 { current_crtc = Some(enc_ref.crtc_id); }
            unsafe { (libs.free_encoder)(enc) };
        }
        connectors.push(Connector { id, name, internal: INTERNAL.contains(&conn.connector_type), connected, modes, possible_crtcs, current_crtc });
        unsafe { (libs.free_connector)(c) };
    }
    unsafe { (libs.free_resources)(res) };
    Ok((crtcs, connectors))
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

/// Waits for the next vblank of the CRTC showing the picture (`ActiveCrtc`). Works on any open
/// file of the card (it needs no DRM master), so each thread that waits has its own.
pub struct DrmVsync { card: File, crtc: ActiveCrtc }

/// How long a wait takes while the picture moves between outputs (no CRTC has vblanks then):
/// about a refresh at 60 Hz.
const SWITCHING_WAIT: Duration = Duration::from_millis(16);

impl DrmVsync {
    /// Opens `card` for waits on the CRTC `crtc` names, whichever it is at each wait. Fails if a
    /// wait fails (the CRTC is off, or the driver has no vblank interrupt), so callers can fall
    /// back to a timer. (Opened while the picture moves, it can't tell, and succeeds.)
    pub fn open(card: &Path, crtc: ActiveCrtc) -> io::Result<Self> {
        let mut v = Self { card: File::options().read(true).write(true).open(card).map_err(|e| context(e, card.display()))?, crtc };
        v.wait_at()?;
        Ok(v)
    }

    /// Waits for the next vblank; returns when it was, on CLOCK_MONOTONIC. While the picture moves
    /// to another output, sleeps `SWITCHING_WAIT` instead, and a wait that fails because the CRTC
    /// went off during it waits again on the new one; so a switch never fails a wait.
    fn wait_at(&mut self) -> io::Result<Duration> {
        loop {
            let index = self.crtc.load(Ordering::Relaxed);
            if index == NO_CRTC {
                std::thread::sleep(SWITCHING_WAIT);
                return Ok(crate::input::clock(libc::CLOCK_MONOTONIC));
            }
            let mut w = WaitVblank { kind: DRM_VBLANK_RELATIVE | crtc_bits(index), sequence: 1, sec: 0, usec: 0 };
            if unsafe { libc::ioctl(self.card.as_raw_fd(), DRM_IOCTL_WAIT_VBLANK as _, &mut w) } == 0 {
                return Ok(Duration::new(w.sec as u64, w.usec as u32 * 1000));
            }
            let e = io::Error::last_os_error();
            if self.crtc.load(Ordering::Relaxed) == index { return Err(context(e, "DRM_IOCTL_WAIT_VBLANK")); }
        }
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

/// How a vblank wait names the CRTC at `index`.
fn crtc_bits(index: u32) -> u32 {
    match index {
        0 => 0,
        1 => DRM_VBLANK_SECONDARY,
        i => (i << DRM_VBLANK_HIGH_CRTC_SHIFT) & DRM_VBLANK_HIGH_CRTC_MASK,
    }
}

impl Vsync for DrmVsync {
    fn wait(&mut self) -> io::Result<()> { self.wait_at().map(|_| ()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mode at `hz` (or, from 1000 up, millihertz).
    fn mode(w: u16, h: u16, hz: u32, preferred: bool, interlaced: bool) -> Mode {
        let (htotal, vtotal) = (w + 100, h + 50);
        let mhz = if hz < 1000 { hz * 1000 } else { hz };
        let clock = (htotal as u64 * vtotal as u64 * mhz as u64 / 1_000_000) as u32;
        Mode { info: ModeInfo { clock, hdisplay: w, hsync_start: w, hsync_end: w, htotal, hskew: 0, vdisplay: h, vsync_start: h, vsync_end: h,
            vtotal, vscan: 0, vrefresh: mhz / 1000, flags: if interlaced { DRM_MODE_FLAG_INTERLACE } else { 0 },
            kind: if preferred { DRM_MODE_TYPE_PREFERRED } else { 0 }, name: [0; 32] } }
    }

    fn connector(id: u32, internal: bool, connected: bool, modes: Vec<Mode>, possible_crtcs: u32) -> Connector {
        Connector { id, name: format!("C-{id}"), internal, connected, modes, possible_crtcs, current_crtc: None }
    }

    #[test]
    fn the_ui_fits_the_tv_keeping_its_shape_centred() {
        let r = |x, y, width, height| Rect { x, y, width, height };
        assert_eq!(fit((1024, 768), (1920, 1080)), r(240, 0, 1440, 1080));
        assert_eq!(fit((1024, 768), (1280, 720)), r(160, 0, 960, 720));
        assert_eq!(fit((1024, 768), (1024, 768)), r(0, 0, 1024, 768));
        assert_eq!(fit((1024, 768), (1280, 1024)), r(0, 32, 1280, 960));
        assert_eq!(fit((1024, 768), (720, 480)), r(40, 0, 640, 480));
    }

    #[test]
    fn a_tv_gets_its_preferred_mode_up_to_1080p() {
        let p1080 = mode(1920, 1080, 60, false, false);
        // Its preferred mode, when progressive and within 1920x1080.
        assert_eq!(tv_mode(&[p1080, mode(1280, 720, 60, true, false)]).unwrap().width(), 1280);
        // A 4K preference gives the largest mode within 1080p, the one nearest 60 Hz.
        let modes = [mode(3840, 2160, 30, true, false), mode(1920, 1080, 50, false, false), p1080, mode(1920, 1080, 24, false, false), mode(1280, 720, 60, false, false)];
        assert_eq!(tv_mode(&modes), Some(p1080));
        // Never an interlaced one.
        let modes = [mode(1920, 1080, 60, true, true), mode(1280, 720, 50, false, false)];
        assert_eq!(tv_mode(&modes).unwrap().width(), 1280);
        assert_eq!(tv_mode(&[mode(3840, 2160, 60, true, false)]), None);
    }

    #[test]
    fn a_game_gets_the_mode_nearest_its_rate_at_the_same_size() {
        let usual = mode(1920, 1080, 60_000, true, false);
        let (m5994, m50) = (mode(1920, 1080, 59_940, false, false), mode(1920, 1080, 50_000, false, false));
        let modes = [usual, m5994, m50, mode(1920, 1080, 24_000, false, false), mode(1280, 720, 59_727, false, false), mode(1920, 1080, 59_727, false, true)];
        let near = |hz| nearest_mode(&modes, usual, hz, 0.005);
        assert_eq!(near(59.727), Some(m5994)); // GBA: not the 720p or the interlaced mode at its exact rate
        assert_eq!(near(60.0988), Some(usual)); // NES
        assert_eq!(near(50.007), Some(m50)); // PAL
        assert_eq!(near(30.0), None);
    }

    #[test]
    fn the_panel_is_the_main_output_and_a_connected_tv_takes_the_picture() {
        let crtcs = [31, 32];
        let panel = connector(1, true, true, vec![mode(1024, 768, 60, true, false)], 0b10);
        let hdmi = |connected| connector(2, false, connected, if connected { vec![mode(1920, 1080, 60, true, false)] } else { vec![] }, 0b11);
        // The panel comes first even after an external output in the card's list.
        let main = choose_main(&[hdmi(true), panel.clone()], &crtcs).unwrap();
        assert_eq!((main.name.as_str(), main.crtc, main.crtc_index, main.external), ("C-1", 32, 1, false));
        assert_eq!(choose_tv(&[panel.clone(), hdmi(false)], &crtcs, &main), None);
        // A TV avoids the panel's CRTC when it can.
        let tv = choose_tv(&[panel.clone(), hdmi(true)], &crtcs, &main).unwrap();
        assert_eq!((tv.name.as_str(), tv.crtc, tv.external, tv.mode.width()), ("C-2", 31, true, 1920));
        // The CRTC already driving it comes first.
        let driven = Connector { current_crtc: Some(32), ..hdmi(true) };
        assert_eq!(choose_tv(&[panel, driven], &crtcs, &main).unwrap().crtc, 32);
        // With no built-in panel, the external output is the main one, and nothing switches.
        let main = choose_main(&[hdmi(true)], &crtcs).unwrap();
        assert!(main.external);
        assert_eq!(choose_tv(&[hdmi(true)], &crtcs, &main), None);
        assert!(choose_main(&[hdmi(false)], &crtcs).is_err());
    }
}
