//! The CPU's minimum speed while a game plays, chosen from the game's own frames.
//!
//! The system's governor sets the CPU's speed from how busy it is, but a game paced by the display
//! idles for part of every refresh, and for a whole refresh more after a frame that missed one, so
//! the governor can settle too low for the game to keep up (docs/runtime.md, Findings). Only
//! Specular sees a missed refresh, so `CpuMin` raises a minimum speed when frames miss theirs and
//! lowers it once they have had time to spare for a while. It only decides; the runtime feeds it
//! the frames, and the device sets the speed.

use std::time::Duration;

/// `settings::builtin::CPU_MIN`'s value for a minimum chosen by `CpuMin` (its default).
pub const AUTO: &str = "auto";
/// `settings::builtin::CPU_MIN`'s value for no minimum: the speed is the system's choice.
pub const OFF: &str = "off";

/// The time the UI needs to draw and present a frame once the core has made it. The runtime's
/// frame delay leaves this much of each refresh, and so must the slowest runs after a step down.
pub const UI_TIME: Duration = Duration::from_millis(6);

/// Two misses at most this far apart raise the minimum a step. One alone (a scene loading, say)
/// doesn't.
const MISS_PAIR: Duration = Duration::from_secs(1);
/// How long every frame must be on time before the minimum may drop a step.
const ON_TIME: Duration = Duration::from_secs(10);
/// A step down is taken only if the slowest runs, slowed to the lower speed, and `UI_TIME` would
/// fit in this share of a refresh (percent).
const HEADROOM: u32 = 75;

/// Chooses the CPU's minimum speed from the game's frames (see the module's documentation), from
/// the CPU's speed steps. Its levels are, lowest first, the system's default (None) and each step.
/// Time is the game's time of play, so time paused doesn't count.
#[derive(Debug, Default)]
pub struct CpuMin {
    /// The CPU's speed steps (MHz, ascending); empty where the speed can't be set.
    steps: Vec<u32>,
    /// The level now: an index into `steps`; None: the system's default.
    level: Option<usize>,
    /// Whether the level follows the frames; if not, it stays as it started.
    adaptive: bool,
    /// When the last missed refresh that didn't raise the level came.
    last_miss: Option<Duration>,
    /// Since when every frame has been on time (or since the level last changed).
    on_time_since: Duration,
    /// How long the core ran for each frame since `on_time_since`.
    runs: Vec<Duration>,
    /// Whether `Decision::Note` has said that frames miss their refresh at the top step.
    told_top: bool,
}

/// What `CpuMin::frame` asks for.
#[derive(Debug, PartialEq)]
pub enum Decision {
    /// Set the minimum to this speed (MHz; None: the system's default), for the reason given (for
    /// the log).
    Set(Option<u32>, String),
    /// Nothing can be done; only log this. Said once: frames still miss their refresh at the top
    /// step (a heat limit may be holding the CPU below it).
    Note(String),
}

impl CpuMin {
    /// A minimum that follows the frames, starting at `start` (MHz, rounded up to a step; above
    /// the top step, the top step; None: the system's default).
    pub fn new(steps: Vec<u32>, start: Option<u32>) -> Self { Self { adaptive: true, ..Self::fixed(steps, start) } }

    /// A minimum that stays at `mhz` (rounded up to a step, as for `new`) for the whole game.
    pub fn fixed(steps: Vec<u32>, mhz: Option<u32>) -> Self {
        let level = mhz.and_then(|m| steps.iter().position(|&s| s >= m).or(steps.len().checked_sub(1)));
        Self { steps, level, ..Self::default() }
    }

    /// The minimum now (MHz; None: the system's default).
    pub fn current(&self) -> Option<u32> { self.level.map(|i| self.steps[i]) }

    /// One frame: `now`, the time of play when it was shown; `missed`, whether it came a refresh
    /// (or more) late; `run`, how long the core took to make it; `period`, the display's refresh.
    /// Returns a decision when there is one to act on.
    pub fn frame(&mut self, now: Duration, missed: bool, run: Duration, period: Duration) -> Option<Decision> {
        if !self.adaptive || self.steps.is_empty() { return None; }
        if missed { return self.missed(now); }
        self.runs.push(run);
        if now.saturating_sub(self.on_time_since) < ON_TIME { return None; }
        let decision = self.step_down(period);
        self.restart(now);
        decision
    }

    /// A frame missed its refresh: the second within `MISS_PAIR` raises the level a step.
    fn missed(&mut self, now: Duration) -> Option<Decision> {
        let first = self.last_miss.filter(|&t| now.saturating_sub(t) <= MISS_PAIR);
        self.restart(now);
        let Some(first) = first else { self.last_miss = Some(now); return None };
        let why = format!("2 frames missed their refresh in {:.1} s at {}", (now - first).as_secs_f64(), text(self.current()));
        let up = self.level.map_or(0, |i| i + 1);
        if up == self.steps.len() {
            if std::mem::replace(&mut self.told_top, true) { return None; }
            return Some(Decision::Note(format!("{why}, the top step: a heat limit may be holding the CPU below it")));
        }
        self.level = Some(up);
        Some(Decision::Set(self.current(), why))
    }

    /// After `ON_TIME` on time: the level a step down, if the slowest runs (the 95th percentile),
    /// slowed in proportion to the lower speed, would still leave `UI_TIME` within `HEADROOM` of a
    /// refresh. Below the lowest step is the system's default, taken as no slower than it.
    fn step_down(&mut self, period: Duration) -> Option<Decision> {
        let i = self.level?;
        let lower = i.checked_sub(1);
        let (speed, lower_speed) = (self.steps[i], self.steps[lower.unwrap_or(0)]);
        self.runs.sort();
        let slowest = self.runs[self.runs.len() * 95 / 100];
        let slowed = slowest * speed / lower_speed;
        if slowed + UI_TIME >= period * HEADROOM / 100 { return None; }
        self.level = lower;
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        Some(Decision::Set(self.current(), format!("{} s on time; slowest runs {:.1} ms, about {:.1} ms at {}",
            ON_TIME.as_secs(), ms(slowest), ms(slowed), text(self.current()))))
    }

    /// Starts the on-time window afresh at `now`.
    fn restart(&mut self, now: Duration) {
        self.on_time_since = now;
        self.runs.clear();
        self.last_miss = None;
    }
}

/// The controller for a game, as `setting` says (a value of `settings::builtin::CPU_MIN`: `AUTO`,
/// `OFF` or a speed in MHz), `start` being its core's start level (`CoreDecl::cpu_start`), and
/// `steps` the CPU's (empty: the speed can't be set, and none is). Also says where its first
/// level comes from, for the log.
pub fn for_game(setting: &str, start: Option<u32>, steps: Vec<u32>) -> (CpuMin, &'static str) {
    match setting {
        OFF => (CpuMin::default(), "the config files turn it off"),
        // The settings accept only `AUTO`, `OFF` and numbers; anything else is taken as `AUTO`.
        _ => match setting.parse() {
            Ok(mhz) => (CpuMin::fixed(steps, Some(mhz)), "as the config files set it, for the whole game"),
            Err(_) => (CpuMin::new(steps, start), "the emulator's start level, then as the frames need"),
        },
    }
}

/// "1800 MHz", or "the system's default" for None, for the log.
pub fn text(mhz: Option<u32>) -> String { mhz.map_or("the system's default".into(), |m| format!("{m} MHz")) }

#[cfg(test)]
mod tests {
    use super::*;

    use crate::device::simulated::STEPS;
    const PERIOD: Duration = Duration::from_micros(16_611); // 60.2 Hz

    fn ms(n: u64) -> Duration { Duration::from_millis(n) }

    /// Plays frames on time for `secs` of play from `from`, each run taking `run`; returns the
    /// decisions made and the time reached.
    fn on_time(c: &mut CpuMin, from: Duration, secs: u64, run: Duration) -> (Vec<Decision>, Duration) {
        let frames = (Duration::from_secs(secs).as_nanos() / PERIOD.as_nanos()) as u32 + 1;
        let decisions = (1..=frames).filter_map(|n| c.frame(from + PERIOD * n, false, run, PERIOD)).collect();
        (decisions, from + PERIOD * frames)
    }

    #[test]
    fn starts_at_its_start_level_rounded_up_to_a_step() {
        assert_eq!(CpuMin::new(STEPS.into(), Some(1800)).current(), Some(1800));
        assert_eq!(CpuMin::new(STEPS.into(), Some(1700)).current(), Some(1800));
        assert_eq!(CpuMin::new(STEPS.into(), Some(5000)).current(), Some(2000));
        assert_eq!(CpuMin::new(STEPS.into(), None).current(), None);
        assert_eq!(CpuMin::new(vec![], Some(1800)).current(), None, "where the speed can't be set");
    }

    #[test]
    fn one_long_frame_is_ignored() {
        let mut c = CpuMin::new(STEPS.into(), Some(1800));
        assert_eq!(c.frame(ms(1000), true, ms(40), PERIOD), None);
        // A second miss more than 1 s later doesn't make a pair either.
        assert_eq!(c.frame(ms(2100), true, ms(40), PERIOD), None);
        assert_eq!(c.current(), Some(1800));
    }

    #[test]
    fn two_misses_within_a_second_step_up() {
        let mut c = CpuMin::new(STEPS.into(), Some(1608));
        assert_eq!(c.frame(ms(1000), true, ms(15), PERIOD), None);
        assert_eq!(c.frame(ms(1400), true, ms(15), PERIOD),
                   Some(Decision::Set(Some(1800), "2 frames missed their refresh in 0.4 s at 1608 MHz".into())));
        // From the system's default, the first step.
        let mut c = CpuMin::new(STEPS.into(), None);
        c.frame(ms(1000), true, ms(15), PERIOD);
        assert!(matches!(c.frame(ms(1100), true, ms(15), PERIOD), Some(Decision::Set(Some(408), _))));
    }

    #[test]
    fn the_top_step_holds_and_says_so_once() {
        let mut c = CpuMin::new(STEPS.into(), Some(2000));
        c.frame(ms(1000), true, ms(20), PERIOD);
        let Some(Decision::Note(note)) = c.frame(ms(1200), true, ms(20), PERIOD) else { panic!("no note") };
        assert!(note.contains("top step"), "{note}");
        c.frame(ms(1300), true, ms(20), PERIOD);
        assert_eq!(c.frame(ms(1400), true, ms(20), PERIOD), None, "said once");
        assert_eq!(c.current(), Some(2000));
    }

    #[test]
    fn ten_seconds_on_time_with_room_to_spare_step_down() {
        let mut c = CpuMin::new(STEPS.into(), Some(1800));
        // 4 ms at 1800 MHz is about 4.5 ms at 1608: with the UI's 6 ms, well within 75 % of a refresh.
        let (decisions, now) = on_time(&mut c, Duration::ZERO, 10, ms(4));
        let [Decision::Set(Some(1608), why)] = &decisions[..] else { panic!("{decisions:?}") };
        assert_eq!(why, "10 s on time; slowest runs 4.0 ms, about 4.5 ms at 1608 MHz");
        // And again after another 10 s, not before.
        assert!(on_time(&mut c, now, 9, ms(4)).0.is_empty());
        assert_eq!(c.current(), Some(1608));
    }

    #[test]
    fn no_step_down_without_room_to_spare() {
        let mut c = CpuMin::new(STEPS.into(), Some(1800));
        // 6 ms at 1800 is 6.7 ms at 1608, and with the UI's 6 ms over 75 % of a refresh (12.5 ms).
        let (decisions, _) = on_time(&mut c, Duration::ZERO, 30, ms(6));
        assert!(decisions.is_empty(), "{decisions:?}");
        assert_eq!(c.current(), Some(1800));
        // A miss restarts the 10 s: none is taken 10 s after the start if a miss came meanwhile.
        let mut c = CpuMin::new(STEPS.into(), Some(1800));
        c.frame(ms(5000), true, ms(4), PERIOD);
        assert!(on_time(&mut c, ms(5000), 9, ms(4)).0.is_empty());
    }

    #[test]
    fn below_the_lowest_step_is_the_systems_default() {
        let mut c = CpuMin::new(STEPS.into(), Some(408));
        let (decisions, now) = on_time(&mut c, Duration::ZERO, 10, ms(2));
        assert!(matches!(&decisions[..], [Decision::Set(None, _)]), "{decisions:?}");
        assert_eq!(c.current(), None);
        // Nothing lower then.
        assert!(on_time(&mut c, now, 10, ms(2)).0.is_empty());
    }

    #[test]
    fn a_fixed_minimum_never_changes() {
        let mut c = CpuMin::fixed(STEPS.into(), Some(1800));
        c.frame(ms(1000), true, ms(30), PERIOD);
        assert_eq!(c.frame(ms(1100), true, ms(30), PERIOD), None);
        assert!(on_time(&mut c, ms(1100), 20, ms(1)).0.is_empty());
        assert_eq!(c.current(), Some(1800));
    }

    #[test]
    fn the_config_files_values() {
        let level = |setting: &str| for_game(setting, Some(1800), STEPS.into()).0.current();
        assert_eq!(level(AUTO), Some(1800), "auto: the core's start level");
        assert_eq!(for_game(AUTO, None, STEPS.into()).0.current(), None, "auto, for a core without one");
        assert_eq!(level(OFF), None);
        assert_eq!(level("1200"), Some(1200));
        assert_eq!(level("1300"), Some(1416), "rounded up to a step");
        assert!(!for_game("1200", Some(1800), STEPS.into()).0.adaptive, "a speed set holds, as `fixed`");
    }
}
