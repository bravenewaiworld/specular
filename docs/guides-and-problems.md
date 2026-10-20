# Guide files and problems

Status: Implemented · Updated: 2026-10-19

How the SD card and the device explain themselves, under the rule "No Manuals" (README.md): everything the user sees or must act on is obvious, or explained where it is presented or in a file beside it. On the card, each folder the user fills has a `README.txt` saying what goes in it and how. On the device, what Specular can't use (games no emulator plays, a core or font turned away, a settings file it can't read or values in one it ignores, settings it can't save) is listed on one screen, each with why and what to do, and home says how many there are.

## Decisions

- **Specular writes a guide into each folder of the card the user fills** (2026-10-19; the project owner's choice): `roms/`, `bios/`, `cores/`, `fonts/` and `cache/` each get a `README.txt`, and so does each old config folder kept after migrating (`configs/`, later `config/vN/`), saying it is no longer read. They are made from the declarations (the systems and their file types, the cores and their BIOS files, the processor cores must be built for), so they can't drift from the code, and are written as the schemas are: read-only, compared before writing, never on a card that can't be written, at start-up and whenever the cores change. Each starts with a line saying Specular writes it and that changes to it are lost. Writing one makes its folder, so a fresh card has every folder the user fills. Crystaline OS writes the card's own `README.txt` and those in `updates/` and `backups/`; Specular leaves those alone. Rejected: a manual on a web page (the rule); one README at the card's root (it is the system's, and a file beside what it explains is found where it is needed); guides written once and left (they would go stale as cores are added).
- **What Specular can't use is a problem, listed with why and what to do** (2026-10-19; the project owner's choice). A `Problem` is what (a row's label), why, and a fix that says what the user can do; one about a turned-away core opens the Cores screen. Home shows "N things need attention" as a plain row under Settings, only while there are any (quiet: no warning colour, as an unusable file is not an emergency); it opens the list. Each problem is still logged in full where it is found. Rejected: a notice per problem (gone before it is read, and many at once at start-up); a warning colour or badge (look-and-motion's quiet look).
- **Problems are what Specular knows now, not a history** (2026-10-19). They are worked out from the app's state when shown: the latest scan's findings, the settings layers in memory, the cores and fonts turned away, the last settings write. A settings file set aside (`<file>.bad`) is found by looking for `.bad` files at each scan and reading each again for why, so it stays a problem, across restarts, until the user fixes or deletes it. Those of the card's files are shown only while it can be read, and a card read afresh starts with none of the last card's. Rejected: collecting problems as they happen (a restart would forget one still there; one put right would stay listed).
- **Unknown settings keys are not problems** (2026-10-19): a key no declaration knows may be a plugin's, or a newer version's (see [settings.md](settings.md)). A value of a declared setting that isn't valid, or a setting that can't be set in its file, is.
- **The Cores screen is for everyone** (2026-10-19): Settings → Cores (after Credits) lists every core and the files in `cores/` turned away, with why. Rejected: keeping it behind Developer details (a user who added a core needs to see why it isn't used).

## Design

```rust
/// crates/specular-core/src/guides.rs
pub const README: &str = "README.txt";
/// By path relative to the card. `systems`: every system, those no emulator plays too, each with
/// its cores (`user_cores::served`); `old_folders`: `settings::migrate::old_folders`.
pub fn guides(systems: &[SystemDecl], old_folders: &[&str]) -> Vec<(PathBuf, String)>;

/// crates/specular-core/src/settings/store.rs
pub fn store_generated(path: &Path, text: &str) -> io::Result<bool>;   // schemas and guides
pub fn set_aside(config: &Path) -> Vec<(PathBuf, String)>;            // the `.bad` files, and why

/// crates/specular-core/src/library/mod.rs
pub struct Scan { pub games: Vec<Game>, pub unplayable: Vec<Game> }   // unplayable: systems with no core
impl Scan { pub fn no_emulator(&self, systems: &[SystemDecl]) -> Vec<(&'static str, usize)>; }   // by name
pub fn scan(roms: &Path, systems: &[SystemDecl]) -> Scan;   // a system with a core wins a shared file type

/// crates/specular-core/src/app/problems.rs
pub struct Problem { pub what: String, pub why: String, pub fix: String, pub opens: Option<ScreenId> }
pub struct LeftOut { pub no_emulator: Vec<(&'static str, usize)>, pub set_aside: Vec<(PathBuf, String)>, pub orphans: Vec<String> }
enum Msg { LeftOut(LeftOut), SettingsSaved(Result<(), String>), /* … */ }
impl App {
    pub fn problems(&self) -> Vec<Problem>;
    pub fn set_fonts_turned_away(&mut self, fonts: Vec<TurnedAway>);
}
impl Settings { pub fn ignored(&self) -> Vec<(Layer, String, String)>; }   // layer, key, why
```

| Problem | Found by | Cleared when |
|---|---|---|
| A system's games no emulator plays (a count per system) | the scan (`Scan::no_emulator`) | a core for it is added |
| A file in `cores/` turned away | the look for cores (`Msg::Cores`) | it is fixed or removed |
| A settings file set aside as `.bad` | the scan (`store::set_aside`) | the `.bad` file is gone (renamed back or deleted) |
| A settings value ignored (file, key) | the layers in memory (`Settings::ignored`) | it is set on the device, or fixed in the file and the card read again |
| A file under `config/v0/games/` for no game | the scan (`load_games_layers`) | its game is back, or it is renamed or deleted |
| A file in `fonts/` turned away | the fonts read at start-up | the next start without it |
| The last settings save failed | the I/O thread (`Msg::SettingsSaved`, one per batch of files saved together) | a later save succeeds (of any file: the failed file is saved again only when its settings next change) |

The scan thread posts `Msg::LeftOut` before each scan's `Msg::Library`, and writes the guides with the schemas. Writing a guide into a watched folder (`roms/`, `cores/`) makes one more scan, which finds the guide unchanged and writes nothing, so it doesn't loop. A settings file of a game no emulator plays isn't reported as matching no game. The library scan passes over `README.txt` (no system takes `.txt`), and so does the look for cores (only `.so` and `.info` files count); the fonts leave out `fonts/README.txt`. Migrating skips a `README.txt` in the old folder, which would be wrong in the new one.

## Open questions

- The config folder isn't watched, so a `.bad` file renamed back or deleted stays listed until the next scan (a change to `roms/` or `cores/`, or a restart). Watching `config/v0/` would clear it at once; is the delay noticed?
- Is "N things need attention" seen where it is, under Settings at the bottom of home, on a long library? Placing it above the systems would be louder; the Brick will show.
