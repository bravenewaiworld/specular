//! Zipped ROMs, for cores that can't open zips: the ROM is unpacked into memory (a memfd), so a
//! core that maps its ROM (mGBA) still can, and nothing is written to the card. The core is given a
//! path ending in the zip's name with the ROM's extension (`Zelda.zip` holding `Zelda (J).fds` is
//! `Zelda.fds`), since some cores look at the path: Nestopia takes a game for a Famicom Disk System
//! one only if its path has `.fds` in it, and names that game's own save file after it. That path
//! is a symbolic link to the memfd's `/proc/self/fd/<n>`, in a folder of this process's own in a
//! private runtime folder (see `link_dir`), which is in memory too.

use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::time::Instant;

use specular_core::library::{extension, zip};

use crate::core::SystemInfo;

/// The memory to leave for everything else: a ROM that would leave less is refused.
const HEADROOM: u64 = 64 << 20;

/// A ROM unpacked into memory. Dropping this frees it and removes its link (the process's whole
/// folder of links: there is one at a time, as one game runs at a time): `path` stops working.
pub struct Unzipped {
    _file: File,
    /// The path to give the core: the link, or the memfd's `/proc/self/fd/<n>` if the link
    /// couldn't be made.
    pub path: PathBuf,
    linked: bool,
}

impl Drop for Unzipped {
    fn drop(&mut self) {
        if self.linked && let Some(dir) = self.path.parent() && let Err(e) = std::fs::remove_dir_all(dir) {
            eprintln!("game: removing {}: {e}", dir.display());
        }
    }
}

/// Unzips `rom` if it is a zip and the core doesn't list zips among its extensions, unless the
/// core sets `block_extract` (libretro's way of asking frontends not to unzip). Otherwise None:
/// the core gets the file as it is.
pub fn if_needed(rom: &Path, info: &SystemInfo) -> Result<Option<Unzipped>, String> {
    let extensions = info.extensions();
    if extension(rom).as_deref() != Some("zip") || info.block_extract || extensions.iter().any(|e| e == "zip") { return Ok(None); }
    unzip(rom, &extensions, &info.name()).map(Some)
}

/// Unpacks the first file in the zip at `path` with one of `extensions` (lower case) into memory.
/// The error is for the player (`core` is the core's name); the log gets the details.
fn unzip(path: &Path, extensions: &[String], core: &str) -> Result<Unzipped, String> {
    let fail = |log: String, player: String| { eprintln!("game: unzipping {}: {log}", path.display()); player };
    let entries = zip::entries(path).map_err(|e| fail(e.to_string(), format!("the zip can't be read ({e})")))?;
    let found = entries.iter().find_map(|e| Some((e, extension(Path::new(&e.name)).filter(|x| extensions.contains(x))?)));
    let Some((rom, ext)) = found else {
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        return Err(fail(format!("it holds {names:?}; {core} plays only .{} files", extensions.join(", .")), format!("the zip holds no game {core} plays")));
    };
    if let Some(free) = available_memory() && rom.size + HEADROOM > free {
        let mb = |n: u64| n >> 20;
        return Err(fail(format!("{} is {} bytes, with {free} bytes of memory available", rom.name, rom.size),
                        format!("the game is too large to unzip ({} MB, with {} MB of memory free)", mb(rom.size), mb(free))));
    }
    let t = Instant::now();
    let name = std::ffi::CString::new(rom.name.replace('\0', "")).unwrap_or_default();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 { let e = std::io::Error::last_os_error(); return Err(fail(format!("memfd_create: {e}"), format!("unzipping failed ({e})"))); }
    let mut file = unsafe { File::from_raw_fd(fd) };
    zip::unpack(path, rom, &mut file).map_err(|e| fail(format!("{}: {e}", rom.name), format!("the zip is damaged ({e})")))?;
    eprintln!("game: unzipped {} ({} bytes) from {} into memory in {} ms", rom.name, rom.size, path.display(), t.elapsed().as_millis());
    let fd_path = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
    let named = path.with_extension(&ext).file_name().map(PathBuf::from).unwrap_or_default();
    let link = link_dir().and_then(|dir| {
        let link = dir.join(&named);
        std::os::unix::fs::symlink(&fd_path, &link).map_err(|e| format!("linking {} to {}: {e}", link.display(), fd_path.display()))?;
        Ok(link)
    });
    let (path, linked) = match link {
        Ok(link) => { eprintln!("game: giving {core} {} (a link to {})", link.display(), fd_path.display()); (link, true) }
        // Cores that tell a game by its contents (mGBA) can still play it.
        Err(e) => { eprintln!("game: {e}; giving {core} {}, without the name {}", fd_path.display(), named.display()); (fd_path, false) }
    };
    Ok(Unzipped { path, linked, _file: file })
}

/// The folder holding each Specular process's folder of links (named by its pid):
/// `$XDG_RUNTIME_DIR/specular` (in memory, and the user's own), else `specular-<uid>` in the
/// temporary folder (`/tmp`, in memory on a handheld).
pub(crate) fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) => PathBuf::from(d).join("specular"),
        None => std::env::temp_dir().join(format!("specular-{}", unsafe { libc::getuid() })),
    }
}

/// An empty folder for this process's links, `<pid>` in `runtime_dir`. One already there (an
/// ended process's with the same pid, or one whose removal failed) is removed first: only one
/// `Unzipped` exists at a time, so nothing in it is in use. So are the folders of processes that
/// have ended (by crashing, say).
fn link_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let (base, uid) = (runtime_dir(), unsafe { libc::getuid() });
    let fail = |what: &str, e: &dyn std::fmt::Display| format!("{what} {}: {e}", base.display());
    if let Err(e) = std::fs::DirBuilder::new().mode(0o700).create(&base) && e.kind() != std::io::ErrorKind::AlreadyExists { return Err(fail("making", &e)); }
    // Another user's folder (in a shared /tmp) mustn't be used.
    let meta = std::fs::symlink_metadata(&base).map_err(|e| fail("reading", &e))?;
    if !meta.is_dir() || meta.uid() != uid { return Err(fail("using", &"not a folder of this user's own")); }
    let ours = std::process::id().to_string();
    for entry in std::fs::read_dir(&base).map_err(|e| fail("reading", &e))?.flatten() {
        let name = entry.file_name();
        let ended = name.to_str().is_some_and(|pid| pid.parse::<u32>().is_ok() && !Path::new("/proc").join(pid).exists());
        if (ended || name == *ours) && let Err(e) = std::fs::remove_dir_all(entry.path()) { eprintln!("game: removing {}: {e}", entry.path().display()); }
    }
    let dir = base.join(ours);
    std::fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|e| fail("making a folder in", &e))?;
    Ok(dir)
}

/// The memory available for new allocations without swapping (`MemAvailable`: free memory plus
/// what the kernel can reclaim), or None if it can't be told (only a failed read is logged).
fn available_memory() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").map_err(|e| eprintln!("game: reading /proc/meminfo: {e}; unzipping without a memory check")).ok()?;
    let kb = info.lines().find_map(|l| l.strip_prefix("MemAvailable:"))?.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
    Some(kb << 10)
}
