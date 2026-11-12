//! Standard Linux implementations of what Specular needs from a device: buttons, the headphone jack
//! and rumble through evdev, the battery from the power-supply class, lights from the LED class,
//! the backlight from the backlight class, sound through ALSA (and Bluetooth headphones through bluez-alsa), WiFi through wpa_supplicant,
//! Bluetooth through BlueZ, the display through EGL and DRM/KMS, and its refresh from the
//! framebuffer device or the DRM card. A port uses these
//! and adds only what its hardware does differently.

pub mod backlight;
pub mod bluez;
pub mod card_state;
pub mod egl;
pub mod input;
pub mod interfaces;
pub mod jack;
pub mod kms;
pub mod leds;
pub mod power_supply;
pub mod rumble;
pub mod sound;
pub mod vsync;
pub mod wpa;
