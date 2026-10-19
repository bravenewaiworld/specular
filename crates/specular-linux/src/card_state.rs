//! The SD card's state from a file the system replaces whole on each change (Crystaline OS's
//! `/run/card/state`; format: `Card::parse`), followed with inotify on its folder. A missing file
//! reads as `Card::mounted()`: a system that doesn't write one.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use specular_core::card::{Card, CardCmd};
use specular_core::wireless::{Events, Wireless};

/// The card service over a state file (see the module's description).
pub struct StateFile {
    path: PathBuf,
    /// What `CardCmd::Recheck` runs: has the system look at the card again (and update the file).
    recheck: Box<dyn FnMut() -> io::Result<()> + Send>,
    events: Option<Arc<Events<Card>>>,
}

impl StateFile {
    /// The card as the file at `path` says; `recheck` has the system look at the card again (and
    /// update the file), for `CardCmd::Recheck`.
    pub fn new(path: impl Into<PathBuf>, recheck: impl FnMut() -> io::Result<()> + Send + 'static) -> Self {
        Self { path: path.into(), recheck: Box::new(recheck), events: None }
    }
}

/// The card as the file at `path` says now.
pub fn read(path: &Path) -> Card {
    match std::fs::read_to_string(path) {
        Ok(text) => Card::parse(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Card::mounted(),
        // Taken as there, rather than keeping the library from a card that may well be fine.
        Err(e) => { eprintln!("card: can't read {}, so the card counts as mounted: {e}", path.display()); Card::mounted() }
    }
}

impl Wireless<CardCmd, Card> for StateFile {
    /// Reports the card now, then after each change, from a thread of its own. The file is
    /// watched before it is first read, so a change in between isn't missed.
    fn start(&mut self, events: Events<Card>) {
        let events = Arc::new(events);
        self.events = Some(events.clone());
        let path = self.path.clone();
        let watch = watch(&path).map_err(|e| eprintln!("card: can't follow {}, so changes to the card won't be seen: {e}", path.display()));
        events(read(&path));
        let Ok((fd, name)) = watch else { return };
        std::thread::Builder::new().name("card".into()).spawn(move || {
            while changed(&fd, &name) { events(read(&path)); }
            eprintln!("card: stopped following {}", path.display());
        }).map(drop).unwrap_or_else(|e| eprintln!("card: can't start its thread: {e}"));
    }

    /// Runs the recheck, then reports the card, changed or not.
    fn send(&mut self, cmd: CardCmd) {
        match cmd {
            CardCmd::Recheck => if let Err(e) = (self.recheck)() { eprintln!("card: looking at the card again: {e}"); },
        }
        if let Some(events) = &self.events { events(read(&self.path)); }
    }
}

/// An inotify instance on the folder holding `path`, and `path`'s file name.
fn watch(path: &Path) -> io::Result<(OwnedFd, Vec<u8>)> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return Err(io::Error::other("not a file's path")) };
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let dir = CString::new(dir.as_os_str().as_encoded_bytes()).map_err(io::Error::other)?;
    // Renamed into place (the system's way), or written in place (by hand, on a desktop).
    if unsafe { libc::inotify_add_watch(fd.as_raw_fd(), dir.as_ptr(), libc::IN_MOVED_TO | libc::IN_CLOSE_WRITE | libc::IN_DELETE) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((fd, name.as_encoded_bytes().to_vec()))
}

/// Waits until the file `name` in the watched folder changes; false if the watch fails.
fn changed(fd: &OwnedFd, name: &[u8]) -> bool {
    let mut buf = [0u8; 4096];
    loop {
        let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 { return false; }
        // Each event: a libc::inotify_event, then its name (padded with NULs) of `len` bytes.
        let (mut at, header) = (0, std::mem::size_of::<libc::inotify_event>());
        let mut ours = false;
        while at + header <= n as usize {
            let event = unsafe { std::ptr::read_unaligned(buf[at..].as_ptr().cast::<libc::inotify_event>()) };
            let named = &buf[at + header..(at + header + event.len as usize).min(n as usize)];
            ours |= named.split(|&b| b == 0).next() == Some(name);
            at += header + event.len as usize;
        }
        if ours { return true; }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use specular_core::card::CardStatus;

    use super::*;

    #[test]
    fn reports_the_card_then_each_change_and_after_a_recheck() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        let rechecked = path.clone();
        let mut file = StateFile::new(&path, move || std::fs::write(&rechecked, "read-only damaged\nRead-only.\n"));
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        file.start(Box::new(move |c| tx.lock().unwrap().send(c).unwrap()));
        let next = || rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(next(), Card::mounted()); // no file yet
        // Replaced whole, as the system does.
        std::fs::write(dir.path().join("state.new"), "absent\nNo SD card.\n").unwrap();
        std::fs::write(dir.path().join("other"), "not the card").unwrap();
        std::fs::rename(dir.path().join("state.new"), &path).unwrap();
        assert_eq!((next().status, rx.try_recv().ok()), (CardStatus::Absent, None)); // the other files aren't the card
        file.send(CardCmd::Recheck);
        let mut c = next();
        if c.status != CardStatus::ReadOnly { c = next(); } // (its write may have been seen first)
        assert_eq!((c.status, c.reason.as_deref()), (CardStatus::ReadOnly, Some("damaged")));
    }
}
