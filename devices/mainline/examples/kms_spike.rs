//! Checks the display stack on a mainline board: KMS + GBM + EGL, page flips at vblank, and the
//! panel's real refresh rate. Run it with the frontend stopped (it needs to be the DRM master).
//! Fills the screen with a changing colour for `frames` frames (default 300), then prints the
//! flip statistics.
//!
//! Usage: kms_spike [frames] [card (default /dev/dri/card0)]

use std::path::PathBuf;
use std::time::Instant;

use glow::HasContext;
use specular_linux::egl::EglDisplay;
use specular_linux::kms::Kms;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let frames: u32 = std::env::args().nth(1).map_or(Ok(300), |a| a.parse())?;
    let card = PathBuf::from(std::env::args().nth(2).unwrap_or_else(|| "/dev/dri/card0".into()));
    let mut kms = Kms::open(&card)?;
    let mut vsync = kms.vsync(&card)?;
    let measured = vsync.measure(30)?;
    eprintln!("refresh: measured {measured:.3} Hz over 30 vblanks, mode says {:.3} Hz", kms.mode().refresh_hz());
    let egl = EglDisplay::open(kms.native())?;
    let gl = unsafe { glow::Context::from_loader_function_cstr(|name| egl.proc_address(name)) };
    eprintln!("gl: {} / {}", unsafe { gl.get_parameter_string(glow::RENDERER) }, unsafe { gl.get_parameter_string(glow::VERSION) });
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
    let took = start.elapsed().as_secs_f64();
    let stats = kms.take_flip_stats();
    eprintln!("{frames} frames in {took:.2} s: {:.2} fps; flips {}, late {}, longest gap {:.1} ms, slowest present {:.1} ms",
        frames as f64 / took, stats.flips, stats.late, stats.longest.as_secs_f64() * 1000.0, slowest.as_secs_f64() * 1000.0);
    Ok(())
}
