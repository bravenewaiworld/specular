//! The CPU's minimum speed while a game plays, chosen from the game's own frames.
//!
//! The system's governor sets the CPU's speed from how busy it is, but a game paced by the display
//! idles for part of every refresh, and for a whole refresh more after a frame that missed one, so
//! the governor can settle too low for the game to keep up (docs/runtime.md, Findings). Only
//! Specular sees a missed refresh, so `CpuMin` raises a minimum speed when frames miss theirs and
//! lowers it once they have had time to spare for a while. It only decides; the runtime measures
//! the frames, and the device sets the speed.

use std::time::Duration;

/// `settings::builtin::CPU_MIN`'s value for a minimum chosen by `CpuMin` (its default).
pub const AUTO: &str = "auto";
/// `settings::builtin::CPU_MIN`'s value for no minimum: the speed is the system's choice.
pub const OFF: &str = "off";

/// The time the UI needs to draw and present a frame once the core has made it. The runtime's
/// frame delay leaves this much of each refresh, and a step down is taken only if it would still
/// leave this much after the slowest runs (see `HEADROOM`).
pub const UI_TIME: Duration = Duration::from_millis(6);

/// The lowest step `CpuMin::new` uses; it drops the steps below. So its first step up from the
/// system's default is to this speed, and a step down from it goes back to the default. Chosen
/// for the Brick: the steps up to 1008 MHz run at 900 mV and 1200 MHz at 937.5 mV (Crystaline OS's
/// docs/power.md), so holding 1200 costs little, and the governor already runs a busy game mostly
/// at or above it (docs/runtime.md, Findings), so a lower minimum would gain little. A CPU whose
/// steps all lie below it gets no automatic minimum.
const LOWEST_USEFUL_MHZ: u32 = 1200;
/// Two misses at most this far apart raise the minimum a step. One alone (a scene loading, say)
/// doesn't.
const MISS_PAIR: Duration = Duration::from_secs(1);
/// How long every frame must be on time before the minimum may drop a step.
const ON_TIME: Duration = Duration::from_secs(10);
/// A step down is taken only if the slowest runs, slowed to the lower speed, and `UI_TIME` would
/// fit in this share of a refresh (percent).
const HEADROOM: u32 = 75;

/// Chooses the CPU's minimum speed, among the CPU's speed steps, from the game's frames (see the
/// module's documentation). Its levels are, lowest first, the system's default (None) and each
/// step it keeps (`new` drops those below `LOWEST_USEFUL_MHZ`). Its clock is the game's time of
/// play: the gaps given to `frame`, added up.
#[derive(Debug, Default)]
pub struct CpuMin {
    /// The steps this minimum may use (MHz, ascending); empty where the speed can't be set, or
    /// none is usable.
    steps: Vec<u32>,
    /// The level now: an index into `steps`; None: the system's default.
    level: Option<usize>,
    /// Whether the level follows the frames; if not, it stays as it started.
    adaptive: bool,
    /// The time of play so far.
    played: Duration,
    /// When (in time of play) the latest miss still waiting for a second within `MISS_PAIR` came.
    last_miss: Option<Duration>,
    /// When (in time of play) the on-time window began: the last miss, or the last check for a
    /// step down.
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
    /// the top step, the top step; None: the system's default). Steps below `LOWEST_USEFUL_MHZ`
    /// are never used: a `start` below it rounds up to it.
    pub fn new(mut steps: Vec<u32>, start: Option<u32>) -> Self {
        steps.retain(|&s| s >= LOWEST_USEFUL_MHZ);
        Self { adaptive: true, ..Self::fixed(steps, start) }
    }

    /// A minimum that stays at `mhz` (rounded up to a step; above the top step, the top step) for
    /// the whole game. Keeps all of `steps`, those below `LOWEST_USEFUL_MHZ` too: it is for a speed
    /// set in the config files, used as set.
    pub fn fixed(steps: Vec<u32>, mhz: Option<u32>) -> Self {
        let level = mhz.and_then(|m| steps.iter().position(|&s| s >= m).or(steps.len().checked_sub(1)));
        Self { steps, level, ..Self::default() }
    }

    /// The minimum now (MHz; None: the system's default).
    pub fn current(&self) -> Option<u32> { self.level.map(|i| self.steps[i]) }

    /// One frame paced by the display. `gap`: the time between the vsync this frame ran after and
    /// the one the previous frame ran after; `run`: how long the core took to make it; `period`:
    /// the display's refresh period. A gap of more than 1.5 periods (halfway to two, allowing for
    /// jitter) is a missed refresh. The gaps add up to the clock of play, so leave out frames whose
    /// gap isn't play: the first after a pause (the gap would include the pause), and any paced by
    /// a timer (which has no refreshes to miss). Returns a decision when there is one to act on.
    pub fn frame(&mut self, gap: Duration, run: Duration, period: Duration) -> Option<Decision> {
        if !self.adaptive || self.steps.is_empty() { return None; }
        self.played += gap;
        if gap > period * 3 / 2 { return self.missed(); }
        self.runs.push(run);
        if self.played - self.on_time_since < ON_TIME { return None; }
        let decision = self.step_down(period);
        self.restart();
        decision
    }

    /// A frame missed its refresh: the second within `MISS_PAIR` raises the level a step. At the
    /// top step it can't, and says so once (`Decision::Note`).
    fn missed(&mut self) -> Option<Decision> {
        let now = self.played;
        let first = self.last_miss.filter(|&t| now - t <= MISS_PAIR);
        self.restart();
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
    /// slowed in proportion to the lower speed, plus `UI_TIME`, fit in `HEADROOM` percent of a
    /// refresh. From the lowest step to the system's default, the runs are judged at the lowest
    /// step's speed, as the governor runs a busy game about that fast (see `LOWEST_USEFUL_MHZ`).
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

    /// Starts the on-time window afresh at the time of play now, forgetting the runs and any miss
    /// still waiting for a second.
    fn restart(&mut self) {
        self.on_time_since = self.played;
        self.runs.clear();
        self.last_miss = None;
    }
}

/// The `CpuMin` for a game, as `setting` says (a value of `settings::builtin::CPU_MIN`: `AUTO`,
/// `OFF` or a speed in MHz), `start` being its core's start level (`CoreDecl::cpu_start`), and
/// `steps` the CPU's (empty: the speed can't be set, and none is). Also says where its first
/// level comes from, for the log.
pub fn for_game(setting: &str, start: Option<u32>, steps: Vec<u32>) -> (CpuMin, &'static str) {
    match setting {
        OFF => (CpuMin::default(), "the config files turn it off"),
        // The settings accept only `AUTO`, `OFF` and numbers; anything else is taken as `AUTO`.
        _ => match setting.parse() {
            Ok(mhz) => (CpuMin::fixed(steps, Some(mhz)), "as the config files set it, for the whole game"),
            Err(_) => (CpuMin::new(steps, start), "the core's start level, then as the frames need"),
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

    /// Plays frames on time (a refresh apart) for at least `play`, each run taking `run`; returns
    /// the decisions made.
    fn on_time(c: &mut CpuMin, play: Duration, run: Duration) -> Vec<Decision> {
        let frames = play.as_nanos().div_ceil(PERIOD.as_nanos()) as u32;
        (0..frames).filter_map(|_| c.frame(PERIOD, run, PERIOD)).collect()
    }

    /// One frame that missed its refresh (shown two refreshes after the one before).
    fn miss(c: &mut CpuMin) -> Option<Decision> { c.frame(PERIOD * 2, ms(15), PERIOD) }

    #[test]
    fn starts_at_its_start_level_rounded_up_to_a_step() {
        assert_eq!(CpuMin::new(STEPS.into(), Some(1800)).current(), Some(1800));
        assert_eq!(CpuMin::new(STEPS.into(), Some(1700)).current(), Some(1800));
        assert_eq!(CpuMin::new(STEPS.into(), Some(5000)).current(), Some(2000));
        assert_eq!(CpuMin::new(STEPS.into(), Some(408)).current(), Some(1200), "no lower than LOWEST_USEFUL_MHZ");
        assert_eq!(CpuMin::new(STEPS.into(), None).current(), None);
        assert_eq!(CpuMin::new(vec![], Some(1800)).current(), None, "where the speed can't be set");
    }

    #[test]
    fn a_gap_of_over_one_and_a_half_refreshes_is_a_miss() {
        let mut c = CpuMin::new(STEPS.into(), Some(1608));
        let half = PERIOD / 2;
        miss(&mut c); // the first of a pair
        // Were it a miss, it would make a pair with the one before and step up.
        assert_eq!(c.frame(PERIOD + half, ms(15), PERIOD), None, "1.5 refreshes is on time");
        assert!(matches!(c.frame(PERIOD + half + ms(1), ms(15), PERIOD), Some(Decision::Set(Some(1800), _))));
    }

    #[test]
    fn one_long_frame_is_ignored() {
        let mut c = CpuMin::new(STEPS.into(), Some(1800));
        assert_eq!(miss(&mut c), None);
        // A second miss more than 1 s of play later doesn't make a pair either.
        assert!(on_time(&mut c, ms(1000), ms(4)).is_empty());
        assert_eq!(miss(&mut c), None);
        assert_eq!(c.current(), Some(1800));
    }

    #[test]
    fn two_misses_within_a_second_step_up() {
        let mut c = CpuMin::new(STEPS.into(), Some(1608));
        assert_eq!(miss(&mut c), None);
        on_time(&mut c, ms(350), ms(15));
        assert_eq!(miss(&mut c), Some(Decision::Set(Some(1800), "2 frames missed their refresh in 0.4 s at 1608 MHz".into())));
    }

    #[test]
    fn from_the_systems_default_the_first_step_up_is_to_1200_mhz() {
        let mut c = CpuMin::new(STEPS.into(), None);
        miss(&mut c);
        assert!(matches!(miss(&mut c), Some(Decision::Set(Some(1200), _))));
        miss(&mut c);
        assert!(matches!(miss(&mut c), Some(Decision::Set(Some(1416), _))));
    }

    #[test]
    fn the_top_step_holds_and_says_so_once() {
        let mut c = CpuMin::new(STEPS.into(), Some(2000));
        miss(&mut c);
        let Some(Decision::Note(note)) = miss(&mut c) else { panic!("no note") };
        assert!(note.contains("top step"), "{note}");
        miss(&mut c);
        assert_eq!(miss(&mut c), None, "said once");
        assert_eq!(c.current(), Some(2000));
    }

    #[test]
    fn ten_seconds_on_time_with_room_to_spare_step_down() {
        let mut c = CpuMin::new(STEPS.into(), Some(1800));
        // 4 ms at 1800 MHz is about 4.5 ms at 1608: with the UI's 6 ms, well within 75 % of a refresh.
        let decisions = on_time(&mut c, ms(10_000), ms(4));
        let [Decision::Set(Some(1608), why)] = &decisions[..] else { panic!("{decisions:?}") };
        assert_eq!(why, "10 s on time; slowest runs 4.0 ms, about 4.5 ms at 1608 MHz");
        // And again after another 10 s, not before.
        assert!(on_time(&mut c, ms(9_000), ms(4)).is_empty());
        assert_eq!(c.current(), Some(1608));
    }

    #[test]
    fn no_step_down_without_room_to_spare() {
        let mut c = CpuMin::new(STEPS.into(), Some(1800));
        // 6 ms at 1800 is 6.7 ms at 1608, and with the UI's 6 ms over 75 % of a refresh (12.5 ms).
        let decisions = on_time(&mut c, ms(30_000), ms(6));
        assert!(decisions.is_empty(), "{decisions:?}");
        assert_eq!(c.current(), Some(1800));
        // A miss restarts the 10 s: none is taken 10 s after the start if a miss came meanwhile.
        let mut c = CpuMin::new(STEPS.into(), Some(1800));
        on_time(&mut c, ms(5_000), ms(4));
        miss(&mut c);
        assert!(on_time(&mut c, ms(9_000), ms(4)).is_empty());
    }

    #[test]
    fn a_step_down_from_1200_mhz_is_to_the_systems_default() {
        let mut c = CpuMin::new(STEPS.into(), Some(1200));
        let decisions = on_time(&mut c, ms(10_000), ms(2));
        assert!(matches!(&decisions[..], [Decision::Set(None, _)]), "{decisions:?}");
        assert_eq!(c.current(), None);
        // Nothing lower then.
        assert!(on_time(&mut c, ms(10_000), ms(2)).is_empty());
    }

    #[test]
    fn a_fixed_minimum_never_changes() {
        let mut c = CpuMin::fixed(STEPS.into(), Some(1800));
        miss(&mut c);
        assert_eq!(miss(&mut c), None);
        assert!(on_time(&mut c, ms(20_000), ms(1)).is_empty());
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
        assert_eq!(level("408"), Some(408), "below LOWEST_USEFUL_MHZ, as set");
        assert!(!for_game("1200", Some(1800), STEPS.into()).0.adaptive, "a speed set holds, as `fixed`");
    }
}
