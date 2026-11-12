//! Specular on boards running mainline Linux (Crystaline OS's mainline family: the Powkiddy RGB20
//! Pro, later the Orange Pi 5). One binary serves them all: the board is found at start-up from
//! the device tree, and what differs between boards is data in `boards`.

pub mod boards;
pub mod device;
