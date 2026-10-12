//! Lists the files in a zip without unpacking it, from the directory at the end of the file
//! (only names are needed, so this is a few kilobytes of reading).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

fn u16_at(b: &[u8], i: usize) -> usize { u16::from_le_bytes([b[i], b[i + 1]]) as usize }
fn u32_at(b: &[u8], i: usize) -> u64 { u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as u64 }

fn bad() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "not a zip") }

/// The names of the files in the zip at `path`.
pub fn file_names(path: &Path) -> io::Result<Vec<String>> {
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
    let mut names = Vec::with_capacity(count);
    let mut i = 0;
    while names.len() < count && i + 46 <= dir.len() && dir[i..i + 4] == *b"PK\x01\x02" {
        let (name_len, extra_len, comment_len) = (u16_at(&dir, i + 28), u16_at(&dir, i + 30), u16_at(&dir, i + 32));
        let name = dir.get(i + 46..i + 46 + name_len).ok_or_else(bad)?;
        names.push(String::from_utf8_lossy(name).into_owned());
        i += 46 + name_len + extra_len + comment_len;
    }
    Ok(names)
}
