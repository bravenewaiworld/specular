//! The app: everything the user can do, as a state machine. Devices and the UI feed it messages
//! (buttons, time passing, scan results) and carry out the effects it returns (save a file, set
//! the brightness, launch a game). It never touches files, hardware or the UI itself, so all of
//! its behaviour is testable.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use crate::device::Battery;
use crate::input::{Button, ButtonEvent, Press};
use crate::library::{Game, SystemDecl};
use crate::list;
use crate::model::{ImageRef, Layout, Row, RowKind, ScreenId, ScreenView};
use crate::nav::{NavStack, ScreenState, Transition};
use crate::repeat::Repeater;
use crate::settings::{Layer, SettingDecl, Settings, Visibility, builtin};

/// Something that happened.
#[derive(Debug)]
pub enum Msg {
    Button(ButtonEvent),
    /// The current time, on the clock button times use; due button repeats are generated.
    Tick(Duration),
    /// A library scan finished.
    Library(Vec<Game>),
    Battery(Battery),
    /// The device woke from sleep.
    Woke,
    /// The running game ended by itself, or failed to start (with why).
    GameEnded(Option<String>),
    /// A settings layer was read from its file.
    SettingsLoaded(Layer, serde_json::Map<String, serde_json::Value>),
}

/// Something the app wants done, in the order given.
#[derive(Debug, PartialEq)]
pub enum Effect {
    /// The screen changed; draw it again (with this transition, if the screen itself changed).
    Redraw(Option<Transition>),
    /// Write this settings layer to its file.
    SaveSettings(Layer),
    /// A global setting has this value (on loading, and when it changes). The UI carries out the
    /// ones the device acts on, such as brightness, and ignores the rest.
    Apply { key: &'static str, value: serde_json::Value },
    /// Start playing a game.
    Launch(Game),
    /// Pause (true) or resume the running game (its in-game menu opened or closed).
    PauseGame(bool),
    /// Save the running game's state, or load the state saved last.
    SaveState,
    LoadState,
    ResetGame,
    /// End the running game (its battery save is written).
    QuitGame,
    Sleep,
}

/// Who a button press is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputTarget {
    /// The app's screens.
    App,
    /// The running game: every button but Menu and Power goes to it (the app gets those two).
    Game,
}

/// What a screen needs to build its view and act on presses.
pub struct Ctx {
    pub systems: Vec<SystemDecl>,
    pub games: Vec<Game>,
    pub settings: Settings,
    pub battery: Option<Battery>,
}

impl Ctx {
    /// Which settings are shown: developer ones too while "Developer details" is on.
    pub fn visibility(&self) -> Visibility {
        let on = self.settings.get(builtin::DEVELOPER_DETAILS, &Layer::Global).is_some_and(|(v, _)| *v == true);
        if on { Visibility::Developer } else { Visibility::User }
    }
}

/// What a press on a screen leads to.
pub enum Outcome {
    /// Nothing on the screen changed (the built screen is kept).
    None,
    /// The screen's content changed; draw it again.
    Changed,
    Push(ScreenState),
    Back,
    /// Start playing this game.
    Launch(Game),
    /// Carry these out (include `Effect::Redraw` if the screen changed too).
    Effects(Vec<Effect>),
}

/// One kind of screen. Built-in screens implement it; plugins will register their own.
pub trait Screen {
    /// The screen's rows. The app builds this once and keeps it until the library, the settings
    /// or the screen shown change, or a press returns `Changed` or `Effects`; so it needn't be
    /// cheap, but it may depend only on `ctx.systems`, `ctx.games`, `ctx.settings` and
    /// `state.params` (not on the battery or the focus).
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView;
    /// Handles a press the app didn't take itself (up, down, L1 and R1 move the focus, and in a
    /// grid so do left and right; Menu goes home, Power sleeps). `focused` is the focused row, if
    /// the screen has one that can take presses (never a header or a disabled row).
    fn press(&self, ctx: &mut Ctx, state: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome;
}

/// The built-in screens' ids.
pub mod screens {
    use super::ScreenId;
    pub const HOME: ScreenId = ScreenId(std::borrow::Cow::Borrowed("home"));
    pub const GAMES: ScreenId = ScreenId(std::borrow::Cow::Borrowed("games"));
    pub const SETTINGS: ScreenId = ScreenId(std::borrow::Cow::Borrowed("settings"));
    /// Over a paused game. Its presses are handled by the app itself (see `App::game_menu`).
    pub const GAME_MENU: ScreenId = ScreenId(std::borrow::Cow::Borrowed("game menu"));
}

/// Makes a `ScreenId` from a name (for screens registered later).
pub fn screen_id(name: impl Into<Cow<'static, str>>) -> ScreenId { ScreenId(name.into()) }

/// The app's state: the screens, where the user is, and what the screens show.
pub struct App {
    ctx: Ctx,
    screens: HashMap<ScreenId, Box<dyn Screen>>,
    nav: NavStack,
    repeater: Repeater,
    /// The game being played (whether or not its menu is open).
    playing: Option<Game>,
    /// How many columns a grid has on this display (from the UI; see `list::grid_columns`).
    grid_columns: usize,
    /// The current screen as last built, and its parent's title; None when they must be built
    /// again.
    cache: RefCell<Option<(Rc<ScreenView>, Option<String>)>>,
}

impl App {
    /// An app with the built-in screens, knowing these systems and settings. It starts at home
    /// with no games until a `Msg::Library` arrives.
    pub fn new(systems: Vec<SystemDecl>, settings: Vec<SettingDecl>) -> Self {
        let mut screens: HashMap<ScreenId, Box<dyn Screen>> = HashMap::new();
        screens.insert(screens::HOME, Box::new(Home));
        screens.insert(screens::GAMES, Box::new(Games));
        screens.insert(screens::SETTINGS, Box::new(SettingsScreen));
        screens.insert(screens::GAME_MENU, Box::new(GameMenu));
        Self {
            ctx: Ctx { systems, games: Vec::new(), settings: Settings::new(settings), battery: None },
            screens,
            nav: NavStack::new(ScreenState::new(screens::HOME, None)),
            repeater: Repeater::default(),
            grid_columns: 1,
            playing: None,
            cache: RefCell::default(),
        }
    }

    /// Adds a screen (or replaces a built-in one).
    pub fn register(&mut self, id: ScreenId, screen: Box<dyn Screen>) { self.screens.insert(id, screen); self.changed(); }

    /// Tells the app how many columns the UI's grids have, so up and down move by a whole row.
    pub fn set_grid_columns(&mut self, n: usize) { self.grid_columns = n.max(1); }

    /// The game being played, if any.
    pub fn playing(&self) -> Option<&Game> { self.playing.as_ref() }

    /// Who buttons are for now: the game while one is played and its menu is closed.
    pub fn input_target(&self) -> InputTarget {
        if self.playing.is_some() && self.nav.top().id != screens::GAME_MENU { InputTarget::Game } else { InputTarget::App }
    }

    /// What the screens see (read-only: changes go through messages, so the cache stays right).
    pub fn ctx(&self) -> &Ctx { &self.ctx }

    /// The title of the screen below the current one (for a breadcrumb), if any.
    pub fn parent_title(&self) -> Option<String> { self.built().1 }

    /// When the next button repeat is due, so the caller can wake up for it.
    pub fn next_deadline(&self) -> Option<Duration> { self.repeater.next_deadline() }

    /// The current screen (built once, then shared until it changes).
    pub fn view(&self) -> Rc<ScreenView> { self.built().0 }

    fn built(&self) -> (Rc<ScreenView>, Option<String>) {
        self.cache.borrow_mut().get_or_insert_with(|| {
            let state = self.nav.top();
            let parent = self.nav.parent().map(|p| self.screens[&p.id].view(&self.ctx, p).title);
            (Rc::new(self.screens[&state.id].view(&self.ctx, state)), parent)
        }).clone()
    }

    /// The focused row of `view()`: the one the user left the focus on, if it is still there and
    /// focusable, else the first focusable row (0 if none).
    pub fn focus(&self) -> usize {
        let v = self.view();
        self.nav.top().focus.as_ref().and_then(|id| v.rows.iter().position(|r| &r.id == id))
            .filter(|&i| v.rows[i].kind.focusable())
            .or_else(|| list::first(&v.rows)).unwrap_or(0)
    }

    /// Forgets the built screen, after anything that may change what it shows.
    fn changed(&self) { self.cache.borrow_mut().take(); }

    /// Feeds one message; returns the effects the caller must carry out, in order.
    pub fn handle(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Button(e) => self.repeater.on_event(&e).map(|p| self.press(p)).unwrap_or_default(),
            Msg::Tick(now) => self.repeater.poll(now).map(|p| self.press(p)).unwrap_or_default(),
            Msg::Library(games) => { self.ctx.games = games; self.changed(); vec![Effect::Redraw(None)] }
            // A game slept through stays paused, with its menu open.
            Msg::Woke => {
                self.repeater.reset();
                if self.input_target() == InputTarget::Game { self.open_game_menu() } else { vec![] }
            }
            Msg::GameEnded(_) => {
                if self.playing.take().is_none() { return vec![]; }
                if self.nav.top().id == screens::GAME_MENU { self.nav.pop(); }
                self.changed();
                vec![Effect::Redraw(None)]
            }
            Msg::Battery(b) => {
                let changed = self.ctx.battery != Some(b);
                self.ctx.battery = Some(b);
                if changed { vec![Effect::Redraw(None)] } else { vec![] }
            }
            Msg::SettingsLoaded(layer, values) => {
                self.ctx.settings.set_layer(layer, values);
                self.changed();
                let s = &self.ctx.settings;
                std::iter::once(Effect::Redraw(None))
                    .chain(s.decls().iter().filter_map(|d| s.get(d.key, &Layer::Global).map(|(v, _)| Effect::Apply { key: d.key, value: v.clone() })))
                    .collect()
            }
        }
    }

    fn press(&mut self, p: Press) -> Vec<Effect> {
        match (self.input_target(), p.button) {
            (InputTarget::Game, Button::Menu) => return self.open_game_menu(),
            (InputTarget::Game, Button::Power) => return vec![Effect::Sleep],
            (InputTarget::Game, _) => return vec![], // the UI gives these to the game
            _ => {}
        }
        let (view, focus) = (self.view(), self.focus());
        // Focus moves change nothing on the screen itself, so its cache stays. In a grid, left and
        // right move one cell, up and down a row of cells and the shoulder buttons two rows; in a
        // list, up and down move one row and the shoulder buttons six.
        let grid = view.layout == Layout::Grid;
        let (row, page) = if grid { (self.grid_columns as isize, 2 * self.grid_columns as isize) } else { (1, 6) };
        let delta = match p.button {
            Button::Up => Some(-row), Button::Down => Some(row), Button::L1 => Some(-page), Button::R1 => Some(page),
            Button::Left if grid => Some(-1), Button::Right if grid => Some(1),
            _ => None,
        };
        if let Some(delta) = delta {
            let to = list::step(&view.rows, focus, delta, p.repeat);
            if to == focus { return vec![]; }
            self.nav.top_mut().focus = view.rows.get(to).map(|r| r.id.clone());
            return vec![Effect::Redraw(None)];
        }
        let focused = view.rows.get(focus).filter(|r| r.kind.focusable() && r.enabled);
        let id = self.nav.top().id.clone();
        if id == screens::GAME_MENU && p.button != Button::Power { return self.game_menu(p.button, focused.map(|r| r.id.0.as_str())); }
        match p.button {
            // (A, B, Menu and Power never repeat.)
            Button::Menu => return self.go(|nav| nav.home()),
            Button::Power => return vec![Effect::Sleep],
            _ => {}
        }
        let outcome = self.screens[&id].press(&mut self.ctx, self.nav.top_mut(), focused, p);
        match outcome {
            Outcome::None => vec![],
            Outcome::Changed => { self.changed(); vec![Effect::Redraw(None)] }
            Outcome::Push(s) => self.go(|nav| Some(nav.push(s))),
            Outcome::Back => self.go(|nav| nav.pop()),
            Outcome::Launch(game) => {
                self.playing = Some(game.clone());
                self.repeater.reset();
                vec![Effect::Launch(game), Effect::Redraw(None)]
            }
            Outcome::Effects(e) => { self.changed(); e }
        }
    }

    fn open_game_menu(&mut self) -> Vec<Effect> {
        let mut e = vec![Effect::PauseGame(true)];
        e.extend(self.go(|nav| Some(nav.push(ScreenState::new(screens::GAME_MENU, None)))));
        e
    }

    /// A press in the in-game menu: B and Menu resume, A does the focused row's action. Every
    /// action closes the menu.
    fn game_menu(&mut self, button: Button, row: Option<&str>) -> Vec<Effect> {
        let action = match (button, row) {
            (Button::B | Button::Menu, _) | (Button::A, Some("resume")) => None,
            (Button::A, Some("save")) => Some(Effect::SaveState),
            (Button::A, Some("load")) => Some(Effect::LoadState),
            (Button::A, Some("reset")) => Some(Effect::ResetGame),
            (Button::A, Some("quit")) => {
                self.playing = None;
                let mut e = vec![Effect::QuitGame];
                e.extend(self.go(|nav| nav.pop()));
                return e;
            }
            _ => return vec![],
        };
        let mut e: Vec<Effect> = action.into_iter().collect();
        e.push(Effect::PauseGame(false));
        e.extend(self.go(|nav| nav.pop()));
        e
    }

    /// Changes screen (if `f` does), with a redraw that animates the change.
    fn go(&mut self, f: impl FnOnce(&mut NavStack) -> Option<Transition>) -> Vec<Effect> {
        let Some(t) = f(&mut self.nav) else { return vec![] };
        self.repeater.reset();
        self.changed();
        vec![Effect::Redraw(Some(t))]
    }
}

/// Home: the systems that have games, then settings.
struct Home;

impl Screen for Home {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        let mut rows = vec![Row::new("header:systems", "Systems", RowKind::Header)];
        for s in &ctx.systems {
            let n = ctx.games.iter().filter(|g| g.system == s.id).count();
            if n > 0 { rows.push(Row::new(format!("system:{}", s.id), s.name, RowKind::Text { detail: Some(n.to_string()) })); }
        }
        rows.push(Row::new("header:device", "Device", RowKind::Header));
        rows.push(Row::new("settings", "Settings", RowKind::Action { value: None }));
        ScreenView { title: "Library".into(), layout: Layout::List, rows, hints: vec![(Button::A, "Open".into())] }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        let (Button::A, Some(row)) = (p.button, focused) else { return Outcome::None };
        match row.id.0.strip_prefix("system:") {
            Some(system) => Outcome::Push(ScreenState::new(screens::GAMES, Some(system.to_string()))),
            None if row.id.0 == "settings" => Outcome::Push(ScreenState::new(screens::SETTINGS, None)),
            None => Outcome::None,
        }
    }
}

/// One system's games, as a list beside the focused game's box art or as a grid of box art
/// (Y switches). A game's row id is its path under `roms/`.
struct Games;

impl Games {
    fn grid(ctx: &Ctx) -> bool { ctx.settings.get(builtin::LIBRARY_VIEW, &Layer::Global).is_some_and(|(v, _)| v == "grid") }
}

impl Screen for Games {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let system = state.params.as_deref().unwrap_or_default();
        let title = ctx.systems.iter().find(|s| s.id == system).map_or(system, |s| s.name).to_string();
        let rows = ctx.games.iter().filter(|g| g.system == system).map(|g| {
            let image = ImageRef::Art { system: g.system, rom: g.path.clone() };
            Row::new(g.path.to_string_lossy(), &g.title, RowKind::Image { image, detail: None })
        }).collect();
        let grid = Self::grid(ctx);
        ScreenView {
            title, layout: if grid { Layout::Grid } else { Layout::ListWithPicture }, rows,
            hints: vec![(Button::Y, if grid { "List" } else { "Grid" }.into()), (Button::B, "Back".into()), (Button::A, "Play".into())],
        }
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        match (p.button, focused) {
            (Button::B, _) => Outcome::Back,
            (Button::Y, _) if !p.repeat => {
                let value = if Self::grid(ctx) { "list" } else { "grid" };
                let _ = ctx.settings.set(builtin::LIBRARY_VIEW, &Layer::Global, value.into());
                Outcome::Effects(vec![Effect::Redraw(None), Effect::SaveSettings(Layer::Global)])
            }
            (Button::A, Some(row)) => ctx.games.iter().find(|g| g.path.as_os_str() == row.id.0.as_str())
                .map_or(Outcome::None, |g| Outcome::Launch(g.clone())),
            _ => Outcome::None,
        }
    }
}

/// The global settings, generated from the declarations.
struct SettingsScreen;

impl Screen for SettingsScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        ScreenView {
            title: "Settings".into(), layout: Layout::List, rows: ctx.settings.rows(&Layer::Global, ctx.visibility()),
            hints: vec![(Button::B, "Back".into())],
        }
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        if p.button == Button::B { return Outcome::Back; }
        let Some((key, value)) = focused.and_then(|row| ctx.settings.press(&row.id.0, &Layer::Global, p)) else { return Outcome::None };
        Outcome::Effects(vec![Effect::Redraw(None), Effect::SaveSettings(Layer::Global), Effect::Apply { key, value }])
    }
}

/// The menu over a paused game. The app handles its presses (`App::game_menu`), since they
/// control the game itself.
struct GameMenu;

impl Screen for GameMenu {
    fn view(&self, _: &Ctx, _: &ScreenState) -> ScreenView {
        let action = |id: &str, label: &str| Row::new(id, label, RowKind::Action { value: None });
        ScreenView {
            title: "Paused".into(), layout: Layout::List,
            rows: vec![action("resume", "Resume"), action("save", "Save state"), action("load", "Load state"),
                       action("reset", "Restart"), action("quit", "Quit game")],
            hints: vec![(Button::B, "Resume".into()), (Button::A, "Select".into())],
        }
    }

    fn press(&self, _: &mut Ctx, _: &mut ScreenState, _: Option<&Row>, _: Press) -> Outcome { Outcome::None }
}

#[cfg(test)]
mod tests;
