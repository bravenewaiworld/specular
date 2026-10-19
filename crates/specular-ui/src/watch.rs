//! Notices when the games or the cores on the card change (copied over WiFi or USB, renamed,
//! deleted) while Specular runs, with Linux's inotify on `roms/` and every folder under it, and on
//! the cores' folders.

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::time::Duration;

/// Changes are waited out until none has come for this long, so a copy of many files (or of one
/// large file) leads to one scan, not one per file.
const QUIET: Duration = Duration::from_secs(2);

/// What counts as a change: files and folders appearing, disappearing or finished writing, and a
/// watched folder deleted or moved away (for `roms/` itself: the folder holding it is watched only
/// for it appearing).
const EVENTS: u32 = libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO | libc::IN_CLOSE_WRITE
    | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF;

/// An inotify instance; `watch` adds the folders, `wait_for_change` waits for their changes.
pub struct CardWatch(OwnedFd);

impl CardWatch {
    /// A watch with no folders yet.
    pub fn new() -> std::io::Result<Self> {
        let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if fd < 0 { return Err(std::io::Error::last_os_error()); }
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    /// Watches `roms` and the folders the scan follows under it, and the folder holding it, so a
    /// `roms/` made later is noticed too (and so is any of `folders` beside it, such as `cores/`;
    /// one elsewhere must exist to be watched); and each of `folders` (not the folders in them).
    /// Call it again after each change, for new folders: adding a folder already watched keeps its
    /// one watch, and the kernel drops the watch of a deleted folder. Folders that can't be
    /// watched (missing ones, or past the kernel's limit on watches) are skipped.
    pub fn watch(&self, roms: &Path, folders: &[&Path]) {
        if let Some(parent) = roms.parent() { self.add(parent, libc::IN_CREATE | libc::IN_MOVED_TO); }
        self.add_tree(roms, 0);
        for f in folders { self.add(f, EVENTS); }
    }

    fn add_tree(&self, dir: &Path, depth: usize) {
        self.add(dir, EVENTS);
        if depth == specular_core::library::MAX_DEPTH { return; }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let followed = e.file_name().to_str().is_some_and(|n| !specular_core::library::skip_dir(n));
            if followed && e.file_type().is_ok_and(|t| t.is_dir()) { self.add_tree(&e.path(), depth + 1); }
        }
    }

    fn add(&self, dir: &Path, mask: u32) {
        let Ok(path) = CString::new(dir.as_os_str().as_encoded_bytes()) else { return };
        unsafe { libc::inotify_add_watch(self.0.as_raw_fd(), path.as_ptr(), mask) };
    }

    /// Waits for a change, then until changes have stopped for `QUIET`. (The events are read
    /// only to clear them: any change leads to a whole scan.)
    pub fn wait_for_change(&self) {
        let mut buf = [0u8; 4096];
        let mut timeout = -1; // the first change: wait as long as it takes
        loop {
            let mut pfd = libc::pollfd { fd: self.0.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let ready = unsafe { libc::poll(&mut pfd, 1, timeout) };
            if ready == 0 { return; } // quiet for QUIET
            if ready > 0 { unsafe { libc::read(self.0.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) }; }
            timeout = QUIET.as_millis() as i32;
        }
    }
}
