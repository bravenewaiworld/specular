//! The running core's log messages (libretro's log interface), in Specular's log as `core: <core>
//! <level>: <message>`. Debug messages are dropped, and at most `MAX_PER_SECOND` are written a
//! second, so a core that logs every frame can't flood the log or slow the game: the rest are
//! counted, and the count logged. The state is global rather than the game thread's: cores may
//! log from threads of their own.

use std::ffi::{CStr, c_char, c_uint};
use std::io::Write;
use std::sync::{Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};

unsafe extern "C" {
    fn specular_log_to(sink: unsafe extern "C" fn(c_uint, *const c_char));
    /// The callback cores are given (`src/log.c`), libretro's `retro_log_printf_t`.
    pub fn specular_log(level: c_uint, fmt: *const c_char, ...);
}

const MAX_PER_SECOND: u32 = 30;

struct CoreLog { core: String, since: Instant, written: u32, dropped: u32 }

impl CoreLog {
    fn report_dropped(&self) {
        if self.dropped > 0 { write(format_args!("core: {}: {} more messages dropped (over {MAX_PER_SECOND} a second)", self.core, self.dropped)); }
    }
}

static LOG: Mutex<Option<CoreLog>> = Mutex::new(None);

fn lock() -> MutexGuard<'static, Option<CoreLog>> { LOG.lock().unwrap_or_else(|e| e.into_inner()) }

/// Messages are from the core called `core` (`SystemInfo::name`) until `stop`.
pub fn start(core: String) {
    static SINK: Once = Once::new();
    SINK.call_once(|| unsafe { specular_log_to(sink) });
    *lock() = Some(CoreLog { core, since: Instant::now(), written: 0, dropped: 0 });
}

/// The game has ended: logs how many of its last messages were dropped, if any.
pub fn stop() { if let Some(l) = lock().take() { l.report_dropped(); } }

/// Never panics (it is called from C): a failed write is ignored.
fn write(line: std::fmt::Arguments) { let _ = writeln!(std::io::stderr(), "{line}"); }

unsafe extern "C" fn sink(level: c_uint, line: *const c_char) {
    let text = unsafe { CStr::from_ptr(line) }.to_string_lossy();
    let text = text.trim_end_matches(['\n', '\r']);
    let level = match level { 1 => "info", 2 => "warn", 3 => "error", _ => "?" };
    let mut log = lock();
    // A core's own thread may log after its game has ended.
    let Some(l) = log.as_mut() else { return write(format_args!("core: {level}: {text}")) };
    if l.since.elapsed() >= Duration::from_secs(1) {
        l.report_dropped();
        (l.since, l.written, l.dropped) = (Instant::now(), 0, 0);
    }
    if l.written == MAX_PER_SECOND { l.dropped += 1; return; }
    l.written += 1;
    write(format_args!("core: {} {level}: {text}", l.core));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_core_logging_every_frame_is_capped_and_debug_messages_dropped() {
        let _turn = crate::tests::ONE_GAME.lock().unwrap_or_else(|e| e.into_inner());
        start("Test".into());
        for frame in 0..50 { unsafe { specular_log(2, c"frame %d\n".as_ptr(), frame as std::ffi::c_int) }; }
        unsafe { specular_log(0, c"debug".as_ptr()) };
        let counts = lock().as_ref().map(|l| (l.written, l.dropped));
        assert_eq!(counts, Some((MAX_PER_SECOND, 50 - MAX_PER_SECOND)));
        stop();
    }
}
