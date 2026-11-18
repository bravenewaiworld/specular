//! SSH: Crystaline OS's crystaline-ssh. On release systems the user switches SSH and
//! imports its login keys; on development systems it is always on with the key built into the
//! system, and crystaline-ssh refuses to change it.

use specular_core::device::{Ssh, SshCmd, SshStatus};

use crate::command::run;

const PROGRAM: &str = "/usr/sbin/crystaline-ssh";

/// `Ssh` through crystaline-ssh.
pub struct CrystalineSsh;

impl Ssh for CrystalineSsh {
    fn send(&self, cmd: SshCmd) -> Result<SshStatus, String> {
        let arg = match cmd { SshCmd::Status => None, SshCmd::Switch(true) => Some("on"), SshCmd::Switch(false) => Some("off"), SshCmd::ImportKeys => Some("import") };
        // crystaline-ssh's error messages start with "crystaline-ssh: ", which the user needn't see.
        let crystaline_ssh = |args: &[&str]| run("ssh", PROGRAM, args).map_err(|e| e.strip_prefix("crystaline-ssh: ").unwrap_or(&e).to_string());
        if let Some(arg) = arg { crystaline_ssh(&[arg])?; }
        status(&crystaline_ssh(&[])?)
    }
}

/// crystaline-ssh's state, one name=value per line: `image=release|development`, `ssh=on|off` and (on
/// release systems) `keys=N`.
fn status(text: &str) -> Result<SshStatus, String> {
    let value = |name: &str| text.lines().find_map(|l| l.trim().strip_prefix(name)?.strip_prefix('='));
    let switchable = match value("image") {
        Some("release") => true,
        Some("development") => false,
        other => return Err(format!("crystaline-ssh said image={}", other.unwrap_or("(nothing)"))),
    };
    Ok(SshStatus { switchable, on: value("ssh") == Some("on"), keys: value("keys").and_then(|n| n.parse().ok()).unwrap_or(0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_crystaline_sshs_state() {
        assert_eq!(status("image=release\nssh=off\nkeys=0\n"), Ok(SshStatus { switchable: true, on: false, keys: 0 }));
        assert_eq!(status("image=release\nssh=on\nkeys=3"), Ok(SshStatus { switchable: true, on: true, keys: 3 }));
        assert_eq!(status("image=development\nssh=on\n"), Ok(SshStatus { switchable: false, on: true, keys: 0 }));
        assert!(status("").is_err());
    }
}
