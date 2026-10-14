//! The Brick's WiFi around wpa_supplicant: the XR829's driver, and runit's `wpa_supplicant` and
//! `wifi-dhcp` services (down at start-up; see their run scripts in the system image).

use std::io;
use std::path::Path;

use specular_linux::wpa::WifiDevice;

/// wpa_supplicant's control socket for the XR829's interface.
pub const SOCKET: &str = "/run/wpa_supplicant/wlan0";

/// Loading the driver powers the chip's WiFi part up and unloading it powers it down again
/// (Bluetooth, on the same chip, is separate and unaffected).
pub struct BrickWifi;

impl WifiDevice for BrickWifi {
    fn set_power(&mut self, on: bool) -> io::Result<()> {
        if on {
            run("/sbin/modprobe", &["xr829_wlan"])?;
            return run("/usr/bin/sv", &["up", "wpa_supplicant", "wifi-dhcp"]);
        }
        // -v: waits for wpa_supplicant to exit, so the driver is no longer in use.
        run("/usr/bin/sv", &["-v", "down", "wifi-dhcp", "wpa_supplicant"])?;
        if Path::new("/sys/module/xr829_wlan").exists() { run("/sbin/rmmod", &["xr829_wlan"])?; }
        Ok(())
    }

    /// Restarts udhcpc, which asks for an address at once rather than at its next retry (up to
    /// 20 s away); the old network's address is flushed first, so it isn't taken for the new one's.
    fn joined(&mut self) {
        let done = run("/sbin/ip", &["-4", "addr", "flush", "dev", "wlan0"]).and_then(|()| run("/usr/bin/sv", &["restart", "wifi-dhcp"]));
        if let Err(e) = done { eprintln!("wifi: asking for an address: {e}"); }
    }
}

/// Runs a program; its output, if it fails, is the error.
fn run(program: &str, args: &[&str]) -> io::Result<()> {
    let out = std::process::Command::new(program).args(args).output()?;
    if out.status.success() { return Ok(()); }
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string() + String::from_utf8_lossy(&out.stdout).trim();
    Err(io::Error::other(format!("{program} {}: {} {said}", args.join(" "), out.status)))
}
