//! The CPU's minimum speed while the game plays: chosen from its frames by `Outputs::cpu_min`,
//! set through `Outputs::set_cpu_min`. The system's default while the game is paused, and once it
//! has ended.

use std::time::Duration;

use specular_core::cpu::{CpuMin, Decision, text};

/// Feeds the game's frames to its `CpuMin` and sets what it decides, logging each change.
pub(crate) struct CpuSpeed {
    min: CpuMin,
    set: Box<dyn Fn(Option<u32>) + Send>,
    /// The minimum last set (None: the system's default, as before the game started).
    applied: Option<u32>,
    /// `CpuMin`'s clock: the time of play, the gaps between the vsync's returns added up.
    played: Duration,
}

impl CpuSpeed {
    /// Sets `min`'s start level.
    pub(crate) fn start(min: CpuMin, set: Box<dyn Fn(Option<u32>) + Send>) -> Self {
        let mut s = Self { min, set, applied: None, played: Duration::ZERO };
        s.apply(s.min.current(), "as the game starts");
        s
    }

    /// The minimum while the game plays (MHz; None: the system's default).
    pub(crate) fn level(&self) -> Option<u32> { self.min.current() }

    /// A frame shown `gap` after the one before (by the vsync: `Pacer::wait`), the core having run
    /// for `run`. A gap of more than 1.5 refreshes is a missed refresh. Returns whether the
    /// minimum changed (the frame delay is then measured again: runs take another time).
    pub(crate) fn frame(&mut self, gap: Duration, run: Duration, period: Duration) -> bool {
        self.played += gap;
        match self.min.frame(self.played, gap > period * 3 / 2, run, period) {
            Some(Decision::Set(mhz, why)) => { self.apply(mhz, &why); true }
            Some(Decision::Note(note)) => { eprintln!("cpu: {note}"); false }
            None => false,
        }
    }

    /// The game paused (its menu, sleep) or resumed: the system's default meanwhile.
    pub(crate) fn pause(&mut self, paused: bool) {
        if paused { self.apply(None, "while the game is paused") } else { self.apply(self.min.current(), "as the game resumes") }
    }

    fn apply(&mut self, mhz: Option<u32>, why: &str) {
        if mhz == self.applied { return; }
        self.applied = mhz;
        eprintln!("cpu: minimum {} ({why})", text(mhz));
        (self.set)(mhz);
    }
}

impl Drop for CpuSpeed {
    fn drop(&mut self) { self.apply(None, "the game has ended"); }
}
