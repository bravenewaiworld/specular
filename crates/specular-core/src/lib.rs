//! Specular's core: what the UI shows and how input moves through it, with no UI toolkit and no
//! hardware. Screens are data (`model`), built by the core and drawn by a fixed set of prefab
//! components in the UI crate; devices deliver `input::ButtonEvent`s.

pub mod app;
pub mod art;
pub mod card;
pub mod cpu;
pub mod device;
pub mod effects;
pub mod input;
pub mod install;
pub mod keyboard;
pub mod launch;
pub mod library;
pub mod list;
pub mod model;
pub mod nav;
pub mod palette;
pub mod repeat;
pub mod rumble;
pub mod settings;
pub mod timezone;
pub mod wireless;
