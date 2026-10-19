//! A running core's options: what it declares (in any of libretro's three versions), and the
//! value of each, which is the one Specular gives (`LaunchSpec::options`, `Session::set_options`)
//! if the core takes it, else the core's own default.

use std::collections::HashMap;
use std::ffi::{CString, c_char};

use specular_core::settings::core_options::{Category, CoreOption, CoreOptions, OptionValue};

use crate::core::{self, OPTION_VALUES_MAX, text};

/// The running core's options and their values.
#[derive(Default)]
pub struct Options {
    /// What the core declared.
    pub declared: CoreOptions,
    /// The values Specular gives, by key.
    given: Vec<(String, String)>,
    /// Each declared option's value, as the core reads it (GET_VARIABLE).
    values: HashMap<String, CString>,
    /// A value changed since the core last asked (GET_VARIABLE_UPDATE).
    pub updated: bool,
}

impl Options {
    /// With these values given, before the core declares its options.
    pub fn new(given: Vec<(String, String)>) -> Self { Self { given, ..Self::default() } }

    /// The core declared its options (again: a later declaration replaces an earlier one).
    pub fn declare(&mut self, declared: CoreOptions) {
        self.declared = declared;
        self.values = self.resolve();
    }

    /// An option's value (None for one the core didn't declare).
    pub fn get(&self, key: &str) -> Option<&CString> { self.values.get(key) }

    /// Gives the core these values instead (its other options go back to its defaults). Returns
    /// what changed, a line each ("option key: old → new"). Values it doesn't take aren't reported:
    /// Specular gives only values from the core's list in data/cores/, so a refusal means the
    /// core differs from its list, which the launch logged.
    pub fn give(&mut self, given: Vec<(String, String)>) -> Vec<String> {
        self.given = given;
        let values = self.resolve();
        let changes: Vec<String> = self.declared.options.iter().filter_map(|o| {
            let (old, new) = (self.values.get(&o.key)?, values.get(&o.key)?);
            (old != new).then(|| format!("option {}: {} → {}", o.key, old.to_string_lossy(), new.to_string_lossy()))
        }).collect();
        self.values = values;
        self.updated |= !changes.is_empty();
        changes
    }

    /// The values given that the core doesn't take, a line each: for an option it doesn't
    /// declare, or one it doesn't list. Its option keeps its default instead.
    pub fn refused(&self) -> Vec<String> {
        self.given.iter().filter_map(|(key, value)| match self.declared.option(key) {
            None => Some(format!("{key} = {value}: it has no such option")),
            Some(o) if !o.takes(value) => Some(format!("{key} = {value}: not one of its values; it keeps \"{}\"", o.default)),
            Some(_) => None,
        }).collect()
    }

    fn resolve(&self) -> HashMap<String, CString> {
        self.declared.options.iter().map(|o| {
            let given = self.given.iter().find(|(k, v)| *k == o.key && o.takes(v));
            let value = given.map_or(&o.default, |(_, v)| v);
            (o.key.clone(), CString::new(value.as_str()).unwrap_or_default())
        }).collect()
    }
}

/// Version 0 (SET_VARIABLES): each "Description; default|other|…", the default first.
///
/// # Safety
/// `v` points to an array of variables ending with a null key, as libretro.h says.
pub unsafe fn from_variables(mut v: *const core::Variable) -> CoreOptions {
    let mut options = Vec::new();
    unsafe {
        while !v.is_null() && !(*v).key.is_null() {
            let spec = text((*v).value).unwrap_or_default();
            let (label, values) = spec.split_once("; ").unwrap_or((&spec, ""));
            let values: Vec<OptionValue> = values.split('|').map(|v| OptionValue { value: v.into(), label: None }).collect();
            let default = values[0].value.clone();
            options.push(CoreOption { key: text((*v).key).unwrap_or_default(), label: label.into(), info: None, category: None, values, default });
            v = v.add(1);
        }
    }
    CoreOptions { categories: vec![], options }
}

/// Version 1 (SET_CORE_OPTIONS, or SET_CORE_OPTIONS_INTL's English ones).
///
/// # Safety
/// `d` points to an array of definitions ending with a null key, as libretro.h says.
pub unsafe fn from_v1(mut d: *const core::OptionDefinition) -> CoreOptions {
    let mut options = Vec::new();
    unsafe {
        while !d.is_null() && !(*d).key.is_null() {
            let o = &*d;
            options.push(option(o.key, o.desc, o.info, std::ptr::null(), &o.values, o.default_value));
            d = d.add(1);
        }
    }
    CoreOptions { categories: vec![], options }
}

/// Version 2 (SET_CORE_OPTIONS_V2, or SET_CORE_OPTIONS_V2_INTL's English ones).
///
/// # Safety
/// `o` is null or points to options whose arrays end with a null key, as libretro.h says.
pub unsafe fn from_v2(o: *const core::OptionsV2) -> CoreOptions {
    let mut list = CoreOptions::default();
    if o.is_null() { return list; }
    unsafe {
        let (mut c, mut d) = ((*o).categories, (*o).definitions);
        while !c.is_null() && !(*c).key.is_null() {
            list.categories.push(Category { key: text((*c).key).unwrap_or_default(), label: text((*c).desc).unwrap_or_default(), info: text((*c).info) });
            c = c.add(1);
        }
        while !d.is_null() && !(*d).key.is_null() {
            let o = &*d;
            list.options.push(option(o.key, o.desc, o.info, o.category_key, &o.values, o.default_value));
            d = d.add(1);
        }
    }
    list
}

/// One option of version 1 or 2. Its default is the first value if it gives none, or one it
/// doesn't list, as libretro.h says.
fn option(key: *const c_char, desc: *const c_char, info: *const c_char, category: *const c_char,
          values: &[core::OptionValue; OPTION_VALUES_MAX], default: *const c_char) -> CoreOption {
    let values: Vec<OptionValue> = values.iter().take_while(|v| !v.value.is_null())
        .map(|v| OptionValue { value: text(v.value).unwrap_or_default(), label: text(v.label).filter(|l| !l.is_empty()) }).collect();
    let mut o = CoreOption { key: text(key).unwrap_or_default(), label: text(desc).unwrap_or_default(), info: text(info).filter(|i| !i.is_empty()),
                             category: text(category).filter(|c| !c.is_empty()), values, default: String::new() };
    o.default = text(default).filter(|d| o.takes(d)).or_else(|| o.values.first().map(|v| v.value.clone())).unwrap_or_default();
    o
}
