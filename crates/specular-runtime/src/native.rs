//! Native cores: a game played by a program of its own, which Specular starts and talks to over a
//! socket and shared memory (`native/specular_native.h` defines the protocol; see
//! docs/native-cores.md). The core draws straight to the screen between `resume` and `pause`;
//! Specular draws only while it is paused, over the last frame it hands back.
//!
//! Threads: one reads the core's messages, one moves its sound from the shared ring to the sound
//! output. Requests (`pause`, `save_state`, ...) are made from one thread at a time, the app's,
//! and wait for the core's answer.

use std::ffi::OsString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::*};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use specular_core::device::SoundOut;
use specular_core::input::Button;

pub mod package;

use crate::sound::Sound;
use crate::FrameSlot;

pub const PROTOCOL: u32 = 1;
const MESSAGE_MAX: usize = 4096;
const SOUND_FRAMES: usize = 8192;
const FRAME_PIXELS: usize = 1024 * 1024;
/// The fd number the core's end of the socket gets in its process.
const CORE_FD: RawFd = 3;

/// How long the core has to answer: its hello, a pause, a resume, quitting. Making or loading a
/// state may take longer (`STATE_TIMEOUT`).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(5);
const STATE_TIMEOUT: Duration = Duration::from_secs(20);

mod msg {
    pub const HELLO: u32 = 1;
    pub const STARTED: u32 = 2;
    pub const PAUSED: u32 = 3;
    pub const RESUMED: u32 = 4;
    pub const STATE_DONE: u32 = 5;
    pub const SAVED: u32 = 6;
    pub const RUMBLE: u32 = 7;
    pub const ERROR: u32 = 8;
    pub const START: u32 = 64;
    pub const PAUSE: u32 = 65;
    pub const RESUME: u32 = 66;
    pub const SAVE_STATE: u32 = 67;
    pub const LOAD_STATE: u32 = 68;
    pub const QUIT: u32 = 69;
}

/// `SpnMessage`'s header.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Header { kind: u32, text_len: u32, a: u64, b: u64 }

/// `SpnShared`, field for field.
#[repr(C)]
struct Shared {
    protocol: u32,
    size: u32,
    buttons_seq: AtomicU32,
    buttons: AtomicU32,
    buttons_time_ns: AtomicU64,
    sound_rate: AtomicU32,
    reserved0: u32,
    sound_written: AtomicU64,
    sound_read: AtomicU64,
    sound: [[i16; 2]; SOUND_FRAMES],
    frames: AtomicU64,
    frame_time_ns: AtomicU64,
    frame_width: u32,
    frame_height: u32,
    frame: [u8; FRAME_PIXELS * 4],
}

/// The core's bit for one of Specular's buttons (`SpnButton`); Menu, the volume keys and Power
/// have none.
pub fn button_bit(b: Button) -> Option<u32> {
    use Button::*;
    let i = match b {
        Up => 0, Down => 1, Left => 2, Right => 3, A => 4, B => 5, X => 6, Y => 7,
        L1 => 8, R1 => 9, L2 => 10, R2 => 11, Select => 12, Start => 13, F1 => 14, F2 => 15,
        Menu | VolUp | VolDown | Power | Mute => return None,
    };
    Some(1 << i)
}

/// What to run.
pub struct NativeLaunch {
    /// The core's program, and arguments before `--specular FD`.
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// The game's package, its saves' folder and a cache folder (both made if missing).
    pub package: PathBuf,
    pub saves: PathBuf,
    pub cache: PathBuf,
    /// The local time at launch, in seconds since 2000-01-01.
    pub clock: u64,
    pub options: Vec<(String, String)>,
}

/// Where the game's output goes.
pub struct NativeOutputs {
    pub sound: Option<Box<dyn SoundOut>>,
    /// Gets the frame on screen each time the core pauses.
    pub frames: Arc<FrameSlot>,
    /// The game saved (for the effects).
    pub on_save: Box<dyn Fn() + Send>,
    /// The game's rumble strength changed (0 to 65535).
    pub on_rumble: Box<dyn Fn(u16) + Send>,
    /// The core stopped without being asked to: what it said (its `SPN_ERROR`, for the player),
    /// if anything. Called once, on the reader's thread; the core no longer has the screen.
    pub on_stop: Box<dyn FnOnce(Option<String>) + Send>,
}

/// What the core said as the game started.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeInfo { pub name: String, pub version: String, pub width: u32, pub height: u32, pub fps: f64 }

enum Event { Message(Header, String), Closed }

/// Why a core couldn't start a game.
#[derive(Debug, PartialEq)]
pub enum StartFailure {
    /// The core's own words, for the player (its `SPN_ERROR`).
    Said(String),
    /// What went wrong, for the log.
    Other(String),
}

impl std::fmt::Display for StartFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self { StartFailure::Said(why) => write!(f, "the core: {why}"), StartFailure::Other(why) => f.write_str(why) }
    }
}

/// A game played by a native core. Dropping it quits the game.
pub struct NativeSession {
    child: Mutex<Child>,
    socket: Arc<OwnedFd>,
    shared: Arc<Mapping>,
    events: Mutex<mpsc::Receiver<Event>>,
    stopping: Arc<AtomicBool>,
    sound: Arc<Mutex<Sound>>,
    sound_thread: Option<std::thread::JoinHandle<()>>,
    reader: Option<std::thread::JoinHandle<()>>,
    frames: Arc<FrameSlot>,
    info: NativeInfo,
    /// Whether the core is paused; held across each request, so requests come one at a time.
    paused: Mutex<bool>,
    /// `paused`'s opposite, for the watchdog's thread.
    running_flag: Arc<AtomicBool>,
}

impl NativeSession {
    /// Starts the core, gives it the game, and waits for it to say it has started. The game is
    /// then paused: Specular still has the screen until `resume`.
    pub fn start(launch: NativeLaunch, out: NativeOutputs) -> Result<NativeSession, StartFailure> {
        let said = Arc::new(Mutex::new(None));
        let session = Self::start_with(launch, out, said.clone()).map_err(StartFailure::Other)?;
        session.map_err(|e| match said.lock().unwrap().take() { Some(why) => StartFailure::Said(why), None => StartFailure::Other(e) })
    }

    /// `start`: Err only before the core runs; Ok(Err) once it has (the session is dropped, which
    /// ends it, and its reader has put what the core said, if anything, in `said`).
    fn start_with(launch: NativeLaunch, out: NativeOutputs, said: Arc<Mutex<Option<String>>>) -> Result<Result<NativeSession, String>, String> {
        for dir in [&launch.saves, &launch.cache] {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let shared = Arc::new(Mapping::new()?);
        let (ours, theirs) = socket_pair()?;
        let theirs_fd = theirs.as_raw_fd();
        let mut command = Command::new(&launch.program);
        command.args(&launch.args).arg("--specular").arg(CORE_FD.to_string())
            .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            command.pre_exec(move || {
                // dup2 leaves the new descriptor without FD_CLOEXEC, but does nothing if it is
                // already that number.
                let r = if theirs_fd == CORE_FD { libc::fcntl(CORE_FD, libc::F_SETFD, 0) } else { libc::dup2(theirs_fd, CORE_FD) };
                if r < 0 { return Err(io::Error::last_os_error()); }
                Ok(())
            });
        }
        let child = command.spawn().map_err(|e| format!("{}: {e}", launch.program.display()))?;
        drop(theirs);
        let socket = Arc::new(ours);

        let (tx, events) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let reader = spawn_reader(socket.clone(), tx, out.on_save, out.on_rumble, out.on_stop, stopping.clone(), said);
        let mut session = NativeSession {
            child: Mutex::new(child), socket, shared, events: Mutex::new(events), stopping,
            sound: Arc::new(Mutex::new(Sound::new(out.sound, 48_000.0, 1.0))), sound_thread: None,
            reader: Some(reader), frames: out.frames, info: NativeInfo { name: String::new(), version: String::new(), width: 0, height: 0, fps: 0.0 },
            paused: Mutex::new(true), running_flag: Arc::new(AtomicBool::new(false)),
        };
        let result = session.handshake(&launch);
        // (A core that failed is ended as the session drops.)
        Ok(result.map(|()| session))
    }

    fn handshake(&mut self, launch: &NativeLaunch) -> Result<(), String> {
        let session = self;
        let (hello, text) = session.wait_for(msg::HELLO, ANSWER_TIMEOUT)?;
        if hello.a != PROTOCOL as u64 { return Err(format!("the core speaks protocol {}, not {PROTOCOL}", hello.a)); }
        let (name, version) = text.split_once('\t').unwrap_or((&text, ""));
        (session.info.name, session.info.version) = (name.to_owned(), version.to_owned());

        let mut keys = format!("package={}\nsaves={}\ncache={}\nclock={}\n",
            launch.package.display(), launch.saves.display(), launch.cache.display(), launch.clock);
        for (k, v) in &launch.options { keys += &format!("option.{k}={v}\n"); }
        send(&session.socket, msg::START, 0, 0, &keys, Some(session.shared.fd.as_raw_fd())).map_err(|e| format!("starting the core: {e}"))?;
        // Booting reads the disc: allow more than an answer's time.
        let (started, fps) = session.wait_for(msg::STARTED, STATE_TIMEOUT)?;
        (session.info.width, session.info.height) = (started.a as u32, started.b as u32);
        session.info.fps = fps.trim().parse().unwrap_or(60.0);
        session.sound_thread = Some(spawn_sound(session.shared.clone(), session.sound.clone(), session.stopping.clone()));
        Ok(())
    }

    /// The core's process ID (for ending one that has stopped responding).
    pub fn pid(&self) -> u32 { self.child.lock().unwrap().id() }

    pub fn info(&self) -> &NativeInfo { &self.info }

    /// The buttons held now, as `button_bit`s. Doesn't wait.
    pub fn set_buttons(&self, bits: u32) {
        let s = self.shared.get();
        let seq = s.buttons_seq.load(Relaxed);
        s.buttons_seq.store(seq.wrapping_add(1), Relaxed);
        std::sync::atomic::fence(Release);
        s.buttons.store(bits, Relaxed);
        s.buttons_time_ns.store(monotonic_ns(), Relaxed);
        s.buttons_seq.store(seq.wrapping_add(2), Release);
    }

    /// Frames the core has presented so far (for the watchdog and the CPU minimum).
    pub fn frames(&self) -> u64 { self.shared.get().frames.load(Relaxed) }

    /// Hands the screen to the core: Specular must have stopped drawing.
    pub fn resume(&self) -> Result<(), String> {
        let mut paused = self.paused.lock().unwrap();
        if !*paused { return Ok(()); }
        self.request(msg::RESUME, "", msg::RESUMED, ANSWER_TIMEOUT)?;
        *paused = false;
        self.running_flag.store(true, Relaxed);
        Ok(())
    }

    /// Whether the core has the screen (between `resume` and `pause`).
    pub fn running(&self) -> bool { !*self.paused.lock().unwrap() }

    /// Stops the game and takes the screen back: once this returns, the core draws nothing, and
    /// the frame it showed is in `NativeOutputs::frames`.
    pub fn pause(&self) -> Result<(), String> {
        let mut paused = self.paused.lock().unwrap();
        if *paused { return Ok(()); }
        // Paused even if the core doesn't answer: it may still draw, but Specular takes the
        // screen back regardless (and the watchdog ends a core that hangs).
        *paused = true;
        self.running_flag.store(false, Relaxed);
        self.request(msg::PAUSE, "", msg::PAUSED, ANSWER_TIMEOUT)?;
        let s = self.shared.get();
        let (w, h) = (s.frame_width, s.frame_height);
        if w > 0 && h > 0 && (w as usize) * (h as usize) <= FRAME_PIXELS {
            let mut f = self.frames.spare();
            f.width = w;
            f.height = h;
            f.rgba.clear();
            f.rgba.extend_from_slice(&s.frame[..(w * h * 4) as usize]);
            self.frames.publish(f);
        }
        Ok(())
    }

    /// Saves the game's state to `path`. Pauses first if needed.
    pub fn save_state(&self, path: &Path) -> io::Result<()> { self.state(msg::SAVE_STATE, path) }
    /// Loads the game's state from `path`. Pauses first if needed.
    pub fn load_state(&self, path: &Path) -> io::Result<()> { self.state(msg::LOAD_STATE, path) }

    /// Plays the game's sound on `out` from now on (None: silence). Open it beforehand: opening
    /// can block.
    pub fn set_sound(&self, out: Option<Box<dyn SoundOut>>) { self.sound.lock().unwrap().set_out(out); }

    /// Asks the core to write its saves and exit; kills it if it hasn't within the answer's time.
    pub fn quit(mut self) -> Result<(), String> { self.stop() }

    /// Ends the core at once, without asking it to save (the storage its saves go to is gone).
    pub fn abandon(self) {
        self.stopping.store(true, SeqCst);
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Calls `stuck` (once, on a thread of its own) if the core presents no frame for `limit`
    /// while it has the screen. Ends with the session.
    pub fn watch(&self, limit: Duration, stuck: impl FnOnce() + Send + 'static) {
        let (shared, stopping) = (Arc::downgrade(&self.shared), self.stopping.clone());
        let running = self.running_flag.clone();
        std::thread::spawn(move || {
            let (mut last, mut since) = (u64::MAX, Instant::now());
            while let Some(shared) = shared.upgrade() {
                if stopping.load(Relaxed) { return; }
                let frames = shared.get().frames.load(Relaxed);
                if frames != last || !running.load(Relaxed) { (last, since) = (frames, Instant::now()); }
                else if since.elapsed() >= limit { return stuck(); }
                drop(shared);
                std::thread::sleep(limit / 8);
            }
        });
    }

    fn stop(&mut self) -> Result<(), String> {
        if self.stopping.swap(true, SeqCst) { return Ok(()); }
        let mut result = self.pause();
        if result.is_ok() {
            result = send(&self.socket, msg::QUIT, 0, 0, "", None).map_err(|e| e.to_string());
        }
        let deadline = Instant::now() + ANSWER_TIMEOUT;
        loop {
            let mut child = self.child.lock().unwrap();
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break,
                Ok(Some(status)) => { result = result.and(Err(format!("the core ended with {status}"))); break }
                Ok(None) if Instant::now() < deadline => { drop(child); std::thread::sleep(Duration::from_millis(10)) }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    result = result.and(Err("the core didn't quit in time; it was ended".into()));
                    break;
                }
            }
        }
        // The reader ends at the socket's close; the sound thread at `stopping`.
        if let Some(t) = self.reader.take() { let _ = t.join(); }
        if let Some(t) = self.sound_thread.take() { let _ = t.join(); }
        result
    }

    fn state(&self, kind: u32, path: &Path) -> io::Result<()> {
        self.pause().map_err(io::Error::other)?;
        let (h, text) = self.request(kind, &path.to_string_lossy(), msg::STATE_DONE, STATE_TIMEOUT).map_err(io::Error::other)?;
        if h.a == 0 { Ok(()) } else { Err(io::Error::new(io::Error::from_raw_os_error(h.a as i32).kind(), text)) }
    }

    fn request(&self, kind: u32, text: &str, answer: u32, timeout: Duration) -> Result<(Header, String), String> {
        send(&self.socket, kind, 0, 0, text, None).map_err(|e| format!("the core has stopped: {e}"))?;
        self.wait_for(answer, timeout)
    }

    /// Waits for message `kind`; other messages are handled by the reader and never arrive here.
    fn wait_for(&self, kind: u32, timeout: Duration) -> Result<(Header, String), String> {
        let events = self.events.lock().unwrap();
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match events.recv_timeout(left) {
                Ok(Event::Message(h, text)) if h.kind == kind => return Ok((h, text)),
                Ok(Event::Message(h, _)) => eprintln!("native: unexpected message {} while waiting for {kind}", h.kind),
                Ok(Event::Closed) | Err(mpsc::RecvTimeoutError::Disconnected) => return Err("the core has stopped".into()),
                Err(mpsc::RecvTimeoutError::Timeout) => return Err(format!("the core didn't answer within {} s", timeout.as_secs())),
            }
        }
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        if let Err(e) = self.stop() { eprintln!("native: quitting: {e}"); }
    }
}

fn spawn_reader(socket: Arc<OwnedFd>, tx: mpsc::Sender<Event>, on_save: Box<dyn Fn() + Send>, on_rumble: Box<dyn Fn(u16) + Send>,
                on_stop: Box<dyn FnOnce(Option<String>) + Send>, stopping: Arc<AtomicBool>, said: Arc<Mutex<Option<String>>>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new().name("native core".into()).spawn(move || {
        let mut error = None;
        loop {
            match receive(&socket) {
                Ok(Some((h, text))) => match h.kind {
                    msg::SAVED => on_save(),
                    msg::RUMBLE => on_rumble(h.a.min(65535) as u16),
                    msg::ERROR => error = Some(text),
                    _ => { let _ = tx.send(Event::Message(h, text)); }
                },
                Ok(None) => break,
                Err(e) => { eprintln!("native: reading the core's messages: {e}"); break }
            }
        }
        // (Before Closed, so that `start` finds it.)
        if let Some(why) = &error { *said.lock().unwrap() = Some(why.clone()); }
        let _ = tx.send(Event::Closed);
        if !stopping.load(SeqCst) {
            on_stop(error);
        }
    }).expect("thread")
}

/// Moves the core's sound to the output every few milliseconds.
fn spawn_sound(shared: Arc<Mapping>, sound: Arc<Mutex<Sound>>, stopping: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new().name("native sound".into()).spawn(move || {
        let s = shared.get();
        let mut rate = 0;
        let mut buf = Vec::with_capacity(SOUND_FRAMES);
        while !stopping.load(Relaxed) {
            std::thread::sleep(Duration::from_millis(4));
            let new_rate = s.sound_rate.load(Acquire);
            let mut sound = sound.lock().unwrap();
            if new_rate != rate && new_rate != 0 {
                rate = new_rate;
                sound.set_rate(rate as f64);
            }
            let (read, written) = (s.sound_read.load(Relaxed), s.sound_written.load(Acquire));
            let n = written.saturating_sub(read).min(SOUND_FRAMES as u64);
            if n == 0 { continue; }
            buf.clear();
            buf.extend((read..read + n).map(|i| s.sound[i as usize % SOUND_FRAMES]));
            s.sound_read.store(read + n, Release);
            sound.play(&buf);
        }
    }).expect("thread")
}

/// The shared memory: a memfd mapped here, passed to the core.
struct Mapping { fd: OwnedFd, ptr: *mut Shared }

// SAFETY: every field the two threads share is atomic, or written only while the other side
// doesn't read it (the frame, before SPN_PAUSED).
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    fn new() -> Result<Mapping, String> {
        let size = std::mem::size_of::<Shared>();
        // SAFETY: plain system calls; the mapping is checked before use.
        unsafe {
            let fd = libc::memfd_create(c"specular-native".as_ptr(), libc::MFD_CLOEXEC);
            if fd < 0 { return Err(format!("shared memory: {}", io::Error::last_os_error())); }
            let fd = OwnedFd::from_raw_fd(fd);
            if libc::ftruncate(fd.as_raw_fd(), size as libc::off_t) != 0 { return Err(format!("shared memory: {}", io::Error::last_os_error())); }
            let ptr = libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd.as_raw_fd(), 0);
            if ptr == libc::MAP_FAILED { return Err(format!("shared memory: {}", io::Error::last_os_error())); }
            let ptr = ptr as *mut Shared;
            (*ptr).protocol = PROTOCOL;
            (*ptr).size = size as u32;
            Ok(Mapping { fd, ptr })
        }
    }
    fn get(&self) -> &Shared { unsafe { &*self.ptr } }
}

impl Drop for Mapping {
    fn drop(&mut self) { unsafe { libc::munmap(self.ptr.cast(), std::mem::size_of::<Shared>()); } }
}

fn socket_pair() -> Result<(OwnedFd, OwnedFd), String> {
    let mut fds = [0; 2];
    // SAFETY: socketpair writes two descriptors into `fds`.
    if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) } != 0 {
        return Err(format!("socket: {}", io::Error::last_os_error()));
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn send(socket: &OwnedFd, kind: u32, a: u64, b: u64, text: &str, fd: Option<RawFd>) -> io::Result<()> {
    let header = Header { kind, text_len: text.len() as u32, a, b };
    let mut packet = Vec::with_capacity(std::mem::size_of::<Header>() + text.len());
    // SAFETY: Header is plain old data.
    packet.extend_from_slice(unsafe { std::slice::from_raw_parts((&header as *const Header).cast::<u8>(), std::mem::size_of::<Header>()) });
    packet.extend_from_slice(text.as_bytes());
    if packet.len() > MESSAGE_MAX { return Err(io::Error::other("message too long")); }
    let mut iov = libc::iovec { iov_base: packet.as_mut_ptr().cast(), iov_len: packet.len() };
    let mut control = [0u8; 64];
    // SAFETY: msghdr is filled in for one iovec and, with `fd`, one SCM_RIGHTS descriptor.
    unsafe {
        let mut m: libc::msghdr = std::mem::zeroed();
        m.msg_iov = &mut iov;
        m.msg_iovlen = 1;
        if let Some(fd) = fd {
            m.msg_control = control.as_mut_ptr().cast();
            m.msg_controllen = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as _;
            let c = libc::CMSG_FIRSTHDR(&m);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
            std::ptr::write_unaligned(libc::CMSG_DATA(c).cast::<RawFd>(), fd);
        }
        if libc::sendmsg(socket.as_raw_fd(), &m, libc::MSG_NOSIGNAL) < 0 { return Err(io::Error::last_os_error()); }
    }
    Ok(())
}

/// The next message; None once the core has closed its end.
fn receive(socket: &OwnedFd) -> io::Result<Option<(Header, String)>> {
    let mut packet = [0u8; MESSAGE_MAX];
    let n = loop {
        // SAFETY: reads at most MESSAGE_MAX bytes into `packet`.
        let n = unsafe { libc::recv(socket.as_raw_fd(), packet.as_mut_ptr().cast(), MESSAGE_MAX, 0) };
        if n >= 0 { break n as usize; }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted { return Err(e); }
    };
    if n == 0 { return Ok(None); }
    let size = std::mem::size_of::<Header>();
    if n < size { return Err(io::Error::other(format!("a message of {n} bytes"))); }
    // SAFETY: `packet` holds at least a header's bytes.
    let h: Header = unsafe { std::ptr::read_unaligned(packet.as_ptr().cast()) };
    let text = String::from_utf8_lossy(&packet[size..n.min(size + h.text_len as usize)]).into_owned();
    Ok(Some((h, text)))
}

fn monotonic_ns() -> u64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t) };
    t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64
}

#[cfg(test)]
mod tests;
