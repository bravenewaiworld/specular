use std::time::Duration;

use serde_json::json;

use super::*;
use crate::card::CardStatus;
use crate::library::builtin_systems;
use crate::wireless::wifi::Interface;

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
    app.handle(Msg::Card(Card::mounted()));
    app.handle(Msg::Library(vec![game("gba", "Golden Sun"), game("gba", "Mother 3"), game("snes", "Super Metroid")]));
    app
}

/// `app()` with the card's settings read (none stored).
fn app_with_settings() -> App {
    let mut a = app();
    a.handle(Msg::SettingsLoaded(Layer::Global, Default::default()));
    a
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

/// Moves the focus to the row labelled `label` and presses A.
fn press(a: &mut App, label: &str) -> Vec<Effect> {
    focus_on(a, label);
    tap(a, Button::A, 0)
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
    assert_eq!(tap(&mut a, Button::A, 20), vec![Effect::Launch(game("gba", "Mother 3")), scaling("integer"), Effect::Apply { key: builtin::ART, value: json!("boxart") }, Effect::Apply { key: builtin::CPU_MIN, value: json!("auto") }, Effect::Apply { key: builtin::AUTOSAVE, value: json!(builtin::AUTOSAVE_CORE) }, Effect::Redraw(None)]);
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
    assert_eq!(app().handle(Msg::LastGameStopped(record(None))), vec![Effect::Notice("Golden Sun closed unexpectedly last time. Progress since its last autosave was lost.".into())]);
    assert_eq!(app().handle(Msg::LastGameStopped(record(Some("it stopped responding")))),
               vec![Effect::Notice("Golden Sun was closed because it stopped responding last time. Progress since its last autosave was lost.".into())]);
}

#[test]
fn a_game_that_stops_for_good_is_quit_and_says_why() {
    let mut a = app();
    tap(&mut a, Button::A, 0);
    tap(&mut a, Button::A, 10);
    tap(&mut a, Button::Menu, 20); // even under its menu
    assert_eq!(a.handle(Msg::GameStopped("It needs a BIOS.".into())),
               vec![Effect::QuitGame, Effect::Notice("The game stopped. It needs a BIOS.".into()), Effect::Redraw(None)]);
    assert_eq!((a.playing(), a.input_target(), a.view().title.as_str()), (None, InputTarget::App, "Game Boy Advance"));
    // Already quit: nothing more to do.
    assert_eq!(a.handle(Msg::GameStopped("it needs a BIOS".into())), vec![]);
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
    let mut a = app_with_settings();
    tap(&mut a, Button::Up, 0); // wraps to Settings
    tap(&mut a, Button::A, 10);
    assert_eq!(a.view().title, "Settings");
    assert_eq!(focused(&a), "Brightness");
    assert_eq!(a.view().rows[a.focus()].hints, [(Button::Right, "Change".into()), (Button::Y, "About".into())]); // A does nothing on a number
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
    a.handle(Msg::Card(Card::mounted()));
    a.handle(Msg::Library((0..3000).map(|i| game("snes", &format!("Game {i:04}"))).collect()));
    tap(&mut a, Button::A, 0);
    let t = std::time::Instant::now();
    for i in 0..1000 {
        tap(&mut a, Button::Down, 10 + i);
        let (v, f) = (a.view(), a.focus());
        std::hint::black_box((crate::list::layout(&v.rows, f, None, 1024.0, 512.0, None), a.parent_title()));
    }
    println!("per move (press, view, focus, layout, breadcrumb; not the UI's drawing): {:?}", t.elapsed() / 1000);
}

#[test]
fn y_switches_games_to_a_grid_where_left_and_right_move() {
    let mut a = App::new(builtin_systems(), quiet());
    a.handle(Msg::Card(Card::mounted()));
    a.handle(Msg::Library((0..7).map(|i| game("gba", &format!("Game {i}"))).collect()));
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
    a.handle(Msg::Card(Card::mounted()));
    a.handle(Msg::Library(vec![game("gba", "Metroid Fusion"), game("gba", "Mother 3"), game("snes", "Super Metroid")]));
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
    assert_eq!(results(&a), ["Metroid Fusion Game Boy Advance", "Super Metroid Super Nintendo"]);
    // A rescan keeps the results (the query is the screen's params) and the focus.
    tap(&mut a, Button::Down, 0);
    a.handle(Msg::Library(vec![game("gb", "Metroid II"), game("gba", "Metroid Fusion"), game("snes", "Super Metroid")]));
    assert_eq!(results(&a), ["Metroid Fusion Game Boy Advance", "Metroid II Game Boy", "Super Metroid Super Nintendo"]);
    assert_eq!(focused(&a), "Super Metroid");
    assert_eq!(tap(&mut a, Button::A, 0), vec![Effect::Launch(game("snes", "Super Metroid")), scaling("integer"), Effect::Apply { key: builtin::ART, value: json!("boxart") }, Effect::Apply { key: builtin::CPU_MIN, value: json!("auto") }, Effect::Apply { key: builtin::AUTOSAVE, value: json!(builtin::AUTOSAVE_CORE) }, Effect::Redraw(None)]);
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
    a.handle(Msg::Card(Card::mounted()));
    let titles = ["1942", "Advance Wars", "Asterix", "Boktai", "Castlevania", "Contra", "The Legend of Zelda"];
    a.handle(Msg::Library(titles.iter().map(|t| game("gba", t)).collect()));
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
    assert!(labels(&a).contains(&"Quitting keeps your place: the game goes on from there next time.".into()));
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
fn the_game_menu_changes_the_disc_of_a_game_on_several() {
    let mut a = app();
    let discs = |count, current| Msg::Discs(Discs { count, current });
    let disc_row = |a: &App| a.view().rows.iter().find(|r| r.label == "Disc").map(|r| r.kind.clone());
    let choice = |options: &[&str], selected| Some(RowKind::Choice { options: options.iter().map(|o| o.to_string()).collect(), selected, swatch: None });
    let changed = |disc| vec![Effect::ChangeDisc(disc), Effect::PauseGame(false), Effect::Redraw(Some(Transition::Pop))];
    assert_eq!(a.handle(discs(3, Some(0))), vec![]); // no game yet
    tap(&mut a, Button::A, 0);
    tap(&mut a, Button::A, 10);
    tap(&mut a, Button::Menu, 20);
    assert_eq!(disc_row(&a), None); // until its core says
    a.handle(discs(1, Some(0)));
    assert_eq!(disc_row(&a), None); // one disc
    assert_eq!(a.handle(discs(3, Some(0))), vec![Effect::Redraw(None)]);
    assert_eq!(labels(&a)[..3], ["Resume", "Disc", "Save state"]);
    assert_eq!(disc_row(&a), choice(&["Disc 1 of 3", "Disc 2 of 3", "Disc 3 of 3"], 0));
    focus_on(&mut a, "Disc");
    assert_eq!(tap(&mut a, Button::Left, 30), changed(2)); // wraps to the last
    a.handle(discs(3, Some(2)));
    tap(&mut a, Button::Menu, 40);
    focus_on(&mut a, "Disc");
    assert_eq!(tap(&mut a, Button::Right, 50), changed(0)); // wraps to the first
    tap(&mut a, Button::Menu, 60);
    focus_on(&mut a, "Disc");
    assert_eq!(tap(&mut a, Button::A, 70), changed(0)); // A as right (no `Msg::Discs` since, so the third is still in)
    // With none in, right puts the first in, left the last.
    a.handle(discs(3, None));
    tap(&mut a, Button::Menu, 80);
    assert_eq!(disc_row(&a), choice(&["Disc 1 of 3", "Disc 2 of 3", "Disc 3 of 3", "No disc"], 3));
    focus_on(&mut a, "Disc");
    assert_eq!(tap(&mut a, Button::Right, 90), changed(0));
    tap(&mut a, Button::Menu, 100);
    focus_on(&mut a, "Disc");
    assert_eq!(tap(&mut a, Button::Left, 110), changed(2));
    // The next game has no Disc row until its core says.
    tap(&mut a, Button::Menu, 120);
    for (i, b) in [Button::Up, Button::A].into_iter().enumerate() { tap(&mut a, b, 130 + i as u64 * 10); } // wraps to "Quit game"
    tap(&mut a, Button::A, 150);
    tap(&mut a, Button::Menu, 160);
    assert_eq!(disc_row(&a), None);
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
fn a_pc_connected_over_usb_keeps_the_device_awake() {
    let mut a = app();
    a.has_radios(true, false); // the interfaces come from the WiFi service
    let sec = |n: u64| Duration::from_secs(n);
    let interfaces = |names: &[&str]| Msg::Wifi(WifiEvent::Interfaces(
        names.iter().map(|n| Interface { name: n.to_string(), address: "192.168.7.2".into() }).collect()));
    tap(&mut a, Button::Down, 0);
    // Another wired link (a USB Ethernet adapter) doesn't hold sleep off.
    a.handle(interfaces(&["eth0"]));
    assert_eq!(a.sleep_deadline(), Some(sec(30)));
    a.handle(interfaces(&["eth0", "usb0"]));
    assert_eq!(a.sleep_deadline(), None);
    assert!(!a.handle(Msg::Tick(sec(3600))).contains(&Effect::Sleep));
    // Unplugged after an hour: the count starts again from the next tick, not from the last press.
    a.handle(interfaces(&[]));
    assert_eq!(a.handle(Msg::Tick(sec(3601))), vec![]);
    assert_eq!(a.sleep_deadline(), Some(sec(3631)));
    assert_eq!(a.handle(Msg::Tick(sec(3631))), vec![Effect::Sleep]);
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
    a.handle(Msg::Card(Card::mounted()));
    a.handle(Msg::Library(vec![]));
    assert!(labels(&a).contains(&"No games found".to_string()));
    let place = labels(&a).into_iter().find(|l| l.starts_with("Put games in roms/")).unwrap();
    assert!(place.contains(" .gba ") && place.ends_with(", or a .zip holding one."), "{place}");
    assert_eq!(a.view().rows[a.focus()].label, "Settings"); // notes are never focused
    a.handle(Msg::Card(Card::parse("absent\nNo SD card: insert the one with your games.\n")));
    assert!(labels(&a).contains(&"No SD card: insert the one with your games.".to_string()));
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
        assert_eq!(a.handle(Msg::GameEnded(Some("no such file".into()))), vec![Effect::Notice("no such file".into()), Effect::Redraw(None)]);
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
        assert_eq!(a.view().rows[a.focus()].kind, RowKind::Text { detail: Some("Security not supported".into()) });
        assert_eq!(tap(&mut a, Button::A, 0), vec![Effect::Notice("Lab can't be joined: only open networks and WPA or WPA2 passwords work here, not WPA3, WEP or work logins".into())]);
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
        let b_hint = |a: &App| a.view().hints.iter().find(|(b, _)| *b == Button::B).unwrap().1.clone();
        assert_eq!(b_hint(&a), "Cancel");
        tap(&mut a, Button::A, 0);
        assert_eq!(b_hint(&a), "Delete");
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
        assert_eq!(a.view().rows[a.focus()].hints, [(Button::X, "Forget".into())]); // a saved network
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
        assert_eq!(addresses(&a), [row("Address for File transfer", "10.0.0.2")]);
        a.handle(Msg::SettingsLoaded(Layer::Global, json!({"ui.developer_details": true}).as_object().unwrap().clone()));
        assert_eq!(addresses(&a), [row("Address for File transfer", "10.0.0.2"), row("usb0", "10.1.1.2")]);
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
        assert_eq!(a.view().rows.iter().find(|r| r.label == "Buds").unwrap().kind, RowKind::Text { detail: Some("Connected · sound plays here".into()) });
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
    use crate::install::{Backup, InstallOffer, Internal, Offer, PendingRestore, Progress, Task, TaskError, TaskKind, UpdateFile};

    const BACKUP: &str = "/mnt/SDCARD/backups/internal-20261014-232447";

    /// An app with an installer, at home.
    fn install_app() -> App {
        let mut a = app();
        a.has_installer(true);
        a
    }

    /// On the SD card's system: installing is possible, with one backup; there is a previous
    /// version, and update files: a newer one, an older one and one that can't be installed.
    fn offer() -> Offer {
        let update = |name: &str, version: Option<&str>, older, problem: Option<&str>| UpdateFile {
            file: format!("/mnt/SDCARD/updates/{name}"), version: version.map(Into::into), older, problem: problem.map(Into::into),
        };
        Offer {
            install: Some(InstallOffer { bytes: 7_818_182_656, backup_possible: true, free: 20_000_000_000 }), why_not: None, internal: Internal::TrimUi,
            backups: vec![Backup { dir: BACKUP.into(), created: "20261014-232447".into(), bytes: 7_818_182_656, holds: Some(Internal::TrimUi) }],
            version: Some("20261018-212347-b3a6d97".into()),
            previous: Some("20261012-090000-1a2b3c4".into()),
            updates: vec![update("new.update", Some("20261019-101500-c4d5e6f"), false, None),
                          update("old.update", Some("20000101-000000-0000000"), true, None),
                          update("bad.update", None, false, Some("/mnt/SDCARD/updates/bad.update isn't signed for this system"))],
            restore: None, restore_pending: None,
        }
    }

    /// On the installed system: restores are prepared, nothing can be installed.
    fn installed() -> Offer {
        Offer { install: None, why_not: Some("the system isn't running from an SD card".into()), internal: Internal::Unknown, restore: Some(Ok(())), ..offer() }
    }

    fn pending() -> Offer {
        Offer { restore_pending: Some(PendingRestore { backup: "internal-20261014-232447".into(), prepared: "20261018-120000".into() }), ..offer() }
    }

    /// Opens Settings (which asks the installer what it can do) and gives it `offer`.
    fn settings(a: &mut App, offer: Offer) {
        tap(a, Button::Up, 0); // home wraps to its last row, Settings
        let e = tap(a, Button::A, 0);
        assert!(e.contains(&Effect::Installer(InstallCmd::Offer)), "{e:?}");
        a.handle(Msg::InstallOffer(Ok(offer)));
    }

    #[test]
    fn settings_offers_installing_and_restoring_only_when_the_installer_can() {
        let mut a = install_app();
        settings(&mut a, offer());
        let labels: Vec<String> = a.view().rows.iter().map(|r| r.label.clone()).collect();
        assert!(labels.ends_with(&["Internal storage".into(), "Install to internal storage".into(),
            "Copies the system onto the device, so it starts from its internal storage. Games and saves stay on the SD card.".into(), "Restore internal storage".into()]), "{labels:?}");
        a.handle(Msg::InstallOffer(Ok(Offer { why_not: Some("running from the internal storage".into()), ..Offer::default() })));
        assert!(!a.view().rows.iter().any(|r| r.label == "Updates" || r.label == "Install to internal storage"));
        let labels: Vec<String> = a.view().rows.iter().map(|r| r.label.clone()).collect();
        assert!(labels.ends_with(&["Internal storage".into(), "Installing to internal storage isn't possible on this device.".into()]), "{labels:?}");
    }

    #[test]
    fn settings_show_the_version_updates_and_the_previous_version() {
        let mut a = install_app();
        settings(&mut a, offer());
        let rows = a.view().rows.clone();
        let at = rows.iter().position(|r| r.label == "Updates").expect("an Updates section");
        assert_eq!(rows[at + 1].kind, RowKind::Note { detail: Some("2026-10-18 21:23 UTC".into()) });
        assert_eq!(rows[at + 2..at + 4].iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["Install update", "Start previous version"]);
        // An older installer reports none of it.
        a.handle(Msg::InstallOffer(Ok(Offer { version: None, previous: None, updates: vec![], ..offer() })));
        assert!(!labels(&a).contains(&"Updates".into()));
    }

    #[test]
    fn an_update_warns_when_older_says_why_one_cant_be_installed_then_restarts() {
        let mut a = install_app();
        settings(&mut a, offer());
        press(&mut a, "Install update");
        assert_eq!(a.view().title, "Install update");
        let rows = a.view().rows.clone();
        assert_eq!(rows[1], Row::new("update:/mnt/SDCARD/updates/old.update", "old.update", RowKind::Action { value: Some("Older".into()) }));
        assert_eq!(rows[2].label, "bad.update can't be installed: /mnt/SDCARD/updates/bad.update isn't signed for this system");
        assert!(!rows[2].kind.focusable());
        press(&mut a, "old.update");
        assert_eq!(a.view().title, "Install update?");
        assert_eq!(focused(&a), "Cancel");
        assert!(labels(&a).contains(&"This update is older than version 2026-10-18 21:23 UTC.".into()), "{:?}", labels(&a));
        let e = press(&mut a, "Install");
        assert_eq!(e[0], Effect::Installer(InstallCmd::Start(Task::Update { file: "/mnt/SDCARD/updates/old.update".into() })));
        assert_eq!(a.view().title, "Updating…");
        for b in [Button::B, Button::Menu] { assert_eq!(tap(&mut a, b, 0), vec![]); }
        assert_eq!(tap(&mut a, Button::Power, 0), vec![Effect::Notice("Please wait until updating has finished".into())]);
        assert_eq!(a.sleep_deadline(), None);
        a.handle(Msg::InstallProgress(Progress::Done(Ok("Version 20000101-000000-0000000 starts at the next restart.".into()))));
        assert_eq!(a.view().title, "Updated");
        assert_eq!(focused(&a), "Restart");
        assert_eq!(tap(&mut a, Button::A, 0), vec![Effect::Restart]);
    }

    #[test]
    fn an_update_refused_on_a_low_battery_says_why_and_goes_back() {
        let mut a = install_app();
        settings(&mut a, offer());
        press(&mut a, "Install update");
        press(&mut a, "new.update");
        press(&mut a, "Install");
        let error = TaskError { message: "the battery is at 30 %".into(), hint: "Connect the charger, or charge to at least 50 %, then try again.".into(), log: Some("/mnt/SDCARD/logs/update-x.log".into()) };
        a.handle(Msg::InstallProgress(Progress::Done(Err(error))));
        assert_eq!(a.view().title, "Update failed");
        let l = labels(&a);
        assert!(l.contains(&"Connect the charger, or charge to at least 50 %, then try again.".into()) && l.contains(&"Log: /mnt/SDCARD/logs/update-x.log".into()), "{l:?}");
        tap(&mut a, Button::B, 0);
        assert_eq!(a.view().title, "Install update?");
        assert!(!a.task_running());
    }

    #[test]
    fn the_previous_version_is_started_after_asking_then_a_restart() {
        let mut a = install_app();
        settings(&mut a, offer());
        press(&mut a, "Start previous version");
        assert_eq!(a.view().title, "Start the previous version?");
        assert_eq!(focused(&a), "Cancel");
        let e = press(&mut a, "Switch");
        assert_eq!(e[0], Effect::Installer(InstallCmd::Start(Task::Activate { version: "20261012-090000-1a2b3c4".into() })));
        assert_eq!(a.view().title, "Switching versions…");
        a.handle(Msg::InstallProgress(Progress::Done(Ok("Version 20261012-090000-1a2b3c4 starts at the next restart.".into()))));
        assert_eq!(press(&mut a, "Restart"), vec![Effect::Restart]);
    }

    #[test]
    fn restoring_from_the_installed_system_prepares_it_then_restarts() {
        let mut a = install_app();
        settings(&mut a, installed());
        assert!(!labels(&a).contains(&"Install to internal storage".into()));
        press(&mut a, "Restore internal storage");
        press(&mut a, "TrimUI's system · 2026-10-14 23:24 UTC");
        assert!(labels(&a)[0].starts_with("The SD card is made to start the device"), "{:?}", labels(&a));
        let e = press(&mut a, "Restore");
        assert_eq!(e[0], Effect::Installer(InstallCmd::Start(Task::PrepareRestore { dir: BACKUP.into() })));
        assert_eq!(a.view().title, "Preparing the restore…");
        a.handle(Msg::InstallProgress(Progress::Done(Ok("Restart to continue the restore from the SD card.".into()))));
        assert_eq!(focused(&a), "Restart");
    }

    #[test]
    fn a_restore_that_cant_be_prepared_isnt_offered() {
        let mut a = install_app();
        settings(&mut a, Offer { restore: Some(Err("the SD card's boot slot \"boot_a\" holds no system".into())), ..installed() });
        assert!(!labels(&a).contains(&"Restore internal storage".into()));
    }

    #[test]
    fn a_prepared_restore_is_asked_about_once_and_never_run_by_itself() {
        let mut a = install_app();
        // At start-up the installer reports it: the question opens, on Not now.
        let e = a.handle(Msg::InstallOffer(Ok(pending())));
        assert!(!e.iter().any(|e| matches!(e, Effect::Installer(_))), "{e:?}");
        assert_eq!(a.view().title, "Restore the internal storage?");
        assert_eq!(focused(&a), "Not now");
        assert!(labels(&a)[0].starts_with("A restore was prepared on 2026-10-18 12:00 UTC"), "{:?}", labels(&a));
        assert_eq!(tap(&mut a, Button::B, 0), vec![Effect::Redraw(Some(Transition::Pop))]);
        // Only once; afterwards it is in Settings.
        settings(&mut a, pending());
        assert_eq!(a.view().title, "Settings");
        press(&mut a, "Continue the prepared restore");
        let e = press(&mut a, "Restore");
        assert_eq!(e[0], Effect::Installer(InstallCmd::Start(Task::Restore { dir: BACKUP.into() })));
        assert_eq!(a.view().title, "Restoring…");
        a.handle(Msg::InstallProgress(Progress::Done(Ok("Restored the internal storage. Power off, remove the SD card, then turn the device on.".into()))));
        assert_eq!(focused(&a), "Power off");
    }

    #[test]
    fn a_prepared_restore_can_be_cancelled() {
        let mut a = install_app();
        a.handle(Msg::InstallOffer(Ok(pending())));
        let e = press(&mut a, "Cancel the restore");
        assert_eq!(e[0], Effect::Installer(InstallCmd::Start(Task::CancelRestore)));
        assert_eq!(a.view().title, "Cancelling the restore…");
        a.handle(Msg::InstallProgress(Progress::Done(Ok("The restore is cancelled. Restart to start from the internal storage.".into()))));
        assert_eq!(focused(&a), "Restart");
    }

    #[test]
    fn a_prepared_restore_whose_backup_is_gone_cant_be_run() {
        let mut a = install_app();
        a.handle(Msg::InstallOffer(Ok(Offer { backups: vec![], ..pending() })));
        assert_eq!(labels(&a), ["The backup this restore was prepared for (internal-20261014-232447) isn't on the SD card any more.", "Not now", "Cancel the restore"]);
    }

    #[test]
    fn a_prepared_restore_isnt_asked_about_over_a_running_task() {
        let mut a = install_app();
        a.handle(Msg::InstallProgress(Progress::Task(TaskKind::Restore)));
        a.handle(Msg::InstallOffer(Ok(pending())));
        assert_eq!(a.view().title, "Restoring…");
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
        a.handle(Msg::InstallProgress(Progress::Task(TaskKind::Restore)));
        a.handle(Msg::InstallProgress(Progress::Step { index: 1, of: 3, label: "Checking the backup".into() }));
        assert_eq!(a.view().title, "Restoring…");
        assert!(a.task_running());
        assert_eq!(a.sleep_deadline(), None);
        assert_eq!(tap(&mut a, Button::Power, 0), vec![Effect::Notice("Please wait until restoring has finished".into())]);
        a.handle(Msg::InstallProgress(Progress::Done(Ok("Restored.".into()))));
        assert_eq!(focused(&a), "Power off");
        // A task this run started also reports which it is first: nothing changes.
        assert_eq!(a.handle(Msg::InstallProgress(Progress::Task(TaskKind::Install))), vec![]);
        // An update left running is named as one.
        let mut a = install_app();
        a.handle(Msg::InstallProgress(Progress::Task(TaskKind::Update)));
        assert_eq!(a.view().title, "Updating…");
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
fn games_show_the_kind_of_picture_set_for_their_system_in_its_shape() {
    let mut a = app();
    let art = |a: &App| a.view().rows.iter().map(|r| match &r.kind {
        RowKind::Image { image: ImageRef::Art { kind, shape, .. }, .. } => (*kind, *shape),
        _ => panic!("{r:?}"),
    }).collect::<Vec<_>>();
    tap(&mut a, Button::A, 0); // Game Boy Advance
    assert_eq!(art(&a), [(ArtKind::BoxArt, (1, 1)); 2]);
    a.handle(Msg::SettingsLoaded(Layer::System("gba".into()), serde_json::Map::from_iter([(builtin::ART.into(), json!("title"))])));
    assert_eq!(art(&a), [(ArtKind::TitleScreen, (240, 160)); 2]);
    // Search results across systems: each its own.
    a.handle(Msg::Library(vec![game("gba", "Metroid Fusion"), game("snes", "Super Metroid")]));
    tap(&mut a, Button::X, 10);
    type_text(&mut a, "metroid");
    tap(&mut a, Button::Start, 20);
    assert_eq!(art(&a), [(ArtKind::TitleScreen, (240, 160)), (ArtKind::BoxArt, (1, 1))]);
}

#[test]
fn start_opens_a_games_settings_where_it_can_differ_from_its_systems() {
    let mut a = app();
    tap(&mut a, Button::A, 0); // Game Boy Advance
    assert!(a.view().hints.contains(&(Button::Start, "Game settings".into())));
    tap(&mut a, Button::Start, 10);
    assert_eq!((a.view().title.as_str(), a.parent_title().as_deref()), ("Golden Sun", Some("Game Boy Advance")));
    assert_eq!(labels(&a), ["Changes here apply only to Golden Sun. A dot marks each one; X puts it back to what all Game Boy Advance games use.",
                            "Display", "Scaling", "Game Boy Advance", "All Game Boy Advance games…"]);
    assert_eq!(focused(&a), "Scaling");
    let golden_sun = game("gba", "Golden Sun").settings_layer();
    assert_eq!(tap(&mut a, Button::Right, 20), vec![Effect::Redraw(None), Effect::SaveSettings(golden_sun.clone()), scaling("integer")]); // not playing
    let row = &a.view().rows[a.focus()];
    assert!(row.overridden);
    assert_eq!(row.hints, [(Button::X, "Reset".into()), (Button::Y, "About".into())]);
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
    assert_eq!((row.overridden, &row.hints[..]), (false, &[(Button::Y, "About".into())][..]));
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
fn a_games_cpu_minimum_starts_at_its_cores_level_unless_a_config_file_says_otherwise() {
    use crate::device::simulated::STEPS;
    let mut a = app();
    a.handle(Msg::Library(vec![game("ps1", "Vagrant Story"), game("gba", "Golden Sun")]));
    assert_eq!(a.cpu_min(STEPS.into()).0.current(), None, "no game");
    tap(&mut a, Button::A, 0); // Game Boy Advance
    tap(&mut a, Button::A, 10);
    assert_eq!(a.cpu_min(STEPS.into()).0.current(), None, "mGBA has no start level");
    a.handle(Msg::GameEnded(None));
    tap(&mut a, Button::B, 20);
    tap(&mut a, Button::Down, 30); // PlayStation
    tap(&mut a, Button::A, 40);
    tap(&mut a, Button::A, 50);
    let (min, line) = a.cpu_min(STEPS.into());
    assert_eq!(min.current(), Some(1800));
    assert!(line.starts_with("SwanStation starts with a minimum of 1800 MHz") && line.ends_with("power.cpu_min = \"auto\", its default)"), "{line}");
    assert_eq!(a.cpu_min(vec![]).0.current(), None, "where the speed can't be set");
    // A game's own file overrides it.
    let layer = game("ps1", "Vagrant Story").settings_layer();
    a.handle(Msg::SettingsLoaded(layer, serde_json::Map::from_iter([(builtin::CPU_MIN.into(), json!("1608"))])));
    let (min, line) = a.cpu_min(STEPS.into());
    assert_eq!(min.current(), Some(1608));
    assert!(line.ends_with("power.cpu_min = \"1608\", in games/ps1/Vagrant Story.json)"), "{line}");
}

#[test]
fn a_games_autosave_is_its_cores_unless_a_config_file_says_otherwise() {
    use crate::library::Autosave::{AtPauses, Every10s};
    let mut a = app();
    a.handle(Msg::Library(vec![game("ps1", "Vagrant Story"), game("gba", "Golden Sun")]));
    tap(&mut a, Button::A, 0); // Game Boy Advance
    tap(&mut a, Button::A, 10);
    assert_eq!(a.autosave(), (Every10s, "mGBA autosaves every 10 s of play, and when play stops (game.autosave = \"core default\", its default)".into()));
    a.handle(Msg::GameEnded(None));
    tap(&mut a, Button::B, 20);
    tap(&mut a, Button::Down, 30); // PlayStation
    tap(&mut a, Button::A, 40);
    tap(&mut a, Button::A, 50);
    assert_eq!(a.autosave().0, AtPauses, "SwanStation's own");
    // Each layer overrides the less specific ones.
    let set = |a: &mut App, layer: Layer, v: &str| { a.handle(Msg::SettingsLoaded(layer, serde_json::Map::from_iter([(builtin::AUTOSAVE.into(), json!(v))]))); };
    set(&mut a, Layer::Global, builtin::AUTOSAVE_EVERY_10S);
    assert_eq!(a.autosave(), (Every10s, "SwanStation autosaves every 10 s of play, and when play stops (game.autosave = \"every 10 s\", in specular.json)".into()));
    set(&mut a, Layer::System("ps1".into()), builtin::AUTOSAVE_CORE);
    assert_eq!(a.autosave().0, AtPauses);
    set(&mut a, game("ps1", "Vagrant Story").settings_layer(), builtin::AUTOSAVE_EVERY_10S);
    let (when, line) = a.autosave();
    assert_eq!(when, Every10s);
    assert!(line.ends_with("(game.autosave = \"every 10 s\", in games/ps1/Vagrant Story.json)"), "{line}");
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

mod user_cores {
    use super::*;
    use crate::library::user_cores::{TurnedAway, UserCores, playable, tests::{GAMBATTE, SNES9X, cores_dir, library}};

    /// The app once the cores of a card holding Snes9x and Gambatte are found, with a file turned
    /// away; and the card (the cores' files go with it).
    fn with_user_cores() -> (App, tempfile::TempDir) {
        let card = cores_dir(&[("snes9x_libretro.so", &library()), ("snes9x_libretro.info", SNES9X.as_bytes()),
                               ("gambatte_libretro.so", &library()), ("gambatte_libretro.info", GAMBATTE.as_bytes())]);
        let found = UserCores::default().find(card.path(), card.path(), &builtin_systems());
        let systems = playable(builtin_systems(), &found.cores);
        let mut a = app();
        let turned_away = vec![TurnedAway { file: "bare_libretro.so".into(), why: "no .info".into() }];
        assert_eq!(a.handle(Msg::Cores { settings: crate::settings::declarations(&systems), systems, turned_away }), vec![Effect::Redraw(None)]);
        (a, card)
    }

    fn open_a_system(a: &mut App, name: &str) {
        focus_on(a, name);
        tap(a, Button::A, 0);
    }

    #[test]
    fn a_game_of_a_system_served_by_a_user_core_plays_with_it_and_its_splash_names_it() {
        let (mut a, _card) = with_user_cores();
        assert_eq!(labels(&a)[..3], ["Systems", "Game Boy Advance", "Super Nintendo"]);
        open_a_system(&mut a, "Super Nintendo");
        tap(&mut a, Button::A, 0);
        assert_eq!(a.playing_core().map(|c| c.name), Some("Snes9x"));
        // Not shipped, so not in the credits: what its .info says instead.
        assert_eq!(a.splash(), Some(Splash { core: "Nintendo - SNES / SFC (Snes9x)", copyright: Some("Snes9x Team · Non-commercial"), url: None }));
        assert_eq!(a.core_options(), vec![]); // none learned yet
        // Its options learned meanwhile, it is given the values set for them.
        let e = a.handle(Msg::Cores { systems: a.ctx().systems.clone(), settings: a.ctx().settings.decls().to_vec(), turned_away: vec![] });
        assert_eq!(e, vec![Effect::Redraw(None), Effect::CoreOptions(vec![])]);
    }

    #[test]
    fn the_core_setting_chooses_a_games_core_and_shows_that_cores_options() {
        let (mut a, _card) = with_user_cores();
        let tetris = game("gb", "Tetris");
        a.handle(Msg::Library(vec![tetris.clone()]));
        let at = tetris.settings_layer();
        assert_eq!(a.ctx().settings.get("core.gb", &at).unwrap().0, &json!("mgba"));
        open_a_system(&mut a, "Game Boy");
        tap(&mut a, Button::Start, 0);
        focus_on(&mut a, "Emulator");
        let e = tap(&mut a, Button::Right, 10);
        assert!(e.contains(&Effect::SaveSettings(at.clone())), "{e:?}");
        assert_eq!(a.ctx().settings.get("core.gb", &at).unwrap().0, &json!("gambatte"));
        tap(&mut a, Button::B, 20);
        tap(&mut a, Button::A, 30);
        assert_eq!(a.playing_core().map(|c| c.id()), Some("gambatte"));
    }

    #[test]
    fn the_cores_screen_lists_every_core_and_the_files_turned_away() {
        let (mut a, _card) = with_user_cores();
        a.handle(Msg::SettingsLoaded(Layer::Global, json!({ builtin::DEVELOPER_DETAILS: true }).as_object().unwrap().clone()));
        press(&mut a, "Settings");
        focus_on(&mut a, "Cores");
        tap(&mut a, Button::A, 20);
        let rows: Vec<(String, Option<String>)> = a.view().rows.iter().map(|r| (r.label.clone(), match &r.kind {
            RowKind::Text { detail } => detail.clone(), _ => None,
        })).collect();
        let row = |l: &str, d: Option<&str>| (l.to_string(), d.map(str::to_string));
        assert_eq!(rows[..8], [row("Built in", None), row("mGBA", Some("GB, GBC, GBA")), row("Nestopia", Some("NES")), row("SwanStation", Some("PS1")),
                               row("On the SD card", None), row("Gambatte", Some("GB, GBC")), rows[6].clone(), row("Snes9x", Some("SNES"))]);
        assert_eq!(rows[6].0, "gambatte_libretro.so; its options are learned when it next plays a game");
        assert_eq!(rows[9..], [row("Turned away", None), row("bare_libretro.so", None), row("No .info", None)]);
    }
}

mod transfer {
    use super::*;
    use crate::wireless::wifi::{Interface, WifiCmd, WifiEvent, WifiLink};

    /// An app with WiFi (on Auto) and file transfer, its settings loaded, at home.
    fn transfer_app() -> App {
        let mut a = app();
        a.has_radios(true, false);
        a.has_file_transfer(true);
        a.handle(Msg::SettingsLoaded(Layer::Global, Default::default()));
        a
    }

    /// Opens Settings → File transfer at time `t` (ms); returns the effects of opening it.
    fn open(a: &mut App, t: u64) -> Vec<Effect> {
        tap(a, Button::Up, t); // home wraps to its last row, Settings
        tap(a, Button::A, t);
        press(a, "File transfer")
    }

    fn started(a: &mut App) { a.handle(Msg::FileTransfer { start: true, result: Ok(()) }); }

    fn row_value(a: &App, label: &str) -> Option<String> {
        a.view().rows.iter().find(|r| r.label == label).and_then(|r| match &r.kind {
            RowKind::Action { value } | RowKind::Note { detail: value } => value.clone(),
            _ => None,
        })
    }

    #[test]
    fn it_runs_while_its_screen_is_open_and_wifi_on_auto_comes_on_for_it() {
        let mut a = transfer_app();
        let e = open(&mut a, 0);
        assert!(e.contains(&Effect::Wifi(WifiCmd::Power(true))) && e.contains(&Effect::FileTransfer(true)), "{e:?}");
        assert!(labels(&a).contains(&"Starting…".into()));
        started(&mut a);
        assert!(labels(&a)[0].starts_with("While this screen is open, anyone on the same network"));
        assert_eq!(row_value(&a, "Wi-Fi").as_deref(), Some("Starting…")); // WiFi is still coming on
        assert_eq!(row_value(&a, "USB cable"), None); // no cable in
        a.handle(Msg::Wifi(WifiEvent::Interfaces(vec![Interface { name: "usb0".into(), address: "192.168.7.2".into() }])));
        assert_eq!(row_value(&a, "USB cable").as_deref(), Some("http://192.168.7.2/"));
        a.handle(Msg::Wifi(WifiEvent::Link(WifiLink::Connected { ssid: "Home".into(), address: "10.0.0.2".into(), bars: None })));
        assert_eq!(row_value(&a, "Wi-Fi").as_deref(), Some("http://10.0.0.2/"));
        // The Wi-Fi screen opens over it, with the server still running.
        assert!(!press(&mut a, "Wi-Fi").contains(&Effect::FileTransfer(false)));
        assert_eq!(a.view().title, "Wi-Fi");
        tap(&mut a, Button::B, 0);
        assert!(tap(&mut a, Button::B, 0).contains(&Effect::FileTransfer(false)));
        // Menu goes home, which closes it too.
        press(&mut a, "File transfer");
        assert!(tap(&mut a, Button::Menu, 0).contains(&Effect::FileTransfer(false)));
    }

    #[test]
    fn it_keeps_the_device_awake_and_stops_while_asleep() {
        let mut a = transfer_app();
        open(&mut a, 0);
        started(&mut a);
        a.handle(Msg::Tick(Duration::ZERO));
        assert_eq!(a.sleep_deadline(), None);
        assert!(!a.handle(Msg::Tick(Duration::from_secs(3600))).contains(&Effect::Sleep));
        // A tap of the power button sleeps, stopping it first; waking starts it again.
        event(&mut a, Button::Power, true, 3_600_000);
        let e = event(&mut a, Button::Power, false, 3_600_100);
        let at = |x: &Effect| e.iter().position(|y| y == x);
        assert!(at(&Effect::FileTransfer(false)).unwrap() < at(&Effect::Sleep).unwrap(), "{e:?}");
        assert!(a.handle(Msg::Woke).contains(&Effect::FileTransfer(true)));
    }

    #[test]
    fn without_a_card_it_says_so_and_starts_once_one_is_in() {
        let mut a = transfer_app();
        a.handle(Msg::Card(Card::parse("absent\nNo SD card.\n")));
        assert!(!open(&mut a, 0).contains(&Effect::FileTransfer(true)));
        assert_eq!(labels(&a)[0], "No SD card.");
        assert!(a.handle(Msg::Card(Card::mounted())).contains(&Effect::FileTransfer(true)));
        started(&mut a);
        // Taken out while it runs: stopped (the system also stops it by itself).
        assert!(a.handle(Msg::Card(Card::parse("absent\nNo SD card.\n"))).contains(&Effect::FileTransfer(false)));
    }

    #[test]
    fn a_failed_start_says_why_and_can_be_tried_again() {
        let mut a = transfer_app();
        open(&mut a, 0);
        a.handle(Msg::FileTransfer { start: true, result: Err("Failed to bind `0.0.0.0:80`".into()) });
        assert!(labels(&a).contains(&"File transfer didn't start. Try again; if it still doesn't, restart the device.".into()));
        assert!(press(&mut a, "Try again").contains(&Effect::FileTransfer(true)));
        assert!(labels(&a).contains(&"Starting…".into()));
    }

    #[test]
    fn a_result_overtaken_by_leaving_changes_nothing_but_a_failed_stop_is_told() {
        let mut a = transfer_app();
        open(&mut a, 0);
        tap(&mut a, Button::B, 0); // left while starting
        assert_eq!(a.handle(Msg::FileTransfer { start: true, result: Ok(()) }), vec![]);
        assert_eq!(a.ctx().transfer.as_ref().unwrap().server, Server::Starting);
        a.handle(Msg::FileTransfer { start: false, result: Ok(()) });
        assert_eq!(a.ctx().transfer.as_ref().unwrap().server, Server::Stopped);
        assert_eq!(a.handle(Msg::FileTransfer { start: false, result: Err("timeout: run: webdav".into()) }),
                   vec![Effect::Notice("File transfer may still be on. Restart the device to be sure it is off.".into())]);
    }
}

mod ssh {
    use super::*;

    /// An app with SSH the user switches, at home with developer details on.
    fn ssh_app() -> App {
        let mut a = app();
        a.has_ssh(true);
        let developer: serde_json::Map<String, serde_json::Value> = [("ui.developer_details".to_string(), json!(true))].into_iter().collect();
        a.handle(Msg::SettingsLoaded(Layer::Global, developer));
        a
    }

    /// Opens Settings (which asks SSH's state) and answers with `status`.
    fn settings(a: &mut App, status: Result<SshStatus, String>) {
        tap(a, Button::Up, 0);
        assert!(tap(a, Button::A, 0).contains(&Effect::Ssh(SshCmd::Status)));
        a.handle(Msg::Ssh { cmd: SshCmd::Status, result: status });
    }

    const RELEASE: SshStatus = SshStatus { switchable: true, on: false, keys: 0 };

    #[test]
    fn a_release_system_offers_the_switch_and_the_import() {
        let mut a = ssh_app();
        settings(&mut a, Ok(RELEASE));
        assert!(labels(&a).contains(&"Import SSH keys from the SD card".into()));
        assert_eq!(press(&mut a, "SSH"), vec![Effect::Ssh(SshCmd::Switch(true))]);
        a.handle(Msg::Ssh { cmd: SshCmd::Switch(true), result: Ok(SshStatus { on: true, ..RELEASE }) });
        assert!(a.view().rows.iter().any(|r| r.label == "SSH" && r.kind == RowKind::Toggle { on: true }));
        assert_eq!(press(&mut a, "Import SSH keys from the SD card"), vec![Effect::Ssh(SshCmd::ImportKeys)]);
        let e = a.handle(Msg::Ssh { cmd: SshCmd::ImportKeys, result: Ok(SshStatus { on: true, switchable: true, keys: 2 }) });
        assert_eq!(e[0], Effect::Notice("Imported 2 SSH keys".into()));
        assert!(a.view().rows.iter().any(|r| r.kind == RowKind::Action { value: Some("2 SSH keys".into()) }));
    }

    #[test]
    fn a_refused_import_says_why_and_keeps_the_keys() {
        let mut a = ssh_app();
        settings(&mut a, Ok(SshStatus { keys: 1, ..RELEASE }));
        let e = a.handle(Msg::Ssh { cmd: SshCmd::ImportKeys, result: Err("no valid key in the SD card's authorized_keys".into()) });
        assert_eq!(e[0], Effect::Notice("Couldn't import SSH keys: no valid key in the SD card's authorized_keys".into()));
        assert!(a.view().rows.iter().any(|r| r.kind == RowKind::Action { value: Some("1 SSH key".into()) }));
    }

    #[test]
    fn a_development_system_or_no_developer_details_shows_no_switch() {
        let mut a = ssh_app();
        settings(&mut a, Ok(SshStatus { switchable: false, on: true, keys: 1 }));
        assert!(!labels(&a).contains(&"SSH".into()));
        assert!(labels(&a).contains(&"SSH is always on (a development system).".into()));
        let mut a = app();
        a.has_ssh(true);
        settings(&mut a, Ok(RELEASE));
        assert!(!labels(&a).contains(&"SSH".into()));
    }
}

mod time_zone {
    use super::*;
    use crate::timezone::{ZoneCmd, ZoneEvent};
    use crate::wireless::wifi::{WifiEvent, WifiLink};

    const ZONES: [&str; 5] = ["UTC", "America/Argentina/Buenos_Aires", "America/Toronto", "Europe/Berlin", "Europe/Paris"];

    /// An app with WiFi (not connected) and a time zone (UTC), at home, its settings loaded with
    /// `stored`.
    fn zone_app(stored: serde_json::Value) -> App {
        let mut a = app();
        a.has_radios(true, false);
        a.has_time_zone(true);
        a.handle(Msg::SettingsLoaded(Layer::Global, stored.as_object().unwrap().clone()));
        a.handle(Msg::TimeZone(ZoneEvent::Started { zone: Some("UTC".into()), zones: ZONES.map(String::from).to_vec() }));
        a
    }

    /// One where the lookup was allowed earlier.
    fn allowed_app() -> App { zone_app(json!({"time.zone_lookup_allowed": true})) }

    fn connected(ssid: &str) -> Msg { Msg::Wifi(WifiEvent::Link(WifiLink::Connected { ssid: ssid.into(), address: "10.0.0.2".into(), bars: None })) }
    fn looked_up(answer: Result<&str, &str>) -> Msg { Msg::TimeZone(ZoneEvent::LookedUp(answer.map(String::from).map_err(String::from))) }
    fn set(zone: &str, result: Result<(), &str>) -> Msg { Msg::TimeZone(ZoneEvent::Set { zone: zone.into(), result: result.map_err(String::from) }) }

    fn zone_cmds(e: &[Effect]) -> Vec<&ZoneCmd> { e.iter().filter_map(|e| match e { Effect::TimeZone(c) => Some(c), _ => None }).collect() }
    fn zone(a: &App) -> Option<String> { a.ctx().time_zone.as_ref().unwrap().zone.clone() }
    fn stored(a: &App, key: &str) -> Option<serde_json::Value> { a.ctx().settings.layer(&Layer::Global).get(key).cloned() }

    /// Moves the focus to the row labelled `label` and presses A.
    fn press(a: &mut App, label: &str) -> Vec<Effect> {
        for _ in 0..a.view().rows.len() {
            if focused(a) == label { return tap(a, Button::A, 0); }
            tap(a, Button::Down, 0);
        }
        panic!("no row {label} in {:?}", labels(a));
    }

    /// Opens Settings, then its Time zone screen.
    fn open_time_zone(a: &mut App) {
        tap(a, Button::Up, 0); // home wraps to its last row, Settings
        tap(a, Button::A, 0);
        press(a, "Time zone");
    }

    #[test]
    fn the_question_is_asked_when_wifi_first_connects_and_allowing_looks_up() {
        let mut a = zone_app(json!({}));
        assert_eq!(a.view().title, "Library"); // not before there is a network
        let e = a.handle(connected("Home"));
        assert_eq!(zone_cmds(&e), Vec::<&ZoneCmd>::new());
        assert_eq!(a.view().title, "Look up the time zone?");
        let e = press(&mut a, "Allow");
        assert_eq!(zone_cmds(&e), [&ZoneCmd::LookUp]);
        assert!(e.contains(&Effect::SaveSettings(Layer::Global)));
        assert_eq!(a.view().title, "Library");
        assert_eq!(stored(&a, "time.zone_lookup_allowed"), Some(json!(true)));
    }

    #[test]
    fn the_question_waits_until_the_game_ends() {
        let mut a = zone_app(json!({}));
        tap(&mut a, Button::A, 0);
        tap(&mut a, Button::A, 0); // plays Golden Sun
        a.handle(connected("Home"));
        assert_eq!(a.input_target(), InputTarget::Game);
        a.handle(Msg::GameEnded(None));
        assert_eq!(a.view().title, "Look up the time zone?");
    }

    #[test]
    fn declining_keeps_the_zone_and_is_never_asked_again_unprompted() {
        let mut a = zone_app(json!({}));
        a.handle(connected("Home"));
        let e = tap(&mut a, Button::B, 0);
        assert_eq!(zone_cmds(&e), Vec::<&ZoneCmd>::new());
        assert_eq!(a.view().title, "Library");
        assert_eq!(stored(&a, "time.zone"), Some(json!("manual")));
        // Nor after a restart, with the settings saved.
        let mut b = zone_app(serde_json::Value::Object(a.ctx().settings.layer(&Layer::Global)));
        assert_eq!(zone_cmds(&b.handle(connected("Home"))), Vec::<&ZoneCmd>::new());
        assert_eq!(b.view().title, "Library");
        assert_eq!(zone(&b).as_deref(), Some("UTC"));
    }

    #[test]
    fn the_question_stays_until_answered_and_is_put_once() {
        let mut a = zone_app(json!({}));
        a.handle(connected("Home"));
        assert_eq!(tap(&mut a, Button::Menu, 0), vec![]);
        a.handle(Msg::Wifi(WifiEvent::Link(WifiLink::Idle)));
        a.handle(connected("Home"));
        assert_eq!(a.view().title, "Look up the time zone?");
        tap(&mut a, Button::B, 0);
        assert_eq!(a.view().title, "Library"); // not a second one below
    }

    #[test]
    fn choosing_it_myself_opens_the_picker_which_sets_the_zone() {
        let mut a = zone_app(json!({}));
        a.handle(connected("Home"));
        press(&mut a, "Choose it myself");
        assert_eq!(a.view().title, "Choose a time zone");
        assert_eq!(labels(&a), ["UTC", "America", "Europe"]);
        press(&mut a, "America");
        assert_eq!(labels(&a), ["Argentina / Buenos Aires", "Toronto"]);
        let e = press(&mut a, "Toronto");
        assert_eq!(zone_cmds(&e), [&ZoneCmd::Set("America/Toronto".into())]);
        assert!(e.contains(&Effect::SaveSettings(Layer::Global)));
        assert_eq!(a.view().title, "Library"); // back where the question was put
        assert_eq!(stored(&a, "time.zone"), Some(json!("manual")));
        a.handle(set("America/Toronto", Ok(())));
        assert_eq!(zone(&a).as_deref(), Some("America/Toronto"));
    }

    #[test]
    fn a_lookup_is_made_once_per_connection() {
        let mut a = allowed_app();
        assert_eq!(zone_cmds(&a.handle(connected("Home"))), [&ZoneCmd::LookUp]);
        // The link is reported again with each signal reading.
        assert_eq!(zone_cmds(&a.handle(connected("Home"))), Vec::<&ZoneCmd>::new());
        a.handle(Msg::Wifi(WifiEvent::Link(WifiLink::Idle)));
        assert_eq!(zone_cmds(&a.handle(connected("Home"))), [&ZoneCmd::LookUp]);
        assert_eq!(zone_cmds(&a.handle(connected("Cafe"))), [&ZoneCmd::LookUp]);
        assert_eq!(a.view().title, "Library"); // allowed earlier: not asked
    }

    #[test]
    fn a_lookup_is_set_unless_the_zone_was_picked_by_hand_since() {
        let mut a = allowed_app();
        a.handle(connected("Home"));
        assert_eq!(zone_cmds(&a.handle(looked_up(Ok("America/Toronto")))), [&ZoneCmd::Set("America/Toronto".into())]);
        a.handle(set("America/Toronto", Ok(())));
        assert_eq!(zone(&a).as_deref(), Some("America/Toronto"));
        // The zone in effect already: nothing to set.
        assert_eq!(a.handle(looked_up(Ok("America/Toronto"))), vec![]);
        // Failures change nothing and say nothing; a zone the device refuses (one it has no file for)
        // leaves the zone as it was.
        assert_eq!(a.handle(looked_up(Err("ipinfo.io: timeout"))), vec![]);
        assert_eq!(zone_cmds(&a.handle(looked_up(Ok("Mars/Olympus_Mons")))), [&ZoneCmd::Set("Mars/Olympus_Mons".into())]);
        assert_eq!(a.handle(set("Mars/Olympus_Mons", Err("no time zone"))), vec![]);
        assert_eq!(zone(&a).as_deref(), Some("America/Toronto"));
        open_time_zone(&mut a);
        press(&mut a, "Zone");
        assert_eq!(focused(&a), "America"); // the zone in effect's
        press(&mut a, "Europe");
        press(&mut a, "Berlin");
        assert_eq!(a.view().title, "Time zone");
        assert_eq!(a.handle(looked_up(Ok("America/Toronto"))), vec![]);
    }

    #[test]
    fn choosing_automatic_asks_unless_allowed_and_looks_up_at_once() {
        let mut a = zone_app(json!({"time.zone": "manual"}));
        a.handle(connected("Home"));
        assert_eq!(a.view().title, "Library"); // manual: not asked
        open_time_zone(&mut a);
        assert_eq!(a.view().title, "Time zone");
        assert_eq!(focused(&a), "Set time zone");
        tap(&mut a, Button::Right, 0);
        assert_eq!(a.view().title, "Look up the time zone?");
        let e = press(&mut a, "Allow");
        assert_eq!(zone_cmds(&e), [&ZoneCmd::LookUp]);
        assert_eq!(a.view().title, "Time zone");
        // Manual and back: looked up again, without asking.
        assert_eq!(zone_cmds(&tap(&mut a, Button::Right, 0)), Vec::<&ZoneCmd>::new());
        assert_eq!(zone_cmds(&tap(&mut a, Button::Right, 0)), [&ZoneCmd::LookUp]);
        assert_eq!(a.view().title, "Time zone");
    }

    #[test]
    fn opening_the_time_zone_screen_asks_if_not_yet_allowed() {
        let mut a = zone_app(json!({}));
        open_time_zone(&mut a);
        assert_eq!(a.view().title, "Look up the time zone?");
        tap(&mut a, Button::B, 0);
        assert_eq!(a.view().title, "Time zone");
        assert!(labels(&a).contains(&"Zone".to_string()));
    }

    #[test]
    fn settings_show_the_zone_in_effect_and_the_picker_marks_it() {
        let mut a = zone_app(json!({"time.zone": "manual"}));
        a.handle(set("America/Argentina/Buenos_Aires", Ok(())));
        tap(&mut a, Button::Up, 0);
        tap(&mut a, Button::A, 0);
        let row = a.view().rows.iter().find(|r| r.label == "Time zone").cloned().unwrap();
        assert_eq!(row.kind, RowKind::Action { value: Some("America/Argentina/Buenos Aires".into()) });
        press(&mut a, "Time zone");
        press(&mut a, "Zone");
        press(&mut a, "America");
        assert_eq!(focused(&a), "Argentina / Buenos Aires");
        assert_eq!(a.view().rows[a.focus()].kind, RowKind::Text { detail: Some("Current".into()) });
        // A device whose zone can't be set has no row.
        let mut b = app();
        tap(&mut b, Button::Up, 0);
        tap(&mut b, Button::A, 0);
        assert!(!labels(&b).contains(&"Time zone".to_string()));
    }
}

mod card {
    use super::*;

    const A: &str = "mounted\n\ncard=aaaa partition=sdcard type=vfat check=clean\n";
    const B: &str = "mounted\n\ncard=bbbb partition=sdcard type=exfat check=clean\n";
    const GONE: &str = "absent\nNo SD card.\n";
    const CHECKING: &str = "checking\nChecking the SD card…\ncard=aaaa\n";

    fn card(text: &str) -> Msg { Msg::Card(Card::parse(text)) }

    fn labels(a: &App) -> Vec<String> { a.view().rows.iter().map(|r| r.label.clone()).collect() }

    /// An app with the card `text` in, its games scanned and its settings read.
    fn with_card(text: &str) -> App {
        let mut a = App::new(builtin_systems(), quiet());
        a.handle(card(text));
        a.handle(Msg::Library(vec![game("gba", "Golden Sun"), game("gba", "Mother 3")]));
        a.handle(Msg::SettingsLoaded(Layer::Global, Default::default()));
        a
    }

    /// `with_card(A)`, playing Golden Sun.
    fn playing() -> App {
        let mut a = with_card(A);
        tap(&mut a, Button::A, 0);
        tap(&mut a, Button::A, 10);
        assert_eq!(a.input_target(), InputTarget::Game);
        a
    }

    #[test]
    fn the_library_and_settings_are_read_once_the_card_is_there() {
        let mut a = App::new(builtin_systems(), quiet());
        assert_eq!(a.handle(Msg::Library(vec![game("gba", "Golden Sun")])), vec![]); // no card yet
        assert!(labels(&a).contains(&"Looking for games…".to_string()));
        assert_eq!(a.handle(card(A)), vec![Effect::ReadCard { writable: true }, Effect::Redraw(None)]);
        assert_eq!(a.handle(card(A)), vec![]); // nothing changed
    }

    #[test]
    fn without_a_card_the_defaults_apply_and_nothing_is_saved() {
        let mut a = App::new(builtin_systems(), quiet());
        let e = a.handle(card(GONE));
        assert!(e.contains(&Effect::Apply { key: "display.brightness", value: json!(6) }), "{e:?}");
        assert!(labels(&a).contains(&"No SD card.".to_string()));
        tap(&mut a, Button::Up, 0);
        tap(&mut a, Button::A, 10); // Settings
        let e = tap(&mut a, Button::Right, 20); // brightness 7
        assert!(e.contains(&Effect::Apply { key: "display.brightness", value: json!(7) }) && !e.contains(&Effect::SaveSettings(Layer::Global)), "{e:?}");
        // The card's own settings replace those in memory...
        assert_eq!(a.handle(card(A)), vec![Effect::ReadCard { writable: true }, Effect::Redraw(None)]);
        assert!(a.ctx().settings.layer(&Layer::Global).is_empty());
        // ...but a change made before they are read wins, and is saved again with them.
        assert!(tap(&mut a, Button::Right, 30).contains(&Effect::SaveSettings(Layer::Global)));
        let stored = serde_json::Map::from_iter([("display.brightness".to_string(), json!(3)), ("video.scaling".to_string(), json!("fit"))]);
        let e = a.handle(Msg::SettingsLoaded(Layer::Global, stored));
        assert!(e.contains(&Effect::Apply { key: "display.brightness", value: json!(7) }) && e.contains(&Effect::SaveSettings(Layer::Global)), "{e:?}");
        assert_eq!(a.ctx().settings.layer(&Layer::Global).get("video.scaling"), Some(&json!("fit")));
    }

    #[test]
    fn a_card_that_goes_empties_the_library_and_says_why() {
        let mut a = with_card(A);
        tap(&mut a, Button::A, 0); // the Game Boy Advance list
        assert_eq!(a.handle(card(GONE)), vec![Effect::ForgetCard, Effect::Redraw(Some(Transition::Pop))]);
        assert_eq!(a.view().title, "Library");
        assert_eq!(labels(&a), ["Systems", "No SD card.", "Device", "Settings"]);
        assert_eq!(a.handle(Msg::Library(vec![game("gba", "Golden Sun")])), vec![]); // a scan of the old card, late
        assert_eq!(a.handle(card(CHECKING)), vec![Effect::Redraw(None)]);
        assert!(labels(&a).contains(&"Checking the SD card…".to_string()));
        assert_eq!(a.handle(card(A)), vec![Effect::ReadCard { writable: true }, Effect::Redraw(None)]);
        assert!(labels(&a).contains(&"Looking for games…".to_string()));
    }

    #[test]
    fn a_read_only_card_says_so_above_the_library_and_nothing_is_saved() {
        let mut a = with_card("read-only write-protected\nLocked: games play, nothing is saved.\ncard=aaaa\n");
        assert_eq!(labels(&a)[..3], ["Locked: games play, nothing is saved.", "Systems", "Game Boy Advance"]);
        tap(&mut a, Button::Up, 0);
        tap(&mut a, Button::A, 10); // Settings
        assert!(!tap(&mut a, Button::Right, 20).contains(&Effect::SaveSettings(Layer::Global)));
        tap(&mut a, Button::Menu, 30);
        tap(&mut a, Button::Down, 35); // from Settings, wraps to the Game Boy Advance
        tap(&mut a, Button::A, 40);
        let e = tap(&mut a, Button::A, 50);
        assert!(e.contains(&Effect::Notice("The SD card is read-only: progress in this game won't be saved".into())), "{e:?}");
        tap(&mut a, Button::Menu, 60);
        tap(&mut a, Button::Down, 70);
        assert_eq!(tap(&mut a, Button::A, 80), vec![Effect::Notice("The SD card is read-only: the state can't be saved".into())]);
        assert_eq!(a.view().title, "Paused");
        for (i, b) in [Button::Up, Button::Up].into_iter().enumerate() { tap(&mut a, b, 90 + i as u64); } // wraps to "Quit game"
        assert_eq!(tap(&mut a, Button::A, 100), vec![Effect::AbandonGame, Effect::Redraw(Some(Transition::Pop))]);
    }

    #[test]
    fn a_repair_is_told_once_until_dismissed() {
        let mut a = with_card("mounted\nErrors were repaired.\ncard=aaaa check=repaired\n");
        assert_eq!(labels(&a)[..2], ["Errors were repaired.", "OK"]);
        assert_eq!(focused(&a), "OK");
        assert_eq!(tap(&mut a, Button::A, 0), vec![Effect::Redraw(None)]);
        assert_eq!(labels(&a)[0], "Systems");
    }

    #[test]
    fn a_long_repair_message_is_read_from_its_top_before_its_ok() {
        let message = vec!["word"; 80].join(" "); // 8 lines in a list 1024 u wide
        let mut a = with_card(&format!("mounted\n{message}\ncard=aaaa check=repaired\n"));
        // In a view 400 u tall, with OK focused the message's top would be out of view.
        a.set_list_area(1024.0, 400.0);
        assert_eq!((focused(&a), a.reading()), (message.clone(), Some(0.0)));
        tap(&mut a, Button::Down, 0);
        assert_eq!((focused(&a), a.reading()), (message.clone(), Some(76.0)));
        tap(&mut a, Button::Down, 10);
        assert_eq!((focused(&a), a.reading()), ("OK".into(), None));
        assert_eq!(tap(&mut a, Button::A, 20), vec![Effect::Redraw(None)]);
    }

    #[test]
    fn a_game_whose_card_goes_waits_for_that_card_then_saves() {
        let mut a = playing();
        assert_eq!(a.handle(card(GONE)), vec![Effect::ForgetCard, Effect::PauseGame(true), Effect::Redraw(Some(Transition::Push))]);
        assert_eq!((a.view().title.as_str(), a.input_target()), ("Card removed", InputTarget::App));
        assert!(labels(&a).contains(&"Insert the SD card again to save the game.".to_string()));
        assert_eq!(tap(&mut a, Button::B, 20), vec![]); // there is no going back to the game
        assert_eq!(tap(&mut a, Button::Menu, 30), vec![]);
        assert_eq!(a.handle(card(CHECKING)), vec![Effect::Redraw(None)]);
        // Another card is read meanwhile, but isn't the game's.
        assert_eq!(a.handle(card(B)), vec![Effect::ReadCard { writable: true }, Effect::Redraw(None)]);
        assert!(labels(&a)[0].starts_with("This isn't the SD card the game was started from"), "{:?}", labels(&a));
        a.handle(card(GONE));
        assert_eq!(a.handle(card(A)), vec![Effect::ReadCard { writable: true }, Effect::QuitGame,
                                             Effect::Notice("The game was saved".into()), Effect::Redraw(Some(Transition::Pop))]);
        assert_eq!((a.playing(), a.view().title.as_str()), (None, "Library"));
    }

    #[test]
    fn a_game_whose_card_comes_back_read_only_can_only_be_quit_unsaved() {
        let mut a = playing();
        tap(&mut a, Button::Menu, 20); // with the game menu open, the wait goes over it
        a.handle(card(GONE));
        a.handle(card("read-only damaged\nGames play, nothing is saved.\ncard=aaaa\n"));
        assert_eq!(labels(&a)[..2], ["The SD card is back, but read-only: the game can't be saved.", "Games play, nothing is saved."]);
        assert_eq!(focused(&a), "Quit unsaved");
        assert_eq!(tap(&mut a, Button::A, 30), vec![Effect::AbandonGame, Effect::Redraw(Some(Transition::Pop))]);
        assert_eq!((a.playing(), a.view().title.as_str()), (None, "Library"));
    }

    #[test]
    fn powering_off_while_waiting_for_the_card_writes_nothing() {
        let mut a = playing();
        a.handle(card(GONE));
        event(&mut a, Button::Power, true, 100);
        a.handle(Msg::Tick(Duration::from_millis(1100)));
        assert_eq!(a.view().title, "Power");
        assert_eq!(tap(&mut a, Button::A, 1200), vec![Effect::AbandonGame, Effect::Notice("Turning off…".into()), Effect::PowerOff]);
    }

    #[test]
    fn when_the_systems_own_card_goes_only_its_message_shows() {
        let mut a = playing();
        a.handle(card("failed system-card-removed\nThe Brick turned off: insert the card and turn it on.\n"));
        assert_eq!(labels(&a), ["The Brick turned off: insert the card and turn it on."]);
    }

    #[test]
    fn a_game_its_bios_halts_while_waiting_for_its_card_ends_unsaved() {
        let mut a = playing();
        a.handle(card(GONE));
        a.handle(card(B)); // writable, but not the game's
        let e = a.handle(Msg::GameStopped("the BIOS halted".into()));
        assert_eq!(e[0], Effect::AbandonGame);
        assert_eq!((a.playing(), a.view().title.as_str()), (None, "Game Boy Advance")); // where it was launched from
    }

    #[test]
    fn nothing_acts_on_the_settings_while_a_new_cards_are_read() {
        use crate::timezone::{ZoneCmd, ZoneEvent};
        use crate::wireless::wifi::{WifiEvent, WifiLink};
        let mut a = App::new(builtin_systems(), quiet());
        a.has_radios(true, false);
        a.has_time_zone(true);
        a.handle(card(A));
        let allowed = || json!({"time.zone_lookup_allowed": true}).as_object().unwrap().clone();
        a.handle(Msg::SettingsLoaded(Layer::Global, allowed()));
        a.handle(Msg::TimeZone(ZoneEvent::Started { zone: Some("UTC".into()), zones: vec!["UTC".into()] }));
        let e = a.handle(Msg::Wifi(WifiEvent::Link(WifiLink::Connected { ssid: "Home".into(), address: "10.0.0.2".into(), bars: None })));
        assert!(e.contains(&Effect::TimeZone(ZoneCmd::LookUp)), "{e:?}");
        a.handle(card(GONE));
        // Until card B's settings are read, its defaults (lookups not allowed) mustn't ask.
        let e = a.handle(card(B));
        assert!(!e.iter().any(|e| matches!(e, Effect::TimeZone(_) | Effect::Wifi(_))), "{e:?}");
        assert_eq!(a.view().title, "Library");
        let e = a.handle(Msg::SettingsLoaded(Layer::Global, allowed()));
        assert!(!e.contains(&Effect::TimeZone(ZoneCmd::LookUp)), "{e:?}"); // already looked up on this network
        assert_eq!(a.view().title, "Library");
    }

    #[test]
    fn a_write_refused_as_read_only_has_the_card_looked_at_once() {
        let mut a = with_card(A);
        let notice = || Effect::Notice("The SD card has turned read-only: nothing more can be saved".into());
        assert_eq!(a.handle(Msg::CardReadOnly), vec![Effect::Card(CardCmd::Recheck), notice()]);
        assert_eq!(a.handle(Msg::CardReadOnly), vec![]); // already asked
        assert_eq!(a.handle(card(A)), vec![]); // the answer: unchanged
        assert_eq!(a.handle(Msg::CardReadOnly), vec![Effect::Card(CardCmd::Recheck), notice()]); // so another failure asks again
        let e = a.handle(card("read-only damaged\nGames play, nothing is saved.\ncard=aaaa\n"));
        assert_eq!(e, vec![Effect::ReadCard { writable: false }, Effect::Redraw(None)]);
        assert_eq!(a.ctx().card.as_ref().map(|c| c.status), Some(CardStatus::ReadOnly));
        assert_eq!(a.handle(Msg::CardReadOnly), vec![]); // known to be read-only
    }
}

mod problems;
mod explained;
