//! A libretro core, loaded from its shared library: the C types and constants of libretro.h that
//! Specular uses, and the core's functions.

use std::ffi::{CStr, CString, c_char, c_uint, c_void};
use std::path::Path;

pub const ENV_GET_CAN_DUPE: c_uint = 3;
pub const ENV_SET_MESSAGE: c_uint = 6;
pub const ENV_SET_PERFORMANCE_LEVEL: c_uint = 8;
pub const ENV_GET_SYSTEM_DIRECTORY: c_uint = 9;
pub const ENV_SET_PIXEL_FORMAT: c_uint = 10;
pub const ENV_SET_INPUT_DESCRIPTORS: c_uint = 11;
pub const ENV_SET_DISK_CONTROL_INTERFACE: c_uint = 13;
pub const ENV_GET_VARIABLE: c_uint = 15;
pub const ENV_SET_VARIABLES: c_uint = 16;
pub const ENV_GET_VARIABLE_UPDATE: c_uint = 17;
pub const ENV_SET_SUPPORT_NO_GAME: c_uint = 18;
pub const ENV_GET_RUMBLE_INTERFACE: c_uint = 23;
pub const ENV_GET_LOG_INTERFACE: c_uint = 27;
pub const ENV_GET_SAVE_DIRECTORY: c_uint = 31;
pub const ENV_SET_SYSTEM_AV_INFO: c_uint = 32;
pub const ENV_SET_CONTROLLER_INFO: c_uint = 35;
pub const ENV_SET_MEMORY_MAPS: c_uint = 36 | EXPERIMENTAL;
pub const ENV_SET_GEOMETRY: c_uint = 37;
pub const ENV_GET_LANGUAGE: c_uint = 39;
pub const ENV_SET_SUPPORT_ACHIEVEMENTS: c_uint = 42 | EXPERIMENTAL;
pub const ENV_GET_INPUT_BITMASKS: c_uint = 51 | EXPERIMENTAL;
pub const ENV_GET_CORE_OPTIONS_VERSION: c_uint = 52;
pub const ENV_SET_CORE_OPTIONS: c_uint = 53;
pub const ENV_SET_CORE_OPTIONS_INTL: c_uint = 54;
pub const ENV_SET_CORE_OPTIONS_DISPLAY: c_uint = 55;
pub const ENV_GET_DISK_CONTROL_INTERFACE_VERSION: c_uint = 57;
pub const ENV_SET_DISK_CONTROL_EXT_INTERFACE: c_uint = 58;
pub const ENV_SET_MINIMUM_AUDIO_LATENCY: c_uint = 63;
pub const ENV_SET_CORE_OPTIONS_V2: c_uint = 67;
pub const ENV_SET_CORE_OPTIONS_V2_INTL: c_uint = 68;
/// A call of Specular's own, from our patched mGBA (`tools/core-patches/mgba/0002`): the game
/// has changed its save memory and finished writing to it. Made inside `retro_run`, once the
/// save memory has gone 15 frames (a quarter of a second) without a write. No data.
pub const ENV_SAVE_RAM_WRITTEN: c_uint = PRIVATE + 1;
/// A call of Specular's own, from our patched SwanStation (`tools/core-patches/swanstation/0001`):
/// its built-in OpenBIOS, used when it finds no BIOS file, has halted the game for good: on a BIOS
/// function it lacks, a change to its code (by the game) it doesn't recognise, or its fatal error.
/// Made only with a disc in (with none, the disc image didn't open: not a BIOS matter), inside
/// `retro_run`, once. No data.
pub const ENV_BIOS_HALTED: c_uint = PRIVATE + 2;
const EXPERIMENTAL: c_uint = 0x10000;
/// The start of the range libretro leaves to private agreements between a frontend and its cores.
const PRIVATE: c_uint = 0x20000;

pub const DEVICE_JOYPAD: c_uint = 1;
/// Asks for every joypad button at once, as a bit mask.
pub const DEVICE_ID_JOYPAD_MASK: c_uint = 256;
pub const MEMORY_SAVE_RAM: c_uint = 0;

/// `retro_rumble_effect`: which motor `set_rumble_state` sets.
pub const RUMBLE_STRONG: c_uint = 0;
pub const RUMBLE_WEAK: c_uint = 1;

pub const PIXEL_0RGB1555: c_uint = 0;
pub const PIXEL_XRGB8888: c_uint = 1;
pub const PIXEL_RGB565: c_uint = 2;

#[repr(C)]
pub struct GameInfo { pub path: *const c_char, pub data: *const c_void, pub size: usize, pub meta: *const c_char }

#[repr(C)]
pub struct SystemInfo {
    pub library_name: *const c_char,
    pub library_version: *const c_char,
    pub valid_extensions: *const c_char,
    pub need_fullpath: bool,
    pub block_extract: bool,
}

impl SystemInfo {
    /// What the core calls itself, e.g. "mGBA", or "the core" if it gives no name.
    pub fn name(&self) -> String { text(self.library_name).unwrap_or_else(|| "the core".into()) }
    /// The extensions of the files it plays, in lower case.
    pub fn extensions(&self) -> Vec<String> {
        text(self.valid_extensions).map_or(vec![], |e| e.split('|').map(str::to_ascii_lowercase).collect())
    }
}

/// A C string from a core, if it isn't null.
pub fn text(p: *const c_char) -> Option<String> { (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()) }

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Geometry { pub base_width: c_uint, pub base_height: c_uint, pub max_width: c_uint, pub max_height: c_uint, pub aspect_ratio: f32 }

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Timing { pub fps: f64, pub sample_rate: f64 }

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AvInfo { pub geometry: Geometry, pub timing: Timing }

#[repr(C)]
pub struct Variable { pub key: *const c_char, pub value: *const c_char }

/// The most values a core option has in versions 1 and 2 of libretro's core options interface (the
/// arrays end early with a null value).
pub const OPTION_VALUES_MAX: usize = 128;

#[repr(C)]
pub struct OptionValue { pub value: *const c_char, pub label: *const c_char }

/// Version 1's option (`retro_core_option_definition`); an array of them ends with a null key.
#[repr(C)]
pub struct OptionDefinition {
    pub key: *const c_char, pub desc: *const c_char, pub info: *const c_char,
    pub values: [OptionValue; OPTION_VALUES_MAX], pub default_value: *const c_char,
}

/// Version 1's options in the user's language, and in English (`retro_core_options_intl`).
#[repr(C)]
pub struct OptionsIntl { pub us: *const OptionDefinition, pub local: *const OptionDefinition }

#[repr(C)]
pub struct OptionCategory { pub key: *const c_char, pub desc: *const c_char, pub info: *const c_char }

/// Version 2's option (`retro_core_option_v2_definition`); an array of them ends with a null key.
#[repr(C)]
pub struct OptionV2Definition {
    pub key: *const c_char, pub desc: *const c_char, pub desc_categorized: *const c_char,
    pub info: *const c_char, pub info_categorized: *const c_char, pub category_key: *const c_char,
    pub values: [OptionValue; OPTION_VALUES_MAX], pub default_value: *const c_char,
}

/// Version 2's options: arrays ending with a null key.
#[repr(C)]
pub struct OptionsV2 { pub categories: *const OptionCategory, pub definitions: *const OptionV2Definition }

#[repr(C)]
pub struct OptionsV2Intl { pub us: *const OptionsV2, pub local: *const OptionsV2 }

#[repr(C)]
pub struct Message { pub msg: *const c_char, pub frames: c_uint }

#[repr(C)]
pub struct RumbleInterface { pub set_rumble_state: unsafe extern "C" fn(c_uint, c_uint, u16) -> bool }

/// libretro's disk control interface (`retro_disk_control_callback`), for games on several discs;
/// see the `discs` module. Called on the game's thread only, like the rest of the core.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DiskControlBasic {
    pub set_eject_state: Option<unsafe extern "C" fn(bool) -> bool>,
    pub get_eject_state: Option<unsafe extern "C" fn() -> bool>,
    pub get_image_index: Option<unsafe extern "C" fn() -> c_uint>,
    pub set_image_index: Option<unsafe extern "C" fn(c_uint) -> bool>,
    pub get_num_images: Option<unsafe extern "C" fn() -> c_uint>,
    pub replace_image_index: Option<unsafe extern "C" fn(c_uint, *const GameInfo) -> bool>,
    pub add_image_index: Option<unsafe extern "C" fn() -> bool>,
}

/// Its extended version (`retro_disk_control_ext_callback`): the same functions, then these.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DiskControl {
    pub basic: DiskControlBasic,
    pub set_initial_image: Option<unsafe extern "C" fn(c_uint, *const c_char) -> bool>,
    pub get_image_path: Option<unsafe extern "C" fn(c_uint, *mut c_char, usize) -> bool>,
    pub get_image_label: Option<unsafe extern "C" fn(c_uint, *mut c_char, usize) -> bool>,
}

#[repr(C)]
pub struct LogCallback { pub log: unsafe extern "C" fn(c_uint, *const c_char, ...) }

pub type EnvironmentFn = unsafe extern "C" fn(c_uint, *mut c_void) -> bool;
pub type VideoRefreshFn = unsafe extern "C" fn(*const c_void, c_uint, c_uint, usize);
pub type AudioSampleFn = unsafe extern "C" fn(i16, i16);
pub type AudioBatchFn = unsafe extern "C" fn(*const i16, usize) -> usize;
pub type InputPollFn = unsafe extern "C" fn();
pub type InputStateFn = unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint) -> i16;

/// A library opened with `dlopen`, closed when dropped.
struct Library(*mut c_void);

impl Drop for Library {
    fn drop(&mut self) { unsafe { libc::dlclose(self.0); } }
}

/// The functions every libretro core exports.
pub struct Core {
    _lib: Library,
    pub api_version: unsafe extern "C" fn() -> c_uint,
    pub init: unsafe extern "C" fn(),
    pub deinit: unsafe extern "C" fn(),
    pub set_environment: unsafe extern "C" fn(EnvironmentFn),
    pub set_video_refresh: unsafe extern "C" fn(VideoRefreshFn),
    pub set_audio_sample: unsafe extern "C" fn(AudioSampleFn),
    pub set_audio_sample_batch: unsafe extern "C" fn(AudioBatchFn),
    pub set_input_poll: unsafe extern "C" fn(InputPollFn),
    pub set_input_state: unsafe extern "C" fn(InputStateFn),
    pub get_system_info: unsafe extern "C" fn(*mut SystemInfo),
    pub get_system_av_info: unsafe extern "C" fn(*mut AvInfo),
    pub reset: unsafe extern "C" fn(),
    pub run: unsafe extern "C" fn(),
    pub serialize_size: unsafe extern "C" fn() -> usize,
    pub serialize: unsafe extern "C" fn(*mut c_void, usize) -> bool,
    pub unserialize: unsafe extern "C" fn(*const c_void, usize) -> bool,
    pub load_game: unsafe extern "C" fn(*const GameInfo) -> bool,
    pub unload_game: unsafe extern "C" fn(),
    pub get_memory_data: unsafe extern "C" fn(c_uint) -> *mut c_void,
    pub get_memory_size: unsafe extern "C" fn(c_uint) -> usize,
}

impl Core {
    /// Loads the core at `path`. Its library stays loaded until the `Core` is dropped.
    pub fn load(path: &Path) -> Result<Self, String> {
        let c_path = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
        let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            let why = unsafe { CStr::from_ptr(libc::dlerror()) }.to_string_lossy().into_owned();
            return Err(format!("loading {}: {why}", path.display()));
        }
        let lib = Library(handle);
        let sym = |name: &CStr| {
            let p = unsafe { libc::dlsym(lib.0, name.as_ptr()) };
            if p.is_null() { Err(format!("{} has no {name:?}", path.display())) } else { Ok(p) }
        };
        // SAFETY: each symbol is the libretro function of that name, whose C signature (from
        // libretro.h) matches the field's type.
        let core = unsafe {
            use std::mem::transmute as t;
            Core {
                api_version: t(sym(c"retro_api_version")?),
                init: t(sym(c"retro_init")?),
                deinit: t(sym(c"retro_deinit")?),
                set_environment: t(sym(c"retro_set_environment")?),
                set_video_refresh: t(sym(c"retro_set_video_refresh")?),
                set_audio_sample: t(sym(c"retro_set_audio_sample")?),
                set_audio_sample_batch: t(sym(c"retro_set_audio_sample_batch")?),
                set_input_poll: t(sym(c"retro_set_input_poll")?),
                set_input_state: t(sym(c"retro_set_input_state")?),
                get_system_info: t(sym(c"retro_get_system_info")?),
                get_system_av_info: t(sym(c"retro_get_system_av_info")?),
                reset: t(sym(c"retro_reset")?),
                run: t(sym(c"retro_run")?),
                serialize_size: t(sym(c"retro_serialize_size")?),
                serialize: t(sym(c"retro_serialize")?),
                unserialize: t(sym(c"retro_unserialize")?),
                load_game: t(sym(c"retro_load_game")?),
                unload_game: t(sym(c"retro_unload_game")?),
                get_memory_data: t(sym(c"retro_get_memory_data")?),
                get_memory_size: t(sym(c"retro_get_memory_size")?),
                _lib: lib,
            }
        };
        let version = unsafe { (core.api_version)() };
        if version != 1 { return Err(format!("{}: libretro API version {version}, not 1", path.display())); }
        Ok(core)
    }

    /// The core's battery-backed memory (its `.srm`), as it is now; empty if it has none.
    pub fn save_ram(&mut self) -> &mut [u8] {
        let (data, size) = unsafe { ((self.get_memory_data)(MEMORY_SAVE_RAM), (self.get_memory_size)(MEMORY_SAVE_RAM)) };
        if data.is_null() || size == 0 { return &mut []; }
        unsafe { std::slice::from_raw_parts_mut(data.cast(), size) }
    }
}
