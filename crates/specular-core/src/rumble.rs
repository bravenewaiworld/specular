//! What the rumble motor should do, handed from the threads that decide it (the game's, for the
//! game's own rumble; the UI's, for effects) to the thread that runs it (`device::Rumble::set` can
//! block). Setting never waits for the motor, and only the latest state is kept: the rumble
//! thread, when it falls behind, skips straight to it.

use std::sync::{Arc, Condvar, Mutex};

use crate::device::Motors;
use crate::settings::builtin::RUMBLE_MAX;

/// What the rumble should do (the game's part, the effects' part and the strength), set from any
/// thread and waited on by the rumble thread. Clones share it.
#[derive(Clone, Default)]
pub struct RumbleLine(Arc<(Mutex<Wanted>, Condvar)>);

#[derive(Default)]
struct Wanted { game: Motors, effects: Motors, level: u8 }

impl Wanted {
    /// The stronger of the game's and the effects' rumble, motor by motor, scaled by the level.
    fn motors(&self) -> Motors {
        let m = self.game.max(self.effects);
        let scale = |v: u16| (v as u32 * self.level.min(RUMBLE_MAX as u8) as u32 / RUMBLE_MAX as u32) as u16;
        Motors { strong: scale(m.strong), weak: scale(m.weak) }
    }
}

impl RumbleLine {
    /// The running game's own rumble (`Motors::STILL` while it is paused or ended).
    pub fn set_game(&self, m: Motors) { self.change(|w| w.game = m); }
    /// The effects' rumble (`effects::Frame::rumble`).
    pub fn set_effects(&self, m: Motors) { self.change(|w| w.effects = m); }
    /// The "Rumble" setting's strength, 0 (off) to `RUMBLE_MAX` (as asked). 0 until set.
    pub fn set_level(&self, level: u8) { self.change(|w| w.level = level); }

    /// What the motors should do now.
    pub fn wanted(&self) -> Motors { self.0.0.lock().unwrap().motors() }

    /// Waits until what the motors should do differs from `last`, and returns it.
    pub fn next(&self, last: Motors) -> Motors {
        let (wanted, changed) = &*self.0;
        changed.wait_while(wanted.lock().unwrap(), |w| w.motors() == last).unwrap().motors()
    }

    fn change(&self, f: impl FnOnce(&mut Wanted)) {
        let (wanted, changed) = &*self.0;
        let mut w = wanted.lock().unwrap();
        let before = w.motors();
        f(&mut w);
        if w.motors() != before { changed.notify_all(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: Motors = Motors { strong: 0xffff, weak: 0xffff };

    #[test]
    fn the_stronger_request_wins_scaled_by_the_level() {
        let line = RumbleLine::default();
        line.set_game(Motors { strong: 0x8000, weak: 0 });
        assert_eq!(line.wanted(), Motors::STILL); // the level is 0 until the setting comes
        line.set_level(RUMBLE_MAX as u8);
        line.set_effects(Motors { strong: 0x4000, weak: 0x2000 });
        assert_eq!(line.wanted(), Motors { strong: 0x8000, weak: 0x2000 });
        line.set_level(1);
        assert_eq!(line.wanted(), Motors { strong: 0x8000 / RUMBLE_MAX as u16, weak: 0x2000 / RUMBLE_MAX as u16 });
        line.set_level(0);
        assert_eq!(line.wanted(), Motors::STILL);
    }

    #[test]
    fn the_motors_thread_skips_to_the_latest_state() {
        let line = RumbleLine::default();
        line.set_level(RUMBLE_MAX as u8);
        // Several changes before the motors' thread looks: it sees only the last.
        line.set_game(Motors { strong: 1, weak: 1 });
        line.set_game(FULL);
        assert_eq!(line.next(Motors::STILL), FULL);
        // It waits for a change, from another thread.
        let other = line.clone();
        let waiting = std::thread::spawn(move || other.next(FULL));
        std::thread::sleep(std::time::Duration::from_millis(20));
        line.set_game(FULL); // no change: still waiting
        line.set_game(Motors::STILL);
        assert_eq!(waiting.join().unwrap(), Motors::STILL);
    }
}
