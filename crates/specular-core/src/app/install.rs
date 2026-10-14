//! Installing onto the internal storage, and restoring a backup of it, in Settings: the rows that
//! offer them (from the installer's `Offer`, asked for each time Settings opens), the screens that
//! ask first, and the screen that follows the installer's progress.
//!
//! While a task runs its screen can't be left, and the device neither sleeps nor powers off: an
//! interrupted install leaves the internal storage unable to start (an SD card still starts the
//! device).

use std::collections::HashMap;

use super::*;
use crate::install::{Offer, Progress, Task, TaskError, size};

/// What the app knows of the installer.
#[derive(Default)]
pub struct InstallState {
    /// The last offer (None until the installer answers; Err: it couldn't tell).
    pub offer: Option<Result<Offer, String>>,
    /// The task started last, while its screen is open.
    pub task: Option<TaskView>,
}

/// A task's progress, as its screen shows it.
pub struct TaskView {
    /// Restoring a backup, rather than installing.
    pub restore: bool,
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
    screens.insert(screens::RESTORE_CONFIRM, Box::new(RestoreConfirmScreen));
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
    pub(super) fn task_wait_notice(&self) -> &'static str {
        match self.ctx.install.as_ref().and_then(|i| i.task.as_ref()) {
            Some(t) if t.restore => "Please wait until restoring has finished",
            _ => "Please wait until installing has finished",
        }
    }

    pub(super) fn install_offer(&mut self, offer: Result<Offer, String>) -> Vec<Effect> {
        let Some(state) = &mut self.ctx.install else { return vec![] };
        state.offer = Some(offer);
        self.changed();
        vec![Effect::Redraw(None)]
    }

    pub(super) fn install_progress(&mut self, p: Progress) -> Vec<Effect> {
        let Some(task) = self.ctx.install.as_mut().and_then(|i| i.task.as_mut()) else { return vec![] };
        let done = matches!(p, Progress::Done(_));
        match p {
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
        let Some(state) = &mut self.ctx.install else { return vec![] };
        state.task = Some(TaskView { restore: matches!(task, Task::Restore { .. }), steps: vec![], of: 0, fraction: 0.0, end: None });
        let mut e = vec![Effect::Installer(InstallCmd::Start(task))];
        e.extend(self.go(|nav| Some(nav.push(ScreenState::new(screens::TASK, None)))));
        e
    }

    /// A press on the install screen: A switches its options, installs, or cancels; B cancels.
    pub(super) fn install_screen(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
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

    /// A press on "Restore …?": A restores or cancels; B cancels.
    pub(super) fn restore_confirm(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        match (button, row) {
            (Button::B, _) | (Button::A, Some("cancel")) => self.go(|nav| nav.pop()),
            (Button::A, Some("restore")) => {
                let dir = self.nav.top().params.clone().unwrap_or_default();
                self.start_task(Task::Restore { dir })
            }
            _ => vec![],
        }
    }

    /// A press on the progress screen: nothing while the task runs; then A does what the screen
    /// offers (restart, power off, or back), and B goes back after a failure.
    pub(super) fn task_screen(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
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

/// Settings' Internal storage section: installing, restoring, or (with developer details on) why
/// installing isn't offered.
pub(super) fn settings_rows(ctx: &Ctx) -> Vec<Row> {
    let Some(state) = &ctx.install else { return vec![] };
    let developer = ctx.visibility() == Visibility::Developer;
    let mut rows = vec![];
    match &state.offer {
        Some(Ok(o)) => {
            if o.install.is_some() { rows.push(action("install:open", "Install to internal storage")); }
            if !o.backups.is_empty() { rows.push(action("install:restore", "Restore internal storage")); }
            if let (None, Some(why), true) = (&o.install, &o.why_not, developer) { rows.push(note("install:why", &format!("Install: {why}"))); }
        }
        Some(Err(e)) if developer => rows.push(note("install:why", &format!("Installer: {e}"))),
        _ => {}
    }
    if !rows.is_empty() { rows.insert(0, Row::new("header:install", "Internal storage", RowKind::Header)); }
    rows
}

pub(super) fn settings_press(ctx: &Ctx, row: &Row, p: Press) -> Option<Outcome> {
    if p.button != Button::A { return None; }
    match row.id.0.as_str() {
        "install:open" => {
            let backup = offer(ctx).and_then(|o| o.install.as_ref()).is_some_and(|i| i.backup_possible);
            Some(Outcome::Push(on_cancel(ScreenState::new(screens::INSTALL, Some(InstallScreen::param(backup, false))))))
        }
        "install:restore" => Some(Outcome::Push(ScreenState::new(screens::RESTORE, None))),
        _ => None,
    }
}

/// A line of explanation (never focused).
fn note(id: &str, text: &str) -> Row { Row { enabled: false, ..Row::new(id, text, RowKind::Text { detail: None }) } }

/// An explanation as lines of at most `NOTE_WIDTH` characters (a row shows one line), split
/// between words; their ids are `id` followed by the line's number.
fn notes(id: &str, text: &str) -> Vec<Row> {
    let mut lines: Vec<String> = vec![];
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if l.chars().count() + 1 + word.chars().count() <= NOTE_WIDTH => { l.push(' '); l.push_str(word); }
            _ => lines.push(word.into()),
        }
    }
    lines.iter().enumerate().map(|(i, l)| note(&format!("{id}:{i}"), l)).collect()
}

/// About the most characters a list row's label shows on the Brick's panel.
const NOTE_WIDTH: usize = 44;

/// A screen that asks before erasing something starts with the focus on Cancel.
fn on_cancel(mut state: ScreenState) -> ScreenState {
    state.focus = Some(crate::model::RowId("cancel".into()));
    state
}

fn offer(ctx: &Ctx) -> Option<&Offer> { ctx.install.as_ref()?.offer.as_ref()?.as_ref().ok() }

/// "Install to internal storage?": what it erases, whether to back up first (offered, and on to
/// start with, if the card has room), and (for developers) whether the card stays bootable. Its
/// params hold the options: "backup=<0|1>,keep=<0|1>".
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
        let mut rows = notes("note:erase", "Erases everything on the internal storage, including TrimUI's original system.");
        rows.extend(notes("note:card", "Games and saves stay on the SD card."));
        match offer(ctx).and_then(|o| o.install.as_ref()) {
            Some(i) if i.backup_possible => {
                rows.push(Row::new("backup", "Back up the internal storage first", RowKind::Toggle { on: backup }));
                if !backup { rows.extend(notes("note:backup", "Without a backup, it can't be put back.")); }
            }
            Some(i) => rows.extend(notes("note:backup", &format!("No room for a backup ({} needed, {} free on the SD card): it can't be put back.", size(i.bytes), size(i.free)))),
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

/// The backups on the SD card, newest first; A asks before restoring one.
struct RestoreScreen;

impl Screen for RestoreScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let rows = offer(ctx).map(|o| o.backups.iter().map(|b| {
            Row::new(format!("backup:{}", b.dir), when(&b.created), RowKind::Action { value: Some(size(b.bytes)) })
        }).collect()).unwrap_or_default();
        ScreenView { title: "Restore internal storage".into(), layout: Layout::List, rows,
                     hints: vec![(Button::B, "Back".into()), (Button::A, "Restore".into())], entry: None }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        match (p.button, focused.and_then(|r| r.id.0.strip_prefix("backup:"))) {
            (Button::B, _) => Outcome::Back,
            (Button::A, Some(dir)) => Outcome::Push(on_cancel(ScreenState::new(screens::RESTORE_CONFIRM, Some(dir.into())))),
            _ => Outcome::None,
        }
    }
}

/// "Restore the internal storage?", for the backup whose folder is in the params.
struct RestoreConfirmScreen;

impl Screen for RestoreConfirmScreen {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let dir = state.params.as_deref().unwrap_or_default();
        let created = offer(ctx).and_then(|o| o.backups.iter().find(|b| b.dir == dir)).map_or(String::new(), |b| when(&b.created));
        ScreenView {
            title: "Restore the internal storage?".into(), layout: Layout::List,
            rows: notes("note:replace", &format!("Everything on the internal storage is replaced by the backup from {created}."))
                .into_iter().chain([action("restore", "Restore"), action("cancel", "Cancel")]).collect(),
            hints: vec![(Button::B, "Cancel".into()), (Button::A, "Select".into())], entry: None,
        }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
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
            Row { enabled: false, ..Row::new(format!("step:{i}"), label, RowKind::Text { detail: Some(detail) }) }
        }).collect();
        let (title, hints) = match &t.end {
            None => {
                rows.extend(notes("note:wait", &format!("Step {} of {}. Keep the device on and the SD card in.", t.steps.len(), t.of.max(1))));
                (if t.restore { "Restoring…" } else { "Installing…" }, vec![])
            }
            Some(Ok(message)) => {
                rows.extend(notes("note:done", message));
                rows.push(if t.restore { action("off", "Power off") } else { action("restart", "Restart") });
                (if t.restore { "Restored" } else { "Installed" }, vec![(Button::A, "Select".into())])
            }
            Some(Err(e)) => {
                rows.extend(notes("note:error", &e.message));
                rows.extend(notes("note:hint", &e.hint));
                if let Some(log) = &e.log { rows.extend(notes("note:log", &format!("Log: {log}"))); }
                rows.push(action("back", "Back"));
                (if t.restore { "Restore failed" } else { "Install failed" }, vec![(Button::B, "Back".into()), (Button::A, "Select".into())])
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
