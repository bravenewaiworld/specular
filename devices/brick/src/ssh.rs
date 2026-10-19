//! SSH on the Brick: Crystaline OS's brick-ssh. On release systems the user switches SSH and
//! imports its login keys; on development systems it is always on with the key built into the
//! system, and brick-ssh refuses to change it.

use specular_core::device::{Ssh, SshCmd, SshStatus};

use crate::command::run;

const PROGRAM: &str = "/usr/sbin/brick-ssh";

/// The Brick's `Ssh`: brick-ssh.
pub struct BrickSsh;

impl Ssh for BrickSsh {
    fn send(&self, cmd: SshCmd) -> Result<SshStatus, String> {
        let arg = match cmd { SshCmd::Status => None, SshCmd::Switch(true) => Some("on"), SshCmd::Switch(false) => Some("off"), SshCmd::ImportKeys => Some("import") };
        // brick-ssh's error messages start with "brick-ssh: ", which the user needn't see.
        let brick_ssh = |args: &[&str]| run("ssh", PROGRAM, args).map_err(|e| e.strip_prefix("brick-ssh: ").unwrap_or(&e).to_string());
        if let Some(arg) = arg { brick_ssh(&[arg])?; }
        status(&brick_ssh(&[])?)
    }
}

/// brick-ssh's state, one name=value per line: `image=release|development`, `ssh=on|off` and (on
/// release systems) `keys=N`.
fn status(text: &str) -> Result<SshStatus, String> {
    let value = |name: &str| text.lines().find_map(|l| l.trim().strip_prefix(name)?.strip_prefix('='));
    let switchable = match value("image") {
        Some("release") => true,
        Some("development") => false,
        other => return Err(format!("brick-ssh said image={}", other.unwrap_or("(nothing)"))),
    };
    Ok(SshStatus { switchable, on: value("ssh") == Some("on"), keys: value("keys").and_then(|n| n.parse().ok()).unwrap_or(0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_brick_sshs_state() {
        assert_eq!(status("image=release\nssh=off\nkeys=0\n"), Ok(SshStatus { switchable: true, on: false, keys: 0 }));
        assert_eq!(status("image=release\nssh=on\nkeys=3"), Ok(SshStatus { switchable: true, on: true, keys: 3 }));
        assert_eq!(status("image=development\nssh=on\n"), Ok(SshStatus { switchable: false, on: true, keys: 0 }));
        assert!(status("").is_err());
    }
}
