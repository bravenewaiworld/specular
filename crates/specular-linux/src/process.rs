//! Starting other programs. Programs the running Specular starts go through `command`.

use std::ffi::OsStr;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// A `Command` for `program` whose process starts with no signals blocked.
///
/// Specular blocks SIGTERM and SIGINT in all its threads (one thread waits for them; see
/// specular-ui's `quit_on_signals`). A started program inherits the blocked set from the thread
/// that starts it, and Rust's `Command` doesn't clear it. Without this, SIGTERM and SIGINT (`kill`'s
/// default, Ctrl-C) would never reach the program or anything it starts in turn.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    // SAFETY: this runs in the child between fork and exec, where only async-signal-safe calls
    // may be made (no allocating, no locks, no printing). sigemptyset and sigprocmask are such
    // calls.
    unsafe {
        command.pre_exec(|| {
            let mut none: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut none);
            if libc::sigprocmask(libc::SIG_SETMASK, &none, std::ptr::null_mut()) != 0 { return Err(io::Error::last_os_error()); }
            Ok(())
        });
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The started program's blocked signals, as /proc reports them (a hex mask).
    fn blocked_in_child() -> String {
        let out = command("sh").args(["-c", "grep SigBlk /proc/self/status"]).output().unwrap();
        String::from_utf8(out.stdout).unwrap().split_whitespace().nth(1).unwrap().to_string()
    }

    #[test]
    fn a_started_program_has_no_signals_blocked() {
        // On a thread of its own, so blocking SIGTERM doesn't affect other tests' threads.
        std::thread::spawn(|| {
            unsafe {
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGTERM);
                libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
            }
            assert_eq!(blocked_in_child(), "0000000000000000");
        }).join().unwrap();
    }
}
