//! File transfer on the Brick: runit's `webdav` service, Crystaline OS's dufs serving the SD card
//! on port 80 as WebDAV and as its own browser page, with no password. The service is down at
//! start-up, and its run script takes it back down when no card is mounted.

use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use specular_core::device::FileTransfer;

use crate::command::run;

const SV: &str = "/usr/bin/sv";
/// dufs takes connections well within a second of starting; one that hasn't by then won't.
const START_WAIT: Duration = Duration::from_secs(5);
/// How many of the service's last log lines a failure shows.
const LOG_LINES: usize = 3;

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
        let mut status = run("transfer", SV, &["status", "webdav"]).unwrap_or_else(|e| e);
        // The run script takes the service down when it finds no card mounted.
        if status.starts_with("down:") { status = format!("it stopped by itself, as it does with no SD card mounted ({status})"); }
        let log = format!("/sbin/logread | grep ' webdav: ' | tail -n {LOG_LINES}");
        let log = run("transfer", "/bin/sh", &["-c", &log]).map(|l| said(&l)).unwrap_or_default();
        // (A failure to stop is logged; leaving the screen stops it again.)
        let _ = self.stop();
        Err([status, log].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("; "))
    }

    fn stop(&self) -> Result<(), String> { run("transfer", SV, &["-v", "down", "webdav"]).map(|_| ()) }
}

/// What the service said in these lines of the system log, without syslog's time, host and tag
/// ("Oct 19 12:00:00 brick user.notice webdav: Error: …" → "Error: …"), joined with "; ".
fn said(log: &str) -> String {
    log.lines().filter_map(|l| Some(l.split_once(" webdav: ")?.1.trim())).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_failure_shows_what_the_service_logged() {
        let log = "Oct 19 12:05:00 brick user.notice webdav: Error: Failed to bind `0.0.0.0:80`\n\
                   Oct 19 12:05:01 brick user.notice webdav: Error: Failed to bind `[::]:80`";
        assert_eq!(super::said(log), "Error: Failed to bind `0.0.0.0:80`; Error: Failed to bind `[::]:80`");
        assert_eq!(super::said(""), "");
    }
}
