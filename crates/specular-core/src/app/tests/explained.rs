//! What the screens explain where it matters: a setting's help, the hints for paging and the
//! letter index, the saved state's time and the questions before losing play, and how the last
//! run ended.

use super::*;
use crate::device::LastShutdown;

fn hints(a: &App) -> Vec<(Button, String)> {
    let v = a.view();
    v.rows.get(a.focus()).map_or(vec![], |r| r.hints.clone()).into_iter().chain(v.hints.clone()).collect()
}

#[test]
fn y_on_a_setting_shows_its_help() {
    let mut a = app_with_settings();
    tap(&mut a, Button::Up, 0); // wraps to Settings
    tap(&mut a, Button::A, 10);
    assert_eq!(focused(&a), "Brightness");
    assert!(hints(&a).contains(&(Button::Y, "About".into())));
    tap(&mut a, Button::Y, 20);
    let help = a.ctx().settings.decl(builtin::BRIGHTNESS).unwrap().help;
    assert_eq!((a.view().title.as_str(), labels(&a)), ("Brightness", vec!["Display".to_string(), help.to_string()]));
    assert_eq!(a.view().hints, [(Button::B, "Back".into())]);
    tap(&mut a, Button::B, 30);
    assert_eq!((a.view().title.as_str(), focused(&a)), ("Settings", "Brightness".into()));
    // Rows that aren't settings have none.
    focus_on(&mut a, "Credits");
    assert!(!hints(&a).contains(&(Button::Y, "About".into())));
    assert_eq!(tap(&mut a, Button::Y, 40), vec![]);
}

#[test]
fn a_long_help_is_read_a_few_lines_a_press() {
    let mut a = app_with_settings();
    a.set_list_area(1024.0, 300.0);
    tap(&mut a, Button::Up, 0);
    tap(&mut a, Button::A, 10);
    focus_on(&mut a, "Art");
    tap(&mut a, Button::Y, 20);
    assert_eq!(a.reading(), Some(0.0)); // its note doesn't fit, so it takes the focus
    tap(&mut a, Button::Down, 30);
    assert!(a.reading().is_some_and(|r| r > 0.0));
}

#[test]
fn a_games_settings_show_help_beside_reset() {
    let mut a = app();
    tap(&mut a, Button::A, 0); // Game Boy Advance
    tap(&mut a, Button::Start, 10);
    tap(&mut a, Button::Right, 20); // Scaling, now set for the game
    assert_eq!(a.view().rows[a.focus()].hints, [(Button::X, "Reset".into()), (Button::Y, "About".into())]);
    tap(&mut a, Button::Y, 30);
    assert_eq!(a.view().title, "Scaling");
}

#[test]
fn game_lists_hint_at_paging_and_the_letter_index() {
    let mut a = app();
    tap(&mut a, Button::A, 0);
    let shoulders = |a: &App| a.view().hints.iter().filter(|(b, _)| matches!(b, Button::L1 | Button::R1 | Button::L2 | Button::R2)).cloned().collect::<Vec<_>>();
    assert_eq!(shoulders(&a), [(Button::L1, "Page".into()), (Button::R1, "Page".into()), (Button::L2, "Letter".into()), (Button::R2, "Letter".into())]);
    tap(&mut a, Button::X, 10);
    type_text(&mut a, "golden");
    tap(&mut a, Button::Start, 20);
    assert_eq!(shoulders(&a), [(Button::L1, "Page".into()), (Button::R1, "Page".into())]); // results aren't by title
}

/// The app playing Golden Sun, its menu open.
fn paused() -> App {
    let mut a = app();
    tap(&mut a, Button::A, 0);
    tap(&mut a, Button::A, 10);
    tap(&mut a, Button::Menu, 20);
    a
}

fn at(year: i32, month: u8, day: u8, hour: u8, minute: u8) -> LocalTime { LocalTime { year, month, day, hour, minute } }

#[test]
fn the_game_menu_says_when_the_state_was_saved() {
    let mut a = paused();
    assert!(labels(&a).contains(&"No state saved yet: Save state makes one (one per game).".into()));
    focus_on(&mut a, "Load state");
    assert_eq!(tap(&mut a, Button::A, 30), vec![]); // nothing to load
    let note = |a: &App| labels(a).into_iter().find(|l| l.starts_with("State saved")).unwrap();
    let now = Some(at(2026, 10, 19, 15, 0));
    a.handle(Msg::SavedState(Some(SavedState { made: at(2026, 10, 19, 14, 32), now })));
    assert_eq!(note(&a), "State saved at 14:32. One per game: Save state replaces it, Load state goes back to it.");
    a.handle(Msg::SavedState(Some(SavedState { made: at(2026, 10, 18, 9, 5), now })));
    assert!(note(&a).starts_with("State saved on 18 Oct."));
    a.handle(Msg::SavedState(Some(SavedState { made: at(2025, 3, 2, 9, 5), now })));
    assert!(note(&a).starts_with("State saved on 2 Mar 2025."));
    a.handle(Msg::SavedState(Some(SavedState { made: at(2026, 10, 19, 9, 5), now: None }))); // the clock isn't set
    assert!(note(&a).starts_with("State saved on 19 Oct 2026."));
}

#[test]
fn loading_the_state_asks_first() {
    let mut a = paused();
    a.handle(Msg::SavedState(Some(SavedState { made: at(2026, 10, 19, 14, 32), now: Some(at(2026, 10, 19, 15, 0)) })));
    focus_on(&mut a, "Load state");
    assert_eq!(tap(&mut a, Button::A, 30), vec![Effect::Redraw(Some(Transition::Push))]);
    assert_eq!(labels(&a), ["Load the state saved at 14:32? Progress since then is lost.", "Load state", "Cancel"]);
    assert_eq!(focused(&a), "Load state");
    assert_eq!(tap(&mut a, Button::B, 40), vec![Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!(a.view().title, "Paused");
    tap(&mut a, Button::A, 50);
    assert_eq!(tap(&mut a, Button::A, 60), vec![Effect::LoadState, Effect::PauseGame(false), Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!(a.input_target(), InputTarget::Game);
}

#[test]
fn restarting_the_game_asks_first() {
    let mut a = paused();
    focus_on(&mut a, "Restart game");
    tap(&mut a, Button::A, 30);
    assert_eq!(a.view().title, "Restart game");
    focus_on(&mut a, "Cancel");
    assert_eq!(tap(&mut a, Button::A, 40), vec![Effect::Redraw(Some(Transition::Pop))]);
    tap(&mut a, Button::A, 50);
    assert_eq!(tap(&mut a, Button::Menu, 60), vec![Effect::PauseGame(false), Effect::Redraw(Some(Transition::Pop))]); // resumes
    tap(&mut a, Button::Menu, 70);
    focus_on(&mut a, "Restart game");
    tap(&mut a, Button::A, 80);
    assert_eq!(tap(&mut a, Button::A, 90), vec![Effect::ResetGame, Effect::PauseGame(false), Effect::Redraw(Some(Transition::Pop))]);
}

#[test]
fn how_the_last_run_ended_is_told_once_in_one_notice() {
    let battery = "The battery ran out last time, so the device turned itself off. Charge it to keep playing.";
    assert_eq!(app().handle(Msg::LastShutdown(LastShutdown::BatteryEmpty)), vec![Effect::Notice(battery.into())]);
    let unexpected = "The device didn't turn off properly last time: its power button was held down, the power was cut, the system crashed, or its SD card was taken out.";
    let record = crate::launch::LaunchRecord {
        core: "mGBA".into(), library: "/cores/mgba_libretro.so".into(), system: "gba".into(), game: "gba/Golden Sun.gba".into(), started: 0, reason: None,
    };
    let both = format!("{unexpected} Golden Sun was being played: progress since its last autosave was lost.");
    // The game's record is read with the card, usually after the device's.
    let mut a = app();
    assert_eq!(a.handle(Msg::LastShutdown(LastShutdown::Unexpected)), vec![Effect::Notice(unexpected.into())]);
    assert_eq!(a.handle(Msg::LastGameStopped(record.clone())), vec![Effect::Notice(both.clone())]);
    let mut a = app();
    a.handle(Msg::LastGameStopped(record));
    assert_eq!(a.handle(Msg::LastShutdown(LastShutdown::Unexpected)), vec![Effect::Notice(both)]);
    assert_eq!(LastShutdown::parse("battery-empty\n"), Some(LastShutdown::BatteryEmpty));
    assert_eq!(LastShutdown::parse("overheated"), None);
}
