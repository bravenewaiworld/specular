//! The config folder's formats, and the steps that carry settings from each to the next. Each
//! format has a folder of its own, and an older one is left as it was, so an older Specular still
//! finds its settings and a faulty step loses nothing. A step builds its folder as `<folder>.tmp`
//! and renames it into place once complete, so a power cut never leaves half a folder: a `.tmp`
//! folder found at start-up is from a step cut off, and is discarded and the step run again.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::store::sync_dir;

/// One change of format: `run` makes the folder `to` (which doesn't exist yet: it is the `.tmp`
/// one) from the previous format's folder `from`.
struct Step { from: &'static str, to: &'static str, run: fn(from: &Path, to: &Path) -> io::Result<()> }

/// Oldest first, each step starting from the folder the one before made. Folders are relative to
/// the data root; the last step's is the current format.
const STEPS: &[Step] = &[
    Step { from: "configs", to: "config/v0", run: copy },
];

/// The current format's folder, relative to the data root.
pub const CONFIG: &str = STEPS[STEPS.len() - 1].to;

/// The current config folder under the data root `root`.
pub fn config_dir(root: &Path) -> PathBuf { root.join(CONFIG) }

/// The older formats' folders under the data root `root` that are still there once the current
/// one exists (they are kept: see `migrate`), oldest first. Specular no longer reads them.
pub fn old_folders(root: &Path) -> Vec<&'static str> {
    if !config_dir(root).is_dir() { return vec![]; }
    STEPS.iter().map(|s| s.from).filter(|f| root.join(f).is_dir()).collect()
}

/// Brings the config folder up to date: run at start-up, before settings are read. If it already
/// exists, older folders are ignored; otherwise the newest older one is migrated, step by step.
/// Returns false if a step failed (the error is logged). The caller must then write nothing to
/// the config folder until the next start, which tries again: a config folder made meanwhile
/// would look migrated, and the older settings would never be carried over.
pub fn migrate(root: &Path) -> bool {
    let first = match STEPS.iter().rposition(|s| root.join(s.to).is_dir()) {
        Some(i) => i + 1,
        None if root.join(STEPS[0].from).is_dir() => 0,
        None => return true, // a fresh card: the folder is made by the first save
    };
    STEPS[first..].iter().all(|step| run(root, step)
        .inspect_err(|e| eprintln!("settings: migrating {}/ to {}/ failed, so settings won't be saved until the next start, which tries again: {e}", step.from, step.to))
        .is_ok())
}

fn run(root: &Path, step: &Step) -> io::Result<()> {
    let to = root.join(step.to);
    let tmp = to.with_extension("tmp");
    if tmp.exists() {
        eprintln!("settings: {} is from a migration that was cut off; starting it again", tmp.display());
        fs::remove_dir_all(&tmp)?;
    }
    eprintln!("settings: migrating {}/ to {}/ (the old folder is kept as it is)", step.from, step.to);
    fs::create_dir_all(tmp.parent().unwrap())?;
    (step.run)(&root.join(step.from), &tmp)?;
    fs::rename(&tmp, &to)?;
    // Makes the rename durable, and any folders made above `to` (`config/`): each folder from
    // its parent up to the data root.
    for dir in to.ancestors().skip(1).take_while(|d| d.starts_with(root)) { sync_dir(dir)?; }
    eprintln!("settings: migrated {}/ to {}/", step.from, step.to);
    Ok(())
}

/// Copies a file, or a folder with everything in it, each flushed to the card. The step from
/// `configs/` is just this, as `configs/` had v0's layout (less the schemas and `cores/`): a
/// damaged file is copied too, and reading it then deals with it as with any other. What can't be
/// read is logged and skipped, and so are symbolic links (following one could loop forever) and
/// any `README.txt` (the guide Specular writes into an old folder, which says it is no longer
/// read: wrong in the new one); failing to write ends the step.
fn copy(from: &Path, to: &Path) -> io::Result<()> {
    if from.file_name().is_some_and(|n| n == crate::guides::README) { return Ok(()); }
    let skip = |e: &io::Error| eprintln!("settings: can't read {}, so it is not migrated: {e}", from.display());
    let Ok(kind) = fs::symlink_metadata(from).inspect_err(skip).map(|m| m.file_type()) else { return Ok(()) };
    if kind.is_dir() {
        let Ok(entries) = fs::read_dir(from).inspect_err(skip) else { return Ok(()) };
        fs::create_dir(to)?;
        for entry in entries.filter_map(|e| e.inspect_err(skip).ok()) { copy(&entry.path(), &to.join(entry.file_name()))?; }
        return sync_dir(to);
    }
    if !kind.is_file() { eprintln!("settings: {} is a link or not a file, so it is not migrated", from.display()); return Ok(()); }
    let Ok(bytes) = fs::read(from).inspect_err(skip) else { return Ok(()) };
    let mut f = File::create(to)?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    eprintln!("settings: migrated {}", from.display());
    Ok(())
}

#[test]
fn each_step_starts_from_the_folder_the_one_before_made() {
    assert!(STEPS.windows(2).all(|w| w[0].to == w[1].from));
}
