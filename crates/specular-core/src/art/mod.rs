//! Box art: which game a ROM is, where its picture is kept, and where pictures come from.
//!
//! A ROM is identified offline, from its system's game table (`GameDb`): by checksum, then by
//! the game code in its header or its disc's serial, then by file name. Art comes from the user's
//! own image next to the ROM, else from `cache/art/`, which `ArtSource`s (such as the libretro
//! thumbnail server) fill. This module has no network or image code; the UI crate's art service
//! does that work.

mod db;
mod disc;
mod index;

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub use db::{GameDb, NoMatch};
pub use disc::ps1_serial;
pub use index::{ArtIndex, Entry};

use crate::library::{Game, SystemDecl, extension, zip};

/// Which game a ROM is, by its name in libretro-database, and how that was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity { pub name: String, pub by: Method }

/// How a ROM was identified, from most to least certain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// The file's CRC32 checksum.
    Crc,
    /// The header's game code: the right game, or for a hack, the game it is based on.
    HeaderCode,
    /// The disc's serial: the right game, though perhaps another of its discs (some games' discs
    /// share one serial, and an .m3u is read through its first disc).
    DiscSerial,
    /// The file name, exactly or once `normalise`d.
    Name,
    /// The one game whose name holds every word of the file's, with the same numbers (see
    /// `GameDb::by_name`).
    FuzzyName,
}

impl Method {
    /// Every method, to read one back by its `name`.
    pub const ALL: [Method; 5] = [Method::Crc, Method::HeaderCode, Method::DiscSerial, Method::Name, Method::FuzzyName];

    /// Its name in the index file and logs.
    pub fn name(self) -> &'static str {
        match self {
            Method::Crc => "crc", Method::HeaderCode => "header code", Method::DiscSerial => "disc serial",
            Method::Name => "name", Method::FuzzyName => "fuzzy name",
        }
    }
}

/// Changes whenever `identify` may find something else for the same file and table, so that
/// `ArtIndex` entries made by another version are made again.
const IDENTIFY_VERSION: u32 = 2;

/// Files larger than this are not hashed (too slow to read on a handheld); they are found by
/// header code, disc serial or name.
const MAX_HASHED: u64 = 64 << 20;

/// Works out which game the ROM at `path` (a file of `system`) is; None if the table doesn't
/// know it. A zip isn't unpacked: it is matched by the checksum its directory records for its
/// first ROM file, else by its own file name (its header code can't be read). Logs a line for a
/// game found by fuzzy name, the least certain method, and for one not found, saying why.
pub fn identify(path: &Path, system: &SystemDecl, db: &GameDb) -> io::Result<Option<Identity>> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let found = match by_contents(path, system, db)? {
        Some(id) => Ok(id),
        None => db.by_name(stem).map(|(name, by)| Identity { name: name.to_string(), by }),
    };
    match &found {
        Ok(id) if id.by == Method::FuzzyName => eprintln!("art: {}: by fuzzy name: {}", path.display(), id.name),
        Err(why) => eprintln!("art: {}: not identified: {why}", path.display()),
        Ok(_) => {}
    }
    Ok(found.ok())
}

/// Identifies a ROM by what it holds: its checksum, header code or disc serial.
fn by_contents(path: &Path, system: &SystemDecl, db: &GameDb) -> io::Result<Option<Identity>> {
    let found = |name: Option<&str>, by| name.map(|n| Identity { name: n.to_string(), by });
    if extension(path).as_deref() == Some("zip") {
        let is_rom = |e: &zip::Entry| extension(Path::new(&e.name)).is_some_and(|x| system.extensions.contains(&x.as_str()));
        if let Some(id) = zip::entries(path)?.into_iter().find(is_rom).and_then(|e| found(db.by_crc(e.crc32), Method::Crc)) {
            return Ok(Some(id));
        }
    } else {
        let mut f = File::open(path)?;
        let len = f.metadata()?.len();
        if len <= MAX_HASHED && system.disc_serial.is_none() && let Some(id) = found(db.by_crc(crc32(&mut f, len, system.dump_header)?), Method::Crc) {
            return Ok(Some(id));
        }
        if let Some((offset, n)) = system.header_code {
            let mut code = vec![0; n];
            f.seek(SeekFrom::Start(offset))?;
            // Game codes are upper-case letters and digits; anything else (such as a homebrew
            // ROM's blank header) is not looked up.
            if f.read_exact(&mut code).is_ok() && code.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                && let Some(id) = found(db.by_code(std::str::from_utf8(&code).unwrap_or_default()), Method::HeaderCode) {
                return Ok(Some(id));
            }
        }
        // Failing to read a serial is only logged: the disc may still be known by its name.
        match system.disc_serial.map(|read| read(path)) {
            Some(Ok(Some(serial))) => match found(db.by_code(&serial), Method::DiscSerial) {
                Some(id) => return Ok(Some(id)),
                None => eprintln!("art: {}: disc serial {serial} isn't in the table", path.display()),
            },
            Some(Err(e)) => eprintln!("art: {}: reading the disc serial: {e}", path.display()),
            Some(Ok(None)) | None => {}
        }
    }
    Ok(None)
}

/// The CRC32 of a file of `len` bytes. A dump header of `dump_header` bytes is left out if the
/// file has one, which it does if its size is that much over a multiple of 1 KiB.
fn crc32(f: &mut File, len: u64, dump_header: u64) -> io::Result<u32> {
    let skip = if dump_header > 0 && len % 1024 == dump_header { dump_header } else { 0 };
    f.seek(SeekFrom::Start(skip))?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = vec![0; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 { break; }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize())
}

/// The user's own picture for the game at `rom` (under `roms/`), if there is one: an image next
/// to the ROM with the same name, `.png`, else `.jpg`, else `.jpeg`. Only lower-case extensions
/// are tried, which on FAT cards (case-insensitive) finds any case.
pub fn user_image(roms: &Path, rom: &Path) -> Option<PathBuf> {
    let rom = roms.join(rom);
    ["png", "jpg", "jpeg"].into_iter().map(|ext| rom.with_extension(ext)).find(|p| p.is_file())
}

/// Where the picture for the game at `rom` is cached, shrunk to `IMAGE_SIZE`: `cache/art/`
/// mirrors `roms/`.
pub fn cached_image(cache: &Path, rom: &Path) -> PathBuf { cache.join("art").join(rom).with_extension("png") }

/// Where a system's `ArtIndex` is kept.
pub fn index_path(cache: &Path, system: &str) -> PathBuf { cache.join("identify").join(format!("{system}.json")) }

/// Pictures are stored and shown at most this many pixels wide and high.
pub const IMAGE_SIZE: u32 = 400;

/// Why an `ArtSource` couldn't answer.
#[derive(Debug)]
pub enum ArtError {
    /// The network or the server can't be reached, or a download was cut off; ask again later.
    Offline(String),
    /// Something else went wrong with this game; it counts as having no image.
    Failed(String),
}

/// Somewhere box art comes from. Sources are asked in order, for games with no picture yet.
pub trait ArtSource: Send {
    /// A short name, for logs.
    fn name(&self) -> &'static str;
    /// The game's picture as an image file's bytes (PNG or JPEG), or None if this source has none.
    fn find(&self, game: &Game, identity: Option<&Identity>, system: &SystemDecl) -> Result<Option<Vec<u8>>, ArtError>;
}

/// The address of a game's box art on the libretro thumbnail server, for its libretro-database
/// `name`. Characters the server's file names can't hold (`&*/:`<>?\|"`) are replaced by `_`,
/// as libretro's own tools do.
pub fn thumbnail_url(system: &SystemDecl, name: &str) -> String {
    let file: String = name.chars().map(|c| if "&*/:`<>?\\|\"".contains(c) { '_' } else { c }).collect();
    format!("http://thumbnails.libretro.com/{}/Named_Boxarts/{}.png", percent_encode(system.libretro_name), percent_encode(&file))
}

/// `s` with every byte other than letters, digits and `-._~` written as `%XX`, for a URL path.
fn percent_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[cfg(test)]
mod tests;
