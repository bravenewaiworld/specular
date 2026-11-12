//! What Specular asks of Crystaline OS, for every device that runs it: its programs and services
//! for SSH (`crystaline-ssh`), file transfer (the `webdav` service), the time zone
//! (`crystaline-timezone`), the SD card (`mount-card`) and powering off and restarting, and the files
//! it leaves for Specular.
//! Crystaline OS's docs describe each contract; a device crate adds only its own hardware.

pub mod command;
pub mod ssh;
pub mod system;
pub mod timezone;
pub mod transfer;
