# Library and box art

Status: In progress · Updated: 2026-10-20

How Specular finds the games on the card, works out their systems, identifies each ROM, and finds its box art (or title screen, screenshot, or the frame you left it on). Art makes the library feel finished, but it must come from somewhere we may use, work offline once fetched, and never cost the UI a frame.

## Decisions

- **The library is scanned from `roms/`, in any folder layout** (2026-10-12). A game's system is worked out from its file extension. Where systems share an extension (none of Specular's systems do yet; `.bin`, `.iso` and `.chd` are each used by several consoles), its header decides, and the folder name only breaks a tie the header leaves (2026-10-18). Rejected: always using folder names (cards are organised in too many ways).
- **Systems without a core are left out of the library** (2026-10-13). A core the user adds to the card counts (2026-10-18): the SNES and the Mega Drive appear once one serves them (see [runtime.md](runtime.md)). Their games are counted, not dropped silently (2026-10-19): home lists how many each such system has as needing attention, with how to add a core (see [guides-and-problems.md](guides-and-problems.md)). Rejected: listing their games and failing when one is launched.
- **Box art comes from libretro-thumbnails, downloaded on the device over WiFi** (2026-10-12), as are title screens and screenshots (2026-10-18). No PC companion yet. The art is the publishers' copyright, so it is never bundled or mirrored; RetroArch's on-demand download is the precedent we follow. Rejected: LaunchBox (no licence for third-party use), IGDB (its secret can't ship in open source), MobyGames (paid), TheGamesDB (name search only).
- **Plain HTTP** (2026-10-13). The server sends the same files either way. Rejected: TLS (about 1.5 MB more, root certificates to keep current, and a dependence on the clock being set).
- **Identification is offline, from tables compiled in** (2026-10-13). `tools/trim-db.py` trims libretro-database, pinned to one commit, to checksum, serial and name per system, in `data/db/`. The data is CC-BY-SA: it is credited in `data/db/NOTICE`, and the trimmed tables keep that licence.
- **The user's own art goes next to the ROM, with the same name** (2026-10-13): `roms/gba/Golden Sun.png`. PNG or JPEG (2026-10-18): `.png`, else `.jpg`, else `.jpeg`, so a PNG wins if several are there (it is lossless, and most likely made for the purpose). It is never replaced by downloaded art. Decoding uses the `image` crate with only its PNG and JPEG features, which Slint already builds in. Rejected: the JPEG decoder (`zune-jpeg`) on its own beside the `png` crate (the same code, plus our own glue for each format).
- **The whole library is fetched** (2026-10-13): games on screen first, then the rest in the background, one at a time, paused while a game runs.
- **Header codes identify ROM hacks as their base game** (2026-10-13), which gives them the base game's art. A GBA header's 4-letter game code is kept by hacks. Hacks and translations without one are matched by name with tags stripped (2026-10-18).
- **Other unusual games** (2026-10-18): an `.m3u` set is identified by its first disc; betas and prototypes by name with tags stripped. Arcade short names need a lookup table, added with the first arcade core.
- **A fuzzy name match is used only when it is unambiguous** (2026-10-18): one candidate left (names with their tags dropped), and every number and roman numeral agrees, so sequels never collapse. Among region variants, the ROM's own region tag is preferred, else any region will do (box art differs little).
  - A candidate is a title: a normalised name, which drops every tag, so a game's discs, betas and language variants are one candidate, chosen between as for exact matches (2026-10-18). Rejected: dropping only region and revision tags (a game with a beta, or several discs, would always be ambiguous).
  - A title is a candidate when it holds every word of the file's name, in order (2026-10-18): "Zelda Minish Cap" finds "Legend of Zelda, The - The Minish Cap". Rejected: the reverse, a title inside the file's name (a short title such as "Tank" would claim "Super Tank Battle"), and edit distance (hard to predict, and similar names are often different games).
  - Roman numerals from I to XXXIX count as numbers, in normalised names too, so "Mega Man II" and "Mega Man 2" are the same name (2026-10-18). Rejected: larger numerals (words such as "Mix" and "DC" would become numbers).
  - The ROM's own region is also preferred among exact normalised matches, and is read from No-Intro tags ("(Europe)") and GoodTools' letters ("(E)", "(UE)") (2026-10-18).
- **How each game was found is recorded** (2026-10-18): the identify cache keeps the method; the log names each fuzzy match and each game not found (with why), and counts the rest. Rejected: a log line per game (thousands on a first scan). The cache carries the version of the matcher as well as the table's, so a Specular that matches differently identifies every game again (art already stored is kept).
- **PS1 discs are identified by the serial their boot file is named after** (2026-10-18), read from `SYSTEM.CNF` (or a PBP's `DISC_ID`), which only takes a few sectors. Redump lists some serials with suffixes (`SLUS-01272GH`, `SLES-02375-P`); `tools/trim-db.py` cuts them to the boot file's form. A `.chd` isn't read; it is matched by name. Rejected for now: the `chd` crate (pure Rust, but it brings its own LZMA, zlib and FLAC decoders to read two sectors).
- **ScreenScraper is not used in V0** (2026-10-18). It may become an optional source later; it needs a developer ID for the project, each user's own account, and quota handling (see Findings).
- **Search covers every system, and shows its results on a screen of their own** (2026-10-18). Search on Home, or X in any game list, opens the on-screen keyboard; confirming the text (Start) shows the matching games, best first, each with its system, as a list or grid like a system's games. Rejected: searching only the system being shown (narrower, and the game you search for is often one whose system you don't remember), filtering the list as you type (needs a new layout with the keyboard and a list on one screen).
- **Search matches titles the way people type them** (2026-10-18). Case, accents, spaces and punctuation are ignored. Titles that start with the query come first, then those with a word that starts with it (or, for several words, a word starting with each, in any order), then those that contain it anywhere, even across words; by title within each.
- **A system's games may be shown with their title screens, screenshots, or where you left them, instead of their box art** (2026-10-18). One setting, "Art" (Library), settable per system (arcade games, say, are known by their title screens): Box art (the default), Title screen (libretro-thumbnails' `Named_Titles`), Screenshot (`Named_Snaps`) or Where you left off (the last frame shown when the game was quit). A game without a picture of its kind shows its box art, else its generated cover; the user's own image beside the ROM counts as box art. Rejected: a setting per game (a game's own box art can already be put beside it); falling back through every kind (which picture a game shows would be harder to predict, and more would be downloaded).
- **Only the kind each system shows is downloaded, then box art for the games without one** (2026-10-18), with the same queue rules (online only, a minute's wait after a network error, 30 days before asking again for a kind a game has none of). Changing the setting makes the worker go through the library again. Rejected: fetching every kind for every game (three times the downloads and the cache, for pictures that may never be shown).
- **The frame on screen as a game is quit is kept as where you left off** (2026-10-18), for every game whatever the setting, so choosing "Where you left off" shows pictures at once. The art worker shrinks and stores it like other art, before any other work, so the quit animation never waits; it is written as other cached pictures are (under another name, then renamed, but not flushed to the card): one lost to a power cut is replaced at the next quit. A game quit during its splash, before it was shown, keeps its previous picture of where it was left off. With "Where you left off" shown, the game shrinks into that frame, and the list shows it at once. Rejected: keeping it on sleep too (the game is resumed after waking, and its quit keeps the frame then; only a battery that dies while asleep leaves the older picture); keeping it only for systems set to show it (choosing it would show nothing until each game was played again).
- **Each kind has its own folder, `cache/art/<kind>/`, mirroring `roms/`** (2026-10-18): `boxart/`, `title/`, `screenshot/` and `left-off/`. Box art cached when `cache/art/` held only box art is moved into `cache/art/boxart/` at start-up, one entry of `cache/art/` at a time (one rename each, so a power cut leaves the rest for the next start). Rejected: box art staying in `cache/art/` beside the other kinds' folders (a folder in `roms/` could have a kind's name); downloading the box art again (a library's worth of downloads).
- **Pictures are drawn in their kind's shape** (2026-10-18): box art in squares (boxes vary in shape), the other kinds in the shape of their system's screen, in the grid's cells and beside the list, so title screens and screenshots fill their places and fly into the running game without changing shape (see [look-and-motion.md](look-and-motion.md)).
- **Game lists have a letter index** (2026-10-18): L2 and R2 jump to the previous or next first letter (see [look-and-motion.md](look-and-motion.md)). Search results are ranked rather than in title order, so they have none.

## Design

### Scanning

`roms/` is scanned recursively on its own thread. Hidden folders, Windows' `System Volume Information` and symbolic links are skipped. A zip counts as the system of the ROM inside it, and is unzipped into memory to be played (see [runtime.md](runtime.md)); a zip64 one (4 GiB or more) isn't listed. Files that only make up another game (the tracks a `.cue` lists, the discs an `.m3u` lists) aren't listed on their own. The card is scanned once the system says it can be read, and again when it comes back (see [sd-card.md](sd-card.md)); while it can't, the library shows the system's message instead.

Linux's inotify watches `roms/` and every folder under it; a scan follows once no change has come for 2 s, so copying many files leads to one scan.

Display titles come from file names, which mostly follow No-Intro and Redump naming: everything from the first bracket on is removed, and a trailing ", The" moves to the front. Games sort by title without a leading "The", ignoring case and accents, with numbers by value ("Mega Man 2" before "Mega Man 10"). The letter index files each game under the first letter of its title as sorted (no "The", no accents), with digits, symbols and letters outside A–Z under "#".

```rust
pub struct SystemDecl {
    pub id: &'static str,                     // stable; used in paths and settings, e.g. "gba"
    pub name: &'static str,
    pub tint: &'static str,                   // a palette colour, for its screens (look-and-motion.md)
    pub extensions: &'static [&'static str],
    pub libretro_name: &'static str,          // e.g. "Nintendo - Game Boy Advance"
    pub libretro_id: &'static str,            // its systemid in libretro's core info files, e.g. "game_boy_advance"
    pub screen: (u32, u32),                   // native size, to place the game before its first frame
    pub game_db: &'static str,                // the trimmed table, compiled in
    pub header_code: Option<(u64, usize)>,    // where the header holds a game code (GBA: 0xAC, 4)
    pub disc_serial: Option<fn(&Path) -> io::Result<Option<String>>>,  // reads a disc's serial (PS1)
    pub cores: Vec<&'static CoreDecl>,        // its own first, then the user's; empty: its games aren't listed
    pub dump_header: u64,                     // a copier header left out of checksums (SNES: 512)
}
```

Built in: GB, GBC and GBA (with mGBA), NES (Nestopia), PS1 (SwanStation), and SNES and Mega Drive (no core of Specular's yet; a core on the card can serve them). The library is scanned with the systems that have a core, which the cores on the card can change: they are looked for before each scan, and a change in `cores/` leads to a scan too. PS1 discs are listed by their `.cue`, `.chd`, `.pbp` or `.m3u`.

### Search

The results screen's params are the query, so a rescan rebuilds the results rather than losing them. Matching runs on the UI thread each time the screen is built, so it reuses one buffer for every title (see Findings).

```rust
pub fn search<'a>(games: &'a [Game], query: &str) -> Vec<&'a Game>;  // best first
pub fn index_letter(title: &str) -> char;                             // 'A'–'Z' or '#'
```

### Identifying a ROM

Once per file, offline. The result is the game's No-Intro/Redump name and how it was found (which says how sure the match is). It is kept in `cache/identify/<system>.json`, keyed by path and valid while the file's size and modification time are unchanged; the whole file is dropped when the game table's version or the matcher's (`IDENTIFY_VERSION`) changes.

Methods, in order:

1. **Checksum (CRC32).** From a zip's directory for free; otherwise hashed, for files of 64 MB or less. A dump header is skipped when the file's size is that much over a multiple of 1 KiB.
2. **Header game code**, where the system has one. Only codes of upper-case letters and digits are looked up (a homebrew ROM's header may be blank).
3. **Disc serial**, for systems on discs, which aren't hashed (an image is too large, and a `.cue` or `.m3u` isn't the disc itself). PS1: the boot file `SYSTEM.CNF` names (`BOOT = cdrom:\SLUS_012.34;1` is `SLUS-01234`), from the ISO 9660 filesystem of a `.bin` or `.iso` (raw 2352-byte sectors, Mode 1 or 2, or 2048-byte ones), a `.cue`'s first file or an `.m3u`'s first disc; or the `DISC_ID` in a `.pbp`'s `PARAM.SFO`. A disc that boots `PSX.EXE` has no serial. *Planned:* PSP `PARAM.SFO`, the Saturn and Dreamcast headers.
4. **File name**: exact, then normalised (tags in brackets dropped, "the" dropped, "&" as "and", roman numerals up to XXXIX as digits, lower case, only letters and digits). Among equal matches, released games beat betas, prototypes and the like, then the ROM's own region (from its tags), then by region (USA, World, Europe, Japan), then originals before revisions.
5. **Fuzzy name**: the one title (normalised name) that holds every word of the file's, in order, and the same numbers. None if several do (ambiguous), or only titles with other numbers (a sequel, or the first game of a series).

```rust
pub struct Identity { pub name: String, pub by: Method }
pub enum Method { Crc, HeaderCode, DiscSerial, Name, FuzzyName }
pub fn identify(path: &Path, system: &SystemDecl, db: &GameDb) -> io::Result<Option<Identity>>;

impl GameDb {
    pub fn by_code(&self, code: &str) -> Option<&str>;   // header codes and disc serials
    pub fn by_name(&self, stem: &str) -> Result<(&str, Method), NoMatch>;
}
pub enum NoMatch { Unknown, Ambiguous(usize, Vec<String>), Numbers(String) }  // why, for the log
pub fn ps1_serial(path: &Path) -> io::Result<Option<String>>;
```

The log has a line for each game found by fuzzy name, and for each not found, saying why; the art worker adds a count per method each time it runs out of work. The other methods are certain enough to only be counted.

Today the per-system parts of identification are data in `SystemDecl`. With plugins, each step becomes an interface registered per system:

```rust
// Planned.
pub trait Identifier: Send + Sync { fn identify(&self, rom: &RomFile, db: &GameDb) -> Option<GameIdentity>; }
```

### Finding the art

A game shows the first picture there is of its system's kind (the "Art" setting), then of box art, then a generated cover. Each kind's pictures come from:

| Kind (setting value) | From, in order |
|---|---|
| Box art (`boxart`) | The user's image next to the ROM (PNG or JPEG); `cache/art/boxart/`; *planned:* art inside the ROM, for systems that have any (PSP `ICON0`/`PIC1`, DS and 3DS icons, the GameCube banner; cartridge systems up to the SNES and Mega Drive have none); libretro-thumbnails' `Named_Boxarts` (needs the network) |
| Title screen (`title`) | `cache/art/title/`; libretro-thumbnails' `Named_Titles` |
| Screenshot (`screenshot`) | `cache/art/screenshot/`; libretro-thumbnails' `Named_Snaps` |
| Where you left off (`left-off`) | `cache/art/left-off/`, the frame on screen when the game was last quit |

Each folder in `cache/art/` mirrors `roms/`: `roms/gba/Golden Sun.gba`'s title screen is `cache/art/title/gba/Golden Sun.png`. Images are shrunk to at most 400×400 px as they are stored (as PNG), so the UI only loads small, ready-to-use files; the user's own are shrunk as they are loaded. A loader thread decodes them for display, always working on the newest request, so scrolling never queues stale work. A picture that would take more than 64 MB to decode (a large photo) is refused.

```rust
pub enum ArtKind { BoxArt, TitleScreen, Screenshot, LeftOff }
impl ArtKind {
    pub fn id(self) -> &'static str;                        // the setting's value and the cache folder: "boxart", "title", …
    pub fn thumbnails(self) -> Option<&'static str>;        // "Named_Boxarts", …; None for LeftOff
    pub fn fallbacks(self) -> &'static [ArtKind];           // [self, BoxArt]
    pub fn shape(self, system: &SystemDecl) -> (u32, u32);  // (1, 1) for box art, else the system's screen
}
pub fn kind(settings: &Settings, system: &str) -> ArtKind;
pub fn files(roms: &Path, cache: &Path, rom: &Path, kind: ArtKind) -> Vec<PathBuf>;     // where one kind's picture may be
pub fn pictures(roms: &Path, cache: &Path, rom: &Path, kind: ArtKind) -> Vec<PathBuf>;  // `files` of each fallback, in order
pub fn move_old_cache(cache: &Path);  // box art from cache/art/ into cache/art/boxart/ (logged)

pub trait ArtSource: Send {
    fn name(&self) -> &'static str;   // for logs
    /// The game's picture of `kind` as PNG or JPEG bytes, or None if this source has none.
    fn find(&self, game: &Game, identity: Option<&Identity>, system: &SystemDecl, kind: ArtKind) -> Result<Option<Vec<u8>>, ArtError>;
}
pub enum ArtError {
    Offline(String),  // network or server unreachable, or a download cut off: ask again later
    Failed(String),   // counts as having no image
}
```

The thumbnail address is `http://thumbnails.libretro.com/<libretro system name>/<Named_Boxarts, Named_Titles or Named_Snaps>/<name>.png`, where each of `` &*/:`<>?\|" `` in the name is replaced by `_`.

How the download queue behaves:

- Missing art is fetched whenever WiFi is up; with WiFi on Auto, the queue asks for WiFi while it has downloads waiting (see [wireless.md](wireless.md)).
- Each game's system's kind is fetched, then box art if no source has that kind's picture; nothing more once the game has a picture of either. The UI tells the worker each system's kind (`ArtCmd::Kinds`) whenever they change.
- Downloads go one at a time, with an identifying User-Agent. *Planned:* honour ETags, so stored art can be refreshed without fetching unchanged images.
- After a network error the queue waits a minute. A game every source said it has no picture of a kind for isn't asked for that kind again for 30 days (the index keeps a date per kind: `"missing": {"title": <seconds>}`).
- A game with the user's own image is skipped before it is identified, so no match, fuzzy or not, replaces it, unless its system shows another kind.

As a game is quit, the UI takes the frame on screen from the runtime (`FrameSlot::take_shown`: the frame the UI drew last is kept aside, not reused) and sends it to the art worker (`ArtCmd::LeftOff`), which stores it before any other work.

## Findings

- (2026-10-12, upstream) The libretro-database data for one system is mostly 1–8 MB. Source: libretro-database.
- (2026-10-13, desktop) The trimmed tables in `data/db/` are 0.1–0.6 MB each; PS1's is 0.65 MB (11,000 discs). Source: `data/db/`.
- (2026-10-18, upstream) Redump's PS1 list has a serial for nearly every disc, though some carry suffixes (`SLUS-00975GH`, `SLES-01324/P`) or aren't serials (`Part-905154`, demo discs' `LSP-990123-0.1`). Source: libretro-database `metadat/redump/Sony - PlayStation.dat`.
- (2026-10-12, upstream) A CHD's built-in checksum matches Redump only if it was made from the same tracks. Source: Redump (redump.org).
- (2026-10-12, upstream) ScreenScraper has better art and checksum lookups, but needs a developer ID for the project (requested from them), each user's own account, and has daily quotas. Source: ScreenScraper (screenscraper.fr).
- (2026-10-18, desktop) Searching 3,000 titles takes about 0.5 ms in a release build (the fastest of 100 runs on a busy PC). Source: the ignored benchmark in `crates/specular-core/src/library/search.rs`.
- (2026-10-18, desktop) Against the real tables, shortened names match by fuzzy name ("Pokemon Emerald", "Zelda Minish Cap", "Castlevania II", "Advance Wars 2"); generic ones are rejected as ambiguous ("Metroid", "Kirby", "Mario Kart" on GBA), and "Super Mario Advance 4" because the title has more numbers. Words split or joined differently aren't matched ("Pokemon Fire Red" for "FireRed", "Megaman 2"). A fuzzy search takes under 1 ms per game in a release build on a PC (PS1, the largest table). Source: a throwaway test over `data/db/`.
