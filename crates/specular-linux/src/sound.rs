//! Game sound through ALSA's standard library, libasound.so.2, loaded when a game starts (as the
//! GPU libraries are), so Specular doesn't link to it and runs without it (silently).

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::io;

use specular_core::device::SoundOut;

/// The rate every game's sound is resampled to.
pub const RATE: u32 = 48_000;
/// How much sound ALSA queues (and so the most that can be waiting to play).
const LATENCY_US: c_uint = 64_000;

const SND_PCM_STREAM_PLAYBACK: c_int = 0;
const SND_PCM_NONBLOCK: c_int = 1;
const SND_PCM_FORMAT_S16_LE: c_int = 2;
const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;

/// The libasound functions used, looked up once per output.
struct Lib {
    handle: *mut c_void,
    open: unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int,
    set_params: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int,
    get_params: unsafe extern "C" fn(*mut c_void, *mut c_ulong, *mut c_ulong) -> c_int,
    writei: unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long,
    avail: unsafe extern "C" fn(*mut c_void) -> c_long,
    recover: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int,
    close: unsafe extern "C" fn(*mut c_void) -> c_int,
    strerror: unsafe extern "C" fn(c_int) -> *const c_char,
}

impl Lib {
    fn load() -> io::Result<Self> {
        let handle = unsafe { libc::dlopen(c"libasound.so.2".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() { return Err(io::Error::other("libasound.so.2 not found")); }
        let sym = |name: &CStr| {
            let p = unsafe { libc::dlsym(handle, name.as_ptr()) };
            if p.is_null() { Err(io::Error::other(format!("libasound has no {name:?}"))) } else { Ok(p) }
        };
        // SAFETY: each symbol is the libasound function of that name, whose C signature matches
        // the field's type.
        unsafe {
            Ok(Self {
                handle,
                open: std::mem::transmute(sym(c"snd_pcm_open")?),
                set_params: std::mem::transmute(sym(c"snd_pcm_set_params")?),
                get_params: std::mem::transmute(sym(c"snd_pcm_get_params")?),
                writei: std::mem::transmute(sym(c"snd_pcm_writei")?),
                avail: std::mem::transmute(sym(c"snd_pcm_avail")?),
                recover: std::mem::transmute(sym(c"snd_pcm_recover")?),
                close: std::mem::transmute(sym(c"snd_pcm_close")?),
                strerror: std::mem::transmute(sym(c"snd_strerror")?),
            })
        }
    }

    fn error(&self, what: &str, code: c_int) -> io::Error {
        let text = unsafe { CStr::from_ptr((self.strerror)(code)) }.to_string_lossy();
        io::Error::other(format!("ALSA {what}: {text}"))
    }
}

/// A playback stream on an ALSA device, 16-bit stereo at `RATE`.
pub struct Alsa { lib: Lib, pcm: *mut c_void, capacity: usize }

// SAFETY: the stream is used from one thread at a time (SoundOut takes &mut self).
unsafe impl Send for Alsa {}

impl Alsa {
    /// Opens the ALSA device `name` ("default" for the system's choice).
    pub fn open(name: &str) -> io::Result<Self> {
        let lib = Lib::load()?;
        let name = CString::new(name).map_err(io::Error::other)?;
        let mut pcm = std::ptr::null_mut();
        let r = unsafe { (lib.open)(&mut pcm, name.as_ptr(), SND_PCM_STREAM_PLAYBACK, SND_PCM_NONBLOCK) };
        if r < 0 { return Err(lib.error("open", r)); }
        let mut out = Self { lib, pcm, capacity: 0 };
        // Soft resampling on: if the card can't do RATE itself, ALSA converts.
        let r = unsafe { (out.lib.set_params)(pcm, SND_PCM_FORMAT_S16_LE, SND_PCM_ACCESS_RW_INTERLEAVED, 2, RATE, 1, LATENCY_US) };
        if r < 0 { return Err(out.lib.error("set_params", r)); }
        let (mut buffer, mut period) = (0, 0);
        let r = unsafe { (out.lib.get_params)(pcm, &mut buffer, &mut period) };
        if r < 0 { return Err(out.lib.error("get_params", r)); }
        out.capacity = buffer as usize;
        Ok(out)
    }

    /// Recovers from an underrun or a resume after sleep; other errors are returned.
    fn recover(&mut self, code: c_long) -> io::Result<()> {
        let r = unsafe { (self.lib.recover)(self.pcm, code as c_int, 1) };
        if r < 0 { Err(self.lib.error("recover", r)) } else { Ok(()) }
    }
}

impl SoundOut for Alsa {
    fn rate(&self) -> u32 { RATE }

    fn write(&mut self, frames: &[[i16; 2]]) -> io::Result<usize> {
        if frames.is_empty() { return Ok(0); }
        loop {
            let n = unsafe { (self.lib.writei)(self.pcm, frames.as_ptr().cast(), frames.len() as c_ulong) };
            match n {
                n if n >= 0 => return Ok(n as usize),
                n if n == -(libc::EAGAIN as c_long) => return Ok(0),
                n => self.recover(n)?,
            }
        }
    }

    fn queued(&mut self) -> usize {
        let avail = unsafe { (self.lib.avail)(self.pcm) };
        // An underrun: nothing is queued, and the next write recovers.
        if avail < 0 { return 0; }
        self.capacity.saturating_sub(avail as usize)
    }

    fn capacity(&self) -> usize { self.capacity }
}

impl Drop for Alsa {
    fn drop(&mut self) {
        unsafe {
            (self.lib.close)(self.pcm);
            libc::dlclose(self.lib.handle);
        }
    }
}
