use serde_json::json;

use super::*;
use crate::library::{builtin_systems, cores};

fn core(id: &str) -> &'static CoreDecl { cores(&builtin_systems()).into_iter().find(|c| c.id() == id).unwrap() }

#[test]
fn every_core_built_has_an_option_list_and_every_list_a_core() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = std::fs::read_to_string(root.join("tools/build-cores.sh")).unwrap();
    let mut built: Vec<&str> = script.lines().filter_map(|l| l.strip_prefix("patched_src ")?.split_whitespace().next()).collect();
    let mut listed: Vec<String> = std::fs::read_dir(root.join("data/cores")).unwrap().flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(String::from)).collect();
    built.sort();
    listed.sort();
    assert_eq!(built, listed, "each core tools/build-cores.sh builds needs data/cores/<core>.json, which `make cores` writes");
    // and Specular's own cores are the ones built, each with its list
    let mut ours: Vec<&str> = cores(&builtin_systems()).iter().map(|c| c.id()).collect();
    ours.sort();
    assert_eq!(ours, built);
}

#[test]
fn specular_defaults_are_values_the_cores_list() {
    for c in cores(&builtin_systems()) {
        assert!(!decls(c).is_empty(), "{}", c.id());
        for (key, value) in c.defaults {
            let o = list(c).option(key).unwrap_or_else(|| panic!("{} has no option {key}", c.name));
            assert!(o.values.iter().any(|v| v.value == *value), "{key} has no value {value}");
            assert_eq!(decls(c).iter().find(|d| d.key == *key).unwrap().default, json!(value));
        }
    }
}

#[test]
fn a_cores_options_are_developer_settings_grouped_by_category() {
    let nestopia = core("nestopia");
    let d = decls(nestopia);
    assert!(d.iter().all(|d| d.visibility == Visibility::Developer && d.deepest == Scope::Game && d.core == Some("nestopia")));
    let filter = d.iter().find(|d| d.key == nestopia::BLARGG_NTSC_FILTER).unwrap();
    assert_eq!(filter.section, "Nestopia: Video");
    assert_eq!(filter.default, json!("disabled")); // Specular's, not the core's "composite"
    assert!(filter.help.contains("\"composite\" (Composite Video)"), "{}", filter.help);
    assert!(filter.help.ends_with("Specular's default; Nestopia's own is \"composite\"."), "{}", filter.help);
    // each section once
    let mut sections: Vec<&str> = d.iter().map(|d| d.section).collect();
    sections.dedup();
    let mut unique = sections.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(sections.len(), unique.len(), "{sections:?}");
}

#[test]
fn a_game_gets_the_values_set_in_its_layers_and_speculars_defaults() {
    let mgba = core("mgba");
    let mut s = Settings::new(super::super::declarations(&builtin_systems()));
    let game = Layer::Game { system: "gba".into(), game: "gba/Golden Sun".into() };
    assert_eq!(values(&s, mgba, &game), [(mgba::IDLE_OPTIMIZATION.to_string(), "Remove Known".to_string())]);
    s.set(mgba::FRAMESKIP, &Layer::Core("mgba".into()), json!(mgba::frameskip::AUTO)).unwrap();
    s.set(mgba::IDLE_OPTIMIZATION, &Layer::System("gba".into()), json!(mgba::idle_optimization::DONT_REMOVE)).unwrap();
    let v = values(&s, mgba, &game);
    assert!(v.contains(&(mgba::FRAMESKIP.into(), "auto".into())) && v.contains(&(mgba::IDLE_OPTIMIZATION.into(), "Don't Remove".into())), "{v:?}");
    assert_eq!(v.len(), 2);
    // a GB game isn't affected by the GBA's settings
    let gb = Layer::Game { system: "gb".into(), game: "gb/Tetris".into() };
    assert!(values(&s, mgba, &gb).contains(&(mgba::IDLE_OPTIMIZATION.into(), "Remove Known".into())));
}

#[test]
fn differences_name_what_a_core_declares_otherwise() {
    let value = |v: &str| OptionValue { value: v.into(), label: None };
    let option = |key: &str, values: &[&str], default: &str| CoreOption {
        key: key.into(), label: key.to_uppercase(), info: None, category: None, values: values.iter().map(|v| value(v)).collect(), default: default.into(),
    };
    let listed = CoreOptions { categories: vec![], options: vec![option("a", &["x", "y"], "x"), option("b", &["on"], "on"), option("gone", &["1"], "1")] };
    assert!(listed.differences(&listed).is_empty());
    let declared = CoreOptions { categories: vec![], options: vec![option("a", &["x", "y"], "y"), option("b", &["on", "off"], "on"), option("new", &["1"], "1")] };
    assert_eq!(listed.differences(&declared), [
        "a's default is \"y\", not \"x\" as listed",
        "b's values are [\"on\", \"off\"], not [\"on\"] as listed",
        "new (\"NEW\") is declared, but isn't in the list",
        "gone is listed, but isn't declared",
    ]);
}
