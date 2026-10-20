//! The SD card, as the system reports it. On Crystaline OS the system mounts the card as it comes
//! and goes and writes how it stands to a state file (`Card::parse` reads it); without a card
//! service (the desktop, by default) the card counts as `Card::mounted()`. A device reports it
//! through a
//! `CardService`, the same kind of service as the radios (`wireless::Wireless`), so the wireless
//! trace and scenarios cover it too.

use serde::{Deserialize, Serialize};

use crate::wireless::Wireless;

/// The SD card's state, as the system words it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Card {
    /// How the card stands (the state file's first word).
    pub status: CardStatus,
    /// Why, for `ReadOnly` and `Failed`: a stable word, e.g. "damaged", "write-protected",
    /// "no-partition", "system-card-removed".
    pub reason: Option<String>,
    /// What to tell the user, as the system words it (what happened, what still works, what to
    /// do); may be empty (see `text`).
    pub message: String,
    /// The card's identity (its CID, the identification number built into the card): tells whether the same card came back after a removal.
    pub id: Option<String>,
    /// How the filesystem check before mounting went: "clean", "unclean", "repaired",
    /// "errors-left" or "not-run".
    pub check: Option<String>,
}

/// How the card stands (the state file's first word).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardStatus {
    /// No card in the slot.
    Absent,
    /// A card was found and is being checked (the first time the system starts from a new card, it is also partitioned).
    Checking,
    /// Mounted read-write.
    #[default]
    Mounted,
    /// Mounted read-only: games play, nothing can be saved.
    ReadOnly,
    /// In the slot but unusable, or (`system-card-removed`) the card the system runs from was
    /// pulled, and the device turns off in about 3 s.
    Failed,
}

/// What can be asked of a `CardService`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardCmd {
    /// A write to the card failed as read-only: have the system look at the card again (the
    /// kernel makes a damaged card read-only without telling anyone). The service then reports
    /// the card's state, changed or not.
    Recheck,
}

/// The service reporting the card: on `start` it reports the card's state, then each change.
pub type CardService = dyn Wireless<CardCmd, Card>;

impl Card {
    /// The state of a card that is simply there and writable: what a device without a state file
    /// (e.g. the desktop) always reports.
    pub fn mounted() -> Self { Self::default() }

    /// Reads the system's state file (Crystaline OS's `/run/card/state`):
    ///
    /// ```text
    /// <absent|checking|mounted|read-only <reason>|failed <reason>>
    /// <message for the user>
    /// card=<cid> partition=<name> type=<vfat|exfat> check=<result>
    /// ```
    ///
    /// Unknown keys on line 3 are ignored, and empty values are None. A first line it doesn't
    /// know (from a newer system, or a damaged file) reads as `Failed`, with the reason
    /// `unknown state: <line>`, so the message is still shown and the library isn't used.
    pub fn parse(text: &str) -> Self {
        let mut lines = text.lines();
        let first = lines.next().unwrap_or_default().trim();
        let (word, rest) = first.split_once(' ').map_or((first, ""), |(w, r)| (w, r.trim()));
        let reason = (!rest.is_empty()).then(|| rest.to_string());
        let (status, reason) = match word {
            "absent" => (CardStatus::Absent, None),
            "checking" => (CardStatus::Checking, None),
            "mounted" => (CardStatus::Mounted, None),
            "read-only" => (CardStatus::ReadOnly, reason),
            "failed" => (CardStatus::Failed, reason),
            _ => (CardStatus::Failed, Some(format!("unknown state: {first}"))),
        };
        let message = lines.next().unwrap_or_default().trim().to_string();
        let mut card = Card { status, reason, message, ..Card::default() };
        for (key, value) in lines.next().unwrap_or_default().split_whitespace().filter_map(|kv| kv.split_once('=')) {
            let value = (!value.is_empty()).then(|| value.to_string());
            match key {
                "card" => card.id = value,
                "check" => card.check = value,
                _ => {}
            }
        }
        card
    }

    /// Whether the games on it can be read (mounted, even read-only).
    pub fn readable(&self) -> bool { matches!(self.status, CardStatus::Mounted | CardStatus::ReadOnly) }

    /// Whether it can be written to (saves, settings).
    pub fn writable(&self) -> bool { self.status == CardStatus::Mounted }

    /// Whether the check before mounting repaired errors (the user is told once).
    pub fn repaired(&self) -> bool { self.status == CardStatus::Mounted && self.check.as_deref() == Some("repaired") }

    /// Whether the card the system runs from was pulled: the device is about to turn off.
    pub fn turning_off(&self) -> bool { self.status == CardStatus::Failed && self.reason.as_deref() == Some("system-card-removed") }

    /// What to tell the user: the system's message, or, if it gave none, a plain one for the
    /// status and the reasons Crystaline OS gives (empty for a mounted card with nothing to say).
    pub fn text(&self) -> String {
        if !self.message.is_empty() { return self.message.clone(); }
        match self.status {
            CardStatus::Absent => "No SD card. Insert the one with your games.".into(),
            CardStatus::Checking => "Checking the SD card…".into(),
            CardStatus::Mounted if self.repaired() => "Errors on the SD card were repaired.".into(),
            CardStatus::Mounted => String::new(),
            CardStatus::ReadOnly => "The SD card is read-only: games play, but nothing is saved. If it has a lock switch, slide it to unlock; \
                                     otherwise the card may be failing: copy your saves to a PC.".into(),
            CardStatus::Failed => match self.reason.as_deref() {
                Some("no-partition") => "This SD card has no FAT32 or exFAT partition. Use a card formatted as FAT32 or exFAT.",
                Some("mount") => "This SD card's files can't be read. Repair it on a PC, or format it as FAT32 or exFAT (this erases it).",
                Some("partition") => "The SD card couldn't be set up for games. Restart the device to try again.",
                Some("system-card-removed") => "The SD card holding the system was removed. Insert it and turn the device on again.",
                _ => "The SD card can't be used. Try another card.",
            }.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_state() {
        let c = Card::parse("read-only damaged\nGames play, nothing is saved.\ncard=1234abcd partition=sdcard type=vfat check=errors-left\n");
        assert_eq!(c, Card { status: CardStatus::ReadOnly, reason: Some("damaged".into()), message: "Games play, nothing is saved.".into(),
                             id: Some("1234abcd".into()), check: Some("errors-left".into()) });
        assert!(c.readable() && !c.writable());
        for (text, status) in [("absent\n", CardStatus::Absent), ("checking\n", CardStatus::Checking), ("mounted\n", CardStatus::Mounted),
                               ("failed mount\n", CardStatus::Failed)] {
            assert_eq!(Card::parse(text).status, status, "{text}");
        }
        assert!(Card::parse("failed system-card-removed\nThe Brick turned off.\n").turning_off());
    }

    #[test]
    fn empty_values_unknown_keys_and_missing_lines_are_tolerated() {
        let c = Card::parse("mounted\n\ncard= partition=sdcard type=exfat check=repaired colour=blue\n");
        assert_eq!((c.id, c.check.as_deref(), c.message.as_str()), (None, Some("repaired"), ""));
        assert_eq!(Card::parse("absent"), Card { status: CardStatus::Absent, ..Card::default() });
        assert!(Card::parse("mounted\n\ncheck=repaired").repaired());
    }

    #[test]
    fn an_unknown_state_is_a_failure_that_keeps_its_message() {
        let c = Card::parse("melting fast\nIt's melting.\n");
        assert_eq!((c.status, c.reason.as_deref(), c.text().as_str()), (CardStatus::Failed, Some("unknown state: melting fast"), "It's melting."));
        assert_eq!(Card::parse("").status, CardStatus::Failed);
    }

    #[test]
    fn a_missing_message_gets_a_plain_one() {
        assert_eq!(Card::parse("absent\n").text(), "No SD card. Insert the one with your games.");
        assert_eq!(Card::parse("failed no-partition\n").text(), "This SD card has no FAT32 or exFAT partition. Use a card formatted as FAT32 or exFAT.");
        assert_eq!(Card::parse("failed melted\n").text(), "The SD card can't be used. Try another card.");
        assert_eq!(Card::mounted().text(), "");
    }
}
