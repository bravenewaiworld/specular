//! Bluetooth through BlueZ, with the board's attach service (`BOARD_BT_ATTACH` in the system's
//! board description) powering the controller: up while Bluetooth is on, down while it is off.

use std::io;

use specular_core::wireless::trace::Traced;
use specular_linux::bluez::{Bluez, BtController};

/// The Bluetooth service for `Hardware::bluetooth`: BlueZ, traced, with the board's attach service
/// as its controller if the board names one (otherwise the controller is always powered).
pub fn bluetooth() -> Box<specular_core::wireless::Bluetooth> {
    let bluez = match crate::system::board_env("BOARD_BT_ATTACH") {
        Some(service) => Bluez::with_controller(Box::new(AttachService { service })),
        None => Bluez::new(),
    };
    Box::new(Traced::new("bluetooth", bluez))
}

/// A runit service that powers the controller up and attaches it while running, and powers it off
/// when stopped (the Brick's `xr829-attach`). It is down at start-up, so the controller stays off
/// until Bluetooth is turned on. `sv` returns at once: attaching takes a few seconds more, and a
/// stop asked for while attaching takes effect once attached.
struct AttachService { service: String }

impl BtController for AttachService {
    fn set_power(&mut self, on: bool) -> io::Result<()> {
        crate::command::run("bluetooth", "/usr/bin/sv", &[if on { "up" } else { "down" }, &self.service])
            .map(drop).map_err(io::Error::other)
    }
}
