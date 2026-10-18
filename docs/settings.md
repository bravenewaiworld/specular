# Settings

Status: In progress · Updated: 2026-10-18

How settings are declared, shown, layered and stored on the SD card, how their files change format over time, and how the card is laid out. The UI shows only the few settings people genuinely disagree on; everything else is still reachable, documented, in plain files on the card.

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
- **Core options are settings** (2026-10-18), declared from the core's own option list, stored in `cores/<core>.json`, and overridable per system and per game. They are in config files only, unless people genuinely disagree about one; then it gets a row in the UI.

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
}
pub enum SettingKind {
    Bool,
    Choice(&'static [(&'static str, &'static str)]),  // (stored value, label)
    Colour { same_as: Option<&'static str> },          // a palette id, or "same" as another colour setting
    Int { min: i64, max: i64, step: i64, unit: &'static str },
}
pub enum Visibility { User, Developer, OwnScreen }    // OwnScreen: shown by a screen of its own (Wi-Fi)
pub enum Scope { Global, System, Game }
pub enum Layer { Global, System(String), Game { system: String, game: String } }
```

A value is looked up from the most specific layer that has a valid one, down to the default. Setting a value equal to what the layer would inherit removes the override. Settings rows mark values that override a less specific layer. Specular's own declarations are in `specular_core::settings::builtin`.

Saves are made on the I/O thread, and saves queued together are merged, so holding a button that steps a value writes the file once. A change made before its layer has loaded (while a migration runs, say) is saved again once it has, merged with the file's values, which the first save left out.

### Files

```
config/v0/
  specular.json                    the interface and global settings
  specular.schema.json
  systems/<system id>.json         per system
  systems/<system id>.schema.json  for each system with a core
  cores/<core>.json                core options (not declared yet, nor their schema)
  games/<path under roms/, without extension>.json   per game, mirroring roms/ (as saves/ does)
  games/<system id>.schema.json    shared by the games of each system with a core
```

Only the global layer is loaded and editable so far. A layer's file is `Layer::path`; its schema's is `schema::file`. A ROM at the top of `roms/` named `<system id>.schema.<ext>` would have its settings file where its system's games' schema is; this is not guarded against.

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
