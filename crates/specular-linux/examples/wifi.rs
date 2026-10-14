//! Drives `WpaSupplicant` by hand, to try it on a device: prints its events (and the trace, on
//! stderr) and reads commands from stdin, one per line:
//!
//! ```text
//! on | off | scan | join <ssid> [password] | forget <ssid> | quit
//! ```
//!
//! Usage: `wifi <control socket> <power-on command> <power-off command> [joined command]`, the
//! commands run by `sh -c`, e.g. on the Brick:
//!
//! ```text
//! wifi /run/wpa_supplicant/wlan0 'modprobe xr829_wlan && sv up wpa_supplicant wifi-dhcp' \
//!     'sv -v down wifi-dhcp wpa_supplicant && rmmod xr829_wlan' \
//!     'ip -4 addr flush dev wlan0 && sv restart wifi-dhcp'
//! ```
//!
//! An SSID with spaces can't be given; a password can (the rest of the line).

use std::io::{self, BufRead};

use specular_core::wireless::wifi::WifiCmd;
use specular_core::wireless::{Secret, Wireless};
use specular_linux::wpa::{WifiDevice, WpaSupplicant};

/// Runs shell commands to power WiFi and to ask for an address.
struct Shell { on: String, off: String, joined: Option<String> }

fn sh(cmd: &str) -> io::Result<()> {
    let status = std::process::Command::new("sh").args(["-c", cmd]).status()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("{cmd}: {status}"))) }
}

impl WifiDevice for Shell {
    fn set_power(&mut self, on: bool) -> io::Result<()> { sh(if on { &self.on } else { &self.off }) }
    fn joined(&mut self) {
        if let Some(Err(e)) = self.joined.as_deref().map(sh) { eprintln!("{e}"); }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [socket, on, off, rest @ ..] = &args[..] else {
        eprintln!("usage: wifi <control socket> <power-on command> <power-off command> [joined command]");
        std::process::exit(2);
    };
    let device = Shell { on: on.clone(), off: off.clone(), joined: rest.first().cloned() };
    let mut wifi = WpaSupplicant::new(socket, Box::new(device));
    wifi.start(Box::new(|e| println!("event {e:?}")));
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let mut words = line.trim().splitn(3, ' ');
        let cmd = match (words.next(), words.next(), words.next()) {
            (Some("on"), ..) => WifiCmd::Power(true),
            (Some("off"), ..) => WifiCmd::Power(false),
            (Some("scan"), ..) => WifiCmd::Scan,
            (Some("join"), Some(ssid), password) => WifiCmd::Join { ssid: ssid.into(), password: password.map(Secret::new) },
            (Some("forget"), Some(ssid), _) => WifiCmd::Forget { ssid: ssid.into() },
            (Some("quit"), ..) => break,
            (Some(""), ..) => continue,
            _ => { eprintln!("on | off | scan | join <ssid> [password] | forget <ssid> | quit"); continue; }
        };
        wifi.send(cmd);
    }
    // Lets the last command finish (the worker runs on its own thread).
    std::thread::sleep(std::time::Duration::from_secs(2));
}
