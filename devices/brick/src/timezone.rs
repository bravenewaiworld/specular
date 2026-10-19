//! The Brick's time zone: the system's brick-timezone program (Crystaline OS's
//! docs/power.md, "Time zone"): `get` prints the zone, `set <zone>` sets it (exit 1 for a zone
//! it has no file for), `list` prints UTC then tzdata's `zone.tab` zones by name. It keeps the
//! zone across restarts, and logs its own errors to the system log too.

use std::process::Command;

use specular_core::timezone::TimeZone;

const PROGRAM: &str = "/usr/sbin/brick-timezone";

/// The Brick's `TimeZone`, through `/usr/sbin/brick-timezone`.
pub struct BrickTimeZone;

impl TimeZone for BrickTimeZone {
    fn get(&mut self) -> Result<String, String> { run(&["get"]) }
    /// `set` exits 1 for a zone it doesn't have.
    fn set(&mut self, zone: &str) -> Result<(), String> { run(&["set", zone]).map(drop) }
    fn list(&mut self) -> Result<Vec<String>, String> { Ok(run(&["list"])?.lines().map(String::from).collect()) }
}

/// What brick-timezone prints, or what went wrong (its exit status and error message).
fn run(args: &[&str]) -> Result<String, String> {
    let out = Command::new(PROGRAM).args(args).output().map_err(|e| format!("{PROGRAM}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{PROGRAM} {}: {}: {}", args.join(" "), out.status, String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
