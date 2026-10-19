//! The Brick as a Specular device: its profile, and its hardware through standard interfaces
//! where the kernel has them (battery, LEDs) and Allwinner's display driver where it doesn't
//! (backlight and panel power: this kernel has no /sys/class/backlight).

use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

use specular_core::device::{Accent, Audio, Device, DeviceProfile, Display, Hardware, Lights, Output, Power, Rgb, SoundOut, Vsync};
use specular_core::input::Button;
use specular_core::wireless::trace::Traced;
use specular_linux::bluez::Bluez;
use specular_linux::card_state::StateFile;
use specular_linux::leds::RgbLeds;
use specular_linux::power_supply::PowerSupply;
use specular_linux::sound::Alsa;
use specular_linux::wpa::WpaSupplicant;

/// Every LED, in chain order (LED-class names "sunxi_led0r" … "sunxi_led13b"): the 8 along the top
/// show the first accent; F2 and F1 on the front, then the 4 under the shoulder buttons, the second.
const LEDS: &[Accent] = {
    use Accent::*;
    &[First, First, First, First, First, First, First, First, Second, Second, Second, Second, Second, Second]
};
/// The LEDs are very bright at full power; like TrimUI's firmware, limit them to about 40/255.
const LED_LIMIT: u8 = 40;

/// The SD card's state, kept by the system's `mount-card`, which a recheck runs.
const CARD_STATE: &str = "/run/card/state";
const MOUNT_CARD: &str = "/usr/libexec/brick/mount-card";

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
            buttons: &[Up, Down, Left, Right, A, B, X, Y, L1, R1, L2, R2, Select, Start, Menu, F1, F2, VolUp, VolDown, Power, Mute],
            leds: LEDS,
        }
    }

    fn hardware(&self) -> Hardware {
        let names: Vec<[String; 3]> = (0..LEDS.len()).map(|i| ["r", "g", "b"].map(|c| format!("sunxi_led{i}{c}"))).collect();
        Hardware {
            power: Box::new(BrickPower(PowerSupply::find(Path::new("/sys/class/power_supply")))),
            display: Box::new(Disp),
            lights: Box::new(BrickLights(RgbLeds::new(Path::new("/sys/class/leds"), &names))),
            audio: Box::new(Volume),
            wifi: Some(Box::new(Traced::new("wifi", WpaSupplicant::new(crate::wifi::SOCKET, Box::new(crate::wifi::BrickWifi))))),
            bluetooth: Some(Box::new(Traced::new("bluetooth", Bluez::new()))),
            installer: Some(Box::new(crate::install::BrickInstaller)),
            time_zone: Some(Box::new(crate::timezone::BrickTimeZone)),
            // Crystaline OS mounts the card as it comes and goes (docs/storage.md there).
            card: Some(Box::new(Traced::new("card", StateFile::new(CARD_STATE, || run(MOUNT_CARD, &[]))))),
            // The codec driver's jack input device, "audiocodec sunxi Audio Jack".
            headphones: Some(Box::new(specular_linux::jack::EvdevJack)),
            // padbridge's pad, "TRIMUI Player1", which passes rumble on to the motor.
            rumble: Some(Box::new(specular_linux::rumble::EvdevRumble::new())),
            file_transfer: Some(Box::new(crate::transfer::BrickTransfer)),
            ssh: Some(Box::new(crate::ssh::BrickSsh)),
        }
    }

    fn open_sound(&self, output: &Output) -> io::Result<Box<dyn SoundOut>> {
        Ok(Box::new(match output {
            Output::Speaker | Output::Headphones => Alsa::open("default")?,
            Output::Bluetooth(address) => Alsa::bluetooth(address)?,
        }))
    }

    /// The wait for the display's refresh on /dev/fb0 (our kernel's; TrimUI's returns at once).
    fn open_vsync(&self) -> Option<Box<dyn Vsync>> {
        match specular_linux::vsync::FbVsync::open(Path::new("/dev/fb0")) {
            Ok(v) => Some(Box::new(v)),
            Err(e) => { eprintln!("vsync: {e}; games are paced by a timer"); None }
        }
    }
}

struct BrickPower(PowerSupply);

impl Power for BrickPower {
    fn battery(&self) -> io::Result<specular_core::device::Battery> { self.0.battery() }

    /// Through the system's `sleepnow`, which also stops WiFi around sleep (its driver can't
    /// suspend while connected).
    fn sleep(&self) -> io::Result<()> { run("/usr/bin/sleepnow", &[]) }

    fn shutdown(&self) -> io::Result<()> { run("/sbin/poweroff", &[]) }

    fn restart(&self) -> io::Result<()> { run("/sbin/reboot", &[]) }
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

/// The volume: the codec's "Speaker Playback Volume" and "Headphone Playback Volume", set with
/// amixer (each 0–255, 0.75 dB per step, 159 = 0 dB). Crystaline OS's codec driver keeps both and
/// plays the level of the output in use, switching as headphones are plugged in or out, so the
/// other output's level can be set without being heard. Specular's levels are 2 dB apart, the
/// top one at 0 dB (louder only clips); 0 is the control's lowest, -119 dB. Bluetooth headphones
/// and speakers have their own (through bluez-alsa).
struct Volume;

impl Audio for Volume {
    fn set_volume(&self, output: &Output, level: u8) -> io::Result<()> {
        let control = match output {
            Output::Speaker => "name=Speaker Playback Volume",
            Output::Headphones => "name=Headphone Playback Volume",
            Output::Bluetooth(address) => return specular_linux::sound::set_bluetooth_volume(address, level),
        };
        run("/usr/bin/amixer", &["-q", "-c", "audiocodec", "cset", control, &codec_step(level).to_string()])
    }
}

/// The codec control's step for a volume level. 2 dB per level below the top is 8/3 of the
/// control's steps; never above 159, since the control goes up to +72 dB.
fn codec_step(level: u8) -> u8 {
    let below_top = (specular_core::settings::builtin::VOLUME_MAX - level as i64).max(0) as f64;
    if level == 0 { 0 } else { (159.0 - below_top * 2.0 / 0.75).round() as u8 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn levels_are_2_db_apart_with_the_top_at_0_db() {
        let steps: Vec<u8> = [0, 1, 10, 18, 19, 20, 21].into_iter().map(super::codec_step).collect();
        assert_eq!(steps, [0, 108, 132, 154, 156, 159, 159]);
    }
}
