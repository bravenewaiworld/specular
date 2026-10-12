//! The Brick as a Specular device: its profile, and its hardware through standard interfaces
//! where the kernel has them (battery, LEDs) and Allwinner's display driver where it doesn't
//! (backlight and panel power: this kernel has no /sys/class/backlight).

use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

use specular_core::device::{Audio, Device, DeviceProfile, Display, Hardware, Lights, Power, Rgb};
use specular_core::input::Button;
use specular_linux::leds::RgbLeds;
use specular_linux::power_supply::PowerSupply;

/// The light bar: the 8 LEDs along the top (LED-class names "sunxi_led0r" … "sunxi_led7b").
const LIGHT_BAR: usize = 8;
/// The LEDs are very bright at full power; like TrimUI's firmware, limit them to about 40/255.
const LED_LIMIT: u8 = 40;

pub struct Brick;

impl Device for Brick {
    fn profile(&self) -> DeviceProfile {
        use Button::*;
        DeviceProfile {
            name: "TrimUI Brick",
            width: 1024,
            height: 768,
            ppi: 400,
            refresh_hz: 60.2,
            buttons: &[Up, Down, Left, Right, A, B, X, Y, L1, R1, L2, R2, Select, Start, Menu, F1, F2, VolUp, VolDown, Power],
            light_bar_leds: LIGHT_BAR,
        }
    }

    fn hardware(&self) -> Hardware {
        let names: Vec<[String; 3]> = (0..LIGHT_BAR).map(|i| ["r", "g", "b"].map(|c| format!("sunxi_led{i}{c}"))).collect();
        Hardware {
            power: Box::new(BrickPower(PowerSupply::find(Path::new("/sys/class/power_supply")))),
            display: Box::new(Disp),
            lights: Box::new(BrickLights(RgbLeds::new(Path::new("/sys/class/leds"), &names))),
            audio: Box::new(NoAudio),
        }
    }
}

struct BrickPower(PowerSupply);

impl Power for BrickPower {
    fn battery(&self) -> io::Result<specular_core::device::Battery> { self.0.battery() }

    /// Through the test card's `sleepnow`, which also stops WiFi around sleep (its driver can't
    /// suspend while connected).
    fn sleep(&self) -> io::Result<()> { run("/usr/bin/sleepnow", &[]) }

    fn shutdown(&self) -> io::Result<()> { run("/sbin/poweroff", &[]) }
}

fn run(program: &str, args: &[&str]) -> io::Result<()> {
    let status = std::process::Command::new(program).args(args).status()?;
    if status.success() { Ok(()) } else { Err(io::Error::other(format!("{program}: {status}"))) }
}

/// The backlight and panel through /dev/disp (ioctls from include/video/sunxi_display2.h).
struct Disp;

const DISP_DEVICE_SWITCH: libc::c_ulong = 0x0f; // args: screen, output type (0 none, 1 LCD), mode
const DISP_LCD_SET_BRIGHTNESS: libc::c_ulong = 0x102; // args: screen, level 0–255

impl Disp {
    fn ioctl(&self, cmd: libc::c_ulong, arg1: u64) -> io::Result<()> {
        let disp = std::fs::OpenOptions::new().read(true).write(true).open("/dev/disp")?;
        let args: [u64; 4] = [0, arg1, 0, 0]; // the driver always reads 4 arguments
        if unsafe { libc::ioctl(disp.as_raw_fd(), cmd, args.as_ptr()) } < 0 { return Err(io::Error::last_os_error()); }
        Ok(())
    }
}

impl Display for Disp {
    /// Levels 1–10 map to 16–255 on a curve, since perceived brightness is roughly logarithmic;
    /// 1 stays readable indoors.
    fn set_brightness(&self, level: u8) -> io::Result<()> {
        let l = level.clamp(1, 10) as f64;
        self.ioctl(DISP_LCD_SET_BRIGHTNESS, (16.0 * (255.0f64 / 16.0).powf((l - 1.0) / 9.0)).round() as u64)
    }

    fn set_panel(&self, on: bool) -> io::Result<()> { self.ioctl(DISP_DEVICE_SWITCH, on as u64) }
}

struct BrickLights(RgbLeds);

impl Lights for BrickLights {
    fn show(&self, colours: &[Rgb]) -> io::Result<()> { self.0.show(colours, LED_LIMIT) }
}

/// Volume isn't mapped onto the codec's controls yet (see ROADMAP.md, "Audio").
struct NoAudio;

impl Audio for NoAudio {
    fn set_volume(&self, _: u8) -> io::Result<()> { Ok(()) }
}
