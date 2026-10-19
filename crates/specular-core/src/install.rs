//! Installing the system onto a device's internal storage, putting back a backup of it, and
//! updating the system or going back to the previous version. The device's installer does the
//! work (the Brick's is brick-install); Specular offers it in Settings, asks first and shows its
//! progress (see app::install).

/// What the installer can do now. Fields an older installer doesn't report are empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Offer {
    /// Installing, if possible now.
    pub install: Option<InstallOffer>,
    /// Why installing isn't possible (with `install` None), e.g. the system already runs from the
    /// internal storage. Meant for developers: it may not be in plain words.
    pub why_not: Option<String>,
    /// What the internal storage holds; `Unknown` unless `install` is Some.
    pub internal: Internal,
    /// Complete backups of the internal storage, newest first.
    pub backups: Vec<Backup>,
    /// The running system's version.
    pub version: Option<String>,
    /// The other installed version, which can be started instead (`Task::Activate`): the one
    /// that ran before the last update or `Task::Activate`, so it may be the newer one.
    pub previous: Option<String>,
    /// Update files on the SD card, newest first.
    pub updates: Vec<UpdateFile>,
    /// On a system running from the internal storage: whether a restore can be prepared
    /// (`Task::PrepareRestore`), or why not. None where restores run directly (the SD card's
    /// system).
    pub restore: Option<Result<(), String>>,
    /// On the SD card's system: a restore prepared from the installed system, waiting to be run
    /// or cancelled.
    pub restore_pending: Option<PendingRestore>,
}

impl Offer {
    /// The backup a prepared restore puts back, if it is still on the SD card.
    pub fn pending_backup(&self) -> Option<&Backup> {
        let name = &self.restore_pending.as_ref()?.backup;
        self.backups.iter().find(|b| file_name(&b.dir) == name)
    }
}

/// The last part of a path, e.g. a backup's folder name.
pub fn file_name(path: &str) -> &str { path.rsplit('/').next().unwrap_or(path) }

/// What the internal storage holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Internal {
    /// TrimUI's system, as the device came.
    TrimUi,
    /// Our system, installed.
    ThisSystem,
    /// An install that was cut short (e.g. by a power cut): it can't start the device. The SD
    /// card still does, and the install can be run again.
    UnfinishedInstall,
    /// A restore that was cut short, likewise.
    UnfinishedRestore,
    /// Unreadable, or a layout we don't recognise.
    #[default]
    Unknown,
}

impl Internal {
    /// "TrimUI's system" and so on; None for `Unknown`.
    pub fn name(self) -> Option<&'static str> {
        Some(match self {
            Internal::TrimUi => "TrimUI's system",
            Internal::ThisSystem => "This system",
            Internal::UnfinishedInstall => "Unfinished install",
            Internal::UnfinishedRestore => "Unfinished restore",
            Internal::Unknown => return None,
        })
    }

    pub fn unfinished(self) -> bool { matches!(self, Internal::UnfinishedInstall | Internal::UnfinishedRestore) }
}

/// Installing is possible now.
#[derive(Clone, Debug, PartialEq)]
pub struct InstallOffer {
    /// The internal storage's size.
    pub bytes: u64,
    /// Whether the SD card has room for a backup of it: as much as it holds (how well it
    /// compresses isn't known beforehand).
    pub backup_possible: bool,
    /// The SD card's free space.
    pub free: u64,
}

/// A backup of the internal storage, on the SD card.
#[derive(Clone, Debug, PartialEq)]
pub struct Backup {
    /// Where it is (passed back in `Task::Restore`).
    pub dir: String,
    /// When it was made, as YYYYMMDD-HHMMSS (UTC).
    pub created: String,
    pub bytes: u64,
    /// What the internal storage held then (None: not recorded, in older backups).
    pub holds: Option<Internal>,
}

/// An update file on the SD card. The installer has checked its signature (a failure is in
/// `problem`) but not its contents, so installing it can still fail.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateFile {
    /// Where it is (passed back in `Task::Update`).
    pub file: String,
    /// The version it holds (None if it can't be read).
    pub version: Option<String>,
    /// Older than the running version: installing it goes back.
    pub older: bool,
    /// Why it can't be installed, if it can't.
    pub problem: Option<String>,
}

/// A restore prepared from the installed system (`Task::PrepareRestore`).
#[derive(Clone, Debug, PartialEq)]
pub struct PendingRestore {
    /// The backup's folder name, not its path (see `Offer::pending_backup`).
    pub backup: String,
    /// When it was prepared, as YYYYMMDD-HHMMSS (UTC).
    pub prepared: String,
}

/// What the installer can be asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    /// Install the running system onto the internal storage, backing that up first if `backup`.
    /// `keep_card_bootable` leaves the SD card able to start the device (for development); without
    /// it, the device starts from the internal storage from then on.
    Install { backup: bool, keep_card_bootable: bool },
    /// Put the backup in `dir` back onto the internal storage, from the SD card's system.
    Restore { dir: String },
    /// From the installed system: make the SD card start the device, with a note to restore the
    /// backup in `dir`; after a restart the SD card's system asks (`Offer::restore_pending`).
    PrepareRestore { dir: String },
    /// From the SD card's system: drop a prepared restore, so the internal storage starts the
    /// device again.
    CancelRestore,
    /// Install the update file `file`; its version starts at the next restart.
    Update { file: String },
    /// Make the installed `version` start at the next restart.
    Activate { version: String },
}

/// What a task does, for its progress screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind { Install, Restore, PrepareRestore, CancelRestore, Update, Activate }

impl Task {
    /// What the task does, without its arguments.
    pub fn kind(&self) -> TaskKind {
        match self {
            Task::Install { .. } => TaskKind::Install,
            Task::Restore { .. } => TaskKind::Restore,
            Task::PrepareRestore { .. } => TaskKind::PrepareRestore,
            Task::CancelRestore => TaskKind::CancelRestore,
            Task::Update { .. } => TaskKind::Update,
            Task::Activate { .. } => TaskKind::Activate,
        }
    }
}

/// How a task is going.
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// Which task this is; comes first. An installer may report a restore, preparing one and
    /// cancelling one all as `Restore` (brick-install does).
    Task(TaskKind),
    /// Step `index` (from 1) of `of` started.
    Step { index: usize, of: usize, label: String },
    /// How far the current step is, 0 to 1.
    Fraction(f32),
    /// Finished: what to tell the user, or what went wrong.
    Done(Result<String, TaskError>),
}

/// A failure the user can act on.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskError {
    pub message: String,
    /// What the user can do about it.
    pub hint: String,
    /// Where the installer's log is, if it wrote one.
    pub log: Option<String>,
}

/// A device's installer.
pub trait Installer: Send {
    /// What can be done now. Slow-ish (it looks at the disks): call it off the UI thread.
    fn offer(&self) -> Result<Offer, String>;
    /// Starts `task`; its progress goes to `progress`, from another thread, ending with `Done`.
    fn start(&self, task: Task, progress: Box<dyn FnMut(Progress) + Send>);
    /// If a task is running that an earlier run of Specular started (it was restarted, or
    /// crashed), reports its progress as `start` would, from its start; reports nothing if none
    /// runs. Returns at once.
    fn follow(&self, progress: Box<dyn FnMut(Progress) + Send>) { let _ = progress; }
}

/// "7.3 GB".
pub fn size(bytes: u64) -> String { format!("{:.1} GB", bytes as f64 / 1e9) }

/// A pretend installer, for the desktop build, so every installer screen can be seen. It runs
/// from the SD card, where installing is possible with room for a backup; there is one backup, a
/// previous version, and three update files: a newer one, an older one and one that can't be
/// installed. Each task takes a few seconds and succeeds, with brick-install's steps and closing
/// message. Environment variables change that:
/// - `SPECULAR_INSTALL_INSTALLED`: it runs from the internal storage, where Restore prepares the
///   SD card instead of restoring.
/// - `SPECULAR_INSTALL_PENDING`: on the SD card, a prepared restore is waiting (ignored with
///   `SPECULAR_INSTALL_INSTALLED`).
/// - `SPECULAR_INSTALL_FAILS`: tasks fail.
/// - `SPECULAR_INSTALL_RUNNING=<install|restore|update|activate>`: that task is running when
///   Specular starts (any other value: an install).
pub struct Simulated;

const SIMULATED_BACKUP: &str = "/mnt/SDCARD/backups/internal-20261014-120000";
const SIMULATED_VERSION: &str = "20261018-212347-b3a6d97";
const SIMULATED_PREVIOUS: &str = "20261012-090000-1a2b3c4";
const SIMULATED_UPDATE: &str = "/mnt/SDCARD/updates/crystaline-v0.2026.10.19.update";

fn env_set(name: &str) -> bool { std::env::var_os(name).is_some() }

impl Installer for Simulated {
    fn offer(&self) -> Result<Offer, String> {
        let installed = env_set("SPECULAR_INSTALL_INSTALLED");
        let backup = Backup { dir: SIMULATED_BACKUP.into(), created: "20261014-120000".into(), bytes: 7_818_182_656, holds: Some(Internal::TrimUi) };
        let update = |file: &str, version: Option<&str>, older, problem: Option<&str>| UpdateFile {
            file: file.into(), version: version.map(Into::into), older, problem: problem.map(Into::into),
        };
        Ok(Offer {
            install: (!installed).then_some(InstallOffer { bytes: backup.bytes, backup_possible: true, free: 20_000_000_000 }),
            why_not: installed.then(|| "the system isn't running from an SD card".into()),
            internal: if installed { Internal::Unknown } else { Internal::TrimUi },
            backups: vec![backup],
            version: Some(SIMULATED_VERSION.into()),
            previous: Some(SIMULATED_PREVIOUS.into()),
            updates: vec![
                update(SIMULATED_UPDATE, Some("20261019-101500-c4d5e6f"), false, None),
                update("/mnt/SDCARD/updates/crystaline-v0.2026.10.01.update", Some("20261001-080000-0a1b2c3"), true, None),
                update("/mnt/SDCARD/updates/copy.update", None, false, Some("/mnt/SDCARD/updates/copy.update isn't signed for this system")),
            ],
            restore: installed.then_some(Ok(())),
            restore_pending: (!installed && env_set("SPECULAR_INSTALL_PENDING"))
                .then(|| PendingRestore { backup: "internal-20261014-120000".into(), prepared: "20261018-120000".into() }),
        })
    }

    fn start(&self, task: Task, progress: Box<dyn FnMut(Progress) + Send>) {
        eprintln!("simulated: {task:?}");
        simulate(task, progress);
    }

    fn follow(&self, progress: Box<dyn FnMut(Progress) + Send>) {
        let Ok(kind) = std::env::var("SPECULAR_INSTALL_RUNNING") else { return };
        let task = match kind.as_str() {
            "restore" => Task::Restore { dir: SIMULATED_BACKUP.into() },
            "update" => Task::Update { file: SIMULATED_UPDATE.into() },
            "activate" => Task::Activate { version: SIMULATED_PREVIOUS.into() },
            _ => Task::Install { backup: false, keep_card_bootable: false },
        };
        simulate(task, progress);
    }
}

/// A pretend task, on a thread of its own, with brick-install's steps and closing message.
fn simulate(task: Task, mut progress: Box<dyn FnMut(Progress) + Send>) {
    let (steps, done): (&[&str], String) = match &task {
        Task::Install { .. } => (&["Checking", "Writing the start-up files", "Writing the system", "Checking the internal storage", "Finishing"],
                                 format!("Installed version {SIMULATED_VERSION}. Restart to start from the internal storage.")),
        Task::Restore { .. } => (&["Checking the backup", "Writing the internal storage", "Checking the internal storage"],
                                 "Restored the internal storage. Power off, remove the SD card, then turn the device on.".into()),
        Task::PrepareRestore { .. } => (&["Checking", "Making the SD card start the device"], "Restart to continue the restore from the SD card.".into()),
        Task::CancelRestore => (&["Cancelling the restore"], "The restore is cancelled. Restart to start from the internal storage.".into()),
        Task::Update { .. } => (&["Checking the update", "Copying the update", "Switching to the new version"], "The new version starts at the next restart.".into()),
        Task::Activate { version } => (&["Switching versions"], format!("Version {version} starts at the next restart.")),
    };
    std::thread::spawn(move || {
        progress(Progress::Task(task.kind()));
        for (i, label) in steps.iter().enumerate() {
            progress(Progress::Step { index: i + 1, of: steps.len(), label: label.to_string() });
            for f in 1..=10 { std::thread::sleep(std::time::Duration::from_millis(100)); progress(Progress::Fraction(f as f32 / 10.0)); }
        }
        progress(Progress::Done(if env_set("SPECULAR_INSTALL_FAILS") {
            Err(TaskError { message: "simulated failure".into(), hint: "Unset SPECULAR_INSTALL_FAILS.".into(), log: Some("/tmp/install.log".into()) })
        } else {
            Ok(done)
        }));
    });
}
