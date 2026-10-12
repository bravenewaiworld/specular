//! RGB lights from /sys/class/leds, where each colour of each LED is its own device with a
//! `brightness` file (e.g. "led0r", "led0g", "led0b").

use std::io;
use std::path::{Path, PathBuf};

use specular_core::device::Rgb;

/// A row of RGB LEDs, each given by the LED-class names of its red, green and blue parts.
pub struct RgbLeds { leds: Vec<[PathBuf; 3]>, max: [u32; 3] }

impl RgbLeds {
    /// The LEDs with these (red, green, blue) names under `root` (normally /sys/class/leds).
    pub fn new(root: &Path, names: &[[String; 3]]) -> Self {
        let leds: Vec<[PathBuf; 3]> = names.iter().map(|n| n.clone().map(|c| root.join(c))).collect();
        let max = leds.first().map_or([255; 3], |l| l.clone().map(|p| {
            std::fs::read_to_string(p.join("max_brightness")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(255)
        }));
        Self { leds, max }
    }

    /// Sets each LED's colour, scaled by `limit` (0–255, for devices whose LEDs are too bright
    /// at full power). Extra colours are ignored; LEDs without one are turned off.
    pub fn show(&self, colours: &[Rgb], limit: u8) -> io::Result<()> {
        for (i, led) in self.leds.iter().enumerate() {
            let Rgb(r, g, b) = colours.get(i).copied().unwrap_or_default();
            for (c, (path, value)) in led.iter().zip([r, g, b]).enumerate() {
                let level = value as u32 * limit as u32 * self.max[c] / (255 * 255);
                std::fs::write(path.join("brightness"), level.to_string())?;
            }
        }
        Ok(())
    }
}
