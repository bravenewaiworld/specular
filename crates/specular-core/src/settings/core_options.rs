//! Core options as settings. A libretro core declares its options when it is loaded: each a key,
//! a description, the values it takes and its default. For the cores Specular builds
//! (tools/build-cores.sh) the list is settled when they are built: `make cores` loads each one and
//! writes what it declares to `data/cores/<core>.json`, which is checked in and compiled in
//! (`CoreDecl::options`). From that list come the core's settings (`decls`): a developer setting
//! each, stored in `cores/<core>.json` and overridable per system and per game; and, for Specular's
//! own code, a constant for each option's key and values (`mgba::IDLE_OPTIMIZATION`, written by
//! build.rs). At launch the runtime compares what the core declares with the list and logs the
//! differences (`CoreOptions::differences`): the list comes from the core built for the PC, and
//! the device's build (for ARM) can declare other options, or other defaults.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{leak, Layer, Scope, SettingDecl, SettingKind, Settings, Visibility};
use crate::library::CoreDecl;

include!(concat!(env!("OUT_DIR"), "/core_options.rs"));

/// The options a core declares, in its order: the contents of `data/cores/<core>.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CoreOptions {
    /// The groups its options are in. Only cores declaring them in version 2 of
    /// libretro's core options interface have any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<Category>,
    pub options: Vec<CoreOption>,
}

/// A group of options, as a core declares it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Category {
    /// What its options name it by (`CoreOption::category`).
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
}

/// One option, as a core declares it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoreOption {
    pub key: String,
    /// Its description, e.g. "Idle Loop Removal".
    pub label: String,
    /// What it does, if the core says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
    /// The key of its `Category`, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub values: Vec<OptionValue>,
    /// The core's own default (one of `values`).
    pub default: String,
}

/// A value an option takes, and the label the core shows for it; None (the core gives none, or an
/// empty one) where the value is its own label.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OptionValue {
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl CoreOption {
    /// Whether `value` is one of its values.
    pub fn takes(&self, value: &str) -> bool { self.values.iter().any(|v| v.value == value) }
}

impl CoreOptions {
    /// An option, by key.
    pub fn option(&self, key: &str) -> Option<&CoreOption> { self.options.iter().find(|o| o.key == key) }

    /// How `declared` (what a core declared as it loaded) differs from this list, one line each:
    /// options only one of them has, and other defaults or values. Descriptions aren't compared.
    pub fn differences(&self, declared: &CoreOptions) -> Vec<String> {
        let values = |o: &CoreOption| o.values.iter().map(|v| v.value.clone()).collect::<Vec<_>>();
        let mut lines = Vec::new();
        for d in &declared.options {
            let Some(o) = self.option(&d.key) else { lines.push(format!("{} (\"{}\") is declared, but isn't in the list", d.key, d.label)); continue };
            if d.default != o.default { lines.push(format!("{}'s default is \"{}\", not \"{}\" as listed", d.key, d.default, o.default)); }
            if values(d) != values(o) { lines.push(format!("{}'s values are {:?}, not {:?} as listed", d.key, values(d), values(o))); }
        }
        for o in self.options.iter().filter(|o| declared.option(&o.key).is_none()) {
            lines.push(format!("{} is listed, but isn't declared", o.key));
        }
        lines
    }
}

/// A core's option list, parsed, and the settings made from it. Made once per core and list (a
/// core the user added gets another once its options are learned) and kept for the rest of the run,
/// as declarations take `'static` text, which they borrow from the list.
struct Compiled { core: &'static str, text: &'static str, list: &'static CoreOptions, decls: Vec<SettingDecl> }

fn compiled(core: &'static CoreDecl) -> &'static Compiled {
    static ALL: Mutex<Vec<&'static Compiled>> = Mutex::new(Vec::new());
    let mut all = ALL.lock().unwrap();
    if let Some(c) = all.iter().find(|c| c.core == core.id() && c.text == core.options) { return c; }
    // (A user core's list always parses: `user_cores` writes it from a parsed list.)
    let list: &'static CoreOptions = Box::leak(Box::new(serde_json::from_str(core.options)
        .unwrap_or_else(|e| panic!("{}'s option list (data/cores/) isn't a list of core options: {e}", core.id()))));
    // Grouped by category (a section each), in the categories' order.
    let mut options: Vec<&'static CoreOption> = list.options.iter().collect();
    options.sort_by_key(|o| list.categories.iter().position(|c| o.category.as_ref() == Some(&c.key)));
    let decls = options.into_iter().map(|o| decl(core, list, o)).collect();
    let c = Box::leak(Box::new(Compiled { core: core.id(), text: core.options, list, decls }));
    all.push(c);
    c
}

/// A core's option list (from `CoreDecl::options`; empty for a core the user added until its
/// options are learned).
pub fn list(core: &'static CoreDecl) -> &'static CoreOptions { compiled(core).list }

/// The settings for a core's options: developer settings (shown with "Developer details"), under
/// a section named after the core (and the option's category, if any), that may be set per system
/// and per game. A setting's default is Specular's (`CoreDecl::defaults`), else the core's.
pub fn decls(core: &'static CoreDecl) -> &'static [SettingDecl] { &compiled(core).decls }

fn decl(core: &'static CoreDecl, list: &'static CoreOptions, o: &'static CoreOption) -> SettingDecl {
    let category = list.categories.iter().find(|c| o.category.as_ref() == Some(&c.key));
    let section = category.map_or(core.name, |c| leak(format!("{}: {}", core.name, c.label)));
    let ours = core.default_of(&o.key);
    let choices: Vec<(&'static str, &'static str)> = o.values.iter().map(|v| (v.value.as_str(), v.label.as_deref().unwrap_or(&v.value))).collect();
    // The schema lists the values but not their labels, nor the core's default if it isn't
    // Specular's, so the help says them.
    let mut help = o.info.clone().unwrap_or_else(|| format!("{}.", o.label));
    if o.values.iter().any(|v| v.label.is_some()) {
        let each: Vec<String> = o.values.iter().map(|v| match &v.label { Some(l) => format!("\"{}\" ({l})", v.value), None => format!("\"{}\"", v.value) }).collect();
        help += &format!(" Values: {}.", each.join(", "));
    }
    if ours.is_some_and(|v| v != o.default) { help += &format!(" Specular's default; {}'s own is \"{}\".", core.name, o.default); }
    SettingDecl {
        key: &o.key, section, label: &o.label, help: leak(help),
        kind: SettingKind::Choice(choices.leak()),
        default: Value::from(ours.unwrap_or(&o.default)),
        visibility: Visibility::Developer, deepest: Scope::Game, core: Some(core.id()), system: None,
    }
}

/// The values to give a core's options for a game at `at` (as `LaunchSpec::options`): those set
/// in a layer, or given a default by Specular. The rest are left to the core: the device's build
/// of a core can have other defaults than the list, which comes from the PC's.
pub fn values(settings: &Settings, core: &'static CoreDecl, at: &Layer) -> Vec<(String, String)> {
    decls(core).iter().filter_map(|d| {
        let (v, from) = settings.get(d.key, at)?;
        if from.is_none() && core.default_of(d.key).is_none() { return None; }
        Some((d.key.to_string(), v.as_str()?.to_string()))
    }).collect()
}

#[cfg(test)]
mod tests;
