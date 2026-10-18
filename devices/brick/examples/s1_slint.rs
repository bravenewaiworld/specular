//! Spike checks "Slint on our EGL", "GL function lookup" and "Input" (docs/brick.md):
//! Slint's FemtoVG renderer on the Brick's EGL display.
//!
//! Shows a Light Bar style list whose focus moves every 1.5 s (a short animation, then idle); up
//! and down on the d-pad move it too. Prints every 5 s: frames drawn; render time per frame and
//! how much of it was spent waiting in the swap (for vsync); the longest gap between frames drawn
//! back to back and how many came late (a missed vsync); CPU use; GPU interrupts; and for d-pad
//! presses, the time from the press (kernel timestamp) to the end of the present that showed it.
//!
//! Usage: s1_slint [seconds (default 20)] [--continuous: animate all the time, to check vsync]
//!        [--idle: never move the focus, to check that an unchanging screen costs nothing]
//!        [--vsync: pace frames by /dev/fb0's FBIO_WAITFORVSYNC, as Specular does, rather than by
//!         the swap alone; the time in the swap should then be about 1 ms]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use specular_linux::input::{Button, clock};
use specular_brick::platform::BrickPlatform;

/// The display's frame period (it runs at about 60.2 Hz).
const PERIOD: Duration = Duration::from_micros(16_611);

// Some colours are written as rgb(): inside slint!, a hex colour whose digits read like a number
// with an exponent (#2ec4b6 starts "2e…") is rejected by Rust's tokenizer.
slint::slint! {
    import "../../../assets/fonts/Geist.ttf";
    import "../../../assets/fonts/GeistMono.ttf";

    export component Scene inherits Window {
        in property <int> selected;
        in property <bool> continuous;
        in property <string> stats;
        background: #08090a;
        default-font-family: "Geist";

        // the light bar along the top edge
        Rectangle { y: 0; height: 3px; background: rgb(46, 196, 182); drop-shadow-blur: 18px; drop-shadow-color: rgba(46, 196, 182, 45%); }

        Text { x: 48px; y: 90px; text: "Library"; font-size: 64px; font-weight: 600; color: #f2f2f2; }

        Rectangle {
            x: 28px; width: 560px; height: 92px;
            y: 200px + root.selected * 92px;
            animate y { duration: 220ms; easing: cubic-bezier(0.3, 1.4, 0.5, 1); }
            background: #202226;
            border-bottom-left-radius: 12px; border-bottom-right-radius: 12px;
            Rectangle { x: 0; y: 18px; width: 4px; height: parent.height - 36px; background: rgb(46, 196, 182); }
        }
        for name[i] in ["Continue", "Game Boy", "Game Boy Color", "Game Boy Advance", "Super Nintendo"] : Text {
            x: 48px; y: 200px + i * 92px; height: 92px; vertical-alignment: center;
            text: name; font-size: 42px; color: i == root.selected ? #f2f2f2 : #8a8f96;
            font-weight: i == root.selected ? 500 : 400;
        }

        // text sizes from the design spec, for judging quality on the panel
        for size[i] in [27, 33, 42, 64] : Text {
            x: 640px; y: 200px + i * 90px; text: "Golden Sun " + size; font-size: size * 1px; color: #f2f2f2;
        }
        Text { x: 640px; y: 560px; text: "21:47"; font-size: 96px; font-family: "Geist Mono"; color: #f2f2f2; }

        // something that never stops, for --continuous
        if root.continuous : Rectangle {
            x: 48px + (Math.sin(animation-tick() / 1s * 1turn * 0.5) + 1) * 400px; y: 700px; width: 40px; height: 40px;
            background: rgb(46, 196, 182);
        }

        Text { x: 48px; y: 730px; text: root.stats; font-size: 22px; font-family: "Geist Mono"; color: #8a8f96; }
    }
}

/// Measurements for one report period; all times on CLOCK_MONOTONIC.
#[derive(Default)]
struct Period {
    frames: u64, render: Duration, swap: Duration, max_gap: Duration, late: u64,
    presses: u64, press_latency: Duration, press_latency_max: Duration,
}

#[derive(Default)]
struct Measure {
    now: Period,
    before_render: Duration,
    after_render: Duration,
    last_present: Option<Duration>,
    /// Time of a d-pad press not yet shown.
    pending_press: Option<Duration>,
    first_frame: Option<Duration>,
}

/// Interrupts raised so far by the GPU driver (pvrsrvkm), summed over all CPUs.
fn gpu_irqs() -> u64 {
    std::fs::read_to_string("/proc/interrupts").unwrap_or_default().lines()
        .find(|l| l.ends_with("pvrsrvkm"))
        .map(|l| l.split_whitespace().skip(1).take(4).filter_map(|n| n.parse::<u64>().ok()).sum())
        .unwrap_or(0)
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let started = clock(libc::CLOCK_MONOTONIC);
    let args: Vec<String> = std::env::args().collect();
    let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let continuous = args.iter().any(|a| a == "--continuous");
    let idle = args.iter().any(|a| a == "--idle");

    let vsync: Option<Box<dyn specular_core::device::Vsync>> = if args.iter().any(|a| a == "--vsync") {
        Some(Box::new(specular_linux::vsync::FbVsync::open(std::path::Path::new("/dev/fb0"))?))
    } else { None };
    let platform = BrickPlatform::new(vsync)?;
    let m = Rc::new(RefCell::new(Measure::default()));
    // The scene exists only after the platform is handed to Slint, so the button handler finds
    // it through this slot.
    let scene_slot: Rc<RefCell<Option<slint::Weak<Scene>>>> = Default::default();
    let (slot, mm) = (scene_slot.clone(), m.clone());
    platform.on_button(move |e| {
        let Some(scene) = slot.borrow().as_ref().and_then(|w| w.upgrade()) else { return };
        if !e.pressed { return; }
        let step = match e.button { Button::Down => 1, Button::Up => 4, b => { println!("button {b:?}"); return; } };
        scene.set_selected((scene.get_selected() + step) % 5);
        mm.borrow_mut().pending_press.get_or_insert(e.time);
    });
    let mm = m.clone();
    platform.on_present(move |t| {
        let m = &mut *mm.borrow_mut();
        let p = &mut m.now;
        p.frames += 1;
        p.render += t.saturating_sub(m.before_render);
        p.swap += t.saturating_sub(m.after_render);
        // Gaps count only between frames drawn back to back (animating), not across idle time.
        if let Some(prev) = m.last_present.filter(|&prev| t - prev < PERIOD * 4) {
            p.max_gap = p.max_gap.max(t - prev);
            if t - prev > PERIOD * 3 / 2 { p.late += 1; }
        }
        m.last_present = Some(t);
        if let Some(press) = m.pending_press.take() {
            let l = t.saturating_sub(press);
            p.presses += 1; p.press_latency += l; p.press_latency_max = p.press_latency_max.max(l);
        }
        m.first_frame.get_or_insert(t - started);
    });
    slint::platform::set_platform(Box::new(platform)).map_err(|e| format!("{e:?}"))?;
    let scene = Scene::new()?;
    scene.set_continuous(continuous);
    *scene_slot.borrow_mut() = Some(scene.as_weak());
    let mm = m.clone();
    scene.window().set_rendering_notifier(move |state, _| {
        let t = clock(libc::CLOCK_MONOTONIC);
        match state {
            slint::RenderingState::BeforeRendering => mm.borrow_mut().before_render = t,
            slint::RenderingState::AfterRendering => mm.borrow_mut().after_render = t,
            _ => {}
        }
    }).map_err(|e| format!("{e:?}"))?;
    scene.show()?;

    let mover = slint::Timer::default();
    if !continuous && !idle {
        let weak = scene.as_weak();
        mover.start(slint::TimerMode::Repeated, Duration::from_millis(1500), move || {
            if let Some(s) = weak.upgrade() { s.set_selected((s.get_selected() + 1) % 5); }
        });
    }

    let reporter = slint::Timer::default();
    let (weak, mm) = (scene.as_weak(), m.clone());
    let mut prev = (clock(libc::CLOCK_PROCESS_CPUTIME_ID), gpu_irqs(), Instant::now());
    reporter.start(slint::TimerMode::Repeated, Duration::from_secs(5), move || {
        let p = std::mem::take(&mut mm.borrow_mut().now);
        let (cpu, irq, now) = (clock(libc::CLOCK_PROCESS_CPUTIME_ID), gpu_irqs(), Instant::now());
        let wall = (now - prev.2).as_secs_f64();
        let per = |d: Duration, n: u64| if n > 0 { d.as_secs_f64() * 1e3 / n as f64 } else { 0.0 };
        let line = format!(
            "{} frames in {wall:.1}s ({:.1}/s), {:.2} ms render per frame, of which {:.2} in the swap; longest gap {:.1} ms, {} late; CPU {:.1}%; GPU irqs {}; {} presses: avg {:.1} / max {:.1} ms to present",
            p.frames, p.frames as f64 / wall, per(p.render, p.frames), per(p.swap, p.frames),
            p.max_gap.as_secs_f64() * 1e3, p.late, (cpu - prev.0).as_secs_f64() / wall * 100.0, irq - prev.1,
            p.presses, per(p.press_latency, p.presses), p.press_latency_max.as_secs_f64() * 1e3);
        println!("{line}");
        if let Some(s) = weak.upgrade() { s.set_stats(line.into()); }
        prev = (cpu, irq, now);
    });

    let stopper = slint::Timer::default();
    stopper.start(slint::TimerMode::SingleShot, Duration::from_secs(seconds), || { let _ = slint::quit_event_loop(); });
    scene.run()?;

    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for key in ["VmRSS", "VmHWM"] {
        if let Some(l) = status.lines().find(|l| l.starts_with(key)) { println!("{l}"); }
    }
    if let Some(t) = m.borrow().first_frame { println!("first frame on screen {:.0} ms after main() started", t.as_secs_f64() * 1e3); }
    Ok(())
}
