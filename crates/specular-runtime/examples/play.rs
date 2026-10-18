//! Runs a game headless for a while, then reports how long the core took per frame and saves
//! the last frame as raw RGBA (for checking a core and a ROM without the UI, on any machine).
//! Usage: play [--vsync] [--by-path] <core.so> <rom> [seconds (default 5)] [out.rgba (default /tmp/play.rgba)]
//! --vsync paces the game by /dev/fb0's refresh (as a device does, assuming the Brick's 60.2 Hz)
//! instead of a timer. --by-path gives the core only the ROM's path (`LaunchSpec::rom_by_path`).
//! Battery saves go to /tmp/play.srm; no sound.

use std::sync::Arc;
use std::time::Duration;

use specular_runtime::{FrameSlot, LaunchSpec, Outputs, Session};

fn main() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().collect();
    let mut flag = |name: &str| args.iter().position(|a| a == name).map(|i| args.remove(i)).is_some();
    let (vsync, rom_by_path) = (flag("--vsync"), flag("--by-path"));
    let [_, core, rom, ..] = &args[..] else { return Err("usage: play [--vsync] [--by-path] <core.so> <rom> [seconds] [out.rgba]".into()) };
    let seconds: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let out = args.get(4).cloned().unwrap_or_else(|| "/tmp/play.rgba".into());
    let frames = Arc::new(FrameSlot::default());
    let session = Session::start(
        LaunchSpec { core: core.into(), rom: rom.into(), save: "/tmp/play.srm".into(), autosave: None, system_dir: "/tmp".into(), options: vec![], rom_by_path, record: None },
        Outputs {
            sound: None, frames: frames.clone(), on_frame: Box::new(|| {}),
            vsync: if vsync { Some(Box::new(specular_linux::vsync::FbVsync::open(std::path::Path::new("/dev/fb0")).map_err(|e| e.to_string())?)) } else { None },
            display_hz: if vsync { 60.2 } else { 60.0 },
        },
    )?;
    eprintln!("{:?}", session.info());
    std::thread::sleep(Duration::from_secs_f64(seconds));
    let stats = session.stats();
    eprintln!("core time per frame: median {:.2} ms, 95th percentile {:.2} ms, worst {:.2} ms ({} frames)",
        stats.median.as_secs_f64() * 1e3, stats.p95.as_secs_f64() * 1e3, stats.worst.as_secs_f64() * 1e3, stats.frames);
    session.quit().map_err(|e| e.to_string())?;
    let f = frames.take().ok_or("no frame")?;
    std::fs::write(&out, &f.rgba).map_err(|e| e.to_string())?;
    let lit = f.rgba.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 30).count();
    eprintln!("last frame {}×{} → {out}; {lit} of {} pixels not black", f.width, f.height, f.width * f.height);
    Ok(())
}
