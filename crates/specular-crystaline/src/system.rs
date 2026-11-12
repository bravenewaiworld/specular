//! The system's power commands, how its last run ended, and its SD card service.

use std::io;

use specular_core::wireless::trace::Traced;
use specular_linux::card_state::StateFile;

/// How the last run ended, when not cleanly: one word and a newline.
const LAST_SHUTDOWN: &str = "/run/crystaline/last-shutdown";

/// The SD card's state file, written by `mount-card` (`MOUNT_CARD`), which a recheck runs again.
const CARD_STATE: &str = "/run/card/state";
const MOUNT_CARD: &str = "/usr/libexec/crystaline/mount-card";

/// The board's description, as the system picked it at start-up (`board-env`): plain shell
/// assignments, `BOARD_NAME=value`, the value maybe in single quotes.
const BOARD_ENV: &str = "/run/crystaline/board.env";

/// A value from the board's description, or None if it has none (or isn't readable).
pub fn board_env(name: &str) -> Option<String> {
    let text = std::fs::read_to_string(BOARD_ENV).map_err(|e| eprintln!("board: {BOARD_ENV}: {e}")).ok()?;
    parse_board_env(&text, name)
}

fn parse_board_env(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|l| l.trim().strip_prefix(name)?.strip_prefix('='))
        .map(|v| v.split(" #").next().unwrap_or(v).trim().trim_matches('\'').to_string())
}

/// Runs `program` with `args`; fails unless it exits with success.
pub fn run(program: &str, args: &[&str]) -> io::Result<()> {
    let status = std::process::Command::new(program).args(args).status()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("{program}: {status}"))) }
}

/// Sleeps until the power button wakes the device, through the system's `sleepnow`, which also
/// stops WiFi around sleep (the WiFi drivers can't suspend while connected).
pub fn sleep() -> io::Result<()> { run("/usr/bin/sleepnow", &[]) }

/// Powers off (/sbin/poweroff).
pub fn shutdown() -> io::Result<()> { run("/sbin/poweroff", &[]) }

/// Restarts (/sbin/reboot).
pub fn restart() -> io::Result<()> { run("/sbin/reboot", &[]) }

/// How the last run ended, if not cleanly, for `Power::last_shutdown`. Crystaline OS writes it at
/// boot; Specular deletes it once read, so it is reported once per boot (Crystaline OS's
/// docs/system-image.md, "How the last run ended").
pub fn last_shutdown() -> io::Result<Option<String>> {
    let text = match std::fs::read_to_string(LAST_SHUTDOWN) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        read => read?,
    };
    // Told anyway if it can't be removed; a restarted Specular then tells it again.
    if let Err(e) = std::fs::remove_file(LAST_SHUTDOWN) { eprintln!("power: can't remove {LAST_SHUTDOWN}: {e}"); }
    Ok(Some(text))
}

/// The SD card, which Crystaline OS mounts as it comes and goes (docs/storage.md there).
pub fn card() -> Box<specular_core::card::CardService> {
    Box::new(Traced::new("card", StateFile::new(CARD_STATE, || run(MOUNT_CARD, &[]))))
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_board_description() {
        let text = "# comment\nBOARD_NAME='RGB20 Pro'\nBOARD_WIFI_MODULE=rtw88_8821cs\nBOARD_USB_NET=192.168.8\n";
        assert_eq!(super::parse_board_env(text, "BOARD_NAME").as_deref(), Some("RGB20 Pro"));
        assert_eq!(super::parse_board_env(text, "BOARD_WIFI_MODULE").as_deref(), Some("rtw88_8821cs"));
        assert_eq!(super::parse_board_env(text, "BOARD_WIFI").as_deref(), None);
    }
}
