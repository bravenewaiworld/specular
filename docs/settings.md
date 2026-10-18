# Settings

Status: In progress · Updated: 2026-10-18

How settings are declared, shown, layered and stored on the SD card, how their files change format over time, and how the card is laid out. The UI shows only the few settings people genuinely disagree on; everything else is still reachable, documented, in plain files on the card.

## Decisions

- **Settings are declared once** (2026-10-12): a key, type, default, label, section and who sees it. The settings screens are generated from the declarations, so a setting needs no hand-written UI.
- **A setting is shown only where people genuinely disagree** (2026-10-12). Everything else has a good default or is detected automatically. Internals (frame time, GPU, memory, file paths) appear only when "Developer details" is on; battery, storage and WiFi are shown in plain words.
- **Layers: built-in default < global < per system < per game** (2026-10-12). Each setting says how specific it may be (brightness is global only; scaling may differ per game).
- **Only values that differ from the default are stored** (2026-10-12), so changing a default in a new version reaches everyone who never changed it. Unknown keys (a plugin not loaded, a newer version's settings) are kept untouched. A stored value of the wrong type or out of range is ignored, as if absent.
- **Writes survive a power cut on FAT32** (2026-10-12). FAT32 has no journal, so a file is never rewritten in place: write a temporary file, fsync it, rename it over the original, then fsync the folder.
- **Settings live in a versioned folder: `{sd card}/config/v0/<name>.json`** (2026-10-18). Every configuration (Specular's own interface, each system, each core) is a file there, so everything not in the UI can still be changed. The `.json` holds only the user's overrides. Beside each, Specular writes `<name>.schema.json`: a fully documented JSON Schema that also carries every default and automatically worked-out value, so users can see what they can set and to what. It is marked read-only as a hint: edits to it are ignored and it may be overwritten at any time. Defaults stay out of the `.json` so that a changed default still reaches everyone who never overrode it (2026-10-18). Rejected: filling every default into the `.json` (a later default change couldn't be told apart from a user's choice). Each future format change gets a new `vN/` folder, migrated from the previous one.
- **One file per scope** (2026-10-18): the interface and global settings, each system, each core's options, and each game's overrides (laid out under Design). Rejected: one file per system holding its core options and per-game overrides (fewer files, harder to edit by hand).
- **The old folder is kept after migrating** (2026-10-18): `configs/`, and each older `vN/`, stay untouched, so a downgrade still finds its settings and a migration bug loses nothing. Rejected: deleting it after migrating.
- **Core options are settings** (2026-10-18), declared from the core's own option list, stored in `cores/<core>.json`, and overridable per system and per game. They are in config files only, unless people genuinely disagree about one; then it gets a row in the UI.

## Design

### Declarations

```rust
pub struct SettingDecl {
    pub key: &'static str,          // e.g. "video.scaling"; never renamed once released
    pub section: &'static str,      // the header it is listed under, e.g. "Display"
    pub label: &'static str,
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

Saves are made on the I/O thread, and saves queued together are merged, so holding a button that steps a value writes the file once.

### Files today

What the `config/v0/` scheme replaces, and migrates from:

```
configs/
  specular.json                 global layer
  systems/<system id>.json      per system
  games/<path under roms/, without extension>.json   per game, mirroring roms/
```

Only the global layer is loaded and editable so far.

Reading a file: a missing file is an empty layer. If the file is missing or not valid JSON, a valid `<file>.tmp` left by a cut-off write is used instead. An empty file counts as an empty object (someone may have emptied it by hand). A file that isn't a JSON object is moved aside to `<file>.bad`, so the user can look at it, and the layer starts empty. Writing an empty layer removes the file (and any stale `.tmp`). The same safe store is used for Specular's own records in `cache/` and `saves/`.

### Files in v0

```
config/v0/
  specular.json                    the interface and global settings
  systems/<system id>.json         per system
  cores/<core>.json                core options
  games/<path under roms/, without extension>.json   per game, mirroring roms/ (as saves/ does)
```

Beside each `<name>.json` is its `<name>.schema.json`: the generated JSON Schema documenting it; a read-only hint, edits ignored.

### SD card layout

| Folder | Contents |
|---|---|
| `roms/` | Games, in any layout (see [library-and-box-art.md](library-and-box-art.md)). The user's box art beside each ROM (`Golden Sun.png`). Save states in a hidden `.states/` folder beside the games (`roms/gba/.states/Golden Sun.state1`, `.autosave`). |
| `bios/` | BIOS files, handed to cores as their system folder |
| `fonts/` | Fonts for characters Specular's own lack (Japanese or Chinese game names, say): TrueType or OpenType files, directly in it, read at start-up (see [look-and-motion.md](look-and-motion.md)) |
| `saves/` | Battery saves (`.srm`), mirroring `roms/` and named after the ROM: `roms/gba/Golden Sun.gba` → `saves/gba/Golden Sun.srm`. Also `saves/.specular-saves.json`, which lets saves follow renamed ROMs (see [runtime.md](runtime.md)). |
| `config/v0/` | Settings, as above |
| `cache/` | Things Specular can rebuild, safe to delete: `cache/art/` (box art, mirroring `roms/`), `cache/identify/` (what is known about each ROM) |
