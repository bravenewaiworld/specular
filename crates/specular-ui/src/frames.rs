//! Developer statistics on the UI's animation frames (the "Frame statistics" setting): whether
//! each display refresh got exactly one new frame while something animated, and what the frames
//! cost the UI thread.

use std::time::Duration;

/// The running count of row slots changed, and the row slots, read as each frame is drawn. An
/// animation's line says how much the count grew from the frame before it started to its last.
#[derive(Clone, Copy, Default)]
pub struct Work {
    /// Row slots given a new row, or a change to theirs.
    pub slots_changed: u64,
    /// The row slots, as of the last `Runner::refresh`: how many there are, how many hold a row
    /// (the rows near the view), and how many of those are on screen.
    pub slots: (usize, usize, usize),
}

/// Times between frames drawn back to back while an animation runs, against the refresh period,
/// and what those frames cost. Only those are counted: a frame drawn after idle time, or only for
/// a game's new frame, isn't due at a particular refresh.
pub struct FrameTimes {
    period: Duration,
    /// When the last frame was drawn, and the UI thread's CPU time then, if an animation was
    /// still running after it.
    last: Option<(Duration, Duration)>,
    /// The UI thread's CPU time as the frame being drawn started.
    started: Option<Duration>,
    /// The frames of the animation in progress.
    run: Vec<Frame>,
    /// The work totals at the frame before the animation in progress.
    start: Work,
    total: Counts,
    /// The last animation's median CPU time per frame, and the part of it spent drawing.
    last_cpu: Option<(Duration, Duration)>,
}

/// One frame of an animation: the time since the frame before, the UI thread's CPU time since
/// then, and the part of it spent drawing this frame.
struct Frame { time: Duration, cpu: Duration, drawing: Duration }

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

/// The median and the longest of `f` over `run` (not empty).
fn median_longest(run: &[Frame], f: impl Fn(&Frame) -> Duration) -> (Duration, Duration) {
    let mut v: Vec<Duration> = run.iter().map(f).collect();
    v.sort();
    (v[v.len() / 2], v[v.len() - 1])
}

impl FrameTimes {
    pub fn new(refresh_hz: f64) -> Self {
        Self {
            period: Duration::from_secs_f64(1.0 / refresh_hz), last: None, started: None, run: Vec::new(),
            start: Work::default(), total: Counts::default(), last_cpu: None,
        }
    }

    /// A frame is about to be drawn; `cpu` is the UI thread's CPU time.
    pub fn drawing(&mut self, cpu: Duration) { self.started = Some(cpu); }

    /// A frame was drawn at `t`, with the UI thread's CPU time at `cpu`, and the work totals at
    /// `work`; `animating` whether an animation runs on after it. Returns the lines on the
    /// animation that this frame ended, if it had frames drawn back to back.
    pub fn drawn(&mut self, t: Duration, cpu: Duration, animating: bool, work: Work) -> Option<[String; 2]> {
        let drawing = cpu.saturating_sub(self.started.take().unwrap_or(cpu));
        if let Some((last, last_cpu)) = self.last {
            let time = t.saturating_sub(last);
            self.run.push(Frame { time, cpu: cpu.saturating_sub(last_cpu), drawing });
            self.total.add(time, self.period);
        }
        self.last = animating.then_some((t, cpu));
        if animating { return None; }
        let start = std::mem::replace(&mut self.start, work);
        if self.run.is_empty() { return None; }
        let run = std::mem::take(&mut self.run);
        let mut c = Counts::default();
        run.iter().for_each(|f| c.add(f.time, self.period));
        let (time, longest_time) = median_longest(&run, |f| f.time);
        let (cpu, longest_cpu) = median_longest(&run, |f| f.cpu);
        let (drawing, longest_drawing) = median_longest(&run, |f| f.drawing);
        self.last_cpu = Some((cpu, drawing));
        let (slots, in_use, on_screen) = work.slots;
        Some([
            format!("{} frame times: median {:.1} ms, longest {:.1} ms; {} late, {} extra",
                c.times, ms(time), ms(longest_time), c.late, c.extra),
            format!("CPU a frame: median {:.1} ms, {:.1} ms of it drawing (longest {:.1}, {:.1}); {} row slots changed; {slots} slots, {in_use} with rows, {on_screen} on screen",
                ms(cpu), ms(drawing), ms(longest_cpu), ms(longest_drawing),
                work.slots_changed - start.slots_changed),
        ])
    }

    /// The totals since these statistics started, and the last animation's CPU time, for the
    /// screen.
    pub fn summary(&self) -> String {
        let c = &self.total;
        let cpu = self.last_cpu.map_or(String::new(), |(cpu, drawing)| format!("\nlast animation: CPU {:.1} ms a frame, {:.1} ms drawing", ms(cpu), ms(drawing)));
        format!("UI: {} frame times · {} late · {} extra · longest {:.1} ms{cpu}", c.times, c.late, c.extra, ms(c.longest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: f64) -> Duration { Duration::from_secs_f64(ms / 1e3) }

    #[test]
    fn counts_late_and_extra_frames_only_while_animating() {
        let mut f = FrameTimes::new(60.0);
        let work = |slots_changed| Work { slots_changed, slots: (23, 17, 7) };
        // The CPU clock keeps pace with the wall clock (so the CPU time per frame is the frame
        // time); 2 ms of each frame is drawing.
        let mut frame = |t: f64, animating, slots_changed| {
            f.drawing(at(t + 1.0));
            f.drawn(at(t), at(t + 3.0), animating, work(slots_changed))
        };
        // A frame after idle time, then an animation: on time, one refresh missed, two frames
        // in one refresh, and its last frame.
        assert_eq!(frame(0.0, false, 1), None);
        assert_eq!(frame(1000.0, true, 2), None);
        assert_eq!(frame(1016.7, true, 2), None);
        assert_eq!(frame(1050.0, true, 3), None);
        assert_eq!(frame(1052.0, true, 3), None);
        let lines = frame(1068.7, false, 4).unwrap();
        assert_eq!(lines[0], "4 frame times: median 16.7 ms, longest 33.3 ms; 1 late, 1 extra");
        // The row slots changed from the end of the frame before the animation to its last.
        assert_eq!(lines[1], "CPU a frame: median 16.7 ms, 2.0 ms of it drawing (longest 33.3, 2.0); 3 row slots changed; 23 slots, 17 with rows, 7 on screen");
        // Idle again: the next frame's time since the last isn't counted.
        assert_eq!(frame(5000.0, false, 4), None);
        assert_eq!(f.summary(), "UI: 4 frame times · 1 late · 1 extra · longest 33.3 ms\nlast animation: CPU 16.7 ms a frame, 2.0 ms drawing");
    }
}
