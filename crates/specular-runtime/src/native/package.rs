//! Native cores' packages: an uncompressed tar (ustar) whose `manifest.json` names the core, the
//! system and the program that plays it (Tesseract's: its converter/README.md). Specular takes the
//! program out into a cache folder to run it; the core reads the rest in place.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// What a package's manifest says Specular needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest { pub core: String, pub system: String, pub program: String, pub title: String }

/// An entry's name, where its bytes start, and its size.
fn entries(f: &mut File) -> io::Result<Vec<(String, u64, u64)>> {
    let (mut at, mut found) = (0u64, Vec::new());
    loop {
        let mut h = [0u8; 512];
        f.seek(SeekFrom::Start(at))?;
        f.read_exact(&mut h)?;
        if h[0] == 0 { return Ok(found); }
        if &h[257..262] != b"ustar" { return Err(io::Error::other(format!("not a package (no tar header at {at})"))); }
        let name = String::from_utf8_lossy(&h[..100]).trim_end_matches('\0').to_owned();
        let size = h[124..136].iter().take_while(|c| (b'0'..=b'7').contains(c)).fold(0u64, |v, c| v * 8 + (c - b'0') as u64);
        if h[156] == b'0' || h[156] == 0 { found.push((name, at + 512, size)); }
        at += 512 + size.div_ceil(512) * 512;
    }
}

/// Reads `package`'s manifest.
pub fn manifest(package: &Path) -> Result<Manifest, String> {
    let fail = |e: String| format!("{}: {e}", package.display());
    let mut f = File::open(package).map_err(|e| fail(e.to_string()))?;
    let list = entries(&mut f).map_err(|e| fail(e.to_string()))?;
    let &(_, at, size) = list.iter().find(|(n, ..)| n == "manifest.json").ok_or_else(|| fail("not a package (no manifest.json)".into()))?;
    if size > 64 << 10 { return Err(fail("its manifest.json is too large".into())); }
    let mut text = vec![0; size as usize];
    f.seek(SeekFrom::Start(at)).and_then(|_| f.read_exact(&mut text)).map_err(|e| fail(e.to_string()))?;
    let v: serde_json::Value = serde_json::from_slice(&text).map_err(|e| fail(format!("its manifest.json: {e}")))?;
    let field = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned).ok_or_else(|| fail(format!("its manifest.json has no \"{k}\"")));
    Ok(Manifest { core: field("core")?, system: field("system")?, program: field("program")?, title: field("title").unwrap_or_default() })
}

/// The package's program, taken out into `dir` (made if missing) unless it is there already from
/// this same package (by its size and modification time). Returns its path.
pub fn unpack_program(package: &Path, manifest: &Manifest, dir: &Path) -> Result<PathBuf, String> {
    let fail = |e: String| format!("{}: {e}", package.display());
    let meta = std::fs::metadata(package).map_err(|e| fail(e.to_string()))?;
    let stamp = format!("{} {:?}", meta.len(), meta.modified().ok());
    let (program, stamp_file) = (dir.join("program"), dir.join("program.from"));
    if program.exists() && std::fs::read_to_string(&stamp_file).is_ok_and(|s| s == stamp) { return Ok(program); }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut f = File::open(package).map_err(|e| fail(e.to_string()))?;
    let list = entries(&mut f).map_err(|e| fail(e.to_string()))?;
    let &(_, at, size) = list.iter().find(|(n, ..)| *n == manifest.program).ok_or_else(|| fail(format!("it has no {}", manifest.program)))?;
    let part = dir.join("program.part");
    let copied = (|| {
        f.seek(SeekFrom::Start(at))?;
        let mut out = File::create(&part)?;
        io::copy(&mut (&mut f).take(size), &mut out)?;
        out.sync_all()?;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&part, &program)?;
        std::fs::write(&stamp_file, &stamp)
    })();
    copied.map_err(|e| format!("taking {} out of {} into {}: {e}", manifest.program, package.display(), dir.display()))?;
    Ok(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_comes_out_once() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("manifest.json"), r#"{"format": 1, "core": "tesseract", "system": "gamecube", "program": "game", "title": "Test"}"#).unwrap();
        std::fs::write(src.join("game"), "#!/bin/sh\necho hi\n").unwrap();
        std::fs::write(src.join("disc.iso"), vec![7u8; 3000]).unwrap();
        let pkg = dir.path().join("Test.tesseract");
        assert!(std::process::Command::new("tar").args(["--format=ustar", "-cf"]).arg(&pkg).arg("-C").arg(&src)
            .args(["manifest.json", "game", "disc.iso"]).status().unwrap().success());
        let m = manifest(&pkg).unwrap();
        assert_eq!(m, Manifest { core: "tesseract".into(), system: "gamecube".into(), program: "game".into(), title: "Test".into() });
        let out = dir.path().join("cache");
        let p = unpack_program(&pkg, &m, &out).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "#!/bin/sh\necho hi\n");
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o755);
        // Again: the same file, not taken out again.
        std::fs::write(&p, "kept").unwrap();
        assert_eq!(std::fs::read_to_string(unpack_program(&pkg, &m, &out).unwrap()).unwrap(), "kept");
        assert!(manifest(&src.join("game")).is_err());
    }
}
