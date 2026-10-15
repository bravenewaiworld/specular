//! Installing the system onto a device's internal storage, and putting back a backup of it. The
//! device's installer does the work (the Brick's is brick-install); Specular offers it in
//! Settings, asks first and shows its progress (see app::install).

/// What the installer can do now.
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
}

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

/// What the installer can be asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    /// Install the running system onto the internal storage, backing that up first if `backup`.
    /// `keep_card_bootable` leaves the SD card able to start the device (for development); without
    /// it, the device starts from the internal storage from then on.
    Install { backup: bool, keep_card_bootable: bool },
    /// Put the backup in `dir` back onto the internal storage.
    Restore { dir: String },
}

/// How a task is going.
#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// Which task this is (`restore`: a restore, else an install); comes first.
    Task { restore: bool },
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

/// A pretend installer, for the desktop build: installing is possible, with room for a backup,
/// there is one backup, and each task takes a few seconds and succeeds. With
/// $SPECULAR_INSTALL_FAILS set, tasks fail instead; with $SPECULAR_INSTALL_RUNNING set, an install
/// is running when Specular starts.
pub struct Simulated;

impl Installer for Simulated {
    fn offer(&self) -> Result<Offer, String> {
        let backup = Backup { dir: "/mnt/SDCARD/backups/internal-20261014-120000".into(), created: "20261014-120000".into(), bytes: 7_818_182_656, holds: Some(Internal::TrimUi) };
        Ok(Offer { install: Some(InstallOffer { bytes: backup.bytes, backup_possible: true, free: 20_000_000_000 }), why_not: None,
                   internal: Internal::TrimUi, backups: vec![backup] })
    }

    fn start(&self, task: Task, progress: Box<dyn FnMut(Progress) + Send>) {
        eprintln!("simulated: {task:?}");
        simulate(matches!(task, Task::Restore { .. }), progress);
    }

    fn follow(&self, progress: Box<dyn FnMut(Progress) + Send>) {
        if std::env::var_os("SPECULAR_INSTALL_RUNNING").is_some() { simulate(false, progress); }
    }
}

/// A pretend task, on a thread of its own.
fn simulate(restore: bool, mut progress: Box<dyn FnMut(Progress) + Send>) {
    std::thread::spawn(move || {
        progress(Progress::Task { restore });
        let steps = ["Checking", "Writing the start-up files", "Writing the system", "Checking the internal storage", "Finishing"];
        for (i, label) in steps.iter().enumerate() {
            progress(Progress::Step { index: i + 1, of: steps.len(), label: label.to_string() });
            for f in 1..=10 { std::thread::sleep(std::time::Duration::from_millis(100)); progress(Progress::Fraction(f as f32 / 10.0)); }
        }
        progress(Progress::Done(if std::env::var_os("SPECULAR_INSTALL_FAILS").is_some() {
            Err(TaskError { message: "simulated failure".into(), hint: "Unset SPECULAR_INSTALL_FAILS.".into(), log: Some("/tmp/install.log".into()) })
        } else {
            Ok("Done (simulated).".into())
        }));
    });
}
