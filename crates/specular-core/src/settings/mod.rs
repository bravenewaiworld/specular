//! Settings: declared once, stored as layered JSON, shown as generated rows.
//!
//! Each setting is a `SettingDecl`. Its value is looked up from the most specific layer that has
//! one: a game's file, then its system's, then the global file, then the declared default. Only
//! overrides are stored, so changing a default in a new version reaches everyone who never
//! changed it. Keys in a file that no declaration knows (settings of a plugin not loaded, or of a
//! newer version) are kept untouched. A stored value of the wrong type or out of range is
//! ignored, as if absent.

pub mod builtin;
pub mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::device::Rgb;
use crate::input::{Button, Press};
use crate::model::{Row, RowKind};
use crate::palette::{self, PALETTE};

/// Who a setting is for. Settings screens show user settings, and developer ones too while the
/// "Developer details" setting is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Visibility { User, Developer }

/// How specific a layer is; also how deep a setting may be overridden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope { Global, System, Game }

/// Where a value is stored.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    Global,
    System(String),
    /// A game, by system and by its ROM's path under `roms/` without the extension (e.g.
    /// "gba/RPG/Golden Sun (USA, Europe)"), so games of the same name in different folders
    /// keep separate settings.
    Game { system: String, game: String },
}

impl Layer {
    pub fn scope(&self) -> Scope { match self { Layer::Global => Scope::Global, Layer::System(_) => Scope::System, Layer::Game { .. } => Scope::Game } }

    /// The layers a value is looked up in, most specific first.
    fn chain(&self) -> Vec<Layer> {
        match self {
            Layer::Global => vec![Layer::Global],
            Layer::System(_) => vec![self.clone(), Layer::Global],
            Layer::Game { system, .. } => vec![self.clone(), Layer::System(system.clone()), Layer::Global],
        }
    }

    /// The layer's file under the `configs/` folder. Games' files mirror the `roms/` tree.
    pub fn path(&self, configs: &Path) -> PathBuf {
        match self {
            Layer::Global => configs.join("specular.json"),
            Layer::System(s) => configs.join("systems").join(format!("{s}.json")),
            Layer::Game { game, .. } => configs.join("games").join(format!("{game}.json")),
        }
    }
}

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
    pub kind: SettingKind,
    pub default: Value,
    pub visibility: Visibility,
    /// The most specific layer allowed to override it (e.g. brightness is global only).
    pub deepest: Scope,
}

impl SettingDecl {
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

    fn decl(&self, key: &str) -> Option<&SettingDecl> { self.decls.iter().find(|d| d.key == key) }

    /// The declared settings.
    pub fn decls(&self) -> &[SettingDecl] { &self.decls }

    /// Adds a layer's contents as loaded from its file (`store::load_layer`). Values already set
    /// in memory win: they were changed after the file was written.
    pub fn set_layer(&mut self, layer: Layer, values: Map<String, Value>) {
        let map = self.layers.entry(layer).or_default();
        for (k, v) in values { map.entry(k).or_insert(v); }
    }

    /// A layer's contents, for `store::store_layer`.
    pub fn layer(&self, layer: &Layer) -> Map<String, Value> { self.layers.get(layer).cloned().unwrap_or_default() }

    /// A setting's value as seen at `at`, and the layer it comes from (None = the default).
    /// None if no such setting is declared.
    pub fn get(&self, key: &str, at: &Layer) -> Option<(&Value, Option<Layer>)> {
        let d = self.decl(key)?;
        for layer in at.chain().into_iter().filter(|l| l.scope() <= d.deepest) {
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
        if at.scope() > d.deepest { return Err(SettingError::NotAtThisLayer); }
        self.layers.entry(at.clone()).or_default().remove(key);
        if self.get(key, at).map(|(inherited, _)| inherited) != Some(&v) {
            self.layers.get_mut(at).unwrap().insert(key.to_string(), v);
        }
        Ok(())
    }

    /// The settings screen for `at`: one row per setting that can be set there and is visible at
    /// `show`, under a header per section, in declaration order. Rows that override a less
    /// specific layer are marked.
    pub fn rows(&self, at: &Layer, show: Visibility) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut section = "";
        for d in self.decls.iter().filter(|d| d.visibility <= show && at.scope() <= d.deepest) {
            if d.section != section {
                section = d.section;
                rows.push(Row::new(format!("section:{section}"), section, RowKind::Header));
            }
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
            rows.push(row);
        }
        rows
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

/// A choice row from (stored value, label) options, with `v` selected (the first if it matches
/// none) and `swatch` shown beside the value.
fn choice_row(options: impl Iterator<Item = (&'static str, String)>, v: &Value, swatch: Option<Rgb>) -> RowKind {
    let (values, labels): (Vec<_>, Vec<_>) = options.unzip();
    let selected = values.iter().position(|o| Some(*o) == v.as_str()).unwrap_or(0);
    RowKind::Choice { options: labels, selected, swatch }
}

#[cfg(test)]
mod tests;
