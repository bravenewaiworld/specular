# The SD card

Status: Implemented · Updated: 2026-10-20

How Specular follows the SD card as it comes and goes: the library, the settings and every save live on it, so a card pulled, inserted, checked, read-only or damaged changes what Specular can show and what it may write. The system mounts the card and says how it stands (Crystaline OS: `docs/storage.md`, "Keeping the card mounted"); Specular shows the user what that means, in the system's own words, and never writes where it can't.

## Decisions

- **The system's report is the truth; Specular never mounts or probes the card itself** (2026-10-19). Crystaline OS writes `/run/card/state` on every change, which Specular follows with inotify. A device without that file reports the card always mounted. Rejected: the earlier test "the data folder is missing or empty" (it can't tell a card being checked from a damaged one, and says nothing about read-only).
- **The card is a service like the radios** (`wireless::Wireless<CardCmd, Card>`) (2026-10-19), so the wireless trace records every state and every recheck (`wireless card …` lines), and a scenario can replay them on a desktop. Rejected: a trait of its own (a second trace and a second scenario format for the same job).
- **The system's message (line 2) is shown as it is, for every state** (2026-10-19): in place of the library while the card can't be read, at the top of home while it is read-only, and once, until dismissed with OK, after a repair. A plain message stands in if the system gives none. Rejected: Specular's own wording per reason (two places to keep in step).
- **A card that can be read and wasn't, or another card, is read afresh** (2026-10-19): its settings replace those in memory, its cores and games are scanned, and its folders watched anew (watches on a card that was unmounted never fire again). This happens on becoming mounted or read-only from any other status, or when the card's id changes. Until the new card's global settings are read, nothing acts on settings (the radios, the time zone lookup and its question), so a new card's defaults never ask or switch anything. Rejected: reading only the global settings again (the per-system, per-game and core layers are on the card too).
- **Nothing is written to a card that can't be written** (2026-10-19). Settings changed meanwhile stay in memory (and are replaced by the next card's); the in-game menu says it can't save a state; launching a game on a read-only card says its progress won't be saved; quitting, powering off or sleeping writes no saves. Rejected: writing and letting the writes fail (they would land on the bare mount point, or fail silently).
- **A game whose card goes waits for that card** (2026-10-19): it is paused under "Card removed", which asks for the card back. When the same card (by its CID) is back and writable, the game is saved and quit, back to the library. Quitting instead loses what wasn't saved, and so does a game its BIOS halts meanwhile. A game ended without saving leaves its launch record on its card; it is removed when that card can next be written (the record compared whole, so another card's is left alone), so the next start doesn't report the game as stopped unexpectedly. Rejected: playing on (disc games stream from the card, and nothing could be saved); saving to another card (its saves belong to that card's games).
- **A write refused as read-only (EROFS) while the card counts as writable has the system look at the card again** (2026-10-19), one recheck at a time, until the system answers: the kernel turns a damaged FAT card read-only without telling anyone, and `mount-card` then reports `read-only damaged`. The user is told at once. Writes from the settings, the game's saves (including those written in the background), the art cache and the schemas all count.
- **The library's reading is tagged** (2026-10-19): every message from a reading of the card (settings files, cores, games) is dropped once the card has been read afresh or forgotten, so a scan of the old card can't land after the new one's.
- **A card inserted later adds its fonts at the next start** (2026-10-20), as fonts added to a card do: Slint reads fallback fonts once ([look-and-motion.md](look-and-motion.md), "The card's fonts are named one by one"). Rejected: restarting Specular when a card with other fonts comes (a game or a screen in use would be cut short, only to fix names shown as empty boxes until the next start).

## Design

```rust
/// crates/specular-core/src/card.rs
pub struct Card {
    pub status: CardStatus,
    pub reason: Option<String>,   // read-only and failed: "damaged", "write-protected", "system-card-removed", …
    pub message: String,          // for the user, as is; may be empty (`Card::text` fills in)
    pub id: Option<String>,       // the CID: the same card back?
    pub check: Option<String>,    // "clean", "unclean", "repaired", "errors-left", "not-run"
}
pub enum CardStatus { Absent, Checking, Mounted, ReadOnly, Failed }
pub enum CardCmd { Recheck }
pub type CardService = dyn Wireless<CardCmd, Card>;

impl Card {
    pub fn parse(text: &str) -> Card;   // the state file; an unknown first line reads as Failed
    pub fn readable(&self) -> bool;     // Mounted or ReadOnly
    pub fn writable(&self) -> bool;     // Mounted
}

pub struct Hardware { /* … */ pub card: Option<Box<CardService>> }   // None: always mounted

// The app
enum Msg { Card(Card), CardReadOnly, Library(Vec<Game>), /* … */ }
enum Effect { ReadCard { writable: bool }, ForgetCard, Card(CardCmd), AbandonGame, /* … */ }
impl Ctx { pub card: Option<Card>, pub fn card_in(&self) -> bool }
```

Other screens read the card from `Ctx::card` (`Ctx::card_in` for "the card's files can be read"); file transfer runs only while it is in (see [remote-access.md](remote-access.md)).

| Card | Home | A game being played |
|---|---|---|
| not reported yet | "Looking for games…" | — |
| absent, checking, failed | the message in place of the library; a system's game list closes | paused under "Card removed" |
| mounted | the library; after a repair, the message and OK | plays |
| read-only | the message above the library | plays; can't save a state; quits without saving |
| failed `system-card-removed` | the message (the device turns off in ~3 s) | the message alone |

The card service runs on a thread of its own, as the radios do. `specular_linux::card_state::StateFile` reads the file at start and on each `IN_MOVED_TO` (also `IN_CLOSE_WRITE`, for a file written by hand, and `IN_DELETE`) of its name in its folder; a missing file is a mounted card. `Recheck` runs the device's command (the Brick: `/usr/libexec/brick/mount-card`), then reports the card, changed or not. The desktop follows `$SPECULAR_CARD_STATE` the same way, with a recheck that reports `read-only damaged` if the data folder has become read-only; or a scenario's `"card"` script.

`ReadCard` goes to the I/O thread, which works in order: saves asked for before are written to the card they were made for, and the card's settings files are read before any save asked for after. It migrates the config folder (only on a writable card), loads the settings layers, then starts a scan thread with a fresh inotify watch; `ForgetCard` (and the next `ReadCard`) stop that thread through an eventfd, so its watches on the old card are let go. The launch record left by an earlier run is reported at the run's first reading only.

## Findings

- (2026-10-19, desktop) A bind mount remounted read-only in a user namespace (`unshare -rm`) gives writes EROFS, as a FAT card the kernel turned read-only does, without root: enough to drive the recheck on a PC (DEVELOPMENT.md).
- (2026-10-19, upstream) The kernel's `errors=remount-ro` changes no mount table and sends no event; only a write failing as read-only shows it. Source: Crystaline OS `docs/storage.md`.

## Open questions

- Is the read-only message "always visible" enough at the top of home? It scrolls with home's list, and isn't on the game lists. A banner outside the list would need a new element in the UI; the Brick will show whether it is missed.
- Does a paused game survive its card going? mGBA maps the ROM file; a page of it not yet read, touched after the card is gone, would end Specular (SIGBUS). The game is paused at once, so the core shouldn't run; the device check in DEVELOPMENT.md (Checking the SD card on the Brick, step 4) would show it.
