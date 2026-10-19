//! Rumble through Linux force feedback: an `FF_RUMBLE` effect on the first gamepad that offers it
//! (on handhelds, a program often presents the buttons as a virtual gamepad and passes its rumble
//! on to the motor, as the Brick's padbridge does). One effect is uploaded, updated as the strength changes, and played
//! until stopped. The kernel erases a file's effects when it closes, which stops them, so the
//! motor stops if Specular ends in any way.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use specular_core::device::{Motors, Rumble};

use crate::input::{EVENT_SIZE, event_devices};

// linux/input-event-codes.h
const EV_FF: u16 = 0x15;
const FF_RUMBLE: u16 = 0x50;
/// The first gamepad button (`BTN_SOUTH`): only gamepads are used, not a motor's own device.
const BTN_GAMEPAD: u16 = 0x130;

/// A missing gamepad is looked for at most this often (when something next rumbles).
const LOOK_AGAIN: Duration = Duration::from_secs(1);

/// `struct ff_effect` (linux/input.h) on a 64-bit kernel, holding an `ff_rumble_effect`.
#[repr(C)]
struct FfEffect {
    kind: u16,
    /// -1 uploads a new effect; the kernel sets its id.
    id: i16,
    direction: u16,
    trigger: [u16; 2],
    /// Length (0: until stopped) and delay, in ms.
    replay: [u16; 2],
    /// The union below holds a pointer, so it starts 8-byte aligned.
    _align: u16,
    /// The union, 32 bytes: an `ff_rumble_effect` is its first two (strong, weak).
    union: [u16; 16],
}

const _: () = assert!(std::mem::size_of::<FfEffect>() == 48);
/// `EVIOCSFF`: `_IOW('E', 0x80, struct ff_effect)`.
const EVIOCSFF: libc::c_ulong = 0x4000_4580 | (std::mem::size_of::<FfEffect>() as libc::c_ulong) << 16;

/// The rumble of the first gamepad offering it.
pub struct EvdevRumble {
    sysfs: PathBuf,
    dev: PathBuf,
    pad: Option<Pad>,
    /// When a missing gamepad may next be looked for.
    next_look: Instant,
    /// Whether the gamepad has been reported missing since it was last found.
    missing: bool,
}

/// The gamepad, opened for writing, and its effect: the kernel's id for it (-1 until first
/// uploaded), and whether it is playing.
struct Pad { name: String, file: File, effect: i16, playing: bool }

impl EvdevRumble {
    /// Looks for the gamepad now, logging what it finds; one missing (or gone) is looked for again
    /// when something next rumbles.
    pub fn new() -> Self { Self::at(Path::new("/sys/class/input"), Path::new("/dev/input")) }

    fn at(sysfs: &Path, dev: &Path) -> Self {
        let mut r = Self { sysfs: sysfs.into(), dev: dev.into(), pad: None, next_look: Instant::now(), missing: false };
        r.look();
        r
    }

    /// Opens the first gamepad (in `sysfs`, from `dev`) offering `FF_RUMBLE`, if it may look now.
    fn look(&mut self) {
        if Instant::now() < self.next_look { return; }
        self.next_look = Instant::now() + LOOK_AGAIN;
        for d in event_devices(&self.sysfs).into_iter().filter(|d| d.has("ff", FF_RUMBLE) && d.has("key", BTN_GAMEPAD)) {
            let path = self.dev.join(&d.node);
            match File::options().write(true).open(&path) {
                Ok(file) => {
                    eprintln!("rumble: {} ({})", d.name, path.display());
                    self.pad = Some(Pad { name: d.name, file, effect: -1, playing: false });
                    self.missing = false;
                    return;
                }
                Err(e) => eprintln!("rumble: {} ({}): {e}; skipped", d.name, path.display()),
            }
        }
        if !std::mem::replace(&mut self.missing, true) {
            eprintln!("rumble: no gamepad offers it (FF_RUMBLE) yet; looking again when something next rumbles");
        }
    }
}

impl Rumble for EvdevRumble {
    fn set(&mut self, motors: Motors) -> io::Result<()> {
        if self.pad.is_none() && motors != Motors::STILL { self.look(); }
        let Some(pad) = self.pad.as_mut() else {
            return if motors == Motors::STILL { Ok(()) } else { Err(io::Error::other("no gamepad offers rumble")) };
        };
        let Err(e) = pad.set(motors) else { return Ok(()) };
        // A gamepad program that restarts makes a new device: it is looked for next time.
        let gone = e.raw_os_error() == Some(libc::ENODEV);
        let e = io::Error::new(e.kind(), format!("{}: {e}", pad.name));
        if gone { eprintln!("rumble: {} is gone; looking again when something next rumbles", pad.name); self.pad = None; }
        Err(e)
    }
}

impl Pad {
    fn set(&mut self, m: Motors) -> io::Result<()> {
        if m == Motors::STILL {
            if self.playing { self.play(0)?; self.playing = false; }
            return Ok(());
        }
        let mut union = [0; 16];
        union[..2].copy_from_slice(&[m.strong, m.weak]);
        let mut e = FfEffect { kind: FF_RUMBLE, id: self.effect, direction: 0, trigger: [0; 2], replay: [0; 2], _align: 0, union };
        // Blocks while the gamepad's program (if it is one) passes the effect on.
        if unsafe { libc::ioctl(self.file.as_raw_fd(), EVIOCSFF, &mut e) } < 0 { return Err(io::Error::last_os_error()); }
        self.effect = e.id;
        if !self.playing { self.play(1)?; self.playing = true; }
        Ok(())
    }

    /// Plays the effect (`count` times; it lasts until stopped) or stops it (0).
    fn play(&mut self, count: i32) -> io::Result<()> {
        // A `struct input_event`: its time (16 bytes, unused here), then type, code and value.
        let mut event = [0u8; EVENT_SIZE];
        event[16..18].copy_from_slice(&EV_FF.to_ne_bytes());
        event[18..20].copy_from_slice(&(self.effect as u16).to_ne_bytes());
        event[20..24].copy_from_slice(&count.to_ne_bytes());
        self.file.write_all(&event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As a C compiler lays out linux/input.h's `struct ff_effect` for x86-64 and AArch64.
    #[test]
    fn the_effect_is_laid_out_as_the_kernel_reads_it() {
        assert_eq!((std::mem::offset_of!(FfEffect, replay), std::mem::offset_of!(FfEffect, union)), (10, 16));
        assert_eq!(EVIOCSFF, 0x4030_4580);
    }

    #[test]
    fn the_gamepad_offering_rumble_is_used_not_the_motors_own_device() {
        let (sysfs, dev) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        // As on the Brick: the motor's own device, the jack, then padbridge's pad. Bitmaps are hex
        // words of 64 bits, the most significant first: FF_RUMBLE is bit 16 of word 1, BTN_SOUTH
        // bit 48 of word 4.
        for (node, name, ff, key) in [("event1", "regulator-haptic", "10000 0", ""), ("event4", "audiocodec sunxi Audio Jack", "", ""),
                                      ("event6", "TRIMUI Player1", "10000 0", "1000000000000 0 0 0 0")] {
            let caps = sysfs.path().join(node).join("device/capabilities");
            std::fs::create_dir_all(&caps).unwrap();
            std::fs::write(sysfs.path().join(node).join("device/name"), format!("{name}\n")).unwrap();
            std::fs::write(caps.join("ff"), ff).unwrap();
            std::fs::write(caps.join("key"), key).unwrap();
            std::fs::write(dev.path().join(node), "").unwrap();
        }
        let r = EvdevRumble::at(sysfs.path(), dev.path());
        assert_eq!(r.pad.as_ref().map(|p| p.name.as_str()), Some("TRIMUI Player1"));
        // Without one, stopping is fine and rumbling an error.
        let mut none = EvdevRumble::at(&sysfs.path().join("none"), dev.path());
        assert!(none.set(Motors::STILL).is_ok());
        assert!(none.set(Motors { strong: 1, weak: 0 }).is_err());
    }
}
