//! Specular on a desktop, for development: a window the size of the Brick's screen, simulated
//! hardware, and the keyboard as the gamepad (see `specular_ui::run` for the keys).
//!
//! Usage: specular [data folder, with roms/ and configs/ (default ./sdcard)]

use specular_core::device::{Device, DeviceProfile, Hardware, simulated};
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
            light_bar_leds: 0,
        }
    }
    fn hardware(&self) -> Hardware { simulated::hardware().0 }
}

fn main() -> Result<(), specular_ui::Error> {
    // One window pixel per panel pixel, whatever the desktop's scaling.
    // SAFETY: before any other thread exists.
    unsafe { std::env::set_var("SLINT_SCALE_FACTOR", "1") };
    let data = std::env::args().nth(1).unwrap_or_else(|| "sdcard".into());
    specular_ui::run(&Desktop, DataRoot(data.into()), None)
}
