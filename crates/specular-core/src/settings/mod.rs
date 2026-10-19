//! Settings: declared once, stored as layered JSON, shown as generated rows.
//!
//! Each setting is a `SettingDecl`. Its value is looked up from the most specific layer that has
//! one: a game's file, then its system's, then the global file (for a core's option, the core's
//! file: see `core_options`), then the declared default. Only overrides are stored, so changing
//! a default in a new version reaches everyone who never changed it. Keys in a file that no declaration knows (settings of a plugin not loaded, or of a
//! newer version) are kept untouched. A stored value of the wrong type or out of range is
//! ignored, as if absent.

pub mod builtin;
pub mod core_options;
pub mod migrate;
pub mod schema;
pub mod store;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::device::Rgb;
use crate::input::{Button, Press};
use crate::library::{CoreDecl, SystemDecl};
use crate::model::{Row, RowKind};
use crate::palette::{self, PALETTE};

/// Who a setting is for. Settings screens show user settings, and developer ones too while the
/// "Developer details" setting is on. `OwnScreen` settings are never listed: a screen of their
/// own shows them (as the Wi-Fi screen shows the Wi-Fi setting).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Visibility { User, Developer, OwnScreen }

/// How specific a layer is; also how deep a setting may be overridden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope { Global, System, Game }

/// Where a value is stored.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    /// Specular's own settings, for every system and game.
    Global,
    /// A core's options, for every system and game it plays, by its id (`CoreDecl::id`). It
    /// stands where `Global` does for Specular's own settings (its scope is `Scope::Global`).
    Core(String),
    System(String),
    /// A game, by system and by its ROM's path under `roms/` without the extension (e.g.
    /// "gba/RPG/Golden Sun (USA, Europe)"), so games of the same name in different folders
    /// keep separate settings.
    Game { system: String, game: String },
}

impl Layer {
    pub fn scope(&self) -> Scope {
        match self { Layer::Global | Layer::Core(_) => Scope::Global, Layer::System(_) => Scope::System, Layer::Game { .. } => Scope::Game }
    }

    /// The system it is for (a system's or a game's layer).
    pub fn system(&self) -> Option<&str> {
        match self { Layer::System(s) | Layer::Game { system: s, .. } => Some(s), Layer::Global | Layer::Core(_) => None }
    }

    /// The layers `d`'s value is looked up in at this one, most specific first: this one and the
    /// system's (for a game), then `d`'s least specific one, `Global` or its core's.
    fn chain(&self, d: &SettingDecl) -> Vec<Layer> {
        let specific = match self {
            Layer::Global | Layer::Core(_) => vec![],
            Layer::System(_) => vec![self.clone()],
            Layer::Game { system, .. } => vec![self.clone(), Layer::System(system.clone())],
        };
        specific.into_iter().chain([d.base()]).collect()
    }

    /// The layer's file in the config folder (`migrate::config_dir`). Games' files mirror the
    /// `roms/` tree.
    pub fn path(&self, config: &Path) -> PathBuf { config.join(self.file()) }

    /// Relative to the config folder.
    fn file(&self) -> PathBuf {
        match self {
            Layer::Global => "specular.json".into(),
            Layer::Core(c) => Path::new("cores").join(format!("{c}.json")),
            Layer::System(s) => Path::new("systems").join(format!("{s}.json")),
            Layer::Game { game, .. } => Path::new("games").join(format!("{game}.json")),
        }
    }

    /// Its file's `$schema`: the path of its schema (`schema::file`) relative to its file. The
    /// schema is in the file's folder or a folder above it (a game's is in `games/`, however deep
    /// the game's file), so the path is a run of `../` and its name.
    fn schema_ref(&self) -> String {
        let schema = schema::file(self);
        let up = self.file().components().count() - schema.components().count();
        format!("{}{}", "../".repeat(up), schema.file_name().unwrap().to_string_lossy())
    }
}

/// The key in a settings file that tells editors where its schema is. Not a setting: it is
/// dropped on reading and written afresh on saving.
pub const SCHEMA_KEY: &str = "$schema";

/// What values a setting takes.
#[derive(Clone, Copy, Debug)]
pub enum SettingKind {
    Bool,
    /// One of these (stored value, label) pairs.
    Choice(&'static [(&'static str, &'static str)]),
    /// A colour from the palette, stored by its id. With `same_as` (another Colour setting's
    /// key), the first option is `SAME`: show that setting's colour.
    Colour { same_as: Option<&'static str> },
    /// A whole number from `min` to `max` in steps of `step`, shown with `unit`.
    Int { min: i64, max: i64, step: i64, unit: &'static str },
}

/// The value of a Colour setting that follows another (see `SettingKind::Colour`).
pub const SAME: &str = "same";

impl SettingKind {
    /// The stored values a Choice or Colour steps through, in order (empty for other kinds).
    fn options(self) -> Vec<&'static str> {
        match self {
            SettingKind::Choice(options) => options.iter().map(|(o, _)| *o).collect(),
            SettingKind::Colour { same_as } => same_as.map(|_| SAME).into_iter().chain(PALETTE.iter().map(|c| c.id)).collect(),
            SettingKind::Bool | SettingKind::Int { .. } => Vec::new(),
        }
    }
}

/// One setting. Declared in code (or later by a plugin); the settings screens are built from these.
#[derive(Clone, Debug)]
pub struct SettingDecl {
    /// Its key in the JSON files, e.g. "video.scaling". Never renamed once released.
    pub key: &'static str,
    /// The section it is listed under, e.g. "Display".
    pub section: &'static str,
    pub label: &'static str,
    /// What it does and what its values mean, in a sentence or two, for people editing the files
    /// by hand (it documents the setting in its schema).
    pub help: &'static str,
    pub kind: SettingKind,
    pub default: Value,
    pub visibility: Visibility,
    /// The most specific layer allowed to override it (e.g. brightness is global only).
    pub deepest: Scope,
    /// The core whose option it is (`CoreDecl::id`), stored in the core's file rather than the
    /// global one; None for Specular's own settings.
    pub core: Option<&'static str>,
    /// The system it belongs to (`SystemDecl::id`): it may be set only in that system's and its
    /// games' files. None: any system's.
    pub system: Option<&'static str>,
}

impl SettingDecl {
    /// Its least specific layer: the global one, or its core's.
    fn base(&self) -> Layer { self.core.map_or(Layer::Global, |c| Layer::Core(c.into())) }

    /// Whether it may be stored in `at`'s file: one no more specific than `deepest`, and at the
    /// global scope only its `base` (its core's file for a core's option, else specular.json); for
    /// one system's setting, only that system's and its games' files.
    fn settable_at(&self, at: &Layer) -> bool {
        self.system.is_none_or(|s| at.system() == Some(s)) && at.scope() <= self.deepest && (at.scope() > Scope::Global || *at == self.base())
    }

    /// Whether `v` is a valid value for this setting.
    fn accepts(&self, v: &Value) -> bool {
        match self.kind {
            SettingKind::Bool => v.is_boolean(),
            SettingKind::Choice(_) | SettingKind::Colour { .. } => v.as_str().is_some_and(|s| self.kind.options().contains(&s)),
            SettingKind::Int { min, max, step, .. } => v.as_i64().is_some_and(|n| (min..=max).contains(&n) && (n - min) % step == 0),
        }
    }
}

/// Why `Settings::set` refused a value.
#[derive(Debug, PartialEq, Eq)]
pub enum SettingError {
    /// No such setting is declared.
    Unknown,
    /// Not a valid value for the setting.
    WrongValue,
    /// The setting can't be overridden that specifically (e.g. per game).
    NotAtThisLayer,
}

/// The declared settings and the layers loaded so far.
#[derive(Debug, Default)]
pub struct Settings {
    decls: Vec<SettingDecl>,
    layers: HashMap<Layer, Map<String, Value>>,
}

impl Settings {
    pub fn new(decls: Vec<SettingDecl>) -> Self { Self { decls, layers: HashMap::new() } }

    /// A setting's declaration, by its key.
    pub fn decl(&self, key: &str) -> Option<&SettingDecl> { self.decls.iter().find(|d| d.key == key) }

    /// The declared settings.
    pub fn decls(&self) -> &[SettingDecl] { &self.decls }

    /// Declares these settings instead (as the cores change: see `user_cores`). Values already
    /// loaded are kept; those of settings no longer declared are kept as unknown keys.
    pub fn declare(&mut self, decls: Vec<SettingDecl>) { self.decls = decls; }

    /// Adds a layer's contents as loaded from its file (`store::load_layer`), less `SCHEMA_KEY`.
    /// Values already set in memory win: they were changed after the file was written. Values that
    /// can't be used are kept (see `ignored`).
    pub fn set_layer(&mut self, layer: Layer, values: Map<String, Value>) {
        let map = self.layers.entry(layer).or_default();
        for (k, v) in values.into_iter().filter(|(k, _)| k != SCHEMA_KEY) { map.entry(k).or_insert(v); }
    }

    /// A layer's contents as its file holds them, for `store::store_layer`: its `$schema` first
    /// (`SCHEMA_KEY`; the map keeps insertion order), then its values. Empty if it has no values,
    /// so the file is removed.
    pub fn layer(&self, layer: &Layer) -> Map<String, Value> {
        let Some(values) = self.layers.get(layer).filter(|m| !m.is_empty()) else { return Map::new() };
        std::iter::once((SCHEMA_KEY.to_string(), Value::from(layer.schema_ref()))).chain(values.clone()).collect()
    }

    /// A setting's value as seen at `at`, and the layer it comes from (None = the default).
    /// None if no such setting is declared.
    pub fn get(&self, key: &str, at: &Layer) -> Option<(&Value, Option<Layer>)> {
        let d = self.decl(key)?;
        for layer in at.chain(d).into_iter().filter(|l| d.settable_at(l)) {
            if let Some(v) = self.layers.get(&layer).and_then(|m| m.get(key)).filter(|v| d.accepts(v)) {
                return Some((v, Some(layer)));
            }
        }
        Some((&d.default, None))
    }

    /// Sets a value at `at`. Setting it to what `at` would inherit anyway removes the override.
    pub fn set(&mut self, key: &str, at: &Layer, v: Value) -> Result<(), SettingError> {
        let d = self.decl(key).ok_or(SettingError::Unknown)?;
        if !d.accepts(&v) { return Err(SettingError::WrongValue); }
        if !d.settable_at(at) { return Err(SettingError::NotAtThisLayer); }
        self.layers.entry(at.clone()).or_default().remove(key);
        if self.get(key, at).map(|(inherited, _)| inherited) != Some(&v) {
            self.layers.get_mut(at).unwrap().insert(key.to_string(), v);
        }
        Ok(())
    }

    /// Removes the value set at `at` itself, so it inherits again. Returns the setting's key if
    /// there was one.
    pub fn reset(&mut self, key: &str, at: &Layer) -> Option<&'static str> {
        let key = self.decl(key)?.key;
        self.layers.get_mut(at)?.remove(key).map(|_| key)
    }

    /// The settings screen for `at`: one row per setting of `core`'s (None: Specular's own) that
    /// may be set there (`settable_at`) and is visible at `show`, under a header per section, in declaration
    /// order. Rows that override a less specific layer are marked.
    pub fn rows(&self, at: &Layer, show: Visibility, core: Option<&str>) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut section = "";
        for d in self.decls.iter().filter(|d| d.visibility <= show && d.core == core && d.settable_at(at)) {
            if d.section != section {
                section = d.section;
                rows.push(Row::new(format!("section:{section}"), section, RowKind::Header));
            }
            rows.push(self.row_of(d, at));
        }
        rows
    }

    /// One setting's row at `at` (for screens that show a setting of their own); None if no such
    /// setting is declared.
    pub fn row(&self, key: &str, at: &Layer) -> Option<Row> { self.decl(key).map(|d| self.row_of(d, at)) }

    fn row_of(&self, d: &SettingDecl, at: &Layer) -> Row {
        let (v, from) = self.get(d.key, at).unwrap();
        let kind = match d.kind {
            SettingKind::Bool => RowKind::Toggle { on: v.as_bool().unwrap_or(false) },
            SettingKind::Choice(options) => choice_row(options.iter().map(|(o, label)| (*o, label.to_string())), v, None),
            SettingKind::Colour { same_as } => {
                let same = same_as.and_then(|k| self.decl(k)).map(|f| (SAME, format!("Same as {}", f.label.to_lowercase())));
                let colours = PALETTE.iter().map(|c| (c.id, c.label.to_string()));
                choice_row(same.into_iter().chain(colours), v, self.colour(d.key, at))
            }
            SettingKind::Int { min, max, step, unit } => RowKind::Number { value: v.as_i64().unwrap_or(min), min, max, step, unit },
        };
        let mut row = Row::new(d.key, d.label, kind);
        row.overridden = at.scope() > Scope::Global && from.as_ref() == Some(at);
        row
    }

    /// The colour a Colour setting shows at `at`, following `SAME`; None for other settings.
    pub fn colour(&self, key: &str, at: &Layer) -> Option<Rgb> {
        let SettingKind::Colour { same_as } = self.decl(key)?.kind else { return None };
        match self.get(key, at)?.0.as_str()? {
            SAME => self.colour(same_as?, at),
            id => palette::find(id),
        }
    }

    /// Applies a press to the setting `key` shown at `at` (A, left or right on its row). Returns
    /// the setting's key and new value if it changed. Choices wrap around on a real press, not
    /// while held; toggles change only on a real press.
    pub fn press(&mut self, key: &str, at: &Layer, p: Press) -> Option<(&'static str, Value)> {
        let d = self.decl(key)?;
        let (kind, key) = (d.kind, d.key);
        let current = self.get(key, at)?.0.clone();
        let delta = match p.button { Button::Left => -1, Button::Right | Button::A => 1, _ => return None };
        let new = match kind {
            SettingKind::Bool if !p.repeat => Value::Bool(!current.as_bool().unwrap_or(false)),
            SettingKind::Bool => return None,
            SettingKind::Choice(_) | SettingKind::Colour { .. } => {
                let options = kind.options();
                let n = options.len() as i64;
                let i = options.iter().position(|o| Some(*o) == current.as_str()).unwrap_or(0) as i64 + delta;
                if !(0..n).contains(&i) && p.repeat { return None; }
                Value::from(options[i.rem_euclid(n) as usize])
            }
            SettingKind::Int { .. } if p.button == Button::A => return None,
            SettingKind::Int { min, max, step, .. } => Value::from((current.as_i64().unwrap_or(min) + delta * step).clamp(min, max)),
        };
        if new == current { return None; }
        self.set(key, at, new.clone()).ok().map(|_| (key, new))
    }
}

/// What a settings file read for `layer` holds that is ignored, one line per key: values that
/// aren't valid for their setting, and settings that can't be set in that file (brightness in a
/// game's, say). Keys no declaration knows are kept and not reported: they may be a plugin's, or
/// a newer version's.
pub fn ignored(decls: &[SettingDecl], layer: &Layer, values: &Map<String, Value>) -> Vec<String> {
    values.iter().filter_map(|(key, v)| {
        let d = decls.iter().find(|d| d.key == key)?;
        if !d.settable_at(layer) { Some(format!("{key} can't be set in this file, so it is ignored")) }
        else if !d.accepts(v) { Some(format!("{key} = {v} isn't one of its values, so it is ignored")) }
        else { None }
    }).collect()
}

/// The games that have a settings file in the config folder `config`, as `Layer::Game`'s `game`
/// (their paths under `roms/` without extension). Only `games/` is walked, which holds files just
/// for games with settings, so this costs little however many games there are. A `.tmp` file
/// counts (`store::load_layer` reads it if its file is missing); the schemas, `.bad` files and
/// links don't.
pub fn games_with_settings(config: &Path) -> BTreeSet<String> {
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let (path, Ok(kind)) = (e.path(), e.file_type()) else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if kind.is_dir() && depth < crate::library::MAX_DEPTH { walk(root, &path, depth + 1, out); continue; }
            let Some(stem) = name.strip_suffix(".json").or_else(|| name.strip_suffix(".json.tmp")) else { continue };
            // Schemas are only at the top (`schema::file`).
            if !kind.is_file() || (depth == 0 && stem.ends_with(".schema")) { continue; }
            let game = path.parent().unwrap().join(stem);
            out.insert(game.strip_prefix(root).unwrap().to_string_lossy().into_owned());
        }
    }
    let mut games = BTreeSet::new();
    walk(&config.join("games"), &config.join("games"), 0, &mut games);
    games
}

/// Every setting: Specular's own (`builtin`), the "Core" of each of these systems with several
/// (`core_of`), then the options of their cores (each core's once). An option of the same key
/// as one already declared (cores derived from one another can share their options' keys) isn't
/// declared again: it is one setting, the first core's.
pub fn declarations(systems: &[SystemDecl]) -> Vec<SettingDecl> {
    let mut decls: Vec<SettingDecl> = builtin::settings().into_iter().chain(systems.iter().filter_map(core_choice)).collect();
    for d in crate::library::cores(systems).into_iter().flat_map(|c| core_options::decls(c)) {
        if !decls.iter().any(|k| k.key == d.key) { decls.push(d.clone()); }
    }
    decls
}

/// The key of the setting choosing which of `system`'s cores plays its games (see `core_of`).
pub fn core_key(system: &str) -> String { format!("core.{system}") }

/// The "Core" setting of a system with several cores: which plays its games, by `CoreDecl::id`;
/// by default the first (Specular's own, if it has one). Its text is leaked: declarations take
/// `'static` text, and the cores' own are kept for the run too (see `core_options`).
fn core_choice(system: &SystemDecl) -> Option<SettingDecl> {
    if system.cores.len() < 2 { return None; }
    let choices: Vec<(&'static str, &'static str)> = system.cores.iter().map(|c| (c.id(), c.name)).collect();
    let each: Vec<String> = system.cores.iter().map(|c| format!("\"{}\" ({}{})", c.id(), c.name, if c.user.is_some() { ", added to the SD card's cores folder" } else { "" })).collect();
    Some(SettingDecl {
        key: leak(core_key(system.id)), section: "Emulator", label: "Core",
        help: leak(format!("Which emulator core plays {} games: {}. A change takes effect when a game next starts.", system.name, each.join(", "))),
        kind: SettingKind::Choice(Box::leak(choices.into_boxed_slice())), default: Value::from(system.cores[0].id()),
        visibility: Visibility::User, deepest: Scope::Game, core: None, system: Some(system.id),
    })
}

/// The core that plays `system`'s games at `at` (its layer, or one of its games'): the one its
/// "Core" setting chooses (`core_key`), else its first. None if it has none.
pub fn core_of(settings: &Settings, system: &SystemDecl, at: &Layer) -> Option<&'static CoreDecl> {
    let chosen = settings.get(&core_key(system.id), at).and_then(|(v, _)| v.as_str());
    chosen.and_then(|id| system.cores.iter().find(|c| c.id() == id)).or(system.cores.first()).copied()
}

/// Text kept for the rest of the run, as declarations take `'static` text.
pub(crate) fn leak(s: impl Into<String>) -> &'static str { Box::leak(s.into().into_boxed_str()) }

/// A choice row from (stored value, label) options, with `v` selected (the first if it matches
/// none) and `swatch` shown beside the value.
fn choice_row(options: impl Iterator<Item = (&'static str, String)>, v: &Value, swatch: Option<Rgb>) -> RowKind {
    let (values, labels): (Vec<_>, Vec<_>) = options.unzip();
    let selected = values.iter().position(|o| Some(*o) == v.as_str()).unwrap_or(0);
    RowKind::Choice { options: labels, selected, swatch }
}

#[cfg(test)]
mod tests;
