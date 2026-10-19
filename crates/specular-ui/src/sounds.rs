//! The interface's sounds (`Effect::Sound`): short sine tones made in code from the design's
//! parameters (docs/look-and-motion.md), played on a thread of their own, as opening an output can
//! block. The output opens at the first sound and closes after `IDLE_CLOSE` without one (an open
//! output keeps the sound hardware, or a Bluetooth link's stream, powered), and before a game
//! opens its own (`Sounds::close`): an ALSA device that doesn't mix can't be opened twice. A sound
//! that can't be played is dropped.

use std::collections::HashMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use specular_core::app::Sound;
use specular_core::device::{Output, SoundOut};

use crate::game::SharedDevice;

/// The output is closed after this long without a sound.
const IDLE_CLOSE: Duration = Duration::from_secs(2);
/// A sound not started within this long of being asked for (the output was slow to open, say a
/// Bluetooth one) is dropped: late, it would no longer match what it is for.
const STALE: Duration = Duration::from_millis(100);
/// After the output fails to open or play, sounds are dropped for this long rather than each
/// trying again. A change of output cuts it short.
const RETRY_AFTER: Duration = Duration::from_secs(2);
/// The longest a sound's writes, or waiting for the queue to play out before closing, may take.
const WRITE_LIMIT: Duration = Duration::from_millis(250);

/// Every tone is the mockup's tick, at its own pitch: its gain rises exponentially from `FLOOR`
/// to `PEAK` (−30 dBFS) over `ATTACK_MS`, then falls back to `FLOOR` by `TONE_MS`, where it
/// stops (an exponential ramp can't reach 0).
const FLOOR: f32 = 0.0001;
const PEAK: f32 = 0.03;
const ATTACK_MS: f32 = 2.0;
const TONE_MS: f32 = 25.0;

/// Each sound's tones: (pitch in Hz, start in ms from the sound's start). The tick is 1250 Hz;
/// opening rises a fifth and going back falls one; a switch turning on is a fifth above the tick,
/// off a fifth below.
fn tones(sound: Sound) -> &'static [(f32, f32)] {
    match sound {
        Sound::Move => &[(1250.0, 0.0)],
        Sound::Open => &[(1250.0, 0.0), (1875.0, 40.0)],
        Sound::Back => &[(1875.0, 0.0), (1250.0, 40.0)],
        Sound::Toggle(true) => &[(1875.0, 0.0)],
        Sound::Toggle(false) => &[(833.0, 0.0)],
    }
}

/// `sound` as 16-bit stereo frames at `rate` frames a second.
fn render(sound: Sound, rate: u32) -> Vec<[i16; 2]> {
    let tones = tones(sound);
    let ms_per_frame = 1000.0 / rate as f32;
    let len = tones.iter().map(|&(_, at)| at + TONE_MS).fold(0.0, f32::max);
    (0..(len / ms_per_frame).ceil() as usize).map(|i| {
        let v: f32 = tones.iter().map(|&(hz, at)| {
            let ms = i as f32 * ms_per_frame - at;
            if !(0.0..TONE_MS).contains(&ms) { return 0.0; }
            let gain = if ms < ATTACK_MS { FLOOR * (PEAK / FLOOR).powf(ms / ATTACK_MS) }
                       else { PEAK * (FLOOR / PEAK).powf((ms - ATTACK_MS) / (TONE_MS - ATTACK_MS)) };
            gain * (std::f32::consts::TAU * hz * ms / 1000.0).sin()
        }).sum();
        let s = (v.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        [s, s]
    }).collect()
}

enum Cmd {
    /// A sound, and when it was asked for (see `STALE`).
    Play(Sound, Instant),
    Output(Output),
    /// Close the output once what is queued has played, then answer.
    Close(mpsc::Sender<()>),
}

/// The interface sounds' player. Its methods return at once, except `close`.
#[derive(Clone)]
pub struct Sounds(mpsc::Sender<Cmd>);

impl Sounds {
    /// Starts the player's thread, playing on the speaker until `set_output`.
    pub fn start(device: SharedDevice) -> Self {
        let (tx, rx) = mpsc::channel();
        let mut player = Player { device, output: Output::Speaker, open: None, rendered: HashMap::new(), retry_at: None, failure: None };
        let spawned = std::thread::Builder::new().name("sounds".into()).spawn(move || loop {
            // The idle timer runs only while the output is open.
            let cmd = if player.open.is_some() { rx.recv_timeout(IDLE_CLOSE) } else { rx.recv().map_err(|_| RecvTimeoutError::Disconnected) };
            match cmd {
                Ok(cmd) => player.handle(cmd),
                Err(RecvTimeoutError::Timeout) => player.close("idle"),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        });
        if let Err(e) = spawned { eprintln!("sound: can't start the interface sounds' thread: {e}"); }
        Self(tx)
    }

    /// Plays `sound` on the current output; dropped if it can't start within `STALE`, or while the
    /// output is failing (`RETRY_AFTER`).
    pub fn play(&self, sound: Sound) { let _ = self.0.send(Cmd::Play(sound, Instant::now())); }

    /// Sounds go to `output` from now on.
    pub fn set_output(&self, output: Output) { let _ = self.0.send(Cmd::Output(output)); }

    /// Closes the output, so a game can open it, once what is queued has played (at most
    /// `WRITE_LIMIT`), and returns then. Blocks: never call it on the UI thread. A later `play`
    /// opens it again.
    pub fn close(&self) {
        let (tx, rx) = mpsc::channel();
        if self.0.send(Cmd::Close(tx)).is_ok() { let _ = rx.recv(); }
    }
}

/// The player's thread's state.
struct Player {
    device: SharedDevice,
    output: Output,
    /// `output`, while open.
    open: Option<Box<dyn SoundOut>>,
    /// Each sound made so far, by sound and rate, so playing one is a copy.
    rendered: HashMap<(Sound, u32), Vec<[i16; 2]>>,
    /// Sounds are dropped until then (see `RETRY_AFTER`).
    retry_at: Option<Instant>,
    /// The last failure, which isn't logged again until something else fails or it works again.
    failure: Option<String>,
}

impl Player {
    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play(sound, asked) => self.play(sound, asked),
            Cmd::Output(output) => {
                // The speaker and the jack are one stream (`Output::same_stream`).
                if !self.output.same_stream(&output) { self.close("the output changed"); }
                (self.output, self.retry_at) = (output, None);
            }
            Cmd::Close(done) => {
                if let Some(out) = &mut self.open {
                    let until = Instant::now() + WRITE_LIMIT;
                    while out.queued() > 0 && Instant::now() < until { std::thread::sleep(Duration::from_millis(5)); }
                }
                self.close("a game is starting");
                let _ = done.send(());
            }
        }
    }

    fn play(&mut self, sound: Sound, asked: Instant) {
        if self.open.is_none() { self.open_output(); }
        let Some(out) = &mut self.open else { return };
        // Checked after opening, as opening is what makes a sound late; the output stays open for
        // the next.
        if asked.elapsed() > STALE { return; }
        let frames = self.rendered.entry((sound, out.rate())).or_insert_with(|| render(sound, out.rate()));
        // The output's queue may hold less than a whole sound (ALSA's 64 ms against Open's 65 ms);
        // the rest is written as it plays.
        let (mut rest, until) = (&frames[..], Instant::now() + WRITE_LIMIT);
        while !rest.is_empty() && Instant::now() < until {
            match out.write(rest) {
                Ok(n) => rest = &rest[n..],
                Err(e) => { self.open = None; return self.fail(format!("playing on {:?}: {e}", self.output)); }
            }
            if !rest.is_empty() { std::thread::sleep(Duration::from_millis(5)); }
        }
    }

    /// Opens `self.output`, unless that failed less than `RETRY_AFTER` ago.
    fn open_output(&mut self) {
        if self.retry_at.is_some_and(|t| Instant::now() < t) { return; }
        match self.device.open_sound(&self.output) {
            Ok(out) => {
                eprintln!("sound: interface sounds open on {:?}", self.output);
                (self.failure, self.retry_at, self.open) = (None, None, Some(out));
            }
            Err(e) => self.fail(format!("opening {:?}: {e}", self.output)),
        }
    }

    fn fail(&mut self, why: String) {
        if self.failure.as_ref() != Some(&why) { eprintln!("sound: interface sounds: {why}; sounds are dropped until it works"); }
        self.failure = Some(why);
        self.retry_at = Some(Instant::now() + RETRY_AFTER);
    }

    fn close(&mut self, why: &str) {
        if self.open.take().is_some() { eprintln!("sound: interface sounds closed on {:?} ({why})", self.output); }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use specular_core::device::{Device, DeviceProfile, Hardware, Vsync};
    use specular_core::wireless::bluetooth::BtAddress;

    use super::*;

    #[test]
    fn the_tick_is_the_mockups() {
        let tick = render(Sound::Move, 48_000);
        assert_eq!(tick.len(), 1200); // 25 ms
        let peak = tick.iter().map(|f| f[0].unsigned_abs()).max().unwrap();
        assert!((900..=983).contains(&peak), "{peak}"); // under 0.03 of full scale, the envelope's peak
        assert!(tick.iter().all(|f| f[0] == f[1]));
        // It starts and ends silent, so it doesn't click.
        assert!(tick[..5].iter().chain(&tick[1190..]).all(|f| f[0].abs() <= 5));
        // Opening is two tones, with a gap between them.
        let open = render(Sound::Open, 48_000);
        assert_eq!(open.len(), 3120); // 65 ms
        assert!(open[1200..1920].iter().all(|f| f[0] == 0));
    }

    /// A device whose sound outputs count the frames they are given; `fail` makes them fail to
    /// open.
    #[derive(Default)]
    struct Fake { opened: Mutex<Vec<Output>>, written: Arc<Mutex<usize>>, fail: Mutex<bool> }

    struct FakeOut(Arc<Mutex<usize>>);

    impl SoundOut for FakeOut {
        fn rate(&self) -> u32 { 48_000 }
        fn write(&mut self, frames: &[[i16; 2]]) -> std::io::Result<usize> { *self.0.lock().unwrap() += frames.len(); Ok(frames.len()) }
        fn queued(&mut self) -> usize { 0 }
        fn capacity(&self) -> usize { 3072 }
    }

    impl Device for Fake {
        fn profile(&self) -> DeviceProfile { unimplemented!() }
        fn hardware(&self) -> Hardware { unimplemented!() }
        fn open_sound(&self, output: &Output) -> std::io::Result<Box<dyn SoundOut>> {
            self.opened.lock().unwrap().push(output.clone());
            if *self.fail.lock().unwrap() { return Err(std::io::Error::other("busy")); }
            Ok(Box::new(FakeOut(self.written.clone())))
        }
        fn open_vsync(&self) -> Option<Box<dyn Vsync>> { None }
    }

    fn player() -> (Player, Arc<Fake>) {
        let fake = Arc::new(Fake::default());
        (Player { device: fake.clone(), output: Output::Speaker, open: None, rendered: HashMap::new(), retry_at: None, failure: None }, fake)
    }

    fn play(p: &mut Player, sound: Sound) { p.handle(Cmd::Play(sound, Instant::now())); }

    #[test]
    fn opens_at_the_first_sound_and_closes_for_a_game() {
        let (mut p, fake) = player();
        play(&mut p, Sound::Move);
        play(&mut p, Sound::Move);
        assert_eq!(*fake.opened.lock().unwrap(), [Output::Speaker]);
        assert_eq!(*fake.written.lock().unwrap(), 2400);
        let (tx, rx) = mpsc::channel();
        p.handle(Cmd::Close(tx));
        assert!(p.open.is_none() && rx.try_recv().is_ok());
        // A sound asked for too long ago (the output was slow to open) is dropped.
        p.handle(Cmd::Play(Sound::Move, Instant::now() - 2 * STALE));
        assert_eq!(*fake.written.lock().unwrap(), 2400);
    }

    #[test]
    fn follows_the_output_reopening_only_for_another_stream() {
        let (mut p, fake) = player();
        play(&mut p, Sound::Move);
        p.handle(Cmd::Output(Output::Headphones));
        play(&mut p, Sound::Move);
        let headset = Output::Bluetooth(BtAddress("00:11:22:33:44:55".into()));
        p.handle(Cmd::Output(headset.clone()));
        assert!(p.open.is_none());
        play(&mut p, Sound::Move);
        assert_eq!(*fake.opened.lock().unwrap(), [Output::Speaker, headset]);
    }

    #[test]
    fn an_output_that_fails_to_open_isnt_retried_at_once() {
        let (mut p, fake) = player();
        *fake.fail.lock().unwrap() = true;
        play(&mut p, Sound::Move);
        play(&mut p, Sound::Move);
        assert_eq!(fake.opened.lock().unwrap().len(), 1);
        // Until the output changes.
        p.handle(Cmd::Output(Output::Headphones));
        play(&mut p, Sound::Move);
        assert_eq!(fake.opened.lock().unwrap().len(), 2);
    }
}
