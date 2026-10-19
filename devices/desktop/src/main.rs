//! Specular on a desktop, for development: a window the size of the Brick's screen, simulated
//! hardware, sound through ALSA, and the keyboard as the gamepad (see `specular_ui::run` for the
//! keys). Games are paced by a timer (a desktop window can't wait for the display's refresh).
//!
//! Usage: specular [data folder, with roms/ and config/ (default ./sdcard)]
//! Cores are looked for in `cores/` next to the binary, or in $SPECULAR_CORES. The SD card's
//! state is read from the file $SPECULAR_CARD_STATE names, if set (format:
//! `specular_core::card::Card::parse`), else from the scenario's, else it is always mounted.

use std::io;
use std::path::{Path, PathBuf};

use specular_core::card::CardStatus;
use specular_core::device::{Device, DeviceProfile, Hardware, Output, SoundOut, Vsync, simulated};
use specular_core::input::Button;
use specular_core::wireless::trace::Traced;
use specular_linux::card_state::{self, StateFile};
use specular_ui::DataRoot;

/// The desktop, with its data folder (the stand-in for the SD card).
struct Desktop(PathBuf);

impl Device for Desktop {
    fn profile(&self) -> DeviceProfile {
        use Button::*;
        DeviceProfile {
            name: "Desktop",
            width: 1024, height: 768, ppi: 400, refresh_hz: 60.0,
            buttons: &[Up, Down, Left, Right, A, B, X, Y, L1, R1, L2, R2, Select, Start, Menu, VolUp, VolDown, Power],
            leds: &[],
        }
    }
    fn hardware(&self) -> Hardware {
        let mut hardware = simulated::hardware();
        if let Some(state) = std::env::var_os("SPECULAR_CARD_STATE") {
            let (state, data) = (PathBuf::from(state), self.0.clone());
            let recheck = { let state = state.clone(); move || simulated_mount_card(&state, &data) };
            hardware.card = Some(Box::new(Traced::new("card", StateFile::new(state, recheck))));
        }
        hardware
    }
    /// Bluetooth outputs through bluez-alsa, if the computer has it (the simulated Bluetooth's
    /// addresses only open if they happen to be real).
    fn open_sound(&self, output: &Output) -> io::Result<Box<dyn SoundOut>> {
        use specular_linux::sound::Alsa;
        Ok(Box::new(match output { Output::Speaker | Output::Headphones => Alsa::open("default")?, Output::Bluetooth(address) => Alsa::bluetooth(address)? }))
    }
    fn open_vsync(&self) -> Option<Box<dyn Vsync>> { None }
}

/// What the system's `mount-card` does when asked to look at the card again, for a card whose
/// state is in the file `state`: a mounted card whose data folder (`data`) has turned read-only
/// (remounted so; see DEVELOPMENT.md) is reported `read-only damaged`, as the kernel's doing.
fn simulated_mount_card(state: &Path, data: &Path) -> io::Result<()> {
    let read_only = {
        let path = std::ffi::CString::new(data.as_os_str().as_encoded_bytes()).map_err(io::Error::other)?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut st) } < 0 { return Err(io::Error::last_os_error()); }
        st.f_flag & libc::ST_RDONLY != 0
    };
    eprintln!("simulated: mount-card: {} is {}", data.display(), if read_only { "read-only" } else { "writable" });
    if !read_only || card_state::read(state).status != CardStatus::Mounted { return Ok(()); }
    let details = std::fs::read_to_string(state).ok().and_then(|t| t.lines().nth(2).map(str::to_string)).unwrap_or_default();
    // Replaced whole, as the system does.
    let new = state.with_extension("new");
    std::fs::write(&new, format!("read-only damaged\nThe SD card turned read-only after an error (simulated): games play, but nothing is saved.\n{details}\n"))?;
    std::fs::rename(&new, state)
}

/// Everything this binary ships, with its licence (written by build.rs).
const CREDITS: &[specular_core::app::Credit] = include!(concat!(env!("OUT_DIR"), "/credits.rs"));

fn main() -> Result<(), specular_ui::Error> {
    // One window pixel per panel pixel, whatever the desktop's scaling.
    // SAFETY: before any other thread exists.
    unsafe { std::env::set_var("SLINT_SCALE_FACTOR", "1") };
    // The simulated time zone starts in UTC, as a new device does, whatever the PC's, and moves
    // by changing $TZ, which must exist before other threads start (see `timezone::Simulated`).
    // SAFETY: before any other thread exists.
    unsafe { std::env::set_var("TZ", "UTC") };
    let data = std::env::args().nth(1).unwrap_or_else(|| "sdcard".into());
    specular_ui::run(Box::new(Desktop(data.clone().into())), DataRoot(data.into()), None, CREDITS)
}
