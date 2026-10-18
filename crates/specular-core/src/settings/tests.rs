use serde_json::{Map, json};

use std::fs;
use std::path::Path;

use super::store::{LoadError, load_layer, store_generated, store_layer};
use super::*;
use crate::input::{Button, Press};

const SCALING: &[(&str, &str)] = &[("integer", "Integer"), ("fit", "Fit"), ("stretch", "Stretch")];

fn decls() -> Vec<SettingDecl> {
    vec![
        SettingDecl { key: "display.brightness", section: "Display", label: "Brightness", help: "How bright.", kind: SettingKind::Int { min: 1, max: 10, step: 1, unit: "" },
            default: json!(7), visibility: Visibility::User, deepest: Scope::Global },
        SettingDecl { key: "video.scaling", section: "Display", label: "Scaling", help: "How big.", kind: SettingKind::Choice(SCALING),
            default: json!("integer"), visibility: Visibility::User, deepest: Scope::Game },
        SettingDecl { key: "dev.frame_stats", section: "Developer", label: "Frame stats", help: "How fast.", kind: SettingKind::Bool,
            default: json!(false), visibility: Visibility::Developer, deepest: Scope::Global },
    ]
}

fn gba() -> Layer { Layer::System("gba".into()) }
fn golden_sun() -> Layer { Layer::Game { system: "gba".into(), game: "gba/Golden Sun".into() } }
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
    assert_eq!(rows[1].kind, RowKind::Choice { options: vec!["Integer".into(), "Fit".into(), "Stretch".into()], selected: 2, swatch: None });
}

#[test]
fn presses_change_values() {
    let mut s = Settings::new(decls());
    let g = Layer::Global;
    // numbers step and stop at their ends, also while held
    assert_eq!(s.press("display.brightness", &g, press(Button::Right)), Some(("display.brightness", json!(8))));
    assert_eq!(s.get("display.brightness", &g).unwrap().0, &json!(8));
    for _ in 0..5 { s.press("display.brightness", &g, held(Button::Right)); }
    assert_eq!(s.get("display.brightness", &g).unwrap().0, &json!(10));
    assert!(s.press("display.brightness", &g, press(Button::A)).is_none());
    // choices wrap on a press, not while held
    assert!(s.press("video.scaling", &g, press(Button::Left)).is_some());
    assert_eq!(s.get("video.scaling", &g).unwrap().0, &json!("stretch"));
    assert!(s.press("video.scaling", &g, held(Button::Right)).is_none());
    assert!(s.press("video.scaling", &g, press(Button::Right)).is_some());
    assert_eq!(s.get("video.scaling", &g).unwrap().0, &json!("integer"));
    // toggles flip on a press only
    assert!(s.press("dev.frame_stats", &g, press(Button::A)).is_some());
    assert_eq!(s.get("dev.frame_stats", &g).unwrap().0, &json!(true));
    assert!(s.press("dev.frame_stats", &g, held(Button::Left)).is_none());
}

#[test]
fn colours_step_through_the_palette_and_can_follow_another() {
    let colour = |key, same_as, default| SettingDecl { key, section: "Look", label: key, help: "", kind: SettingKind::Colour { same_as },
        default: json!(default), visibility: Visibility::User, deepest: Scope::Global };
    let mut s = Settings::new(vec![colour("Accent", None, "teal"), colour("second", Some("Accent"), "coral")]);
    let g = Layer::Global;
    assert_eq!(s.colour("second", &g), palette::find("coral"));
    // "same" comes first, and follows the other setting's colour
    assert_eq!(s.press("second", &g, press(Button::Right)), Some(("second", json!("amber"))));
    s.set("second", &g, json!(SAME)).unwrap();
    s.set("Accent", &g, json!("lime")).unwrap();
    assert_eq!(s.colour("second", &g), palette::find("lime"));
    let rows = s.rows(&g, Visibility::User);
    let RowKind::Choice { options, selected: 0, swatch } = &rows[2].kind else { panic!("{:?}", rows[2]) };
    assert_eq!((options[0].as_str(), options[1].as_str(), *swatch), ("Same as accent", "Teal", palette::find("lime")));
    // without `same_as` there is no "same"; a stored id not in the palette is ignored, and the
    // default applies
    assert_eq!(s.set("Accent", &g, json!(SAME)), Err(SettingError::WrongValue));
    s.layers.clear();
    s.set_layer(g.clone(), Map::from_iter([("Accent".into(), json!("mauve"))]));
    assert_eq!(s.colour("Accent", &g), palette::find("teal"));
    assert!(s.press("Accent", &g, press(Button::Left)).is_some());
    assert_eq!(s.get("Accent", &g).unwrap().0, &json!("lime"));
}

#[test]
fn layers_round_trip_through_files() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let path = golden_sun().path(dir);
    assert!(path.ends_with("games/gba/Golden Sun.json"));
    assert!(load_layer(&path).unwrap().is_empty());
    let m = Map::from_iter([("video.scaling".into(), json!("fit")), ("plugin.x".into(), json!([1, 2]))]);
    store_layer(&path, &m).unwrap();
    assert_eq!(load_layer(&path).unwrap(), m);
    assert!(!path.with_extension("json.tmp").exists());
    // emptying a layer removes its file, and any stale .tmp with it
    std::fs::write(path.with_extension("json.tmp"), "{}").unwrap();
    store_layer(&path, &Map::new()).unwrap();
    assert!(!path.exists() && !path.with_extension("json.tmp").exists());
}

#[test]
fn an_interrupted_save_is_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let path = Layer::Global.path(dir);
    // the power went out after writing the new file, before renaming it
    std::fs::write(path.with_extension("json.tmp"), r#"{"display.brightness": 3}"#).unwrap();
    assert_eq!(load_layer(&path).unwrap().get("display.brightness"), Some(&json!(3)));
}

#[test]
fn a_corrupt_file_is_moved_aside() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let path = Layer::Global.path(dir);
    std::fs::write(&path, "{ not json").unwrap();
    match load_layer(&path) {
        Err(LoadError::Corrupt { moved_to }) => assert!(moved_to.exists()),
        other => panic!("{other:?}"),
    }
    assert!(!path.exists());
}

#[test]
fn schemas_document_what_each_file_may_hold() {
    let gba = crate::library::builtin_systems().into_iter().find(|s| s.id == "gba").unwrap();
    let all = schema::all(&decls(), &[gba]);
    let files: Vec<_> = all.iter().map(|(f, _)| f.to_str().unwrap()).collect();
    assert_eq!(files, ["specular.schema.json", "systems/gba.schema.json", "games/gba.schema.json"]);
    let (global, system, game) = (&all[0].1, &all[1].1, &all[2].1);
    assert_eq!(global["$schema"], "https://json-schema.org/draft/2020-12/schema");
    assert!(global["$comment"].as_str().unwrap().contains("read-only"));
    assert_eq!(global["properties"]["display.brightness"],
        json!({"title": "Display: Brightness", "description": "How bright.", "type": "integer", "minimum": 1, "maximum": 10, "default": 7}));
    assert_eq!(global["properties"]["video.scaling"]["enum"], json!(["integer", "fit", "stretch"]));
    assert_eq!(global["properties"]["dev.frame_stats"]["type"], "boolean");
    // a system's and its games' files hold only what may be set that specifically
    for s in [system, game] {
        assert_eq!(s["properties"].as_object().unwrap().keys().collect::<Vec<_>>(), ["video.scaling"]);
    }
    assert_eq!(game["title"], "Game Boy Advance game settings");
    // a colour lists the palette, after "same" if it can follow another
    let colour = SettingDecl { key: "c", section: "", label: "", help: "", kind: SettingKind::Colour { same_as: Some("a") },
        default: json!("same"), visibility: Visibility::User, deepest: Scope::Global };
    let s = schema::schema(&[colour], Scope::Global, "", "");
    assert_eq!(s["properties"]["c"]["enum"].as_array().unwrap()[..2], [json!("same"), json!("teal")]);
}

#[test]
fn files_point_to_their_schema_which_is_not_a_setting() {
    assert_eq!(Layer::Global.schema_ref(), "specular.schema.json");
    assert_eq!(gba().schema_ref(), "gba.schema.json");
    assert_eq!(Layer::Game { system: "gba".into(), game: "Tetris".into() }.schema_ref(), "gba.schema.json");
    assert_eq!(Layer::Game { system: "gba".into(), game: "gba/RPG/Golden Sun".into() }.schema_ref(), "../../gba.schema.json");
    let dir = tempfile::tempdir().unwrap();
    let path = golden_sun().path(dir.path());
    let mut s = Settings::new(decls());
    // on reading, `$schema` (here a stale one) is dropped, not kept as an unknown key; saving
    // writes the current one
    s.set_layer(golden_sun(), Map::from_iter([(SCHEMA_KEY.into(), json!("old.json")), ("video.scaling".into(), json!("fit"))]));
    store_layer(&path, &s.layer(&golden_sun())).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("{\n  \"$schema\": \"../gba.schema.json\",\n  \"video.scaling\": \"fit\""), "{text}");
    let mut again = Settings::new(decls());
    again.set_layer(golden_sun(), load_layer(&path).unwrap());
    assert_eq!(again.layers[&golden_sun()], Map::from_iter([("video.scaling".into(), json!("fit"))]));
    // with no values left, there is no `$schema` either, so saving removes the file
    s.set("video.scaling", &golden_sun(), json!("integer")).unwrap();
    assert!(s.layer(&golden_sun()).is_empty());
}

#[test]
fn a_read_only_schema_is_replaced_only_when_it_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("games/gba.schema.json");
    let tmp = path.with_extension("json.tmp");
    assert!(store_generated(&path, &json!({"a": 1})).unwrap());
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    assert!(!store_generated(&path, &json!({"a": 1})).unwrap());
    // a read-only .tmp left by a cut-off write is no obstacle either
    fs::write(&tmp, "").unwrap();
    let mut ro = fs::metadata(&tmp).unwrap().permissions();
    ro.set_readonly(true);
    fs::set_permissions(&tmp, ro).unwrap();
    assert!(store_generated(&path, &json!({"a": 2})).unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), "{\n  \"a\": 2\n}\n");
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
}

/// A data root with settings in the legacy `configs/` folder, a damaged game file among them.
fn legacy_card() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let configs = root.path().join("configs");
    fs::create_dir_all(configs.join("systems")).unwrap();
    fs::create_dir_all(configs.join("games/gba/RPG")).unwrap();
    fs::write(configs.join("specular.json"), r#"{"display.brightness": 3}"#).unwrap();
    fs::write(configs.join("systems/gba.json"), r#"{"video.scaling": "fit"}"#).unwrap();
    fs::write(configs.join("games/gba/RPG/Golden Sun.json"), "{ damaged").unwrap();
    root
}

/// Every file under `dir`, relative to it, sorted.
fn files_under(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in fs::read_dir(d).unwrap().map(Result::unwrap) {
            if e.path().is_dir() { dirs.push(e.path()); } else { files.push(e.path().strip_prefix(dir).unwrap().to_string_lossy().into_owned()); }
        }
    }
    files.sort();
    files
}

#[test]
fn legacy_settings_are_copied_and_kept() {
    let root = legacy_card();
    let root = root.path();
    let before = files_under(&root.join("configs"));
    assert!(migrate::migrate(root));
    let config = migrate::config_dir(root);
    assert_eq!(config, root.join("config/v0"));
    assert_eq!(files_under(&config), before);
    assert_eq!(files_under(&root.join("configs")), before);
    assert_eq!(load_layer(&Layer::Global.path(&config)).unwrap().get("display.brightness"), Some(&json!(3)));
    assert!(!root.join("config/v0.tmp").exists());
    // a damaged file is copied as it is, and then set aside when read, as any other
    let game = Layer::Game { system: "gba".into(), game: "gba/RPG/Golden Sun".into() }.path(&config);
    assert_eq!(fs::read_to_string(&game).unwrap(), "{ damaged");
    assert!(matches!(load_layer(&game), Err(LoadError::Corrupt { .. })));
}

#[test]
fn once_migrated_the_legacy_folder_is_ignored() {
    let root = legacy_card();
    let root = root.path();
    migrate::migrate(root);
    // changes to configs/ after migrating, and files removed from v0, aren't copied again
    fs::write(root.join("configs/specular.json"), r#"{"display.brightness": 9}"#).unwrap();
    fs::remove_file(root.join("config/v0/systems/gba.json")).unwrap();
    assert!(migrate::migrate(root));
    assert_eq!(fs::read_to_string(root.join("config/v0/specular.json")).unwrap(), r#"{"display.brightness": 3}"#);
    assert!(!root.join("config/v0/systems/gba.json").exists());
}

#[test]
fn a_migration_cut_off_is_done_again() {
    let root = legacy_card();
    let root = root.path();
    // a half-built v0.tmp, with stale and extra files, is discarded rather than merged
    fs::create_dir_all(root.join("config/v0.tmp/systems")).unwrap();
    fs::write(root.join("config/v0.tmp/systems/gba.json"), "{").unwrap();
    fs::write(root.join("config/v0.tmp/leftover.json"), "{}").unwrap();
    assert!(migrate::migrate(root));
    assert_eq!(files_under(&root.join("config/v0")), files_under(&root.join("configs")));
    assert_eq!(fs::read_to_string(root.join("config/v0/systems/gba.json")).unwrap(), r#"{"video.scaling": "fit"}"#);
    assert!(!root.join("config/v0.tmp").exists());
}

#[test]
fn a_failed_migration_says_so_and_leaves_no_folder() {
    let root = legacy_card();
    let root = root.path();
    // the config folder can't be made: a file is in its place
    fs::write(root.join("config"), "").unwrap();
    assert!(!migrate::migrate(root));
    assert!(!migrate::config_dir(root).exists());
    // once it can, the next start migrates
    fs::remove_file(root.join("config")).unwrap();
    assert!(migrate::migrate(root));
    assert!(migrate::config_dir(root).join("specular.json").exists());
}

#[test]
fn a_fresh_card_has_nothing_to_migrate() {
    let root = tempfile::tempdir().unwrap();
    assert!(migrate::migrate(root.path()));
    assert!(!root.path().join("config").exists());
}
