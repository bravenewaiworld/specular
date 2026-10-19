# Installer

Status: Implemented · Updated: 2026-10-19

Installing the system onto a device's internal storage and putting a backup of it back, from Settings → Internal storage; installing system updates and going back to the previous version, from Settings → Updates. An interrupted install can leave the device's internal storage unable to start, so the flow is built around never being interrupted by Specular, and around recognising when it was interrupted anyway.

## Decisions

- **The device's installer program does the work; Specular offers it, asks first and shows progress** (2026-10-14). The work is the OS's (on the Brick, `brick-install` in Crystaline OS); Specular only talks to it through the `Installer` interface, so a device without one simply has no such settings.
- **A backup of the internal storage to the SD card is offered first** (2026-10-14), when the card has room for as much as the internal storage holds (how well it compresses isn't known beforehand). It is on to start with, unless the internal storage holds an unfinished install or restore.
- **A developer option keeps the SD card bootable** (2026-10-14). Without it, the device starts from the internal storage from then on.
- **While a task runs, nothing may interrupt it** (2026-10-14). Its screen can't be left, and sleep and power off wait for it, saying so.
- **A task cut short is recognised** (2026-10-14). One still running (Specular crashed or was restarted) is shown again; an unfinished one (a power cut) shows in what the internal storage holds, and can be run again. The SD card still starts the device meanwhile.
- **Failures say what happened, what to do, and where the log is** (2026-10-14). That includes the installer refusing a task on a low battery. Why installing or preparing a restore isn't possible at all is meant for developers and shown only under Developer details.
- **Every update file on the SD card is listed: an older one with a warning, one that can't be installed with why** (2026-10-19). Going back to a known-good release must stay possible, and a file that doesn't show would leave the user guessing. Rejected: hiding older or unusable files.
- **An update or a switch to the previous version takes effect at the next start, so its progress screen ends on Restart** (2026-10-19). The version running before stays installed, to go back to.
- **On the installed system, Restore prepares the SD card; the card's system asks before restoring** (2026-10-19). The question (Restore, Not now, Cancel the restore) opens once per run, with the focus on Not now, when the first offer that reports the prepared restore arrives; after Not now it is in Settings. To ask at start-up, the installer is asked for an offer then, not only when Settings opens. Rejected: restoring by itself at start-up (a forgotten note would erase the internal storage unattended); asking each time Settings opens (nagging).
- **A task's kind is the one Specular started; a task followed from an earlier run takes the installer's name for it** (2026-10-19). `brick-install` names a restore, preparing one and cancelling one all `restore`, so a followed one is shown as a restore, ending on Power off. That is right for all three: the next start goes where the task left things, a restart is only quicker. Rejected: telling them apart from the offer (it isn't known yet while following at start-up).

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

pub struct Offer {                      // fields an older installer doesn't report are empty
    pub install: Option<InstallOffer>,  // { bytes, backup_possible, free }
    pub why_not: Option<String>,        // for developers
    pub internal: Internal,
    pub backups: Vec<Backup>,           // newest first: { dir, created (YYYYMMDD-HHMMSS UTC), bytes, holds }
    pub version: Option<String>,        // the running system's
    pub previous: Option<String>,       // the other installed version
    pub updates: Vec<UpdateFile>,       // newest first: { file, version, older, problem }
    pub restore: Option<Result<(), String>>,       // on the installed system: can a restore be prepared
    pub restore_pending: Option<PendingRestore>,   // on the SD card: { backup (folder name), prepared }
}
pub enum Internal { TrimUi, ThisSystem, UnfinishedInstall, UnfinishedRestore, Unknown }
pub enum Task {
    Install { backup: bool, keep_card_bootable: bool }, Restore { dir: String },
    PrepareRestore { dir: String }, CancelRestore, Update { file: String }, Activate { version: String },
}
pub enum TaskKind { Install, Restore, PrepareRestore, CancelRestore, Update, Activate }  // Task::kind()
pub enum Progress { Task(TaskKind), Step { index: usize, of: usize, label: String }, Fraction(f32), Done(Result<String, TaskError>) }
pub struct TaskError { pub message: String, pub hint: String, pub log: Option<String> }
```

The installer's thread first asks it to `follow` any task left running, then for an offer; the offer is asked for again each time Settings opens and after a task ends. Every task reports `Task` first: if this run of Specular didn't start one, it is one left running, and its progress screen opens. When a task ends, the progress screen offers what fits (Power off after a restore, else Restart), and B goes back.

Settings shows, from the last offer:

- **Updates**: the running version; Install update (the update files, then "Install update?"); Start previous version ("Start the previous version?").
- **Internal storage**: what an unfinished install or restore left; Continue the prepared restore; Install to internal storage; Restore internal storage (the backups, then "Restore the internal storage?", which prepares the restore when the offer says the system runs from the internal storage).

The desktop build has a simulated installer that covers every one of these screens (see DEVELOPMENT.md).

### brick-install

The Brick's installer runs `/usr/sbin/brick-install` and reads its output.

- `brick-install offer` prints the offer as JSON (its shape is in brick-install's `main.rs`). What the internal storage holds is one of `trimui`, `this-system`, `unfinished-install`, `unfinished-restore`; anything else is `Unknown`. `restore` is `{ "possible": true }` or `{ "why_not": … }`; `restore_pending` names the backup by its folder name, which Specular looks up among the backups (`Offer::pending_backup`). Specular logs the offer whole (`install: offer …`).
- The tasks are `install [--backup] [--keep-card-bootable]`, `restore <dir>` (restores from the SD card; prepares from the installed system), `restore --cancel`, `update <file>` and `activate <version>`. They, and `follow`, print one message per line:

```text
LOG-FILE <path>         where the full log is (first line, if there is a log)
TASK <name>             which task this is: install, restore, update or activate
STEP <i>/<n> <label>    a step started
PROGRESS <0.00-1.00>    how far the current step is
DONE <message>          finished; the last line
FAIL <message>\t<hint>  failed: what happened, and what the user can do; the last line
```

A line Specular doesn't understand (a task from a newer `brick-install`, say) is logged and skipped. If the program stops without a `DONE` or `FAIL`, the task crashed or was killed, and Specular reports that, pointing to the log on the SD card (`logs/`). When following, `brick-install` exits normally even if the task it follows was killed, so its exit status isn't used then; and a `follow` that prints nothing means no task is running. The log has every step, every command run with its exit status and output, and timings, flushed at every step, so a failure can be understood (and reproduced against image files) afterwards.

## Open questions

- Should the desktop build replay an offer logged on a device (`install: offer …`), as it replays wireless scenarios? It would show exactly what a user saw in Settings; so far the simulated offers have been enough.
