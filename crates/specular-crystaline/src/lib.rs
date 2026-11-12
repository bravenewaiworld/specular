//! What Specular asks of Crystaline OS, for every device that runs it: its programs and services
//! for WiFi (the `wpa_supplicant` service and the board's driver), SSH (`crystaline-ssh`), file transfer (the `webdav` service), the time zone
//! (`crystaline-timezone`), the SD card (`mount-card`), the CPU's speed (`crystaline-power`) and powering off and restarting, and the files
//! it leaves for Specular.
//! Crystaline OS's docs describe each contract; a device crate adds only its own hardware.

pub mod command;
pub mod cpu;
pub mod ssh;
pub mod system;
pub mod timezone;
pub mod transfer;
pub mod wifi;
