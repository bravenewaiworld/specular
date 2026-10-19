# Settings

Status: In progress · Updated: 2026-10-18

How settings are declared, shown, layered and stored on the SD card, how cores' options become settings, how their files change format over time, and how the card is laid out. The UI shows only the few settings people genuinely disagree on; everything else is still reachable, documented, in plain files on the card.

## Decisions

- **Settings are declared once** (2026-10-12): a key, type, default, label, section and who sees it. The settings screens are generated from the declarations, so a setting needs no hand-written UI.
- **A setting is shown only where people genuinely disagree** (2026-10-12). Everything else has a good default or is detected automatically. Internals (frame time, GPU, memory, file paths) appear only when "Developer details" is on; battery, storage and WiFi are shown in plain words.
- **Layers: built-in default < global < per system < per game** (2026-10-12). Each setting says how specific it may be (brightness is global only; scaling may differ per game).
- **Only values that differ from the default are stored** (2026-10-12), so changing a default in a new version reaches everyone who never changed it. Unknown keys (a plugin not loaded, a newer version's settings) are kept untouched. A stored value of the wrong type or out of range is ignored, as if absent.
- **Writes survive a power cut on FAT32** (2026-10-12). FAT32 has no journal, so a file is never rewritten in place: write a temporary file, fsync it, rename it over the original, then fsync the folder.
- **Settings live in a versioned folder: `{sd card}/config/v0/<name>.json`** (2026-10-18). Every configuration (Specular's own interface, each system, each core, each game) is a file there, so everything not in the UI can still be changed. The `.json` holds only the user's overrides. Next to the files, Specular writes fully documented JSON Schemas carrying every setting and its default (and, once there are any, values Specular works out by itself), so users can see what they can set and to what. Defaults stay out of the `.json` so that a changed default still reaches everyone who never overrode it. Rejected: filling every default into the `.json` (a later default change couldn't be told apart from a user's choice). Each future format change gets a new `vN/` folder, migrated from the previous one.
- **A system's games share one schema, `games/<system id>.schema.json`** (2026-10-18), as a game's settings depend only on its system (and so its core). Rejected: one beside each game's file (a second file for every game with settings), and one for every game in the library (thousands of identical files to write and keep current).
- **Schemas are generated, read-only and rewritten only when they change** (2026-10-18). They are built from the declarations at every start, so they can't drift from the code; each says it is generated, that edits to it are ignored and that it may be overwritten. The read-only mark (on FAT32, the read-only attribute) is a hint: Specular still replaces the file, and a card that refuses the mark is only logged. Comparing before writing spares the card a write per schema per start. Each setting's description comes from its declaration's `help`, written for people editing the files by hand.
- **Each settings file names its schema in a `"$schema"` key** (2026-10-18; the project owner's choice), the schema's path relative to the file, so editors such as VS Code find it with no setup. Specular writes it on every save and drops it on reading: it is neither a setting nor an unknown key to keep. Rejected: leaving editors to match files to schemas by name (each user would have to set that up, per editor).
- **Schemas allow keys they don't list** (2026-10-18), as Specular keeps unknown keys (a plugin's settings, a newer version's). The cost: an editor doesn't flag a misspelt key. Rejected: `"additionalProperties": false` (it would mark valid files as wrong).
- **A migration builds the new folder aside, then renames it into place** (2026-10-18), so a power cut never leaves a half-migrated folder that looks complete; a leftover `.tmp` folder is discarded and the step redone. Migrations are an ordered list of steps, one per format change. A damaged file is copied as it is, and reading then sets it aside as for any other file. If a step fails, nothing is written to the new folder until the next start retries it. Rejected: copying file by file into `config/v0/` (a cut-off copy would be taken as finished), checking files while migrating (damaged files would be handled in two places), and carrying on with an empty `config/v0/` after a failure (the old settings would be ignored from then on).
- **One file per scope** (2026-10-18): the interface and global settings, each system, each core's options, and each game's overrides (laid out under Design). Rejected: one file per system holding its core options and per-game overrides (fewer files, harder to edit by hand).
- **The old folder is kept after migrating** (2026-10-18): `configs/`, and each older `vN/`, stay untouched, so a downgrade still finds its settings (as they were when migrated) and a migration bug loses nothing. Rejected: deleting it after migrating.
- **Core options are settings** (2026-10-18), declared from the core's own option list, stored in `cores/<core>.json`, and overridable per system and per game. They are in config files only, unless people genuinely disagree about one; then it gets a row in the UI. With "Developer details" on, the game and system settings screens list the core's options too (they are developer settings).
- **A game's settings open from the game** (2026-10-18; the project owner's choice): Start on a focused game (in a system's list or grid, and in search results) opens its settings, and the in-game menu has a "Game settings" row opening the same screen over the paused game. It lists the settings that may be set per game, then "All <system> games…", the system's screen (the settings that may be set per system). Rows that override are marked; X resets one (its hint shows only on those rows). A change applies to the running game at once where it can: scaling, and the core's options (see Core options). Rejected: systems and games listed under Settings (a second library to find a game in).
- **Every layer is loaded before anything can use it** (2026-10-18). At start-up the I/O thread loads the global layer, then each system's and each core's (a handful of files). Games' are loaded by the scan thread, before it posts the library: their games can't be listed, played or have their settings shown before. Only `games/` is walked, which holds a file only for each game with settings, so the cost doesn't grow with the library; a rescan loads the files of games new to it. Rejected: loading a game's layer on demand, before its launch or its screen (the app would wait for a file at each, a state of its own for every way in); looking up each game's file after a scan (a lookup on the card per game, thousands).
- **The options of the cores Specular builds are settled when they are built** (2026-10-18; the project owner's choice). `make cores`, building for this PC, loads each core and writes what it declares (key, description, values and their labels, the core's default, its category and info text) to `data/cores/<core>.json`, which is checked in and compiled in. From it come the core's setting declarations, so its options are in `cores/<core>.json`'s schema, documented; and constants for Specular's own code (`settings::core_options::mgba::IDLE_OPTIMIZATION`), so a renamed option fails the build rather than being ignored. A test checks each list against the core built for this PC. At launch, the runtime compares what the core declares with its list and logs the differences: a core can declare other options, or other defaults, on another processor (recompilers on ARM and on x86). Rejected: asking each core at start-up, in a helper process (a process per core at every start, and no declarations, schemas or constants until it answers) or in Specular's own (a core's code would run at start-up, where a crash or a hang takes the menus down, and its globals are shared with the game it may be playing); reading the lists from the cores' sources (some values are only filled in at run time, such as mGBA's palettes).
- **A core's option is looked up in the game's file, the system's, then the core's file, then its default** (2026-10-18). The global file plays no part: `cores/<core>.json` stands where it does for Specular's own settings. The values Specular chose before options were settings (mGBA's idle loop removal, Nestopia's NTSC filter off, SwanStation's software renderer) are Specular's defaults for those options, and their schemas say so. A core is given only the values set in a layer or given a default by Specular; its other options keep its own defaults. Rejected: giving it every option's value (the defaults of the core built for this PC would override those of the device's build).

## Design

### Declarations

```rust
pub struct SettingDecl {
    pub key: &'static str,          // e.g. "video.scaling"; never renamed once released
    pub section: &'static str,      // the header it is listed under, e.g. "Display"
    pub label: &'static str,
    pub help: &'static str,         // what it does and what its values mean, for its schema
    pub kind: SettingKind,
    pub default: serde_json::Value,
    pub visibility: Visibility,
    pub deepest: Scope,             // the most specific layer allowed to override it
    pub core: Option<&'static str>, // whose option it is (`CoreDecl::id`); None: Specular's own
}
pub enum SettingKind {
    Bool,
    Choice(&'static [(&'static str, &'static str)]),  // (stored value, label)
    Colour { same_as: Option<&'static str> },          // a palette id, or "same" as another colour setting
    Int { min: i64, max: i64, step: i64, unit: &'static str },
}
pub enum Visibility { User, Developer, OwnScreen }    // OwnScreen: shown by a screen of its own (Wi-Fi)
pub enum Scope { Global, System, Game }
pub enum Layer { Global, Core(String), System(String), Game { system: String, game: String } }

impl Settings {
    pub fn get(&self, key: &str, at: &Layer) -> Option<(&Value, Option<Layer>)>;  // and where it comes from
    pub fn set(&mut self, key: &str, at: &Layer, v: Value) -> Result<(), SettingError>;
    pub fn reset(&mut self, key: &str, at: &Layer) -> Option<&'static str>;     // inherit again
    pub fn rows(&self, at: &Layer, show: Visibility, core: Option<&str>) -> Vec<Row>; // Specular's, or one core's
}
pub fn declarations(systems: &[SystemDecl]) -> Vec<SettingDecl>;   // Specular's, then each core's options
pub fn ignored(decls: &[SettingDecl], layer: &Layer, values: &Map<String, Value>) -> Vec<String>;
pub fn games_with_settings(config: &Path) -> Vec<String>;          // from the files under games/
```

A value is looked up from the most specific layer that has a valid one, down to the default: the game's, the system's, then the global layer for Specular's settings or the core's (`Layer::Core`, whose scope is `Global`) for its options. Setting a value equal to what the layer would inherit removes the override. Settings rows mark values that override a less specific layer. Specular's own declarations are in `specular_core::settings::builtin`.

### Game and system settings

`app::layers` has the two screens: `GAME_SETTINGS` (a game's path under `roms/` in its params) and `SYSTEM_SETTINGS` (a system's id). Each shows `rows(at, show, None)`, the game's screen then a header and "All <system> games…", then, with Developer details, `rows(at, show, Some(core))`. A row that overrides gets a hint of its own, shown while it is focused (`Row::hint`: X Reset). Presses return `Outcome::SettingChanged(layer, key)`, and the app saves the layer and applies the setting where it takes effect: `Effect::Apply` for Specular's (at the running game's layer while one is played), `Effect::CoreOptions` for an option of the running game's core. A game starting is sent `Apply` for each setting that may differ per system or game. Over the paused game, the screens count as its menu (buttons go to the app), B goes back to the menu and Menu resumes the game.

```rust
Outcome::SettingChanged(Layer, &'static str)    // a screen changed (or reset) a setting at that layer
Effect::CoreOptions(Vec<(String, String)>)      // the running game's core options now, as App::core_options
impl App { pub fn core_options(&self) -> Vec<(String, String)>; }  // core_options::values at the game's layer
pub struct Row { /* … */ pub hint: Option<(Button, String)> }
```

The UI starts a game with `App::core_options` (`LaunchSpec::options`), hands `CoreOptions` to the running session (`Session::set_options`), and gives the session `core_options` again once it has started, for a change made while it loaded.

### Core options

`specular_core::settings::core_options`, apart from the generic settings:

```rust
pub struct CoreOptions { pub categories: Vec<Category>, pub options: Vec<CoreOption> }  // data/cores/<core>.json
pub struct Category { pub key: String, pub label: String, pub info: Option<String> }
pub struct CoreOption {
    pub key: String,                 // "mgba_idle_optimization"
    pub label: String,               // the core's description, "Idle Loop Removal"
    pub info: Option<String>,
    pub category: Option<String>,    // a Category's key
    pub values: Vec<OptionValue>,
    pub default: String,             // the core's
}
pub struct OptionValue { pub value: String, pub label: Option<String> }
impl CoreOptions { pub fn differences(&self, declared: &CoreOptions) -> Vec<String>; }

pub fn list(core: &'static CoreDecl) -> &'static CoreOptions;      // parsed from CoreDecl::options
pub fn decls(core: &'static CoreDecl) -> &'static [SettingDecl];
pub fn values(settings: &Settings, core: &'static CoreDecl, at: &Layer) -> Vec<(String, String)>;  // for LaunchSpec::options
pub mod mgba { pub const LIST: &str; pub const IDLE_OPTIMIZATION: &str; pub mod idle_optimization { pub const REMOVE_KNOWN: &str; /* … */ } /* … */ }

pub struct CoreDecl {                                   // library: how Specular runs a core
    pub options: &'static str,                          // its list, e.g. mgba::LIST
    pub defaults: &'static [(&'static str, &'static str)],  // Specular's, e.g. (mgba::IDLE_OPTIMIZATION, mgba::idle_optimization::REMOVE_KNOWN)
    /* name, library, rom_by_path, states_restore_save, bios */
}
impl CoreDecl { pub fn id(&self) -> &'static str; }    // "mgba": its library's name less "_libretro"
```

Each option is a developer setting under a section named after the core and its category ("mGBA: Performance"), settable per system and per game, its values as a choice labelled as the core labels them. Its `help`, for the schema, is the core's info text, then the values' labels if they have any, then the core's own default where Specular's differs. A setting declaration takes `'static` text, so each core's list is parsed once and kept for the run, its declarations borrowing from it. `crates/specular-core/build.rs` writes a module of constants per `data/cores/*.json`: each key, less the core's prefix, and each value, in upper case (`DONT_REMOVE`, `V15` for "15", `MINUS_1` for "-1"); one that would clash with another is left out, with a build warning.

The runtime tells cores it takes libretro's options version 2 and reads versions 0, 1 and 2 alike (`specular-runtime`'s `options`). `specular_runtime::declared_options(core)` loads a core without a game and returns what it declares; the `core-options` example writes that to `data/cores/<core>.json` (see DEVELOPMENT.md). Options a core declares only once it has a game would be missed; no built-in core does.

Saves are made on the I/O thread, and saves queued together are merged, so holding a button that steps a value writes the file once. A change made before its layer has loaded (while a migration runs, say) is saved again once it has, merged with the file's values, which the first save left out.

### Files

```
config/v0/
  specular.json                    the interface and global settings
  specular.schema.json
  systems/<system id>.json         per system
  systems/<system id>.schema.json  for each system with a core
  cores/<core>.json                each core's options, for every game it plays
  cores/<core>.schema.json
  games/<path under roms/, without extension>.json   per game, mirroring roms/ (as saves/ does)
  games/<system id>.schema.json    shared by the games of each system with a core
```

A layer's file is `Layer::path`; its schema's is `schema::file`. Each file loaded is logged with its values (`settings: loaded …`), and each value in it that is ignored with its key and why (`settings::ignored`: not one of the setting's values, or a setting that can't be set in that file); so is a file under `games/` for no game in the library (a ROM renamed, say), which is left alone. A ROM at the top of `roms/` named `<system id>.schema.<ext>` would have its settings file where its system's games' schema is; this is not guarded against.

A file Specular writes starts with `"$schema"`, the path to its schema from the file (`"../gba.schema.json"` in `games/gba/Golden Sun.json`), then the overrides, as in `specular.json`:

```json
{
  "$schema": "specular.schema.json",
  "display.brightness": 3
}
```

The schemas (JSON Schema draft 2020-12) are made by `settings::schema::all` from the declarations: per setting valid in that file, its section and label as `title`, its `help` as `description`, its type and allowed values, and its `default`. The I/O thread writes them at start-up, after loading the settings, and not into an empty data folder (without its card, that is the bare mount point). A schema that can't be written is logged and doesn't stop settings from loading or saving.

Reading a file: a missing file is an empty layer. If the file is missing or not valid JSON, a valid `<file>.tmp` left by a cut-off write is used instead. An empty file counts as an empty object (someone may have emptied it by hand). A file that isn't a JSON object is moved aside to `<file>.bad`, so the user can look at it, and the layer starts empty. Writing an empty layer removes the file (and any stale `.tmp`). The same safe store is used for Specular's own records in `cache/` and `saves/`.

### Migrations

```rust
/// One change of format: fills a new, empty folder (`to`) from the previous format's (`from`).
struct Step { from: &'static str, to: &'static str, run: fn(from: &Path, to: &Path) -> io::Result<()> }

/// Oldest first; the last step's folder is the current format.
const STEPS: &[Step] = &[
    Step { from: "configs", to: "config/v0", run: copy },
];
```

At start-up, before settings are read (`settings::migrate::migrate`): if the current format's folder (`config/v0/`) exists, older folders are ignored. Otherwise the newest older folder is carried forward one step at a time. Each step fills `<to>.tmp/` (every file and folder fsynced), renames it to `<to>/`, and fsyncs the folders above it, up to the data root. A file that can't be read is logged and skipped, and links aren't followed. Each step, each file copied and each failure is logged under `settings:`.

A failure to write ends the step. Specular then writes nothing to `config/v0/` until the next start, which tries again: no schemas, and no settings saved (changes stay in memory), since a `config/v0/` made meanwhile would pass for migrated and `configs/` would be ignored for good. A notice says that settings can't be saved this time, and that the log says why.

Before v0, settings were in `configs/`, laid out as v0 less the schemas and `cores/`. Its step copies everything in it as it is, `.tmp` and `.bad` files too: reading then deals with them as it would have in `configs/`.

### SD card layout

| Folder | Contents |
|---|---|
| `roms/` | Games, in any layout (see [library-and-box-art.md](library-and-box-art.md)). The user's box art beside each ROM (`Golden Sun.png`). Save states in a hidden `.states/` folder beside the games (`roms/gba/.states/Golden Sun.state1`, `.autosave`). |
| `bios/` | BIOS files, handed to cores as their system folder |
| `fonts/` | Fonts for characters Specular's own lack (Japanese or Chinese game names, say): TrueType or OpenType files, directly in it, read at start-up (see [look-and-motion.md](look-and-motion.md)) |
| `saves/` | Battery saves (`.srm`), mirroring `roms/` and named after the ROM: `roms/gba/Golden Sun.gba` → `saves/gba/Golden Sun.srm`. Also `saves/.specular-saves.json`, which lets saves follow renamed ROMs (see [runtime.md](runtime.md)). |
| `config/v0/` | Settings, as above |
| `cache/` | Things Specular can rebuild, safe to delete: `cache/art/` (box art, mirroring `roms/`), `cache/identify/` (what is known about each ROM) |

## Open questions

- Cores the user adds (not built by Specular) have no list in `data/cores/`. Their options could be learnt at their first launch (the runtime has what a core declares) and kept in `cache/`, then declared as for the built-in cores (`CoreDecl::options` would come from that file; the declarations' `'static` text is kept for the run either way). Settled when user cores are.
