//! A developer screen listing every core: Specular's own, those found in the SD card's `cores/`
//! (with where each is and whether its options have been learned), and the files there turned
//! away, with why. Each with the systems it plays.

use std::collections::HashMap;

use super::{Ctx, Outcome, Screen, screens};
use crate::input::{Button, Press};
use crate::library;
use crate::model::{Row, RowKind, ScreenId, ScreenView};
use crate::nav::ScreenState;

pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) { screens.insert(screens::CORES, Box::new(CoresScreen)); }

struct CoresScreen;

impl Screen for CoresScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let header = |h: &str| Row::new(format!("header:{h}"), h, RowKind::Header);
        let (user, own): (Vec<_>, Vec<_>) = library::cores(&ctx.systems).into_iter().partition(|c| c.user.is_some());
        let mut rows = Vec::new();
        for (title, cores) in [("Built in", &own), ("On the SD card", &user)] {
            rows.push(header(title));
            for c in cores {
                let systems: Vec<String> = ctx.systems.iter().filter(|s| s.cores.iter().any(|k| k.id() == c.id())).map(|s| s.id.to_uppercase()).collect();
                rows.push(Row::new(format!("core:{}", c.id()), c.name, RowKind::Text { detail: Some(systems.join(", ")) }));
                if let Some(u) = c.user {
                    let file = u.path.file_name().unwrap_or_default().to_string_lossy();
                    rows.push(Row::note(format!("note:{}", c.id()), format!("{file}; {}", u.status())));
                }
            }
        }
        if user.is_empty() { rows.push(Row::note("note:none", "No cores in its cores folder")); }
        if !ctx.turned_away.is_empty() { rows.push(header("Turned away")); }
        for (i, t) in ctx.turned_away.iter().enumerate() {
            rows.push(Row::new(format!("away:{i}"), &t.file, RowKind::Text { detail: None }));
            rows.push(Row::note(format!("why:{i}"), sentence(&t.why)));
        }
        ScreenView::list("Cores", rows)
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, p: Press) -> Outcome {
        if p.button == Button::B { Outcome::Back } else { Outcome::None }
    }
}

/// `why` with its first letter in capitals: reasons are written to follow a colon in the log
/// ("not using <file>: …"), and on screen stand as sentences of their own.
pub(super) fn sentence(why: &str) -> String {
    let mut chars = why.chars();
    chars.next().map_or(String::new(), |c| c.to_uppercase().chain(chars).collect())
}
