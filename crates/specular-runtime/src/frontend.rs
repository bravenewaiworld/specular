//! Specular's side of the libretro API: the callbacks a core calls. libretro passes them no
//! context, so the running game's state lives in a thread-local on the game's thread, where
//! every call into the core is made.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_char, c_uint, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering::Relaxed};

use crate::FrameSlot;
use crate::core::*;

/// What the callbacks work with.
pub struct Frontend {
    pub buttons: Arc<AtomicU16>,
    pub frames: Arc<FrameSlot>,
    pub on_frame: Box<dyn Fn() + Send>,
    pub pixel_format: c_uint,
    /// Samples made by the core during the current `retro_run`.
    pub samples: Vec<[i16; 2]>,
    /// Core options: their values, the defaults overridden by `overrides`.
    pub variables: HashMap<String, CString>,
    pub overrides: HashMap<String, String>,
    pub system_dir: CString,
    pub save_dir: CString,
    /// Set when the core makes the `ENV_SAVE_RAM_WRITTEN` call; the game's thread clears it after
    /// each `retro_run`.
    pub save_ram_written: bool,
    /// The environment calls refused so far, each logged once.
    pub refused: HashSet<c_uint>,
}

thread_local! {
    static FRONTEND: RefCell<Option<Frontend>> = const { RefCell::new(None) };
}

/// Makes `f` the state the callbacks on this thread use (None removes it).
pub fn install(f: Option<Frontend>) { FRONTEND.with(|s| *s.borrow_mut() = f); }

/// Runs `f` on the installed state; does nothing without one.
pub fn with<R>(f: impl FnOnce(&mut Frontend) -> R) -> Option<R> {
    FRONTEND.with(|s| s.borrow_mut().as_mut().map(f))
}

/// Gives a core all of the callbacks.
pub fn connect(core: &Core) {
    unsafe {
        (core.set_environment)(environment);
        (core.set_video_refresh)(video_refresh);
        (core.set_audio_sample)(audio_sample);
        (core.set_audio_sample_batch)(audio_batch);
        (core.set_input_poll)(input_poll);
        (core.set_input_state)(input_state);
    }
}

unsafe extern "C" fn environment(cmd: c_uint, data: *mut c_void) -> bool {
    with(|f| unsafe { f.environment(cmd, data) }).unwrap_or(false)
}

impl Frontend {
    /// Answers one environment call. Calls not listed are refused (false), which libretro cores
    /// must cope with; those accepted without doing anything only tell us things we don't use
    /// (but for input bitmasks, which `input_state` answers). That includes a change of timing or
    /// picture size while running: frames carry their own size, and the pacing and sound keep the
    /// timing the game started with.
    unsafe fn environment(&mut self, cmd: c_uint, data: *mut c_void) -> bool {
        unsafe {
            match cmd {
                ENV_SET_PIXEL_FORMAT => {
                    let format = *(data as *const c_uint);
                    let ok = matches!(format, PIXEL_0RGB1555 | PIXEL_XRGB8888 | PIXEL_RGB565);
                    if ok { self.pixel_format = format; }
                    ok
                }
                ENV_GET_SYSTEM_DIRECTORY => { *(data as *mut *const c_char) = self.system_dir.as_ptr(); true }
                ENV_GET_SAVE_DIRECTORY => { *(data as *mut *const c_char) = self.save_dir.as_ptr(); true }
                ENV_GET_CAN_DUPE => { *(data as *mut bool) = true; true }
                ENV_GET_LANGUAGE => { *(data as *mut c_uint) = 0; true } // English
                // Version 0: cores then declare their options with SET_VARIABLES, the simplest form.
                ENV_GET_CORE_OPTIONS_VERSION => { *(data as *mut c_uint) = 0; true }
                ENV_SET_VARIABLES => {
                    let mut v = data as *const Variable;
                    while !v.is_null() && !(*v).key.is_null() {
                        let key = CStr::from_ptr((*v).key).to_string_lossy().into_owned();
                        // "Description; default|other|…"
                        let spec = CStr::from_ptr((*v).value).to_string_lossy();
                        let default = spec.split_once("; ").map_or("", |(_, o)| o.split('|').next().unwrap_or(""));
                        let value = self.overrides.get(&key).map_or(default, |s| s.as_str());
                        self.variables.insert(key, CString::new(value).unwrap_or_default());
                        v = v.add(1);
                    }
                    true
                }
                ENV_GET_VARIABLE => {
                    let v = data as *mut Variable;
                    let key = CStr::from_ptr((*v).key).to_string_lossy();
                    match self.variables.get(key.as_ref()) {
                        Some(value) => { (*v).value = value.as_ptr(); true }
                        None => { (*v).value = std::ptr::null(); false }
                    }
                }
                ENV_GET_VARIABLE_UPDATE => { *(data as *mut bool) = false; true }
                ENV_SET_MESSAGE => {
                    let m = &*(data as *const Message);
                    if !m.msg.is_null() { eprintln!("core: {}", CStr::from_ptr(m.msg).to_string_lossy()); }
                    true
                }
                ENV_GET_LOG_INTERFACE => { (*(data as *mut LogCallback)).log = crate::log::specular_log; true }
                ENV_SAVE_RAM_WRITTEN => { self.save_ram_written = true; true }
                ENV_GET_INPUT_BITMASKS | ENV_SET_INPUT_DESCRIPTORS | ENV_SET_CONTROLLER_INFO | ENV_SET_MEMORY_MAPS
                | ENV_SET_SUPPORT_ACHIEVEMENTS | ENV_SET_SUPPORT_NO_GAME | ENV_SET_PERFORMANCE_LEVEL
                | ENV_SET_CORE_OPTIONS_DISPLAY | ENV_SET_MINIMUM_AUDIO_LATENCY | ENV_SET_SYSTEM_AV_INFO
                | ENV_SET_GEOMETRY => true,
                _ => {
                    // Logged as its number in libretro.h (without the RETRO_ENVIRONMENT_EXPERIMENTAL
                    // bit, 0x10000) and as the core passed it.
                    if self.refused.insert(cmd) { eprintln!("game: refused the core's environment call {} ({cmd:#x})", cmd & 0xffff); }
                    false
                }
            }
        }
    }
}

/// Converts the core's frame to RGBA and hands it to the UI. A null frame repeats the last one
/// (GET_CAN_DUPE), so nothing new is shown.
unsafe extern "C" fn video_refresh(data: *const c_void, width: c_uint, height: c_uint, pitch: usize) {
    if data.is_null() || width == 0 || height == 0 { return; }
    with(|f| {
        let mut frame = f.frames.spare();
        frame.width = width;
        frame.height = height;
        frame.rgba.clear();
        frame.rgba.reserve(width as usize * height as usize * 4);
        for y in 0..height as usize {
            let row = unsafe { (data as *const u8).add(y * pitch) };
            match f.pixel_format {
                PIXEL_XRGB8888 => {
                    let px = unsafe { std::slice::from_raw_parts(row as *const u32, width as usize) };
                    frame.rgba.extend(px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]));
                }
                format => {
                    let px = unsafe { std::slice::from_raw_parts(row as *const u16, width as usize) };
                    frame.rgba.extend(px.iter().flat_map(|&p| rgb16(p, format == PIXEL_RGB565)));
                }
            }
        }
        f.frames.publish(frame);
        (f.on_frame)();
    });
}

/// A 16-bit pixel (RGB565, or 0RGB1555 when `!rgb565`) as RGBA, scaling each channel to 8 bits.
fn rgb16(p: u16, rgb565: bool) -> [u8; 4] {
    // Each channel's top bits repeat into the low bits, so full scale maps to 255.
    let five = |c: u16| ((c << 3) | (c >> 2)) as u8;
    let six = |c: u16| ((c << 2) | (c >> 4)) as u8;
    if rgb565 { [five(p >> 11), six((p >> 5) & 0x3f), five(p & 0x1f), 255] }
    else { [five((p >> 10) & 0x1f), five((p >> 5) & 0x1f), five(p & 0x1f), 255] }
}

unsafe extern "C" fn audio_sample(left: i16, right: i16) { with(|f| f.samples.push([left, right])); }

unsafe extern "C" fn audio_batch(data: *const i16, frames: usize) -> usize {
    if data.is_null() { return 0; }
    with(|f| {
        let s = unsafe { std::slice::from_raw_parts(data as *const [i16; 2], frames) };
        f.samples.extend_from_slice(s);
    });
    frames
}

/// Buttons are read when the core asks (`input_state`), so as late as possible.
unsafe extern "C" fn input_poll() {}

unsafe extern "C" fn input_state(port: c_uint, device: c_uint, _index: c_uint, id: c_uint) -> i16 {
    if port != 0 || device != DEVICE_JOYPAD { return 0; }
    let bits = with(|f| f.buttons.load(Relaxed)).unwrap_or(0);
    match id {
        DEVICE_ID_JOYPAD_MASK => bits as i16,
        id if id < 16 => ((bits >> id) & 1) as i16,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sixteen_bit_pixels_scale_to_full_range() {
        assert_eq!(super::rgb16(0xffff, true), [255, 255, 255, 255]);
        assert_eq!(super::rgb16(0xf800, true), [255, 0, 0, 255]);
        assert_eq!(super::rgb16(0x7c00, false), [255, 0, 0, 255]);
        assert_eq!(super::rgb16(0, true), [0, 0, 0, 255]);
        assert_eq!(super::rgb16(0, false), [0, 0, 0, 255]);
    }
}
