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
