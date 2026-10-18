//! Reads zips: lists their files from the directory at the end of the file (names and checksums
//! are all that scanning needs, so this is a few kilobytes of reading), and unpacks one file.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

fn u16_at(b: &[u8], i: usize) -> usize { u16::from_le_bytes([b[i], b[i + 1]]) as usize }
fn u32_at(b: &[u8], i: usize) -> u64 { u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as u64 }

fn bad() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "not a zip") }

/// A file in a zip, as the directory records it.
pub struct Entry {
    pub name: String,
    /// The CRC32 of its uncompressed contents.
    pub crc32: u32,
    /// Its size uncompressed.
    pub size: u64,
    compressed: u64,
    method: usize,
    /// Where its local header starts.
    offset: u64,
}

/// The files in the zip at `path`. A zip64 one (a file of 4 GiB or more) is refused.
pub fn entries(path: &Path) -> io::Result<Vec<Entry>> {
    let mut f = File::open(path)?;
    let len = f.seek(SeekFrom::End(0))?;
    // The end-of-directory record: 22 bytes, plus a comment of up to 64 KiB.
    let tail_len = len.min(22 + 65535);
    f.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0; tail_len as usize];
    f.read_exact(&mut tail)?;
    let end = (0..tail.len().saturating_sub(21)).rev().find(|&i| tail[i..i + 4] == *b"PK\x05\x06").ok_or_else(bad)?;
    let (count, dir_size, dir_start) = (u16_at(&tail, end + 10), u32_at(&tail, end + 12), u32_at(&tail, end + 16));
    if dir_start + dir_size > len || dir_size > 16 << 20 { return Err(bad()); }
    f.seek(SeekFrom::Start(dir_start))?;
    let mut dir = vec![0; dir_size as usize];
    f.read_exact(&mut dir)?;
    // Central directory entries: 46 fixed bytes, then the name, extra field and comment.
    let mut entries = Vec::with_capacity(count);
    let mut i = 0;
    while entries.len() < count && i + 46 <= dir.len() && dir[i..i + 4] == *b"PK\x01\x02" {
        let (name_len, extra_len, comment_len) = (u16_at(&dir, i + 28), u16_at(&dir, i + 30), u16_at(&dir, i + 32));
        let name = dir.get(i + 46..i + 46 + name_len).ok_or_else(bad)?;
        let (compressed, size, offset) = (u32_at(&dir, i + 20), u32_at(&dir, i + 24), u32_at(&dir, i + 42));
        // Zip64 marks its sizes and offsets this way, and keeps the real ones elsewhere.
        if [compressed, size, offset].contains(&0xffff_ffff) { return Err(io::Error::new(io::ErrorKind::Unsupported, "a zip64 zip")); }
        entries.push(Entry { name: String::from_utf8_lossy(name).into_owned(), crc32: u32_at(&dir, i + 16) as u32,
                             size, compressed, method: u16_at(&dir, i + 10), offset });
        i += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

/// Unpacks `e`, a file in the zip at `path`, into `out`. Fails (InvalidData) if what comes out
/// isn't the size and checksum the directory records, and (Unsupported) for compression other
/// than none or deflate, the only two that ROM zips use.
pub fn unpack(path: &Path, e: &Entry, out: &mut impl Write) -> io::Result<()> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(e.offset))?;
    // The local header: 30 fixed bytes, then the name and an extra field (whose length can
    // differ from the directory's).
    let mut local = [0u8; 30];
    f.read_exact(&mut local)?;
    if local[..4] != *b"PK\x03\x04" { return Err(bad()); }
    f.seek(SeekFrom::Current((u16_at(&local, 26) + u16_at(&local, 28)) as i64))?;
    let data = BufReader::new(f).take(e.compressed);
    let mut out = Checked { out, crc: crc32fast::Hasher::new(), len: 0 };
    match e.method {
        0 => io::copy(&mut { data }, &mut out)?,
        // At most one byte more than recorded: enough to tell it is too long.
        8 => io::copy(&mut flate2::read::DeflateDecoder::new(data).take(e.size + 1), &mut out)?,
        m => return Err(io::Error::new(io::ErrorKind::Unsupported, format!("compression method {m}"))),
    };
    if out.len != e.size || out.crc.finalize() != e.crc32 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} came out as {} bytes, not {}, or with the wrong checksum", e.name, out.len, e.size)));
    }
    Ok(())
}

/// Passes writes on, counting and checksumming them.
struct Checked<'a, W> { out: &'a mut W, crc: crc32fast::Hasher, len: u64 }

impl<W: Write> Write for Checked<'_, W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.out.write(b)?;
        self.crc.update(&b[..n]);
        self.len += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> { self.out.flush() }
}
