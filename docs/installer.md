# Installer

Status: Implemented · Updated: 2026-10-18

Installing the system onto a device's internal storage, and putting a backup of it back, from Settings → Internal storage. An interrupted install can leave the device's internal storage unable to start, so the flow is built around never being interrupted by Specular, and around recognising when it was interrupted anyway.

## Decisions

- **The device's installer program does the work; Specular offers it, asks first and shows progress** (2026-10-14). The work is the OS's (on the Brick, `brick-install` in Crystaline OS); Specular only talks to it through the `Installer` interface, so a device without one simply has no such settings.
- **A backup of the internal storage to the SD card is offered first** (2026-10-14), when the card has room for as much as the internal storage holds (how well it compresses isn't known beforehand). It is on to start with, unless the internal storage holds an unfinished install or restore.
- **A developer option keeps the SD card bootable** (2026-10-14). Without it, the device starts from the internal storage from then on.
- **While a task runs, nothing may interrupt it** (2026-10-14). Its screen can't be left, and sleep and power off wait for it, saying so.
- **A task cut short is recognised** (2026-10-14). One still running (Specular crashed or was restarted) is shown again; an unfinished one (a power cut) shows in what the internal storage holds, and can be run again. The SD card still starts the device meanwhile.
- **Failures say what happened, what to do, and where the log is** (2026-10-14). Why installing isn't possible at all is meant for developers and shown only under Developer details.

## Design

```rust
pub trait Installer: Send {
    /// What can be done now. Looks at the disks: call it off the UI thread.
    fn offer(&self) -> Result<Offer, String>;
    /// Progress goes to `progress` from another thread, ending with `Done`.
    fn start(&self, task: Task, progress: Box<dyn FnMut(Progress) + Send>);
    /// Reports a task an earlier run of Specular left running, from its start; nothing if none.
    fn follow(&self, progress: Box<dyn FnMut(Progress) + Send>);
}

pub struct Offer {
    pub install: Option<InstallOffer>,  // { bytes, backup_possible, free }
    pub why_not: Option<String>,        // for developers
    pub internal: Internal,
    pub backups: Vec<Backup>,           // newest first: { dir, created (YYYYMMDD-HHMMSS UTC), bytes, holds }
}
pub enum Internal { TrimUi, ThisSystem, UnfinishedInstall, UnfinishedRestore, Unknown }
pub enum Task { Install { backup: bool, keep_card_bootable: bool }, Restore { dir: String } }
pub enum Progress { Task { restore: bool }, Step { index: usize, of: usize, label: String }, Fraction(f32), Done(Result<String, TaskError>) }
pub struct TaskError { pub message: String, pub hint: String, pub log: Option<String> }
```

The installer's thread first asks it to `follow` any task left running, then for an offer; the offer is asked for again each time Settings opens and after a task ends. Every task reports `Task` first: if this run of Specular didn't start one, it is one left running, and its progress screen opens. When a task ends, the progress screen offers what fits (restart, power off, or back), and B goes back after a failure.

The desktop build has a simulated installer that can be made to fail or to look as if a task was left running (see DEVELOPMENT.md).

### brick-install

The Brick's installer runs `/usr/sbin/brick-install` and reads its output.

- `brick-install offer` prints the offer as JSON. What the internal storage holds is one of `trimui`, `this-system`, `unfinished-install`, `unfinished-restore`; anything else is `Unknown`.
- `brick-install install [--backup] [--keep-card-bootable]`, `brick-install restore <dir>` and `brick-install follow` print one message per line:

```text
LOG-FILE <path>         where the full log is (first line, if there is a log)
TASK <install|restore>  which task this is
STEP <i>/<n> <label>    a step started
PROGRESS <0.00-1.00>    how far the current step is
DONE <message>          finished; the last line
FAIL <message>\t<hint>  failed: what happened, and what the user can do; the last line
```

If the program stops without a `DONE` or `FAIL`, the task crashed or was killed, and Specular reports that, pointing to the log on the SD card (`logs/`). When following, `brick-install` exits normally even if the task it follows was killed, so its exit status isn't used then; and a `follow` that prints nothing means no task is running. The log has every step, every command run with its exit status and output, and timings, flushed at every step, so a failure can be understood (and reproduced against image files) afterwards.
