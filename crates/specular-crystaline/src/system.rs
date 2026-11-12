//! The system's power commands, how its last run ended, and its SD card service.

use std::io;

use specular_core::wireless::trace::Traced;
use specular_linux::card_state::StateFile;

/// How the last run ended, when not cleanly: one word and a newline.
const LAST_SHUTDOWN: &str = "/run/crystaline/last-shutdown";

/// The SD card's state file, written by `mount-card` (`MOUNT_CARD`), which a recheck runs again.
const CARD_STATE: &str = "/run/card/state";
const MOUNT_CARD: &str = "/usr/libexec/crystaline/mount-card";

/// Runs `program` with `args`; fails unless it exits with success.
pub fn run(program: &str, args: &[&str]) -> io::Result<()> {
    let status = std::process::Command::new(program).args(args).status()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("{program}: {status}"))) }
}

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
