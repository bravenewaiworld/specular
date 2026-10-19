//! Changing discs, through libretro's disk control interface, which a core for games on several
//! discs gives at `retro_init` (SwanStation does, for an `.m3u` playlist or a `.pbp` holding
//! several discs). Its functions are called outside `frontend::with` (`disks()` in lib.rs hands
//! out a copy of the interface), since a core may make environment calls, such as logging, from
//! them. libretro says "disk", so its interface keeps that spelling; Specular's own names say "disc".

use std::ffi::{CStr, CString, c_char, c_uint};
use std::path::Path;

use specular_core::app::Discs;

use crate::core::DiskControl;

/// The disc a save state was made on, recorded with it (see `StateTags`).
#[derive(Clone, Debug, PartialEq)]
pub struct Disc {
    pub index: usize,
    /// Its image's path, as the core gave it (empty if it gave none): `set_initial_image` wants it.
    pub path: String,
}

impl DiskControl {
    /// What the core says now; `current` is None if the tray is open or the core's disc number
    /// is out of range.
    pub fn discs(&self) -> Discs {
        let count = self.count();
        let index = unsafe { self.basic.get_image_index.map_or(0, |f| f()) } as usize;
        let ejected = unsafe { self.basic.get_eject_state.is_some_and(|f| f()) };
        Discs { count, current: (!ejected && index < count).then_some(index) }
    }

    fn count(&self) -> usize { unsafe { self.basic.get_num_images.map_or(0, |f| f()) as usize } }

    /// Disc `i`'s label, else its image's file name, else its number.
    pub fn label(&self, i: usize) -> String {
        let file = || text(self.get_image_path, i).map(|p| Path::new(&p).file_name().map_or(p.clone(), |n| n.to_string_lossy().into_owned()));
        text(self.get_image_label, i).or_else(file).unwrap_or_else(|| format!("disc {}", i + 1))
    }

    /// The disc in, for a game on several (else None): to record with a save state.
    pub fn current(&self) -> Option<Disc> {
        let Discs { count, current } = self.discs();
        let index = current.filter(|_| count > 1)?;
        Some(Disc { index, path: text(self.get_image_path, index).unwrap_or_default() })
    }

    /// Asks the core to start the game on `disc`, before `retro_load_game`. False if it can't
    /// (only the extended interface can). True is no promise: a core starts on the first disc if
    /// `disc` isn't one of the game's (libretro has it check the path).
    pub fn start_on(&self, disc: &Disc) -> bool {
        let (Some(f), Ok(path)) = (self.set_initial_image, CString::new(disc.path.as_str())) else { return false };
        unsafe { f(disc.index as c_uint, path.as_ptr()) }
    }

    /// Puts disc `to` (from 0) in: opens the tray, selects it and closes the tray, as libretro
    /// requires (a disc can only be selected with the tray open). On failure the disc that was in
    /// goes back if it can; Err says why, for the player.
    pub fn change(&self, to: usize) -> Result<(), String> {
        let (Some(eject), Some(select)) = (self.basic.set_eject_state, self.basic.set_image_index) else { return Err("the core can't change discs".into()) };
        if to >= self.count() { return Err(format!("the game has no disc {}", to + 1)); }
        let was = self.discs().current;
        unsafe {
            if !eject(true) { return Err("the core couldn't open the disc tray".into()); }
            if select(to as c_uint) && eject(false) { return Ok(()); }
            if let Some(w) = was { select(w as c_uint); eject(false); }
        }
        Err(format!("the core couldn't put disc {} in", to + 1))
    }
}

/// What `get_image_label` or `get_image_path` gives for disc `i`, if anything.
fn text(f: Option<unsafe extern "C" fn(c_uint, *mut c_char, usize) -> bool>, i: usize) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    if !unsafe { f?(i as c_uint, buf.as_mut_ptr().cast(), buf.len()) } { return None; }
    let s = CStr::from_bytes_until_nul(&buf).ok()?.to_string_lossy().into_owned();
    (!s.is_empty()).then_some(s)
}
