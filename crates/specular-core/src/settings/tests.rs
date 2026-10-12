use std::path::PathBuf;

use serde_json::{Map, json};

use super::store::{LoadError, load_layer, store_layer};
use super::*;
use crate::input::{Button, Press};

const SCALING: &[(&str, &str)] = &[("integer", "Integer"), ("fit", "Fit"), ("stretch", "Stretch")];

fn decls() -> Vec<SettingDecl> {
    vec![
        SettingDecl { key: "display.brightness", section: "Display", label: "Brightness", kind: SettingKind::Int { min: 1, max: 10, step: 1, unit: "" },
            default: json!(7), visibility: Visibility::User, deepest: Layer::Global },
        SettingDecl { key: "video.scaling", section: "Display", label: "Scaling", kind: SettingKind::Choice(SCALING),
            default: json!("integer"), visibility: Visibility::User, deepest: Layer::Game { system: String::new(), game: String::new() } },
        SettingDecl { key: "dev.frame_stats", section: "Developer", label: "Frame stats", kind: SettingKind::Bool,
            default: json!(false), visibility: Visibility::Developer, deepest: Layer::Global },
    ]
}

fn gba() -> Layer { Layer::System("gba".into()) }
fn golden_sun() -> Layer { Layer::Game { system: "gba".into(), game: "Golden Sun".into() } }
fn press(button: Button) -> Press { Press { button, repeat: false } }
fn held(button: Button) -> Press { Press { button, repeat: true } }

#[test]
fn lookup_goes_from_game_to_system_to_global_to_default() {
    let mut s = Settings::new(decls());
    assert_eq!(s.get("video.scaling", &golden_sun()), Some((&json!("integer"), None)));
    s.set_layer(Layer::Global, Map::from_iter([("video.scaling".into(), json!("fit"))]));
    assert_eq!(s.get("video.scaling", &golden_sun()), Some((&json!("fit"), Some(Layer::Global))));
    s.set_layer(gba(), Map::from_iter([("video.scaling".into(), json!("stretch"))]));
    assert_eq!(s.get("video.scaling", &golden_sun()), Some((&json!("stretch"), Some(gba()))));
    assert_eq!(s.get("video.scaling", &Layer::Global).unwrap().0, &json!("fit"));
}

#[test]
fn global_only_settings_ignore_deeper_layers() {
    let mut s = Settings::new(decls());
    s.set_layer(gba(), Map::from_iter([("display.brightness".into(), json!(3))]));
    assert_eq!(s.get("display.brightness", &gba()).unwrap().0, &json!(7));
    assert_eq!(s.set("display.brightness", &gba(), json!(3)), Err(SettingError::NotAtThisLayer));
}

#[test]
fn bad_stored_values_count_as_absent() {
    let mut s = Settings::new(decls());
    s.set_layer(Layer::Global, Map::from_iter([("display.brightness".into(), json!(42)), ("video.scaling".into(), json!("zoom"))]));
    assert_eq!(s.get("display.brightness", &Layer::Global).unwrap().0, &json!(7));
    assert_eq!(s.get("video.scaling", &Layer::Global).unwrap().0, &json!("integer"));
}

#[test]
fn setting_the_inherited_value_removes_the_override() {
    let mut s = Settings::new(decls());
    s.set("video.scaling", &gba(), json!("fit")).unwrap();
    assert_eq!(s.layer(&gba()).get("video.scaling"), Some(&json!("fit")));
    s.set("video.scaling", &gba(), json!("integer")).unwrap(); // the default again
    assert!(s.layer(&gba()).is_empty());
    assert_eq!(s.set("video.scaling", &gba(), json!("zoom")), Err(SettingError::WrongValue));
    assert_eq!(s.set("nope", &gba(), json!(1)), Err(SettingError::Unknown));
}

#[test]
fn unknown_keys_are_kept() {
    let mut s = Settings::new(decls());
    s.set_layer(Layer::Global, Map::from_iter([("plugin.thing".into(), json!({"a": 1}))]));
    s.set("display.brightness", &Layer::Global, json!(4)).unwrap();
    let l = s.layer(&Layer::Global);
    assert_eq!(l.get("plugin.thing"), Some(&json!({"a": 1})));
    assert_eq!(l.get("display.brightness"), Some(&json!(4)));
}

#[test]
fn rows_follow_sections_visibility_and_layer() {
    let mut s = Settings::new(decls());
    let labels = |rows: &[crate::model::Row]| rows.iter().map(|r| r.label.clone()).collect::<Vec<_>>();
    assert_eq!(labels(&s.rows(&Layer::Global, Visibility::User)), ["Display", "Brightness", "Scaling"]);
    assert_eq!(labels(&s.rows(&Layer::Global, Visibility::Developer)), ["Display", "Brightness", "Scaling", "Developer", "Frame stats"]);
    // a game's settings show only what a game may override, marked when it does
    s.set("video.scaling", &golden_sun(), json!("stretch")).unwrap();
    let rows = s.rows(&golden_sun(), Visibility::User);
    assert_eq!(labels(&rows), ["Display", "Scaling"]);
    assert!(rows[1].overridden);
    assert_eq!(rows[1].kind, RowKind::Choice { options: vec!["Integer".into(), "Fit".into(), "Stretch".into()], selected: 2 });
}

#[test]
fn presses_change_values() {
    let mut s = Settings::new(decls());
    let g = Layer::Global;
    // numbers step and stop at their ends, also while held
    assert!(s.press("display.brightness", &g, press(Button::Right)));
    assert_eq!(s.get("display.brightness", &g).unwrap().0, &json!(8));
    for _ in 0..5 { s.press("display.brightness", &g, held(Button::Right)); }
    assert_eq!(s.get("display.brightness", &g).unwrap().0, &json!(10));
    assert!(!s.press("display.brightness", &g, press(Button::A)));
    // choices wrap on a press, not while held
    assert!(s.press("video.scaling", &g, press(Button::Left)));
    assert_eq!(s.get("video.scaling", &g).unwrap().0, &json!("stretch"));
    assert!(!s.press("video.scaling", &g, held(Button::Right)));
    assert!(s.press("video.scaling", &g, press(Button::Right)));
    assert_eq!(s.get("video.scaling", &g).unwrap().0, &json!("integer"));
    // toggles flip on a press only
    assert!(s.press("dev.frame_stats", &g, press(Button::A)));
    assert_eq!(s.get("dev.frame_stats", &g).unwrap().0, &json!(true));
    assert!(!s.press("dev.frame_stats", &g, held(Button::Left)));
}

/// A fresh, empty directory for one test.
fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("specular-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn layers_round_trip_through_files() {
    let dir = temp_dir("round-trip");
    let path = golden_sun().path(&dir);
    assert!(path.ends_with("games/gba/Golden Sun.json"));
    assert!(load_layer(&path).unwrap().is_empty());
    let m = Map::from_iter([("video.scaling".into(), json!("fit")), ("plugin.x".into(), json!([1, 2]))]);
    store_layer(&path, &m).unwrap();
    assert_eq!(load_layer(&path).unwrap(), m);
    assert!(!path.with_extension("json.tmp").exists());
    store_layer(&path, &Map::new()).unwrap();
    assert!(!path.exists());
}

#[test]
fn an_interrupted_save_is_recovered() {
    let dir = temp_dir("interrupted");
    let path = Layer::Global.path(&dir);
    // the power went out after writing the new file, before renaming it
    std::fs::write(path.with_extension("json.tmp"), r#"{"display.brightness": 3}"#).unwrap();
    assert_eq!(load_layer(&path).unwrap().get("display.brightness"), Some(&json!(3)));
}

#[test]
fn a_corrupt_file_is_moved_aside() {
    let dir = temp_dir("corrupt");
    let path = Layer::Global.path(&dir);
    std::fs::write(&path, "{ not json").unwrap();
    match load_layer(&path) {
        Err(LoadError::Corrupt { moved_to }) => assert!(moved_to.exists()),
        other => panic!("{other:?}"),
    }
    assert!(!path.exists());
}
