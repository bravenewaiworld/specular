//! The time zone local times (the clock) are shown in. The device keeps the zone, across restarts
//! (the Brick's brick-timezone); its clocks stay in UTC, so a change moves only what is shown.
//! Specular chooses it: by hand, or automatically by asking a lookup service for the network's
//! zone (see app::timezone and docs/time-zone.md).

/// The device's time zone. Calls may be slow (they run a program), so the UI crate makes them on
/// a thread of its own.
pub trait TimeZone: Send {
    /// The zone in effect, such as "Europe/Paris".
    fn get(&mut self) -> Result<String, String>;
    /// Makes `zone` the zone in effect. Refuses a zone the device doesn't have. It may accept
    /// zones `list` leaves out (aliases, such as "US/Eastern").
    fn set(&mut self, zone: &str) -> Result<(), String>;
    /// The zones to choose from, in order.
    fn list(&mut self) -> Result<Vec<String>, String>;
}

/// A command for the time zone's thread (`Effect::TimeZone`).
#[derive(Clone, Debug, PartialEq)]
pub enum ZoneCmd {
    /// Make this the zone in effect; answered with `ZoneEvent::Set`.
    Set(String),
    /// Ask the lookup service for the network's zone; answered with `ZoneEvent::LookedUp`.
    LookUp,
}

/// What the time zone's thread reports. It logs every failure, with its details.
#[derive(Clone, Debug, PartialEq)]
pub enum ZoneEvent {
    /// Once, at start: the zone in effect (None if the device couldn't say) and the zones to
    /// choose from (empty if it couldn't say).
    Started { zone: Option<String>, zones: Vec<String> },
    /// The answer to `ZoneCmd::Set`.
    Set { zone: String, result: Result<(), String> },
    /// The answer to `ZoneCmd::LookUp`: a zone name (checked with `zone_name`, but maybe one the
    /// device doesn't have), or why there is none.
    LookedUp(Result<String, String>),
}

/// The zone name in a lookup's answer (a bare name such as "America/Toronto", maybe with a line
/// break after it). Anything else (an error page, say) is refused, so only a plausible name is
/// ever handed to the device.
pub fn zone_name(answer: &str) -> Result<String, String> {
    let name = answer.trim();
    let plausible = !name.is_empty() && name.len() <= 64 && !name.starts_with('/') && !name.ends_with('/')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_+-/".contains(&b));
    if plausible { Ok(name.to_string()) } else { Err(format!("not a time zone name: {answer:?}")) }
}

/// A stand-in for the desktop build: the zone is kept in memory (UTC at start) and given to the
/// process as $TZ, and the zones are the PC's (`ZONEINFO`), listed as the Brick lists them. Needs
/// $TZ to be set before any thread starts (the desktop's `main` sets it to UTC), so that changing
/// it later only replaces its value (see `set`).
pub struct Simulated { zone: String }

/// Where the PC keeps its zones.
const ZONEINFO: &str = "/usr/share/zoneinfo";

impl Simulated {
    pub fn new() -> Self { Self { zone: "UTC".into() } }
}

impl Default for Simulated {
    fn default() -> Self { Self::new() }
}

impl TimeZone for Simulated {
    fn get(&mut self) -> Result<String, String> { Ok(self.zone.clone()) }

    /// Refuses names that aren't a zone file, as brick-timezone does.
    fn set(&mut self, zone: &str) -> Result<(), String> {
        let path = std::path::Path::new(ZONEINFO).join(zone);
        let tzif = std::fs::read(&path).is_ok_and(|b| b.starts_with(b"TZif"));
        // (`zone_name` allows no dots, so no "..".)
        if zone_name(zone).is_err() || zone.starts_with("posix/") || zone.starts_with("right/") || !tzif {
            return Err(format!("no time zone {zone:?} in {ZONEINFO}"));
        }
        eprintln!("simulated: time zone {} -> {zone}", self.zone);
        // SAFETY: not strictly, as other threads run. But $TZ was set before they started, so
        // glibc only swaps the pointer to its value (it never frees the old one), and a reader
        // sees either value.
        unsafe { std::env::set_var("TZ", zone) };
        self.zone = zone.into();
        Ok(())
    }

    fn list(&mut self) -> Result<Vec<String>, String> {
        let path = format!("{ZONEINFO}/zone.tab");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
        let mut zones: Vec<String> = text.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split('\t').nth(2)).map(String::from).collect();
        zones.sort();
        Ok(std::iter::once("UTC".to_string()).chain(zones).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plausible_zone_names_are_taken_from_a_lookup() {
        assert_eq!(zone_name("America/Toronto\n"), Ok("America/Toronto".into()));
        assert_eq!(zone_name("America/Argentina/Buenos_Aires"), Ok("America/Argentina/Buenos_Aires".into()));
        assert_eq!(zone_name("Etc/GMT+5"), Ok("Etc/GMT+5".into()));
        for bad in ["", "  ", "<html>", "../etc/passwd", "/etc/passwd", "Europe/", "Europe/Paris extra"] {
            assert!(zone_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_stand_in_refuses_what_isnt_a_zone() {
        let mut tz = Simulated::new();
        for bad in ["Mars/Olympus_Mons", "zone.tab", "../zoneinfo/UTC", "posix/UTC"] {
            assert!(tz.set(bad).is_err(), "{bad}");
        }
        assert_eq!(tz.get(), Ok("UTC".into()));
    }
}
