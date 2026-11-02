//! Drives a native core without Specular's UI, to check a core on a device (docs/native-cores.md):
//! starts it, resumes, lets it run, pauses (writing the picture it hands back as a PPM), resumes
//! again, then quits, timing each step. Stop the frontend first: nothing else may draw.
//!
//! Usage: native_drive PROGRAM PACKAGE DIR [SECONDS] [-- CORE ARGS...]
//!   DIR gets saves/, cache/ and pause-N.ppm; SECONDS of play between steps (default 10).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use specular_runtime::FrameSlot;
use specular_runtime::native::{NativeLaunch, NativeOutputs, NativeSession};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let core_args = args.iter().position(|a| a == "--").map(|i| args.split_off(i)[1..].to_vec()).unwrap_or_default();
    if args.len() < 3 { eprintln!("usage: native_drive PROGRAM PACKAGE DIR [SECONDS] [-- CORE ARGS...]"); std::process::exit(2); }
    let dir = PathBuf::from(&args[2]);
    let play = Duration::from_secs_f64(args.get(3).map_or(10.0, |s| s.parse().expect("seconds")));
    let frames = Arc::new(FrameSlot::default());
    let launch = NativeLaunch {
        program: args[0].clone().into(), args: core_args.into_iter().map(Into::into).collect(), package: args[1].clone().into(),
        saves: dir.join("saves"), cache: dir.join("cache"),
        clock: (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()).saturating_sub(946_684_800),
        options: vec![],
    };
    let out = NativeOutputs {
        sound: None, frames: frames.clone(),
        on_save: Box::new(|| eprintln!("drive: the game saved")),
        on_rumble: Box::new(|_| {}),
        on_stop: Box::new(|why| { eprintln!("drive: the core stopped: {why}"); std::process::exit(1) }),
    };
    let t = Instant::now();
    let mut s = NativeSession::start(launch, out).unwrap_or_else(|e| { eprintln!("drive: start: {e}"); std::process::exit(1) });
    eprintln!("drive: started in {:?}: {:?}", t.elapsed(), s.info());
    for round in 1..=2 {
        let t = Instant::now();
        s.resume().expect("resume");
        eprintln!("drive: resumed in {:?}", t.elapsed());
        let f0 = s.frames();
        std::thread::sleep(play);
        eprintln!("drive: {} frames in {:?}", s.frames() - f0, play);
        let t = Instant::now();
        s.pause().expect("pause");
        eprintln!("drive: paused in {:?}", t.elapsed());
        match frames.take() {
            Some(f) => {
                let path = dir.join(format!("pause-{round}.ppm"));
                let mut ppm = format!("P6 {} {} 255\n", f.width, f.height).into_bytes();
                ppm.extend(f.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
                std::fs::write(&path, ppm).unwrap();
                eprintln!("drive: picture {}x{} in {}", f.width, f.height, path.display());
                frames.give_back(f);
            }
            None => eprintln!("drive: no picture"),
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let t = Instant::now();
    match s.quit() {
        Ok(()) => eprintln!("drive: quit in {:?}", t.elapsed()),
        Err(e) => { eprintln!("drive: quit: {e}"); std::process::exit(1) }
    }
}
