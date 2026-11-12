//! A mainline board as a Specular device: its profile from the board table and the display, and
//! its hardware through the standard interfaces and Crystaline OS's services. The picture can move
//! to a TV and back (`specular_ui::platform::KmsSurface`); `Mainline::on_switch` hears of each
//! move and moves the rest along: the backlight, the refresh rate games are paced by, and,
//! through the app (`Hardware::hdmi`), the sound and a game being played.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex, mpsc};

use specular_core::input::Button;
use specular_core::device::{Sticks, Audio, Battery, Device, DeviceProfile, Display, Hardware, Hdmi, Lights, Output, Power, Rgb, SoundOut, Vsync};
use specular_crystaline::system;
use specular_core::wireless::trace::Traced;
use specular_linux::backlight::SysfsBacklight;
use specular_linux::bluez::Bluez;
use specular_linux::rumble::EvdevRumble;
use specular_linux::sticks::EvdevSticks;
use specular_linux::kms::{self, ActiveCrtc, DrmVsync, Kms, Mode, ModeRequest};
use specular_linux::power_supply::PowerSupply;
use specular_linux::sound::{Alsa, SoftVolume};
use specular_ui::platform::OnSwitch;

use crate::boards::{Board, Mixer};

/// The vblanks the refresh rate is measured over, at start-up and after the picture moves (half a
/// second at 60 Hz).
const MEASURE_VBLANKS: u32 = 30;

/// The board Specular runs on, with what it knows of the display.
pub struct Mainline {
    board: &'static Board,
    card: PathBuf,
    /// The UI's size: the main output's (the panel's), whichever output shows it.
    size: (u32, u32),
    active_crtc: ActiveCrtc,
    request: ModeRequest,
    shown: Arc<Mutex<Shown>>,
    backlight: Arc<Mutex<Backlight>>,
    /// The TV's volume level, which its sound is scaled by (`SoftVolume`).
    hdmi_volume: Arc<AtomicU8>,
    /// The moves to (true) and from a TV, for the app; the first `hardware` call takes the
    /// receiver.
    hdmi: (mpsc::Sender<bool>, Mutex<Option<mpsc::Receiver<bool>>>),
}

/// The output showing the picture, as the device knows it.
struct Shown {
    output: kms::Output,
    /// Its usual mode's measured refresh rate (the mode's own until measured).
    refresh_hz: f64,
    /// The mode asked for last (`match_refresh`).
    current: Mode,
    /// Counts the moves, so a measurement that ends after the next move is dropped.
    moves: u32,
}

/// How far a game's rate may be from a display mode's for that mode to be used for it (0.5 %);
/// the sound is resampled for the rest.
const MATCH: f64 = 0.005;

impl Mainline {
    /// The device on `board`, showing on `kms` (opened from `card`). Measures the panel's refresh
    /// rate, which can differ from what its mode says (the display clock may not reach the mode's
    /// own); falls back to the mode's if vblanks can't be waited for.
    pub fn new(board: &'static Board, kms: &Kms, card: &Path, request: ModeRequest) -> Self {
        let output = kms.output().clone();
        let mode = output.mode;
        let refresh_hz = match kms.vsync(card).and_then(|mut v| v.measure(MEASURE_VBLANKS)) {
            Ok(hz) => { eprintln!("kms: refresh measured {hz:.3} Hz over {MEASURE_VBLANKS} vblanks (mode says {:.3} Hz)", mode.refresh_hz()); hz }
            Err(e) => { eprintln!("kms: can't measure the refresh ({e}); taking the mode's {:.3} Hz", mode.refresh_hz()); mode.refresh_hz() }
        };
        let light = SysfsBacklight::open(Path::new("/sys/class/backlight"), board.backlight)
            .inspect_err(|e| eprintln!("hardware: backlight {}: {e}", board.backlight)).ok();
        let (sender, receiver) = mpsc::channel();
        Self { board, card: card.into(), size: kms.size(), active_crtc: kms.active_crtc(), request,
            shown: Arc::new(Mutex::new(Shown { output, refresh_hz, current: mode, moves: 0 })),
            backlight: Arc::new(Mutex::new(Backlight { light, wanted: true, tv: false })),
            // 0 (silent) until the app sets the TV's level, as sound moves there.
            hdmi_volume: Arc::default(), hdmi: (sender, Mutex::new(Some(receiver))) }
    }

    /// For the display's owner (`KmsSurface::new`), to call as the picture moves to another
    /// output: keeps the backlight off while on a TV, tells the app of a move to or from one,
    /// forgets the mode a game asked for (`match_refresh`: a game resuming asks again), and
    /// measures the new output's refresh rate on a thread of its own, then logs the move as a
    /// `display:` line.
    pub fn on_switch(&self) -> OnSwitch {
        let (board, card, crtc) = (self.board, self.card.clone(), self.active_crtc.clone());
        let (shown, backlight, to_app) = (self.shown.clone(), self.backlight.clone(), self.hdmi.0.clone());
        Box::new(move |output, picture, took| {
            let (moves, was_tv) = {
                let mut s = shown.lock().unwrap();
                let was_tv = s.output.external;
                *s = Shown { output: output.clone(), refresh_hz: output.mode.refresh_hz(), current: output.mode, moves: s.moves + 1 };
                (s.moves, was_tv)
            };
            if let Err(e) = backlight.lock().unwrap().set_tv(output.external) { eprintln!("display: backlight: {e}"); }
            if was_tv != output.external { let _ = to_app.send(output.external); }
            let (shown, card, crtc, output) = (shown.clone(), card.clone(), crtc.clone(), output.clone());
            std::thread::spawn(move || {
                let measured = DrmVsync::open(&card, crtc).and_then(|mut v| v.measure(MEASURE_VBLANKS));
                let refresh = match measured {
                    Ok(hz) => {
                        // A later move's output has its own measurement coming.
                        let mut s = shown.lock().unwrap();
                        if s.moves == moves { s.refresh_hz = hz; }
                        format!("measured {hz:.3} Hz")
                    }
                    Err(e) => format!("refresh not measured ({e})"),
                };
                let sound = if output.external { board.hdmi_sound.unwrap_or("none") } else { board.sound };
                let picture = picture.map_or(String::new(), |p| format!("; the UI scaled to {p}"));
                eprintln!("display: on {} in {}, {refresh}{picture}; sound on {sound}; moved in {} ms",
                    output.name, output.mode, took.as_millis());
            });
        })
    }
}

impl Device for Mainline {
    fn profile(&self) -> DeviceProfile {
        DeviceProfile {
            name: self.board.name,
            width: self.size.0,
            height: self.size.1,
            ppi: self.board.ppi,
            refresh_hz: self.shown.lock().unwrap().refresh_hz,
            buttons: self.board.buttons,
            labels: if self.board.menu_chord { &[(Button::Menu, "SELECT+START")] } else { &[] },
            leds: &[],
        }
    }

    fn hardware(&self) -> Hardware {
        Hardware {
            power: Box::new(MainlinePower(PowerSupply::find(Path::new("/sys/class/power_supply")))),
            display: Box::new(SharedBacklight(self.backlight.clone())),
            lights: Box::new(NoLights),
            audio: Box::new(Volume { mixer: &self.board.volume, hdmi: self.hdmi_volume.clone() }),
            wifi: specular_crystaline::wifi::wifi(),
            bluetooth: Some(Box::new(Traced::new("bluetooth", Bluez::new()))),
            installer: Some(Box::new(specular_crystaline::install::CrystalineInstaller)),
            time_zone: Some(Box::new(specular_crystaline::timezone::CrystalineTimeZone)),
            card: Some(system::card()),
            // The jack cuts the speaker in hardware and reports nothing: one output, one volume.
            headphones: None,
            hdmi: self.hdmi.1.lock().unwrap().take().map(|r| Box::new(HdmiMoves(r)) as Box<dyn Hdmi>),
            rumble: self.board.rumble.map(|m| Box::new(EvdevRumble::motor(m)) as Box<dyn specular_core::device::Rumble>),
            file_transfer: Some(Box::new(specular_crystaline::transfer::WebdavTransfer)),
            ssh: Some(Box::new(specular_crystaline::ssh::CrystalineSsh)),
            cpu: Some(Box::new(specular_crystaline::cpu::CrystalineCpu)),
        }
    }

    fn open_sound(&self, output: &Output) -> io::Result<Box<dyn SoundOut>> {
        Ok(match output {
            Output::Speaker | Output::Headphones => Box::new(Alsa::open(self.board.sound)?),
            Output::Bluetooth(address) => Box::new(Alsa::bluetooth(address)?),
            Output::Hdmi => {
                let card = self.board.hdmi_sound.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "this board has no HDMI sound"))?;
                Box::new(SoftVolume::new(Alsa::open(card)?, self.hdmi_volume.clone()))
            }
        })
    }

    fn open_sticks(&self) -> Option<Box<dyn Sticks>> {
        Some(Box::new(EvdevSticks::new(self.board.sticks?)))
    }

    /// The mode of the output showing the picture nearest `hz`, if one is within `MATCH` of it
    /// (else its usual mode), switched to at the display's next frame. On the panel only where
    /// the board allows it (`Board::game_rates`); on a TV always. The usual mode's rate is the
    /// measured one.
    fn match_refresh(&self, hz: Option<f64>) -> f64 {
        let mut s = self.shown.lock().unwrap();
        let usual = s.output.mode;
        let hz = hz.filter(|_| self.board.game_rates || s.output.external);
        let mode = hz.and_then(|hz| kms::nearest_mode(&s.output.modes, usual, hz, MATCH)).unwrap_or(usual);
        if s.current != mode {
            eprintln!("display: {} → {mode}", hz.map_or("the menus".into(), |hz| format!("a game at {hz:.3} Hz")));
            s.current = mode;
            *self.request.lock().unwrap() = Some(mode);
        }
        if mode == usual { s.refresh_hz } else { mode.refresh_hz() }
    }

    /// The vblanks of the output showing the picture, on a file of its own.
    fn open_vsync(&self) -> Option<Box<dyn Vsync>> {
        match DrmVsync::open(&self.card, self.active_crtc.clone()) {
            Ok(v) => Some(Box::new(v)),
            Err(e) => { eprintln!("vsync: {e}; games are paced by a timer"); None }
        }
    }
}

/// The moves to and from a TV, as `Mainline::on_switch` sends them.
struct HdmiMoves(mpsc::Receiver<bool>);

impl Hdmi for HdmiMoves {
    fn watch(self: Box<Self>, mut on_tv: Box<dyn FnMut(bool) + Send>) { self.0.into_iter().for_each(|tv| on_tv(tv)); }
}

/// The panel's backlight: on as the UI asks (it turns it off and on for screen-off and waking),
/// but always off while the picture is on a TV.
struct Backlight { light: Option<SysfsBacklight>, wanted: bool, tv: bool }

impl Backlight {
    fn light(&self) -> io::Result<&SysfsBacklight> { self.light.as_ref().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no backlight")) }

    fn set_tv(&mut self, tv: bool) -> io::Result<()> { self.tv = tv; self.apply() }

    fn apply(&self) -> io::Result<()> { self.light()?.set_panel(self.wanted && !self.tv) }
}

/// `Backlight` as the app's `Display` (used on its hardware thread); `on_switch` holds it too.
struct SharedBacklight(Arc<Mutex<Backlight>>);

impl Display for SharedBacklight {
    fn set_brightness(&self, level: u8) -> io::Result<()> { self.0.lock().unwrap().light()?.set_brightness(level) }

    fn set_panel(&self, on: bool) -> io::Result<()> {
        let mut b = self.0.lock().unwrap();
        b.wanted = on;
        b.apply()
    }
}

struct MainlinePower(PowerSupply);

impl Power for MainlinePower {
    fn battery(&self) -> io::Result<Battery> { self.0.battery() }

    fn sleep(&self) -> io::Result<()> { system::sleep() }

    fn shutdown(&self) -> io::Result<()> { system::shutdown() }

    fn restart(&self) -> io::Result<()> { system::restart() }

    fn last_shutdown(&self) -> io::Result<Option<String>> { system::last_shutdown() }
}

/// The LEDs stay with the system (status and charging), as the profile lists none.
struct NoLights;

impl Lights for NoLights {
    fn show(&self, _: &[Rgb]) -> io::Result<()> { Ok(()) }
}

/// The volume through the board's mixer control, set with amixer. Specular's levels are 2 dB
/// apart, the top one at 0 dB; 0 is the control's lowest. The speaker's level is the one used
/// (there is no headphone detection), Bluetooth has its own (through bluez-alsa), and a TV its
/// own, in software (`hdmi`, which its `SoftVolume` reads).
struct Volume { mixer: &'static Mixer, hdmi: Arc<AtomicU8> }

impl Audio for Volume {
    fn set_volume(&self, output: &Output, level: u8) -> io::Result<()> {
        match output {
            Output::Speaker => {
                let m = self.mixer;
                system::run("/usr/bin/amixer", &["-q", "-c", m.card, "cset", &format!("name={}", m.control), &step(m, level).to_string()])
            }
            Output::Headphones => Ok(()),
            Output::Bluetooth(address) => specular_linux::sound::set_bluetooth_volume(address, level),
            Output::Hdmi => { self.hdmi.store(level, std::sync::atomic::Ordering::Relaxed); Ok(()) }
        }
    }
}

/// The mixer's value for a volume level.
fn step(m: &Mixer, level: u8) -> u32 {
    let below_top = (specular_core::settings::builtin::VOLUME_MAX - level as i64).max(0) as f64;
    if level == 0 { 0 } else { (m.max as f64 - below_top * 2.0 / m.db_per_step).round().max(0.0) as u32 }
}

#[cfg(test)]
mod tests {
    #[test]
    fn levels_are_2_db_apart_with_the_top_at_0_db() {
        let m = &crate::boards::BOARDS[0].volume;
        let steps: Vec<u32> = [0, 1, 10, 19, 20].into_iter().map(|l| super::step(m, l)).collect();
        assert_eq!(steps, [0, 153, 201, 250, 255]);
    }
}
