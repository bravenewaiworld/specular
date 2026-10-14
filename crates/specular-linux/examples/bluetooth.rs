//! Tries Bluetooth on a device: runs BlueZ commands and plays a tone on a headset, printing the
//! wireless trace (on stderr). Steps run in order:
//!
//! ```text
//! bluetooth power:on search:on wait:10 search:off pair:AA:BB:CC:DD:EE:FF wait:20 \
//!           volume:AA:BB:CC:DD:EE:FF:10 tone:AA:BB:CC:DD:EE:FF:5 disconnect:AA:BB:CC:DD:EE:FF power:off
//! ```
//!
//! `wait:N` waits N seconds (events keep arriving); `tone:ADDRESS:N` plays 440 Hz for N seconds,
//! keeping the queue half full as games do, and prints how full it was and how many underruns
//! there were.

use std::time::{Duration, Instant};

use specular_core::device::SoundOut;
use specular_core::wireless::Wireless;
use specular_core::wireless::bluetooth::{BtAddress, BtCmd};
use specular_core::wireless::trace::Traced;
use specular_linux::bluez::Bluez;
use specular_linux::sound::{Alsa, set_bluetooth_volume};

fn main() {
    let mut bt = Traced::new("bluetooth", Bluez::new());
    bt.start(Box::new(|_| {}));
    std::thread::sleep(Duration::from_secs(1));
    for step in std::env::args().skip(1) {
        let (what, arg) = step.split_once(':').unwrap_or((&step, ""));
        // An address followed by a number: "AA:BB:CC:DD:EE:FF:5".
        let addr_n = || { let (a, n) = arg.rsplit_once(':').expect("ADDRESS:N"); (BtAddress(a.into()), n.parse::<u64>().expect("a number")) };
        match what {
            "power" => bt.send(BtCmd::Power(arg == "on")),
            "search" => bt.send(BtCmd::Search(arg == "on")),
            "pair" => bt.send(BtCmd::Pair(BtAddress(arg.into()))),
            "connect" => bt.send(BtCmd::Connect(BtAddress(arg.into()))),
            "disconnect" => bt.send(BtCmd::Disconnect(BtAddress(arg.into()))),
            "forget" => bt.send(BtCmd::Forget(BtAddress(arg.into()))),
            "wait" => std::thread::sleep(Duration::from_secs(arg.parse().expect("seconds"))),
            "volume" => { let (a, n) = addr_n(); println!("volume {n}: {:?}", set_bluetooth_volume(&a, n as u8)); }
            "tone" => { let (a, n) = addr_n(); tone(&a, Duration::from_secs(n)); }
            _ => eprintln!("unknown step {step}"),
        }
    }
    std::thread::sleep(Duration::from_secs(1));
}

fn tone(address: &BtAddress, length: Duration) {
    let t = Instant::now();
    let mut out = match Alsa::bluetooth(address) {
        Ok(o) => o,
        Err(e) => return println!("tone: {e}"),
    };
    println!("tone: opened in {:?}, rate {}, capacity {} frames", t.elapsed(), out.rate(), out.capacity());
    let (rate, target) = (out.rate() as f64, out.capacity() / 2);
    let (mut phase, mut underruns, mut lowest, mut highest) = (0.0f64, 0, usize::MAX, 0);
    let start = Instant::now();
    // About one frame of a 60 Hz game at a time, as games write.
    while start.elapsed() < length {
        let queued = out.queued();
        if queued == 0 && start.elapsed() > Duration::from_millis(200) { underruns += 1; }
        (lowest, highest) = (lowest.min(queued), highest.max(queued));
        let n = target.saturating_sub(queued).max(800);
        let frames: Vec<[i16; 2]> = (0..n).map(|_| {
            phase += 440.0 / rate;
            let v = ((phase * std::f64::consts::TAU).sin() * 8000.0) as i16;
            [v, v]
        }).collect();
        if let Err(e) = out.write(&frames) { return println!("tone: write failed after {:?}: {e}", start.elapsed()); }
        std::thread::sleep(Duration::from_micros(16_600));
    }
    println!("tone: queue {lowest}..{highest} of {} frames, {underruns} underruns", out.capacity());
}
