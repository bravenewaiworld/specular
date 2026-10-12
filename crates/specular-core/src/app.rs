//! The app: everything the user can do, as a state machine. Devices and the UI feed it messages
//! (buttons, time passing, scan results) and carry out the effects it returns (save a file, set
//! the brightness, launch a game). It never touches files, hardware or the UI itself, so all of
//! its behaviour is testable.

use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Duration;

use crate::device::Battery;
use crate::input::{Button, ButtonEvent, Press};
use crate::library::{Game, SystemDecl};
use crate::list;
use crate::model::{Layout, Row, RowKind, ScreenId, ScreenView};
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
    Launch(Game),
    Sleep,
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
    /// Nothing changed.
    None,
    /// The screen's content changed; draw it again.
    Changed,
    Push(ScreenState),
    Back,
    /// Carry these out (include `Effect::Redraw` if the screen changed too).
    Effects(Vec<Effect>),
}

/// One kind of screen. Built-in screens implement it; plugins will register their own.
pub trait Screen {
    /// The screen's rows. `focus` is filled in by the app (leave it 0).
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView;
    /// Handles a press the app didn't take itself (up, down, L1 and R1 move the focus, Menu goes
    /// home, Power sleeps). `focused` is the focused row, if the screen has one that can take
    /// presses (never a header or a disabled row).
    fn press(&self, ctx: &mut Ctx, state: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome;
}

/// The built-in screens' ids.
pub mod screens {
    use super::ScreenId;
    pub const HOME: ScreenId = ScreenId(std::borrow::Cow::Borrowed("home"));
    pub const GAMES: ScreenId = ScreenId(std::borrow::Cow::Borrowed("games"));
    pub const SETTINGS: ScreenId = ScreenId(std::borrow::Cow::Borrowed("settings"));
}

/// Makes a `ScreenId` from a name (for screens registered later).
pub fn screen_id(name: impl Into<Cow<'static, str>>) -> ScreenId { ScreenId(name.into()) }

/// The app's state: the screens, where the user is, and what the screens show.
pub struct App {
    pub ctx: Ctx,
    screens: HashMap<ScreenId, Box<dyn Screen>>,
    nav: NavStack,
    repeater: Repeater,
}

impl App {
    /// An app with the built-in screens, knowing these systems and settings. It starts at home
    /// with no games until a `Msg::Library` arrives.
    pub fn new(systems: Vec<SystemDecl>, settings: Vec<SettingDecl>) -> Self {
        let mut screens: HashMap<ScreenId, Box<dyn Screen>> = HashMap::new();
        screens.insert(screens::HOME, Box::new(Home));
        screens.insert(screens::GAMES, Box::new(Games));
        screens.insert(screens::SETTINGS, Box::new(SettingsScreen));
        Self {
            ctx: Ctx { systems, games: Vec::new(), settings: Settings::new(settings), battery: None },
            screens,
            nav: NavStack::new(ScreenState::new(screens::HOME, None)),
            repeater: Repeater::default(),
        }
    }

    /// Adds a screen (or replaces a built-in one).
    pub fn register(&mut self, id: ScreenId, screen: Box<dyn Screen>) { self.screens.insert(id, screen); }

    /// The title of the screen below the current one (for a breadcrumb), if any.
    pub fn parent_title(&self) -> Option<String> {
        let parent = self.nav.parent()?;
        Some(self.screens[&parent.id].view(&self.ctx, parent).title)
    }

    /// When the next button repeat is due, so the caller can wake up for it.
    pub fn next_deadline(&self) -> Option<Duration> { self.repeater.next_deadline() }

    /// The current screen, with the focus on a focusable row.
    pub fn view(&self) -> ScreenView {
        let state = self.nav.top();
        let mut v = self.screens[&state.id].view(&self.ctx, state);
        v.focus = state.focus.as_ref().and_then(|id| v.rows.iter().position(|r| &r.id == id))
            .filter(|&i| v.rows[i].kind.focusable())
            .or_else(|| list::first(&v.rows)).unwrap_or(0);
        v
    }

    /// Feeds one message; returns the effects the caller must carry out, in order.
    pub fn handle(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Button(e) => self.repeater.on_event(&e).map(|p| self.press(p)).unwrap_or_default(),
            Msg::Tick(now) => self.repeater.poll(now).map(|p| self.press(p)).unwrap_or_default(),
            Msg::Library(games) => { self.ctx.games = games; vec![Effect::Redraw(None)] }
            Msg::Woke => { self.repeater.reset(); vec![] }
            Msg::Battery(b) => {
                let changed = self.ctx.battery != Some(b);
                self.ctx.battery = Some(b);
                if changed { vec![Effect::Redraw(None)] } else { vec![] }
            }
            Msg::SettingsLoaded(layer, values) => {
                self.ctx.settings.set_layer(layer, values);
                let s = &self.ctx.settings;
                std::iter::once(Effect::Redraw(None))
                    .chain(s.decls().iter().filter_map(|d| s.get(d.key, &Layer::Global).map(|(v, _)| Effect::Apply { key: d.key, value: v.clone() })))
                    .collect()
            }
        }
    }

    fn press(&mut self, p: Press) -> Vec<Effect> {
        let view = self.view();
        match p.button {
            Button::Up | Button::Down | Button::L1 | Button::R1 => {
                let delta = match p.button { Button::Up => -1, Button::Down => 1, Button::L1 => -6, _ => 6 };
                let to = list::step(&view.rows, view.focus, delta, p.repeat);
                if to == view.focus { return vec![]; }
                self.nav.top_mut().focus = view.rows.get(to).map(|r| r.id.clone());
                return vec![Effect::Redraw(None)];
            }
            // (A, B, Menu and Power never repeat.)
            Button::Menu => return self.nav.home().map(|t| { self.repeater.reset(); vec![Effect::Redraw(Some(t))] }).unwrap_or_default(),
            Button::Power => return vec![Effect::Sleep],
            _ => {}
        }
        let focused = view.rows.get(view.focus).filter(|r| r.kind.focusable() && r.enabled);
        let id = self.nav.top().id.clone();
        match self.screens[&id].press(&mut self.ctx, self.nav.top_mut(), focused, p) {
            Outcome::None => vec![],
            Outcome::Changed => vec![Effect::Redraw(None)],
            Outcome::Push(s) => { self.repeater.reset(); vec![Effect::Redraw(Some(self.nav.push(s)))] }
            Outcome::Back => self.nav.pop().map(|t| { self.repeater.reset(); vec![Effect::Redraw(Some(t))] }).unwrap_or_default(),
            Outcome::Effects(e) => e,
        }
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
        ScreenView { title: "Library".into(), layout: Layout::List, rows, focus: 0, hints: vec![(Button::A, "Open".into())] }
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

/// One system's games. A game's row id is its path under `roms/`.
struct Games;

impl Screen for Games {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let system = state.params.as_deref().unwrap_or_default();
        let title = ctx.systems.iter().find(|s| s.id == system).map_or(system, |s| s.name).to_string();
        let rows = ctx.games.iter().filter(|g| g.system == system)
            .map(|g| Row::new(g.path.to_string_lossy(), &g.title, RowKind::Text { detail: None })).collect();
        ScreenView { title, layout: Layout::List, rows, focus: 0, hints: vec![(Button::B, "Back".into()), (Button::A, "Play".into())] }
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        match (p.button, focused) {
            (Button::B, _) => Outcome::Back,
            (Button::A, Some(row)) => ctx.games.iter().find(|g| g.path.as_os_str() == row.id.0.as_str())
                .map_or(Outcome::None, |g| Outcome::Effects(vec![Effect::Launch(g.clone())])),
            _ => Outcome::None,
        }
    }
}

/// The global settings, generated from the declarations.
struct SettingsScreen;

impl Screen for SettingsScreen {
    fn view(&self, ctx: &Ctx, _: &ScreenState) -> ScreenView {
        ScreenView {
            title: "Settings".into(), layout: Layout::List, rows: ctx.settings.rows(&Layer::Global, ctx.visibility()), focus: 0,
            hints: vec![(Button::B, "Back".into())],
        }
    }

    fn press(&self, ctx: &mut Ctx, _: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        if p.button == Button::B { return Outcome::Back; }
        let Some((key, value)) = focused.and_then(|row| ctx.settings.press(&row.id.0, &Layer::Global, p)) else { return Outcome::None };
        Outcome::Effects(vec![Effect::Redraw(None), Effect::SaveSettings(Layer::Global), Effect::Apply { key, value }])
    }
}

#[cfg(test)]
mod tests;
