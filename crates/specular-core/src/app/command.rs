//! Carrying out commands sent from outside Specular (see `crate::command`).

use super::*;
use crate::command::{Command, HELP};

impl App {
    /// Carries out `cmd`: its reply (Ok, or Err: why it couldn't be done), and the effects to
    /// carry out. `Command::Press` is the UI's to carry out, as it takes time; here it is refused.
    pub fn command(&mut self, cmd: &Command) -> (Result<String, String>, Vec<Effect>) {
        match cmd {
            Command::Play(what) => match self.play(what) {
                Ok((title, effects)) => (Ok(format!("starting {title}")), effects),
                Err(why) => (Err(why), vec![]),
            },
            Command::Quit => {
                if self.asleep { return (Err("the device is asleep".into()), vec![]); }
                let Some(title) = self.playing.as_ref().map(|g| g.title.clone()) else { return (Err("no game is playing".into()), vec![]) };
                let mut effects: Vec<Effect> = self.end_game().into_iter().collect();
                // Closing the game menu redraws; with none open, the game's picture still has to go.
                let closed = self.go(|nav| nav.close(&screens::GAME_MENU));
                effects.extend(if closed.is_empty() { vec![Effect::Redraw(None)] } else { closed });
                (Ok(format!("quitting {title}")), effects)
            }
            Command::Status => (Ok(self.command_status().to_string()), vec![]),
            Command::Help => (Ok(HELP.into()), vec![]),
            Command::Press(..) => (Err("press can't be carried out by the app".into()), vec![]),
        }
    }

    /// Starts the game `what` names (`find_game`) over whatever screen is showing: its title, and
    /// the effects. Refused while asleep, while a game plays, while the installer works, or
    /// without the card.
    fn play(&mut self, what: &str) -> Result<(String, Vec<Effect>), String> {
        if self.asleep { return Err("the device is asleep".into()); }
        if self.task_running() { return Err(self.task_wait_notice()); }
        if let Some(game) = &self.playing { return Err(format!("{} is playing; quit it first", game.title)); }
        if !self.ctx.card_in() { return Err("the SD card isn't in".into()); }
        let game = find_game(&self.ctx.games, what)?.clone();
        let (title, path) = (game.title.clone(), game.path.clone());
        let effects = self.launch_or_check(game);
        if matches!(effects[..], [Effect::CheckAutosave(_)]) { self.command_launch = Some(path); }
        Ok((title, effects))
    }

    /// The screen showing (its id and params), the game playing (its path under roms/) and
    /// whether the device is asleep.
    fn command_status(&self) -> serde_json::Value {
        let top = self.nav.top();
        serde_json::json!({
            "screen": top.id.0, "params": top.params,
            "playing": self.playing.as_ref().map(|g| g.path.to_string_lossy()),
            "asleep": self.asleep,
        })
    }
}

/// The game at `what` (a path under roms/), else the only one titled `what` (in any case).
fn find_game<'a>(games: &'a [Game], what: &str) -> Result<&'a Game, String> {
    if let Some(game) = games.iter().find(|g| g.path.as_os_str() == what) { return Ok(game); }
    let titled: Vec<&Game> = games.iter().filter(|g| g.title.eq_ignore_ascii_case(what)).collect();
    match titled[..] {
        [game] => Ok(game),
        [] => Err(format!("no game at roms/{what} or titled {what:?}")),
        _ => Err(format!("{} games are titled {what:?}; give one's path: {}", titled.len(),
                         titled.iter().map(|g| g.path.to_string_lossy()).collect::<Vec<_>>().join(", "))),
    }
}
