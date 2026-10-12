use std::time::Duration;

use serde_json::json;

use super::*;
use crate::library::builtin_systems;

fn game(system: &'static str, title: &str) -> Game {
    Game { system, path: format!("{system}/{title}.rom").into(), title: title.into(), sort_key: title.to_lowercase() }
}

fn app() -> App {
    let mut app = App::new(builtin_systems(), crate::settings::builtin::settings());
    app.handle(Msg::Library(vec![game("gba", "Golden Sun"), game("gba", "Mother 3"), game("snes", "Super Metroid")]));
    app
}

/// Presses and releases a button at time `t` (ms).
fn tap(app: &mut App, button: Button, t: u64) -> Vec<Effect> {
    let time = Duration::from_millis(t);
    let e = app.handle(Msg::Button(ButtonEvent { button, pressed: true, time }));
    app.handle(Msg::Button(ButtonEvent { button, pressed: false, time }));
    e
}

fn focused(app: &App) -> String { let v = app.view(); v.rows[v.focus].label.clone() }

#[test]
fn home_lists_systems_with_games() {
    let v = app().view();
    let labels: Vec<&str> = v.rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["Systems", "Game Boy Advance", "Super Nintendo", "Device", "Settings"]);
    assert_eq!(v.focus, 1); // the header is skipped
}

#[test]
fn open_a_system_play_and_come_back() {
    let mut a = app();
    assert_eq!(tap(&mut a, Button::A, 0), vec![Effect::Redraw(Some(Transition::Push))]);
    assert_eq!(a.view().title, "Game Boy Advance");
    tap(&mut a, Button::Down, 10);
    assert_eq!(focused(&a), "Mother 3");
    assert_eq!(tap(&mut a, Button::A, 20), vec![Effect::Launch(game("gba", "Mother 3"))]);
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
    assert_eq!(a.ctx.settings.layer(&Layer::Global).get("display.brightness"), Some(&json!(7)));
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
    assert_eq!(tap(&mut a, Button::Power, 20), vec![Effect::Sleep]);
}
