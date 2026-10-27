//! Commands to a running Specular from outside it, for development, tests and scripts: one line of
//! words in, one line back (see docs/commands.md). The UI carries them out (`App::command`, and
//! `Press` itself, as it takes time).

use std::time::Duration;

use crate::input::Button;

/// One command, parsed from its line.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Start a game: by its path under roms/, or by its title if only one game has it.
    Play(String),
    /// Quit the running game, writing its saves.
    Quit,
    /// What is showing and playing, as JSON.
    Status,
    /// Press a button, releasing it after this long.
    Press(Button, Duration),
    Help,
}

/// `help`'s reply: each command and what it does.
pub const HELP: &str = "commands: play <path under roms/ | title> · quit (the running game, saving it) · \
status (JSON) · press <button> [hold ms, default 50] · help";

/// The longest `press` holds a button.
pub const MAX_HOLD: Duration = Duration::from_secs(10);

impl std::str::FromStr for Command {
    type Err = String;

    /// The command on `line`; Err: why it isn't one, for the reply.
    fn from_str(line: &str) -> Result<Self, String> {
        let line = line.trim();
        let (word, rest) = line.split_once(char::is_whitespace).map_or((line, ""), |(w, r)| (w, r.trim()));
        match (word, rest) {
            ("play", "") => Err("play what? A path under roms/, or a title".into()),
            ("play", what) => Ok(Command::Play(what.into())),
            ("quit", "") => Ok(Command::Quit),
            ("status", "") => Ok(Command::Status),
            ("press", args) => press(args),
            ("help", "") => Ok(Command::Help),
            ("", _) => Err("no command; try help".into()),
            ("quit" | "status" | "help", _) => Err(format!("{word} takes nothing after it")),
            _ => Err(format!("unknown command {word:?}; try help")),
        }
    }
}

/// `press`'s arguments: a button's name (`Button::NAMES`), and optionally how long to hold it (ms).
fn press(args: &str) -> Result<Command, String> {
    let mut words = args.split_whitespace();
    let name = words.next().ok_or("press which button?")?;
    let button = Button::named(name).ok_or_else(|| {
        let names: Vec<&str> = Button::NAMES.iter().map(|(_, n)| *n).collect();
        format!("no button {name:?}; the buttons are {}", names.join(", "))
    })?;
    let hold = match words.next() {
        None => Duration::from_millis(50),
        Some(ms) => Duration::from_millis(ms.parse().map_err(|_| format!("{ms:?} isn't a number of milliseconds"))?),
    };
    if words.next().is_some() { return Err("press takes a button and a hold time, nothing more".into()); }
    if hold > MAX_HOLD { return Err(format!("the longest hold is {} ms", MAX_HOLD.as_millis())); }
    Ok(Command::Press(button, hold))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Command, String> { line.parse() }

    #[test]
    fn lines_parse_into_commands() {
        assert_eq!(parse("  play gba/Golden Sun (USA).gba "), Ok(Command::Play("gba/Golden Sun (USA).gba".into())));
        assert_eq!(parse("quit"), Ok(Command::Quit));
        assert_eq!(parse("status\n"), Ok(Command::Status));
        assert_eq!(parse("press A"), Ok(Command::Press(Button::A, Duration::from_millis(50))));
        assert_eq!(parse("press down 3000"), Ok(Command::Press(Button::Down, Duration::from_secs(3))));
        assert_eq!(parse("help"), Ok(Command::Help));
    }

    #[test]
    fn mistakes_say_what_is_wrong() {
        assert!(parse("").unwrap_err().contains("try help"));
        assert!(parse("jump").unwrap_err().contains("unknown command \"jump\""));
        assert!(parse("play").unwrap_err().contains("play what"));
        assert!(parse("quit now").unwrap_err().contains("takes nothing"));
        assert!(parse("press").unwrap_err().contains("which button"));
        assert!(parse("press z").unwrap_err().contains("the buttons are up, down"));
        assert!(parse("press a soon").unwrap_err().contains("milliseconds"));
        assert!(parse("press a 20000").unwrap_err().contains("longest hold"));
        assert!(parse("press a 50 60").unwrap_err().contains("nothing more"));
    }
}
