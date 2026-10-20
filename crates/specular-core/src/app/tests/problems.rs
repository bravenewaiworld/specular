//! What needs attention (`app::problems`): home's row, the screen, and where each problem comes from.

use super::*;
use crate::library::user_cores::TurnedAway;

fn whats(a: &App) -> Vec<String> { a.problems().into_iter().map(|p| p.what).collect() }

fn turned_away(file: &str, why: &str) -> TurnedAway { TurnedAway { file: file.into(), why: why.into() } }

#[test]
fn home_counts_the_problems_and_opens_them() {
    let mut a = app();
    assert!(a.problems().is_empty());
    assert!(!labels(&a).iter().any(|l| l.contains("attention")));
    a.handle(Msg::LeftOut(LeftOut { no_emulator: vec![("Super Nintendo", 1)], ..Default::default() }));
    assert_eq!(labels(&a).last().map(String::as_str), Some("1 thing needs attention"));
    a.handle(Msg::LeftOut(LeftOut { no_emulator: vec![("Super Nintendo", 2), ("Mega Drive", 1)], ..Default::default() }));
    assert_eq!(press(&mut a, "2 things need attention"), vec![Effect::Redraw(Some(Transition::Push))]);
    assert_eq!(a.view().title, "Needs attention");
    assert_eq!(labels(&a), [
        "2 Super Nintendo games aren't shown",
        "No emulator on the device plays Super Nintendo games. Add an emulator core for Super Nintendo games to cores/ on the SD card (cores/README.txt there says how).",
        "1 Mega Drive game isn't shown",
        "No emulator on the device plays Mega Drive games. Add an emulator core for Mega Drive games to cores/ on the SD card (cores/README.txt there says how).",
    ]);
    // Put right meanwhile: the screen says so, and home's row goes.
    a.handle(Msg::LeftOut(LeftOut::default()));
    assert_eq!(labels(&a), ["Nothing needs attention."]);
    tap(&mut a, Button::B, 0);
    assert!(!labels(&a).iter().any(|l| l.contains("attention")));
}

#[test]
fn files_the_scan_left_out_and_settings_values_ignored_are_problems_but_unknown_keys_are_not() {
    let mut a = app();
    a.handle(Msg::LeftOut(LeftOut {
        set_aside: vec![("systems/gba.json.bad".into(), "it isn't a JSON object (expected value at line 1 column 1)".into())],
        orphans: vec!["gba/Old Name".into()],
        ..Default::default()
    }));
    let values = json!({ "display.brightness": 3, "video.scaling": "zoom", "plugin.thing": 1 }).as_object().unwrap().clone();
    a.handle(Msg::SettingsLoaded(game("gba", "Golden Sun").settings_layer(), values));
    let problems = a.problems();
    let labels: Vec<&str> = problems.iter().map(|p| p.what.as_str()).collect();
    assert_eq!(labels, [
        "config/v0/systems/gba.json couldn't be read",
        "display.brightness in config/v0/games/gba/Golden Sun.json is ignored",
        "video.scaling in config/v0/games/gba/Golden Sun.json is ignored",
        "config/v0/games/gba/Old Name.json matches no game",
    ]);
    assert_eq!(problems[0].why, "Its settings aren't used: it isn't a JSON object (expected value at line 1 column 1). Specular renamed it gba.json.bad.");
    assert_eq!(problems[0].fix, "On a PC, correct it (it is JSON text; the error says where) and rename it back to gba.json, replacing any gba.json made since; it is read when the device next starts. Or delete it to start over.");
    assert_eq!(problems[2].why, "\"zoom\" isn't one of its values, so it has no effect.");
    assert_eq!(problems[2].fix, "Change it in Settings on the device, or correct or remove it in the file on a PC. config/v0/games/gba.schema.json lists the settings the file can hold and the values each takes.");
    // Set right on the device, it is no longer ignored.
    a.handle(Msg::SettingsLoaded(Layer::Global, Default::default()));
    open_game_settings(&mut a);
    focus_on(&mut a, "Scaling");
    tap(&mut a, Button::Right, 0);
    assert!(!whats(&a).iter().any(|w| w.starts_with("video.scaling")), "{:?}", whats(&a));
}

/// Opens Golden Sun's settings from its system's list.
fn open_game_settings(a: &mut App) {
    press(a, "Game Boy Advance");
    tap(a, Button::Start, 0);
}

#[test]
fn a_failed_settings_write_is_a_problem_until_one_succeeds() {
    let mut a = app();
    assert_eq!(a.handle(Msg::SettingsSaved(Err("/sd/config/v0/specular.json: No space left on device".into()))), vec![Effect::Redraw(None)]);
    let p = &a.problems()[0];
    assert_eq!((p.what.as_str(), p.why.as_str()), ("Settings can't be saved",
        "Writing them to the SD card failed (/sd/config/v0/specular.json: No space left on device). Changes to settings last only until the device turns off."));
    assert_eq!(a.handle(Msg::SettingsSaved(Ok(()))), vec![Effect::Redraw(None)]);
    assert!(a.problems().is_empty());
    assert_eq!(a.handle(Msg::SettingsSaved(Ok(()))), vec![]); // nothing changed
}

#[test]
fn a_core_turned_away_opens_the_cores_screen_and_fonts_turned_away_are_listed_too() {
    let mut a = app();
    a.set_fonts_turned_away(vec![turned_away("notes.txt", "it isn't a TrueType or OpenType font")]);
    a.handle(Msg::Cores { systems: a.ctx().systems.clone(), settings: a.ctx().settings.decls().to_vec(),
                          turned_away: vec![turned_away("bare_libretro.so", "there is no bare_libretro.info beside it")] });
    assert_eq!(whats(&a), ["cores/bare_libretro.so isn't used", "fonts/notes.txt isn't used"]);
    assert_eq!(a.problems()[1].why, "It isn't a TrueType or OpenType font.");
    press(&mut a, "2 things need attention");
    assert_eq!(a.view().rows[a.focus()].hints, [(Button::A, "Details".into())]);
    tap(&mut a, Button::A, 0);
    assert_eq!(a.view().title, "Cores");
    // A font isn't the card's: it stays a problem once the card is gone, the core doesn't.
    a.handle(Msg::Card(Card { status: CardStatus::Absent, ..Card::mounted() }));
    assert_eq!(whats(&a), ["fonts/notes.txt isn't used"]);
}

#[test]
fn a_card_read_afresh_drops_what_the_last_one_left_out() {
    let mut a = app();
    a.handle(Msg::LeftOut(LeftOut { orphans: vec!["gba/Old".into()], ..Default::default() }));
    a.handle(Msg::SettingsSaved(Err("full".into())));
    assert_eq!(a.problems().len(), 2);
    a.handle(Msg::Card(Card { status: CardStatus::Absent, ..Card::mounted() }));
    a.handle(Msg::LeftOut(LeftOut { orphans: vec!["gba/Old".into()], ..Default::default() })); // late, from its scan
    a.handle(Msg::Card(Card::mounted()));
    assert!(a.problems().is_empty());
}

#[test]
fn the_cores_screen_is_in_settings_without_developer_details() {
    let mut a = app_with_settings();
    press(&mut a, "Settings");
    press(&mut a, "Cores");
    assert_eq!(a.view().title, "Cores");
}

#[test]
fn set_aside_files_are_looked_for_again_as_home_or_the_problems_are_shown_while_any_are_listed() {
    let mut a = app();
    let bad = ("systems/gba.json.bad".into(), "it isn't a JSON object (expected value at line 1 column 1)".into());
    a.handle(Msg::LeftOut(LeftOut { set_aside: vec![bad], ..Default::default() }));
    assert!(press(&mut a, "1 thing needs attention").contains(&Effect::CheckSetAside));
    // Renamed back on a PC meanwhile: the answer clears it.
    a.handle(Msg::SetAside(vec![]));
    assert_eq!(labels(&a), ["Nothing needs attention."]);
    // With none listed, going back home looks for none.
    assert!(!tap(&mut a, Button::B, 0).contains(&Effect::CheckSetAside));
    // An answer older than the scan that found a new one doesn't add it: only scans do.
    a.handle(Msg::SetAside(vec![("games/gba/X.json.bad".into(), "it can't be read".into())]));
    assert!(a.problems().is_empty());
}
