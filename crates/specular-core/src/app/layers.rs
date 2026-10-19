//! The settings of one game, and of all of one system's games: the settings that may be set that
//! specifically (with "Developer details", the system's core's options too), each marked when it
//! overrides a less specific layer. X resets a marked one, so it inherits again.

use std::collections::HashMap;

use super::{Ctx, Outcome, Screen, screens};
use crate::input::{Button, Press};
use crate::library::{Game, SystemDecl};
use crate::model::{Layout, Row, RowKind, ScreenId, ScreenView};
use crate::nav::ScreenState;
use crate::settings::Layer;

pub(super) fn register(screens: &mut HashMap<ScreenId, Box<dyn Screen>>) {
    screens.insert(screens::GAME_SETTINGS, Box::new(GameSettings));
    screens.insert(screens::SYSTEM_SETTINGS, Box::new(SystemSettings));
}

/// One game's settings (its path under `roms/` in the params), then a row opening its system's,
/// then its core's options.
struct GameSettings;

impl GameSettings {
    fn game<'a>(ctx: &'a Ctx, state: &ScreenState) -> Option<(&'a Game, &'a SystemDecl)> {
        let game = ctx.games.iter().find(|g| Some(g.path.to_string_lossy().as_ref()) == state.params.as_deref())?;
        Some((game, ctx.systems.iter().find(|s| s.id == game.system)?))
    }
}

impl Screen for GameSettings {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        // The library was scanned again since the screen opened, and the game is gone.
        let Some((game, system)) = Self::game(ctx, state) else {
            return view("Game settings".into(), vec![Row::note("note:gone", "This game is no longer on the SD card")]);
        };
        let all = [Row::new("header:system", system.name, RowKind::Header),
                   Row::new("open:system", format!("All {} games…", system.name), RowKind::Action { value: None })];
        view(game.title.clone(), rows(ctx, &game.settings_layer(), system, all.into()))
    }

    fn press(&self, ctx: &mut Ctx, state: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        let game = Self::game(ctx, state);
        if let (Some(row), Some((_, system))) = (focused, game) && row.id.0 == "open:system" {
            let open = ScreenState::new(screens::SYSTEM_SETTINGS, Some(system.id.into()));
            return if p.button == Button::A { Outcome::Push(open) } else { Outcome::None };
        }
        press(ctx, game.map(|(g, _)| g.settings_layer()), focused, p)
    }
}

/// The settings of all of one system's games (its id in the params), then its core's options.
struct SystemSettings;

impl SystemSettings {
    fn system<'a>(ctx: &'a Ctx, state: &ScreenState) -> Option<&'a SystemDecl> { ctx.systems.iter().find(|s| Some(s.id) == state.params.as_deref()) }
}

impl Screen for SystemSettings {
    fn view(&self, ctx: &Ctx, state: &ScreenState) -> ScreenView {
        let Some(system) = Self::system(ctx, state) else { return view("Settings".into(), vec![]) };
        view(format!("All {} games", system.name), rows(ctx, &Layer::System(system.id.into()), system, vec![]))
    }

    fn press(&self, ctx: &mut Ctx, state: &mut ScreenState, focused: Option<&Row>, p: Press) -> Outcome {
        press(ctx, Self::system(ctx, state).map(|s| Layer::System(s.id.into())), focused, p)
    }
}

fn view(title: String, rows: Vec<Row>) -> ScreenView {
    ScreenView { title, layout: Layout::List, rows, hints: vec![(Button::B, "Back".into())], entry: None }
}

/// The rows of `at`'s settings: Specular's, then `between`, then the options of `system`'s core
/// (developer settings, so only with "Developer details"). A row that overrides has a hint for
/// resetting it.
fn rows(ctx: &Ctx, at: &Layer, system: &SystemDecl, between: Vec<Row>) -> Vec<Row> {
    let show = ctx.visibility();
    let core = system.core.map(|c| ctx.settings.rows(at, show, Some(c.id()))).unwrap_or_default();
    let mut rows: Vec<Row> = ctx.settings.rows(at, show, None).into_iter().chain(between).chain(core).collect();
    for r in rows.iter_mut().filter(|r| r.overridden) { r.hint = Some((Button::X, "Reset".into())); }
    rows
}

/// A press on a screen of `at`'s settings (None: its game or system is gone): B goes back; on a
/// setting's row A, left and right change it, and X resets it if it overrides.
fn press(ctx: &mut Ctx, at: Option<Layer>, focused: Option<&Row>, p: Press) -> Outcome {
    if p.button == Button::B { return Outcome::Back; }
    let (Some(at), Some(row)) = (at, focused) else { return Outcome::None };
    let key = match p.button {
        Button::X if row.overridden && !p.repeat => ctx.settings.reset(&row.id.0, &at),
        _ => ctx.settings.press(&row.id.0, &at, p).map(|(key, _)| key),
    };
    key.map_or(Outcome::None, |key| Outcome::SettingChanged(at, key))
}
