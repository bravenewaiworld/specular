//! What differs between mainline boards, as data: one entry per board, found at start-up by the
//! device tree's `compatible`. Everything else is found through the standard interfaces.

use std::io;

use specular_core::input::Button;

/// One board.
pub struct Board {
    /// Matched against the entries of /proc/device-tree/compatible.
    pub compatible: &'static str,
    /// As the UI names the device.
    pub name: &'static str,
    /// The panel's pixels per inch (UI sizes scale by ppi / 400).
    pub ppi: u32,
    /// The buttons it has (hints show only these).
    pub buttons: &'static [Button],
    /// The input devices its buttons come from, by evdev name.
    pub inputs: &'static [&'static str],
    /// Its backlight, by /sys/class/backlight name.
    pub backlight: &'static str,
    /// The ALSA device sound plays on.
    pub sound: &'static str,
    /// The volume control.
    pub volume: Mixer,
}

/// An ALSA mixer control that sets the volume of every output: values `0..=max`, where `max` is
/// 0 dB and each step `db_per_step` below it.
pub struct Mixer {
    /// The ALSA card's name, and the control's.
    pub card: &'static str,
    pub control: &'static str,
    pub max: u32,
    pub db_per_step: f64,
}

/// Every board this binary runs on.
pub const BOARDS: &[Board] = &[
    Board {
        compatible: "powkiddy,rgb20-pro",
        name: "Powkiddy RGB20 Pro",
        ppi: 400, // 1024x768 on 65x49 mm
        // FN is Menu (BTN_MODE, from adc-keys). The stick clicks (BTN_THUMBL/R, reported as F1/F2) aren't
        // listed: Specular gives them no use yet.
        buttons: {
            use Button::*;
            &[Up, Down, Left, Right, A, B, X, Y, L1, R1, L2, R2, Select, Start, Menu, VolUp, VolDown, Power]
        },
        inputs: &["gpio-keys-control", "gpio-keys-vol", "rk805 pwrkey", "adc-keys"],
        backlight: "backlight",
        // Card 0 is HDMI; the RK817 codec's card plays on the speaker and the headphones (the jack
        // cuts the speaker in hardware).
        sound: "default:CARD=rk817ext",
        volume: Mixer { card: "rk817ext", control: "Master Playback Volume", max: 255, db_per_step: 95.0 / 255.0 }, // -95 to 0 dB in 255 steps (amixer's dB range)
    },
];

/// The board in `compatible` (the device tree's list, NUL-separated), or an error naming it.
pub fn find(compatible: &[u8]) -> io::Result<&'static Board> {
    let names: Vec<&str> = compatible.split(|&b| b == 0).filter_map(|n| std::str::from_utf8(n).ok()).filter(|n| !n.is_empty()).collect();
    BOARDS.iter().find(|b| names.contains(&b.compatible)).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound,
        format!("no board entry for this device (compatible: {}); boards: {}", names.join(", "),
            BOARDS.iter().map(|b| b.compatible).collect::<Vec<_>>().join(", "))))
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_the_board_by_any_compatible_entry() {
        assert_eq!(super::find(b"powkiddy,rgb20-pro\0rockchip,rk3566\0").unwrap().name, "Powkiddy RGB20 Pro");
        let e = super::find(b"rockchip,rk3566\0").err().unwrap().to_string();
        assert!(e.contains("compatible: rockchip,rk3566"), "{e}");
    }
}
