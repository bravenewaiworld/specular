//! The display's refresh from a framebuffer device, with the standard `FBIO_WAITFORVSYNC`, and
//! a pacer that times a drawing loop's frames to it.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

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

/// Paces a drawing loop's continuous frames (an animation) by the display: each is drawn only
/// once the frame before it is on screen, so each refresh gets one new frame, started right
/// after the vsync. After a frame is `presented`, the pacer's thread waits for the next vsync
/// (when that frame goes on screen) and calls `wake`; until then `ready` is false. Nothing waits
/// unless a frame was presented, so an idle screen costs nothing.
///
/// A failed wait counts as a vsync once it returns (our kernel's times out after 50 ms).
pub struct FramePacer {
    /// Whether the last frame presented is on screen.
    on_screen: Arc<AtomicBool>,
    /// Asks the thread to wait for the vsync after a present; None without a vsync, or if the
    /// thread couldn't start.
    request: Option<mpsc::Sender<()>>,
}

impl FramePacer {
    /// `wake` is called on the pacer's thread each time a presented frame is on screen. Without a
    /// `vsync`, `ready` is always true.
    pub fn new(vsync: Option<Box<dyn Vsync>>, wake: impl Fn() + Send + 'static) -> Self {
        let on_screen = Arc::new(AtomicBool::new(true));
        let flag = on_screen.clone();
        let request = vsync.and_then(|mut vsync| {
            let (tx, rx) = mpsc::channel::<()>();
            let spawned = std::thread::Builder::new().name("vsync".into()).spawn(move || {
                let mut failing = false;
                for () in rx {
                    match vsync.wait() {
                        Ok(()) => failing = false,
                        Err(e) if !failing => {
                            eprintln!("vsync: waiting failed ({e}); counted as a vsync until a wait works again");
                            failing = true;
                        }
                        Err(_) => {}
                    }
                    flag.store(true, Ordering::Release);
                    wake();
                }
            });
            if let Err(e) = spawned { eprintln!("vsync: can't start its thread ({e}); drawing unpaced"); return None; }
            Some(tx)
        });
        Self { on_screen, request }
    }

    /// Whether the next frame may be drawn: the last one presented is on screen.
    pub fn ready(&self) -> bool { self.on_screen.load(Ordering::Acquire) }

    /// To be called after presenting a frame that another is to follow at the next refresh.
    pub fn presented(&self) {
        let Some(tx) = &self.request else { return };
        self.on_screen.store(false, Ordering::Release);
        // (The thread is gone only if it panicked, which said so.)
        if tx.send(()).is_err() { self.on_screen.store(true, Ordering::Release); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    /// A vsync that comes when the test sends it, counting the waits for it.
    struct Fake { vsyncs: mpsc::Receiver<io::Result<()>>, waits: Arc<AtomicUsize> }

    impl Vsync for Fake {
        fn wait(&mut self) -> io::Result<()> {
            self.waits.fetch_add(1, Ordering::SeqCst);
            self.vsyncs.recv().unwrap_or(Ok(()))
        }
    }

    fn pacer() -> (FramePacer, mpsc::Sender<io::Result<()>>, mpsc::Receiver<()>, Arc<AtomicUsize>) {
        let (vsync, vsyncs) = mpsc::channel();
        let (woke, wakes) = mpsc::channel();
        let waits = Arc::new(AtomicUsize::new(0));
        let fake = Fake { vsyncs, waits: waits.clone() };
        (FramePacer::new(Some(Box::new(fake)), move || woke.send(()).unwrap()), vsync, wakes, waits)
    }

    const SOON: Duration = Duration::from_millis(50);

    #[test]
    fn a_frame_waits_for_the_one_before_to_be_on_screen() {
        let (p, vsync, wakes, waits) = pacer();
        assert!(p.ready());
        p.presented();
        assert!(!p.ready());
        assert!(wakes.recv_timeout(SOON).is_err());
        vsync.send(Ok(())).unwrap();
        wakes.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(p.ready());
        // Idle: no frame presented, so nothing waits for the display.
        std::thread::sleep(SOON);
        assert_eq!(waits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_failed_wait_counts_as_a_vsync() {
        let (p, vsync, wakes, _) = pacer();
        p.presented();
        vsync.send(Err(io::Error::from_raw_os_error(libc::ETIMEDOUT))).unwrap();
        wakes.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(p.ready());
    }

    #[test]
    fn without_a_vsync_every_frame_is_ready() {
        let p = FramePacer::new(None, || unreachable!());
        p.presented();
        assert!(p.ready());
    }
}
