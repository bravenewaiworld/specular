//! What Specular needs from a device. A port describes its screen and buttons in a
//! `DeviceProfile` and implements these interfaces; most Linux handhelds can use the standard
//! implementations in specular-linux and override only their quirks.
//!
//! Calls may touch slow hardware (a battery reading can be a slow bus transfer), so the UI crate
//! makes them on its hardware thread, never on the UI thread.

use std::io;
use std::time::Duration;

use crate::input::Button;
use crate::wireless::{self, bluetooth::BtAddress};

/// The screen, buttons and lights of a device.
#[derive(Clone, Debug)]
pub struct DeviceProfile {
    pub name: &'static str,
    /// The screen in pixels.
    pub width: u32,
    pub height: u32,
    /// Pixels per inch. Sizes in the UI scale by `ppi / 400`, so text keeps its physical size.
    pub ppi: u32,
    /// The display's refresh rate (game frames will be paced against it).
    pub refresh_hz: f64,
    /// The buttons the device has; hints never show others.
    pub buttons: &'static [Button],
    /// The device's RGB LEDs in chain order, with the accent each one shows (empty for none).
    pub leds: &'static [Accent],
}

/// Which of the two accent colours an LED shows (`settings::builtin::ACCENT` or `SECOND_ACCENT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent { First, Second }

/// The battery and charger, as the status bar shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    /// Charge in percent, if the device knows it.
    pub percent: Option<u8>,
    pub charging: bool,
    /// On external power (charging or full).
    pub plugged: bool,
}

impl Battery {
    /// Charge in percent at and below which the battery counts as low.
    const LOW: u8 = 15;

    /// At or below `LOW` and not on external power (when plugged in, being low is being fixed).
    /// False when the charge is unknown.
    pub fn low(&self) -> bool { !self.plugged && self.percent.is_some_and(|p| p <= Self::LOW) }
}

/// Battery and power state.
pub trait Power: Send {
    /// The battery now (may be a slow hardware read).
    fn battery(&self) -> io::Result<Battery>;
    /// Sleeps until woken (power button), returning once awake again.
    fn sleep(&self) -> io::Result<()>;
    /// Turns the device off. The system may stop Specular (SIGTERM) before this returns, so
    /// nothing after the call can be relied on to run.
    fn shutdown(&self) -> io::Result<()>;
    /// Restarts the device. As with `shutdown`, Specular may be stopped before this returns.
    fn restart(&self) -> io::Result<()>;
}

/// The screen's backlight and panel.
pub trait Display: Send {
    /// Backlight brightness, 1–10 (Specular's scale; the device maps it to its own levels).
    fn set_brightness(&self, level: u8) -> io::Result<()>;
    /// Turns the panel off (and its backlight) or back on, for screen-off without sleeping.
    fn set_panel(&self, on: bool) -> io::Result<()>;
}

/// A colour, 0–255 per channel, before the device's brightness limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// The RGB LEDs.
pub trait Lights: Send {
    /// Sets the LEDs, one colour per LED (as many as the profile lists). Implementations apply
    /// the device's brightness limits.
    fn show(&self, colours: &[Rgb]) -> io::Result<()>;
}

/// Where sound plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// The device's own speaker; also its headphone jack on a device that can't tell when
    /// headphones are plugged in (`Hardware::headphones` is None).
    Speaker,
    /// Headphones in the device's jack.
    Headphones,
    /// A connected Bluetooth audio device.
    Bluetooth(BtAddress),
}

impl Output {
    /// The setting holding this output's volume (all Bluetooth devices share one).
    pub fn volume_setting(&self) -> &'static str {
        use crate::settings::builtin;
        match self { Output::Speaker => builtin::VOLUME, Output::Headphones => builtin::HEADPHONE_VOLUME, Output::Bluetooth(_) => builtin::BLUETOOTH_VOLUME }
    }

    /// Whether a game's open `SoundOut` can carry on unchanged when sound moves between these
    /// outputs. True for the speaker and the jack: the device's sound driver switches one stream
    /// between them.
    pub fn same_stream(&self, other: &Output) -> bool {
        self == other || !matches!(self, Output::Bluetooth(_)) && !matches!(other, Output::Bluetooth(_))
    }
}

/// Sound output.
pub trait Audio: Send {
    /// An output's volume, 0–20 (Specular's scale; the device maps it to its own controls).
    /// The speaker's and the headphones' are set whenever they change or the sound is muted or
    /// unmuted, whichever output plays. A port must keep both and play the level of the output in
    /// use (on the Brick the kernel's codec driver does), so setting the other one isn't heard; a
    /// port without a jack (`Hardware::headphones` is None) may ignore the headphones'. A
    /// Bluetooth output's is set only while sound goes to it.
    fn set_volume(&self, output: &Output, level: u8) -> io::Result<()>;
}

/// Whether headphones are plugged into the device's jack. The device moves the sound between the
/// speaker and the jack by itself, each at its own level (see `Audio::set_volume`); this only says
/// which level the volume keys and the gauge act on.
pub trait HeadphoneJack: Send {
    /// Follows the jack until it can't any more, blocking (the UI crate gives it a thread of its
    /// own): calls `plugged` with the state at the start, if it can be read, then on each change.
    fn watch(self: Box<Self>, plugged: Box<dyn FnMut(bool) + Send>);
}

/// Where a game's or the interface's sound goes: a queue of 16-bit stereo frames, played at a
/// fixed rate.
pub trait SoundOut: Send {
    /// Frames played per second.
    fn rate(&self) -> u32;
    /// Queues as many of `frames` (left, right) as fit without waiting; returns how many.
    fn write(&mut self, frames: &[[i16; 2]]) -> io::Result<usize>;
    /// Frames queued and not played yet.
    fn queued(&mut self) -> usize;
    /// How many frames the queue holds.
    fn capacity(&self) -> usize;
}

/// How strongly a rumble's two motors run, 0 (still) to 65535 each, as libretro and Linux's force
/// feedback take them: the strong (low-frequency) motor and the weak (high-frequency) one. A device
/// with one motor runs it at the stronger of the two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Motors { pub strong: u16, pub weak: u16 }

impl Motors {
    pub const STILL: Motors = Motors { strong: 0, weak: 0 };

    /// The stronger of the two, motor by motor.
    pub fn max(self, o: Motors) -> Motors { Motors { strong: self.strong.max(o.strong), weak: self.weak.max(o.weak) } }
}

/// The rumble motors.
pub trait Rumble: Send {
    /// Runs the motors at `motors` until the next call (`Motors::STILL` stops them). May block for
    /// a while (on Linux a program presenting the gamepad may have to answer first), so the UI
    /// crate calls it on a thread of its own (see `rumble::RumbleLine`).
    fn set(&mut self, motors: Motors) -> io::Result<()>;
}

/// The display's refresh, for pacing a game's frames, and the UI's where the device's Slint
/// platform uses it.
pub trait Vsync: Send {
    /// Waits until the display next starts showing a new frame.
    fn wait(&mut self) -> io::Result<()>;
}

/// A device's hardware services.
pub struct Hardware {
    pub power: Box<dyn Power>,
    pub display: Box<dyn Display>,
    pub lights: Box<dyn Lights>,
    pub audio: Box<dyn Audio>,
    /// None on devices without WiFi.
    pub wifi: Option<Box<wireless::Wifi>>,
    /// None on devices without Bluetooth.
    pub bluetooth: Option<Box<wireless::Bluetooth>>,
    /// None on devices without internal storage to install onto.
    pub installer: Option<Box<dyn crate::install::Installer>>,
    /// None on devices that can't tell when headphones are plugged in.
    pub headphones: Option<Box<dyn HeadphoneJack>>,
    /// None on devices without a rumble motor.
    pub rumble: Option<Box<dyn Rumble>>,
}

/// A device: what it is, and its hardware.
pub trait Device {
    fn profile(&self) -> DeviceProfile;
    /// The hardware services. Called once; they then move to the UI crate's hardware thread.
    fn hardware(&self) -> Hardware;
    /// Opens a sound output for a game (each game opens its own and closes it when it ends, and
    /// opens another when the output changes while it plays, unless `Output::same_stream`); also
    /// for the interface sounds. Specular never has two open at once (the interface sounds close
    /// theirs before a game opens one), so an implementation needn't support that.
    fn open_sound(&self, output: &Output) -> io::Result<Box<dyn SoundOut>>;
    /// The display's refresh, if the device can wait for it; without, games are paced by a
    /// timer at their own frame rate. Each call opens a separate one: give each waiting thread its
    /// own.
    fn open_vsync(&self) -> Option<Box<dyn Vsync>>;
}

/// Simulated hardware, for the desktop build: it prints what it is asked to do (rumble too), its
/// battery drains by 1 % a minute from 80 % (or $SPECULAR_BATTERY %) down to 5 %, it has no
/// headphone jack (a desktop key simulates one, see `specular_ui::run`), and its radios follow a
/// scenario (`wireless::scenario`): the file named by $SPECULAR_SCENARIO, `{"wifi": <script>,
/// "bluetooth": <script>}`, else a built-in one with a few networks and devices.
pub mod simulated {
    use std::time::Instant;

    use crate::wireless::scenario::Scenario;
    use crate::wireless::trace::Traced;

    use super::*;

    struct Sim(Instant);

    impl Power for Sim {
        fn battery(&self) -> io::Result<Battery> {
            let start = std::env::var("SPECULAR_BATTERY").ok().and_then(|s| s.parse().ok()).unwrap_or(80u8).clamp(5, 100);
            let drained = (self.0.elapsed().as_secs() / 60).min(100) as u8;
            Ok(Battery { percent: Some(start.saturating_sub(drained).max(5)), charging: false, plugged: false })
        }
        fn sleep(&self) -> io::Result<()> { eprintln!("simulated: sleep"); std::thread::sleep(Duration::from_millis(500)); Ok(()) }
        fn shutdown(&self) -> io::Result<()> { eprintln!("simulated: shut down"); Ok(()) }
        fn restart(&self) -> io::Result<()> { eprintln!("simulated: restart"); Ok(()) }
    }
    impl Display for Sim {
        fn set_brightness(&self, level: u8) -> io::Result<()> { eprintln!("simulated: brightness {level}"); Ok(()) }
        fn set_panel(&self, on: bool) -> io::Result<()> { eprintln!("simulated: panel {}", if on { "on" } else { "off" }); Ok(()) }
    }
    impl Lights for Sim {
        fn show(&self, colours: &[Rgb]) -> io::Result<()> { eprintln!("simulated: lights {colours:?}"); Ok(()) }
    }
    impl Audio for Sim {
        fn set_volume(&self, output: &Output, level: u8) -> io::Result<()> { eprintln!("simulated: {output:?} volume {level}"); Ok(()) }
    }
    impl Rumble for Sim {
        fn set(&mut self, m: Motors) -> io::Result<()> { eprintln!("simulated: rumble strong {} weak {}", m.strong, m.weak); Ok(()) }
    }

    pub fn hardware() -> Hardware {
        let t = Instant::now();
        let script = scenario().unwrap_or_else(|e| panic!("$SPECULAR_SCENARIO: {e}"));
        Hardware {
            power: Box::new(Sim(t)), display: Box::new(Sim(t)), lights: Box::new(Sim(t)), audio: Box::new(Sim(t)),
            wifi: Some(Box::new(Traced::new("wifi", radio(&script, "wifi")))),
            bluetooth: Some(Box::new(Traced::new("bluetooth", radio(&script, "bluetooth")))),
            installer: Some(Box::new(crate::install::Simulated)),
            headphones: None,
            rumble: Some(Box::new(Sim(t))),
        }
    }

    /// The scenario for one radio, `name`, in `script` (an empty one if it has none).
    pub(super) fn radio<E: serde::de::DeserializeOwned + 'static>(script: &serde_json::Value, name: &str) -> Scenario<E> {
        Scenario::from_json(script.get(name).unwrap_or(&serde_json::Value::Null)).unwrap_or_else(|e| panic!("scenario: {name}: {e}"))
    }

    /// The radios' scenario: $SPECULAR_SCENARIO's, or the built-in one.
    fn scenario() -> Result<serde_json::Value, String> {
        let text = match std::env::var_os("SPECULAR_SCENARIO") {
            Some(path) => std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.to_string_lossy()))?,
            None => include_str!("wireless/default-scenario.json").to_string(),
        };
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_battery_is_low_only_off_power() {
        let battery = |percent, plugged| super::Battery { percent, charging: plugged, plugged };
        assert!(battery(Some(15), false).low());
        assert!(!battery(Some(16), false).low());
        assert!(!battery(Some(5), true).low());
        assert!(!battery(None, false).low());
    }

    #[test]
    fn the_built_in_scenario_reads() {
        let script = serde_json::from_str(include_str!("wireless/default-scenario.json")).unwrap();
        let _: crate::wireless::scenario::Scenario<crate::wireless::wifi::WifiEvent> = super::simulated::radio(&script, "wifi");
        let _: crate::wireless::scenario::Scenario<crate::wireless::bluetooth::BtEvent> = super::simulated::radio(&script, "bluetooth");
    }
}
