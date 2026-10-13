//! The display's refresh from a framebuffer device, with the standard `FBIO_WAITFORVSYNC`.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

use specular_core::device::Vsync;

/// `_IOW('F', 0x20, __u32)`
const FBIO_WAITFORVSYNC: libc::c_ulong = 0x4004_4620;

/// Waits for the refresh of a framebuffer, e.g. /dev/fb0.
pub struct FbVsync(File);

impl FbVsync {
    /// Opens `path`; fails if the device doesn't implement the wait (so callers can fall back
    /// to a timer).
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut v = Self(File::options().read(true).write(true).open(path)?);
        v.wait()?;
        Ok(v)
    }
}

impl Vsync for FbVsync {
    fn wait(&mut self) -> io::Result<()> {
        let screen: u32 = 0;
        let r = unsafe { libc::ioctl(self.0.as_raw_fd(), FBIO_WAITFORVSYNC as _, &screen) };
        if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
}
