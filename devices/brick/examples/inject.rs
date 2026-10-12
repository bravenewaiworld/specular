//! Test helper: presses the d-pad down/up on an input device by writing events to its node (the
//! kernel passes injected events to every reader). Skips padbridge's hop from the real buttons.
//! Usage: inject <event node of "TRIMUI Player1", e.g. /dev/input/event5> [presses (default 20)]
//!        [interval ms (default 400)]

use std::io::Write;

const EV_SYN: u16 = 0; const SYN_REPORT: u16 = 0; const EV_ABS: u16 = 3; const ABS_HAT0Y: u16 = 17;

fn event(kind: u16, code: u16, value: i32) -> [u8; 24] {
    let mut b = [0u8; 24]; // struct input_event on 64-bit: timeval (set by the kernel), type, code, value
    b[16..18].copy_from_slice(&kind.to_ne_bytes());
    b[18..20].copy_from_slice(&code.to_ne_bytes());
    b[20..24].copy_from_slice(&value.to_ne_bytes());
    b
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut dev = std::fs::OpenOptions::new().write(true).open(&args[1])?;
    let presses: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let interval = std::time::Duration::from_millis(args.get(3).and_then(|s| s.parse().ok()).unwrap_or(400));
    for i in 0..presses {
        let dir = if i % 2 == 0 { 1 } else { -1 }; // 1 = down
        for v in [dir, 0] {
            dev.write_all(&[event(EV_ABS, ABS_HAT0Y, v), event(EV_SYN, SYN_REPORT, 0)].concat())?;
            std::thread::sleep(interval / 2);
        }
    }
    Ok(())
}
