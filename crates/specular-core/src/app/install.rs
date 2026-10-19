//! The installer in Settings: installing onto the internal storage and restoring a backup of it,
//! installing updates and starting the previous version. The rows that offer them (from the
//! installer's `Offer`, asked for at start-up and each time Settings opens), the screens that ask
//! first, and the screen that follows the installer's progress.
//!
//! While a task runs its screen can't be left, and the device neither sleeps nor powers off: an
//! interrupted install leaves the internal storage unable to start (an SD card still starts the
//! device). A task that an earlier run of Specular started (before a crash) is shown the same way
//! once the installer reports it (`Installer::follow`).
//!
//! A restore prepared from the installed system continues on the SD card's system, which asks
//! about it once per run, when the first offer that reports it arrives (and from Settings after
//! that). It never restores by itself.

use std::collections::HashMap;

use super::*;
use crate::install::{Internal, Offer, Progress, Task, TaskError, TaskKind, size};

/// What the app knows of the installer.
#[derive(Default)]
pub struct InstallState {
    /// The last offer (None until the installer answers; Err: it couldn't tell).
    pub offer: Option<Result<Offer, String>>,
    /// The task started last, while its screen is open.
    pub task: Option<TaskView>,
    /// Whether this run has asked about a prepared restore yet.
    pub asked_pending: bool,
}

/// A task's progress, as its screen shows it.
pub struct TaskView {
    pub kind: TaskKind,
    /// The steps started so far, and how many there are.
    pub steps: Vec<String>,
    pub of: usize,
    /// How far the current (last) step is.
    pub fraction: f32,
    pub end: Option<Result<String, TaskError>>,
}

impl TaskView {
    fn running(&self) -> bool { self.end.is_none() }
}

/// A command for the device's installer (`Effect::Installer`).
#[derive(Clone, Debug, PartialEq)]
pub enum InstallCmd {
    /// Ask what can be done now; the answer comes back as `Msg::InstallOffer`.
    Offer,
    /// Start a task; its progress comes back as `Msg::InstallProgress`.
    Start(Task),
}

pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) {
    screens.insert(screens::INSTALL, Box::new(InstallScreen));
    screens.insert(screens::RESTORE, Box::new(RestoreScreen));
    screens.insert(screens::UPDATES, Box::new(UpdatesScreen));
    screens.insert(screens::CONFIRM, Box::new(ConfirmScreen));
    screens.insert(screens::RESTORE_PENDING, Box::new(PendingScreen));
    screens.insert(screens::TASK, Box::new(TaskScreen));
}

impl App {
    /// Says whether the device has an installer (none until told). Call before the first message.
    pub fn has_installer(&mut self, yes: bool) { self.ctx.install = yes.then(InstallState::default); }

    /// Whether an installer task is running: then nothing may interrupt it.
    pub fn task_running(&self) -> bool {
        self.ctx.install.as_ref().and_then(|i| i.task.as_ref()).is_some_and(TaskView::running)
    }

    /// What the power button says while a task runs.
    pub(super) fn task_wait_notice(&self) -> String {
        let kind = self.ctx.install.as_ref().and_then(|i| i.task.as_ref()).map_or(TaskKind::Install, |t| t.kind);
        format!("Please wait until {} has finished", words(kind).3)
    }

    pub(super) fn install_offer(&mut self, offer: Result<Offer, String>) -> Vec<Effect> {
        let playing = self.playing.is_some();
        let Some(state) = &mut self.ctx.install else { return vec![] };
        let pending = offer.as_ref().is_ok_and(|o| o.restore_pending.is_some());
        let ask = pending && !state.asked_pending && state.task.is_none() && !playing;
        state.asked_pending |= ask;
        state.offer = Some(offer);
        self.changed();
        if ask { return self.go(|nav| Some(nav.push(focus_on(ScreenState::new(screens::RESTORE_PENDING, None), "later")))); }
        vec![Effect::Redraw(None)]
    }

    pub(super) fn install_progress(&mut self, p: Progress) -> Vec<Effect> {
        let Some(state) = &mut self.ctx.install else { return vec![] };
        if let Progress::Task(kind) = p {
            // Every task reports `Task` first. If this run started none, it is one left running by
            // an earlier run (`Installer::follow`): show it.
            return if state.task.is_some() { vec![] } else { self.show_task(kind) };
        }
        let Some(task) = state.task.as_mut() else { return vec![] };
        let done = matches!(p, Progress::Done(_));
        match p {
            Progress::Task(_) => {} // handled above
            Progress::Step { index, of, label } => { task.steps.truncate(index.saturating_sub(1)); task.steps.push(label); task.of = of; task.fraction = 0.0; }
            Progress::Fraction(f) => task.fraction = f,
            Progress::Done(end) => task.end = Some(end),
        }
        self.changed();
        let mut e = vec![Effect::Redraw(None)];
        // What can be done has changed (e.g. a backup was made).
        if done { e.push(Effect::Installer(InstallCmd::Offer)); }
        e
    }

    /// Starts `task`, with its progress screen over the screen that asked.
    fn start_task(&mut self, task: Task) -> Vec<Effect> {
        let kind = task.kind();
        let mut e = vec![Effect::Installer(InstallCmd::Start(task))];
        e.extend(self.show_task(kind));
        e
    }

    /// Opens a task's progress screen (one this run started, or one left running by an earlier
    /// run), over the screen shown.
    fn show_task(&mut self, kind: TaskKind) -> Vec<Effect> {
        let Some(state) = &mut self.ctx.install else { return vec![] };
        state.task = Some(TaskView { kind, steps: vec![], of: 0, fraction: 0.0, end: None });
        self.go(|nav| Some(nav.push(ScreenState::new(screens::TASK, None))))
    }

    /// A press on one of the installer's screens whose presses the app handles itself (they start
    /// tasks); None for the others.
    pub(super) fn install_press(&mut self, id: &ScreenId, button: Button, row: Option<&str>) -> Option<Vec<Effect>> {
        Some(match id {
            id if *id == screens::INSTALL => self.install_screen(button, row),
            id if *id == screens::CONFIRM => self.confirm_screen(button, row),
            id if *id == screens::RESTORE_PENDING => self.pending_screen(button, row),
            id if *id == screens::TASK => self.task_screen(button, row),
            _ => return None,
        })
    }

    /// A press on the install screen: A switches its options, installs, or cancels; B cancels.
    fn install_screen(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        let state = self.nav.top_mut();
        let (backup, keep) = InstallScreen::options(state);
        match (button, row) {
            (Button::B, _) | (Button::A, Some("cancel")) => self.go(|nav| nav.pop()),
            (Button::A, Some("backup")) => { state.params = Some(InstallScreen::param(!backup, keep)); self.changed(); vec![Effect::Redraw(None)] }
            (Button::A, Some("keep")) => { state.params = Some(InstallScreen::param(backup, !keep)); self.changed(); vec![Effect::Redraw(None)] }
            (Button::A, Some("install")) => self.start_task(Task::Install { backup, keep_card_bootable: keep }),
            _ => vec![],
        }
    }

    /// A press on a confirm screen: A starts its task or cancels; B cancels.
    fn confirm_screen(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        match (button, row) {
            (Button::B, _) | (Button::A, Some("cancel")) => self.go(|nav| nav.pop()),
            (Button::A, Some("go")) => match self.nav.top().params.as_deref().and_then(|p| confirmed_task(&self.ctx, p)) {
                Some(task) => self.start_task(task),
                None => vec![],
            },
            _ => vec![],
        }
    }

    /// A press on the prepared restore's screen: A restores, cancels the restore, or leaves it for
    /// now; B leaves it for now.
    fn pending_screen(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        match (button, row) {
            (Button::B, _) | (Button::A, Some("later")) => self.go(|nav| nav.pop()),
            (Button::A, Some("restore")) => match offer(&self.ctx).and_then(Offer::pending_backup) {
                Some(b) => { let dir = b.dir.clone(); self.start_task(Task::Restore { dir }) }
                None => vec![],
            },
            (Button::A, Some("drop")) => self.start_task(Task::CancelRestore),
            _ => vec![],
        }
    }

    /// A press on the progress screen: nothing while the task runs; then A does what the screen
    /// offers (restart, power off, or back), and B goes back after a failure.
    fn task_screen(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        if self.task_running() { return vec![]; }
        match (button, row) {
            (Button::A, Some("restart")) => vec![Effect::Restart],
            (Button::A, Some("off")) => vec![Effect::PowerOff],
            (Button::B, _) | (Button::A, Some("back")) => {
                if let Some(state) = &mut self.ctx.install { state.task = None; }
                self.go(|nav| nav.pop())
            }
            _ => vec![],
        }
    }
}

/// Settings' Updates and Internal storage sections, from the last offer. Why something isn't
/// offered is shown only with developer details on.
pub(super) fn settings_rows(ctx: &Ctx) -> Vec<Row> {
    let Some(state) = &ctx.install else { return vec![] };
    let developer = ctx.visibility() == Visibility::Developer;
    let (mut updates, mut storage) = (vec![], vec![]);
    match &state.offer {
        Some(Ok(o)) => {
            if let Some(v) = &o.version { updates.push(Row::new("install:version", "Version", RowKind::Note { detail: Some(v.clone()) })); }
            if !o.updates.is_empty() { updates.push(action("install:update", "Install update")); }
            if o.previous.is_some() { updates.push(action("install:previous", "Start previous version")); }

            match o.internal {
                Internal::UnfinishedInstall => storage.push(Row::note("install:unfinished", "The last install didn't finish, so the internal storage can't start the device. Install again, or restore a backup.")),
                Internal::UnfinishedRestore => storage.push(Row::note("install:unfinished", "The last restore didn't finish, so the internal storage can't start the device. Restore again.")),
                _ => {}
            }
            if o.restore_pending.is_some() { storage.push(action("install:pending", "Continue the prepared restore")); }
            if o.install.is_some() { storage.push(action("install:open", "Install to internal storage")); }
            if !o.backups.is_empty() && !matches!(o.restore, Some(Err(_))) { storage.push(action("install:restore", "Restore internal storage")); }
            if developer {
                if let (None, Some(why)) = (&o.install, &o.why_not) { storage.push(Row::note("install:why", format!("Install: {why}"))); }
                if let Some(Err(why)) = &o.restore { storage.push(Row::note("install:why-restore", format!("Restore: {why}"))); }
            }
        }
        Some(Err(e)) if developer => storage.push(Row::note("install:why", format!("Installer: {e}"))),
        _ => {}
    }
    let mut rows = vec![];
    for (id, title, section) in [("header:updates", "Updates", updates), ("header:install", "Internal storage", storage)] {
        if section.is_empty() { continue; }
        rows.push(Row::new(id, title, RowKind::Header));
        rows.extend(section);
    }
    rows
}

pub(super) fn settings_press(ctx: &Ctx, row: &Row, p: Press) -> Option<Outcome> {
    if p.button != Button::A { return None; }
    let push = |state| Some(Outcome::Push(state));
    match row.id.0.as_str() {
        "install:open" => {
            // Off to start with when the internal storage holds an unfinished install or restore
            // (nothing worth keeping); it can still be turned on.
            let backup = offer(ctx).is_some_and(|o| !o.internal.unfinished() && o.install.as_ref().is_some_and(|i| i.backup_possible));
            push(focus_on(ScreenState::new(screens::INSTALL, Some(InstallScreen::param(backup, false))), "cancel"))
        }
        "install:restore" => push(ScreenState::new(screens::RESTORE, None)),
        "install:update" => push(ScreenState::new(screens::UPDATES, None)),
        "install:previous" => push(confirm(format!("activate:{}", offer(ctx)?.previous.as_ref()?))),
        "install:pending" => push(focus_on(ScreenState::new(screens::RESTORE_PENDING, None), "later")),
        _ => None,
    }
}

/// `state` with the focus starting on its row `row`. A screen that asks before erasing or
/// changing something starts on the choice that does nothing.
fn focus_on(mut state: ScreenState, row: &str) -> ScreenState {
    state.focus = Some(crate::model::RowId(row.into()));
    state
}

fn offer(ctx: &Ctx) -> Option<&Offer> { ctx.install.as_ref()?.offer.as_ref()?.as_ref().ok() }

/// "Install to internal storage?": what it erases, whether to back up first (offered if the card
/// has room; on to start with, unless the internal storage holds an unfinished install or
/// restore), and (for developers) whether the card stays bootable. Its params hold the options:
/// "backup=<0|1>,keep=<0|1>".
struct InstallScreen;

impl InstallScreen {
    fn param(backup: bool, keep: bool) -> String { format!("backup={},keep={}", backup as u8, keep as u8) }
    fn options(state: &ScreenState) -> (bool, bool) {
        let p = state.params.as_deref().unwrap_or_default();
        (p.contains("backup=1"), p.contains("keep=1"))
    }
}

impl Screen for InstallScreen {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let (backup, keep) = Self::options(state);
        let erase = match offer(ctx).map_or(Internal::Unknown, |o| o.internal) {
            Internal::TrimUi => "Erases everything on the internal storage, including TrimUI's original system.",
            Internal::ThisSystem => "Erases everything on the internal storage, including the system installed on it.",
            Internal::UnfinishedInstall => "Erases everything on the internal storage, including what the unfinished install left.",
            Internal::UnfinishedRestore => "Erases everything on the internal storage, including what the unfinished restore left.",
            Internal::Unknown => "Erases everything on the internal storage.",
        };
        let mut rows = vec![Row::note("note:erase", erase), Row::note("note:card", "Games and saves stay on the SD card.")];
        match offer(ctx).and_then(|o| o.install.as_ref()) {
            Some(i) if i.backup_possible => {
                rows.push(Row::new("backup", "Back up the internal storage first", RowKind::Toggle { on: backup }));
                if !backup { rows.push(Row::note("note:backup", "Without a backup, it can't be put back.")); }
            }
            Some(i) => rows.push(Row::note("note:backup", format!("No room for a backup ({} needed, {} free on the SD card): it can't be put back.", size(i.bytes), size(i.free)))),
            None => {}
        }
        if ctx.visibility() == Visibility::Developer {
            rows.push(Row::new("keep", "Keep the SD card bootable", RowKind::Toggle { on: keep }));
        }
        rows.extend([action("install", "Install"), action("cancel", "Cancel")]);
        ScreenView { title: "Install to internal storage?".into(), layout: Layout::List, rows,
                     hints: vec![(Button::B, "Cancel".into()), (Button::A, "Select".into())], entry: None }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
}

/// The backups on the SD card, newest first, each with what it holds (when known) and when it was
/// made; A asks before restoring one.
struct RestoreScreen;

impl Screen for RestoreScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let rows = offer(ctx).map(|o| o.backups.iter().map(|b| {
            let label = match b.holds.and_then(Internal::name) { Some(what) => format!("{what} · {}", when(&b.created)), None => when(&b.created) };
            Row::new(format!("backup:{}", b.dir), label, RowKind::Action { value: Some(size(b.bytes)) })
        }).collect()).unwrap_or_default();
        ScreenView { title: "Restore internal storage".into(), layout: Layout::List, rows,
                     hints: vec![(Button::B, "Back".into()), (Button::A, "Restore".into())], entry: None }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        match (p.button, focused.and_then(|r| r.id.0.strip_prefix("backup:"))) {
            (Button::B, _) => Outcome::Back,
            (Button::A, Some(dir)) => Outcome::Push(confirm(format!("restore:{dir}"))),
            _ => Outcome::None,
        }
    }
}

/// The update files on the SD card, newest first: those that can be installed (marked when older
/// than the running version), and why the others can't; A asks before installing one.
struct UpdatesScreen;

impl Screen for UpdatesScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let rows = offer(ctx).map(|o| o.updates.iter().map(|u| {
            let name = file_name(&u.file);
            match &u.problem {
                None => Row::new(format!("update:{}", u.file), name, RowKind::Action { value: u.older.then(|| "Older".into()) }),
                Some(why) => Row::note(format!("note:{}", u.file), format!("{name} can't be installed: {why}")),
            }
        }).collect()).unwrap_or_default();
        ScreenView { title: "Install update".into(), layout: Layout::List, rows,
                     hints: vec![(Button::B, "Back".into()), (Button::A, "Install".into())], entry: None }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        match (p.button, focused.and_then(|r| r.id.0.strip_prefix("update:"))) {
            (Button::B, _) => Outcome::Back,
            (Button::A, Some(file)) => Outcome::Push(confirm(format!("update:{file}"))),
            _ => Outcome::None,
        }
    }
}

/// A confirm screen for the task its params name: "restore:<backup dir>", "update:<file>" or
/// "activate:<version>". The focus starts on Cancel.
fn confirm(params: String) -> ScreenState { focus_on(ScreenState::new(screens::CONFIRM, Some(params)), "cancel") }

/// The task a confirm screen's params name. A restore from the installed system (the offer says
/// whether one can be prepared there) prepares it.
fn confirmed_task(ctx: &Ctx, params: &str) -> Option<Task> {
    Some(match params.split_once(':')? {
        ("restore", dir) if offer(ctx)?.restore.is_some() => Task::PrepareRestore { dir: dir.into() },
        ("restore", dir) => Task::Restore { dir: dir.into() },
        ("update", file) => Task::Update { file: file.into() },
        ("activate", version) => Task::Activate { version: version.into() },
        _ => return None,
    })
}

/// "Restore the internal storage?", "Install update?" or "Start the previous version?": what the
/// task in the params does, then the go-ahead and Cancel.
struct ConfirmScreen;

impl Screen for ConfirmScreen {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let o = offer(ctx);
        let running = o.and_then(|o| o.version.as_ref()).map_or("the version running now".into(), |v| format!("version {v}"));
        let created = |dir: &str| o.and_then(|o| o.backups.iter().find(|b| b.dir == dir)).map_or(String::new(), |b| when(&b.created));
        let (title, notes, go) = match state.params.as_deref().and_then(|p| confirmed_task(ctx, p)) {
            Some(Task::Restore { dir }) => ("Restore the internal storage?",
                vec![format!("Everything on the internal storage is replaced by the backup from {}.", created(&dir))], "Restore"),
            Some(Task::PrepareRestore { dir }) => ("Restore the internal storage?",
                vec![format!("The SD card is made to start the device, to restore the backup from {}. After a restart it asks before replacing everything on the internal storage.", created(&dir))], "Restore"),
            Some(Task::Update { file }) => {
                let update = o.and_then(|o| o.updates.iter().find(|u| u.file == file));
                let version = update.and_then(|u| u.version.clone()).unwrap_or_else(|| file_name(&file).into());
                let mut notes = vec![format!("Version {version} starts at the next restart; {running} stays installed, to go back to.")];
                if update.is_some_and(|u| u.older) { notes.push(format!("This update is older than {running}.")); }
                ("Install update?", notes, "Install")
            }
            Some(Task::Activate { version }) => ("Start the previous version?",
                vec![format!("Version {version} starts at the next restart; {running} stays installed, to switch back to.")], "Switch"),
            _ => ("", vec![], ""),
        };
        let mut rows: Vec<Row> = notes.into_iter().enumerate().map(|(i, n)| Row::note(format!("note:{i}"), n)).collect();
        if !go.is_empty() { rows.push(action("go", go)); }
        rows.push(action("cancel", "Cancel"));
        ScreenView { title: title.into(), layout: Layout::List, rows,
                     hints: vec![(Button::B, "Cancel".into()), (Button::A, "Select".into())], entry: None }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
}

/// On the SD card's system, a restore prepared from the installed system: Restore, Not now (where
/// the focus starts) or Cancel the restore.
struct PendingScreen;

impl Screen for PendingScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let o = offer(ctx);
        let mut rows = vec![];
        match (o.and_then(|o| o.restore_pending.as_ref()), o.and_then(Offer::pending_backup)) {
            (Some(p), Some(b)) => {
                rows.push(Row::note("note:replace", format!("A restore was prepared on {}: everything on the internal storage is replaced by the backup from {}.", when(&p.prepared), when(&b.created))));
                rows.push(Row::note("note:later", "Until it is restored or the restore is cancelled, the device starts from the SD card."));
                rows.push(action("restore", "Restore"));
            }
            (Some(p), None) => rows.push(Row::note("note:missing", format!("The backup this restore was prepared for ({}) isn't on the SD card any more.", p.backup))),
            (None, _) => {}
        }
        rows.extend([action("later", "Not now"), action("drop", "Cancel the restore")]);
        ScreenView { title: "Restore the internal storage?".into(), layout: Layout::List, rows,
                     hints: vec![(Button::B, "Not now".into()), (Button::A, "Select".into())], entry: None }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
}

/// A task's progress screen titles: while running, once done and once failed; and the task's name
/// for "Please wait until … has finished".
fn words(kind: TaskKind) -> (&'static str, &'static str, &'static str, &'static str) {
    match kind {
        TaskKind::Install => ("Installing…", "Installed", "Install failed", "installing"),
        TaskKind::Restore => ("Restoring…", "Restored", "Restore failed", "restoring"),
        TaskKind::PrepareRestore => ("Preparing the restore…", "Restore prepared", "Preparing the restore failed", "preparing the restore"),
        TaskKind::CancelRestore => ("Cancelling the restore…", "Restore cancelled", "Cancelling the restore failed", "cancelling the restore"),
        TaskKind::Update => ("Updating…", "Updated", "Update failed", "updating"),
        TaskKind::Activate => ("Switching versions…", "Versions switched", "Switching versions failed", "switching versions"),
    }
}

/// A task's progress: its steps so far (done, or how far), then how it ended and what next.
struct TaskScreen;

impl Screen for TaskScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let Some(t) = ctx.install.as_ref().and_then(|i| i.task.as_ref()) else {
            return ScreenView { title: String::new(), layout: Layout::List, rows: vec![], hints: vec![], entry: None };
        };
        let mut rows: Vec<Row> = t.steps.iter().enumerate().map(|(i, label)| {
            let detail = match (&t.end, i + 1 == t.steps.len()) {
                (None, true) => format!("{:.0} %", t.fraction * 100.0),
                (Some(Err(_)), true) => "Failed".into(),
                _ => "Done".into(),
            };
            Row::new(format!("step:{i}"), label, RowKind::Note { detail: Some(detail) })
        }).collect();
        let (running, done, failed, _) = words(t.kind);
        let (title, hints) = match &t.end {
            None => {
                rows.push(Row::note("note:wait", format!("Step {} of {}. Keep the device on and the SD card in.", t.steps.len(), t.of.max(1))));
                (running, vec![])
            }
            Some(Ok(message)) => {
                rows.push(Row::note("note:done", message.as_str()));
                // A restore ends with the SD card still able to start the device: it is taken out
                // with the device off. Everything else takes effect at the next start.
                rows.push(if t.kind == TaskKind::Restore { action("off", "Power off") } else { action("restart", "Restart") });
                (done, vec![(Button::A, "Select".into())])
            }
            Some(Err(e)) => {
                rows.push(Row::note("note:error", e.message.as_str()));
                rows.push(Row::note("note:hint", e.hint.as_str()));
                if let Some(log) = &e.log { rows.push(Row::note("note:log", format!("Log: {log}"))); }
                rows.push(action("back", "Back"));
                (failed, vec![(Button::B, "Back".into()), (Button::A, "Select".into())])
            }
        };
        ScreenView { title: title.into(), layout: Layout::List, rows, hints, entry: None }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
}

/// "2026-10-14 23:24 UTC" from "20261014-232447" (the device's clock knows no time zone).
fn when(created: &str) -> String {
    if created.len() < 13 || !created.is_ascii() { return created.into(); }
    format!("{}-{}-{} {}:{} UTC", &created[0..4], &created[4..6], &created[6..8], &created[9..11], &created[11..13])
}

/// The last part of a path.
fn file_name(path: &str) -> &str { path.rsplit('/').next().unwrap_or(path) }
