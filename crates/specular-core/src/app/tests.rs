use std::time::Duration;

use serde_json::json;

use super::*;
use crate::library::builtin_systems;

fn game(system: &'static str, title: &str) -> Game {
    Game { system, path: format!("{system}/{title}.rom").into(), title: title.into(), sort_key: title.to_lowercase() }
}

fn app() -> App {
    let mut app = App::new(builtin_systems(), crate::settings::builtin::settings());
    app.handle(Msg::Library { games: vec![game("gba", "Golden Sun"), game("gba", "Mother 3"), game("snes", "Super Metroid")], card_found: true });
    app
}

/// Presses and releases a button at time `t` (ms); returns the effects of both.
fn tap(app: &mut App, button: Button, t: u64) -> Vec<Effect> {
    let mut e = event(app, button, true, t);
    e.extend(event(app, button, false, t));
    e
}

/// A press (`pressed`) or release of `button` at time `t` (ms).
fn event(app: &mut App, button: Button, pressed: bool, t: u64) -> Vec<Effect> {
    app.handle(Msg::Button(ButtonEvent { button, pressed, time: Duration::from_millis(t) }))
}

fn focused(app: &App) -> String { app.view().rows[app.focus()].label.clone() }

#[test]
fn home_lists_systems_with_games() {
    let v = app().view();
    let labels: Vec<&str> = v.rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["Systems", "Game Boy Advance", "Super Nintendo", "Device", "Settings"]);
    assert_eq!(app().focus(), 1); // the header is skipped
}

#[test]
fn open_a_system_play_and_come_back() {
    let mut a = app();
    assert_eq!(tap(&mut a, Button::A, 0), vec![Effect::Redraw(Some(Transition::Push))]);
    assert_eq!(a.view().title, "Game Boy Advance");
    assert_eq!(a.parent_title().as_deref(), Some("Library"));
    tap(&mut a, Button::Down, 10);
    assert_eq!(focused(&a), "Mother 3");
    assert_eq!(tap(&mut a, Button::A, 20), vec![Effect::Launch(game("gba", "Mother 3")), Effect::Redraw(None)]);
    assert_eq!(a.input_target(), InputTarget::Game);
    a.handle(Msg::GameEnded(None));
    assert_eq!(a.input_target(), InputTarget::App);
    assert_eq!(tap(&mut a, Button::B, 30), vec![Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!(focused(&a), "Game Boy Advance"); // focus remembered
    assert_eq!(tap(&mut a, Button::B, 40), vec![]); // home stays
}

#[test]
fn holding_down_repeats_and_stops_at_the_end() {
    let mut a = app();
    a.handle(Msg::Button(ButtonEvent { button: Button::Down, pressed: true, time: Duration::ZERO }));
    assert_eq!(focused(&a), "Super Nintendo");
    for t in (0..2000).step_by(10) { a.handle(Msg::Tick(Duration::from_millis(t))); }
    assert_eq!(focused(&a), "Settings"); // held: no wrap back to the top
    assert!(a.next_deadline().is_some());
    a.handle(Msg::Button(ButtonEvent { button: Button::Down, pressed: false, time: Duration::from_millis(2000) }));
    assert_eq!(a.next_deadline(), None);
    tap(&mut a, Button::Down, 2100); // a fresh press wraps
    assert_eq!(focused(&a), "Game Boy Advance");
}

#[test]
fn settings_change_and_save() {
    let mut a = app();
    tap(&mut a, Button::Up, 0); // wraps to Settings
    tap(&mut a, Button::A, 10);
    assert_eq!(a.view().title, "Settings");
    assert_eq!(focused(&a), "Brightness");
    let e = tap(&mut a, Button::Right, 20);
    assert_eq!(e, vec![Effect::Redraw(None), Effect::SaveSettings(Layer::Global), Effect::Apply { key: "display.brightness", value: json!(7) }]);
    // loading settings applies every global one
    let loaded = a.handle(Msg::SettingsLoaded(Layer::Global, serde_json::Map::new()));
    assert!(loaded.contains(&Effect::Apply { key: "lights.brightness", value: json!(3) }));
    assert_eq!(a.ctx().settings.layer(&Layer::Global).get("display.brightness"), Some(&json!(7)));
}

#[test]
fn developer_details_show_developer_settings() {
    let mut a = app();
    let labels = |a: &App| a.view().rows.iter().map(|r| r.label.clone()).collect::<Vec<_>>();
    tap(&mut a, Button::Up, 0);
    tap(&mut a, Button::A, 10);
    assert!(!labels(&a).contains(&"Frame statistics".to_string()));
    while focused(&a) != "Developer details" { tap(&mut a, Button::Down, 20); }
    tap(&mut a, Button::A, 30);
    assert!(labels(&a).contains(&"Frame statistics".to_string()));
}

#[test]
fn menu_goes_home_and_power_sleeps() {
    let mut a = app();
    tap(&mut a, Button::A, 0);
    assert_eq!(tap(&mut a, Button::Menu, 10), vec![Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!(a.view().title, "Library");
    // A tap sleeps on its release, if that comes before the hold is due.
    assert_eq!(event(&mut a, Button::Power, true, 20), vec![]);
    assert_eq!(a.next_deadline(), Some(Duration::from_millis(1020)));
    assert_eq!(event(&mut a, Button::Power, false, 900), vec![Effect::Sleep]);
    assert_eq!(a.next_deadline(), None);
}

#[test]
fn holding_power_opens_the_power_menu() {
    let mut a = app();
    event(&mut a, Button::Power, true, 0);
    assert_eq!(a.handle(Msg::Tick(Duration::from_millis(999))), vec![]);
    assert_eq!(a.handle(Msg::Tick(Duration::from_millis(1000))), vec![Effect::Redraw(Some(Transition::Push))]);
    assert_eq!(a.view().title, "Power");
    assert_eq!(event(&mut a, Button::Power, false, 1500), vec![]); // the hold's release doesn't sleep
    assert_eq!(tap(&mut a, Button::B, 1600), vec![Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!(a.view().title, "Library");
    event(&mut a, Button::Power, true, 2000);
    a.handle(Msg::Tick(Duration::from_millis(3000)));
    assert_eq!(tap(&mut a, Button::A, 3100), vec![Effect::Notice("Turning off…".into()), Effect::PowerOff]);
}

#[test]
fn holding_power_in_a_game_pauses_it_and_restarting_quits_it() {
    let mut a = app();
    tap(&mut a, Button::A, 0);
    tap(&mut a, Button::A, 10); // playing
    event(&mut a, Button::Power, true, 20);
    assert_eq!(a.handle(Msg::Tick(Duration::from_millis(1020))),
               vec![Effect::PauseGame(true), Effect::Redraw(Some(Transition::Push)), Effect::Redraw(Some(Transition::Push))]);
    assert_eq!(a.input_target(), InputTarget::App);
    // Cancelling leaves the game menu open, the game paused.
    assert_eq!(tap(&mut a, Button::B, 1100), vec![Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!(a.view().title, "Paused");
    event(&mut a, Button::Power, true, 1200);
    a.handle(Msg::Tick(Duration::from_millis(2200)));
    tap(&mut a, Button::Down, 2300);
    assert_eq!(tap(&mut a, Button::A, 2400), vec![Effect::QuitGame, Effect::Notice("Restarting…".into()), Effect::Restart]);
    assert_eq!(a.playing(), None);
}

#[test]
fn the_press_that_woke_the_device_is_neither_a_tap_nor_a_hold() {
    let mut a = app();
    event(&mut a, Button::Power, true, 0);
    a.handle(Msg::Woke);
    assert_eq!(a.next_deadline(), None);
    assert_eq!(event(&mut a, Button::Power, false, 100), vec![]);
}

/// Not a test: how long a focus move takes with 3,000 games (run with --ignored --nocapture).
#[test]
#[ignore]
fn time_focus_moves_in_a_long_list() {
    let mut a = App::new(builtin_systems(), crate::settings::builtin::settings());
    a.handle(Msg::Library { games: (0..3000).map(|i| game("snes", &format!("Game {i:04}"))).collect(), card_found: true });
    tap(&mut a, Button::A, 0);
    let t = std::time::Instant::now();
    for i in 0..1000 {
        tap(&mut a, Button::Down, 10 + i);
        let (v, f) = (a.view(), a.focus());
        std::hint::black_box((crate::list::layout(&v.rows, f, 1024.0, 512.0, false), a.parent_title()));
    }
    println!("per move (press, view, focus, layout, breadcrumb; not the UI's drawing): {:?}", t.elapsed() / 1000);
}

#[test]
fn y_switches_games_to_a_grid_where_left_and_right_move() {
    let mut a = App::new(builtin_systems(), crate::settings::builtin::settings());
    a.handle(Msg::Library { games: (0..7).map(|i| game("gba", &format!("Game {i}"))).collect(), card_found: true });
    a.set_grid_columns(3);
    tap(&mut a, Button::A, 0);
    assert_eq!(a.view().layout, Layout::ListWithPicture);
    assert_eq!(tap(&mut a, Button::Y, 10), vec![Effect::Redraw(None), Effect::SaveSettings(Layer::Global)]);
    assert_eq!(a.view().layout, Layout::Grid);
    tap(&mut a, Button::Right, 20);
    assert_eq!(focused(&a), "Game 1");
    tap(&mut a, Button::Down, 30);
    assert_eq!(focused(&a), "Game 4");
    tap(&mut a, Button::Down, 40); // a row down from 4 is past the end: stops at the last game
    assert_eq!(focused(&a), "Game 6");
    tap(&mut a, Button::Right, 50); // one cell past the end wraps
    assert_eq!(focused(&a), "Game 0");
    tap(&mut a, Button::Y, 60);
    assert_eq!(a.view().layout, Layout::ListWithPicture);
    assert_eq!(focused(&a), "Game 0");
}

#[test]
fn the_game_menu_pauses_and_quits() {
    let mut a = app();
    tap(&mut a, Button::A, 0);
    tap(&mut a, Button::A, 10);
    assert_eq!(a.playing().map(|g| g.title.as_str()), Some("Golden Sun"));
    assert_eq!(tap(&mut a, Button::Down, 20), vec![]); // goes to the game
    assert_eq!(tap(&mut a, Button::Menu, 30), vec![Effect::PauseGame(true), Effect::Redraw(Some(Transition::Push))]);
    assert_eq!((a.input_target(), a.view().title.as_str()), (InputTarget::App, "Paused"));
    assert_eq!(tap(&mut a, Button::B, 40), vec![Effect::PauseGame(false), Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!(a.input_target(), InputTarget::Game);
    tap(&mut a, Button::Menu, 42);
    assert_eq!(tap(&mut a, Button::Menu, 44), vec![Effect::PauseGame(false), Effect::Redraw(Some(Transition::Pop))]);
    tap(&mut a, Button::Menu, 50);
    tap(&mut a, Button::Down, 60);
    assert_eq!(tap(&mut a, Button::A, 70), vec![Effect::SaveState, Effect::PauseGame(false), Effect::Redraw(Some(Transition::Pop))]);
    // Sleeping and waking reopens the menu over the paused game.
    assert_eq!(a.handle(Msg::Woke), vec![Effect::PauseGame(true), Effect::Redraw(Some(Transition::Push))]);
    for (i, b) in [Button::Up, Button::A].into_iter().enumerate() { tap(&mut a, b, 80 + i as u64 * 10); } // wraps to "Quit game"
    assert_eq!(a.playing(), None);
    assert_eq!(a.view().title, "Game Boy Advance");
}

#[test]
fn volume_keys_work_in_games_and_stop_at_the_ends() {
    let mut a = app();
    let level = |e: &[Effect]| e.iter().find_map(|e| match e { Effect::ShowVolume { level, .. } => Some(*level), _ => None });
    assert_eq!(level(&tap(&mut a, Button::VolUp, 0)), Some(13));
    tap(&mut a, Button::A, 10);
    tap(&mut a, Button::A, 20); // playing
    assert_eq!(level(&tap(&mut a, Button::VolDown, 30)), Some(12));
    for t in 0..30 { tap(&mut a, Button::VolDown, 40 + t); }
    assert_eq!(tap(&mut a, Button::VolDown, 100), vec![Effect::ShowVolume { level: 0, muted: false }]); // nothing changed to save
}

#[test]
fn the_mute_switch_mutes_without_touching_the_volume() {
    let mut a = app();
    let switch = |a: &mut App, on: bool, t: u64| a.handle(Msg::Button(ButtonEvent { button: Button::Mute, pressed: on, time: Duration::from_millis(t) }));
    assert_eq!(switch(&mut a, true, 0), vec![Effect::Mute(true), Effect::ShowVolume { level: 12, muted: true }]);
    assert_eq!(switch(&mut a, true, 10), vec![]); // already on
    // The volume keys still work, and say the sound is muted.
    assert!(tap(&mut a, Button::VolUp, 20).contains(&Effect::ShowVolume { level: 13, muted: true }));
    assert_eq!(switch(&mut a, false, 30), vec![Effect::Mute(false), Effect::ShowVolume { level: 13, muted: false }]);
}

#[test]
fn sleeps_after_30_seconds_without_a_press_outside_a_game() {
    let mut a = app(); // the default is 30 seconds
    let sec = |n: u64| Duration::from_secs(n);
    assert_eq!(a.sleep_deadline(), None); // the clock hasn't been read yet
    tap(&mut a, Button::Down, 0);
    assert_eq!(a.sleep_deadline(), Some(sec(30)));
    assert_eq!(a.handle(Msg::Tick(sec(29))), vec![]);
    tap(&mut a, Button::Down, 29_000);
    assert_eq!(a.handle(Msg::Tick(sec(58))), vec![]);
    assert_eq!(a.handle(Msg::Tick(sec(59))), vec![Effect::Sleep]);
    a.handle(Msg::Woke);
    assert_eq!(a.sleep_deadline(), None);
    assert_eq!(a.handle(Msg::Tick(sec(60))), vec![]); // counting starts again
    assert_eq!(a.handle(Msg::Tick(sec(90))), vec![Effect::Sleep]);
}

#[test]
fn a_running_game_is_not_idle_but_its_menu_is() {
    let mut a = app();
    let sec = |n: u64| Duration::from_secs(n);
    tap(&mut a, Button::A, 0);
    tap(&mut a, Button::A, 10); // playing
    assert_eq!(a.sleep_deadline(), None);
    assert_eq!(a.handle(Msg::Tick(sec(600))), vec![]);
    tap(&mut a, Button::Menu, 600_000); // the game menu pauses
    assert_eq!(a.sleep_deadline(), Some(sec(630)));
    assert!(a.handle(Msg::Tick(sec(630))).contains(&Effect::Sleep));
    a.handle(Msg::Woke);
    a.handle(Msg::Tick(sec(640)));
    tap(&mut a, Button::B, 641_000); // resume
    assert_eq!(a.handle(Msg::Tick(sec(1000))), vec![]);
    // A game that ends by itself starts the count from the next reading of the clock.
    a.handle(Msg::GameEnded(None));
    assert_eq!(a.sleep_deadline(), None);
    assert_eq!(a.handle(Msg::Tick(sec(1001))), vec![]);
    assert_eq!(a.sleep_deadline(), Some(sec(1031)));
}

#[test]
fn home_says_where_games_go_when_there_are_none() {
    let mut a = App::new(builtin_systems(), crate::settings::builtin::settings());
    let labels = |a: &App| a.view().rows.iter().map(|r| r.label.clone()).collect::<Vec<_>>();
    assert!(labels(&a).contains(&"Looking for games…".to_string()));
    a.handle(Msg::Library { games: vec![], card_found: true });
    assert!(labels(&a).contains(&"No games found".to_string()));
    assert_eq!(a.view().rows[a.focus()].label, "No games found");
    assert_eq!(tap(&mut a, Button::A, 0), vec![]); // a note does nothing
    a.handle(Msg::Library { games: vec![], card_found: false });
    assert!(labels(&a).contains(&"No SD card".to_string()));
}
