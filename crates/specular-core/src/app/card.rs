//! Following the SD card as the device reports it (`Msg::Card`). The library and the settings
//! live on the card, so:
//!
//! - A card that can be read and wasn't before (or another card) is read afresh: its settings
//!   replace those in memory, and its library is scanned (`Effect::ReadCard`).
//! - A card that can't be read (absent, being checked, failed) empties the library, and home
//!   shows the system's message in its place. A read-only card's message stays above the
//!   library; a repair is told once, until dismissed.
//! - Nothing is written to a card that can't be written: settings stay in memory, a game can't
//!   save its state, and quitting doesn't write its saves.
//! - A game whose card goes is paused under `screens::CARD_WAIT`, which asks for the card back.
//!   The same card back (by id) and writable saves the game and returns to the library; quitting
//!   instead loses what wasn't saved. Playing on isn't offered: disc games stream from the card.
//! - A write that fails as read-only while the card counts as writable means the kernel turned
//!   it read-only after an error, which it announces nowhere: the device is asked to look again
//!   (`CardCmd::Recheck`), which reports it read-only.

use super::*;

/// The row on home that dismisses the card's one-time message.
pub(super) const DISMISS: &str = "card:dismiss";


pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) {
    screens.insert(screens::CARD_WAIT, Box::new(CardWait));
}

impl App {
    /// Whether the card can be written to now.
    pub(super) fn card_writable(&self) -> bool { self.ctx.card.as_ref().is_some_and(Card::writable) }

    pub(super) fn card_changed(&mut self, card: Card) -> Vec<Effect> {
        self.rechecking_card = false;
        let before = self.ctx.card.replace(card.clone());
        if before.as_ref() == Some(&card) { return vec![]; }
        let mut e = vec![];
        let fresh = card.readable() && before.as_ref().is_none_or(|b| b.status != card.status || b.id != card.id);
        if fresh {
            self.ctx.settings.clear_layers();
            // Nothing acts on the settings (the radios, the time zone) until the card's are read.
            self.settings_loaded = false;
            self.ctx.scanned = false;
            self.ctx.card_note = card.repaired();
            e.push(Effect::ReadCard { writable: card.writable() });
        }
        if !card.readable() {
            if before.as_ref().is_some_and(Card::readable) { e.push(Effect::ForgetCard); }
            self.ctx.games.clear();
            self.ctx.card_note = false;
            // Started without a card: the device runs on the default settings meanwhile.
            if !self.settings_loaded { e.extend(self.layer_loaded(Layer::Global, Default::default())); }
            // A system's game list has nothing left to show; home says why.
            if self.playing.is_none() && self.nav.top().id == screens::GAMES { e.extend(self.go(|nav| nav.home())); }
        }
        if self.playing.is_some() { e.extend(self.game_card(before.as_ref(), &card)); }
        self.changed();
        if !e.iter().any(|e| matches!(e, Effect::Redraw(_))) { e.push(Effect::Redraw(None)); }
        e
    }

    /// What the card's change means for the game being played.
    fn game_card(&mut self, before: Option<&Card>, card: &Card) -> Vec<Effect> {
        let waiting = self.nav.find(&screens::CARD_WAIT).map(|s| s.params.clone());
        match waiting {
            None if !card.readable() => {
                let id = before.and_then(|b| b.id.clone());
                let mut e = vec![Effect::PauseGame(true)];
                e.extend(self.go(|nav| Some(nav.push(ScreenState::new(screens::CARD_WAIT, id)))));
                e
            }
            Some(id) if card.writable() && card.id == id => {
                self.playing = None;
                let mut e = vec![Effect::QuitGame, Effect::Notice("The game was saved".into())];
                e.extend(self.go(|nav| nav.home()));
                e
            }
            // Still waiting: its screen shows the card's new state.
            _ => vec![],
        }
    }

    /// A write failed as read-only (`Msg::CardReadOnly`).
    pub(super) fn card_read_only(&mut self) -> Vec<Effect> {
        if self.rechecking_card || !self.card_writable() { return vec![]; }
        self.rechecking_card = true;
        vec![Effect::Card(CardCmd::Recheck), Effect::Notice("The SD card has turned read-only: nothing more can be saved".into())]
    }

    /// Drops settings saves while the card can't take them: the changes stay in memory.
    pub(super) fn hold_settings_saves(&self, e: &mut Vec<Effect>) {
        if !self.card_writable() { e.retain(|e| !matches!(e, Effect::SaveSettings(_))); }
    }

    /// A press while waiting for the card: A on "quit" ends the game without saving.
    pub(super) fn card_wait(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        if (button, row) != (Button::A, Some("quit")) { return vec![]; }
        self.playing = None;
        let mut e = vec![Effect::AbandonGame];
        e.extend(self.go(|nav| nav.home()));
        e
    }
}

/// Home's rows above the library: a read-only card's message, or a repair's until dismissed.
pub(super) fn home_notes(ctx: &Ctx) -> Vec<Row> {
    match &ctx.card {
        Some(card) if card.status == crate::card::CardStatus::ReadOnly => vec![Row::note("note:card", card.text())],
        Some(card) if ctx.card_note => vec![Row::note("note:card", card.text()), action(DISMISS, "OK")],
        _ => vec![],
    }
}

/// Over a paused game whose card went: what it takes to save it, and quitting without saving.
/// Its params hold the id of the card the game runs from.
struct CardWait;

impl Screen for CardWait {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let card = ctx.card.clone().unwrap_or_default();
        let title = "Card removed".to_string();
        if card.turning_off() {
            return ScreenView { title, layout: Layout::List, rows: vec![Row::note("note:card", card.text())], hints: vec![], entry: None };
        }
        let ask = match (card.readable(), card.id == state.params) {
            (true, false) => "This isn't the SD card the game was started from. Insert that one again to save the game.",
            (true, true) => "The SD card is back, but read-only: the game can't be saved.",
            (false, _) => "Insert the SD card again to save the game.",
        };
        let mut rows = vec![Row::note("note:ask", ask)];
        if !card.text().is_empty() { rows.push(Row::note("note:card", card.text())); }
        rows.extend([action("quit", "Quit unsaved"), Row::note("note:lost", "Quitting loses what the game hasn't saved yet.")]);
        ScreenView { title, layout: Layout::List, rows, hints: vec![(Button::A, "Select".into())], entry: None }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
}
