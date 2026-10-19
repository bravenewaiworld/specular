//! The kinds of picture a game can be shown with (the "Art" setting, per system), and where each
//! is kept: `cache/art/<kind id>/` mirrors `roms/`.

use std::io;
use std::path::{Path, PathBuf};

use super::user_image;
use crate::library::SystemDecl;
use crate::settings::{Layer, Settings, builtin};

/// What a game is shown with. A game without a picture of its kind shows its box art, else a
/// generated cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArtKind {
    /// The box: the user's own image beside the ROM, else libretro-thumbnails' `Named_Boxarts`.
    BoxArt,
    /// The title screen, from libretro-thumbnails' `Named_Titles`.
    TitleScreen,
    /// A moment of play, from libretro-thumbnails' `Named_Snaps`.
    Screenshot,
    /// The last frame shown when the game was last quit, kept by Specular.
    LeftOff,
}

impl ArtKind {
    /// Every kind, in declaration order (arrays of one item per kind are indexed by `kind as usize`).
    pub const ALL: [ArtKind; 4] = [ArtKind::BoxArt, ArtKind::TitleScreen, ArtKind::Screenshot, ArtKind::LeftOff];

    /// Its value in the "Art" setting, its folder in `cache/art/` and its key in `ArtIndex`.
    pub fn id(self) -> &'static str {
        match self { ArtKind::BoxArt => "boxart", ArtKind::TitleScreen => "title", ArtKind::Screenshot => "screenshot", ArtKind::LeftOff => "left-off" }
    }

    /// Its folder on the libretro thumbnail server; None for the one Specular makes itself.
    pub fn thumbnails(self) -> Option<&'static str> {
        match self { ArtKind::BoxArt => Some("Named_Boxarts"), ArtKind::TitleScreen => Some("Named_Titles"), ArtKind::Screenshot => Some("Named_Snaps"), ArtKind::LeftOff => None }
    }

    /// The kinds to try for a game shown as this kind, best first: this kind, then box art.
    pub fn fallbacks(self) -> &'static [ArtKind] {
        match self {
            ArtKind::BoxArt => &[ArtKind::BoxArt],
            ArtKind::TitleScreen => &[ArtKind::TitleScreen, ArtKind::BoxArt],
            ArtKind::Screenshot => &[ArtKind::Screenshot, ArtKind::BoxArt],
            ArtKind::LeftOff => &[ArtKind::LeftOff, ArtKind::BoxArt],
        }
    }

    /// The shape (width, height) of the place its pictures are shown in, for `system`'s games:
    /// square for box art (boxes vary), else the system's screen.
    pub fn shape(self, system: &SystemDecl) -> (u32, u32) { if self == ArtKind::BoxArt { (1, 1) } else { system.screen } }
}

/// The kind of picture `system`'s games are shown with.
pub fn kind(settings: &Settings, system: &str) -> ArtKind {
    let id = settings.get(builtin::ART, &Layer::System(system.to_string())).and_then(|(v, _)| v.as_str());
    ArtKind::ALL.into_iter().find(|k| Some(k.id()) == id).unwrap_or(ArtKind::BoxArt)
}

/// Where the picture of `kind` for the game at `rom` (under `roms/`) is cached, shrunk to
/// `IMAGE_SIZE`.
pub fn cached_image(cache: &Path, kind: ArtKind, rom: &Path) -> PathBuf { cache.join("art").join(kind.id()).join(rom).with_extension("png") }

/// The files the game's picture of `kind` may be in, best first: for box art, the user's own
/// image (only if there is one), then the cached one (whether or not it exists).
pub fn files(roms: &Path, cache: &Path, rom: &Path, kind: ArtKind) -> Vec<PathBuf> {
    let user = if kind == ArtKind::BoxArt { user_image(roms, rom) } else { None };
    user.into_iter().chain([cached_image(cache, kind, rom)]).collect()
}

/// The files a game shown as `kind` takes its picture from, best first: `files` of each of
/// `ArtKind::fallbacks`.
pub fn pictures(roms: &Path, cache: &Path, rom: &Path, kind: ArtKind) -> Vec<PathBuf> {
    kind.fallbacks().iter().flat_map(|&k| files(roms, cache, rom, k)).collect()
}

/// Moves box art cached when `cache/art/` held only box art, mirroring `roms/`, into
/// `cache/art/boxart/`: each entry of `cache/art/` that isn't named as a kind's folder. Each move
/// is one rename, so a power cut leaves the rest for the next start. If an entry (a whole system's
/// folder, say) is already in `boxart/`, as after an older Specular ran again and refilled
/// `cache/art/`, the old one is deleted. A folder in `roms/` named as a kind's (`roms/title/`)
/// can't be told from that kind's folder, so its box art stays, taken for that kind's pictures.
/// Logs how many entries it moved, or the error.
pub fn move_old_cache(cache: &Path) {
    let art = cache.join("art");
    let to = art.join(ArtKind::BoxArt.id());
    let moved = || -> io::Result<usize> {
        let Ok(entries) = std::fs::read_dir(&art) else { return Ok(0) }; // no cache yet
        let old: Vec<_> = entries.filter_map(Result::ok).map(|e| e.file_name())
            .filter(|name| !ArtKind::ALL.iter().any(|k| name.to_str() == Some(k.id()))).collect();
        if !old.is_empty() { std::fs::create_dir_all(&to)?; }
        for name in &old {
            let (from, dest) = (art.join(name), to.join(name));
            match std::fs::symlink_metadata(&dest) {
                Err(_) => std::fs::rename(&from, &dest)?,
                Ok(_) if from.is_dir() => std::fs::remove_dir_all(&from)?,
                Ok(_) => std::fs::remove_file(&from)?,
            }
        }
        Ok(old.len())
    };
    match moved() {
        Ok(0) => {}
        Ok(n) => eprintln!("art: moved {n} entries of {} into {}", art.display(), to.display()),
        Err(e) => eprintln!("art: moving the box art in {} into {}: {e}", art.display(), to.display()),
    }
}
