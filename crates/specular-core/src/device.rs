//! What Specular needs from a device. A port describes its screen and buttons in a
//! `DeviceProfile` and implements these interfaces; most Linux handhelds can use the standard
//! implementations in specular-linux and override only their quirks.
//!
//! Calls may touch slow hardware (a battery reading can be an I2C transfer), so the app makes
//! them from a worker thread, never from the UI thread.

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
    /// The display's refresh rate, which game frames are paced against.
    pub refresh_hz: f64,
    /// The buttons the device has; hints never show others.
    pub buttons: &'static [Button],
    /// How many LEDs the light bar has (0 for none).
    pub light_bar_leds: usize,
}

/// The battery and charger, as the status bar shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    /// Charge in percent, if the device knows it.
    pub percent: Option<u8>,
    pub charging: bool,
    /// On external power (charging or full).
    pub plugged: bool,
}

pub trait Power: Send {
    fn battery(&self) -> io::Result<Battery>;
    /// Sleeps until woken (power button), returning once awake again.
    fn sleep(&self) -> io::Result<()>;
    fn shutdown(&self) -> io::Result<()>;
}

pub trait Display: Send {
    /// Backlight brightness, 1–10 (Specular's scale; the device maps it to its own levels).
    fn set_brightness(&self, level: u8) -> io::Result<()>;
    /// Turns the panel off (and its backlight) or back on, for screen-off without sleeping.
    fn set_panel(&self, on: bool) -> io::Result<()>;
}

/// An RGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub trait Lights: Send {
    /// Sets the light bar, one colour per LED (as many as the profile says). Implementations
    /// apply the device's brightness limits.
    fn show(&self, colours: &[Rgb]) -> io::Result<()>;
}

pub trait Audio: Send {
    /// Output volume, 0–20 (Specular's scale; the device maps it to its own controls).
    fn set_volume(&self, level: u8) -> io::Result<()>;
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
    fn hardware(&self) -> Hardware;
}

/// Simulated hardware, for the desktop build and tests: it logs what it is asked to do, and its
/// battery drains slowly from 80 %.
pub mod simulated {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use super::*;

    /// Shared state of the simulated hardware, readable by tests.
    #[derive(Debug, Default)]
    pub struct State { pub brightness: u8, pub panel_on: bool, pub volume: u8, pub lights: Vec<Rgb>, pub sleeps: u32 }

    struct Sim { state: Arc<Mutex<State>>, start: Instant }

    impl Power for Sim {
        fn battery(&self) -> io::Result<Battery> {
            let drained = (self.start.elapsed().as_secs() / 60).min(75) as u8; // 1 % a minute
            Ok(Battery { percent: Some(80 - drained), charging: false, plugged: false })
        }
        fn sleep(&self) -> io::Result<()> {
            eprintln!("simulated: sleep");
            self.state.lock().unwrap().sleeps += 1;
            std::thread::sleep(Duration::from_millis(500));
            Ok(())
        }
        fn shutdown(&self) -> io::Result<()> { eprintln!("simulated: shut down"); Ok(()) }
    }
    impl Display for Sim {
        fn set_brightness(&self, level: u8) -> io::Result<()> { self.state.lock().unwrap().brightness = level; Ok(()) }
        fn set_panel(&self, on: bool) -> io::Result<()> { self.state.lock().unwrap().panel_on = on; Ok(()) }
    }
    impl Lights for Sim {
        fn show(&self, colours: &[Rgb]) -> io::Result<()> { self.state.lock().unwrap().lights = colours.to_vec(); Ok(()) }
    }
    impl Audio for Sim {
        fn set_volume(&self, level: u8) -> io::Result<()> { self.state.lock().unwrap().volume = level; Ok(()) }
    }

    /// Simulated hardware, and its state.
    pub fn hardware() -> (Hardware, Arc<Mutex<State>>) {
        let state = Arc::new(Mutex::new(State { panel_on: true, ..Default::default() }));
        let sim = || Box::new(Sim { state: state.clone(), start: Instant::now() });
        (Hardware { power: sim(), display: sim(), lights: sim(), audio: sim() }, state.clone())
    }
}
