//! Checks the display stack on a mainline board: KMS + GBM + EGL, page flips at vblank, and the
//! panel's real refresh rate. Run it with the frontend stopped (it needs to be the DRM master).
//! Fills the screen with a changing colour for `frames` frames (default 300), then prints the
//! flip statistics. With `all`, does that in each of the panel's modes in turn, measuring each
//! mode's real rate after switching to it.
//!
//! Usage: kms_spike [frames] [all] [card (default /dev/dri/card0)]

use std::path::PathBuf;
use std::time::Instant;

use glow::HasContext;
use specular_linux::egl::EglDisplay;
use specular_linux::kms::Kms;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let frames: u32 = args.first().map_or(Ok(300), |a| a.parse())?;
    let all = args.iter().any(|a| a == "all");
    let card = PathBuf::from(args.iter().skip(1).find(|a| *a != "all").cloned().unwrap_or_else(|| "/dev/dri/card0".into()));
    let mut kms = Kms::open(&card)?;
    let mut vsync = kms.vsync(&card)?;
    let measured = vsync.measure(30)?;
    eprintln!("refresh: measured {measured:.3} Hz over 30 vblanks, mode says {:.3} Hz", kms.mode().refresh_hz());
    let egl = EglDisplay::open(kms.native())?;
    let gl = unsafe { glow::Context::from_loader_function_cstr(|name| egl.proc_address(name)) };
    eprintln!("gl: {} / {}", unsafe { gl.get_parameter_string(glow::RENDERER) }, unsafe { gl.get_parameter_string(glow::VERSION) });
    let modes: Vec<_> = if all { kms.modes().to_vec() } else { vec![kms.mode()] };
    for mode in modes {
        kms.set_mode(mode)?;
        run(&gl, &egl, &mut kms, 1)?; // sets the mode
        kms.take_flip_stats();
        let measured = vsync.measure(60)?;
        eprintln!("mode {mode}: measured {measured:.3} Hz over 60 vblanks ({:+.0} ppm)", (measured / mode.refresh_hz() - 1.0) * 1e6);
        run(&gl, &egl, &mut kms, frames)?;
    }
    Ok(())
}

/// Draws and presents `frames` frames, then prints how the flips went.
fn run(gl: &glow::Context, egl: &EglDisplay, kms: &mut Kms, frames: u32) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();
    let mut slowest = std::time::Duration::ZERO;
    for i in 0..frames {
        let t = Instant::now();
        let x = (i % 120) as f32 / 120.0;
        unsafe { gl.clear_color(x, 0.3, 1.0 - x, 1.0); gl.clear(glow::COLOR_BUFFER_BIT); }
        egl.swap_buffers()?;
        kms.present()?;
        slowest = slowest.max(t.elapsed());
    }
    if frames < 2 { return Ok(()); }
    let took = start.elapsed().as_secs_f64();
    let stats = kms.take_flip_stats();
    eprintln!("  {frames} frames in {took:.2} s: {:.3} fps; flips {}, late {}, longest gap {:.1} ms, slowest present {:.1} ms",
        frames as f64 / took, stats.flips, stats.late, stats.longest.as_secs_f64() * 1000.0, slowest.as_secs_f64() * 1000.0);
    Ok(())
}
