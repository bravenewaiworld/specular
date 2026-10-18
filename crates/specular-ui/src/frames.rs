//! Developer statistics on the UI's animation frames (the "Frame statistics" setting): whether
//! each display refresh got exactly one new frame while something animated.

use std::time::Duration;

/// Times between frames drawn back to back while an animation runs, against the refresh period.
/// Only those are counted: a frame drawn after idle time, or only for a game's new frame, isn't
/// due at a particular refresh.
pub struct FrameTimes {
    period: Duration,
    /// When the last frame was drawn, if an animation was still running after it.
    last: Option<Duration>,
    /// The frame times of the animation in progress.
    run: Vec<Duration>,
    total: Counts,
}

/// Frame times counted, for one animation or since the statistics started.
#[derive(Default)]
struct Counts {
    times: u32,
    /// More than 1.5 refreshes after the frame before: a refresh showed that frame again.
    late: u32,
    /// Less than half a refresh after the frame before: two frames for one refresh, so one of
    /// them was never shown (frames paced by something other than the display).
    extra: u32,
    longest: Duration,
}

impl Counts {
    fn add(&mut self, t: Duration, period: Duration) {
        self.times += 1;
        self.late += (t > period * 3 / 2) as u32;
        self.extra += (t < period / 2) as u32;
        self.longest = self.longest.max(t);
    }
}

fn ms(d: Duration) -> f64 { d.as_secs_f64() * 1e3 }

impl FrameTimes {
    pub fn new(refresh_hz: f64) -> Self {
        Self { period: Duration::from_secs_f64(1.0 / refresh_hz), last: None, run: Vec::new(), total: Counts::default() }
    }

    /// A frame was drawn at `t`; `animating` whether an animation runs on after it. Returns a
    /// line on the animation that this frame ended, if it had frames drawn back to back.
    pub fn drawn(&mut self, t: Duration, animating: bool) -> Option<String> {
        if let Some(last) = self.last {
            let dt = t.saturating_sub(last);
            self.run.push(dt);
            self.total.add(dt, self.period);
        }
        self.last = animating.then_some(t);
        if animating || self.run.is_empty() { return None; }
        let mut run = std::mem::take(&mut self.run);
        run.sort();
        let mut c = Counts::default();
        run.iter().for_each(|&t| c.add(t, self.period));
        Some(format!("{} frame times: median {:.1} ms, longest {:.1} ms; {} late, {} extra",
            c.times, ms(run[run.len() / 2]), ms(c.longest), c.late, c.extra))
    }

    /// The totals since these statistics started, for the screen.
    pub fn summary(&self) -> String {
        let c = &self.total;
        format!("animation: {} frame times · {} late · {} extra · longest {:.1} ms", c.times, c.late, c.extra, ms(c.longest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: f64) -> Duration { Duration::from_secs_f64(ms / 1e3) }

    #[test]
    fn counts_late_and_extra_frames_only_while_animating() {
        let mut f = FrameTimes::new(60.0);
        // A frame after idle time, then an animation: on time, one refresh missed, two frames
        // in one refresh, and its last frame.
        assert_eq!(f.drawn(at(0.0), false), None);
        assert_eq!(f.drawn(at(1000.0), true), None);
        assert_eq!(f.drawn(at(1016.7), true), None);
        assert_eq!(f.drawn(at(1050.0), true), None);
        assert_eq!(f.drawn(at(1052.0), true), None);
        let line = f.drawn(at(1068.7), false).unwrap();
        assert_eq!(line, "4 frame times: median 16.7 ms, longest 33.3 ms; 1 late, 1 extra");
        // Idle again: the next frame's time since the last isn't counted.
        assert_eq!(f.drawn(at(5000.0), false), None);
        assert_eq!(f.summary(), "animation: 4 frame times · 1 late · 1 extra · longest 33.3 ms");
    }
}
