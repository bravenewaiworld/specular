//! The Brick's installer: the system's brick-install program (Crystaline OS's
//! userspace/brick-install), run with its output read line by line (its protocol is in its
//! report.rs, its offer in its main.rs).

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use specular_core::install::{Backup, InstallOffer, Installer, Internal, Offer, PendingRestore, Progress, Task, TaskError, TaskKind, UpdateFile};

const PROGRAM: &str = "/usr/sbin/brick-install";

pub struct BrickInstaller;

impl Installer for BrickInstaller {
    fn offer(&self) -> Result<Offer, String> {
        let out = Command::new(PROGRAM).arg("offer").output().map_err(|e| format!("{PROGRAM}: {e}"))?;
        // Logged whole, so what Settings showed can be reproduced (e.g. in the desktop build).
        eprintln!("install: offer {}", String::from_utf8_lossy(&out.stdout).trim());
        let v: serde_json::Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| format!("{PROGRAM} offer: {e} ({}; {})", out.status, String::from_utf8_lossy(&out.stderr).trim()))?;
        Ok(offer(&v))
    }

    fn start(&self, task: Task, progress: Box<dyn FnMut(Progress) + Send>) { run(args(task), false, progress); }

    fn follow(&self, progress: Box<dyn FnMut(Progress) + Send>) { run(vec!["follow".into()], true, progress); }
}

/// brick-install's command line for `task`. It decides by itself whether `restore` restores or
/// prepares a restore (by where the system runs from).
fn args(task: Task) -> Vec<String> {
    match task {
        Task::Install { backup, keep_card_bootable } => std::iter::once("install".to_string())
            .chain(backup.then(|| "--backup".into())).chain(keep_card_bootable.then(|| "--keep-card-bootable".into())).collect(),
        Task::Restore { dir } | Task::PrepareRestore { dir } => vec!["restore".into(), dir],
        Task::CancelRestore => vec!["restore".into(), "--cancel".into()],
        Task::Update { file } => vec!["update".into(), file],
        Task::Activate { version } => vec!["activate".into(), version],
    }
}

/// Runs brick-install with `args`, reporting what it prints to `progress`, from a thread of its
/// own. With `may_say_nothing` (`follow`: no task running), reports nothing if it prints nothing.
fn run(args: Vec<String>, may_say_nothing: bool, mut progress: Box<dyn FnMut(Progress) + Send>) {
    eprintln!("install: {PROGRAM} {}", args.join(" "));
    let child = Command::new(PROGRAM).args(&args).stdout(Stdio::piped()).spawn();
    std::thread::spawn(move || {
        let mut child = match child {
            Ok(c) => c,
            Err(e) => return progress(Progress::Done(Err(TaskError { message: format!("{PROGRAM}: {e}"), hint: "The system is incomplete: write a fresh SD card.".into(), log: None }))),
        };
        let (mut log, mut said, mut ended) = (None, false, false);
        for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
            said = true;
            if let Some(path) = line.strip_prefix("LOG-FILE ") { log = Some(path.to_string()); continue; }
            match parse(&line, &log) {
                Some(p) => {
                    ended |= matches!(p, Progress::Done(_));
                    progress(p);
                }
                None => eprintln!("install: not understood: {line}"),
            }
        }
        let status = child.wait();
        // It stopped without saying how it ended: the task crashed or was killed. (When following,
        // brick-install exits normally even if the task it follows was killed, so its exit status
        // says nothing then.)
        if !ended && (said || !may_say_nothing) {
            let how = match status { Ok(s) if s.success() => String::new(), Ok(s) => format!(" ({s})"), Err(e) => format!(" ({e})") };
            progress(Progress::Done(Err(TaskError { message: format!("the installer stopped unexpectedly{how}"), hint: "Details are in the log on the SD card (logs/).".into(), log })));
        }
    });
}

/// One line of brick-install's output, as progress (None for lines that aren't, including a task
/// this version of Specular doesn't know).
fn parse(line: &str, log: &Option<String>) -> Option<Progress> {
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    Some(match word {
        // `restore` also names preparing and cancelling one: brick-install doesn't tell them apart.
        "TASK" => Progress::Task(match rest {
            "install" => TaskKind::Install,
            "restore" => TaskKind::Restore,
            "update" => TaskKind::Update,
            "activate" => TaskKind::Activate,
            _ => return None,
        }),
        "STEP" => {
            let (counts, label) = rest.split_once(' ')?;
            let (index, of) = counts.split_once('/')?;
            Progress::Step { index: index.parse().ok()?, of: of.parse().ok()?, label: label.into() }
        }
        "PROGRESS" => Progress::Fraction(rest.parse().ok()?),
        "DONE" => Progress::Done(Ok(rest.into())),
        "FAIL" => {
            let (message, hint) = rest.split_once('\t').unwrap_or((rest, ""));
            Progress::Done(Err(TaskError { message: message.into(), hint: hint.into(), log: log.clone() }))
        }
        _ => return None,
    })
}

/// brick-install's offer (see its main.rs). Fields an older brick-install doesn't print are left
/// empty.
fn offer(v: &serde_json::Value) -> Offer {
    let install = v["install"].as_object().map(|i| InstallOffer {
        bytes: i["bytes"].as_u64().unwrap_or(0),
        backup_possible: i["backup"]["possible"].as_bool().unwrap_or(false),
        free: i["backup"]["free"].as_u64().unwrap_or(0),
    });
    let backups = v["backups"].as_array().into_iter().flatten().filter_map(|b| Some(Backup {
        dir: b["dir"].as_str()?.into(), created: b["created"].as_str()?.into(), bytes: b["bytes"].as_u64()?,
        holds: b["holds"].as_str().map(internal),
    })).collect();
    let updates = v["updates"].as_array().into_iter().flatten().filter_map(|u| Some(UpdateFile {
        file: u["file"].as_str()?.into(), version: u["version"].as_str().map(Into::into),
        older: u["older"].as_bool().unwrap_or(false), problem: u["problem"].as_str().map(Into::into),
    })).collect();
    // { "possible": true } or { "why_not": "…" }
    let restore = v["restore"].is_object().then(|| v["restore"]["why_not"].as_str().map_or(Ok(()), |why| Err(why.into())));
    let p = &v["restore_pending"];
    let restore_pending = p["backup"].as_str().map(|b| PendingRestore { backup: b.into(), prepared: p["prepared"].as_str().unwrap_or_default().into() });
    Offer {
        install, why_not: v["why_not"].as_str().map(Into::into), internal: v["internal"].as_str().map_or(Internal::Unknown, internal), backups,
        version: v["running"]["version"].as_str().map(Into::into), previous: v["previous"]["version"].as_str().map(Into::into),
        updates, restore, restore_pending,
    }
}

/// What the internal storage holds, as brick-install names it (brick-install's `install::holds`).
fn internal(name: &str) -> Internal {
    match name {
        "trimui" => Internal::TrimUi,
        "this-system" => Internal::ThisSystem,
        "unfinished-install" => Internal::UnfinishedInstall,
        "unfinished-restore" => Internal::UnfinishedRestore,
        _ => Internal::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines as brick-install prints them (its report.rs, and the tasks in its install.rs,
    // backup.rs, update.rs and slots.rs; checked in its tools/system/test-installer.sh).
    #[test]
    fn reads_brick_installs_output() {
        let log = Some("/mnt/SDCARD/logs/install-x.log".to_string());
        for (line, kind) in [("TASK install", TaskKind::Install), ("TASK restore", TaskKind::Restore), ("TASK update", TaskKind::Update), ("TASK activate", TaskKind::Activate)] {
            assert_eq!(parse(line, &log), Some(Progress::Task(kind)), "{line}");
        }
        assert_eq!(parse("TASK defragment", &log), None); // a task from a newer brick-install
        assert_eq!(parse("STEP 2/6 Backing up the internal storage", &log), Some(Progress::Step { index: 2, of: 6, label: "Backing up the internal storage".into() }));
        assert_eq!(parse("STEP 3/3 Switching to version 20261018-212347-b3a6d97", &log),
                   Some(Progress::Step { index: 3, of: 3, label: "Switching to version 20261018-212347-b3a6d97".into() }));
        assert_eq!(parse("PROGRESS 0.42", &log), Some(Progress::Fraction(0.42)));
        assert_eq!(parse("DONE Version 20261018-212347-b3a6d97 starts at the next restart.", &log),
                   Some(Progress::Done(Ok("Version 20261018-212347-b3a6d97 starts at the next restart.".into()))));
        assert_eq!(parse("DONE Restart to continue the restore from the SD card.", &log), Some(Progress::Done(Ok("Restart to continue the restore from the SD card.".into()))));
        assert_eq!(parse("FAIL the battery is at 30 %\tConnect the charger, or charge to at least 50 %, then try again.", &log),
                   Some(Progress::Done(Err(TaskError { message: "the battery is at 30 %".into(), hint: "Connect the charger, or charge to at least 50 %, then try again.".into(), log: log.clone() }))));
        assert_eq!(parse("something else", &log), None);
    }

    #[test]
    fn runs_each_task_with_its_arguments() {
        let dir = "/mnt/SDCARD/backups/internal-20261014-101010".to_string();
        assert_eq!(args(Task::Install { backup: true, keep_card_bootable: false }), ["install", "--backup"]);
        assert_eq!(args(Task::Restore { dir: dir.clone() }), ["restore", dir.as_str()]);
        assert_eq!(args(Task::PrepareRestore { dir: dir.clone() }), ["restore", dir.as_str()]);
        assert_eq!(args(Task::CancelRestore), ["restore", "--cancel"]);
        assert_eq!(args(Task::Update { file: "/mnt/SDCARD/updates/a.update".into() }), ["update", "/mnt/SDCARD/updates/a.update"]);
        assert_eq!(args(Task::Activate { version: "20261012-090000-1a2b3c4".into() }), ["activate", "20261012-090000-1a2b3c4"]);
    }

    #[test]
    fn reads_its_offer() {
        let v = serde_json::json!({"backups":[{"bytes":7818182656u64,"created":"20261014-232447","dir":"/mnt/SDCARD/backups/internal-20261014-232447","holds":"trimui"},
                                              {"bytes":7818182656u64,"created":"20261014-120000","dir":"/mnt/SDCARD/backups/internal-20261014-120000","holds":null}],
                                   "install":{"backup":{"free":16027918336u64,"possible":true},"bytes":7818182656u64},"internal":"unfinished-install",
                                   "running":{"version":"20261018-212347-b3a6d97","disk":"card","slot":"boot_a"},
                                   "previous":{"version":"20261012-090000-1a2b3c4","slot":"boot_b"},
                                   "updates":[{"file":"/mnt/SDCARD/updates/good.update","older":false,"problem":null,"version":"20261019-101500-c4d5e6f"},
                                              {"file":"/mnt/SDCARD/updates/older.update","older":true,"problem":null,"version":"20000101-000000-0000000"},
                                              {"file":"/mnt/SDCARD/updates/other-key.update","older":false,"problem":"/mnt/SDCARD/updates/other-key.update isn't signed for this system","version":null}],
                                   "restore":null,
                                   "restore_pending":{"backup":"internal-20261014-232447","bytes":7818182656u64,"prepared":"20261018-120000"}});
        let o = offer(&v);
        assert_eq!(o.install, Some(InstallOffer { bytes: 7818182656, backup_possible: true, free: 16027918336 }));
        assert_eq!(o.internal, Internal::UnfinishedInstall);
        assert_eq!(o.backups.iter().map(|b| b.holds).collect::<Vec<_>>(), [Some(Internal::TrimUi), None]);
        assert_eq!(o.version.as_deref(), Some("20261018-212347-b3a6d97"));
        assert_eq!(o.previous.as_deref(), Some("20261012-090000-1a2b3c4"));
        assert_eq!(o.updates[1], UpdateFile { file: "/mnt/SDCARD/updates/older.update".into(), version: Some("20000101-000000-0000000".into()), older: true, problem: None });
        assert_eq!(o.updates[2].version, None);
        assert_eq!(o.updates[2].problem.as_deref(), Some("/mnt/SDCARD/updates/other-key.update isn't signed for this system"));
        assert_eq!(o.restore, None);
        assert_eq!(o.restore_pending, Some(PendingRestore { backup: "internal-20261014-232447".into(), prepared: "20261018-120000".into() }));
        assert_eq!(o.pending_backup().map(|b| b.created.as_str()), Some("20261014-232447"));

        // On the installed system.
        let installed = offer(&serde_json::json!({"install":null,"why_not":"the system isn't running from an SD card","backups":[],
                                                  "running":{"version":"20261018-212347-b3a6d97","disk":"internal","slot":"boot_b"},"previous":null,"updates":[],
                                                  "restore":{"why_not":"the SD card's boot slot \"boot_a\" holds no system"},"restore_pending":null}));
        assert_eq!(installed.why_not.as_deref(), Some("the system isn't running from an SD card"));
        assert_eq!(installed.restore, Some(Err("the SD card's boot slot \"boot_a\" holds no system".into())));
        assert_eq!(installed.previous, None);
        assert_eq!(offer(&serde_json::json!({"restore":{"possible":true}})).restore, Some(Ok(())));

        // An older brick-install, without the newer fields.
        let older = offer(&serde_json::json!({"install": null, "why_not": "the system isn't running from an SD card", "backups": []}));
        assert_eq!(older, Offer { why_not: Some("the system isn't running from an SD card".into()), ..Offer::default() });
    }
}
