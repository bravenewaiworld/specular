//! Standard Linux implementations of what Specular needs from a device: buttons from evdev, the
//! battery from the power-supply class, lights from the LED class, sound through ALSA (and
//! Bluetooth headphones through bluez-alsa), WiFi through wpa_supplicant, Bluetooth through
//! BlueZ, the display's refresh from the framebuffer device. A port uses these and adds only what
//! its hardware does differently.

pub mod bluez;
pub mod input;
pub mod interfaces;
pub mod leds;
pub mod power_supply;
pub mod sound;
pub mod vsync;
pub mod wpa;
