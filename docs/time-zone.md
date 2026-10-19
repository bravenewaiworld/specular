# Time zone

Status: Implemented · Updated: 2026-10-19

How the zone the clock is shown in is chosen: by hand from the system's list, or automatically by asking a lookup service, which needs the user's permission because it tells that service the network's public address. The system keeps the zone, and keeps its clocks in UTC (Crystaline OS's `docs/power.md`, "Time zone"); choosing it is Specular's.

## Decisions

- **Automatic is the default, and asks first** (2026-10-19). [The ethos](../README.md#ethos) wants a setting that works by itself, but a lookup tells a third party the network's address, so it waits for a yes. The question is put once, unprompted, the first time a lookup would happen (WiFi connected) while the user is browsing (home, a game list, Settings, the radios' screens): in practice right after the first network is joined on the Wi-Fi screen (a device without WiFi counts as connected from the start). Never during a game: it waits for the game to end. It is also put when the user opens Settings → Time zone, or chooses Automatic, without having allowed it. Rejected: asking at first start (no network yet, and nothing to explain it by); only when the user opens Time zone (most people never would, and keep UTC).
- **Declining makes the zone manual, and keeps the zone in effect** (2026-10-19): UTC on a new device, or the one found or picked before. "Choose it myself" also opens the picker. Manual never asks; only choosing Automatic again does. So the permission is one switch (`time.zone_lookup_allowed`) and "declined" needs no value of its own. Rejected: a three-way permission (asked / allowed / refused), which says nothing the mode doesn't.
- **The question stays until answered** (2026-10-19). Menu doesn't leave it, as on Specular's other questions; B declines.
- **ipinfo.io, over plain HTTP** (2026-10-19; the project owner's choice). It answers a bare zone name. Specular's HTTP client has no TLS and the system no certificates (as for box art, [library-and-box-art.md](library-and-box-art.md)). Rejected: ipapi.co (rate-limited, and redirects HTTP to HTTPS), worldtimeapi.org (refused connections), ip-api.com (non-commercial use only); adding rustls with built-in roots (about 1.5 MB, roots to keep current).
- **One lookup per connection** (2026-10-19): when WiFi connects (to a network other than the one last looked up for, or again after disconnecting), and when Automatic is chosen while connected. A failed lookup isn't retried until the next. Signal readings, which report the link again every 10 s, don't count.
- **The system keeps the zone; Specular stores only how it is chosen** (2026-10-19). `time.zone` is "auto" or "manual"; the zone itself is `brick-timezone`'s, kept across restarts. Rejected: the zone in `specular.json` too (two copies that could disagree, e.g. after `brick-timezone set` over SSH).
- **An answer is checked, then offered to the system, which may refuse it** (2026-10-19). Only something shaped like a zone name (letters, digits and `_+-/`, at most 64 characters) is passed on; the system refuses names it has no zone file for. An alias the list leaves out (`US/Eastern`) is accepted if the system has it, and shown as it is. Rejected: mapping aliases to the list's names (needs tzdata's link table, for a name only shown).
- **Failures only show as the zone staying as it was** (2026-10-19). No network, an HTTP error, an unknown zone or the system's program failing are logged (`timezone:`), never shown.
- **The picker is areas, then one area's zones, with the letter index** (2026-10-19). About 420 zones; the biggest area, America, has about 145, which L2 and R2 jump through by first letter. The focus starts on the zone in effect.

## Design

```rust
/// The device's time zone (in `Hardware`; None if Specular can't set it). On the time zone thread.
pub trait TimeZone: Send {
    fn get(&mut self) -> Result<String, String>;
    fn set(&mut self, zone: &str) -> Result<(), String>;   // refuses a zone the device hasn't
    fn list(&mut self) -> Result<Vec<String>, String>;
}

pub enum ZoneCmd { Set(String), LookUp }                     // Effect::TimeZone
pub enum ZoneEvent {                                         // Msg::TimeZone
    Started { zone: Option<String>, zones: Vec<String> },
    Set { zone: String, result: Result<(), String> },
    LookedUp(Result<String, String>),
}
```

The UI's time zone thread owns the device's `TimeZone`. At start it reports the zone and the list; then it carries out `Set` (the device's `set`) and `LookUp` (`GET http://ipinfo.io/timezone`, 10 s timeout, the answer checked with `timezone::zone_name`). The app (`app::timezone`) decides when to look up and when to ask, after every message, and sends `Set` for a lookup's answer only if the zone is still automatic and the answer differs from the zone in effect. A `Set` that succeeds redraws the clock at once.

glibc's `localtime_r` keeps the zone it loaded first, so the clock calls `tzset` before each reading; with `$TZ` unset it checks `/etc/localtime` (a stat) and reloads it if it changed.

The Brick's `TimeZone` runs `/usr/sbin/brick-timezone get|set|list`. The desktop's stand-in (`timezone::Simulated`) keeps the zone in memory, starting at UTC, refuses names that aren't a zone file in the PC's `/usr/share/zoneinfo`, lists `zone.tab`'s zones as `brick-timezone` does, and sets `$TZ` for the process (the desktop's `main` sets it to UTC before any thread starts).

## Findings

- (2026-10-18, upstream) glibc's `tzset` re-reads the zone only when `$TZ` changes, or, with `$TZ` unset, when `/etc/localtime` changed (device, inode or modification time). With `$TZ` set to a file (`:/path`), changes to the file are not seen. Source: glibc `time/tzset.c`.
- (2026-10-19, inferred) Changing `$TZ` with `setenv` while other threads may read the environment is safe in practice once `$TZ` exists: glibc then replaces the pointer in its slot and never frees old values. Hence `$TZ` set before any thread starts. Source: glibc `stdlib/setenv.c`.

## Open questions

- Does the question put right after joining a network take a press meant for the Wi-Fi screen? Watching a few people join a network on the Brick answers it; if so, the question could ignore presses for its first half second.
