//! Headphone plugging, from the kernel. An ALSA driver that detects a jack makes an input device
//! for it ("<card> <jack>", e.g. the Brick's "audiocodec sunxi Audio Jack") that reports the
//! switch `SW_HEADPHONE_INSERT`; the first input device with that switch is followed. The driver
//! itself moves the sound between the speaker and the jack.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use specular_core::device::HeadphoneJack;

use crate::input::{EV_SW, EV_SYN, EVENT_SIZE, SYN_DROPPED, SYN_REPORT, event_devices, parse_event, switches};

const SW_HEADPHONE_INSERT: u16 = 2; // linux/input-event-codes.h

/// The jack, through its input device (see the module's notes).
pub struct EvdevJack;

impl HeadphoneJack for EvdevJack {
    fn watch(self: Box<Self>, plugged: Box<dyn FnMut(bool) + Send>) {
        follow(Path::new("/sys/class/input"), Path::new("/dev/input"), plugged);
    }
}

/// Follows the first input device in `sysfs` (/sys/class/input) with a headphone switch, opened
/// from `dev` (/dev/input), until it fails. Calls `plugged` with the first state read, then only
/// when the state differs from the last one reported.
fn follow(sysfs: &Path, dev: &Path, mut plugged: Box<dyn FnMut(bool) + Send>) {
    let Some((name, mut file)) = find(sysfs, dev) else {
        eprintln!("input: no headphone jack found (no input device has SW_HEADPHONE_INSERT); headphones get the speaker's volume");
        return;
    };
    let mut last = None;
    let mut report = |now: bool| if last.replace(now) != Some(now) {
        eprintln!("input: headphones {}", if now { "plugged in" } else { "unplugged" });
        plugged(now);
    };
    let read_state = |file: &File, report: &mut dyn FnMut(bool)| match switches(file) {
        Some(bits) => report(bits & (1 << SW_HEADPHONE_INSERT) != 0),
        None => eprintln!("input: {name}: can't read the jack's state; waiting for it to change"),
    };
    read_state(&file, &mut report);
    let (mut events, mut buf) = (JackEvents::default(), [0u8; EVENT_SIZE]);
    loop {
        if let Err(e) = file.read_exact(&mut buf) {
            return eprintln!("input: {name}: {e}; headphones are no longer followed");
        }
        let (_, kind, code, value) = parse_event(&buf);
        match events.event(kind, code, value) {
            Some(Seen::Plugged(now)) => report(now),
            Some(Seen::Resync) => read_state(&file, &mut report),
            None => {}
        }
    }
}

/// The first event device in `sysfs` whose capabilities include the headphone switch: its name
/// and the device, opened from `dev`.
fn find(sysfs: &Path, dev: &Path) -> Option<(String, File)> {
    for d in event_devices(sysfs).into_iter().filter(|d| d.has("sw", SW_HEADPHONE_INSERT)) {
        let path = dev.join(&d.node);
        match File::open(&path) {
            Ok(file) => { eprintln!("input: headphone jack: {} ({})", d.name, path.display()); return Some((d.name, file)); }
            Err(e) => eprintln!("input: headphone jack {} ({}): {e}; skipped", d.name, path.display()),
        }
    }
    None
}

/// What the jack device's events say.
#[derive(Debug, PartialEq)]
enum Seen {
    Plugged(bool),
    /// Events were lost: the state must be read afresh.
    Resync,
}

/// Interprets the jack device's events by evdev's rules: after `SYN_DROPPED`, events up to the next
/// `SYN_REPORT` are dropped too, and the state is read again then.
#[derive(Default)]
struct JackEvents { dropping: bool }

impl JackEvents {
    fn event(&mut self, kind: u16, code: u16, value: i32) -> Option<Seen> {
        match (kind, code) {
            (EV_SYN, SYN_DROPPED) => { self.dropping = true; None }
            (EV_SYN, SYN_REPORT) if self.dropping => { self.dropping = false; Some(Seen::Resync) }
            _ if self.dropping => None,
            (EV_SW, SW_HEADPHONE_INSERT) => Some(Seen::Plugged(value != 0)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn events_after_a_loss_are_dropped_until_the_next_report() {
        let mut j = JackEvents::default();
        assert_eq!(j.event(EV_SW, SW_HEADPHONE_INSERT, 1), Some(Seen::Plugged(true)));
        assert_eq!(j.event(EV_SYN, SYN_REPORT, 0), None);
        assert_eq!(j.event(EV_SW, 4, 1), None); // the microphone switch
        assert_eq!(j.event(EV_SYN, SYN_DROPPED, 0), None);
        assert_eq!(j.event(EV_SW, SW_HEADPHONE_INSERT, 0), None);
        assert_eq!(j.event(EV_SYN, SYN_REPORT, 0), Some(Seen::Resync));
        assert_eq!(j.event(EV_SW, SW_HEADPHONE_INSERT, 0), Some(Seen::Plugged(false)));
    }

    /// A `struct input_event` as the kernel writes it.
    fn event(kind: u16, code: u16, value: i32) -> Vec<u8> {
        [&[0u8; 16][..], &kind.to_ne_bytes(), &code.to_ne_bytes(), &value.to_ne_bytes()].concat()
    }

    #[test]
    fn follows_the_device_with_a_headphone_switch_and_reports_changes() {
        let (sysfs, dev) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        for (node, name, sw) in [("event2", "TRIMUI Player1", "2"), ("event10", "audiocodec sunxi Audio Jack", "14")] {
            std::fs::create_dir_all(sysfs.path().join(node).join("device/capabilities")).unwrap();
            std::fs::write(sysfs.path().join(node).join("device/name"), format!("{name}\n")).unwrap();
            std::fs::write(sysfs.path().join(node).join("device/capabilities/sw"), format!("{sw}\n")).unwrap();
        }
        // A plain file stands in for the device: its state can't be read, and its end is where
        // the device fails.
        let events: Vec<u8> = [1, 1, 0, 1].iter().flat_map(|&v| [event(EV_SW, SW_HEADPHONE_INSERT, v), event(EV_SYN, SYN_REPORT, 0)].concat()).collect();
        std::fs::write(dev.path().join("event10"), events).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        follow(sysfs.path(), dev.path(), Box::new(move |p| s.lock().unwrap().push(p)));
        assert_eq!(*seen.lock().unwrap(), [true, false, true]);
    }
}
