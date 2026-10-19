//! The CPU's speed on the Brick: Crystaline OS's brick-power, which sets the speed range of the
//! one clock all four CPU cores share, and a minimum within it (its `cpu-min`, refused until a profile
//! has been applied since start-up). Nothing it sets outlasts a restart.

use specular_core::device::Cpu;

use crate::command::run;

const PROGRAM: &str = "/usr/sbin/brick-power";

/// The Brick's `Cpu`: brick-power.
pub struct BrickCpu;

impl Cpu for BrickCpu {
    /// Applies the balanced profile (408–2000 MHz), the one the system boots with, again: `cpu-min` is
    /// refused until a profile has been applied since boot. Then reads the speed steps.
    fn prepare(&self) -> Result<Vec<u32>, String> {
        run("cpu", PROGRAM, &["profile", "balanced"])?;
        steps(&run("cpu", PROGRAM, &[])?)
    }

    fn set_min(&self, mhz: Option<u32>) -> Result<(), String> {
        let arg = mhz.map_or("default".into(), |m| m.to_string());
        run("cpu", PROGRAM, &["cpu-min", &arg]).map(|_| ())
    }
}

/// The speed steps in brick-power's status: its line `cpu-steps 408 600 … 2000` (MHz).
fn steps(status: &str) -> Result<Vec<u32>, String> {
    let line = status.lines().find_map(|l| l.strip_prefix("cpu-steps")).ok_or("brick-power's status has no cpu-steps line")?;
    let steps: Vec<u32> = line.split_whitespace().map(|s| s.parse().map_err(|_| format!("brick-power's cpu-steps has {s:?}"))).collect::<Result<_, _>>()?;
    if steps.is_empty() || !steps.is_sorted() { return Err(format!("brick-power's cpu-steps aren't a list of speeds, slowest first: {:?}", line.trim())); }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_the_speed_steps_from_brick_powers_status() {
        let status = "profile   balanced\ncpu       408-2000 MHz, now 1200 MHz, thermal limit step 0/8 (0: not limited)\ncpu-min   default\n\
                      cpu-steps 408 600 816 1008 1200 1416 1608 1800 2000\ngovernor  ondemand, sampling 50 ms\n";
        assert_eq!(super::steps(status), Ok(vec![408, 600, 816, 1008, 1200, 1416, 1608, 1800, 2000]));
        assert!(super::steps("profile   balanced\n").is_err());
        assert!(super::steps("cpu-steps\n").is_err());
        assert!(super::steps("cpu-steps 600 408\n").is_err());
    }
}
