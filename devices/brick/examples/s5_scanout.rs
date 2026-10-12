//! Spike check "Display queue" (specular/ROADMAP.md, step 2): how long from starting a frame to
//! the display scanning it out, and whether the swap's buffering adds a frame.
//!
//! Draws frames whose colour encodes their number. A watcher thread polls which of /dev/fb0's
//! buffers the display scans out (its pan offset, every 250 µs) and timestamps each switch with
//! the frame it then shows. Reports, per frame: start of drawing → on screen. Caveat: the pan
//! offset is the display driver's record of the flip, which may differ from the panel by up to a
//! vsync; only a camera can tell exactly.
//!
//! Usage: s5_scanout [mode] [frames (default 300)]. Modes:
//!   plain        draw, swap, repeat (the swap waits for the previous frame's flip)
//!   wait-flip    after each swap, wait until that frame is on screen before starting the next,
//!                so a frame never waits behind another
//!   late N       like wait-flip, then also wait N ms before starting the frame (a "frame
//!                delay": the work, and reading the buttons, happen later, closer to the vsync)
//!   vsync N      after each swap, block in the FBIO_WAITFORVSYNC ioctl, then wait N ms (no
//!                polling): the same pacing from a blocking vsync wait

use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glow::HasContext;
use specular_brick::egl::EglDisplay;
use specular_brick::platform::SCREEN;

const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIO_WAITFORVSYNC: libc::c_ulong = 0x4004_4620; // _IOW('F', 0x20, __u32)

/// struct fb_var_screeninfo: the fields up to yoffset, then the rest as padding.
#[repr(C)]
#[derive(Default)]
struct VarInfo { xres: u32, yres: u32, xres_virtual: u32, yres_virtual: u32, xoffset: u32, yoffset: u32, _rest: [[u32; 17]; 2] }

fn var_info(fb: &std::fs::File) -> VarInfo {
    let mut i = VarInfo::default();
    unsafe { libc::ioctl(fb.as_raw_fd(), FBIOGET_VSCREENINFO, &mut i as *mut VarInfo) };
    i
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map_or("plain", |s| s.as_str()).to_string();
    let takes_ms = mode == "late" || mode == "vsync";
    let delay = Duration::from_millis(if takes_ms { args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8) } else { 0 });
    let frames: u32 = args.get(if takes_ms { 3 } else { 2 }).and_then(|s| s.parse().ok()).unwrap_or(300);
    let egl = EglDisplay::open()?;
    let gl = unsafe { glow::Context::from_loader_function_cstr(|n| egl.proc_address(n)) };
    let fb = std::fs::OpenOptions::new().read(true).open("/dev/fb0")?;
    let fb_vsync = fb.try_clone()?;
    let mut vsync_waits = Vec::new();
    let info = var_info(&fb);
    let (w, h, pages) = (info.xres as usize, info.yres as usize, (info.yres_virtual / info.yres) as usize);
    let map = unsafe { libc::mmap(std::ptr::null_mut(), w * h * 4 * pages, libc::PROT_READ, libc::MAP_SHARED, fb.as_raw_fd(), 0) };
    if map == libc::MAP_FAILED { return Err(std::io::Error::last_os_error().into()); }
    let map = map as usize;
    // The frame number (mod 256) in a buffer: its centre pixel's green channel.
    let frame_in = move |page: usize| unsafe { *(map as *const u8).add(page * w * h * 4 + (h / 2 * w + w / 2) * 4 + 1) };

    // When each frame number (mod 256) went on screen.
    let on_screen: Arc<Mutex<[Option<Instant>; 256]>> = Arc::new(Mutex::new([None; 256]));
    let watch = on_screen.clone();
    std::thread::spawn(move || {
        let mut page = usize::MAX;
        loop {
            let p = var_info(&fb).yoffset as usize / h;
            if p != page { page = p; watch.lock().unwrap()[frame_in(p) as usize] = Some(Instant::now()); }
            std::thread::sleep(Duration::from_micros(250));
        }
    });

    let mut started = vec![Instant::now(); frames as usize];
    let begin = Instant::now();
    for n in 0..frames {
        std::thread::sleep(delay);
        started[n as usize] = Instant::now();
        on_screen.lock().unwrap()[(n % 256) as usize] = None;
        unsafe {
            gl.viewport(0, 0, SCREEN.width as i32, SCREEN.height as i32);
            gl.clear_color(0.2, (n % 256) as f32 / 255.0, 0.4, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
        egl.swap_buffers()?;
        if mode == "vsync" {
            let t = Instant::now();
            let zero = 0u32;
            if unsafe { libc::ioctl(fb_vsync.as_raw_fd(), FBIO_WAITFORVSYNC, &zero) } < 0 { return Err(std::io::Error::last_os_error().into()); }
            vsync_waits.push(t.elapsed().as_secs_f64() * 1e3);
        } else if mode != "plain" {
            let t = Instant::now();
            while on_screen.lock().unwrap()[(n % 256) as usize].is_none() && t.elapsed() < Duration::from_millis(50) {
                std::thread::sleep(Duration::from_micros(250));
            }
        }
    }
    let fps = frames as f64 / begin.elapsed().as_secs_f64();
    std::thread::sleep(Duration::from_millis(50));
    // Frames still in the 256-entry table (the last ones), start → on screen.
    let table = on_screen.lock().unwrap();
    let mut ms: Vec<f64> = (frames.saturating_sub(200)..frames)
        .filter_map(|n| table[(n % 256) as usize].map(|t| t.saturating_duration_since(started[n as usize]).as_secs_f64() * 1e3))
        .collect();
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if ms.is_empty() { println!("{mode}: no switches seen"); return Ok(()); }
    if !vsync_waits.is_empty() {
        vsync_waits.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("  FBIO_WAITFORVSYNC blocked {:.1} min, {:.1} median, {:.1} max ms", vsync_waits[0], vsync_waits[vsync_waits.len() / 2], vsync_waits[vsync_waits.len() - 1]);
    }
    println!("{mode}{}: {fps:.1} fps; frame start → on screen: {:.1} min, {:.1} median, {:.1} max ms ({} frames)",
        if delay.is_zero() { String::new() } else { format!(" {} ms", delay.as_millis()) },
        ms[0], ms[ms.len() / 2], ms[ms.len() - 1], ms.len());
    Ok(())
}
