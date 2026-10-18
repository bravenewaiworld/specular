//! PlayStation disc serials, read from the disc image, as a checksum is of no use for discs
//! (an image is too large to hash, and a .cue or .m3u isn't the disc itself): the boot file
//! that `SYSTEM.CNF` names (`BOOT = cdrom:\SLUS_012.34;1` is "SLUS-01234"), or a PBP's
//! `DISC_ID`. Only a few sectors are read, and every offset is checked, so a truncated or
//! garbage file is an error, never a panic or a large read.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::library::{extension, listed_files};

/// The serial of the PlayStation disc at `path`, as game tables list it. A .cue is read through
/// its first file (track 1 holds the data), an .m3u through its first disc. None if the image
/// gives none: a .chd (compressed; not read), or a disc that boots `PSX.EXE`. An error if the
/// image can't be read or isn't a disc image.
pub fn ps1_serial(path: &Path) -> io::Result<Option<String>> {
    match extension(path).as_deref() {
        Some("bin" | "iso" | "img") => iso_serial(&mut File::open(path)?),
        Some("pbp") => pbp_serial(&mut File::open(path)?),
        Some("cue") => iso_serial(&mut File::open(first_listed(path)?)?),
        Some("m3u") => {
            let disc = first_listed(path)?;
            // A playlist that lists a playlist isn't followed: it could loop.
            if extension(&disc).as_deref() == Some("m3u") { Ok(None) } else { ps1_serial(&disc) }
        }
        _ => Ok(None),
    }
}

fn bad(what: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, what) }

/// The first file the .cue sheet or .m3u playlist at `path` lists.
fn first_listed(path: &Path) -> io::Result<PathBuf> {
    let text = String::from_utf8_lossy(&read_at(&mut File::open(path)?, 0, 64 << 10, false)?).into_owned();
    let first = listed_files(extension(path).as_deref() == Some("cue"), &text).next().ok_or_else(|| bad("lists no files"))?;
    Ok(path.parent().unwrap_or(Path::new("")).join(first))
}

/// Up to `len` bytes of `f` from `at`; with `exact`, fewer is an `UnexpectedEof` error.
fn read_at(f: &mut File, at: u64, len: usize, exact: bool) -> io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(at))?;
    let mut buf = Vec::with_capacity(len);
    f.take(len as u64).read_to_end(&mut buf)?;
    if exact && buf.len() < len { return Err(io::ErrorKind::UnexpectedEof.into()); }
    Ok(buf)
}

fn u16_at(b: &[u8], i: usize) -> Option<usize> { Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as usize) }
fn u32_at(b: &[u8], i: usize) -> Option<usize> { Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as usize) }

/// "SLUS_012.34" or "SLUS01234" as "SLUS-01234": four letters and five digits, else None.
fn serial_from(name: &str) -> Option<String> {
    let s = name.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_uppercase();
    let (letters, digits) = s.split_at_checked(4)?;
    (letters.bytes().all(|c| c.is_ascii_alphabetic()) && digits.len() == 5 && digits.bytes().all(|c| c.is_ascii_digit()))
        .then(|| format!("{letters}-{digits}"))
}

const SECTOR: usize = 2048;

/// How an image may store a disc's 2048-byte data sectors: (bytes per sector, where the data
/// starts in each). Raw Mode 2 (PlayStation discs: 12 sync, 4 header and 8 subheader bytes),
/// raw Mode 1, plain ISO, and Mode 2 without sync and header.
const LAYOUTS: [(u64, u64); 4] = [(2352, 24), (2352, 16), (2048, 0), (2336, 8)];

/// `count` data sectors from sector `lba`.
fn sectors(f: &mut File, (size, offset): (u64, u64), lba: usize, count: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(count * SECTOR);
    for n in lba as u64..lba as u64 + count as u64 { out.extend(read_at(f, n * size + offset, SECTOR, true)?); }
    Ok(out)
}

/// The serial from `SYSTEM.CNF` in the root of an ISO 9660 image.
fn iso_serial(f: &mut File) -> io::Result<Option<String>> {
    // The primary volume descriptor is sector 16: type 1, "CD001".
    let (layout, pvd) = LAYOUTS.into_iter().find_map(|layout| {
        sectors(f, layout, 16, 1).ok().filter(|pvd| pvd[..6] == *b"\x01CD001").map(|pvd| (layout, pvd))
    }).ok_or_else(|| bad("no ISO 9660 filesystem"))?;
    // The root directory's record (at 156): its first sector at +2, its size at +10.
    let (root, root_len) = (u32_at(&pvd, 158).unwrap_or_default(), u32_at(&pvd, 166).unwrap_or_default());
    // A PlayStation disc's root lists a handful of files; 32 sectors is plenty.
    let dir = sectors(f, layout, root, root_len.div_ceil(SECTOR).clamp(1, 32))?;
    let Some((cnf, cnf_len)) = find_file(&dir, "SYSTEM.CNF") else { return Ok(None) };
    let cnf = sectors(f, layout, cnf, 1)?;
    let text = String::from_utf8_lossy(&cnf[..cnf_len.min(SECTOR)]);
    // BOOT = cdrom:\SLUS_012.34;1
    let boot = text.lines().filter_map(|l| l.split_once('=')).find(|(k, _)| k.trim().eq_ignore_ascii_case("BOOT"));
    Ok(boot.and_then(|(_, v)| {
        let file = v.trim().rsplit(['\\', '/', ':']).next()?;
        serial_from(file.split(';').next()?)
    }))
}

/// The first sector and size of the file `name` in directory `dir`. A directory record holds
/// its length at +0, first sector at +2, size at +10, name length at +32 and name at +33 (with a
/// ";1" version); records never cross a sector's end, and a zero length pads to it.
fn find_file(dir: &[u8], name: &str) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < dir.len() {
        let len = dir[i] as usize;
        if len == 0 { i = (i / SECTOR + 1) * SECTOR; continue; }
        let n = *dir.get(i + 32)? as usize;
        let file = dir.get(i + 33..i + 33 + n)?;
        if file.split(|&c| c == b';').next()?.eq_ignore_ascii_case(name.as_bytes()) { return Some((u32_at(dir, i + 2)?, u32_at(dir, i + 10)?)); }
        i += len;
    }
    None
}

/// The serial from the `DISC_ID` in a PBP's `PARAM.SFO`.
fn pbp_serial(f: &mut File) -> io::Result<Option<String>> {
    // "\0PBP", a version, then where each part starts: PARAM.SFO at +8, ICON0 (which follows
    // it) at +12.
    let head = read_at(f, 0, 16, true)?;
    let (start, end) = (u32_at(&head, 8).unwrap_or_default(), u32_at(&head, 12).unwrap_or_default());
    if !head.starts_with(b"\0PBP") || end < start || end - start > 64 << 10 { return Err(bad("not a PBP")); }
    let sfo = read_at(f, start as u64, end - start, true)?;
    Ok(sfo_value(&sfo, "DISC_ID").and_then(serial_from))
}

/// A string value from a PARAM.SFO: "\0PSF", a version, where the keys and the values start,
/// and the number of entries; then the entries, 16 bytes each: the key's offset (u16, from
/// where the keys start) at +0, the value's length at +4 and its offset (from where the values
/// start) at +12.
fn sfo_value<'a>(sfo: &'a [u8], key: &str) -> Option<&'a str> {
    if sfo.get(..4)? != b"\0PSF" { return None; }
    let (keys, values, count) = (u32_at(sfo, 8)?, u32_at(sfo, 12)?, u32_at(sfo, 16)?);
    (0..count.min(256)).find_map(|n| {
        let e = 20 + n * 16;
        let k = sfo.get(keys.checked_add(u16_at(sfo, e)?)?..)?;
        if k.split(|&c| c == 0).next()? != key.as_bytes() { return None; }
        let at = values.checked_add(u32_at(sfo, e + 12)?)?;
        let v = sfo.get(at..at.checked_add(u32_at(sfo, e + 4)?)?)?;
        std::str::from_utf8(v.split(|&c| c == 0).next()?).ok()
    })
}
