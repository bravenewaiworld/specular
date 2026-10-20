//! Resuming a game whose autosave was made on another BIOS than it would start on now: a PS1
//! autosave made on SwanStation's built-in OpenBIOS, say, once a BIOS file is in `bios/`. A state
//! holds the BIOS it was made on, so resuming it keeps that one; the player is asked before the
//! game starts (`screens::RESUME`) whether to resume it so, or start afresh with its autosave set
//! aside (`Effect::LaunchAfresh`). Only games whose core has a stand-in for a missing BIOS file
//! (`Bios::fallback`) are checked; an autosave that doesn't say what it was made on isn't asked
//! about (see docs/runtime.md).

use super::*;
use crate::library::BIOS_FILE;

pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) {
    screens.insert(screens::RESUME, Box::new(Resume));
}

impl App {
    /// Starts `game`; for a core with a stand-in BIOS, its autosave is checked first
    /// (`Effect::CheckAutosave`).
    pub(super) fn launch_or_check(&mut self, game: Game) -> Vec<Effect> {
        let stand_in = self.core_of(&game).is_some_and(|c| c.bios.iter().any(|b| b.fallback.is_some()));
        if stand_in { vec![Effect::CheckAutosave(game)] } else { self.launch(game, false) }
    }

    /// `Msg::AutosaveChecked`: starts the game, or asks first if its autosave's BIOS differs.
    /// Ignored unless a list of games is still showing, with no game playing: the player may have
    /// left it, or pressed A twice, while the autosave was read.
    pub(super) fn autosave_checked(&mut self, game: Game, bios: Option<BiosChange>) -> Vec<Effect> {
        let listing = [screens::GAMES, screens::SEARCH].contains(&self.nav.top().id);
        if self.playing.is_some() || !listing { return vec![]; }
        let Some(change) = bios else { return self.launch(game, false) };
        self.ctx.resume = Some((game, change));
        self.go(|nav| Some(nav.push(ScreenState::new(screens::RESUME, None))))
    }

    /// A press on the question: A on Resume or Start afresh starts the game so; B, or A on
    /// Cancel, goes back; Menu goes home. (`ctx.resume` is left as it is: only this screen reads
    /// it, and `autosave_checked` sets it before opening it.)
    pub(super) fn resume_press(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        let afresh = match (button, row) {
            (Button::A, Some("resume")) => false,
            (Button::A, Some("afresh")) => true,
            (Button::B, _) | (Button::A, Some("cancel")) => return self.go(|nav| nav.pop()),
            (Button::Menu, _) => return self.go(|nav| nav.home()),
            _ => return vec![],
        };
        let Some((game, _)) = self.ctx.resume.take() else { return vec![] };
        let mut e = self.go(|nav| nav.pop());
        e.extend(self.launch(game, afresh));
        e
    }
}

/// "made on OpenBIOS", or "made with a BIOS file" for `BIOS_FILE`.
pub(super) fn made_on(bios: &str) -> String {
    if bios == BIOS_FILE { "made with a BIOS file".into() } else { format!("made on {bios}") }
}

struct Resume;

impl Screen for Resume {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let Some((game, BiosChange { made_on: made, now })) = &ctx.resume else { return ScreenView::list("", vec![]) };
        let what = if now == BIOS_FILE {
            format!("Its autosave was {}, and there is a BIOS file in bios/ now. Resume keeps {made}; Start afresh uses the BIOS file.", made_on(made))
        } else {
            format!("Its autosave was {}, and bios/ has no BIOS file now. Resume keeps that BIOS; Start afresh uses {now}.", made_on(made))
        };
        let note = format!("{what} Starting afresh sets the autosave aside, in the .states folder beside the game. The memory card (saves made in the game) is kept either way.");
        ScreenView {
            title: game.title.clone(), layout: Layout::List,
            rows: vec![Row::note("note:question", note), action("resume", "Resume"), action("afresh", "Start afresh"), action("cancel", "Cancel")],
            hints: vec![(Button::B, "Cancel".into()), (Button::A, "Select".into())],
            entry: None,
        }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
}
