//! A mainline board as a Specular device: its profile from the board table and the display, and
//! its hardware through the standard interfaces and Crystaline OS's services.

use std::io;
use std::path::{Path, PathBuf};

use specular_core::input::Button;
use specular_core::device::{Audio, Battery, Device, DeviceProfile, Hardware, Lights, Output, Power, Rgb, SoundOut, Vsync};
use specular_crystaline::system;
use specular_core::wireless::trace::Traced;
use specular_linux::backlight::SysfsBacklight;
use specular_linux::bluez::Bluez;
use specular_linux::rumble::EvdevRumble;
use specular_linux::kms::{DrmVsync, Kms};
use specular_linux::power_supply::PowerSupply;
use specular_linux::sound::Alsa;

use crate::boards::{Board, Mixer};

/// The vblanks the refresh rate is measured over at start-up (half a second at 60 Hz).
const MEASURE_VBLANKS: u32 = 30;

/// The board Specular runs on, with what start-up found out about its display.
pub struct Mainline {
    board: &'static Board,
    width: u32,
    height: u32,
    refresh_hz: f64,
    card: PathBuf,
    crtc_index: u32,
}

impl Mainline {
    /// The device on `board`, showing on `kms` (opened from `card`). Measures the panel's refresh
    /// rate, which can differ from what its mode says (the display clock may not reach the mode's
    /// own); falls back to the mode's if vblanks can't be waited for.
    pub fn new(board: &'static Board, kms: &Kms, card: &Path) -> Self {
        let mode = kms.mode();
        let refresh_hz = match kms.vsync(card).and_then(|mut v| v.measure(MEASURE_VBLANKS)) {
            Ok(hz) => { eprintln!("kms: refresh measured {hz:.3} Hz over {MEASURE_VBLANKS} vblanks (mode says {:.3} Hz)", mode.refresh_hz()); hz }
            Err(e) => { eprintln!("kms: can't measure the refresh ({e}); taking the mode's {:.3} Hz", mode.refresh_hz()); mode.refresh_hz() }
        };
        Self { board, width: mode.width(), height: mode.height(), refresh_hz, card: card.into(), crtc_index: kms.crtc_index() }
    }
}

impl Device for Mainline {
    fn profile(&self) -> DeviceProfile {
        DeviceProfile {
            name: self.board.name,
            width: self.width,
            height: self.height,
            ppi: self.board.ppi,
            refresh_hz: self.refresh_hz,
            buttons: self.board.buttons,
            labels: if self.board.menu_chord { &[(Button::Menu, "SELECT+START")] } else { &[] },
            leds: &[],
        }
    }

    fn hardware(&self) -> Hardware {
        let display: Box<dyn specular_core::device::Display> = match SysfsBacklight::open(Path::new("/sys/class/backlight"), self.board.backlight) {
            Ok(b) => Box::new(b),
            Err(e) => { eprintln!("hardware: backlight {}: {e}", self.board.backlight); Box::new(NoBacklight) }
        };
        Hardware {
            power: Box::new(MainlinePower(PowerSupply::find(Path::new("/sys/class/power_supply")))),
            display,
            lights: Box::new(NoLights),
            audio: Box::new(Volume(&self.board.volume)),
            wifi: specular_crystaline::wifi::wifi(),
            bluetooth: Some(Box::new(Traced::new("bluetooth", Bluez::new()))),
            installer: None,
            time_zone: Some(Box::new(specular_crystaline::timezone::CrystalineTimeZone)),
            card: Some(system::card()),
            // The jack cuts the speaker in hardware and reports nothing: one output, one volume.
            headphones: None,
            rumble: self.board.rumble.map(|m| Box::new(EvdevRumble::motor(m)) as Box<dyn specular_core::device::Rumble>),
            file_transfer: Some(Box::new(specular_crystaline::transfer::WebdavTransfer)),
            ssh: Some(Box::new(specular_crystaline::ssh::CrystalineSsh)),
            cpu: Some(Box::new(specular_crystaline::cpu::CrystalineCpu)),
        }
    }

    fn open_sound(&self, output: &Output) -> io::Result<Box<dyn SoundOut>> {
        Ok(Box::new(match output {
            Output::Speaker | Output::Headphones => Alsa::open(self.board.sound)?,
            Output::Bluetooth(address) => Alsa::bluetooth(address)?,
        }))
    }

    /// The panel's vblanks, on a file of its own.
    fn open_vsync(&self) -> Option<Box<dyn Vsync>> {
        match DrmVsync::open(&self.card, self.crtc_index) {
            Ok(v) => Some(Box::new(v)),
            Err(e) => { eprintln!("vsync: {e}; games are paced by a timer"); None }
        }
    }
}

struct MainlinePower(PowerSupply);

impl Power for MainlinePower {
    fn battery(&self) -> io::Result<Battery> { self.0.battery() }

    fn sleep(&self) -> io::Result<()> { system::sleep() }

    fn shutdown(&self) -> io::Result<()> { system::shutdown() }

    fn restart(&self) -> io::Result<()> { system::restart() }

    fn last_shutdown(&self) -> io::Result<Option<String>> { system::last_shutdown() }
}

/// The LEDs stay with the system (status and charging), as the profile lists none.
struct NoLights;

impl Lights for NoLights {
    fn show(&self, _: &[Rgb]) -> io::Result<()> { Ok(()) }
}

/// A board whose backlight couldn't be opened: each change fails with NotFound.
struct NoBacklight;

impl specular_core::device::Display for NoBacklight {
    fn set_brightness(&self, _: u8) -> io::Result<()> { Err(io::Error::new(io::ErrorKind::NotFound, "no backlight")) }
    fn set_panel(&self, _: bool) -> io::Result<()> { Err(io::Error::new(io::ErrorKind::NotFound, "no backlight")) }
}

/// The volume through the board's mixer control, set with amixer. Specular's levels are 2 dB
/// apart, the top one at 0 dB; 0 is the control's lowest. The speaker's level is the one used
/// (there is no headphone detection), and Bluetooth has its own (through bluez-alsa).
struct Volume(&'static Mixer);

impl Audio for Volume {
    fn set_volume(&self, output: &Output, level: u8) -> io::Result<()> {
        match output {
            Output::Speaker => {
                let m = self.0;
                system::run("/usr/bin/amixer", &["-q", "-c", m.card, "cset", &format!("name={}", m.control), &step(m, level).to_string()])
            }
            Output::Headphones => Ok(()),
            Output::Bluetooth(address) => specular_linux::sound::set_bluetooth_volume(address, level),
        }
    }
}

/// The mixer's value for a volume level.
fn step(m: &Mixer, level: u8) -> u32 {
    let below_top = (specular_core::settings::builtin::VOLUME_MAX - level as i64).max(0) as f64;
    if level == 0 { 0 } else { (m.max as f64 - below_top * 2.0 / m.db_per_step).round().max(0.0) as u32 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn levels_are_2_db_apart_with_the_top_at_0_db() {
        let m = &crate::boards::BOARDS[0].volume;
        let steps: Vec<u32> = [0, 1, 10, 19, 20].into_iter().map(|l| super::step(m, l)).collect();
        assert_eq!(steps, [0, 153, 201, 250, 255]);
    }
}
