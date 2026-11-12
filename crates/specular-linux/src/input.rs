//! Buttons from Linux input devices (evdev), read without blocking inside an event loop. Devices
//! are found by name and reopened when they appear late or disappear (the program that presents
//! a handheld's buttons as a standard gamepad, e.g. the Brick's padbridge, can start after
//! Specular or restart). The mute switch is reported as the button `Mute`, held while it is on.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
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
pub(crate) const EV_SYN: u16 = 0; const EV_KEY: u16 = 1; const EV_ABS: u16 = 3; pub(crate) const SYN_REPORT: u16 = 0; pub(crate) const SYN_DROPPED: u16 = 3;
pub(crate) const EV_SW: u16 = 5; const SW_TABLET_MODE: u16 = 1; // how the Brick's gamepad program (padbridge) reports its FN slider, which Specular uses as the mute switch
const ABS_Z: u16 = 2; const ABS_RZ: u16 = 5; const ABS_HAT0X: u16 = 16; const ABS_HAT0Y: u16 = 17;

/// The d-pad's hat axes (X, Y) and the button for each end (-1, +1).
const HAT: [[Button; 2]; 2] = [[Button::Left, Button::Right], [Button::Up, Button::Down]];

/// Key codes of an Xbox 360 style pad (what most handhelds' gamepad programs present, e.g. the
/// Brick's padbridge), the keys a handheld's own `gpio-keys` report instead of its hat and trigger
/// axes, and the volume and power keys. Specular names buttons by position, so the Xbox pad's
/// bottom button (its "A") is Specular's B, its left one ("X") Specular's Y, and so on. Such pads
/// send 0x133 for the left button and 0x134 for the top one, as SDL's Xbox mapping reads them; a
/// kernel driver following Documentation/input/gamepad.rst sends them the other way round
/// (`Input::positional`).
fn key(code: u16) -> Option<Button> {
    Some(match code {
        0x130 => Button::B, 0x131 => Button::A, 0x133 => Button::Y, 0x134 => Button::X, // Xbox A, B, X, Y
        0x136 => Button::L1, 0x137 => Button::R1,                                        // BTN_TL/TR
        0x138 => Button::L2, 0x139 => Button::R2,                                        // BTN_TL2/TR2 (triggers as keys)
        0x13a => Button::Select, 0x13b => Button::Start, 0x13c => Button::Menu,          // BTN_SELECT/START/MODE
        0x13d => Button::F1, 0x13e => Button::F2,                                        // BTN_THUMBL/THUMBR
        0x220 => Button::Up, 0x221 => Button::Down, 0x222 => Button::Left, 0x223 => Button::Right, // BTN_DPAD_* (a d-pad as keys)
        115 => Button::VolUp, 114 => Button::VolDown, 116 => Button::Power,              // KEY_VOLUMEUP/DOWN, KEY_POWER
        _ => return None,
    })
}

/// One input device, and what is held on it.
struct InputDevice { name: &'static str, file: Option<File>, held: Vec<Button>, hat: [i32; 2], dropping: bool }

/// The input devices Specular reads, by name.
pub struct Input { devices: Vec<InputDevice>, next_retry: Instant, positional: bool, menu_chord: Option<Chord>, menu_stick: Option<MenuStick> }

/// The left stick as a d-pad for the menus (`Input::menu_stick`), and the direction it holds on each
/// axis (X, Y).
struct MenuStick { sticks: crate::sticks::EvdevSticks, held: [Option<Button>; 2] }

/// How far the left stick must go to press a direction in the menus, and how far back to release
/// it: apart, so a stick held near the edge doesn't flicker.
const STICK_PRESS: f64 = 0.5;
const STICK_RELEASE: f64 = 0.3;

/// Whether Select and Start are held (on any device), and so Menu is (`Input::menu_chord`).
#[derive(Default)]
struct Chord { select: bool, start: bool, menu: bool }

const RETRY: Duration = Duration::from_secs(1);
const EVIOCSCLOCKID: libc::c_ulong = 0x4004_45a0; // _IOW('E', 0xa0, int)
const EVIOCGSW_4: libc::c_ulong = 0x8004_451b; // _IOC(_IOC_READ, 'E', 0x1b, 4): the switches' states, as 4 bytes of bits

/// The size of a `struct input_event` on 64-bit Linux: a timeval (16 bytes), type, code, value.
pub(crate) const EVENT_SIZE: usize = 24;

/// A `struct input_event`'s time, type, code and value.
pub(crate) fn parse_event(buf: &[u8; EVENT_SIZE]) -> (Duration, u16, u16, i32) {
    let n = |r: std::ops::Range<usize>| u64::from_ne_bytes(buf[r].try_into().unwrap());
    let time = Duration::new(n(0..8), n(8..16) as u32 * 1000);
    (time, u16::from_ne_bytes([buf[16], buf[17]]), u16::from_ne_bytes([buf[18], buf[19]]), i32::from_ne_bytes(buf[20..24].try_into().unwrap()))
}

/// A `struct input_event` of this type, code and value, at time 0 (a device written to ignores
/// the time).
pub(crate) fn make_event(kind: u16, code: u16, value: i32) -> [u8; EVENT_SIZE] {
    let mut buf = [0u8; EVENT_SIZE];
    buf[16..18].copy_from_slice(&kind.to_ne_bytes());
    buf[18..20].copy_from_slice(&code.to_ne_bytes());
    buf[20..24].copy_from_slice(&value.to_ne_bytes());
    buf
}

/// The positions of an input device's first 32 switches (bit n is switch n), or None if they
/// can't be read.
pub(crate) fn switches(file: &File) -> Option<u32> {
    let mut bits = [0u8; 4];
    (unsafe { libc::ioctl(file.as_raw_fd(), EVIOCGSW_4, bits.as_mut_ptr()) } >= 0).then(|| u32::from_le_bytes(bits))
}

/// Where the kernel describes its input devices.
pub(crate) const SYSFS: &str = "/sys/class/input";
/// Where their device nodes are.
pub(crate) const DEV: &str = "/dev/input";

/// An event device as sysfs describes it, for finding one by its name or what it can do.
pub(crate) struct EventDevice {
    /// Its node, e.g. "event3" (under `DEV`).
    node: String,
    /// Its name, or the node's if it has none.
    pub(crate) name: String,
    /// The folder holding its `capabilities/`.
    info: PathBuf,
}

impl EventDevice {
    /// Whether its capability bitmap `kind` ("ev", "key", "sw", "ff", …) has bit `bit`.
    pub fn has(&self, kind: &str, bit: u16) -> bool {
        std::fs::read_to_string(self.info.join("capabilities").join(kind)).is_ok_and(|caps| has_bit(&caps, bit))
    }
}

/// The first event device in `sysfs` that `wanted` accepts and that opens (from `dev`, with
/// `options`): its name and file. Logs it, or each that doesn't open (and is skipped), as `what`
/// (the log line's start, e.g. "rumble").
pub(crate) fn open_first(sysfs: &Path, dev: &Path, options: &OpenOptions, what: &str, wanted: impl Fn(&EventDevice) -> bool) -> Option<(String, File)> {
    for d in event_devices(sysfs).into_iter().filter(wanted) {
        let path = dev.join(&d.node);
        match options.open(&path) {
            Ok(file) => { eprintln!("{what}: {} ({})", d.name, path.display()); return Some((d.name, file)); }
            Err(e) => eprintln!("{what}: {} ({}): {e}; skipped", d.name, path.display()),
        }
    }
    None
}

/// The event devices in `sysfs` (`SYSFS`), in the order the kernel registered them.
fn event_devices(sysfs: &Path) -> Vec<EventDevice> {
    let Ok(entries) = std::fs::read_dir(sysfs) else { return Vec::new() };
    let mut nodes: Vec<String> = entries.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.starts_with("event")).collect();
    nodes.sort_by_key(|n| n[5..].parse::<u32>().unwrap_or(u32::MAX));
    nodes.into_iter().map(|node| {
        let info = sysfs.join(&node).join("device");
        let name = std::fs::read_to_string(info.join("name")).map_or_else(|_| node.clone(), |n| n.trim().to_string());
        EventDevice { node, name, info }
    }).collect()
}

/// Whether a sysfs capability bitmap has bit `bit`: hex words, the most significant first, each as
/// wide as the kernel's `long` (64 bits on the 64-bit kernels Specular runs on).
fn has_bit(bitmap: &str, bit: u16) -> bool {
    let word = bitmap.split_whitespace().rev().nth(bit as usize / 64);
    word.and_then(|w| u64::from_str_radix(w, 16).ok()).is_some_and(|w| w & (1 << (bit % 64)) != 0)
}

fn open_by_name(name: &str) -> Option<File> {
    let d = event_devices(Path::new(SYSFS)).into_iter().find(|d| d.name == name)?;
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(Path::new(DEV).join(&d.node)).ok()?;
    // Timestamp events on CLOCK_MONOTONIC (as `clock` reads it) instead of wall-clock time,
    // which button repeats and latency are measured against. Without it, skip the device.
    if unsafe { libc::ioctl(file.as_raw_fd(), EVIOCSCLOCKID, &libc::CLOCK_MONOTONIC) } < 0 {
        eprintln!("input: {name}: can't use CLOCK_MONOTONIC timestamps; skipped");
        return None;
    }
    eprintln!("input: {name} open");
    Some(file)
}

impl Input {
    /// Input devices with these names (as in /sys/class/input/*/device/name). Nothing is open
    /// until `retry` is called.
    pub fn new(names: &[&'static str]) -> Self {
        let devices = names.iter().map(|&name| InputDevice { name, file: None, held: Vec::new(), hat: [0; 2], dropping: false }).collect();
        Self { devices, next_retry: Instant::now(), positional: false, menu_chord: None, menu_stick: None }
    }

    /// Reads 0x133 as the top button (`BTN_NORTH`) and 0x134 as the left one (`BTN_WEST`), as
    /// kernel drivers send them (Documentation/input/gamepad.rst), rather than as Xbox style pads do.
    pub fn positional(mut self) -> Self { self.positional = true; self }

    /// Also reports the left stick of the joystick device `name` as the d-pad (`ButtonEvent::stick`),
    /// so it moves through the menus.
    pub fn menu_stick(mut self, name: &'static str) -> Self {
        self.menu_stick = Some(MenuStick { sticks: crate::sticks::EvdevSticks::new(name), held: [None; 2] });
        self
    }

    /// Also reports Menu while Select and Start are both held, for a device without a Menu button.
    /// Select and Start are still reported themselves, so a game sees both pressed briefly.
    pub fn menu_chord(mut self) -> Self { self.menu_chord = Some(Chord::default()); self }

    /// Adds Menu's presses and releases to the events in `out` from `from` on (`menu_chord`), each
    /// right after the Select or Start event that makes it.
    fn add_menu_chord(&mut self, out: &mut Vec<ButtonEvent>, from: usize) {
        let Some(c) = self.menu_chord.as_mut() else { return };
        let events = out.split_off(from);
        for e in events {
            out.push(e);
            match e.button { Button::Select => c.select = e.pressed, Button::Start => c.start = e.pressed, _ => continue }
            let both = c.select && c.start;
            if both != c.menu {
                c.menu = both;
                out.push(ButtonEvent { button: Button::Menu, pressed: both, time: e.time, stick: false });
            }
        }
    }

    /// Tries to open missing devices, at most once a second. A mute switch found already on
    /// is reported in `out` as a press. Returns how long until the next try, or None if all are open.
    pub fn retry(&mut self, out: &mut Vec<ButtonEvent>) -> Option<Duration> {
        let stick = self.menu_stick.as_mut().and_then(|m| m.sticks.retry());
        let buttons = self.retry_buttons(out);
        [stick, buttons].into_iter().flatten().min()
    }

    fn retry_buttons(&mut self, out: &mut Vec<ButtonEvent>) -> Option<Duration> {
        if self.devices.iter().all(|d| d.file.is_some()) { return None; }
        let now = Instant::now();
        if now >= self.next_retry {
            for d in self.devices.iter_mut().filter(|d| d.file.is_none()) {
                d.file = open_by_name(d.name);
                d.sync_switch(out, clock(libc::CLOCK_MONOTONIC));
            }
            self.next_retry = now + RETRY;
        }
        self.devices.iter().any(|d| d.file.is_none()).then(|| self.next_retry - now)
    }

    /// File descriptors of the open devices, for poll().
    pub fn fds(&self) -> impl Iterator<Item = RawFd> + '_ {
        self.devices.iter().filter_map(|d| d.file.as_ref().map(|f| f.as_raw_fd())).chain(self.menu_stick.as_ref().and_then(|m| m.sticks.fd()))
    }

    /// Reads everything pending. A device that fails is closed, with its buttons released, and
    /// `retry` brings it back. (`Mute` isn't released: the switch is still where it was, and
    /// `retry` re-reads it.)
    pub fn read(&mut self, out: &mut Vec<ButtonEvent>) {
        let from = out.len();
        for d in &mut self.devices {
            let mut buf = [0u8; EVENT_SIZE];
            while let Some(file) = d.file.as_mut() {
                match file.read(&mut buf) {
                    Ok(EVENT_SIZE) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    _ => { eprintln!("input: {} gone", d.name); d.file = None; d.release_all(out, clock(libc::CLOCK_MONOTONIC)); break; }
                }
                let (time, kind, code, value) = parse_event(&buf);
                d.event(kind, code, value, time, out);
            }
        }
        if let Some(m) = self.menu_stick.as_mut() && m.sticks.drain() {
            m.press(clock(libc::CLOCK_MONOTONIC), out);
        }
        if self.positional {
            for e in &mut out[from..] {
                e.button = match e.button { Button::X => Button::Y, Button::Y => Button::X, b => b };
            }
        }
        self.add_menu_chord(out, from);
    }
}

impl MenuStick {
    /// Presses or releases the d-pad direction of each axis the left stick has crossed a threshold
    /// on since the last time.
    fn press(&mut self, time: Duration, out: &mut Vec<ButtonEvent>) {
        let (x, y) = self.sticks.left();
        for (axis, (v, [neg, pos])) in [(x, [Button::Left, Button::Right]), (y, [Button::Up, Button::Down])].into_iter().enumerate() {
            let held = self.held[axis];
            let now = if v.abs() >= STICK_PRESS { Some(if v < 0.0 { neg } else { pos }) }
                else if v.abs() <= STICK_RELEASE { None }
                else { held };
            if now == held { continue; }
            if let Some(b) = held { out.push(ButtonEvent { button: b, pressed: false, time, stick: true }); }
            if let Some(b) = now { out.push(ButtonEvent { button: b, pressed: true, time, stick: true }); }
            self.held[axis] = now;
        }
    }
}

impl InputDevice {
    fn set(&mut self, button: Button, pressed: bool, time: Duration, out: &mut Vec<ButtonEvent>) {
        if self.held.contains(&button) == pressed { return; }
        if pressed { self.held.push(button) } else { self.held.retain(|&b| b != button) }
        out.push(ButtonEvent { button, pressed, time, stick: false });
    }

    /// Releases every button held, but not the mute switch: that is a position, not a held key,
    /// and `sync_switch` re-reads it.
    fn release_all(&mut self, out: &mut Vec<ButtonEvent>, time: Duration) {
        let (switch, buttons) = std::mem::take(&mut self.held).into_iter().partition(|&b| b == Button::Mute);
        self.held = switch;
        for b in buttons { out.push(ButtonEvent { button: b, pressed: false, time, stick: false }); }
        self.hat = [0; 2];
    }

    /// Reads the mute switch's position from the device and reports it if it changed (at open,
    /// and after events were lost). A device without the switch, or where the query fails, counts as off.
    fn sync_switch(&mut self, out: &mut Vec<ButtonEvent>, time: Duration) {
        let Some(file) = self.file.as_ref() else { return };
        let on = switches(file).is_some_and(|bits| bits & (1 << SW_TABLET_MODE) != 0);
        self.set(Button::Mute, on, time, out);
    }

    fn event(&mut self, kind: u16, code: u16, value: i32, time: Duration, out: &mut Vec<ButtonEvent>) {
        // Events were lost: release everything held (rather than leave a button stuck) and ignore
        // events up to the next report, as the kernel's evdev rules say.
        if self.dropping {
            if (kind, code) == (EV_SYN, SYN_REPORT) { self.dropping = false; }
            return;
        }
        match (kind, code) {
            (EV_SYN, SYN_DROPPED) => { self.release_all(out, time); self.sync_switch(out, time); self.dropping = true; }
            // value 2 is the kernel's own key repeat; Specular repeats keys itself
            (EV_KEY, code) => if let (Some(b), 0 | 1) = (key(code), value) { self.set(b, value == 1, time, out) },
            (EV_SW, SW_TABLET_MODE) => self.set(Button::Mute, value != 0, time, out),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_and_start_together_are_menu() {
        let mut input = Input::new(&[]).menu_chord();
        let e = |button, pressed| ButtonEvent { button, pressed, time: Duration::ZERO, stick: false };
        let mut out = vec![e(Button::Select, true), e(Button::Start, true), e(Button::Select, false), e(Button::Start, false)];
        input.add_menu_chord(&mut out, 0);
        assert_eq!(out, [e(Button::Select, true), e(Button::Start, true), e(Button::Menu, true), e(Button::Select, false),
            e(Button::Menu, false), e(Button::Start, false)]);
    }

    #[test]
    fn capability_bitmaps() {
        use super::has_bit;
        assert!(has_bit("14", 2)); // the Brick's jack: SW_HEADPHONE_INSERT and SW_MICROPHONE_INSERT
        assert!(!has_bit("2", 2)); // padbridge's pad: SW_TABLET_MODE only
        assert!(!has_bit("0", 2));
        assert!(!has_bit("", 2));
        assert!(has_bit("1 0\n", 64));
    }
}
