//! A game's sound, matched to the output. The core's sample rate is converted to the output's by
//! linear interpolation, at a ratio that also absorbs two kinds of drift:
//! - the game running at the display's rate rather than its own (Game Boy games run at 59.73 fps;
//!   paced by a 60.2 Hz display they play 0.8 % fast, so they also make sound 0.8 % faster), and
//! - the clocks of the core and the sound card not agreeing exactly: the ratio follows how full
//!   the output's queue is, aiming at half full (what RetroArch calls dynamic rate control), by
//!   at most `MAX_ADJUST` so the pitch change can't be heard.

use specular_core::device::SoundOut;

/// The most the ratio is changed by to keep the queue half full.
const MAX_ADJUST: f64 = 0.005;
/// A game this much faster or slower than the display is paced by a timer instead, at its own
/// rate, with no speed correction (see `game_thread`).
pub const MAX_SPEED_CHANGE: f64 = 0.05;

/// The loudest sample in `samples`, left or right: 0 to 32768.
pub fn peak(samples: &[[i16; 2]]) -> u16 {
    samples.iter().map(|[l, r]| l.unsigned_abs().max(r.unsigned_abs())).max().unwrap_or(0)
}

/// Linear interpolation between consecutive stereo frames, keeping its position across calls.
#[derive(Default)]
struct Resampler {
    /// Where the next output frame falls, in input frames from the start of the next input; -1
    /// is `last`, the final frame of the previous input.
    position: f64,
    last: [i16; 2],
}

impl Resampler {
    /// Appends `input` resampled to `out`, making `ratio` output frames per input frame.
    fn process(&mut self, input: &[[i16; 2]], ratio: f64, out: &mut Vec<[i16; 2]>) {
        if input.is_empty() { return; }
        let step = 1.0 / ratio;
        let at = |i: isize| if i < 0 { self.last } else { input[i as usize] };
        let mut p = self.position;
        while p < (input.len() - 1) as f64 {
            let i = p.floor();
            let (a, b, f) = (at(i as isize), at(i as isize + 1), p - i);
            out.push([0, 1].map(|c| (a[c] as f64 + (b[c] as f64 - a[c] as f64) * f).round() as i16));
            p += step;
        }
        self.position = p - input.len() as f64;
        self.last = input[input.len() - 1];
    }
}

/// Plays a game's samples on a `SoundOut`.
pub struct Sound {
    out: Option<Box<dyn SoundOut>>,
    resampler: Resampler,
    /// The rate the game's samples are made at, as paced (its own rate, scaled by the speed-up).
    in_rate: f64,
    buf: Vec<[i16; 2]>,
}

impl Sound {
    /// `sample_rate` is the core's; `speed` is how much faster than its own rate the game runs
    /// (display rate / game frame rate, or 1). Without an output, samples are dropped.
    pub fn new(out: Option<Box<dyn SoundOut>>, sample_rate: f64, speed: f64) -> Self {
        Self { out, resampler: Resampler::default(), in_rate: sample_rate * speed, buf: Vec::new() }
    }

    /// Replaces (and closes) the output. A fresh output reports nothing queued, so the next `play`
    /// first fills it with silence, as after an underrun.
    pub fn set_out(&mut self, out: Option<Box<dyn SoundOut>>) { self.out = out; }

    /// Queues one frame's samples. Returns how much sound was queued before them (None without
    /// an output).
    pub fn play(&mut self, samples: &[[i16; 2]]) -> Option<std::time::Duration> {
        let out = self.out.as_mut()?;
        let (queued, target) = (out.queued(), out.capacity() / 2);
        let queued_time = std::time::Duration::from_secs_f64(queued as f64 / out.rate() as f64);
        self.buf.clear();
        if queued == 0 {
            // Starting, on a new output, or after an underrun (the game was paused or slow): fill
            // half the queue with silence, so jitter doesn't starve it again at once.
            self.buf.resize(target, [0, 0]);
        }
        let fill = (target as f64 - queued as f64) / target.max(1) as f64;
        let ratio = out.rate() as f64 * (1.0 + MAX_ADJUST * fill.clamp(-1.0, 1.0)) / self.in_rate;
        self.resampler.process(samples, ratio, &mut self.buf);
        // A full queue drops the rest: better than waiting, which would delay the next frame.
        if let Err(e) = out.write(&self.buf) {
            eprintln!("sound: {e}; no sound until the output changes");
            self.out = None;
        }
        Some(queued_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_rate_across_calls() {
        let mut r = Resampler::default();
        let mut out = Vec::new();
        let input: Vec<[i16; 2]> = (0..1000).map(|i| [i as i16, -(i as i16)]).collect();
        for chunk in input.chunks(97) { r.process(chunk, 0.5, &mut out); }
        // Half as many frames: every second input frame, from the first, across chunk boundaries.
        assert!((out.len() as i32 - 500).abs() <= 1, "{}", out.len());
        assert_eq!(out[60], [120, -120]);
        let mut up = Vec::new();
        Resampler::default().process(&input[..100], 2.0, &mut up);
        assert!((up.len() as i32 - 200).abs() <= 2);
        assert_eq!(up[5], [3, -3]); // halfway between frames 2 and 3, rounded up
    }

    #[test]
    fn the_peak_is_the_loudest_sample_either_side() {
        assert_eq!(peak(&[[10, -300], [200, 5]]), 300);
        assert_eq!(peak(&[[i16::MIN, 0]]), 32768);
        assert_eq!(peak(&[]), 0);
    }

    struct Queue { queued: usize, written: Vec<[i16; 2]> }
    impl SoundOut for Queue {
        fn rate(&self) -> u32 { 48_000 }
        fn write(&mut self, f: &[[i16; 2]]) -> std::io::Result<usize> { self.written.extend_from_slice(f); self.queued += f.len(); Ok(f.len()) }
        fn queued(&mut self) -> usize { self.queued }
        fn capacity(&self) -> usize { 3072 }
    }

    #[test]
    fn primes_with_silence_and_slows_down_when_full() {
        let mut s = Sound::new(Some(Box::new(Queue { queued: 0, written: Vec::new() })), 48_000.0, 1.0);
        s.play(&[[100, 100]; 800]);
        let out = s.out.as_mut().unwrap();
        let first = out.queued();
        assert!(first >= 1536 + 790, "{first}"); // half the queue of silence, then the samples
        // A queue past half full gets fewer frames than it was given.
        let before = out.queued();
        s.play(&[[100, 100]; 800]);
        let added = s.out.as_mut().unwrap().queued() - before;
        assert!(added < 800, "{added}");
    }

    #[test]
    fn a_new_output_is_primed_again() {
        let mut s = Sound::new(Some(Box::new(Queue { queued: 2000, written: Vec::new() })), 48_000.0, 1.0);
        s.set_out(Some(Box::new(Queue { queued: 0, written: Vec::new() })));
        s.play(&[[100, 100]; 800]);
        // Half the queue (3072 frames) of silence, plus about 800 resampled samples.
        assert!(s.out.as_mut().unwrap().queued() >= 1536 + 790);
    }
}
