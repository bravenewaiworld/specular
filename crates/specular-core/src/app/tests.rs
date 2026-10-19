use std::time::Duration;

use serde_json::json;

use super::*;
use crate::library::builtin_systems;

fn game(system: &'static str, title: &str) -> Game {
    Game { system, path: format!("{system}/{title}.rom").into(), title: title.into(), sort_key: title.to_lowercase() }
}

/// Specular's settings and its cores' options, with interface sounds off by default, so that only
/// the sounds' own tests see them.
fn quiet() -> Vec<SettingDecl> {
    let mut decls = crate::settings::declarations(&builtin_systems());
    decls.iter_mut().find(|d| d.key == builtin::INTERFACE_SOUNDS).unwrap().default = json!(false);
    decls
}

fn app() -> App { app_with(quiet()) }

fn app_with(settings: Vec<SettingDecl>) -> App {
    let mut app = App::new(builtin_systems(), settings);
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

fn labels(app: &App) -> Vec<String> { app.view().rows.iter().map(|r| r.label.clone()).collect() }

/// Scaling applied (to the running game) with this value.
fn scaling(value: &str) -> Effect { Effect::Apply { key: builtin::SCALING, value: json!(value) } }

/// Moves the focus down to the row labelled `label`.
fn focus_on(a: &mut App, label: &str) {
    for _ in 0..100 { if focused(a) == label { return; } tap(a, Button::Down, 0); }
    panic!("no row {label} in {:?}", labels(a));
}

/// Types `text` on the on-screen keyboard: moves to each key in turn and presses it.
fn type_text(a: &mut App, text: &str) {
    for c in text.chars() {
        let mut moves = 0;
        while focused(a) != c.to_string() {
            // Right goes round a line (of at most 10 keys), then down to the next.
            tap(a, if moves % 11 == 10 { Button::Down } else { Button::Right }, 0);
            moves += 1;
            assert!(moves < 100, "no key {c}");
        }
        tap(a, Button::A, 0);
    }
}

#[test]
fn home_lists_systems_with_games() {
    let v = app().view();
    let labels: Vec<&str> = v.rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["Systems", "Game Boy Advance", "Super Nintendo", "Search", "Device", "Settings"]);
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
    assert_eq!(tap(&mut a, Button::A, 20), vec![Effect::Launch(game("gba", "Mother 3")), scaling("integer"), Effect::Redraw(None)]);
    assert_eq!(a.input_target(), InputTarget::Game);
    a.handle(Msg::GameEnded(None));
    assert_eq!(a.input_target(), InputTarget::App);
    assert_eq!(tap(&mut a, Button::B, 30), vec![Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!(focused(&a), "Game Boy Advance"); // focus remembered
    assert_eq!(tap(&mut a, Button::B, 40), vec![]); // home stays
}

#[test]
fn a_game_that_didnt_end_last_time_is_reported_at_start_up() {
    use crate::launch::LaunchRecord;
    let record = |reason: Option<&str>| LaunchRecord {
        core: "mGBA".into(), library: "/cores/mgba_libretro.so".into(), system: "gba".into(),
        game: "gba/Golden Sun (USA, Europe).zip".into(), started: 0, reason: reason.map(Into::into),
    };
    assert_eq!(app().handle(Msg::LastGameStopped(record(None))), vec![Effect::Notice("mGBA stopped unexpectedly while playing Golden Sun".into())]);
    assert_eq!(app().handle(Msg::LastGameStopped(record(Some("it stopped responding")))),
               vec![Effect::Notice("mGBA was stopped while playing Golden Sun: it stopped responding".into())]);
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
    // loading settings applies every global one, and saves the change made before (whose save
    // left out the file's values)
    let loaded = a.handle(Msg::SettingsLoaded(Layer::Global, serde_json::Map::from_iter([("sound.volume".into(), json!(5))])));
    assert!(loaded.contains(&Effect::Apply { key: "lights.brightness", value: json!(3) }));
    assert!(loaded.contains(&Effect::SaveSettings(Layer::Global)));
    let global = a.ctx().settings.layer(&Layer::Global);
    assert_eq!((global.get("display.brightness"), global.get("sound.volume")), (Some(&json!(7)), Some(&json!(5))));
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
    let mut a = App::new(builtin_systems(), quiet());
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
    let mut a = App::new(builtin_systems(), quiet());
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
fn search_finds_games_in_every_system_best_first_and_plays_them() {
    let mut a = App::new(builtin_systems(), quiet());
    a.handle(Msg::Library { games: vec![game("gba", "Metroid Fusion"), game("gba", "Mother 3"), game("snes", "Super Metroid")], card_found: true });
    tap(&mut a, Button::Down, 0);
    tap(&mut a, Button::Down, 0);
    assert_eq!(tap(&mut a, Button::A, 0), vec![Effect::Redraw(Some(Transition::Push))]);
    assert_eq!((a.view().title.as_str(), a.view().layout), ("Search", Layout::Keys));
    type_text(&mut a, "metroid");
    // The results take the keyboard's place.
    assert_eq!(tap(&mut a, Button::Start, 0), vec![Effect::Redraw(Some(Transition::Push))]);
    assert_eq!((a.view().title.as_str(), a.parent_title().as_deref()), ("“metroid”", Some("Library")));
    let results = |a: &App| a.view().rows.iter().map(|r| match &r.kind {
        RowKind::Image { detail: Some(d), .. } => format!("{} {d}", r.label),
        _ => r.label.clone(),
    }).collect::<Vec<_>>();
    assert_eq!(results(&a), ["Metroid Fusion GBA", "Super Metroid SNES"]);
    // A rescan keeps the results (the query is the screen's params) and the focus.
    tap(&mut a, Button::Down, 0);
    a.handle(Msg::Library { games: vec![game("gb", "Metroid II"), game("gba", "Metroid Fusion"), game("snes", "Super Metroid")], card_found: true });
    assert_eq!(results(&a), ["Metroid Fusion GBA", "Metroid II GB", "Super Metroid SNES"]);
    assert_eq!(focused(&a), "Super Metroid");
    assert_eq!(tap(&mut a, Button::A, 0), vec![Effect::Launch(game("snes", "Super Metroid")), scaling("integer"), Effect::Redraw(None)]);
    a.handle(Msg::GameEnded(None));
    assert_eq!(tap(&mut a, Button::B, 0), vec![Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!(focused(&a), "Search");
}

#[test]
fn x_searches_from_a_game_list_and_no_matches_say_so() {
    let mut a = app();
    tap(&mut a, Button::A, 0);
    tap(&mut a, Button::X, 0);
    assert_eq!(tap(&mut a, Button::Start, 0), vec![Effect::Redraw(Some(Transition::Pop))]); // nothing typed: back
    assert_eq!(a.view().title, "Game Boy Advance");
    tap(&mut a, Button::X, 0);
    type_text(&mut a, "zz");
    tap(&mut a, Button::Start, 0);
    assert_eq!(a.view().layout, Layout::List);
    assert_eq!(a.view().rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["No games match this search"]);
    assert_eq!(tap(&mut a, Button::A, 0), vec![]);
    // A new search replaces this one.
    tap(&mut a, Button::X, 0);
    type_text(&mut a, "su");
    assert_eq!(tap(&mut a, Button::Start, 0), vec![Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!(a.view().title, "“su”");
    assert_eq!(a.view().layout, Layout::ListWithPicture);
    assert_eq!(focused(&a), "Super Metroid"); // starts with it, before Golden Sun's word
    assert_eq!(tap(&mut a, Button::R2, 0), vec![]); // results are by relevance: no letter index
    tap(&mut a, Button::B, 0);
    assert_eq!(a.view().title, "Game Boy Advance");
}

#[test]
fn l2_and_r2_jump_between_first_letters() {
    let mut a = App::new(builtin_systems(), quiet());
    let titles = ["1942", "Advance Wars", "Asterix", "Boktai", "Castlevania", "Contra", "The Legend of Zelda"];
    a.handle(Msg::Library { games: titles.iter().map(|t| game("gba", t)).collect(), card_found: true });
    tap(&mut a, Button::A, 0);
    assert_eq!(tap(&mut a, Button::R2, 0), vec![Effect::Redraw(None), Effect::ShowLetter('A')]);
    tap(&mut a, Button::R2, 0);
    assert_eq!(focused(&a), "Boktai");
    tap(&mut a, Button::L2, 0);
    assert_eq!(focused(&a), "Advance Wars");
    assert_eq!(tap(&mut a, Button::L2, 0), vec![Effect::Redraw(None), Effect::ShowLetter('#')]);
    tap(&mut a, Button::L2, 0); // a fresh press wraps
    assert_eq!(focused(&a), "The Legend of Zelda"); // filed under L
    tap(&mut a, Button::R2, 0);
    assert_eq!(focused(&a), "1942");
    // Holding R2 repeats, and stops at the last letter, which stays shown.
    event(&mut a, Button::R2, true, 1000);
    let mut last = vec![];
    for t in (1000..3000).step_by(10) {
        let e = a.handle(Msg::Tick(Duration::from_millis(t)));
        if !e.is_empty() { last = e; }
    }
    assert_eq!(focused(&a), "The Legend of Zelda");
    assert_eq!(last, vec![Effect::ShowLetter('L')]);
    event(&mut a, Button::R2, false, 3000);
    // The grid has it too.
    tap(&mut a, Button::Y, 3100);
    tap(&mut a, Button::R2, 3200);
    assert_eq!((a.view().layout, focused(&a)), (Layout::Grid, "1942".into()));
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
fn a_systems_game_list_and_its_games_take_its_colour() {
    let mut a = app();
    let (indigo, violet) = (crate::palette::find("indigo"), crate::palette::find("violet"));
    assert_eq!(a.tint(), None);
    tap(&mut a, Button::A, 0);
    assert_eq!(a.tint(), indigo);
    // The keyboard over the list isn't the list, and search results span systems...
    tap(&mut a, Button::X, 10);
    assert_eq!(a.tint(), None);
    type_text(&mut a, "metroid");
    tap(&mut a, Button::Start, 20);
    assert_eq!((a.view().title.as_str(), a.tint()), ("“metroid”", None));
    // ...but a game played from them takes its system's colour, also under its menu.
    tap(&mut a, Button::A, 30);
    assert_eq!(a.tint(), violet);
    tap(&mut a, Button::Menu, 40);
    assert_eq!(a.tint(), violet);
    for (i, b) in [Button::Up, Button::A].into_iter().enumerate() { tap(&mut a, b, 50 + i as u64 * 10); } // wraps to "Quit game"
    assert_eq!(a.tint(), None);
    tap(&mut a, Button::B, 70);
    assert_eq!((a.view().title.as_str(), a.tint()), ("Game Boy Advance", indigo));
    // Off, nothing is tinted.
    a.handle(Msg::SettingsLoaded(Layer::Global, serde_json::Map::from_iter([("appearance.tint_by_system".into(), json!(false))])));
    assert_eq!(a.tint(), None);
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
    assert_eq!(tap(&mut a, Button::VolDown, 100), vec![Effect::ShowVolume { level: 0, muted: false, output: Output::Speaker }]); // nothing changed to save
}

#[test]
fn the_mute_switch_mutes_without_touching_the_volume() {
    let mut a = app();
    let switch = |a: &mut App, on: bool, t: u64| a.handle(Msg::Button(ButtonEvent { button: Button::Mute, pressed: on, time: Duration::from_millis(t) }));
    assert_eq!(switch(&mut a, true, 0), vec![Effect::Mute(true), Effect::ShowVolume { level: 12, muted: true, output: Output::Speaker }]);
    assert_eq!(switch(&mut a, true, 10), vec![]); // already on
    // The volume keys still work, and say the sound is muted.
    assert!(tap(&mut a, Button::VolUp, 20).contains(&Effect::ShowVolume { level: 13, muted: true, output: Output::Speaker }));
    assert_eq!(switch(&mut a, false, 30), vec![Effect::Mute(false), Effect::ShowVolume { level: 13, muted: false, output: Output::Speaker }]);
}

#[test]
fn headphones_in_the_jack_have_their_own_volume_also_mid_game() {
    let mut a = app();
    a.handle(Msg::SettingsLoaded(Layer::Global, Default::default()));
    tap(&mut a, Button::A, 0);
    tap(&mut a, Button::A, 10); // playing
    assert_eq!(a.handle(Msg::Headphones(true)), vec![Effect::SoundTo(Output::Headphones)]);
    assert_eq!(a.input_target(), InputTarget::Game);
    let e = tap(&mut a, Button::VolUp, 20);
    assert!(e.contains(&Effect::Apply { key: builtin::HEADPHONE_VOLUME, value: json!(11) }), "{e:?}");
    assert!(e.contains(&Effect::ShowVolume { level: 11, muted: false, output: Output::Headphones }));
    assert_eq!(a.handle(Msg::Headphones(true)), vec![]); // no change
    // Unplugged: back to the speaker, at its own level.
    assert_eq!(a.handle(Msg::Headphones(false)), vec![Effect::SoundTo(Output::Speaker)]);
    assert!(tap(&mut a, Button::VolDown, 30).contains(&Effect::ShowVolume { level: 11, muted: false, output: Output::Speaker }));
}

#[test]
fn headphones_in_at_start_up_take_over_once_the_settings_load() {
    let mut a = app();
    assert_eq!(a.handle(Msg::Headphones(true)), vec![]);
    let e = a.handle(Msg::SettingsLoaded(Layer::Global, Default::default()));
    assert!(e.contains(&Effect::Apply { key: builtin::HEADPHONE_VOLUME, value: json!(10) }));
    assert_eq!(e.last(), Some(&Effect::SoundTo(Output::Headphones)));
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
    let mut a = App::new(builtin_systems(), quiet());
    let labels = |a: &App| a.view().rows.iter().map(|r| r.label.clone()).collect::<Vec<_>>();
    assert!(labels(&a).contains(&"Looking for games…".to_string()));
    a.handle(Msg::Library { games: vec![], card_found: true });
    assert!(labels(&a).contains(&"No games found".to_string()));
    assert_eq!(a.view().rows[a.focus()].label, "Settings"); // notes are never focused
    a.handle(Msg::Library { games: vec![], card_found: false });
    assert!(labels(&a).contains(&"No SD card".to_string()));
}

mod credits {
    use super::*;

    const PROJECTS: &[Credit] = &[
        Credit { kind: CreditKind::Specular, name: "Specular", version: None, licence: "GPL-3.0-only", url: None, copyright: None },
        Credit { kind: CreditKind::Core, name: "mGBA", version: Some("7a12d6d4b9ac"), licence: "MPL-2.0", url: Some("https://github.com/libretro/mgba"),
                 copyright: Some("Copyright © 2013 – 2026 Jeffrey Pfau") },
        Credit { kind: CreditKind::Font, name: "Geist", version: None, licence: "OFL-1.1", url: Some("https://github.com/vercel/geist-font"), copyright: None },
        Credit { kind: CreditKind::Library, name: "Slint", version: None, licence: "GPL-3.0-only", url: Some("https://slint.dev"), copyright: None },
        Credit { kind: CreditKind::Library, name: "serde", version: Some("1.0.219"), licence: "MIT OR Apache-2.0", url: None, copyright: None },
    ];

    /// The app on its credits screen, opened from Settings.
    fn credits_app(texts: Option<&str>) -> App {
        let mut a = app();
        a.set_credits(Credits { projects: PROJECTS, texts: texts.map(Into::into) });
        tap(&mut a, Button::Up, 0);
        tap(&mut a, Button::A, 10);
        while focused(&a) != "Credits" { tap(&mut a, Button::Down, 20); }
        let rows = &a.view().rows;
        assert_eq!(rows[a.focus() - 1].label, "Developer details"); // in the System section
        tap(&mut a, Button::A, 30);
        a
    }

    #[test]
    fn credits_list_specular_then_each_kind_of_project_with_its_licence() {
        let mut a = credits_app(Some("/usr/lib/specular/licences.txt"));
        assert_eq!(a.view().title, "Credits");
        let rows: Vec<(String, Option<String>)> = a.view().rows.iter().map(|r| match &r.kind {
            RowKind::Text { detail } => (r.label.clone(), detail.clone()),
            _ => (r.label.clone(), None),
        }).collect();
        let row = |label: &str, detail: Option<&str>| (label.to_string(), detail.map(String::from));
        assert_eq!(rows, [
            row("The licence texts are in /usr/lib/specular/licences.txt", None),
            row("Specular", Some("GPL-3.0-only")),
            row("Emulator cores", None), row("mGBA 7a12d6d4b9ac", Some("MPL-2.0")),
            row("Fonts", None), row("Geist", Some("OFL-1.1")),
            row("Libraries", None), row("Slint", Some("GPL-3.0-only")), row("serde 1.0.219", Some("MIT OR Apache-2.0")),
        ]);
        // A shows a project's web address, if it has one.
        assert_eq!(focused(&a), "Specular");
        assert_eq!(tap(&mut a, Button::A, 40), vec![]);
        tap(&mut a, Button::Down, 50);
        assert_eq!(tap(&mut a, Button::A, 60), vec![Effect::Notice("https://github.com/libretro/mgba".into())]);
        assert_eq!(tap(&mut a, Button::B, 70), vec![Effect::Redraw(Some(Transition::Pop))]);
        assert_eq!(a.view().title, "Settings");
    }

    #[test]
    fn credits_say_when_the_licence_texts_are_missing() {
        let a = credits_app(None);
        assert_eq!(a.view().rows[0].label, "The licence texts weren't installed with Specular");
    }

    #[test]
    fn the_splash_names_the_core_of_the_game_played_with_its_credit() {
        let mut a = app();
        tap(&mut a, Button::A, 0);
        assert_eq!(a.splash(), None);
        tap(&mut a, Button::A, 10);
        assert_eq!(a.splash(), Some(Splash { core: "mGBA", copyright: None, url: None })); // no credits given
        a.set_credits(Credits { projects: PROJECTS, texts: None });
        assert_eq!(a.splash(), Some(Splash { core: "mGBA", copyright: Some("Copyright © 2013 – 2026 Jeffrey Pfau"), url: Some("https://github.com/libretro/mgba") }));
        // A game that can't start takes its splash with it, and says why.
        assert_eq!(a.handle(Msg::GameEnded(Some("no such file".into()))), vec![Effect::Notice("The game couldn't start: no such file".into()), Effect::Redraw(None)]);
        assert_eq!(a.splash(), None);
    }
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
        assert!(e.contains(&Effect::Wifi(WifiCmd::Join { ssid: "Office".into(), password: Some(Secret::new("qqqqqqqq")) })));
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
    fn select_shows_a_password_until_the_keyboard_closes() {
        let mut a = radio_app();
        open(&mut a, "Wi-Fi", 0);
        a.handle(Msg::Wifi(WifiEvent::Networks(vec![net("Office", true, false)])));
        focus_on(&mut a, "Office");
        let shown = |a: &App| {
            let v = a.view();
            (v.entry.as_ref().unwrap().text.clone(), v.hints.iter().find(|(b, _)| *b == Button::Select).map(|(_, l)| l.clone()))
        };
        tap(&mut a, Button::A, 0);
        tap(&mut a, Button::A, 0);
        assert_eq!(shown(&a), ("•".into(), Some("Show".into())));
        tap(&mut a, Button::Select, 0);
        assert_eq!(shown(&a), ("q".into(), Some("Hide".into())));
        tap(&mut a, Button::B, 0);
        tap(&mut a, Button::B, 0); // the first deletes the "q", the second closes the keyboard
        tap(&mut a, Button::A, 0);
        tap(&mut a, Button::A, 0);
        assert_eq!(shown(&a), ("•".into(), Some("Show".into())));
    }

    #[test]
    fn the_wifi_screen_shows_the_address_and_for_developers_the_other_interfaces() {
        use crate::wireless::wifi::Interface;
        let mut a = radio_app();
        open(&mut a, "Wi-Fi", 0);
        let addresses = |a: &App| a.view().rows.iter().filter(|r| r.id.0.starts_with("address")).map(|r| (r.label.clone(), r.kind.clone())).collect::<Vec<_>>();
        let row = |label: &str, address: &str| (label.to_string(), RowKind::Note { detail: Some(address.into()) });
        a.handle(Msg::Wifi(WifiEvent::Interfaces(vec![Interface { name: "usb0".into(), address: "10.1.1.2".into() }])));
        assert_eq!(addresses(&a), []);
        a.handle(connected("Home"));
        assert_eq!(addresses(&a), [row("Address", "10.0.0.2")]);
        a.handle(Msg::SettingsLoaded(Layer::Global, json!({"ui.developer_details": true}).as_object().unwrap().clone()));
        assert_eq!(addresses(&a), [row("Address", "10.0.0.2"), row("usb0", "10.1.1.2")]);
        a.handle(Msg::Wifi(WifiEvent::Link(WifiLink::Idle)));
        assert_eq!(addresses(&a), [row("usb0", "10.1.1.2")]);
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
        assert!(e.contains(&Effect::ShowVolume { level: 11, muted: false, output: Output::Bluetooth(address.clone()) }));
        // A headset whose sound doesn't open is left until it reconnects.
        let e = a.handle(Msg::SoundFailed(Output::Bluetooth(address.clone())));
        assert_eq!(e[0], Effect::Notice("Couldn't play sound on Buds".into()));
        assert_eq!(radio_effects(&e), [&Effect::SoundTo(Output::Speaker)]);
        let gone = BtDevice { connected: false, audio: false, ..connected.clone() };
        assert_eq!(radio_effects(&a.handle(Msg::Bluetooth(BtEvent::Devices(vec![gone])))), Vec::<&Effect>::new());
        let e = a.handle(Msg::Bluetooth(BtEvent::Devices(vec![connected])));
        assert_eq!(radio_effects(&e), [&Effect::SoundTo(Output::Bluetooth(address))]);
        // Leaving stops the search; turning Bluetooth off brings the sound back.
        assert_eq!(radio_effects(&tap(&mut a, Button::B, 0)), [&Effect::Bluetooth(BtCmd::Search(false))]);
        assert_eq!(focused(&a), "Bluetooth"); // its link on the settings screen
        tap(&mut a, Button::A, 0);
        let e = tap(&mut a, Button::A, 0);
        assert_eq!(radio_effects(&e), [&Effect::Bluetooth(BtCmd::Search(false)), &Effect::Bluetooth(BtCmd::Power(false)), &Effect::SoundTo(Output::Speaker)]);
    }

    #[test]
    fn a_bluetooth_headset_comes_before_headphones_in_the_jack() {
        let mut a = radio_app();
        let mut on = serde_json::Map::new();
        on.insert(builtin::BLUETOOTH.into(), json!(true));
        a.handle(Msg::SettingsLoaded(Layer::Global, on));
        assert_eq!(radio_effects(&a.handle(Msg::Headphones(true))), [&Effect::SoundTo(Output::Headphones)]);
        let address = BtAddress("00:11:22:33:44:55".into());
        let headset = BtDevice { address: address.clone(), name: "Buds".into(), kind: BtKind::Headphones, paired: true, connected: true, audio: true };
        let e = a.handle(Msg::Bluetooth(BtEvent::Devices(vec![headset.clone()])));
        assert_eq!(radio_effects(&e), [&Effect::SoundTo(Output::Bluetooth(address.clone()))]);
        // The jack changes nothing while the headset plays.
        assert_eq!(radio_effects(&a.handle(Msg::Headphones(false))), Vec::<&Effect>::new());
        assert_eq!(radio_effects(&a.handle(Msg::Headphones(true))), Vec::<&Effect>::new());
        // A headset that fails, or goes, leaves the sound to the headphones.
        assert_eq!(radio_effects(&a.handle(Msg::SoundFailed(Output::Bluetooth(address.clone())))), [&Effect::SoundTo(Output::Headphones)]);
        a.handle(Msg::Bluetooth(BtEvent::Devices(vec![])));
        assert_eq!(radio_effects(&a.handle(Msg::Bluetooth(BtEvent::Devices(vec![headset])))), [&Effect::SoundTo(Output::Bluetooth(address))]);
        a.handle(Msg::Bluetooth(BtEvent::Devices(vec![])));
        assert!(tap(&mut a, Button::VolUp, 0).contains(&Effect::ShowVolume { level: 11, muted: false, output: Output::Headphones }));
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

/// The sounds among `effects`.
fn sounds(effects: Vec<Effect>) -> Vec<Sound> {
    effects.into_iter().filter_map(|e| match e { Effect::Sound(s) => Some(s), _ => None }).collect()
}

#[test]
fn moves_tick_and_screens_sound_as_they_open_and_close_but_never_over_a_game() {
    let mut a = app_with(crate::settings::builtin::settings());
    assert_eq!(sounds(tap(&mut a, Button::Down, 0)), [Sound::Move]);
    assert_eq!(sounds(tap(&mut a, Button::Up, 100)), [Sound::Move]);
    assert_eq!(sounds(tap(&mut a, Button::A, 200)), [Sound::Open]);
    assert_eq!(sounds(tap(&mut a, Button::B, 300)), [Sound::Back]);
    assert_eq!(sounds(tap(&mut a, Button::B, 400)), []); // home stays
    tap(&mut a, Button::A, 500);
    assert_eq!(sounds(tap(&mut a, Button::A, 600)), []); // the game starts
    // The game has the sound output, also while paused in its menu, and as it is quit from there.
    assert_eq!(sounds(tap(&mut a, Button::Menu, 700)), []);
    for t in 0..5 { assert_eq!(sounds(tap(&mut a, Button::Down, 800 + 100 * t)), []); }
    assert_eq!(focused(&a), "Quit game");
    assert_eq!(sounds(tap(&mut a, Button::A, 1300)), []);
    assert_eq!(a.playing(), None);
    assert_eq!(sounds(tap(&mut a, Button::B, 1400)), [Sound::Back]);
}

#[test]
fn ticks_closer_than_the_gap_are_dropped() {
    let mut a = app_with(crate::settings::builtin::settings());
    assert_eq!(sounds(tap(&mut a, Button::Down, 0)), [Sound::Move]);
    assert_eq!(sounds(tap(&mut a, Button::Down, TICK_GAP.as_millis() as u64 - 1)), []);
    assert_eq!(focused(&a), "Search"); // the focus still moved
    assert_eq!(sounds(tap(&mut a, Button::Down, TICK_GAP.as_millis() as u64)), [Sound::Move]);
}

#[test]
fn switches_sound_their_new_state_and_values_tick() {
    let mut a = app_with(crate::settings::builtin::settings());
    tap(&mut a, Button::Up, 0);
    tap(&mut a, Button::A, 0);
    assert_eq!(focused(&a), "Brightness");
    assert_eq!(sounds(tap(&mut a, Button::Right, 1000)), [Sound::Move]);
    while focused(&a) != "Developer details" { tap(&mut a, Button::Down, 1000); }
    assert_eq!(sounds(tap(&mut a, Button::A, 2000)), [Sound::Toggle(true)]);
    assert_eq!(sounds(tap(&mut a, Button::A, 3000)), [Sound::Toggle(false)]);
    // Switching the sounds off is silent; switching them back on is heard.
    while focused(&a) != "Interface sounds" { tap(&mut a, Button::Up, 3000); }
    assert_eq!(sounds(tap(&mut a, Button::A, 4000)), []);
    assert_eq!(sounds(tap(&mut a, Button::Down, 5000)), []);
    assert_eq!(sounds(tap(&mut a, Button::Up, 6000)), []);
    assert_eq!(sounds(tap(&mut a, Button::A, 7000)), [Sound::Toggle(true)]);
}

#[test]
fn no_sounds_while_muted_or_at_volume_zero() {
    let mut a = app_with(crate::settings::builtin::settings());
    event(&mut a, Button::Mute, true, 0);
    assert_eq!(sounds(tap(&mut a, Button::Down, 100)), []);
    event(&mut a, Button::Mute, false, 200);
    assert_eq!(sounds(tap(&mut a, Button::Down, 300)), [Sound::Move]);
    a.handle(Msg::SettingsLoaded(Layer::Global, serde_json::Map::from_iter([("sound.volume".into(), json!(0))])));
    assert_eq!(sounds(tap(&mut a, Button::Down, 400)), []);
}

#[test]
fn start_opens_a_games_settings_where_it_can_differ_from_its_systems() {
    let mut a = app();
    tap(&mut a, Button::A, 0); // Game Boy Advance
    assert!(a.view().hints.contains(&(Button::Start, "Settings".into())));
    tap(&mut a, Button::Start, 10);
    assert_eq!((a.view().title.as_str(), a.parent_title().as_deref()), ("Golden Sun", Some("Game Boy Advance")));
    assert_eq!(labels(&a), ["Display", "Scaling", "Game Boy Advance", "All Game Boy Advance games…"]);
    assert_eq!(focused(&a), "Scaling");
    let golden_sun = game("gba", "Golden Sun").settings_layer();
    assert_eq!(tap(&mut a, Button::Right, 20), vec![Effect::Redraw(None), Effect::SaveSettings(golden_sun.clone()), scaling("integer")]); // not playing
    let row = &a.view().rows[a.focus()];
    assert!(row.overridden);
    assert_eq!(row.hint, Some((Button::X, "Reset".into())));
    // all of the system's games: set there, it is inherited, so not marked in the game's
    focus_on(&mut a, "All Game Boy Advance games…");
    tap(&mut a, Button::A, 30);
    assert_eq!(a.view().title, "All Game Boy Advance games");
    tap(&mut a, Button::Left, 40);
    assert_eq!(a.ctx().settings.layer(&Layer::System("gba".into())).get(builtin::SCALING), Some(&json!("stretch")));
    tap(&mut a, Button::B, 50);
    tap(&mut a, Button::Up, 60);
    assert_eq!(focused(&a), "Scaling");
    // X resets the game's own value, which then follows the system's
    assert_eq!(tap(&mut a, Button::X, 70), vec![Effect::Redraw(None), Effect::SaveSettings(golden_sun.clone()), scaling("integer")]);
    let row = &a.view().rows[a.focus()];
    assert_eq!((row.overridden, &row.hint), (false, &None));
    assert_eq!(a.ctx().settings.get(builtin::SCALING, &golden_sun).unwrap().0, &json!("stretch"));
    assert_eq!(tap(&mut a, Button::X, 80), vec![]); // nothing to reset
}

#[test]
fn a_games_settings_over_its_menu_apply_as_it_plays() {
    let mut a = app();
    tap(&mut a, Button::A, 0);
    let e = tap(&mut a, Button::A, 10);
    assert_eq!(e[0], Effect::Launch(game("gba", "Golden Sun")));
    assert_eq!(a.core_options(), [("mgba_idle_optimization".to_string(), "Remove Known".to_string())]);
    tap(&mut a, Button::Menu, 20);
    focus_on(&mut a, "Game settings");
    tap(&mut a, Button::A, 30);
    assert_eq!((a.view().title.as_str(), a.input_target()), ("Golden Sun", InputTarget::App));
    // the change reaches the running game
    assert!(tap(&mut a, Button::Right, 40).contains(&scaling("fit")));
    // B goes back to the menu, Menu resumes the game from either
    tap(&mut a, Button::B, 50);
    assert_eq!(a.view().title, "Paused");
    focus_on(&mut a, "Game settings");
    tap(&mut a, Button::A, 60);
    assert_eq!(tap(&mut a, Button::Menu, 70), vec![Effect::PauseGame(false), Effect::Redraw(Some(Transition::Pop))]);
    assert_eq!((a.input_target(), a.view().title.as_str()), (InputTarget::Game, "Game Boy Advance"));
    // started again, it gets its own scaling
    a.handle(Msg::GameEnded(None));
    tap(&mut a, Button::Down, 80);
    assert!(tap(&mut a, Button::A, 90).contains(&scaling("integer"))); // Mother 3
    a.handle(Msg::GameEnded(None));
    tap(&mut a, Button::Up, 100);
    assert!(tap(&mut a, Button::A, 110).contains(&scaling("fit")));
}

#[test]
fn a_game_ending_closes_its_settings_over_its_menu() {
    let mut a = app();
    tap(&mut a, Button::A, 0);
    tap(&mut a, Button::A, 10);
    tap(&mut a, Button::Menu, 20);
    focus_on(&mut a, "Game settings");
    tap(&mut a, Button::A, 30);
    a.handle(Msg::GameEnded(Some("it crashed".into())));
    assert_eq!(a.view().title, "Game Boy Advance");
}

#[test]
fn with_developer_details_the_cores_options_are_listed_and_reach_the_running_game() {
    let mut a = app();
    a.handle(Msg::SettingsLoaded(Layer::Global, json!({"ui.developer_details": true}).as_object().unwrap().clone()));
    tap(&mut a, Button::A, 0);
    tap(&mut a, Button::A, 10); // Golden Sun
    tap(&mut a, Button::Menu, 20);
    focus_on(&mut a, "Game settings");
    tap(&mut a, Button::A, 30);
    assert!(labels(&a).contains(&"mGBA: Performance".to_string()), "{:?}", labels(&a));
    focus_on(&mut a, "Frameskip");
    let e = tap(&mut a, Button::Right, 40);
    let golden_sun = game("gba", "Golden Sun").settings_layer();
    assert!(e.contains(&Effect::SaveSettings(golden_sun.clone())));
    assert!(e.contains(&Effect::CoreOptions(vec![("mgba_idle_optimization".into(), "Remove Known".into()), ("mgba_frameskip".into(), "auto".into())])), "{e:?}");
    // the global settings don't list them
    a.handle(Msg::GameEnded(None));
    tap(&mut a, Button::Menu, 50);
    tap(&mut a, Button::Up, 60);
    tap(&mut a, Button::A, 70);
    assert_eq!(a.view().title, "Settings");
    assert!(!labels(&a).iter().any(|l| l.starts_with("mGBA")));
}

#[test]
fn settings_files_other_than_the_global_one_apply_nothing_when_they_load() {
    let mut a = app();
    let gba = Layer::System("gba".into());
    let e = a.handle(Msg::SettingsLoaded(gba.clone(), json!({"video.scaling": "fit"}).as_object().unwrap().clone()));
    assert_eq!(e, vec![Effect::Redraw(None)]);
    assert_eq!(a.ctx().settings.get(builtin::SCALING, &game("gba", "Golden Sun").settings_layer()).unwrap().0, &json!("fit"));
}
