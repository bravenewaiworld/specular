//! The colours people can pick for the accents: hues at about teal's saturation and lightness,
//! so any of them suits the dark UI and reads on the LEDs.

use crate::device::Rgb;

/// A colour from the palette; `id` is what settings store (never renamed once released).
pub struct Colour { pub id: &'static str, pub label: &'static str, pub rgb: Rgb }

/// In the order the settings screen steps through them.
pub const PALETTE: &[Colour] = &[
    Colour { id: "teal", label: "Teal", rgb: Rgb(0x2e, 0xc4, 0xb6) },
    Colour { id: "sky", label: "Sky", rgb: Rgb(0x3a, 0xa6, 0xe8) },
    Colour { id: "indigo", label: "Indigo", rgb: Rgb(0x6f, 0x7c, 0xf2) },
    Colour { id: "violet", label: "Violet", rgb: Rgb(0xa9, 0x70, 0xf0) },
    Colour { id: "rose", label: "Rose", rgb: Rgb(0xe8, 0x60, 0x9a) },
    Colour { id: "coral", label: "Coral", rgb: Rgb(0xf2, 0x66, 0x5e) },
    Colour { id: "amber", label: "Amber", rgb: Rgb(0xf0, 0xa6, 0x3c) },
    Colour { id: "lime", label: "Lime", rgb: Rgb(0x9c, 0xd2, 0x3f) },
];

/// The palette colour stored as `id`.
pub fn find(id: &str) -> Option<Rgb> { PALETTE.iter().find(|c| c.id == id).map(|c| c.rgb) }

/// The one colour outside the palette: a low battery. Like every palette colour, it is light
/// enough for dark text (`Theme.bg`) to be read on it.
pub const WARNING: Rgb = Rgb(0xe5, 0x48, 0x4d);

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance.
    fn luminance(Rgb(r, g, b): Rgb) -> f64 {
        let linear = |c: u8| { let c = c as f64 / 255.0; if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) } };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    /// The battery's number is dark (`Theme.bg`) over its fill and light (`Theme.fg`) over the
    /// empty part, so each colour a fill can have must be readable under the dark one. The
    /// theme's colours are repeated here (from theme.slint), as the core can't read them.
    #[test]
    fn dark_text_reads_on_every_fill_colour() {
        let (bg, fg) = (Rgb(0x08, 0x09, 0x0a), Rgb(0xf2, 0xf2, 0xf2));
        let contrast = |a: Rgb, b: Rgb| { let (a, b) = (luminance(a), luminance(b)); (a.max(b) + 0.05) / (a.min(b) + 0.05) };
        for c in PALETTE.iter().map(|c| (c.id, c.rgb)).chain([("warning", WARNING)]) {
            assert!(contrast(c.1, bg) >= 4.5, "{} is too dark for dark text", c.0);
        }
        // And the light number on the empty part (the gauge's `Theme.bg`; checked against the
        // lighter `Theme.raised` for margin).
        assert!(contrast(fg, Rgb(0x20, 0x22, 0x26)) >= 4.5);
    }
}
