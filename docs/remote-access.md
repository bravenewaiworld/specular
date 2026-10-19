# Remote access

Status: Implemented · Updated: 2026-10-19

Reaching the device from a PC: file transfer (a browser page and WebDAV, served from the device, that read and write the SD card) and SSH. Both open the device to the network, so Specular decides when they run and says so plainly; both are the system's services (Crystaline OS's `webdav` and `brick-ssh`), which Specular only starts, stops and asks about.

## Decisions

- **File transfer runs only while its screen is open** (2026-10-19). The server has no password: while it runs, anyone on the network can read and change everything on the card, which the screen says. It lives in Settings → Connections, beside Wi-Fi and Bluetooth. Rejected: a switch in Settings (easily left on and forgotten).
- **The server is wanted while its screen is open, the SD card is in and the device is awake; the app sends only changes** (2026-10-19), as it does for the radios. A card that comes back starts it again; waking with the screen open starts it again. "The card is in" is `Ctx::card_found` for now, in one place (`App::reconcile_transfer`) and the screen's view.
- **Sleeping stops it first; the device doesn't sleep by itself while the screen is open** (2026-10-19), since a transfer may be under way. A tap of the power button still sleeps. Rejected: idle sleep as usual (it would cut an upload short).
- **WiFi on Auto counts the open screen as a need**, as the Wi-Fi screen does (2026-10-19). A USB cable works without WiFi.
- **Started means port 80 takes connections, within 5 s** (2026-10-19). `sv up` returns as soon as the service manager is told, whatever then happens. A start that fails stops the service again (rather than leaving it retrying every second out of sight) and shows the service manager's status and the service's last log lines. Rejected: trusting `sv up`'s exit status.
- **File transfer is stopped whenever Specular starts and quits** (2026-10-19), so a run that crashed or was stopped with the screen open doesn't leave the card open. Rejected: remembering whether it was on (more state, and the stop is cheap and harmless when it is already down).
- **File transfer and SSH commands run on the hardware thread** (2026-10-19), in order with the other hardware commands, so a stop sent before a sleep is done before it. Starting can block the thread for up to 5 s (only when it fails). Rejected: a thread of their own (sleeping would then need a handshake).
- **The addresses shown are those Specular already follows** (2026-10-19): WiFi's from its link, and the other interfaces' with a link from the WiFi service (`WifiEvent::Interfaces`), so the USB cable's shows only while a PC is at the other end. Interfaces named `usb…` are called "USB cable".
- **SSH is in the developer settings, only where the user switches it** (2026-10-19): Crystaline OS release systems, which `brick-ssh` reports as `image=release`. On development systems it is always on and `brick-ssh` refuses to change it, so there is only a note. Its state is the system's, asked for each time Settings opens. Rejected: a Specular setting (a second copy of the state, which could disagree).

## Design

### Device interfaces

```rust
pub struct Hardware {
    // ...
    pub file_transfer: Option<Box<dyn FileTransfer>>,
    pub ssh: Option<Box<dyn Ssh>>,
}

pub trait FileTransfer: Send {
    /// Starts the server and waits until it takes connections. Err: why not; left stopped.
    fn start(&self) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
}

pub trait Ssh: Send {
    /// Carries out `cmd`, then reports SSH's state.
    fn send(&self, cmd: SshCmd) -> Result<SshStatus, String>;
}
pub enum SshCmd { Status, Switch(bool), ImportKeys }
pub struct SshStatus { pub switchable: bool, pub on: bool, pub keys: u32 }
```

On the Brick (`devices/brick/src/transfer.rs`, `ssh.rs`):

| | |
|---|---|
| Start | `sv up webdav`, then connect to 127.0.0.1:80 until it answers (5 s). If it doesn't: `sv status webdav`, the last 3 `webdav:` lines of `logread`, then a stop |
| Stop | `sv -v down webdav` (waits until it is down) |
| SSH | `brick-ssh [on\|off\|import]`, then `brick-ssh` for the state (`image=`, `ssh=`, `keys=` lines). Its errors are its stderr, without the `brick-ssh: ` prefix |

### The app

```rust
pub enum Msg { /* ... */
    FileTransfer { start: bool, result: Result<(), String> },
    Ssh { cmd: SshCmd, result: Result<SshStatus, String> },
}
pub enum Effect { /* ... */ FileTransfer(bool), Ssh(SshCmd) }

pub struct TransferState { pub server: Server, told: bool }
pub enum Server { Stopped, Starting, Running, Failed(String) }
pub struct SshState { pub status: Option<Result<SshStatus, String>> }
```

`App::reconcile_transfer` runs after every message and before a sleep: if what is wanted differs from what the server was last told, it sends `Effect::FileTransfer`. A result overtaken by the opposite command (the screen left while starting) changes nothing, except that a failed stop is always shown. "Try again" after a failed start sends the start again. A failed SSH switch or import shows the system's message; the state is then as before.

### The log

Every command the Brick runs is logged with its exit status and output (`transfer: /usr/bin/sv up webdav: exit status: 0, stdout "", stderr ""`), as is each SSH result (`ssh: ImportKeys: Ok(SshStatus { … })`) and each failed start or stop (`transfer: starting: …`).

## Findings

- (2026-10-19, upstream) runit's `sv up` only tells the service's supervisor, and returns before the service runs; the supervisor restarts a run script that exits every second. `sv -v down` waits (7 s by default) for the service to be down. Source: runit's sv(8); BusyBox `runit/sv.c`.
- (2026-10-19, inferred) dufs logs each request, through `logger -t webdav` into the system log, so the service's lines there can be many; only the last 3 are read.

## Open questions

- If Specular can't start at all (Crystaline OS falls back to its status screen after 3 crashes in a minute), WebDAV left on by the last crash stays up until a restart. Stopping it in the OS's `frontend-run` before each start would cover that.
- Another program on port 80 would pass the start check. Nothing else listens there on Crystaline OS today.
