//! What needs the user's attention: files on the card Specular can't use, and settings it can't
//! save, each with what it is, why, and what to do (`Problem`). Home counts them ("3 things need
//! attention") and opens `screens::PROBLEMS`, which lists them. Each is also logged in full where
//! it is found (the scan, the settings files, the fonts). Unknown settings keys aren't problems:
//! they can't be told apart from a plugin's, or a newer version's.

use std::path::{Path, PathBuf};

use super::{Ctx, Effect, Outcome, Screen, action, cores::sentence, screens};
use crate::input::{Button, Press};
use crate::library::user_cores::TurnedAway;
use crate::model::{Row, RowKind, ScreenId, ScreenView};
use crate::nav::ScreenState;
use crate::settings::{Layer, schema};

/// Something the user should know of and can put right.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// What it is, as a row's label: "cores/foo_libretro.so isn't used".
    pub what: String,
    /// Why, as a sentence or two.
    pub why: String,
    /// What the user can do about it.
    pub fix: String,
    /// A screen with more about it, opened from its row.
    pub opens: Option<ScreenId>,
}

/// What the card's latest scan found that Specular can't use (`Msg::LeftOut`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LeftOut {
    /// How many games each system no emulator plays has, by the system's name
    /// (`library::Scan::no_emulator`).
    pub no_emulator: Vec<(&'static str, usize)>,
    /// Settings files set aside as damaged: each `<file>.bad`, relative to the config folder, and
    /// why its settings aren't used (`store::set_aside`).
    pub set_aside: Vec<(PathBuf, String)>,
    /// Settings files under `games/` that match no game on the card, each as its game's path under
    /// `roms/` without the file type ("gba/Old Name").
    pub orphans: Vec<String>,
}

pub(super) fn register(screens: &mut std::collections::HashMap<ScreenId, Box<dyn Screen>>) {
    screens.insert(screens::PROBLEMS, Box::new(ProblemsScreen));
}

/// Every problem now. Those of the card's files only while it can be read.
pub(super) fn problems(ctx: &Ctx) -> Vec<Problem> {
    let mut all = vec![];
    if ctx.card_in() {
        all.extend(ctx.left_out.no_emulator.iter().map(|&(system, n)| no_emulator(system, n)));
        all.extend(ctx.turned_away.iter().map(core_turned_away));
        all.extend(ctx.left_out.set_aside.iter().map(|(bad, why)| set_aside(bad, why)));
        all.extend(ctx.settings.ignored().iter().map(|(layer, key, why)| ignored(layer, key, why)));
        all.extend(ctx.left_out.orphans.iter().map(|game| orphan(game)));
        all.extend(ctx.settings_unsaved.as_deref().map(unsaved));
    }
    all.extend(ctx.fonts_turned_away.iter().map(font_turned_away));
    all
}

/// `Effect::CheckSetAside` as `screen` is shown, if it lists problems (home, the problems
/// screen) and `.bad` files are among them, so one fixed on a PC goes from the list. Not while none
/// are listed: new ones come from reading the card, whose scan lists them.
pub(super) fn recheck_set_aside(ctx: &Ctx, screen: &ScreenId) -> Option<Effect> {
    let lists_problems = [screens::HOME, screens::PROBLEMS].contains(screen);
    (lists_problems && !ctx.left_out.set_aside.is_empty()).then_some(Effect::CheckSetAside)
}

/// Home's row for the problems ("3 things need attention"); None while there are none.
pub(super) fn home_row(ctx: &Ctx) -> Option<Row> {
    let label = match problems(ctx).len() {
        0 => return None,
        1 => "1 thing needs attention".to_string(),
        n => format!("{n} things need attention"),
    };
    Some(action(OPEN, &label))
}

/// The id of home's row that opens the problems.
pub(super) const OPEN: &str = "open:problems";

fn problem(what: impl Into<String>, why: impl Into<String>, fix: impl Into<String>) -> Problem {
    Problem { what: what.into(), why: why.into(), fix: fix.into(), opens: None }
}

/// A path under the config folder, from the card's root: "config/v0/systems/gba.json".
fn in_config(file: &Path) -> String { Path::new(crate::settings::migrate::CONFIG).join(file).display().to_string() }

fn no_emulator(system: &str, n: usize) -> Problem {
    let what = if n == 1 { format!("1 {system} game isn't shown") } else { format!("{n} {system} games aren't shown") };
    problem(what, format!("No emulator on the device plays {system} games."),
            format!("Add an emulator core for {system} games to cores/ on the SD card (cores/README.txt there says how)."))
}

fn core_turned_away(t: &TurnedAway) -> Problem {
    let why = format!("{}.", sentence(&t.why));
    // `UserCores::find` names the folder itself, by its whole path, when it can't be read.
    let p = if Path::new(&t.file).is_absolute() {
        problem("The cores/ folder can't be read", why, "Check the SD card for errors on a PC; copying the folder off, deleting it and copying it back may mend it.")
    } else {
        problem(format!("cores/{} isn't used", t.file), why, "Replace or remove it in cores/ on the SD card; cores/README.txt there says what a core needs.")
    };
    Problem { opens: Some(screens::CORES), ..p }
}

fn set_aside(bad: &Path, why: &str) -> Problem {
    let file = bad.with_extension("");
    let name = file.file_name().unwrap_or_default().to_string_lossy();
    let fix = if why == crate::settings::store::FIXED_SINCE {
        format!("Rename it back to {name} (replacing any {name} made since); it is read when the device next starts.")
    } else {
        format!("On a PC, correct it (it is JSON text; the error says where) and rename it back to {name}, replacing any {name} made since; it is read when the device next starts. Or delete it to start over.")
    };
    problem(format!("{} couldn't be read", in_config(&file)), format!("Its settings aren't used: {why}. Specular renamed it {name}.bad."), fix)
}

fn ignored(layer: &Layer, key: &str, why: &str) -> Problem {
    problem(format!("{key} in {} is ignored", in_config(&layer.file())), format!("{}, so it has no effect.", sentence(why)),
            format!("Change it in Settings on the device, or correct or remove it in the file on a PC. {} lists the settings the file can hold and the values each takes.", in_config(&schema::file(layer))))
}

fn orphan(game: &str) -> Problem {
    problem(format!("{} matches no game", in_config(&Path::new("games").join(format!("{game}.json")))),
            format!("There is no game at roms/{game} (of any file type), so its settings aren't used."),
            "If the game was renamed or moved, rename or move this file to match; otherwise delete it.")
}

fn unsaved(why: &str) -> Problem {
    problem("Settings can't be saved",
            format!("Writing them to the SD card failed ({why}). Changes to settings last only until the device turns off."),
            "Make room on the SD card, or check it for errors on a PC. Specular tries again the next time a setting changes.")
}

fn font_turned_away(t: &TurnedAway) -> Problem {
    problem(format!("fonts/{} isn't used", t.file), format!("{}.", sentence(&t.why)),
            "Remove it from fonts/ on the SD card: only TrueType or OpenType font files (.ttf, .otf, .ttc) directly in fonts/ are used. Restart the device after changing fonts.")
}

/// Every problem: what it is, then why and what to do. A on one with a screen of its own opens it.
struct ProblemsScreen;

impl Screen for ProblemsScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let problems = problems(ctx);
        if problems.is_empty() { return ScreenView::list("Needs attention", vec![Row::note("note:none", "Nothing needs attention.")]); }
        let rows = problems.into_iter().enumerate().flat_map(|(i, p)| {
            let mut row = Row::new(format!("problem:{i}"), p.what, RowKind::Text { detail: None });
            if p.opens.is_some() { (row.kind, row.hints) = (RowKind::Action { value: None }, vec![(Button::A, "Details".into())]); }
            [row, Row::note(format!("note:{i}"), format!("{} {}", p.why, p.fix))]
        }).collect();
        ScreenView::list("Needs attention", rows)
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        let opens = || problems(ctx).into_iter().nth(focused?.id.0.strip_prefix("problem:")?.parse().ok()?)?.opens;
        match p.button {
            Button::B => Outcome::Back,
            Button::A => opens().map_or(Outcome::None, |screen| Outcome::Push(ScreenState::new(screen, None))),
            _ => Outcome::None,
        }
    }
}
