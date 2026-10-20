use serde_json::{Map, json};

use std::fs;
use std::path::Path;

use super::store::{LoadError, json_text, load_layer, store_generated, store_layer};
use super::*;
use crate::input::{Button, Press};

const SCALING: &[(&str, &str)] = &[("integer", "Integer"), ("fit", "Fit"), ("stretch", "Stretch")];

fn decls() -> Vec<SettingDecl> {
    vec![
        SettingDecl { key: "display.brightness", section: "Display", label: "Brightness", help: "How bright.", kind: SettingKind::Int { min: 1, max: 10, step: 1, unit: "" },
            default: json!(7), visibility: Visibility::User, deepest: Scope::Global, core: None, system: None },
        SettingDecl { key: "video.scaling", section: "Display", label: "Scaling", help: "How big.", kind: SettingKind::Choice(SCALING),
            default: json!("integer"), visibility: Visibility::User, deepest: Scope::Game, core: None, system: None },
        SettingDecl { key: "dev.frame_stats", section: "Developer", label: "Frame stats", help: "How fast.", kind: SettingKind::Bool,
            default: json!(false), visibility: Visibility::Developer, deepest: Scope::Global, core: None, system: None },
    ]
}

/// A core's option, as `core_options::decls` makes them.
fn idle() -> SettingDecl {
    SettingDecl { key: "mgba_idle", section: "mGBA", label: "Idle loops", help: "Skips them.", kind: SettingKind::Choice(&[("on", "on"), ("off", "off")]),
        default: json!("on"), visibility: Visibility::Developer, deepest: Scope::Game, core: Some("mgba"), system: None }
}

fn gba() -> Layer { Layer::System("gba".into()) }
fn mgba() -> Layer { Layer::Core("mgba".into()) }
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
    assert_eq!(labels(&s.rows(&Layer::Global, Visibility::User, None)), ["Display", "Brightness", "Scaling"]);
    assert_eq!(labels(&s.rows(&Layer::Global, Visibility::Developer, None)), ["Display", "Brightness", "Scaling", "Developer", "Frame stats"]);
    // a game's settings show only what a game may override, marked when it does
    s.set("video.scaling", &golden_sun(), json!("stretch")).unwrap();
    let rows = s.rows(&golden_sun(), Visibility::User, None);
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
        default: json!(default), visibility: Visibility::User, deepest: Scope::Global, core: None, system: None };
    let mut s = Settings::new(vec![colour("Accent", None, "teal"), colour("second", Some("Accent"), "coral")]);
    let g = Layer::Global;
    assert_eq!(s.colour("second", &g), palette::find("coral"));
    // "same" comes first, and follows the other setting's colour
    assert_eq!(s.press("second", &g, press(Button::Right)), Some(("second", json!("amber"))));
    s.set("second", &g, json!(SAME)).unwrap();
    s.set("Accent", &g, json!("lime")).unwrap();
    assert_eq!(s.colour("second", &g), palette::find("lime"));
    let rows = s.rows(&g, Visibility::User, None);
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
        Err(LoadError::Corrupt { moved_to, reason }) => assert!(moved_to.exists() && reason.contains("line 1 column 3"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(!path.exists());
}

#[test]
fn schemas_document_what_each_file_may_hold() {
    let gba = crate::library::builtin_systems().into_iter().find(|s| s.id == "gba").unwrap();
    let all = schema::all(&[decls(), vec![idle()]].concat(), &[gba]);
    let files: Vec<_> = all.iter().map(|(f, _)| f.to_str().unwrap()).collect();
    assert_eq!(files, ["specular.schema.json", "systems/gba.schema.json", "games/gba.schema.json", "cores/mgba.schema.json"]);
    let (global, system, game, core) = (&all[0].1, &all[1].1, &all[2].1, &all[3].1);
    assert_eq!(global["$schema"], "https://json-schema.org/draft/2020-12/schema");
    assert!(global["$comment"].as_str().unwrap().contains("read-only"));
    // each says where else it may be set, and a choice's labels go with its values
    assert_eq!(global["properties"]["display.brightness"],
        json!({"title": "Display: Brightness", "description": "How bright. Can be set only in specular.json.", "type": "integer", "minimum": 1, "maximum": 10, "default": 7}));
    assert_eq!(global["properties"]["video.scaling"]["description"], "How big. Can be set in specular.json, systems/<id>.json or a game's file in games/.");
    assert_eq!(global["properties"]["video.scaling"]["enum"], json!(["integer", "fit", "stretch"]));
    assert_eq!(global["properties"]["video.scaling"]["enumDescriptions"], json!(["Integer", "Fit", "Stretch"]));
    assert_eq!(global["properties"]["dev.frame_stats"]["type"], "boolean");
    assert_eq!(core["properties"]["mgba_idle"]["description"], "Skips them. Can be set in cores/mgba.json, systems/<id>.json or a game's file in games/.");
    // a system's and its games' files hold what may be set that specifically, and its core's
    // options; the core's file only those. The settings that can't be set there are marked as
    // never valid, so an editor flags them, saying where they can be.
    let keys = |s: &Value, valid: bool| s["properties"].as_object().unwrap().iter().filter(|(_, p)| p.get("not").is_none() == valid).map(|(k, _)| k.clone()).collect::<Vec<_>>();
    for s in [system, game] {
        assert_eq!(keys(s, true), ["video.scaling", "mgba_idle"]);
        assert_eq!(keys(s, false), ["display.brightness", "dev.frame_stats"]);
    }
    assert_eq!(system["properties"]["display.brightness"], json!({"not": {}, "description": "Can only be set in specular.json, not in this file."}));
    assert_eq!(keys(core, true), ["mgba_idle"]);
    assert_eq!(keys(core, false), ["display.brightness", "video.scaling", "dev.frame_stats"]);
    assert_eq!(keys(global, false), ["mgba_idle"]);
    assert_eq!(game["title"], "Game Boy Advance game settings");
    assert!(game["description"].as_str().unwrap().contains("roms/gba/RPG/Some Game.gba → games/gba/RPG/Some Game.json"), "{}", game["description"]);
    assert!(system["description"].as_str().unwrap().contains("This file is systems/gba.json"), "{}", system["description"]);
    assert_eq!(core["title"], "mGBA options");
    // a colour lists the palette, after "same" if it can follow another
    let colour = SettingDecl { key: "c", section: "", label: "", help: "", kind: SettingKind::Colour { same_as: Some("a") },
        default: json!("same"), visibility: Visibility::User, deepest: Scope::Global, core: None, system: None };
    let s = schema::schema(&[colour], &Layer::Global, &[], "", "");
    assert_eq!(s["properties"]["c"]["enum"].as_array().unwrap()[..2], [json!("same"), json!("teal")]);
    assert_eq!(s["properties"]["c"]["enumDescriptions"][1], "Teal");
}

#[test]
fn a_setting_only_for_the_config_files_is_in_their_schemas_but_on_no_screen() {
    let decls = builtin::settings();
    let ps1 = Layer::Game { system: "ps1".into(), game: "ps1/Vagrant Story".into() };
    let s = schema::schema(&decls, &ps1, &[], "", "");
    assert_eq!(s["properties"][builtin::CPU_MIN]["default"], "auto");
    assert_eq!(s["properties"][builtin::CPU_MIN]["enum"].as_array().unwrap()[..3], [json!("auto"), json!("off"), json!("408")]);
    assert_eq!(s["properties"][builtin::AUTOSAVE]["default"], "core default");
    assert_eq!(s["properties"][builtin::AUTOSAVE]["enum"], json!(["core default", "every 10 s", "at pauses"]));
    let settings = Settings::new(decls);
    for at in [Layer::Global, ps1] {
        assert!(settings.rows(&at, Visibility::Developer, None).iter().all(|r| r.id.0 != builtin::CPU_MIN && r.id.0 != builtin::AUTOSAVE));
    }
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
    assert!(store_generated(&path, &json_text(&json!({"a": 1}))).unwrap());
    assert!(fs::metadata(&path).unwrap().permissions().readonly());
    assert!(!store_generated(&path, &json_text(&json!({"a": 1}))).unwrap());
    // a read-only .tmp left by a cut-off write is no obstacle either
    fs::write(&tmp, "").unwrap();
    let mut ro = fs::metadata(&tmp).unwrap().permissions();
    ro.set_readonly(true);
    fs::set_permissions(&tmp, ro).unwrap();
    assert!(store_generated(&path, &json_text(&json!({"a": 2}))).unwrap());
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

#[test]
fn a_cores_options_are_looked_up_in_its_file_not_the_global_one() {
    let mut s = Settings::new([decls(), vec![idle()]].concat());
    assert_eq!(mgba().path(Path::new("")), Path::new("cores/mgba.json"));
    assert_eq!(mgba().schema_ref(), "mgba.schema.json");
    s.set_layer(Layer::Global, Map::from_iter([("mgba_idle".into(), json!("off"))]));
    assert_eq!(s.get("mgba_idle", &golden_sun()), Some((&json!("on"), None)));
    assert_eq!(s.set("mgba_idle", &Layer::Global, json!("off")), Err(SettingError::NotAtThisLayer));
    assert_eq!(s.set("display.brightness", &mgba(), json!(3)), Err(SettingError::NotAtThisLayer));
    s.set("mgba_idle", &mgba(), json!("off")).unwrap();
    assert_eq!(s.get("mgba_idle", &golden_sun()), Some((&json!("off"), Some(mgba()))));
    s.set("mgba_idle", &golden_sun(), json!("on")).unwrap();
    assert_eq!(s.get("mgba_idle", &golden_sun()), Some((&json!("on"), Some(golden_sun()))));
    assert_eq!(s.get("mgba_idle", &gba()).unwrap().0, &json!("off"));
    // rows are Specular's settings, or one core's options
    let keys = |rows: Vec<crate::model::Row>| rows.into_iter().filter(|r| r.kind != RowKind::Header).map(|r| r.id.0).collect::<Vec<_>>();
    assert_eq!(keys(s.rows(&golden_sun(), Visibility::Developer, None)), ["video.scaling"]);
    assert_eq!(keys(s.rows(&golden_sun(), Visibility::Developer, Some("mgba"))), ["mgba_idle"]);
    assert!(s.rows(&golden_sun(), Visibility::User, Some("mgba")).is_empty());
}

#[test]
fn resetting_an_override_inherits_again() {
    let mut s = Settings::new(decls());
    s.set("video.scaling", &gba(), json!("fit")).unwrap();
    s.set("video.scaling", &golden_sun(), json!("stretch")).unwrap();
    assert_eq!(s.reset("video.scaling", &golden_sun()), Some("video.scaling"));
    assert_eq!(s.get("video.scaling", &golden_sun()), Some((&json!("fit"), Some(gba()))));
    assert_eq!(s.reset("video.scaling", &golden_sun()), None); // nothing left to reset
    assert_eq!(s.reset("nope", &gba()), None);
}

#[test]
fn values_a_file_holds_in_vain_are_reported() {
    let values = Map::from_iter([("display.brightness".into(), json!(3)), ("video.scaling".into(), json!("zoom")),
                                 ("plugin.thing".into(), json!(1)), ("mgba_idle".into(), json!("off"))]);
    let decls = [decls(), vec![idle()]].concat();
    let pair = |k: &str, why: &str| (k.to_string(), why.to_string());
    assert_eq!(ignored(&decls, &golden_sun(), &values), [
        pair("display.brightness", "it can't be set in this file"),
        pair("video.scaling", "\"zoom\" isn't one of its values"),
    ]);
    assert_eq!(ignored(&decls, &Layer::Global, &values).len(), 2); // brightness is fine there, the core's option isn't
}

#[test]
fn games_with_settings_are_found_from_their_files() {
    let dir = tempfile::tempdir().unwrap();
    let games = dir.path().join("games");
    fs::create_dir_all(games.join("gba/RPG")).unwrap();
    for f in ["gba.schema.json", "gba/RPG/Golden Sun.json", "gba/Tetris.json.tmp", "gba/Mother 3.json", "gba/Mother 3.json.tmp",
              "gba/Broken.json.bad", "gba/notes.txt"] {
        fs::write(games.join(f), "{}").unwrap();
    }
    assert_eq!(games_with_settings(dir.path()).into_iter().collect::<Vec<_>>(), ["gba/Mother 3", "gba/RPG/Golden Sun", "gba/Tetris"]);
    assert!(games_with_settings(&dir.path().join("none")).is_empty());
}

#[test]
fn what_depends_on_the_core_is_said_for_each_core() {
    let decls = declarations(&crate::library::builtin_systems());
    let help = |key| decls.iter().find(|d| d.key == key).unwrap().help;
    assert!(help(builtin::CPU_MIN).ends_with("Automatic starts at 1800 MHz for SwanStation; the speed the system chooses for the others."), "{}", help(builtin::CPU_MIN));
    assert!(help(builtin::AUTOSAVE).ends_with("Core default is At pauses for SwanStation; Every 10 s for the others."), "{}", help(builtin::AUTOSAVE));
}
