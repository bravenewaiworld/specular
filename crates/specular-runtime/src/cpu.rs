//! The CPU's minimum speed while the game plays: chosen from its frames by the `CpuMin` in `Outputs::cpu_min`,
//! set through `Outputs::set_cpu_min`. The system's default while the game is paused, and once it
//! has ended.

use std::time::Duration;

use specular_core::cpu::{CpuMin, Decision, text};

/// How many frames after a save state is made can't miss a refresh, as far as the CPU minimum is
/// told. Making the state on the game's thread delays the next frame, and compressing it on the
/// writer's can slow the frames after it (SwanStation's 11.5 MB: a miss 7–8 frames later on the
/// Brick; docs/runtime.md, Findings). Those misses aren't for want of speed. Half a second covers
/// them; a miss that is, within it, goes unseen.
const AFTER_STATE: u32 = 30;

/// Feeds the game's frames to its `CpuMin` and sets what it decides, logging each change.
pub(crate) struct CpuSpeed {
    min: CpuMin,
    set: Box<dyn Fn(Option<u32>) + Send>,
    /// The minimum last set (None: the system's default, as before the game started).
    applied: Option<u32>,
    /// How many more frames can't miss (see `AFTER_STATE`).
    after_state: u32,
}

impl CpuSpeed {
    /// Sets `min`'s start level.
    pub(crate) fn start(min: CpuMin, set: Box<dyn Fn(Option<u32>) + Send>) -> Self {
        let mut s = Self { min, set, applied: None, after_state: 0 };
        s.apply(s.min.current(), "as the game starts");
        s
    }

    /// The minimum while the game plays (MHz; None: the system's default).
    pub(crate) fn level(&self) -> Option<u32> { self.min.current() }

    /// Hands one frame to `CpuMin::frame`. The caller takes `gap` from `Pacer::wait` and skips the
    /// frames it gives none for. Within `AFTER_STATE` frames of `state_made`, a long gap counts
    /// as one refresh: it still adds to `CpuMin`'s clock, but isn't a miss.
    pub(crate) fn frame(&mut self, mut gap: Duration, period: Duration) {
        if self.after_state > 0 {
            self.after_state -= 1;
            gap = gap.min(period);
        }
        match self.min.frame(gap, period) {
            Some(Decision::Set(mhz, why)) => self.apply(Some(mhz), &why),
            Some(Decision::Note(note)) => eprintln!("cpu: {note}"),
            None => {}
        }
    }

    /// A save state was made (on the game's thread) since the last frame.
    pub(crate) fn state_made(&mut self) { self.after_state = AFTER_STATE; }

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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn frames_just_after_a_state_is_made_dont_miss() {
        let period = Duration::from_micros(16_667);
        let set = Arc::new(Mutex::new(Vec::new()));
        let s = set.clone();
        let mut cpu = CpuSpeed::start(CpuMin::new(vec![1800, 2000], Some(1800)), Box::new(move |mhz| s.lock().unwrap().push(mhz)));
        cpu.state_made();
        // Making the state, then compressing it, made two frames late.
        cpu.frame(period * 2, period);
        for _ in 0..7 { cpu.frame(period, period); }
        cpu.frame(period * 2, period);
        for _ in 0..AFTER_STATE { cpu.frame(period, period); }
        assert_eq!(cpu.level(), Some(1800));
        // Two misses afterwards still raise it.
        cpu.frame(period * 2, period);
        cpu.frame(period * 2, period);
        assert_eq!(cpu.level(), Some(2000));
        drop(cpu);
        assert_eq!(*set.lock().unwrap(), [Some(1800), Some(2000), None]);
    }
}
