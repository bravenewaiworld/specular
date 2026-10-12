//! Auto-repeat for held buttons. Driven by the caller's clock (no timers inside), so it is
//! deterministic and testable: feed it events, ask it for repeats when its deadline comes.

use std::time::Duration;

use crate::input::{Button, ButtonEvent, Press};

/// How held buttons repeat.
#[derive(Clone, Copy, Debug)]
pub struct RepeatTiming {
    /// From the press to the first repeat.
    pub delay: Duration,
    /// Between repeats at first.
    pub interval: Duration,
    /// After repeating this long, repeats come every `fast_interval`.
    pub speed_up_after: Duration,
    pub fast_interval: Duration,
}

impl Default for RepeatTiming {
    /// About 14 repeats a second after 350 ms, then about 30 a second after 1.5 s of repeating.
    fn default() -> Self {
        Self {
            delay: Duration::from_millis(350),
            interval: Duration::from_millis(70),
            speed_up_after: Duration::from_millis(1500),
            fast_interval: Duration::from_millis(33),
        }
    }
}

/// Turns button events into presses, adding repeats while a repeating button is held. Only the
/// most recently pressed repeating button repeats; pressing any other button, or releasing it,
/// stops it.
#[derive(Debug, Default)]
pub struct Repeater {
    timing: RepeatTiming,
    held: Option<Held>,
}

#[derive(Debug)]
struct Held { button: Button, first_repeat: Duration, next: Duration }

impl Repeater {
    pub fn new(timing: RepeatTiming) -> Self { Self { timing, held: None } }

    /// Feeds a device event. Returns the press it makes, if any (releases make none).
    pub fn on_event(&mut self, e: &ButtonEvent) -> Option<Press> {
        if !e.pressed {
            if self.held.as_ref().is_some_and(|h| h.button == e.button) { self.held = None; }
            return None;
        }
        let first_repeat = e.time + self.timing.delay;
        self.held = e.button.repeats().then_some(Held { button: e.button, first_repeat, next: first_repeat });
        Some(Press { button: e.button, repeat: false })
    }

    /// The repeat due at `now`, if any. Repeats missed by more than one interval (the caller was
    /// late) are skipped rather than delivered in a burst.
    pub fn poll(&mut self, now: Duration) -> Option<Press> {
        let t = self.timing;
        let h = self.held.as_mut()?;
        if now < h.next { return None; }
        let interval = if h.next - h.first_repeat >= t.speed_up_after { t.fast_interval } else { t.interval };
        h.next = if h.next + interval > now { h.next + interval } else { now + interval };
        Some(Press { button: h.button, repeat: true })
    }

    /// When the next repeat is due, for the caller's timer.
    pub fn next_deadline(&self) -> Option<Duration> { self.held.as_ref().map(|h| h.next) }

    /// Forgets the held button, e.g. after waking from sleep or a screen change.
    pub fn reset(&mut self) { self.held = None; }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration { Duration::from_millis(n) }
    fn ev(button: Button, pressed: bool, t: u64) -> ButtonEvent { ButtonEvent { button, pressed, time: ms(t) } }

    /// Polls every millisecond from `from` to `to`; returns the times repeats came.
    fn repeats(r: &mut Repeater, from: u64, to: u64) -> Vec<u64> {
        (from..=to).filter(|&t| r.poll(ms(t)).is_some()).collect()
    }

    #[test]
    fn press_then_delay_then_interval() {
        let mut r = Repeater::default();
        assert_eq!(r.on_event(&ev(Button::Down, true, 0)), Some(Press { button: Button::Down, repeat: false }));
        assert_eq!(repeats(&mut r, 0, 500), vec![350, 420, 490]);
    }

    #[test]
    fn speeds_up_after_holding() {
        let mut r = Repeater::default();
        r.on_event(&ev(Button::Down, true, 0));
        let times = repeats(&mut r, 0, 2100);
        let gaps: Vec<u64> = times.windows(2).map(|w| w[1] - w[0]).collect();
        assert_eq!(gaps.first(), Some(&70));
        assert_eq!(gaps.last(), Some(&33));
        // the switch happens once 1.5 s of repeating has passed
        let switch = times.windows(2).find(|w| w[1] - w[0] == 33).unwrap()[0];
        assert!(switch >= 350 + 1500 && switch < 350 + 1500 + 70, "{switch}");
    }

    #[test]
    fn release_stops_and_other_press_takes_over() {
        let mut r = Repeater::default();
        r.on_event(&ev(Button::Down, true, 0));
        r.on_event(&ev(Button::Up, true, 100));
        r.on_event(&ev(Button::Down, false, 120)); // releasing the old button changes nothing
        assert_eq!(r.poll(ms(450)), Some(Press { button: Button::Up, repeat: true }));
        r.on_event(&ev(Button::Up, false, 460));
        assert_eq!(r.poll(ms(1000)), None);
        assert_eq!(r.next_deadline(), None);
    }

    #[test]
    fn non_repeating_press_stops_repeat() {
        let mut r = Repeater::default();
        r.on_event(&ev(Button::Down, true, 0));
        assert_eq!(r.on_event(&ev(Button::A, true, 200)), Some(Press { button: Button::A, repeat: false }));
        assert_eq!(repeats(&mut r, 200, 1000), Vec::<u64>::new());
    }

    #[test]
    fn late_caller_gets_no_burst() {
        let mut r = Repeater::default();
        r.on_event(&ev(Button::Down, true, 0));
        assert!(r.poll(ms(1000)).is_some()); // many intervals late: one repeat…
        assert_eq!(r.poll(ms(1001)), None); // …then the rhythm restarts from now
        assert_eq!(r.next_deadline(), Some(ms(1070)));
    }

    #[test]
    fn reset_forgets() {
        let mut r = Repeater::default();
        r.on_event(&ev(Button::Left, true, 0));
        r.reset();
        assert_eq!(r.poll(ms(400)), None);
    }
}
