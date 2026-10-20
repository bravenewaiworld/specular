//! A setting's help on screen: on the settings screens, Y on a setting that has help
//! (`SettingDecl::help`) opens a screen showing it under the setting's section and label.

use std::collections::HashMap;

use super::{Ctx, Outcome, Screen, screens};
use crate::input::{Button, Press};
use crate::model::{Row, RowKind, ScreenId, ScreenView};
use crate::nav::ScreenState;

pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) {
    screens.insert(screens::ABOUT_SETTING, Box::new(AboutSetting));
}

/// Whether `row` belongs to a setting that has help.
pub(super) fn has_help(ctx: &Ctx, row: &Row) -> bool { ctx.settings.decl(&row.id.0).is_some_and(|d| !d.help.is_empty()) }

/// Y on a setting's row with help opens its help; None for any other press.
pub(super) fn press(ctx: &Ctx, row: &Row, p: Press) -> Option<Outcome> {
    (p.button == Button::Y && !p.repeat && has_help(ctx, row))
        .then(|| Outcome::Push(ScreenState::new(screens::ABOUT_SETTING, Some(row.id.0.clone()))))
}

/// The screen of one setting's help; its params hold the setting's key. The help is a note, so a long one scrolls a few
/// lines a press (`list::read_step`).
struct AboutSetting;

impl Screen for AboutSetting {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let Some(d) = state.params.as_deref().and_then(|key| ctx.settings.decl(key)) else { return ScreenView::list("About", vec![]) };
        ScreenView::list(d.label, vec![Row::new("header", d.section, RowKind::Header), Row::note("note:help", d.help)])
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, p: Press) -> Outcome {
        if p.button == Button::B { Outcome::Back } else { Outcome::None }
    }
}
