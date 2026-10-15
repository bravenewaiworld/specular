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
    assert_eq!(tap(&mut a, Button::VolDown, 100), vec![Effect::ShowVolume { level: 0, muted: false, bluetooth: false }]); // nothing changed to save
}

#[test]
fn the_mute_switch_mutes_without_touching_the_volume() {
    let mut a = app();
    let switch = |a: &mut App, on: bool, t: u64| a.handle(Msg::Button(ButtonEvent { button: Button::Mute, pressed: on, time: Duration::from_millis(t) }));
    assert_eq!(switch(&mut a, true, 0), vec![Effect::Mute(true), Effect::ShowVolume { level: 12, muted: true, bluetooth: false }]);
    assert_eq!(switch(&mut a, true, 10), vec![]); // already on
    // The volume keys still work, and say the sound is muted.
    assert!(tap(&mut a, Button::VolUp, 20).contains(&Effect::ShowVolume { level: 13, muted: true, bluetooth: false }));
    assert_eq!(switch(&mut a, false, 30), vec![Effect::Mute(false), Effect::ShowVolume { level: 13, muted: false, bluetooth: false }]);
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
    assert_eq!(a.view().rows[a.focus()].label, "Settings"); // notes are never focused
    a.handle(Msg::Library { games: vec![], card_found: false });
    assert!(labels(&a).contains(&"No SD card".to_string()));
}

mod wireless {
    use super::*;
    use crate::app::wireless::{NEED_TIMEOUT, WIFI_GRACE};
    use crate::device::Output;
    use crate::wireless::Secret;
    use crate::wireless::bluetooth::{BtAddress, BtCmd, BtDevice, BtEvent, BtKind};
    use crate::wireless::wifi::{Network, WifiCmd, WifiEvent, WifiLink};

    /// An app with both radios and its settings loaded (WiFi on Auto, Bluetooth off).
    fn radio_app() -> App {
        let mut a = app();
        a.has_radios(true, true);
        assert_eq!(a.handle(Msg::Tick(Duration::ZERO)), vec![]); // nothing before the settings
        let e = a.handle(Msg::SettingsLoaded(Layer::Global, Default::default()));
        assert!(e.ends_with(&[Effect::Wifi(WifiCmd::Power(false)), Effect::Bluetooth(BtCmd::Power(false))]), "{e:?}");
        a
    }

    fn radio_effects(e: &[Effect]) -> Vec<&Effect> {
        e.iter().filter(|e| matches!(e, Effect::Wifi(_) | Effect::Bluetooth(_) | Effect::SoundTo(_))).collect()
    }

    /// Opens Settings, then the row labelled `label` there.
    fn open(a: &mut App, label: &str, t: u64) {
        assert_eq!(a.view().title, "Library");
        tap(a, Button::Up, t); // home wraps to its last row, Settings
        tap(a, Button::A, t);
        focus_on(a, label);
        tap(a, Button::A, t);
    }

    /// Moves the focus down to the row labelled `label`.
    fn focus_on(a: &mut App, label: &str) {
        for _ in 0..a.view().rows.len() {
            if focused(a) == label { return; }
            tap(a, Button::Down, 0);
        }
        panic!("no row {label}");
    }

    fn net(ssid: &str, secured: bool, saved: bool) -> Network { Network { ssid: ssid.into(), bars: 3, secured, saved, in_range: true, supported: true } }

    fn connected(ssid: &str) -> Msg { Msg::Wifi(WifiEvent::Link(WifiLink::Connected { ssid: ssid.into(), address: "10.0.0.2".into(), bars: None })) }

    #[test]
    fn the_status_bar_shows_the_battery_and_the_radios_that_are_on() {
        use crate::device::Battery;
        let mut a = radio_app();
        assert_eq!(a.status(), []);
        let battery = Battery { percent: Some(78), charging: false, plugged: false };
        a.handle(Msg::Battery(battery));
        assert_eq!(a.status(), [Icon::Battery(battery)]);
        let wifi = |bars| Icon::Wifi { bars, locked: false };
        // WiFi on but not connected: empty bars; connected: the live signal, unknown at first.
        a.handle(Msg::Wifi(WifiEvent::Link(WifiLink::Idle)));
        assert_eq!(a.status()[0], wifi(Some(0)));
        a.handle(connected("Home"));
        assert_eq!(a.status()[0], wifi(None));
        a.handle(Msg::Wifi(WifiEvent::Link(WifiLink::Connected { ssid: "Home".into(), address: "10.0.0.2".into(), bars: Some(3) })));
        assert_eq!(a.status()[0], wifi(Some(3)));
        // Bluetooth shows once powered, lit with a device connected.
        let device = BtDevice { address: BtAddress("00:11:22:33:44:55".into()), name: "Buds".into(), kind: BtKind::Headphones, paired: true, connected: false, audio: false };
        a.handle(Msg::Bluetooth(BtEvent::Powered(true)));
        assert_eq!(a.status()[1], Icon::Bluetooth { kind: BtKind::Other, lit: false });
        a.handle(Msg::Bluetooth(BtEvent::Devices(vec![BtDevice { connected: true, ..device }])));
        assert_eq!(a.status()[1], Icon::Bluetooth { kind: BtKind::Other, lit: true });
        a.handle(Msg::Wifi(WifiEvent::Link(WifiLink::Off)));
        a.handle(Msg::Bluetooth(BtEvent::Powered(false)));
        assert_eq!(a.status(), [Icon::Battery(battery)]);
    }

    #[test]
    fn rows_carry_icons_for_signal_security_and_device_type() {
        let mut a = radio_app();
        open(&mut a, "Wi-Fi", 0);
        let strong = Network { bars: 4, ..net("Home", true, true) };
        a.handle(Msg::Wifi(WifiEvent::Networks(vec![strong, Network { bars: 1, ..net("Cafe", false, false) }])));
        let icon = |a: &App, label: &str| a.view().rows.iter().find(|r| r.label == label).unwrap().icon;
        assert_eq!(icon(&a, "Home"), Some(Icon::Wifi { bars: Some(4), locked: true }));
        assert_eq!(icon(&a, "Cafe"), Some(Icon::Wifi { bars: Some(1), locked: false }));
        // The network joined shows the live signal, not the scan's.
        a.handle(Msg::Wifi(WifiEvent::Link(WifiLink::Connected { ssid: "Cafe".into(), address: "10.0.0.2".into(), bars: Some(3) })));
        assert_eq!(icon(&a, "Cafe"), Some(Icon::Wifi { bars: Some(3), locked: false }));
        tap(&mut a, Button::B, 0);
        focus_on(&mut a, "Bluetooth");
        tap(&mut a, Button::A, 0);
        tap(&mut a, Button::A, 0); // switches it on
        let address = BtAddress("00:11:22:33:44:55".into());
        let device = BtDevice { address, name: "Pad".into(), kind: BtKind::Controller, paired: true, connected: false, audio: false };
        a.handle(Msg::Bluetooth(BtEvent::Powered(true)));
        a.handle(Msg::Bluetooth(BtEvent::Devices(vec![device.clone()])));
        assert_eq!(icon(&a, "Pad"), Some(Icon::Bluetooth { kind: BtKind::Controller, lit: false }));
        a.handle(Msg::Bluetooth(BtEvent::Devices(vec![BtDevice { connected: true, ..device }])));
        assert_eq!(icon(&a, "Pad"), Some(Icon::Bluetooth { kind: BtKind::Controller, lit: true }));
    }

    #[test]
    fn the_icons_screen_is_a_developer_setting() {
        let mut a = app();
        tap(&mut a, Button::Up, 0);
        tap(&mut a, Button::A, 0);
        assert!(!a.view().rows.iter().any(|r| r.label == "Icons"));
        focus_on(&mut a, "Developer details");
        tap(&mut a, Button::A, 0);
        focus_on(&mut a, "Icons");
        tap(&mut a, Button::A, 0);
        assert_eq!(a.view().title, "Icons");
        assert!(a.view().rows.iter().filter(|r| r.icon.is_some()).count() > 20);
    }

    #[test]
    fn settings_link_to_the_radios_only_when_the_device_has_them() {
        let mut a = app();
        tap(&mut a, Button::Up, 0);
        tap(&mut a, Button::A, 0);
        assert!(!a.view().rows.iter().any(|r| r.label == "Wi-Fi"));
        let mut a = radio_app();
        open(&mut a, "Wi-Fi", 0);
        assert_eq!(a.view().title, "Wi-Fi");
    }

    #[test]
    fn auto_wifi_is_on_while_its_screen_is_open_then_for_the_grace() {
        let mut a = radio_app();
        tap(&mut a, Button::Up, 1000);
        tap(&mut a, Button::A, 1000);
        focus_on(&mut a, "Wi-Fi");
        let e = tap(&mut a, Button::A, 1000);
        assert_eq!(radio_effects(&e), [&Effect::Wifi(WifiCmd::Power(true)), &Effect::Wifi(WifiCmd::Scan)]);
        let e = tap(&mut a, Button::B, 5000);
        assert_eq!(radio_effects(&e), Vec::<&Effect>::new()); // the grace
        let off = Duration::from_millis(5000) + WIFI_GRACE;
        assert_eq!(a.wifi_deadline(), Some(off));
        assert_eq!(radio_effects(&a.handle(Msg::Tick(off - Duration::from_millis(1)))), Vec::<&Effect>::new());
        assert_eq!(radio_effects(&a.handle(Msg::Tick(off))), [&Effect::Wifi(WifiCmd::Power(false))]);
        assert_eq!(a.wifi_deadline(), None);
    }

    #[test]
    fn a_need_lapses_unless_wifi_connects() {
        let mut a = radio_app();
        let need = |a: &mut App, wanted, ms| a.handle(Msg::WifiNeed { need: WifiNeed::BoxArt, wanted, time: Duration::from_millis(ms) });
        assert_eq!(radio_effects(&need(&mut a, true, 0)), [&Effect::Wifi(WifiCmd::Power(true))]);
        assert_eq!(a.wifi_deadline(), Some(NEED_TIMEOUT));
        // Never connected: off after the timeout and the grace.
        a.handle(Msg::Tick(NEED_TIMEOUT));
        assert_eq!(radio_effects(&a.handle(Msg::Tick(NEED_TIMEOUT + WIFI_GRACE))), [&Effect::Wifi(WifiCmd::Power(false))]);
        // Connected in time: it holds until dropped.
        let t = (NEED_TIMEOUT + WIFI_GRACE).as_millis() as u64;
        need(&mut a, true, t);
        a.handle(connected("Home"));
        assert_eq!(radio_effects(&a.handle(Msg::Tick(Duration::from_millis(t) + NEED_TIMEOUT * 10))), Vec::<&Effect>::new());
        need(&mut a, false, t + 1);
        assert!(a.wifi_deadline().is_some());
    }

    #[test]
    fn joining_a_secured_network_asks_for_its_password() {
        let mut a = radio_app();
        open(&mut a, "Wi-Fi", 0);
        a.handle(Msg::Wifi(WifiEvent::Link(WifiLink::Idle)));
        a.handle(Msg::Wifi(WifiEvent::Networks(vec![net("Office", true, false), net("Cafe", false, false)])));
        focus_on(&mut a, "Office");
        tap(&mut a, Button::A, 0);
        assert_eq!(a.view().title, "Password for Office");
        assert_eq!(a.view().layout, Layout::Keys);
        for _ in 0..3 { tap(&mut a, Button::A, 0); } // "qqq": too short
        assert_eq!(tap(&mut a, Button::Start, 0), vec![Effect::Notice("A Wi-Fi password has 8 to 63 characters".into())]);
        assert_eq!(a.view().entry.as_ref().unwrap().text, "•••");
        for _ in 0..5 { tap(&mut a, Button::A, 0); }
        let e = tap(&mut a, Button::Start, 0);
        assert_eq!(e[0], Effect::Wifi(WifiCmd::Join { ssid: "Office".into(), password: Some(Secret::new("qqqqqqqq")) }));
        assert_eq!(a.view().title, "Wi-Fi");
        assert_eq!(focused(&a), "Office");
        // An open network joins at once.
        focus_on(&mut a, "Cafe");
        assert_eq!(radio_effects(&tap(&mut a, Button::A, 0)), [&Effect::Wifi(WifiCmd::Join { ssid: "Cafe".into(), password: None })]);
    }

    #[test]
    fn a_refused_saved_password_is_asked_again_and_unsupported_networks_say_so() {
        let mut a = radio_app();
        open(&mut a, "Wi-Fi", 0);
        let wpa3 = Network { supported: false, ..net("Lab", true, false) };
        a.handle(Msg::Wifi(WifiEvent::Networks(vec![net("Home", true, true), wpa3])));
        focus_on(&mut a, "Lab");
        assert_eq!(a.view().rows[a.focus()].kind, RowKind::Text { detail: Some("Not supported".into()) });
        assert_eq!(tap(&mut a, Button::A, 0), vec![Effect::Notice("Lab uses security this device doesn't support".into())]);
        focus_on(&mut a, "Home");
        assert_eq!(radio_effects(&tap(&mut a, Button::A, 0)), [&Effect::Wifi(WifiCmd::Join { ssid: "Home".into(), password: None })]);
        a.handle(Msg::Wifi(WifiEvent::Failed(crate::wireless::wifi::WifiError::WrongPassword { ssid: "Home".into() })));
        tap(&mut a, Button::A, 0);
        assert_eq!(a.view().title, "Password for Home");
    }

    #[test]
    fn b_on_an_empty_keyboard_goes_back() {
        let mut a = radio_app();
        open(&mut a, "Wi-Fi", 0);
        a.handle(Msg::Wifi(WifiEvent::Networks(vec![net("Office", true, false)])));
        focus_on(&mut a, "Office");
        tap(&mut a, Button::A, 0);
        tap(&mut a, Button::A, 0);
        tap(&mut a, Button::B, 0); // deletes the letter
        assert_eq!(a.view().layout, Layout::Keys);
        tap(&mut a, Button::B, 0);
        assert_eq!(a.view().title, "Wi-Fi");
    }

    #[test]
    fn forgetting_a_network_is_confirmed() {
        let mut a = radio_app();
        open(&mut a, "Wi-Fi", 0);
        a.handle(Msg::Wifi(WifiEvent::Networks(vec![net("Home", true, true)])));
        focus_on(&mut a, "Home");
        tap(&mut a, Button::X, 0);
        assert_eq!(a.view().title, "Forget Home?");
        assert_eq!(focused(&a), "Cancel");
        tap(&mut a, Button::Down, 0);
        let e = tap(&mut a, Button::A, 0);
        assert_eq!(e[0], Effect::Wifi(WifiCmd::Forget { ssid: "Home".into() }));
        assert_eq!(a.view().title, "Wi-Fi");
    }

    #[test]
    fn bluetooth_searches_on_its_screen_and_routes_sound_to_a_headset() {
        let mut a = radio_app();
        open(&mut a, "Bluetooth", 0);
        assert_eq!(focused(&a), "Bluetooth");
        let e = tap(&mut a, Button::A, 0);
        assert_eq!(radio_effects(&e), [&Effect::Bluetooth(BtCmd::Power(true)), &Effect::Bluetooth(BtCmd::Search(true))]);
        a.handle(Msg::Bluetooth(BtEvent::Powered(true)));
        let address = BtAddress("00:11:22:33:44:55".into());
        let headset = BtDevice { address: address.clone(), name: "Buds".into(), kind: BtKind::Headphones, paired: false, connected: false, audio: false };
        a.handle(Msg::Bluetooth(BtEvent::Devices(vec![headset.clone()])));
        tap(&mut a, Button::Down, 0);
        assert_eq!(focused(&a), "Buds");
        assert_eq!(radio_effects(&tap(&mut a, Button::A, 0)), [&Effect::Bluetooth(BtCmd::Pair(address.clone()))]);
        assert_eq!(a.view().rows.iter().find(|r| r.label == "Buds").unwrap().kind, RowKind::Text { detail: Some("Pairing…".into()) });
        let connected = BtDevice { paired: true, connected: true, audio: true, ..headset };
        let e = a.handle(Msg::Bluetooth(BtEvent::Devices(vec![connected.clone()])));
        assert_eq!(radio_effects(&e), [&Effect::SoundTo(Output::Bluetooth(address.clone()))]);
        assert_eq!(a.view().rows.iter().find(|r| r.label == "Buds").unwrap().kind, RowKind::Text { detail: Some("Connected · sound".into()) });
        // The volume keys now set the headset's volume.
        let e = tap(&mut a, Button::VolUp, 0);
        assert!(e.contains(&Effect::Apply { key: builtin::BLUETOOTH_VOLUME, value: json!(11) }), "{e:?}");
        assert!(e.contains(&Effect::ShowVolume { level: 11, muted: false, bluetooth: true }));
        // A headset whose sound doesn't open is left until it reconnects.
        let e = a.handle(Msg::SoundFailed(Output::Bluetooth(address.clone())));
        assert_eq!(e[0], Effect::Notice("Couldn't play sound on Buds".into()));
        assert_eq!(radio_effects(&e), [&Effect::SoundTo(Output::Builtin)]);
        let gone = BtDevice { connected: false, audio: false, ..connected.clone() };
        assert_eq!(radio_effects(&a.handle(Msg::Bluetooth(BtEvent::Devices(vec![gone])))), Vec::<&Effect>::new());
        let e = a.handle(Msg::Bluetooth(BtEvent::Devices(vec![connected])));
        assert_eq!(radio_effects(&e), [&Effect::SoundTo(Output::Bluetooth(address))]);
        // Leaving stops the search; turning Bluetooth off brings the sound back.
        assert_eq!(radio_effects(&tap(&mut a, Button::B, 0)), [&Effect::Bluetooth(BtCmd::Search(false))]);
        assert_eq!(focused(&a), "Bluetooth"); // its link on the settings screen
        tap(&mut a, Button::A, 0);
        let e = tap(&mut a, Button::A, 0);
        assert_eq!(radio_effects(&e), [&Effect::Bluetooth(BtCmd::Search(false)), &Effect::Bluetooth(BtCmd::Power(false)), &Effect::SoundTo(Output::Builtin)]);
    }
}

mod install {
    use super::*;
    use crate::install::{Backup, InstallOffer, Internal, Offer, Progress, Task, TaskError};

    /// An app with an installer, at home.
    fn install_app() -> App {
        let mut a = app();
        a.has_installer(true);
        a
    }

    fn offer() -> Offer {
        Offer {
            install: Some(InstallOffer { bytes: 7_818_182_656, backup_possible: true, free: 20_000_000_000 }), why_not: None, internal: Internal::TrimUi,
            backups: vec![Backup { dir: "/mnt/SDCARD/backups/internal-20261014-232447".into(), created: "20261014-232447".into(), bytes: 7_818_182_656, holds: Some(Internal::TrimUi) }],
        }
    }

    /// Opens Settings (which asks the installer what it can do) and gives it `offer`.
    fn settings(a: &mut App, offer: Offer) {
        tap(a, Button::Up, 0); // home wraps to its last row, Settings
        let e = tap(a, Button::A, 0);
        assert!(e.contains(&Effect::Installer(InstallCmd::Offer)), "{e:?}");
        a.handle(Msg::InstallOffer(Ok(offer)));
    }

    /// Moves the focus to the row labelled `label` and presses A.
    fn press(a: &mut App, label: &str) -> Vec<Effect> {
        for _ in 0..a.view().rows.len() {
            if focused(a) == label { return tap(a, Button::A, 0); }
            tap(a, Button::Down, 0);
        }
        panic!("no row {label} in {:?}", a.view().rows.iter().map(|r| &r.label).collect::<Vec<_>>());
    }

    #[test]
    fn settings_offers_installing_and_restoring_only_when_the_installer_can() {
        let mut a = install_app();
        settings(&mut a, offer());
        let labels: Vec<String> = a.view().rows.iter().map(|r| r.label.clone()).collect();
        assert!(labels.ends_with(&["Internal storage".into(), "Install to internal storage".into(), "Restore internal storage".into()]), "{labels:?}");
        a.handle(Msg::InstallOffer(Ok(Offer { install: None, why_not: Some("running from the internal storage".into()), internal: Internal::Unknown, backups: vec![] })));
        assert!(!a.view().rows.iter().any(|r| r.label == "Internal storage"));
    }

    #[test]
    fn installing_asks_first_runs_without_interruption_then_restarts() {
        let mut a = install_app();
        settings(&mut a, offer());
        press(&mut a, "Install to internal storage");
        assert_eq!(a.view().title, "Install to internal storage?");
        assert_eq!(focused(&a), "Cancel"); // it erases: the focus starts on Cancel
        press(&mut a, "Back up the internal storage first"); // on by default (there's room): now off
        let e = press(&mut a, "Install");
        assert_eq!(e[0], Effect::Installer(InstallCmd::Start(Task::Install { backup: false, keep_card_bootable: false })));
        assert_eq!(a.view().title, "Installing…");
        // Nothing interrupts it: not B, Menu, the power button or sleep.
        for b in [Button::B, Button::Menu] { assert_eq!(tap(&mut a, b, 0), vec![]); }
        assert_eq!(tap(&mut a, Button::Power, 0), vec![Effect::Notice("Please wait until installing has finished".into())]);
        assert_eq!(a.sleep_deadline(), None);
        a.handle(Msg::InstallProgress(Progress::Step { index: 1, of: 5, label: "Checking".into() }));
        a.handle(Msg::InstallProgress(Progress::Step { index: 2, of: 5, label: "Partitioning".into() }));
        a.handle(Msg::InstallProgress(Progress::Fraction(0.5)));
        let details: Vec<_> = a.view().rows.iter().map(|r| match &r.kind { RowKind::Note { detail } => detail.clone(), _ => None }).collect();
        assert_eq!(details[..2], [Some("Done".into()), Some("50 %".into())]);
        let e = a.handle(Msg::InstallProgress(Progress::Done(Ok("Installed.".into()))));
        assert!(e.contains(&Effect::Installer(InstallCmd::Offer))); // what can be done has changed
        assert_eq!(a.view().title, "Installed");
        assert_eq!(focused(&a), "Restart"); // the steps and the message are notes: never focused
        assert_eq!(tap(&mut a, Button::A, 0), vec![Effect::Restart]);
    }

    #[test]
    fn a_failed_install_says_why_and_goes_back() {
        let mut a = install_app();
        settings(&mut a, offer());
        press(&mut a, "Install to internal storage");
        press(&mut a, "Install");
        let error = TaskError { message: "the battery is at 20 %".into(), hint: "Connect the charger.".into(), log: Some("/mnt/SDCARD/logs/install.log".into()) };
        a.handle(Msg::InstallProgress(Progress::Done(Err(error))));
        assert_eq!(a.view().title, "Install failed");
        let labels: Vec<String> = a.view().rows.iter().map(|r| r.label.clone()).collect();
        assert!(labels.contains(&"Connect the charger.".into()) && labels.contains(&"Log: /mnt/SDCARD/logs/install.log".into()), "{labels:?}");
        assert_eq!(tap(&mut a, Button::B, 0), vec![Effect::Redraw(Some(Transition::Pop))]);
        assert_eq!(a.view().title, "Install to internal storage?");
        assert!(!a.task_running());
    }

    #[test]
    fn restoring_lists_the_backups_and_asks_first() {
        let mut a = install_app();
        settings(&mut a, offer());
        press(&mut a, "Restore internal storage");
        press(&mut a, "TrimUI's system · 2026-10-14 23:24 UTC");
        assert_eq!(a.view().title, "Restore the internal storage?");
        let e = press(&mut a, "Restore");
        assert_eq!(e[0], Effect::Installer(InstallCmd::Start(Task::Restore { dir: "/mnt/SDCARD/backups/internal-20261014-232447".into() })));
        a.handle(Msg::InstallProgress(Progress::Done(Ok("Restored.".into()))));
        assert_eq!(press(&mut a, "Power off"), vec![Effect::PowerOff]);
    }

    #[test]
    fn a_task_left_running_by_an_earlier_run_is_shown_and_not_interrupted() {
        let mut a = install_app();
        // The installer, following it (Installer::follow), reports it from its start.
        a.handle(Msg::InstallProgress(Progress::Task { restore: true }));
        a.handle(Msg::InstallProgress(Progress::Step { index: 1, of: 3, label: "Checking the backup".into() }));
        assert_eq!(a.view().title, "Restoring…");
        assert!(a.task_running());
        assert_eq!(a.sleep_deadline(), None);
        assert_eq!(tap(&mut a, Button::Power, 0), vec![Effect::Notice("Please wait until restoring has finished".into())]);
        a.handle(Msg::InstallProgress(Progress::Done(Ok("Restored.".into()))));
        assert_eq!(focused(&a), "Power off");
        // A task this run started also reports which it is first: nothing changes.
        assert_eq!(a.handle(Msg::InstallProgress(Progress::Task { restore: false })), vec![]);
    }

    #[test]
    fn an_unfinished_install_is_pointed_out_and_not_backed_up_by_default() {
        let mut a = install_app();
        settings(&mut a, Offer { internal: Internal::UnfinishedInstall, ..offer() });
        assert!(a.view().rows.iter().any(|r| r.label.starts_with("The last install didn't finish")));
        press(&mut a, "Install to internal storage");
        let rows = a.view().rows.clone();
        assert!(rows.iter().any(|r| r.label == "Erases everything on the internal storage, including what the unfinished install left."));
        assert!(rows.iter().any(|r| r.label == "Back up the internal storage first" && r.kind == RowKind::Toggle { on: false }));
    }
}
