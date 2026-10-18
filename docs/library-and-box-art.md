# Library and box art

Status: In progress · Updated: 2026-10-18

How Specular finds the games on the card, works out their systems, identifies each ROM, and finds its box art. Art makes the library feel finished, but it must come from somewhere we may use, work offline once fetched, and never cost the UI a frame.

## Decisions

- **The library is scanned from `roms/`, in any folder layout** (2026-10-12). A game's system is worked out from its file extension, and from its header where an extension is shared. The folder name only breaks a tie the header leaves, for extensions several systems share (e.g. `.bin`, `.iso`, `.chd`) (2026-10-18). Rejected: always using folder names (cards are organised in too many ways).
- **Systems without a core are left out of the library** (2026-10-13). Rejected: listing their games and failing when one is launched.
- **Box art comes from libretro-thumbnails, downloaded on the device over WiFi** (2026-10-12). Box art only to start with; no PC companion yet. The art is the publishers' copyright, so it is never bundled or mirrored; RetroArch's on-demand download is the precedent we follow. Rejected: LaunchBox (no licence for third-party use), IGDB (its secret can't ship in open source), MobyGames (paid), TheGamesDB (name search only).
- **Plain HTTP** (2026-10-13). The server sends the same files either way. Rejected: TLS (about 1.5 MB more, root certificates to keep current, and a dependence on the clock being set).
- **Identification is offline, from tables compiled in** (2026-10-13). `tools/trim-db.py` trims libretro-database, pinned to one commit, to checksum, serial and name per system, in `data/db/`. The data is CC-BY-SA: it is credited in `data/db/NOTICE`, and the trimmed tables keep that licence.
- **The user's own art goes next to the ROM, with the same name** (2026-10-13): `roms/gba/Golden Sun.png`. PNG only for now. It is never replaced by downloaded art.
- **The whole library is fetched** (2026-10-13): games on screen first, then the rest in the background, one at a time, paused while a game runs.
- **Header codes identify ROM hacks as their base game** (2026-10-13), which gives them the base game's art. A GBA header's 4-letter game code is kept by hacks. Hacks and translations without one are matched by name with tags stripped (2026-10-18).
- **Other unusual games** (2026-10-18): an `.m3u` set is identified by its first disc; betas and prototypes by name with tags stripped. Arcade short names need a lookup table, added with the first arcade core.
- **A fuzzy name match is used only when it is unambiguous** (2026-10-18): one candidate left after dropping region and revision tags, and every number and roman numeral agrees, so sequels never collapse. Among region variants, the ROM's own region tag is preferred, else any region will do (box art differs little).
- **ScreenScraper is not used in V0** (2026-10-18). It may become an optional source later; it needs a developer ID for the project, each user's own account, and quota handling (see Findings).

## Design

### Scanning

`roms/` is scanned recursively on its own thread. Hidden folders, Windows' `System Volume Information` and symbolic links are skipped. A zip counts as the system of the ROM inside it. Files that only make up another game (the tracks a `.cue` lists, the discs an `.m3u` lists) aren't listed on their own. A missing or empty data folder is how a device looks without its SD card, and the library says so.

Linux's inotify watches `roms/` and every folder under it; a scan follows once no change has come for 2 s, so copying many files leads to one scan.

Display titles come from file names, which mostly follow No-Intro and Redump naming: everything from the first bracket on is removed, and a trailing ", The" moves to the front.

```rust
pub struct SystemDecl {
    pub id: &'static str,                     // stable; used in paths and settings, e.g. "gba"
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub libretro_name: &'static str,          // e.g. "Nintendo - Game Boy Advance"
    pub screen: (u32, u32),                   // native size, to place the game before its first frame
    pub game_db: &'static str,                // the trimmed table, compiled in
    pub header_code: Option<(u64, usize)>,    // where the header holds a game code (GBA: 0xAC, 4)
    pub core: Option<&'static CoreDecl>,      // None: its games aren't listed
    pub dump_header: u64,                     // a copier header left out of checksums (SNES: 512)
}
```

Built in: GB, GBC, GBA (with mGBA), and NES, SNES, Mega Drive and PS1 (no core yet). PS1 discs are listed by their `.cue`, `.chd`, `.pbp` or `.m3u`.

### Identifying a ROM

Once per file, offline. The result is the game's No-Intro/Redump name and how it was found (which says how sure the match is). It is kept in `cache/identify/<system>.json`, keyed by path and valid while the file's size and modification time are unchanged; the whole file is dropped when the game table's version changes.

Methods, in order (the later ones are planned):

1. **Checksum (CRC32).** From a zip's directory for free; otherwise hashed, for files of 64 MB or less. A dump header is skipped when the file's size is that much over a multiple of 1 KiB.
2. **Header game code**, where the system has one. Only codes of upper-case letters and digits are looked up (a homebrew ROM's header may be blank).
3. *Planned:* **disc serial**, for discs too large to hash: PS1 `SYSTEM.CNF`, PSP `PARAM.SFO`, the Saturn and Dreamcast headers.
4. **File name**: exact, then normalised (tags in brackets dropped, "the" dropped, "&" as "and", lower case, only letters and digits). Among equal matches, released games beat betas, prototypes and the like, then by region (USA, World, Europe, Japan), then originals before revisions. *Planned:* numerals made consistent, then fuzzy matching.

```rust
pub struct Identity { pub name: String, pub by: Method }
pub enum Method { Crc, HeaderCode, Name }
pub fn identify(path: &Path, system: &SystemDecl, db: &GameDb) -> io::Result<Option<Identity>>;
```

Today the per-system parts of identification are data in `SystemDecl`. With plugins, each step becomes an interface registered per system:

```rust
// Planned.
pub trait Identifier: Send + Sync { fn identify(&self, rom: &RomFile, db: &GameDb) -> Option<GameIdentity>; }
```

### Finding the art

Sources are tried in order; the first hit wins.

1. The user's image, next to the ROM.
2. `cache/art/`, which mirrors `roms/`.
3. *Planned:* art inside the ROM, for systems that have any: PSP `ICON0`/`PIC1`, DS and 3DS icons, the GameCube banner. Cartridge systems up to the SNES and Mega Drive have none.
4. libretro-thumbnails (needs the network).
5. *Planned:* a screenshot Specular takes when you leave a game.

Images are shrunk to at most 400×400 px as they are stored, so the UI only loads small, ready-to-use files. A loader thread decodes them for display, always working on the newest request, so scrolling never queues stale work.

```rust
pub trait ArtSource: Send {
    fn name(&self) -> &'static str;   // for logs
    /// The game's picture as PNG bytes, or None if this source has none.
    fn find(&self, game: &Game, identity: Option<&Identity>, system: &SystemDecl) -> Result<Option<Vec<u8>>, ArtError>;
}
pub enum ArtError {
    Offline(String),  // network or server unreachable, or a download cut off: ask again later
    Failed(String),   // counts as having no image
}
```

The thumbnail address is `http://thumbnails.libretro.com/<libretro system name>/Named_Boxarts/<name>.png`, where each of `` &*/:`<>?\|" `` in the name is replaced by `_`.

How the download queue behaves:

- Missing art is fetched whenever WiFi is up; with WiFi on Auto, the queue asks for WiFi while it has downloads waiting (see [wireless.md](wireless.md)).
- Downloads go one at a time, with an identifying User-Agent. *Planned:* honour ETags, so stored art can be refreshed without fetching unchanged images.
- After a network error the queue waits a minute. A game every source said it has no image for isn't asked about again for 30 days.
- A fuzzy name match must never replace an image the user supplied.

## Findings

- (2026-10-12, upstream) The libretro-database data for one system is mostly 1–8 MB. Source: libretro-database.
- (2026-10-13, desktop) The trimmed tables in `data/db/` are 0.1–0.6 MB each. Source: `data/db/`.
- (2026-10-12, upstream) A CHD's built-in checksum matches Redump only if it was made from the same tracks. Source: Redump (redump.org).
- (2026-10-12, upstream) ScreenScraper has better art and checksum lookups, but needs a developer ID for the project (requested from them), each user's own account, and has daily quotas. Source: ScreenScraper (screenscraper.fr).
