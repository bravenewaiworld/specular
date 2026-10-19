//! Effects: brief reactions of the lights and the rumble to events (the battery becoming low, the
//! game saving, its sound), drawn over the accent colours. Each is an `Effect`; `Effects` runs an
//! ordered list of them (the built-ins, `builtin`; plugins will add their own), each turned on by
//! a setting. Effects only react: between events nothing runs, and nothing is redrawn.
//!
//! The lights are one colour per accent (`device::Accent`): each LED shows the colour of the accent
//! the device profile gives it (`DeviceProfile::leds`), and the on-screen light bar the first
//! accent's, as the LED bar does. So the screen mirrors the LEDs on any device, with or without
//! them.
//!
//! Times are on one monotonic clock (the UI's `now()`), for `event` and `draw` alike.

use std::time::Duration;

use crate::device::{Accent, Battery, Motors, Rgb};
use crate::palette;
use crate::settings::builtin;

/// Something effects react to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A battery reading (every reading, changed or not).
    Battery(Battery),
    /// The running game wrote its battery save (it saved in-game).
    GameSaved,
    /// A save state was made from the in-game menu.
    StateSaved,
    /// The loudest sample of the game's sound since the last `Sound`, 0–32768. Sent with each game
    /// frame the UI receives, while the game runs.
    Sound(u16),
    /// The game paused (its menu, sleep), was quit or ended.
    GameStopped,
}

/// What the lights and the rumble show at a moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// A colour for each accent's lights, by `Accent as usize`, before the LEDs' brightness.
    pub lights: [Rgb; 2],
    /// The effects' rumble (the game's own is added by `rumble::RumbleLine`).
    pub rumble: Motors,
}

impl Frame {
    pub fn light(&self, accent: Accent) -> Rgb { self.lights[accent as usize] }
}

/// A brief reaction of the lights or the rumble to events.
pub trait Effect {
    /// Its name, for the log.
    fn name(&self) -> &'static str;
    /// The Bool setting that turns it on. While off it gets no events and draws nothing.
    fn setting(&self) -> &'static str;
    /// Reacts to `event` at `now`. True if it starts running (it wasn't already), which is logged.
    fn event(&mut self, event: Event, now: Duration) -> bool;
    /// Draws itself at `now` over `frame` (the accents and the effects before it in the list): it
    /// may replace the lights, and only raises the rumble (`Motors::max`), so the strongest wins.
    /// Returns the time it next changes, or None if it isn't showing anything (it draws nothing
    /// then).
    fn draw(&self, now: Duration, frame: &mut Frame) -> Option<Duration>;
}

/// The effects, in drawing order (a later one's lights cover an earlier one's), each on or off by
/// its setting. All are off until their settings are applied.
pub struct Effects { list: Vec<(Box<dyn Effect>, bool)> }

impl Effects {
    pub fn new(list: Vec<Box<dyn Effect>>) -> Self { Self { list: list.into_iter().map(|e| (e, false)).collect() } }

    /// Turns on or off each effect whose setting is `key`; other keys are ignored.
    pub fn apply(&mut self, key: &str, value: &serde_json::Value) {
        for (e, on) in &mut self.list { if e.setting() == key { *on = value.as_bool().unwrap_or(false); } }
    }

    /// Gives `event` to the effects that are on; calls `started` with the name of each that starts.
    /// Then `frame` says what to show.
    pub fn event(&mut self, event: Event, now: Duration, mut started: impl FnMut(&'static str)) {
        for (e, on) in &mut self.list { if *on && e.event(event, now) { started(e.name()); } }
    }

    /// What to show at `now` with these accents (first, second), and when it next changes (None:
    /// not until the next event).
    pub fn frame(&self, now: Duration, accents: [Rgb; 2]) -> (Frame, Option<Duration>) {
        let mut frame = Frame { lights: accents, rumble: Motors::STILL };
        // (Every effect draws, in order: `min` takes them all.)
        let next = self.list.iter().filter(|(_, on)| *on).filter_map(|(e, _)| e.draw(now, &mut frame)).min();
        (frame, next)
    }
}

/// The built-in effects, in drawing order.
pub fn builtin() -> Vec<Box<dyn Effect>> {
    vec![Box::new(LowBattery::default()), Box::new(SaveFlash::default()), Box::new(FollowSound::default())]
}

/// As the battery becomes low (also at the first reading, if it already is), the lights pulse
/// between the warning colour and the accents `PULSES` times.
#[derive(Default)]
pub struct LowBattery { low: bool, since: Option<Duration> }

impl LowBattery {
    const PULSES: u32 = 3;
    /// Each half of a pulse: warning, then the accents. The light bar on screen fades between them
    /// (its colour changes over 350 ms, `Motion.tint` in theme.slint); the LEDs switch.
    const HALF: Duration = Duration::from_millis(500);
}

impl Effect for LowBattery {
    fn name(&self) -> &'static str { "low battery" }
    fn setting(&self) -> &'static str { builtin::LOW_BATTERY_LIGHTS }

    fn event(&mut self, event: Event, now: Duration) -> bool {
        let Event::Battery(b) = event else { return false };
        let became = b.low() && !self.low;
        self.low = b.low();
        if became { self.since = Some(now); }
        became
    }

    fn draw(&self, now: Duration, frame: &mut Frame) -> Option<Duration> {
        let since = self.since?;
        let half = (now.saturating_sub(since).as_millis() / Self::HALF.as_millis()) as u32;
        if half >= 2 * Self::PULSES { return None; }
        if half.is_multiple_of(2) { frame.lights = [palette::WARNING; 2]; }
        Some(since + Self::HALF * (half + 1))
    }
}

/// When the game saves (in-game, or a save state), the lights flash white for `FLASH`.
#[derive(Default)]
pub struct SaveFlash { at: Option<Duration> }

impl SaveFlash {
    const FLASH: Duration = Duration::from_millis(300);
    const WHITE: Rgb = Rgb(255, 255, 255);
}

impl Effect for SaveFlash {
    fn name(&self) -> &'static str { "save flash" }
    fn setting(&self) -> &'static str { builtin::SAVE_FLASH }

    fn event(&mut self, event: Event, now: Duration) -> bool {
        if !matches!(event, Event::GameSaved | Event::StateSaved) { return false; }
        let running = self.at.is_some_and(|t| now < t + Self::FLASH);
        self.at = Some(now);
        !running
    }

    fn draw(&self, now: Duration, frame: &mut Frame) -> Option<Duration> {
        let end = self.at? + Self::FLASH;
        if now >= end { return None; }
        frame.lights = [Self::WHITE; 2];
        Some(end)
    }
}

/// Rumbles with the game's loud sounds: a frame whose peak is at least `MIN_PEAK` and 1.5 times the
/// recent average rumbles for `KICK`, more strongly the louder it is. So sudden sounds (explosions,
/// drums) rumble, and steady loud music settles. It counts as running (for the log) from its first
/// rumble until the game stops.
#[derive(Default)]
pub struct FollowSound {
    /// The recent peaks' average: each frame's peak moves it an eighth of the way.
    average: u16,
    /// The last rumble's end and strength; None until the first since the game stopped.
    kick: Option<(Duration, u16)>,
}

impl FollowSound {
    /// About -12 dBFS.
    const MIN_PEAK: u16 = 8192;
    const KICK: Duration = Duration::from_millis(100);
}

impl Effect for FollowSound {
    fn name(&self) -> &'static str { "rumble following the sound" }
    fn setting(&self) -> &'static str { builtin::RUMBLE_FOLLOWS_SOUND }

    fn event(&mut self, event: Event, now: Duration) -> bool {
        match event {
            Event::Sound(peak) => {
                let average = self.average as u32;
                self.average = ((average * 7 + peak as u32) / 8) as u16;
                if peak < Self::MIN_PEAK || (peak as u32) * 2 < average * 3 { return false; }
                let above = (peak - Self::MIN_PEAK) as u32 * 0xffff / (32768 - Self::MIN_PEAK) as u32;
                // At least a quarter: a motor barely turning can't be felt.
                let strength = above.max(0x4000) as u16;
                self.kick.replace((now + Self::KICK, strength)).is_none()
            }
            Event::GameStopped => { *self = Self::default(); false }
            _ => false,
        }
    }

    fn draw(&self, now: Duration, frame: &mut Frame) -> Option<Duration> {
        let (end, strength) = self.kick.filter(|(end, _)| now < *end)?;
        frame.rumble = frame.rumble.max(Motors { strong: strength, weak: strength });
        Some(end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCENTS: [Rgb; 2] = [Rgb(1, 2, 3), Rgb(4, 5, 6)];
    const fn ms(n: u64) -> Duration { Duration::from_millis(n) }

    /// The built-in effects, all on.
    fn effects() -> Effects {
        let mut e = Effects::new(builtin());
        for key in [builtin::LOW_BATTERY_LIGHTS, builtin::SAVE_FLASH, builtin::RUMBLE_FOLLOWS_SOUND] { e.apply(key, &true.into()); }
        e
    }

    fn battery(percent: u8) -> Event { Event::Battery(Battery { percent: Some(percent), charging: false, plugged: false }) }

    /// Feeds `event` at `now`, returning the names of the effects that started.
    fn feed(e: &mut Effects, event: Event, now: Duration) -> Vec<&'static str> {
        let mut started = Vec::new();
        e.event(event, now, |name| started.push(name));
        started
    }

    #[test]
    fn nothing_shows_but_the_accents_until_an_event() {
        assert_eq!(effects().frame(ms(5000), ACCENTS), (Frame { lights: ACCENTS, rumble: Motors::STILL }, None));
    }

    #[test]
    fn the_lights_pulse_once_as_the_battery_becomes_low() {
        let mut e = effects();
        assert!(feed(&mut e, battery(40), ms(0)).is_empty());
        assert_eq!(feed(&mut e, battery(15), ms(10_000)), ["low battery"]);
        let at = |t: u64| e.frame(ms(10_000 + t), ACCENTS);
        assert_eq!(at(0), (Frame { lights: [palette::WARNING; 2], rumble: Motors::STILL }, Some(ms(10_500))));
        assert_eq!(at(700).0.lights, ACCENTS);
        assert_eq!(at(700).1, Some(ms(11_000)));
        assert_eq!(at(2_100).0.lights, [palette::WARNING; 2]); // the third pulse
        assert_eq!(at(3_000), (Frame { lights: ACCENTS, rumble: Motors::STILL }, None));
        // Still low: no pulse again, until it has been charged above low and becomes low again.
        assert!(feed(&mut e, battery(12), ms(20_000)).is_empty());
        assert_eq!(e.frame(ms(20_000), ACCENTS).1, None);
        feed(&mut e, Event::Battery(Battery { percent: Some(12), charging: true, plugged: true }), ms(30_000));
        assert_eq!(feed(&mut e, battery(12), ms(40_000)), ["low battery"]);
    }

    #[test]
    fn saving_flashes_the_lights_over_the_low_battery_pulse() {
        let mut e = effects();
        feed(&mut e, battery(10), ms(0));
        assert_eq!(feed(&mut e, Event::GameSaved, ms(600)), ["save flash"]);
        assert_eq!(e.frame(ms(700), ACCENTS), (Frame { lights: [SaveFlash::WHITE; 2], rumble: Motors::STILL }, Some(ms(900))));
        // A second save while flashing makes it last longer, but isn't a new start.
        assert!(feed(&mut e, Event::StateSaved, ms(800)).is_empty());
        assert_eq!(e.frame(ms(1_050), ACCENTS).0.lights, [SaveFlash::WHITE; 2]);
        assert_eq!(e.frame(ms(1_100), ACCENTS).0.lights, [palette::WARNING; 2]);
    }

    #[test]
    fn an_effect_turned_off_neither_reacts_nor_shows() {
        let mut e = effects();
        feed(&mut e, Event::GameSaved, ms(0));
        e.apply(builtin::SAVE_FLASH, &false.into());
        assert_eq!(e.frame(ms(100), ACCENTS).0.lights, ACCENTS);
        assert!(feed(&mut e, Event::GameSaved, ms(200)).is_empty());
        e.apply(builtin::SAVE_FLASH, &true.into());
        assert_eq!(e.frame(ms(1_000), ACCENTS).1, None);
    }

    #[test]
    fn sudden_loud_sounds_rumble_and_steady_ones_settle() {
        let mut e = effects();
        // Quiet sound: nothing.
        for i in 0..30 { assert!(feed(&mut e, Event::Sound(3_000), ms(i * 16)).is_empty()); }
        assert_eq!(e.frame(ms(480), ACCENTS).0.rumble, Motors::STILL);
        // A bang: a rumble, logged once, strongest at full scale.
        assert_eq!(feed(&mut e, Event::Sound(32_768), ms(496)), ["rumble following the sound"]);
        assert_eq!(e.frame(ms(500), ACCENTS), (Frame { lights: ACCENTS, rumble: Motors { strong: 0xffff, weak: 0xffff } }, Some(ms(596))));
        assert_eq!(e.frame(ms(596), ACCENTS), (Frame { lights: ACCENTS, rumble: Motors::STILL }, None));
        // Held loud: the average catches up, and the rumbling stops.
        let rumbling: Vec<bool> = (0..30).map(|i| {
            let t = ms(600 + i * 16);
            feed(&mut e, Event::Sound(20_000), t);
            e.frame(t, ACCENTS).0.rumble != Motors::STILL
        }).collect();
        assert!(rumbling[0] && !rumbling[29], "{rumbling:?}");
        // A pause ends it; its next rumble is a new start.
        feed(&mut e, Event::GameStopped, ms(2_000));
        assert_eq!(feed(&mut e, Event::Sound(20_000), ms(3_000)), ["rumble following the sound"]);
    }
}
