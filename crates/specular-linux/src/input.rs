//! Buttons from Linux input devices (evdev), read without blocking inside an event loop. Devices
//! are found by name and reopened when they appear late or disappear (a device's input bridge
//! can start after Specular or restart).

use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

pub use specular_core::input::{Button, ButtonEvent};

/// The time on one of the kernel's clocks (e.g. CLOCK_MONOTONIC, the timeline of button event
/// times, or CLOCK_THREAD_CPUTIME_ID for the CPU time the calling thread has used).
pub fn clock(id: libc::clockid_t) -> Duration {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(id, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

// linux/input-event-codes.h
const EV_SYN: u16 = 0; const EV_KEY: u16 = 1; const EV_ABS: u16 = 3; const SYN_REPORT: u16 = 0; const SYN_DROPPED: u16 = 3;
const ABS_Z: u16 = 2; const ABS_RZ: u16 = 5; const ABS_HAT0X: u16 = 16; const ABS_HAT0Y: u16 = 17;

/// The d-pad's hat axes (X, Y) and the button for each end (-1, +1).
const HAT: [[Button; 2]; 2] = [[Button::Left, Button::Right], [Button::Up, Button::Down]];

/// Key codes of an Xbox 360 style pad (what most handhelds' input bridges present, e.g. the
/// Brick's padbridge) and the power key.
fn key(code: u16) -> Option<Button> {
    Some(match code {
        0x130 => Button::B, 0x131 => Button::A, 0x133 => Button::Y, 0x134 => Button::X, // BTN_SOUTH/EAST, Xbox Y/X
        0x136 => Button::L1, 0x137 => Button::R1,                                        // BTN_TL/TR
        0x13a => Button::Select, 0x13b => Button::Start, 0x13c => Button::Menu,          // BTN_SELECT/START/MODE
        0x13d => Button::F1, 0x13e => Button::F2,                                        // BTN_THUMBL/THUMBR
        115 => Button::VolUp, 114 => Button::VolDown, 116 => Button::Power,              // KEY_VOLUMEUP/DOWN, KEY_POWER
        _ => return None,
    })
}

/// One open input device and what is held on it.
struct Device { name: &'static str, file: Option<File>, held: Vec<Button>, hat: [i32; 2], dropping: bool }

/// The input devices Specular reads, by name.
pub struct Input { devices: Vec<Device>, next_retry: Instant }

const RETRY: Duration = Duration::from_secs(1);
const EVIOCSCLOCKID: libc::c_ulong = 0x4004_45a0; // _IOW('E', 0xa0, int)

fn open_by_name(name: &str) -> Option<File> {
    for entry in std::fs::read_dir("/sys/class/input").ok()?.flatten() {
        let node = entry.file_name();
        let Some(node) = node.to_str().filter(|n| n.starts_with("event")) else { continue };
        if !std::fs::read_to_string(entry.path().join("device/name")).is_ok_and(|s| s.trim() == name) { continue; }
        let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(format!("/dev/input/{node}")).ok()?;
        // Timestamp events on CLOCK_MONOTONIC (as `clock` reads it) instead of wall-clock time.
        if unsafe { libc::ioctl(file.as_raw_fd(), EVIOCSCLOCKID, &libc::CLOCK_MONOTONIC) } < 0 {
            eprintln!("input: {name}: can't use CLOCK_MONOTONIC timestamps");
        }
        eprintln!("input: {name} open");
        return Some(file);
    }
    None
}

impl Input {
    /// Opens the input devices with these names (as in /sys/class/input/*/device/name). Missing
    /// ones are not an error: `retry` opens them later.
    pub fn new(names: &[&'static str]) -> Self {
        let devices = names.iter().map(|&name| Device { name, file: None, held: Vec::new(), hat: [0; 2], dropping: false }).collect();
        let mut input = Self { devices, next_retry: Instant::now() };
        input.retry();
        input
    }

    /// Tries to open missing devices, at most once a second. Returns how long until the next try,
    /// or None if all are open.
    pub fn retry(&mut self) -> Option<Duration> {
        if self.devices.iter().all(|d| d.file.is_some()) { return None; }
        let now = Instant::now();
        if now >= self.next_retry {
            for d in self.devices.iter_mut().filter(|d| d.file.is_none()) { d.file = open_by_name(d.name); }
            self.next_retry = now + RETRY;
        }
        self.devices.iter().any(|d| d.file.is_none()).then(|| self.next_retry - now)
    }

    /// File descriptors of the open devices, for poll().
    pub fn fds(&self) -> impl Iterator<Item = RawFd> + '_ { self.devices.iter().filter_map(|d| d.file.as_ref().map(|f| f.as_raw_fd())) }

    /// Reads everything pending. A device that fails is closed, with its buttons released, and
    /// `retry` brings it back.
    pub fn read(&mut self, out: &mut Vec<ButtonEvent>) {
        for d in &mut self.devices {
            let mut buf = [0u8; 24]; // struct input_event on 64-bit: timeval (16), type, code, value
            while let Some(file) = d.file.as_mut() {
                match file.read(&mut buf) {
                    Ok(24) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    _ => { eprintln!("input: {} gone", d.name); d.file = None; d.release_all(out, Duration::ZERO); break; }
                }
                let n = |r: std::ops::Range<usize>| u64::from_ne_bytes(buf[r].try_into().unwrap());
                let time = Duration::new(n(0..8), n(8..16) as u32 * 1000);
                let kind = u16::from_ne_bytes([buf[16], buf[17]]);
                let code = u16::from_ne_bytes([buf[18], buf[19]]);
                let value = i32::from_ne_bytes(buf[20..24].try_into().unwrap());
                d.event(kind, code, value, time, out);
            }
        }
    }
}

impl Device {
    fn set(&mut self, button: Button, pressed: bool, time: Duration, out: &mut Vec<ButtonEvent>) {
        if self.held.contains(&button) == pressed { return; }
        if pressed { self.held.push(button) } else { self.held.retain(|&b| b != button) }
        out.push(ButtonEvent { button, pressed, time });
    }

    fn release_all(&mut self, out: &mut Vec<ButtonEvent>, time: Duration) {
        for b in std::mem::take(&mut self.held) { out.push(ButtonEvent { button: b, pressed: false, time }); }
        self.hat = [0; 2];
    }

    fn event(&mut self, kind: u16, code: u16, value: i32, time: Duration, out: &mut Vec<ButtonEvent>) {
        // Events were lost: release everything held (rather than leave a button stuck) and ignore
        // events up to the next report, as the kernel's evdev rules say.
        if self.dropping {
            if (kind, code) == (EV_SYN, SYN_REPORT) { self.dropping = false; }
            return;
        }
        match (kind, code) {
            (EV_SYN, SYN_DROPPED) => { self.release_all(out, time); self.dropping = true; }
            (EV_KEY, code) => if let Some(b) = key(code) { if value != 2 { self.set(b, value == 1, time, out) } },
            // The d-pad is a hat: -1, 0 or 1 per axis.
            (EV_ABS, c @ (ABS_HAT0X | ABS_HAT0Y)) => {
                let axis = (c - ABS_HAT0X) as usize;
                let old = std::mem::replace(&mut self.hat[axis], value);
                if old != 0 { self.set(HAT[axis][(old > 0) as usize], false, time, out); }
                if value != 0 { self.set(HAT[axis][(value > 0) as usize], true, time, out); }
            }
            // Triggers as axes (0–255 on Xbox style pads); past half is pressed.
            (EV_ABS, ABS_Z) => self.set(Button::L2, value > 127, time, out),
            (EV_ABS, ABS_RZ) => self.set(Button::R2, value > 127, time, out),
            _ => {}
        }
    }
}
