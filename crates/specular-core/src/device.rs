//! What Specular needs from a device. A port describes its screen and buttons in a
//! `DeviceProfile` and implements these interfaces; most Linux handhelds can use the standard
//! implementations in specular-linux and override only their quirks.
//!
//! Calls may touch slow hardware (a battery reading can be a slow bus transfer), so the UI crate
//! makes them on its hardware thread, never on the UI thread.

use std::io;
use std::time::Duration;

use crate::input::Button;

/// The screen, buttons and lights of a device.
#[derive(Clone, Debug)]
pub struct DeviceProfile {
    pub name: &'static str,
    /// The screen in pixels.
    pub width: u32,
    pub height: u32,
    /// Pixels per inch. Sizes in the UI scale by `ppi / 400`, so text keeps its physical size.
    pub ppi: u32,
    /// The display's refresh rate (game frames will be paced against it).
    pub refresh_hz: f64,
    /// The buttons the device has; hints never show others.
    pub buttons: &'static [Button],
    /// The device's RGB LEDs in chain order, with the accent each one shows (empty for none).
    pub leds: &'static [Accent],
}

/// Which of the two accent colours an LED shows (`settings::builtin::ACCENT` or `SECOND_ACCENT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent { First, Second }

/// The battery and charger, as the status bar shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    /// Charge in percent, if the device knows it.
    pub percent: Option<u8>,
    pub charging: bool,
    /// On external power (charging or full).
    pub plugged: bool,
}

/// Battery and power state.
pub trait Power: Send {
    /// The battery now (may be a slow hardware read).
    fn battery(&self) -> io::Result<Battery>;
    /// Sleeps until woken (power button), returning once awake again.
    fn sleep(&self) -> io::Result<()>;
    /// Turns the device off. The system may stop Specular (SIGTERM) before this returns, so
    /// nothing after the call can be relied on to run.
    fn shutdown(&self) -> io::Result<()>;
    /// Restarts the device. As with `shutdown`, Specular may be stopped before this returns.
    fn restart(&self) -> io::Result<()>;
}

/// The screen's backlight and panel.
pub trait Display: Send {
    /// Backlight brightness, 1–10 (Specular's scale; the device maps it to its own levels).
    fn set_brightness(&self, level: u8) -> io::Result<()>;
    /// Turns the panel off (and its backlight) or back on, for screen-off without sleeping.
    fn set_panel(&self, on: bool) -> io::Result<()>;
}

/// A colour, 0–255 per channel, before the device's brightness limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// The RGB LEDs.
pub trait Lights: Send {
    /// Sets the LEDs, one colour per LED (as many as the profile lists). Implementations apply
    /// the device's brightness limits.
    fn show(&self, colours: &[Rgb]) -> io::Result<()>;
}

/// Sound output.
pub trait Audio: Send {
    /// Output volume, 0–20 (Specular's scale; the device maps it to its own controls).
    fn set_volume(&self, level: u8) -> io::Result<()>;
}

/// Where a game's sound goes: a queue of 16-bit stereo frames, played at a fixed rate.
pub trait SoundOut: Send {
    /// Frames played per second.
    fn rate(&self) -> u32;
    /// Queues as many of `frames` (left, right) as fit without waiting; returns how many.
    fn write(&mut self, frames: &[[i16; 2]]) -> io::Result<usize>;
    /// Frames queued and not played yet.
    fn queued(&mut self) -> usize;
    /// How many frames the queue holds.
    fn capacity(&self) -> usize;
}

/// The display's refresh, for pacing game frames.
pub trait Vsync: Send {
    /// Waits until the display next starts showing a new frame.
    fn wait(&mut self) -> io::Result<()>;
}

/// A device's hardware services.
pub struct Hardware {
    pub power: Box<dyn Power>,
    pub display: Box<dyn Display>,
    pub lights: Box<dyn Lights>,
    pub audio: Box<dyn Audio>,
}

/// A device: what it is, and its hardware.
pub trait Device {
    fn profile(&self) -> DeviceProfile;
    /// The hardware services. Called once; they then move to the UI crate's hardware thread.
    fn hardware(&self) -> Hardware;
    /// Opens the sound output for a game (each game opens its own and closes it when it ends).
    fn open_sound(&self) -> io::Result<Box<dyn SoundOut>>;
    /// The display's refresh, if the device can wait for it; without, games are paced by a
    /// timer at their own frame rate.
    fn open_vsync(&self) -> Option<Box<dyn Vsync>>;
}

/// Simulated hardware, for the desktop build: it prints what it is asked to do, and its battery
/// drains from 80 % by 1 % a minute.
pub mod simulated {
    use std::time::Instant;

    use super::*;

    struct Sim(Instant);

    impl Power for Sim {
        fn battery(&self) -> io::Result<Battery> {
            let drained = (self.0.elapsed().as_secs() / 60).min(75) as u8;
            Ok(Battery { percent: Some(80 - drained), charging: false, plugged: false })
        }
        fn sleep(&self) -> io::Result<()> { eprintln!("simulated: sleep"); std::thread::sleep(Duration::from_millis(500)); Ok(()) }
        fn shutdown(&self) -> io::Result<()> { eprintln!("simulated: shut down"); Ok(()) }
        fn restart(&self) -> io::Result<()> { eprintln!("simulated: restart"); Ok(()) }
    }
    impl Display for Sim {
        fn set_brightness(&self, level: u8) -> io::Result<()> { eprintln!("simulated: brightness {level}"); Ok(()) }
        fn set_panel(&self, on: bool) -> io::Result<()> { eprintln!("simulated: panel {}", if on { "on" } else { "off" }); Ok(()) }
    }
    impl Lights for Sim {
        fn show(&self, colours: &[Rgb]) -> io::Result<()> { eprintln!("simulated: lights {colours:?}"); Ok(()) }
    }
    impl Audio for Sim {
        fn set_volume(&self, level: u8) -> io::Result<()> { eprintln!("simulated: volume {level}"); Ok(()) }
    }

    pub fn hardware() -> Hardware {
        let t = Instant::now();
        Hardware { power: Box::new(Sim(t)), display: Box::new(Sim(t)), lights: Box::new(Sim(t)), audio: Box::new(Sim(t)) }
    }
}
