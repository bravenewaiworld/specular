//! SSH in the developer settings, on systems where the user may switch it
//! (`SshStatus::switchable`): a switch, and importing login keys from the SD card. The system
//! keeps the state; the app asks for it each time Settings opens.

use super::*;
use crate::device::{SshCmd, SshStatus};

/// What the app knows of SSH.
#[derive(Default)]
pub struct SshState {
    /// The last report (None until the first; Err: SSH's state couldn't be read).
    pub status: Option<Result<SshStatus, String>>,
}

impl App {
    /// Says whether the app manages the device's SSH (not until told). Call before the first
    /// message.
    pub fn has_ssh(&mut self, yes: bool) { self.ctx.ssh = yes.then(SshState::default); }

    /// `cmd` finished. A failed switch or import says why; the state is then as before.
    pub(super) fn ssh_done(&mut self, cmd: SshCmd, result: Result<SshStatus, String>) -> Vec<Effect> {
        let Some(state) = &mut self.ctx.ssh else { return vec![] };
        let notice = match (cmd, &result) {
            (SshCmd::ImportKeys, Ok(s)) => Some(format!("Imported {}", keys(s.keys))),
            (SshCmd::ImportKeys, Err(e)) => Some(format!("Couldn't import SSH keys: {e}")),
            (SshCmd::Switch(on), Err(e)) => Some(format!("Couldn't switch SSH {}: {e}", if on { "on" } else { "off" })),
            _ => None,
        };
        if result.is_ok() || cmd == SshCmd::Status { state.status = Some(result); }
        self.changed();
        notice.map(Effect::Notice).into_iter().chain([Effect::Redraw(None)]).collect()
    }
}

/// "1 SSH key", "2 SSH keys".
fn keys(n: u32) -> String { format!("{n} SSH key{}", if n == 1 { "" } else { "s" }) }

/// The developer settings' SSH rows: the switch and the import where the user switches SSH; else
/// why there are none.
pub(super) fn settings_rows(ctx: &Ctx) -> Vec<Row> {
    let Some(state) = &ctx.ssh else { return vec![] };
    match &state.status {
        Some(Ok(s)) if s.switchable => {
            let mut rows = vec![
                Row::new("ssh:switch", "SSH", RowKind::Toggle { on: s.on }),
                Row::new("ssh:import", "Import SSH keys from the SD card", RowKind::Action { value: Some(keys(s.keys)) }),
            ];
            if s.keys == 0 { rows.push(Row::note("ssh:how", "SSH accepts only imported keys: put your public keys in authorized_keys at the SD card's root, then import them.")); }
            rows
        }
        Some(Ok(_)) => vec![Row::note("ssh:why", "SSH is always on (a development system).")],
        Some(Err(e)) => vec![Row::note("ssh:why", format!("SSH: {e}"))],
        None => vec![],
    }
}

/// A press on a row of `settings_rows`: the switch switches (A, left or right, as other
/// switches), and A imports.
pub(super) fn settings_press(ctx: &Ctx, row: &Row, p: Press) -> Option<Outcome> {
    if p.repeat { return None; }
    let on = matches!(ctx.ssh.as_ref()?.status, Some(Ok(s)) if s.on);
    let cmd = match (row.id.0.as_str(), p.button) {
        ("ssh:switch", Button::A | Button::Left | Button::Right) => SshCmd::Switch(!on),
        ("ssh:import", Button::A) => SshCmd::ImportKeys,
        _ => return None,
    };
    Some(Outcome::Effects(vec![Effect::Ssh(cmd)]))
}
