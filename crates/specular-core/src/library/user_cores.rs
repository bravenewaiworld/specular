//! Cores the user adds: libretro cores in the SD card's `cores/`, as the libretro buildbot ships
//! them: `<name>_libretro.so` beside its `<name>_libretro.info` (RetroArch's core info file). The
//! `.info` says which systems a core plays; only systems Specular declares are served, and a core
//! for none of them is turned away, as is one without a `.info` or built for another processor.
//! Each core found becomes a `CoreDecl`, used from then on as Specular's own cores are.
//!
//! A core's options can't be known before it runs (Specular's own are listed when they are built:
//! see `settings::core_options`). The first time one plays a game, what it declares is kept in
//! `cache/cores/<name>.json` (`learn`), with its library's size and modification time, so a
//! replaced core is learned again; from then on its options are settings like the built-in
//! cores'. Until then it runs with its own defaults.

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::{Bios, CoreDecl, SystemDecl};
use crate::settings::core_options::{self, CoreOptions};
use crate::settings::{leak, store};

/// What a core the user added says about itself (its `.info`), and where it is.
#[derive(Debug)]
pub struct UserCore {
    /// Its library, e.g. `{sd card}/cores/snes9x_libretro.so`.
    pub path: PathBuf,
    /// What the splash names it by: its `.info`'s `display_name` ("Nintendo - SNES / SFC
    /// (Snes9x)"), else its name.
    pub display_name: String,
    /// Its authors and licence, as the splash shows them ("Snes9x Team · Non-commercial"); empty
    /// if its `.info` gives neither.
    pub notice: String,
    /// The ids of the systems it plays.
    pub systems: Vec<&'static str>,
    /// Whether its options have been learned (`learn`); until then they have no settings, and it
    /// runs with its own defaults.
    pub learned: bool,
    /// Its library's size and modification time (ns since 1970) when it was found: options
    /// learned from a library of another size or time are learned again.
    size: u64,
    modified: u64,
}

/// What a look in `cores/` found: the cores to use, and the files turned away (each in file name
/// order).
#[derive(Debug, Default)]
pub struct Found { pub cores: Vec<&'static CoreDecl>, pub turned_away: Vec<TurnedAway> }

impl PartialEq for Found {
    /// The same cores (unchanged ones are the same `CoreDecl`: see `UserCores`), and the same files
    /// turned away for the same reasons.
    fn eq(&self, other: &Self) -> bool {
        self.turned_away == other.turned_away && self.cores.len() == other.cores.len()
            && self.cores.iter().zip(&other.cores).all(|(a, b)| std::ptr::eq(*a, *b))
    }
}

/// A file in `cores/` that isn't used (or the folder itself, by its path, if it can't be read),
/// and why (for the log and the Cores screen).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnedAway { pub file: String, pub why: String }

/// The cores found so far, with what each was made from. A `CoreDecl` is leaked, as settings take
/// `'static` text from it (see `core_options`), so a look that finds a core unchanged gives the
/// same one again: looking costs no memory until a core's files change.
#[derive(Default)]
pub struct UserCores { made: HashMap<String, (Made, &'static CoreDecl)> }

/// What a core's `CoreDecl` is made from: its `.info`, its library's size and modification time,
/// and its learned options as JSON (`learned_options`; None until learned).
#[derive(PartialEq)]
struct Made { info: String, size: u64, modified: u64, learned: Option<String> }

impl UserCores {
    /// Looks for cores in `dir` (the card's `cores/`) that play some of `systems` (Specular's,
    /// with their own cores), with the options learned in `learned` (`cache/cores/`). No folder
    /// means no cores. Files that aren't `.so` or `.info` files are ignored, as are hidden files.
    pub fn find(&mut self, dir: &Path, learned: &Path, systems: &[SystemDecl]) -> Found {
        let mut found = Found::default();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return found,
            Err(e) => {
                found.turned_away.push(TurnedAway { file: dir.display().to_string(), why: format!("the folder can't be read ({e})") });
                return found;
            }
        };
        let mut names: Vec<String> = entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().into_string().ok()).filter(|n| !n.starts_with('.')).collect();
        names.sort();
        for name in &names {
            let why = if let Some(id) = name.strip_suffix("_libretro.so").filter(|id| !id.is_empty()) {
                match self.core(dir, learned, systems, id) {
                    Ok(core) => { found.cores.push(core); continue; }
                    Err(why) => why,
                }
            } else if let Some(id) = name.strip_suffix("_libretro.info").filter(|id| !id.is_empty()) {
                if names.contains(&format!("{id}_libretro.so")) { continue; }
                format!("there is no {id}_libretro.so beside it")
            } else if name.ends_with(".so") || name.ends_with(".info") {
                "a core's files are named <name>_libretro.so and <name>_libretro.info".into()
            } else { continue };
            found.turned_away.push(TurnedAway { file: name.clone(), why });
        }
        found
    }

    /// The core `id` in `dir`, or why it is turned away.
    fn core(&mut self, dir: &Path, learned: &Path, systems: &[SystemDecl], id: &str) -> Result<&'static CoreDecl, String> {
        if let Some(own) = super::cores(systems).into_iter().find(|c| c.id() == id) {
            return Err(format!("Specular has its own {}, which it uses instead", own.name));
        }
        let path = dir.join(format!("{id}_libretro.so"));
        let info = std::fs::read_to_string(dir.join(format!("{id}_libretro.info"))).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => format!("there is no {id}_libretro.info beside it, to say which systems it plays (the libretro buildbot has one for each core)"),
            _ => format!("its .info can't be read ({e})"),
        })?;
        let plays = systems_of(&Info::parse(&info), systems)?;
        check_library(&path)?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("it can't be read ({e})"))?;
        let (size, modified) = (meta.len(), modified_ns(&meta));
        let made = Made { learned: learned_options(learned, id, size, modified), info, size, modified };
        if let Some((m, core)) = self.made.get(id) && *m == made { return Ok(core); }
        let core = decl(id, path, &made, plays);
        self.made.insert(id.into(), (made, core));
        Ok(core)
    }
}

/// `systems`, each with the cores `found` that play it added after its own; the systems left with
/// no core are dropped.
pub fn playable(mut systems: Vec<SystemDecl>, found: &[&'static CoreDecl]) -> Vec<SystemDecl> {
    for s in &mut systems {
        s.cores.extend(found.iter().filter(|c| c.user.is_some_and(|u| u.systems.contains(&s.id))));
    }
    systems.retain(|s| !s.cores.is_empty());
    systems
}

impl UserCore {
    /// Whether its options have been learned, for the log and the Cores screen.
    pub fn status(&self) -> &'static str {
        if self.learned { "its options are learned" } else { "its options are learned when it next plays a game" }
    }
}

/// A core info file's `key = "value"` lines (RetroArch's configuration format), less comments.
struct Info<'a>(HashMap<&'a str, &'a str>);

impl<'a> Info<'a> {
    fn parse(text: &'a str) -> Self {
        Self(text.lines().filter_map(|l| {
            let (key, value) = l.trim().split_once('=')?;
            let value = value.trim();
            let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
            Some((key.trim(), value)).filter(|(k, _)| !k.starts_with('#'))
        }).collect())
    }

    /// A value, if it isn't empty.
    fn get(&self, key: &str) -> Option<&'a str> { self.0.get(key).copied().filter(|v| !v.is_empty()) }

    /// A list of values ("a|b|c").
    fn list(&self, key: &str) -> Vec<&'a str> { self.get(key).map_or(vec![], |v| v.split('|').map(str::trim).filter(|v| !v.is_empty()).collect()) }
}

/// The ids of the `systems` a core's `.info` names. They are matched by their libretro database
/// names (`database`, which lists every system a core plays), or, in a `.info` without one, by its
/// `systemid` (which names one system). Of those, only the ones some of whose files it plays are
/// kept (`supported_extensions`, if it lists any). Err if none: why, for the player.
fn systems_of(info: &Info, systems: &[SystemDecl]) -> Result<Vec<&'static str>, String> {
    let databases = info.list("database");
    let named: Vec<&SystemDecl> = systems.iter().filter(|s| match databases.is_empty() {
        true => info.get("systemid") == Some(s.libretro_id),
        false => databases.contains(&s.libretro_name),
    }).collect();
    let extensions: Vec<String> = info.list("supported_extensions").iter().map(|e| e.to_ascii_lowercase()).collect();
    let plays: Vec<&'static str> = named.iter().filter(|s| extensions.is_empty() || s.extensions.iter().any(|e| extensions.iter().any(|x| x == e)))
        .map(|s| s.id).collect();
    if !plays.is_empty() { return Ok(plays); }
    if !named.is_empty() {
        let names: Vec<&str> = named.iter().map(|s| s.name).collect();
        return Err(format!("it plays none of the files of {} (its .info lists .{})", names.join(", "), extensions.join(", .")));
    }
    let names = if databases.is_empty() { info.get("systemid").unwrap_or("no system").into() } else { databases.join(", ") };
    Err(format!("its .info is for {names}, which Specular doesn't have"))
}

/// The `CoreDecl` of the core `id`, at `path`, from what `made` holds, playing `systems`. Its
/// strings are leaked (see `UserCores`).
fn decl(id: &str, path: PathBuf, made: &Made, systems: Vec<&'static str>) -> &'static CoreDecl {
    let info = Info::parse(&made.info);
    let name = info.get("corename").unwrap_or(id);
    let authors = info.list("authors").join(", ");
    let notice = [authors.as_str(), info.get("license").unwrap_or_default()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    // Only the firmware it can't do without is checked: for an optional one, the .info doesn't say
    // which games need it, so a missing one can't be reported usefully.
    let games = leak(format!("Games played with {name}"));
    let count: usize = info.get("firmware_count").and_then(|n| n.parse().ok()).unwrap_or(0);
    let bios: Vec<Bios> = (0..count).filter(|i| info.get(&format!("firmware{i}_opt")) != Some("true"))
        .filter_map(|i| info.get(&format!("firmware{i}_path")))
        // Lower case, as `CoreDecl::check_bios` compares names with those in `bios/` in lower case;
        // a path into a folder is looked for as it is.
        .map(|f| leak(if f.contains('/') { f.to_string() } else { f.to_lowercase() }))
        .map(|f| Bios { games, extensions: &[], files: Box::leak(Box::new([f])), fallback: None })
        .collect();
    let user = UserCore { path, display_name: info.get("display_name").unwrap_or(name).into(), notice, systems,
                          learned: made.learned.is_some(), size: made.size, modified: made.modified };
    Box::leak(Box::new(CoreDecl {
        name: leak(name), library: leak(format!("{id}_libretro")),
        options: leak(made.learned.clone().unwrap_or_else(|| r#"{"options":[]}"#.into())),
        defaults: &[], rom_by_path: false, controller: None,
        // Assumed: Specular can't tell whether its states hold the save memory, and assuming they do
        // keeps a newer battery save rather than letting a state roll it back.
        states_restore_save: true, cpu_start: None,
        bios: bios.leak(), user: Some(Box::leak(Box::new(user))),
    }))
}

/// Checks that `path` is a shared library for this machine: an ELF file of its word size, byte
/// order and processor. Err: why not, so a core built for another device is turned away with a
/// plain reason rather than failing to load.
pub fn check_library(path: &Path) -> Result<(), String> {
    let mut head = [0u8; 20];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => "it isn't a library (it is too short)".to_string(),
        _ => format!("it can't be read ({e})"),
    })?;
    if head[..4] != *b"\x7fELF" { return Err("it isn't a library (not an ELF file)".into()); }
    let little = head[5] == 1;
    let half = |at: usize| if little { u16::from_le_bytes([head[at], head[at + 1]]) } else { u16::from_be_bytes([head[at], head[at + 1]]) };
    let theirs = Arch { machine: half(18), bits: if head[4] == 2 { 64 } else { 32 }, little };
    if theirs != Arch::OURS { return Err(format!("it is built for {theirs}, and this device is {}", Arch::OURS)); }
    // ET_DYN: a shared library (an executable is ET_EXEC). Position-independent executables are
    // ET_DYN too: this only rules out other kinds of file.
    if half(16) != 3 { return Err("it isn't a shared library".into()); }
    Ok(())
}

/// What an ELF file is built for.
#[derive(PartialEq)]
struct Arch { machine: u16, bits: u8, little: bool }

impl Arch {
    /// This machine's: `e_machine` as Linux's elf.h numbers them.
    const OURS: Arch = Arch {
        machine: if cfg!(target_arch = "x86_64") { 62 } else if cfg!(target_arch = "aarch64") { 183 } else if cfg!(target_arch = "arm") { 40 }
                 else if cfg!(target_arch = "x86") { 3 } else if cfg!(target_arch = "riscv64") { 243 } else { 0 },
        bits: if cfg!(target_pointer_width = "64") { 64 } else { 32 },
        little: cfg!(target_endian = "little"),
    };
}

impl std::fmt::Display for Arch {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let name = match self.machine { 62 => "x86-64", 183 => "64-bit ARM (AArch64)", 40 => "32-bit ARM", 3 => "32-bit x86", 243 => "RISC-V", 8 => "MIPS", _ => "" };
        if name.is_empty() { write!(f, "processor {}", self.machine)? } else { f.write_str(name)? }
        // Only where the processor's name doesn't say.
        if !matches!(self.machine, 62 | 183 | 40 | 3) { write!(f, " ({}-bit, {})", self.bits, if self.little { "little-endian" } else { "big-endian" })?; }
        Ok(())
    }
}

/// A core's options as learned, in `cache/cores/<name>.json`.
#[derive(Serialize, Deserialize)]
struct Learned { library_size: u64, library_modified: u64, options: CoreOptions }

fn learned_path(dir: &Path, id: &str) -> PathBuf { dir.join(format!("{id}.json")) }

/// The options learned for the core `id` in `dir`, as JSON (`CoreDecl::options`), if they were
/// learned from its library as it is now (of this `size` and `modified` time).
fn learned_options(dir: &Path, id: &str, size: u64, modified: u64) -> Option<String> {
    let map = store::load_layer(&learned_path(dir, id)).ok()?;
    let learned: Learned = serde_json::from_value(map.into()).ok()?;
    let current = learned.library_size == size && learned.library_modified == modified;
    current.then(|| serde_json::to_string(&learned.options).expect("options always serialize"))
}

/// Writes the options a core the user added `declared` as it played a game (`GameInfo::options` in
/// specular-runtime) to `dir` (`cache/cores/`), unless it was found with exactly these learned.
/// Does nothing for Specular's own cores. Returns whether it wrote them; then the next look in
/// `cores/` (the scan thread watches `dir`) gives the core with them.
pub fn learn(core: &'static CoreDecl, dir: &Path, declared: &CoreOptions) -> io::Result<bool> {
    let Some(u) = core.user else { return Ok(false) };
    if u.learned && core_options::list(core) == declared { return Ok(false); }
    let learned = Learned { library_size: u.size, library_modified: u.modified, options: declared.clone() };
    store::store_record(&learned_path(dir, core.id()), &learned).map(|()| true)
}

fn modified_ns(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64)
}

#[cfg(test)]
pub(crate) mod tests;
