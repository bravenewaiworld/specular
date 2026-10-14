//! The Brick as a Specular device: its profile, and its hardware through standard interfaces
//! where the kernel has them (battery, LEDs) and Allwinner's display driver where it doesn't
//! (backlight and panel power: this kernel has no /sys/class/backlight).

use std::io;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
use std::os::fd::AsRawFd;
use std::path::Path;

use specular_core::device::{Audio, Device, DeviceProfile, Display, Hardware, Lights, Power, Rgb, SoundOut, Vsync};
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
            audio: Box::new(DacVolume),
        }
    }

    fn open_sound(&self) -> io::Result<Box<dyn SoundOut>> {
        Ok(Box::new(BrickSound { alsa: specular_linux::sound::Alsa::open("default")?, started: false }))
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

    /// Through the test card's `sleepnow`, which also stops WiFi around sleep (its driver can't
    /// suspend while connected).
    /// The codec's driver resets its volume on waking, so it is set again (see `DacVolume`).
    fn sleep(&self) -> io::Result<()> {
        let slept = run("/usr/bin/sleepnow", &[]);
        let level = VOLUME.load(Relaxed);
        if level != u8::MAX && let Err(e) = set_dac_volume(level) { eprintln!("volume after waking: {e}"); }
        slept
    }

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

/// The volume, as the codec's digital volume ("DAC volume", 0.75 dB per step, 159 = 0 dB), which
/// both the speaker and the headphones follow, set with amixer. Specular's levels are 2 dB apart,
/// the top one at 0 dB (louder only clips); 0 is the control's lowest, -119 dB. (TrimUI's ALSA
/// set-up also has a software volume, "Soft Volume Master", but the driver refuses writes to it.)
/// One level for the speaker and headphones alike, for now.
struct DacVolume;

/// The volume last set (u8::MAX: none yet), for `BrickSound`.
static VOLUME: AtomicU8 = AtomicU8::new(u8::MAX);

impl Audio for DacVolume {
    fn set_volume(&self, level: u8) -> io::Result<()> {
        VOLUME.store(level, Relaxed);
        set_dac_volume(level)
    }
}

/// Sound output whose start sets the volume again: the codec's driver puts its DAC volume back
/// to its default whenever playback starts (and on waking; see `BrickPower::sleep`).
struct BrickSound { alsa: specular_linux::sound::Alsa, started: bool }

impl SoundOut for BrickSound {
    fn rate(&self) -> u32 { self.alsa.rate() }
    fn write(&mut self, frames: &[[i16; 2]]) -> io::Result<usize> {
        let n = self.alsa.write(frames)?;
        if n > 0 && !self.started {
            self.started = true;
            // On a thread of its own: amixer takes longer than a game frame may wait.
            let level = VOLUME.load(Relaxed);
            if level != u8::MAX { std::thread::spawn(move || { let _ = set_dac_volume(level); }); }
        }
        Ok(n)
    }
    fn queued(&mut self) -> usize { self.alsa.queued() }
    fn capacity(&self) -> usize { self.alsa.capacity() }
}

fn set_dac_volume(level: u8) -> io::Result<()> {
    let max = specular_core::settings::builtin::VOLUME_MAX as i32;
    let value = if level == 0 { 0 } else { ((2 * (level as i32 - max)) as f32 / 0.75 + 159.0).round() as i32 };
    run("/usr/bin/amixer", &["-q", "-c", "audiocodec", "cset", "name=DAC volume", &format!("{value},{value}")])
}
