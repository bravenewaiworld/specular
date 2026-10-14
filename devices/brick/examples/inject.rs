//! Test helper: presses buttons on an input device by writing events to its node (the kernel
//! passes injected events to every reader), skipping padbridge's hop from the real buttons.
//! Usage: inject <event node of "TRIMUI Player1", e.g. /dev/input/event5> <buttons> [step ms (default 400)]
//! <buttons> is a comma-separated list of up, down, left, right, a, b, x, y, l1, r1, select, start,
//! menu, volup, voldown and power, pressed one per step and each held for 50 ms; "wait" leaves a step
//! empty.

use std::io::Write;
use std::time::Duration;

const EV_SYN: u16 = 0; const SYN_REPORT: u16 = 0; const EV_KEY: u16 = 1; const EV_ABS: u16 = 3;
const ABS_HAT0X: u16 = 16; const ABS_HAT0Y: u16 = 17;

fn event(kind: u16, code: u16, value: i32) -> [u8; 24] {
    let mut b = [0u8; 24]; // struct input_event on 64-bit: timeval (set by the kernel), type, code, value
    b[16..18].copy_from_slice(&kind.to_ne_bytes());
    b[18..20].copy_from_slice(&code.to_ne_bytes());
    b[20..24].copy_from_slice(&value.to_ne_bytes());
    b
}

/// A button as padbridge's virtual pad reports it: (event type, code, value while pressed). The
/// face buttons are an Xbox pad's, named by position as in specular_linux::input.
fn button(name: &str) -> Option<(u16, u16, i32)> {
    Some(match name {
        "up" => (EV_ABS, ABS_HAT0Y, -1), "down" => (EV_ABS, ABS_HAT0Y, 1),
        "left" => (EV_ABS, ABS_HAT0X, -1), "right" => (EV_ABS, ABS_HAT0X, 1),
        "b" => (EV_KEY, 0x130, 1), "a" => (EV_KEY, 0x131, 1), "y" => (EV_KEY, 0x133, 1), "x" => (EV_KEY, 0x134, 1),
        "l1" => (EV_KEY, 0x136, 1), "r1" => (EV_KEY, 0x137, 1),
        "select" => (EV_KEY, 0x13a, 1), "start" => (EV_KEY, 0x13b, 1), "menu" => (EV_KEY, 0x13c, 1),
        "volup" => (EV_KEY, 115, 1), "voldown" => (EV_KEY, 114, 1), // KEY_VOLUMEUP/DOWN
        "power" => (EV_KEY, 116, 1), // KEY_POWER: on the power key's own device, axp2202-pek
        _ => return None,
    })
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(node), Some(script)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: inject <event node> <buttons> [step ms]");
        std::process::exit(2);
    };
    let step = Duration::from_millis(args.get(3).and_then(|s| s.parse().ok()).unwrap_or(400));
    let hold = Duration::from_millis(50).min(step);
    let mut dev = std::fs::OpenOptions::new().write(true).open(node)?;
    for name in script.split(',').map(str::trim) {
        if name != "wait" {
            let Some((kind, code, on)) = button(name) else { eprintln!("inject: no button {name:?}"); std::process::exit(2) };
            for value in [on, 0] {
                dev.write_all(&[event(kind, code, value), event(EV_SYN, SYN_REPORT, 0)].concat())?;
                std::thread::sleep(if value == on { hold } else { step - hold });
            }
        } else {
            std::thread::sleep(step);
        }
    }
    Ok(())
}
