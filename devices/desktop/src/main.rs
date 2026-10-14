//! Specular on a desktop, for development: a window the size of the Brick's screen, simulated
//! hardware, sound through ALSA, and the keyboard as the gamepad (see `specular_ui::run` for the
//! keys). Games are paced by a timer (a desktop window can't wait for the display's refresh).
//!
//! Usage: specular [data folder, with roms/ and configs/ (default ./sdcard)]
//! Cores are looked for in `cores/` next to the binary, or in $SPECULAR_CORES.

use std::io;

use specular_core::device::{Device, DeviceProfile, Hardware, Output, SoundOut, Vsync, simulated};
use specular_core::input::Button;
use specular_ui::DataRoot;

struct Desktop;

impl Device for Desktop {
    fn profile(&self) -> DeviceProfile {
        use Button::*;
        DeviceProfile {
            name: "Desktop",
            width: 1024, height: 768, ppi: 400, refresh_hz: 60.0,
            buttons: &[Up, Down, Left, Right, A, B, X, Y, L1, R1, L2, R2, Select, Start, Menu, VolUp, VolDown, Power],
            leds: &[],
        }
    }
    fn hardware(&self) -> Hardware { simulated::hardware() }
    /// Bluetooth outputs through bluez-alsa, if the computer has it (the simulated Bluetooth's
    /// addresses only open if they happen to be real).
    fn open_sound(&self, output: &Output) -> io::Result<Box<dyn SoundOut>> {
        use specular_linux::sound::Alsa;
        Ok(Box::new(match output { Output::Builtin => Alsa::open("default")?, Output::Bluetooth(address) => Alsa::bluetooth(address)? }))
    }
    fn open_vsync(&self) -> Option<Box<dyn Vsync>> { None }
}

fn main() -> Result<(), specular_ui::Error> {
    // One window pixel per panel pixel, whatever the desktop's scaling.
    // SAFETY: before any other thread exists.
    unsafe { std::env::set_var("SLINT_SCALE_FACTOR", "1") };
    specular_ui::use_fallback_fonts();
    let data = std::env::args().nth(1).unwrap_or_else(|| "sdcard".into());
    specular_ui::run(Box::new(Desktop), DataRoot(data.into()), None)
}
