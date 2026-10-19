//! File transfer on the Brick: runit's `webdav` service, in which dufs (a small HTTP and WebDAV
//! file server) serves the SD card on port 80, as WebDAV and as Crystaline OS's own browser page,
//! with no password. The system boots with the service down, and its run script takes it back
//! down when no card is mounted.

use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use specular_core::device::FileTransfer;

use crate::command::run;

const SV: &str = "/usr/bin/sv";
/// dufs takes connections well within a second of starting; this leaves a wide margin, and one
/// that hasn't by then won't.
const START_WAIT: Duration = Duration::from_secs(5);
/// How many of the service's last log lines a failure shows.
const LOG_LINES: usize = 3;

/// The Brick's `FileTransfer`: runit's `webdav` service.
pub struct BrickTransfer;

impl FileTransfer for BrickTransfer {
    /// Started once port 80 takes connections. If it doesn't, the service is stopped rather
    /// than left retrying out of sight, and the error is the service manager's status and the
    /// service's last log lines.
    fn start(&self) -> Result<(), String> {
        run("transfer", SV, &["up", "webdav"])?;
        let port = SocketAddr::from(([127, 0, 0, 1], 80));
        let until = Instant::now() + START_WAIT;
        while Instant::now() < until {
            if TcpStream::connect_timeout(&port, Duration::from_millis(500)).is_ok() {
                eprintln!("transfer: port 80 takes connections");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        eprintln!("transfer: port 80 took no connection within {} s", START_WAIT.as_secs());
        let status = explain(run("transfer", SV, &["status", "webdav"]).unwrap_or_else(|e| e));
        // Only the service's own lines (its syslog tag), not Specular's lines about it.
        let log = format!("/sbin/logread | grep -E '\\.[a-z]+ webdav: ' | tail -n {LOG_LINES}");
        let log = run("transfer", "/bin/sh", &["-c", &log]).map(|l| said(&l)).unwrap_or_default();
        // (A failure to stop is logged; leaving the screen stops it again.)
        let _ = self.stop();
        Err([status, log].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("; "))
    }

    fn stop(&self) -> Result<(), String> { run("transfer", SV, &["-v", "down", "webdav"]).map(|_| ()) }
}

/// `sv status`'s line with what it means. "down: …, want up" is a server that exited, which runit
/// keeps starting again; a plain "down: …" is the run script taking it down, which it does when no
/// card is mounted.
fn explain(status: String) -> String {
    if !status.starts_with("down:") { return status; }
    let why = if status.contains("want up") { "it exited as it started" } else { "it stopped by itself, as it does with no SD card mounted" };
    format!("{why} ({status})")
}

/// What the service said in these lines of the system log, without syslog's time, host and tag
/// ("Oct 19 12:00:00 brick user.notice webdav: Error: …" → "Error: …"), joined with "; ". A line
/// ending in ':' (dufs's "Caused by:") runs on into the next.
fn said(log: &str) -> String {
    let mut out = String::new();
    for line in log.lines().filter_map(|l| Some(l.split_once(" webdav: ")?.1.trim())) {
        if !out.is_empty() { out += if out.ends_with(':') { " " } else { "; " }; }
        out += line;
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_failure_shows_what_the_service_logged() {
        let log = "Oct 19 12:05:00 brick user.notice webdav: Error: Failed to bind `0.0.0.0:80`\n\
                   Oct 19 12:05:00 brick user.notice webdav: Caused by:\n\
                   Oct 19 12:05:00 brick user.notice webdav:     Permission denied (os error 13)\n\
                   Oct 19 12:05:01 brick user.notice webdav: Error: Failed to bind `[::]:80`";
        assert_eq!(super::said(log), "Error: Failed to bind `0.0.0.0:80`; Caused by: Permission denied (os error 13); \
                                      Error: Failed to bind `[::]:80`");
        assert_eq!(super::said(""), "");
    }

    #[test]
    fn a_server_that_exited_is_told_from_one_taken_down() {
        assert_eq!(super::explain("down: webdav: 1s, want up".into()), "it exited as it started (down: webdav: 1s, want up)");
        assert_eq!(super::explain("down: webdav: 3s".into()), "it stopped by itself, as it does with no SD card mounted (down: webdav: 3s)");
        assert_eq!(super::explain("run: webdav: (pid 9) 4s".into()), "run: webdav: (pid 9) 4s");
    }
}
