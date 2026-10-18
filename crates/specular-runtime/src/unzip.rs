//! Zipped ROMs, for cores that can't open zips: the ROM is unpacked into memory (a memfd) and the
//! core is given its `/proc/self/fd/<n>` path, so a core that maps its ROM (mGBA) still can, and
//! nothing is written to the card. The path has no extension, so this suits only cores that
//! recognise a ROM by its contents (as mGBA does), not by its extension.

use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::time::Instant;

use specular_core::library::{extension, zip};

use crate::core::SystemInfo;

/// The memory to leave for everything else: a ROM that would leave less is refused.
const HEADROOM: u64 = 64 << 20;

/// A ROM unpacked into memory. Dropping this frees it, and `path` (which only works inside this
/// process) stops working.
pub struct Unzipped { _file: File, pub path: PathBuf }

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
    let rom = entries.iter().find(|e| extension(Path::new(&e.name)).is_some_and(|x| extensions.contains(&x)));
    let Some(rom) = rom else {
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
    Ok(Unzipped { path: format!("/proc/self/fd/{}", file.as_raw_fd()).into(), _file: file })
}

/// The memory available for new allocations without swapping (`MemAvailable`: free memory plus
/// what the kernel can reclaim), or None if it can't be told (only a failed read is logged).
fn available_memory() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").map_err(|e| eprintln!("game: reading /proc/meminfo: {e}; unzipping without a memory check")).ok()?;
    let kb = info.lines().find_map(|l| l.strip_prefix("MemAvailable:"))?.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
    Some(kb << 10)
}
