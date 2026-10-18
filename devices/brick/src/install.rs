//! The Brick's installer: the system's brick-install program (Crystaline OS's
//! userspace/brick-install), run with its output read line by line (its protocol is in its
//! report.rs).

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use specular_core::install::{Backup, InstallOffer, Installer, Internal, Offer, Progress, Task, TaskError};

const PROGRAM: &str = "/usr/sbin/brick-install";

pub struct BrickInstaller;

impl Installer for BrickInstaller {
    fn offer(&self) -> Result<Offer, String> {
        let out = Command::new(PROGRAM).arg("offer").output().map_err(|e| format!("{PROGRAM}: {e}"))?;
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("{PROGRAM} offer: {e}"))?;
        Ok(offer(&v))
    }

    fn start(&self, task: Task, progress: Box<dyn FnMut(Progress) + Send>) {
        let args: Vec<String> = match task {
            Task::Install { backup, keep_card_bootable } => std::iter::once("install".to_string())
                .chain(backup.then(|| "--backup".into())).chain(keep_card_bootable.then(|| "--keep-card-bootable".into())).collect(),
            Task::Restore { dir } => vec!["restore".into(), dir],
        };
        run(args, false, progress);
    }

    fn follow(&self, progress: Box<dyn FnMut(Progress) + Send>) { run(vec!["follow".into()], true, progress); }
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
            if let Some(p) = parse(&line, &log) {
                ended |= matches!(p, Progress::Done(_));
                progress(p);
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

/// One line of brick-install's output, as progress (None for lines that aren't).
fn parse(line: &str, log: &Option<String>) -> Option<Progress> {
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    Some(match word {
        "TASK" => Progress::Task { restore: rest == "restore" },
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

/// brick-install's offer (see its main.rs).
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
    Offer { install, why_not: v["why_not"].as_str().map(Into::into), internal: v["internal"].as_str().map_or(Internal::Unknown, internal), backups }
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

    #[test]
    fn reads_brick_installs_output() {
        let log = Some("/mnt/SDCARD/logs/install-x.log".to_string());
        assert_eq!(parse("TASK restore", &log), Some(Progress::Task { restore: true }));
        assert_eq!(parse("STEP 2/6 Backing up the internal storage", &log), Some(Progress::Step { index: 2, of: 6, label: "Backing up the internal storage".into() }));
        assert_eq!(parse("PROGRESS 0.42", &log), Some(Progress::Fraction(0.42)));
        assert_eq!(parse("DONE Installed.", &log), Some(Progress::Done(Ok("Installed.".into()))));
        assert_eq!(parse("FAIL the battery is at 20 %\tConnect the charger.", &log),
                   Some(Progress::Done(Err(TaskError { message: "the battery is at 20 %".into(), hint: "Connect the charger.".into(), log: log.clone() }))));
        assert_eq!(parse("something else", &log), None);
    }

    #[test]
    fn reads_its_offer() {
        let v = serde_json::json!({"backups":[{"bytes":7818182656u64,"created":"20261014-232447","dir":"/mnt/SDCARD/backups/internal-20261014-232447","holds":"trimui"},
                                              {"bytes":7818182656u64,"created":"20261014-120000","dir":"/mnt/SDCARD/backups/internal-20261014-120000","holds":null}],
                                   "install":{"backup":{"free":16027918336u64,"possible":true},"bytes":7818182656u64},"internal":"unfinished-install"});
        let o = offer(&v);
        assert_eq!(o.install, Some(InstallOffer { bytes: 7818182656, backup_possible: true, free: 16027918336 }));
        assert_eq!(o.internal, Internal::UnfinishedInstall);
        assert_eq!(o.backups.iter().map(|b| b.holds).collect::<Vec<_>>(), [Some(Internal::TrimUi), None]);
        assert_eq!(offer(&serde_json::json!({"install": null, "why_not": "the system isn't running from an SD card", "backups": []})).why_not.as_deref(),
                   Some("the system isn't running from an SD card"));
    }
}
