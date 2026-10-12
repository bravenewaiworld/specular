//! The battery and charger from /sys/class/power_supply.

use std::io;
use std::path::{Path, PathBuf};

use specular_core::device::Battery;

/// The power supplies of a device, found by their type.
pub struct PowerSupply { battery: Option<PathBuf>, mains: Vec<PathBuf> }

fn read(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok().map(|s| s.trim().to_string())
}

impl PowerSupply {
    /// Finds the supplies under `root` (normally /sys/class/power_supply): the first of type
    /// "Battery", and every external one ("USB", "Mains") for whether the device is plugged in.
    pub fn find(root: &Path) -> Self {
        let mut s = Self { battery: None, mains: Vec::new() };
        let Ok(entries) = std::fs::read_dir(root) else { return s };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            match read(&d, "type").as_deref() {
                Some("Battery") if s.battery.is_none() => s.battery = Some(d),
                Some("USB" | "Mains") => s.mains.push(d),
                _ => {}
            }
        }
        s
    }

    /// Reads the battery's state. Each read may be a slow hardware access on some devices.
    pub fn battery(&self) -> io::Result<Battery> {
        let dir = self.battery.as_ref().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no battery"))?;
        let status = read(dir, "status").unwrap_or_default();
        let online = self.mains.iter().any(|m| read(m, "online").as_deref() == Some("1"));
        Ok(Battery {
            percent: read(dir, "capacity").and_then(|c| c.parse::<u8>().ok()).map(|c| c.min(100)),
            charging: status == "Charging",
            plugged: online || matches!(status.as_str(), "Charging" | "Full"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_battery_and_charger() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for (dir, files) in [("bat", [("type", "Battery"), ("capacity", "78\n"), ("status", "Charging\n")]),
                             ("usb", [("type", "USB"), ("online", "1"), ("status", "")])] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            for (f, v) in files { std::fs::write(root.join(dir).join(f), v).unwrap(); }
        }
        let b = PowerSupply::find(root).battery().unwrap();
        assert_eq!(b, Battery { percent: Some(78), charging: true, plugged: true });
        assert!(PowerSupply::find(&root.join("nothing")).battery().is_err());
    }
}
