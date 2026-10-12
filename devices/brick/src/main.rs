//! Specular on the TrimUI Brick, with the SD card's data partition as its data folder.
//!
//! Usage: specular [data folder (default /mnt/SDCARD)]

use specular_brick::device::Brick;
use specular_brick::platform::BrickPlatform;
use specular_ui::DataRoot;

fn main() -> Result<(), specular_ui::Error> {
    let data = std::env::args().nth(1).unwrap_or_else(|| "/mnt/SDCARD".into());
    specular_ui::run(&Brick, DataRoot(data.into()), Some(Box::new(|on_button| {
        let platform = BrickPlatform::new()?;
        platform.on_button(on_button);
        slint::platform::set_platform(Box::new(platform)).map_err(|e| format!("{e:?}"))?;
        Ok(())
    })))
}
