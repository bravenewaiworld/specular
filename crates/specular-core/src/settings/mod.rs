//! Settings: declared once, stored as layered JSON, shown as generated rows.
//!
//! Each setting is a `SettingDecl`. Its value is looked up from the most specific layer that has
//! one: a game's file, then its system's, then the global file, then the declared default. Only
//! overrides are stored, so changing a default in a new version reaches everyone who never
//! changed it. Keys a layer doesn't declare (settings of a plugin not loaded, or of a newer
//! version) are kept untouched. A stored value of the wrong type or out of range is ignored, as
//! if absent.

pub mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::input::{Button, Press};
use crate::model::{Row, RowKind};

/// Who a setting is for. Screens show settings up to the visibility the user picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Visibility { User, Advanced, Developer }

/// Where a value is stored, from least to most specific.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    Global,
    System(String),
    /// A game, by system and ROM file name without its extension.
    Game { system: String, game: String },
}

impl Layer {
    /// How specific the layer is: 0 global, 1 system, 2 game.
    fn depth(&self) -> u8 { match self { Layer::Global => 0, Layer::System(_) => 1, Layer::Game { .. } => 2 } }

    /// The layers a value is looked up in, most specific first.
    fn chain(&self) -> Vec<Layer> {
        match self {
            Layer::Global => vec![Layer::Global],
            Layer::System(_) => vec![self.clone(), Layer::Global],
            Layer::Game { system, .. } => vec![self.clone(), Layer::System(system.clone()), Layer::Global],
        }
    }

    /// The layer's file under the `configs/` folder.
    pub fn path(&self, configs: &Path) -> PathBuf {
        match self {
            Layer::Global => configs.join("specular.json"),
            Layer::System(s) => configs.join("systems").join(format!("{s}.json")),
            Layer::Game { system, game } => configs.join("games").join(system).join(format!("{game}.json")),
        }
    }
}

/// What values a setting takes.
#[derive(Clone, Copy, Debug)]
pub enum SettingKind {
    Bool,
    /// One of these (stored value, label) pairs.
    Choice(&'static [(&'static str, &'static str)]),
    /// A whole number from `min` to `max` in steps of `step`, shown with `unit`.
    Int { min: i64, max: i64, step: i64, unit: &'static str },
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
    pub deepest: Layer,
}

impl SettingDecl {
    /// Whether `v` is a valid value for this setting.
    fn accepts(&self, v: &Value) -> bool {
        match self.kind {
            SettingKind::Bool => v.is_boolean(),
            SettingKind::Choice(options) => v.as_str().is_some_and(|s| options.iter().any(|(o, _)| *o == s)),
            SettingKind::Int { min, max, step, .. } => v.as_i64().is_some_and(|n| (min..=max).contains(&n) && (n - min) % step == 0),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SettingError { Unknown, WrongValue, NotAtThisLayer }

/// The declared settings and the layers loaded so far.
#[derive(Debug, Default)]
pub struct Settings {
    decls: Vec<SettingDecl>,
    layers: HashMap<Layer, Map<String, Value>>,
}

impl Settings {
    pub fn new(decls: Vec<SettingDecl>) -> Self { Self { decls, layers: HashMap::new() } }

    fn decl(&self, key: &str) -> Option<&SettingDecl> { self.decls.iter().find(|d| d.key == key) }

    /// Puts a layer's contents in place (as loaded by `store::load_layer`).
    pub fn set_layer(&mut self, layer: Layer, values: Map<String, Value>) { self.layers.insert(layer, values); }

    /// A layer's contents, for `store::store_layer`.
    pub fn layer(&self, layer: &Layer) -> Map<String, Value> { self.layers.get(layer).cloned().unwrap_or_default() }

    /// A setting's value as seen at `at`, and the layer it comes from (None = the default).
    pub fn get(&self, key: &str, at: &Layer) -> Option<(&Value, Option<Layer>)> {
        let d = self.decl(key)?;
        for layer in at.chain().into_iter().filter(|l| l.depth() <= d.deepest.depth()) {
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
        if at.depth() > d.deepest.depth() { return Err(SettingError::NotAtThisLayer); }
        let map = self.layers.entry(at.clone()).or_default();
        map.remove(key);
        let inherited = self.get(key, at).map(|(v, _)| v.clone());
        if inherited.as_ref() != Some(&v) { self.layers.get_mut(at).unwrap().insert(key.to_string(), v); }
        Ok(())
    }

    /// Removes the override at `at`, if any.
    pub fn reset(&mut self, key: &str, at: &Layer) {
        if let Some(m) = self.layers.get_mut(at) { m.remove(key); }
    }

    /// The settings screen for `at`: one row per setting that can be set there and is visible at
    /// `show`, under a header per section, in declaration order.
    pub fn rows(&self, at: &Layer, show: Visibility) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut section = "";
        for d in self.decls.iter().filter(|d| d.visibility <= show && at.depth() <= d.deepest.depth()) {
            if d.section != section {
                section = d.section;
                rows.push(Row::new(format!("section:{section}"), section, RowKind::Header));
            }
            let (v, from) = self.get(d.key, at).unwrap();
            let kind = match d.kind {
                SettingKind::Bool => RowKind::Toggle { on: v.as_bool().unwrap_or(false) },
                SettingKind::Choice(options) => RowKind::Choice {
                    options: options.iter().map(|(_, label)| label.to_string()).collect(),
                    selected: options.iter().position(|(o, _)| Some(*o) == v.as_str()).unwrap_or(0),
                },
                SettingKind::Int { min, max, step, unit } => RowKind::Number { value: v.as_i64().unwrap_or(min), min, max, step, unit },
            };
            let mut row = Row::new(d.key, d.label, kind);
            row.overridden = from.as_ref().is_some_and(|l| l == at) && at.depth() > 0;
            rows.push(row);
        }
        rows
    }

    /// Applies a press to the setting `key` shown at `at` (A or left/right on its row). Returns
    /// whether its value changed. Choices wrap around on a real press, not while held.
    pub fn press(&mut self, key: &str, at: &Layer, p: Press) -> bool {
        let Some(d) = self.decl(key).cloned() else { return false };
        let Some((current, _)) = self.get(key, at).map(|(v, l)| (v.clone(), l)) else { return false };
        let delta = match p.button { Button::Left => -1, Button::Right => 1, Button::A => 1, _ => return false };
        let new = match d.kind {
            SettingKind::Bool => { if p.repeat { return false; } Value::Bool(!current.as_bool().unwrap_or(false)) }
            SettingKind::Choice(options) => {
                let n = options.len() as i64;
                let i = options.iter().position(|(o, _)| Some(*o) == current.as_str()).unwrap_or(0) as i64 + delta;
                if !(0..n).contains(&i) && p.repeat { return false; }
                Value::from(options[i.rem_euclid(n) as usize].0)
            }
            SettingKind::Int { min, max, step, .. } => {
                if p.button == Button::A { return false; }
                Value::from((current.as_i64().unwrap_or(min) + delta * step).clamp(min, max))
            }
        };
        if new == current { return false; }
        self.set(key, at, new).is_ok()
    }
}

#[cfg(test)]
mod tests;
