//! The panel's backlight through the backlight class (`/sys/class/backlight/<name>`).

use std::io;
use std::path::{Path, PathBuf};

use specular_core::device::Display;

/// `bl_power` values (FB_BLANK_UNBLANK, FB_BLANK_POWERDOWN).
const ON: &str = "0";
const OFF: &str = "4";

/// One backlight device. Writing it needs root.
pub struct SysfsBacklight { dir: PathBuf, max: u32 }

impl SysfsBacklight {
    /// The backlight `name` under `root` (/sys/class/backlight); fails if it has no
    /// `max_brightness`.
    pub fn open(root: &Path, name: &str) -> io::Result<Self> {
        let dir = root.join(name);
        let max = std::fs::read_to_string(dir.join("max_brightness"))?.trim().parse().map_err(io::Error::other)?;
        Ok(Self { dir, max })
    }

    fn write(&self, file: &str, value: &str) -> io::Result<()> {
        std::fs::write(self.dir.join(file), value).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", self.dir.join(file).display())))
    }
}

impl Display for SysfsBacklight {
    /// Levels 1–10 map to 1/16 of the maximum up to all of it on a curve, since perceived
    /// brightness is roughly logarithmic; 1 stays readable indoors.
    fn set_brightness(&self, level: u8) -> io::Result<()> { self.write("brightness", &step(level, self.max).to_string()) }

    /// Only the backlight: the panel keeps showing what it was given, unlit.
    fn set_panel(&self, on: bool) -> io::Result<()> { self.write("bl_power", if on { ON } else { OFF }) }
}

/// The backlight's value for a level 1–10, on a scale of 0 to `max`.
fn step(level: u8, max: u32) -> u32 {
    let l = level.clamp(1, 10) as f64;
    (max as f64 / 16.0 * 16f64.powf((l - 1.0) / 9.0)).round().max(1.0) as u32
}

#[cfg(test)]
mod tests {
    #[test]
    fn levels_climb_from_a_sixteenth_to_full() {
        let steps: Vec<u32> = [1, 5, 10].into_iter().map(|l| super::step(l, 255)).collect();
        assert_eq!(steps, [16, 55, 255]);
        assert_eq!(super::step(1, 7), 1);
    }
}
