//! Settings → Credits: every project Specular ships and its licence, grouped by kind. The list is
//! worked out when the binary is built (the `specular-credits` crate) and given to the app.

use std::collections::HashMap;
use std::path::PathBuf;

use super::*;

/// One project Specular ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credit {
    pub kind: CreditKind,
    pub name: &'static str,
    /// A crate's version, or the commit a core is built from (abbreviated).
    pub version: Option<&'static str>,
    /// The licence it is used under, as an SPDX expression.
    pub licence: &'static str,
    pub url: Option<&'static str>,
}

/// What a project is: the credits screen's groups, in the order the build lists them. The variants
/// must be named as specular-credits' `Kind`'s are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CreditKind { Specular, Core, Font, Data, Library }

/// What the credits screen shows.
#[derive(Clone, Debug, Default)]
pub struct Credits {
    /// Grouped by kind: the screen starts a group wherever the kind changes.
    pub projects: &'static [Credit],
    /// The file with every licence text in full; None if it wasn't installed.
    pub texts: Option<PathBuf>,
}

pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) { screens.insert(screens::CREDITS, Box::new(CreditsScreen)); }

impl App {
    /// Gives the app the credits screen's contents (empty until given).
    pub fn set_credits(&mut self, credits: Credits) { self.ctx.credits = credits; self.changed(); }
}

/// Settings' row opening the credits.
pub(super) fn settings_row() -> Row { action("open:credits", "Credits") }

struct CreditsScreen;

impl Screen for CreditsScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let texts = match &ctx.credits.texts {
            Some(path) => format!("The licence texts are in {}", path.display()),
            None => "The licence texts weren't installed with Specular".into(),
        };
        let mut rows = vec![Row::note("note:texts", texts)];
        let mut kind = None;
        for (i, c) in ctx.credits.projects.iter().enumerate() {
            if kind != Some(c.kind) {
                kind = Some(c.kind);
                // Specular itself comes first, under no heading.
                let heading = match c.kind { CreditKind::Specular => None, CreditKind::Core => Some("Emulator cores"), CreditKind::Font => Some("Fonts"),
                                             CreditKind::Data => Some("Data"), CreditKind::Library => Some("Libraries") };
                rows.extend(heading.map(|h| Row::new(format!("header:{h}"), h, RowKind::Header)));
            }
            let label = c.version.map_or(c.name.to_string(), |v| format!("{} {v}", c.name));
            rows.push(Row::new(format!("credit:{i}"), label, RowKind::Text { detail: Some(c.licence.into()) }));
        }
        ScreenView { title: "Credits".into(), layout: Layout::List, rows, hints: vec![(Button::B, "Back".into()), (Button::A, "Web address".into())], entry: None }
    }

    /// A shows the focused project's web address.
    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        match p.button {
            Button::B => Outcome::Back,
            Button::A => {
                let credit = focused.and_then(|r| r.id.0.strip_prefix("credit:")?.parse::<usize>().ok()).and_then(|i| ctx.credits.projects.get(i));
                match credit.and_then(|c| c.url) {
                    Some(url) => Outcome::Effects(vec![Effect::Notice(url.into())]),
                    None => Outcome::None,
                }
            }
            _ => Outcome::None,
        }
    }
}
