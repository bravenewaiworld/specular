//! Specular on a mainline board, with the SD card's data partition as its data folder.
//!
//! Usage: specular [data folder (default /mnt/SDCARD)]

use std::path::Path;

use specular_core::device::Device;
use specular_linux::kms::{Kms, ModeRequest};
use specular_mainline::boards;
use specular_mainline::device::Mainline;
use specular_crystaline::system::{GAMES_CARD_ROMS, GAMES_CARD_STATE};
use specular_ui::{DataRoot, GamesCard};
use specular_ui::platform::{GlPlatform, KmsSurface};

/// Everything this binary ships, with its licence (written by build.rs).
const CREDITS: &[specular_core::app::Credit] = include!(concat!(env!("OUT_DIR"), "/credits.rs"));

/// The display card (card0 on the supported Rockchip boards).
const CARD: &str = "/dev/dri/card0";

fn main() -> Result<(), specular_ui::Error> {
    let data = std::env::args().nth(1).unwrap_or_else(|| "/mnt/SDCARD".into());
    let board = boards::find(&std::fs::read("/proc/device-tree/compatible")?)?;
    eprintln!("board: {}", board.name);
    let card = Path::new(CARD);
    let kms = Kms::open(card)?;
    let request = ModeRequest::default();
    let device = Mainline::new(board, &kms, card, request.clone());
    let vsync = device.open_vsync();
    let size = slint::PhysicalSize::new(kms.mode().width(), kms.mode().height());
    let games_card = board.games_card.then(|| GamesCard { roms: GAMES_CARD_ROMS.into(), changes: GAMES_CARD_STATE.into() });
    specular_ui::run(Box::new(device), DataRoot(data.into(), games_card), Some(Box::new(move |on_button| {
        let platform = GlPlatform::new(KmsSurface::new(kms, request)?, size, board.input(), vsync)?;
        platform.on_button(on_button);
        slint::platform::set_platform(Box::new(platform)).map_err(|e| format!("{e:?}"))?;
        Ok(())
    })), CREDITS)
}
