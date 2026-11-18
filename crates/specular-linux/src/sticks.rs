//! Analog sticks from a Linux joystick device (the left stick on `ABS_X`/`ABS_Y`, the right on
//! `ABS_RX`/`ABS_RY`, as kernel drivers such as `adc-joystick` report them): their positions for
//! games, and the left stick as a d-pad for the menus (`Input::menu_stick`). The device is found by
//! name and reopened when it goes away.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use specular_core::device::Sticks;

use crate::input::{DEV, EVENT_SIZE, SYSFS, parse_event};

const EV_ABS: u16 = 3;
/// `ABS_X`, `ABS_Y`, `ABS_RX`, `ABS_RY`: the order of `EvdevSticks::read`'s positions.
const AXES: [u16; 4] = [0, 1, 3, 4];
/// The share of each stick's travel around its centre that reads as centred: sticks rarely rest
/// exactly at the middle of their range.
const DEAD_ZONE: f64 = 0.10;
const RETRY: Duration = Duration::from_secs(1);

/// `EVIOCGABS(abs)`: `_IOR('E', 0x40 + abs, struct input_absinfo)` (six 32-bit values).
fn eviocgabs(axis: u16) -> libc::c_ulong { 0x8018_4540 + axis as libc::c_ulong }

/// One axis: its range as the driver gives it, and its last value.
#[derive(Clone, Copy, Default)]
struct Axis { min: i32, max: i32, value: i32 }

impl Axis {
    /// The position from -1 (min) to 1 (max), 0 at the middle of the range.
    fn position(&self) -> f64 {
        if self.max <= self.min { return 0.0; }
        let mid = (self.min as f64 + self.max as f64) / 2.0;
        ((self.value as f64 - mid) / ((self.max - self.min) as f64 / 2.0)).clamp(-1.0, 1.0)
    }
}

/// Both sticks of one joystick device, read without blocking.
pub struct EvdevSticks { name: &'static str, file: Option<File>, axes: [Axis; 4], next_retry: Instant }

impl EvdevSticks {
    /// The joystick device of this name (as /sys/class/input/*/device/name has it). Nothing is open
    /// until the first `read` or `retry`.
    pub fn new(name: &'static str) -> Self { Self { name, file: None, axes: [Axis::default(); 4], next_retry: Instant::now() } }

    /// Opens the device if it is missing, at most once a second. Returns how long until the next
    /// try, or None if it is open.
    pub fn retry(&mut self) -> Option<Duration> {
        if self.file.is_some() { return None; }
        let now = Instant::now();
        if now >= self.next_retry {
            self.open();
            self.next_retry = now + RETRY;
        }
        self.file.is_none().then(|| self.next_retry - now)
    }

    fn open(&mut self) {
        let Some(node) = std::fs::read_dir(SYSFS).ok().and_then(|entries| entries.flatten().map(|e| e.path())
            .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("event"))
                && std::fs::read_to_string(p.join("device/name")).is_ok_and(|n| n.trim() == self.name)))
        else { return };
        let path = Path::new(DEV).join(node.file_name().unwrap_or_default());
        let file = match OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&path) {
            Ok(f) => f,
            Err(e) => { eprintln!("sticks: {} ({}): {e}", self.name, path.display()); return; }
        };
        for (axis, &code) in self.axes.iter_mut().zip(&AXES) {
            let mut info = [0i32; 6]; // value, minimum, maximum, fuzz, flat, resolution
            if unsafe { libc::ioctl(file.as_raw_fd(), eviocgabs(code), info.as_mut_ptr()) } < 0 { *axis = Axis::default(); continue; }
            *axis = Axis { value: info[0], min: info[1], max: info[2] };
        }
        eprintln!("sticks: {} open, ranges {:?}", self.name, self.axes.map(|a| (a.min, a.max)));
        self.file = Some(file);
    }

    /// The file descriptor, for poll(), while open.
    pub fn fd(&self) -> Option<RawFd> { self.file.as_ref().map(|f| f.as_raw_fd()) }

    /// Reads every pending event. Returns whether any axis moved. A device that fails is closed,
    /// its sticks centred, and `retry` brings it back.
    pub fn drain(&mut self) -> bool {
        let mut moved = false;
        let mut buf = [0u8; EVENT_SIZE];
        while let Some(file) = self.file.as_mut() {
            match file.read(&mut buf) {
                Ok(EVENT_SIZE) => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                _ => {
                    eprintln!("sticks: {} gone", self.name);
                    self.file = None;
                    for a in &mut self.axes { a.value = (a.min + a.max) / 2; }
                    return true;
                }
            }
            let (_, kind, code, value) = parse_event(&buf);
            if kind != EV_ABS { continue; }
            if let Some(i) = AXES.iter().position(|&c| c == code) { self.axes[i].value = value; moved = true; }
        }
        moved
    }

    /// The left stick's position, each axis from -1 to 1 (right and down positive), past the dead
    /// zone.
    pub fn left(&self) -> (f64, f64) { stick(self.axes[0].position(), self.axes[1].position()) }

    /// Both sticks' positions as libretro takes them: -32768 to 32767 per axis (left X, left Y,
    /// right X, right Y).
    pub fn positions(&self) -> [i16; 4] {
        let (lx, ly) = stick(self.axes[0].position(), self.axes[1].position());
        let (rx, ry) = stick(self.axes[2].position(), self.axes[3].position());
        [lx, ly, rx, ry].map(|v| (v * 32767.0).round() as i16)
    }
}

impl Sticks for EvdevSticks {
    fn read(&mut self) -> [i16; 4] {
        self.retry();
        self.drain();
        self.positions()
    }
}

/// A stick's position with its dead zone taken out: centred within `DEAD_ZONE` of the middle, and
/// scaled so the rest of the travel still reaches 1.
fn stick(x: f64, y: f64) -> (f64, f64) {
    let r = x.hypot(y);
    if r <= DEAD_ZONE { return (0.0, 0.0); }
    let scale = ((r - DEAD_ZONE) / (1.0 - DEAD_ZONE)).min(1.0) / r;
    (x * scale, y * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_span_the_range_with_a_dead_zone() {
        let axis = |value| Axis { min: 250, max: 800, value };
        assert_eq!(axis(525).position(), 0.0);
        assert_eq!(axis(800).position(), 1.0);
        assert_eq!(axis(250).position(), -1.0);
        assert_eq!(stick(0.05, -0.05), (0.0, 0.0));
        assert_eq!(stick(1.0, 0.0), (1.0, 0.0));
        let (x, _) = stick(0.55, 0.0);
        assert!((x - 0.5).abs() < 1e-9, "{x}");
    }
}
