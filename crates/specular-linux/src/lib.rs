//! Standard Linux implementations of what Specular needs from a device: buttons from evdev, the
//! battery from the power-supply class, lights from the LED class. A port uses these and adds
//! only what its hardware does differently.

pub mod input;
pub mod leds;
pub mod power_supply;
